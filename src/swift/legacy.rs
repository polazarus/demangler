//! Pre-stable Swift (`_T`) demangling.
//!
//! Swift 2/3 used a prefix-ordered grammar that predates the stable postfix
//! grammar in `refs/swift/Mangling.rst`. The reference implementation routes
//! every name without a version prefix through its old demangler
//! (`Context::demangleSymbolAsNode`), so any `_T` name that is not `_T0`
//! belongs here.
//!
//! This is a recursive-descent port of that grammar
//! (`lib/Demangling/OldDemangler.cpp`, upstream commit
//! `fa6c26084a4a0bf51016b7dcdbd0a3c23ed69ee7`). It builds the same node graph
//! as the modern parser, so [`crate::swift::printer`] renders both. Mappings
//! that are not one-to-one are documented at the construction site:
//!
//! - accessor entities become `Variable`/`Subscript` nodes carrying an
//!   [`Accessor`] code, because the shared printer renders accessors there;
//! - `protocol-list` becomes [`Kind::ProtocolListType`], whose printer already
//!   produces `Any` / `A & B`;
//! - `uncurried-function-type` becomes [`Kind::FunctionType`], which prints the
//!   same signature;
//! - old globals (metadata, witness tables, thunks) become the [`Global`]
//!   codes the printer already knows.
//!
//! Parsing is bounded by recursion depth, an input-work budget and the shared
//! node budget. Forms this port does not implement fail with `None`, so the
//! symbol is rejected instead of printed wrongly.

use alloc::vec::Vec;

use crate::swift::node::{Accessor, Context, Global, Kind, NodeRef};

/// Maximum recursion depth (matches the modern parser's cap).
const MAX_DEPTH: u16 = 96;

/// Maximum input-work units one parse may consume.
const WORK_BUDGET: u32 = 200_000;

/// Maximum substitutions one parse may record.
const MAX_SUBS: usize = 1024;

/// Maximum length of a nested payload symbol that is demangled.
const MAX_PAYLOAD_LEN: usize = 256;

/// Maximum nesting of payload symbols (a payload may contain a payload).
const MAX_PAYLOAD_DEPTH: u8 = 4;

/// Upper bound on a builtin type's size or element count.
const MAX_BUILTIN_SIZE: u32 = 4096;

/// Standard library module name.
const STDLIB: &str = "Swift";

/// Objective-C module name.
const OBJC_MODULE: &str = "__C";

/// Clang importer module name.
const CLANG_MODULE: &str = "__C_Synthesized";

/// Parse a legacy `_T` symbol.
///
/// `text` is the symbol with the leading `_T` already removed. Returns the
/// assembled top-level node, or `None` when the symbol is not a form this
/// parser implements.
pub(crate) fn parse(text: &str, payload_depth: u8) -> Option<NodeRef<'_>> {
    let mut parser = LegacyParser {
        text: text.as_bytes(),
        pos: 0,
        context: Context::new(),
        subs: Vec::new(),
        budget: WORK_BUDGET,
        payload_depth,
    };
    let root = parser.top_level()?;
    if parser.context.failed() || !root.seal() {
        return None;
    }
    Some(root)
}

/// Recursive-descent parser for the pre-stable grammar.
struct LegacyParser<'a> {
    text: &'a [u8],
    pos: usize,
    context: Context<'a>,
    subs: Vec<NodeRef<'a>>,
    budget: u32,
    /// Nesting level of nested payload symbols (`cp`/`cl` specializations).
    payload_depth: u8,
}

impl<'a> LegacyParser<'a> {
    // ---- input ---------------------------------------------------------

    fn peek(&self) -> Option<u8> {
        self.text.get(self.pos).copied()
    }

    fn peek_at(&self, offset: usize) -> Option<u8> {
        self.text.get(self.pos.checked_add(offset)?).copied()
    }

    fn at_end(&self) -> bool {
        self.pos >= self.text.len()
    }

    /// Consume one byte, charging the work budget.
    fn take(&mut self) -> Option<u8> {
        self.budget = self.budget.checked_sub(1)?;
        let byte = self.peek()?;
        self.pos += 1;
        Some(byte)
    }

    fn eat(&mut self, byte: u8) -> bool {
        if self.peek() == Some(byte) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn eat_str(&mut self, bytes: &[u8]) -> bool {
        let end = match self.pos.checked_add(bytes.len()) {
            Some(end) => end,
            None => return false,
        };
        if self.text.get(self.pos..end) == Some(bytes) {
            self.pos = end;
            true
        } else {
            false
        }
    }

    fn has(&self, len: usize) -> bool {
        self.pos
            .checked_add(len)
            .is_some_and(|end| end <= self.text.len())
    }

    fn slice(&self, len: usize) -> Option<&'a [u8]> {
        let end = self.pos.checked_add(len)?;
        let slice = self.text.get(self.pos..end)?;
        // Identifier lengths are byte counts from the mangling, so a slice can
        // split a UTF-8 sequence; reject those rather than borrow invalid text.
        if core::str::from_utf8(slice).is_err() {
            return None;
        }
        Some(slice)
    }

    fn advance(&mut self, len: usize) {
        self.pos = self.pos.saturating_add(len);
    }

    /// Read a decimal number, saturating at [`u32::MAX`].
    ///
    /// Saturating keeps malformed names with huge counts parseable; the
    /// printer bounds what it renders, so the value never becomes work.
    fn natural(&mut self) -> Option<u32> {
        let first = self.peek()?;
        if !first.is_ascii_digit() {
            return None;
        }
        let mut value: u32 = 0;
        while let Some(byte) = self.peek() {
            if !byte.is_ascii_digit() {
                break;
            }
            value = value
                .saturating_mul(10)
                .saturating_add(u32::from(byte - b'0'));
            self.pos += 1;
        }
        Some(value)
    }

    /// An `<index>` packaged as a node of `kind`.
    fn index_node(&mut self, kind: Kind) -> Option<NodeRef<'a>> {
        let index = self.index()?;
        self.value_node(kind, index)
    }

    /// Index encoding: `_` = 0, `N_` = N + 1.
    fn index(&mut self) -> Option<u32> {
        if self.eat(b'_') {
            return Some(0);
        }
        let value = self.natural()?;
        if self.eat(b'_') {
            Some(value.saturating_add(1))
        } else {
            None
        }
    }

    // ---- nodes ---------------------------------------------------------

    fn node(&self, kind: Kind) -> Option<NodeRef<'a>> {
        if self.context.failed() {
            return None;
        }
        Some(self.context.alloc(kind))
    }

    fn text_node(&self, kind: Kind, text: &'a str) -> Option<NodeRef<'a>> {
        if self.context.failed() {
            return None;
        }
        Some(self.context.alloc_text(kind, text))
    }

    fn owned_node(&self, kind: Kind, text: &str) -> Option<NodeRef<'a>> {
        if self.context.failed() {
            return None;
        }
        Some(self.context.alloc_owned(kind, text))
    }

    fn value_node(&self, kind: Kind, value: u32) -> Option<NodeRef<'a>> {
        if self.context.failed() {
            return None;
        }
        Some(self.context.alloc_value(kind, value))
    }

    fn global_node(&self, code: Global) -> Option<NodeRef<'a>> {
        if self.context.failed() {
            return None;
        }
        Some(self.context.alloc_global(code))
    }

    fn with_children(&self, kind: Kind, children: &[NodeRef<'a>]) -> Option<NodeRef<'a>> {
        let node = self.node(kind)?;
        for child in children {
            self.context.add_child(&node, child.clone());
        }
        Some(node)
    }

    fn with_children_opt(
        &self,
        kind: Kind,
        children: &[Option<NodeRef<'a>>],
    ) -> Option<NodeRef<'a>> {
        let node = self.node(kind)?;
        for child in children.iter().flatten() {
            self.context.add_child(&node, child.clone());
        }
        Some(node)
    }

    fn add(&self, parent: &NodeRef<'a>, child: NodeRef<'a>) {
        self.context.add_child(parent, child);
    }

    fn sub_push(&mut self, node: NodeRef<'a>) {
        if self.subs.len() < MAX_SUBS {
            self.subs.push(node);
        }
    }

    fn sub(&self, index: u32) -> Option<NodeRef<'a>> {
        self.subs.get(usize::try_from(index).ok()?).cloned()
    }

    /// Wrap `inner` in a [`Kind::Type`], the old grammar's type marker.
    fn wrap_type(&self, inner: NodeRef<'a>) -> Option<NodeRef<'a>> {
        self.with_children(Kind::Type, &[inner])
    }

    fn over_depth(depth: u16) -> bool {
        depth > MAX_DEPTH
    }

    // ---- top level -----------------------------------------------------

    /// `mangled-name ::= '_T' global`, plus the old attribute prefixes.
    fn top_level(&mut self) -> Option<NodeRef<'a>> {
        let top = self.node(Kind::TopLevel)?;
        // Specialization prefixes are chained and each resets the
        // substitution table, exactly as in the reference implementation.
        if self.eat_str(b"TS") {
            loop {
                let attribute = self.specialized_attribute(0)?;
                self.add(&top, attribute);
                self.subs.clear();
                if !self.eat_str(b"_TTS") {
                    break;
                }
            }
            if !self.eat_str(b"_T") {
                return None;
            }
        } else if self.eat_str(b"To") {
            self.add(&top, self.attribute(0)?);
        } else if self.eat_str(b"TO") {
            self.add(&top, self.attribute(1)?);
        } else if self.eat_str(b"TD") {
            self.add(&top, self.attribute(2)?);
        } else if self.eat_str(b"Td") {
            // `super` in the shared printer's attribute table.
            self.add(&top, self.attribute(3)?);
        } else if self.eat_str(b"TV") {
            // `override` in the shared printer's attribute table.
            self.add(&top, self.attribute(4)?);
        }

        let global = self.global(0)?;
        self.add(&top, global);

        // Trailing text is an unmangled suffix, as the reference does.
        if !self.at_end() {
            let rest = core::str::from_utf8(&self.text[self.pos..]).ok()?;
            let suffix = self.text_node(Kind::Suffix, rest)?;
            self.add(&top, suffix);
            self.pos = self.text.len();
        }
        Some(top)
    }

    fn attribute(&self, code: u8) -> Option<NodeRef<'a>> {
        let node = self.node(Kind::Attribute)?;
        node.set_attribute_code(code);
        Some(node)
    }

    /// `specialized-attribute ::= generic-specialization | function-specialization`
    fn specialized_attribute(&mut self, depth: u16) -> Option<NodeRef<'a>> {
        if Self::over_depth(depth) {
            return None;
        }
        if self.eat(b'f') {
            return self.function_signature_specialization(depth);
        }
        let not_reabstracted = if self.eat(b'g') {
            false
        } else if self.eat(b'r') {
            true
        } else {
            return None;
        };
        // `B` and `G` select the printer's "generic specialization" and
        // "generic not re-abstracted specialization" descriptions.
        let flags = if not_reabstracted { b'G' } else { b'B' };
        let specialization = self.context.alloc(Kind::GenericSpecialization);
        specialization.set_specialization_flags(flags);
        if self.eat(b'q') {
            let serialized = self.node(Kind::IsSerialized)?;
            self.add(&specialization, serialized);
        }
        // The specialization pass id is not printed.
        let _ = self.take()?;
        self.generic_specialization(specialization, depth)
    }

    /// `function-signature-specialization ::= (param)* '_'`
    ///
    /// Payload-carrying parameters (`cp*`, `cl*`) demangle a nested symbol in
    /// the reference printer; they are rejected here rather than printed
    /// without their payload.
    fn function_signature_specialization(&mut self, depth: u16) -> Option<NodeRef<'a>> {
        if Self::over_depth(depth) {
            return None;
        }
        let specialization = self.context.alloc(Kind::FunctionSignatureSpecialization);
        if self.eat(b'q') {
            let serialized = self.node(Kind::IsSerialized)?;
            self.add(&specialization, serialized);
        }
        // The specialization pass id is not printed.
        let _ = self.take()?;
        while !self.eat(b'_') {
            let param = self.context.alloc(Kind::FuncSpecParam);
            if self.eat_str(b"n_") {
                // An empty parameter still occupies an argument slot.
            } else if self.eat_str(b"cp") {
                self.constant_propagation(&param, depth + 1)?;
            } else if self.eat_str(b"cl") {
                self.closure_propagation(&param, depth + 1)?;
            } else if self.eat_str(b"i_") {
                self.func_spec_kind(&param, 6)?;
            } else if self.eat_str(b"k_") {
                self.func_spec_kind(&param, 7)?;
            } else if self.eat_str(b"r_") {
                self.func_spec_kind(&param, 8)?;
            } else {
                // Option flags: Dead, OwnedToGuaranteed, GuaranteedToOwned,
                // Exploded. The raw value is the bit set the printer reads.
                let mut value = 0_u32;
                if self.eat(b'd') {
                    value |= 1 << 6;
                }
                if self.eat(b'g') {
                    value |= 1 << 7;
                }
                if self.eat(b'o') {
                    value |= 1 << 9;
                }
                if self.eat(b's') {
                    value |= 1 << 8;
                }
                if !self.eat(b'_') || value == 0 {
                    return None;
                }
                self.func_spec_kind(&param, value)?;
            }
            self.add(&specialization, param);
        }
        Some(specialization)
    }

    /// `constant-propagation ::= 'fr' identifier '_' | 'g' identifier '_' |
    /// 'i' <digits> '_' | 'fl' <digits> '_' |
    /// 'se' ('0'|'1') 'v' identifier '_'`
    fn constant_propagation(&mut self, param: &NodeRef<'a>, depth: u16) -> Option<()> {
        if Self::over_depth(depth) {
            return None;
        }
        if self.eat_str(b"fr") {
            let name = self.identifier(depth + 1, Some(Kind::Identifier))?;
            if !self.eat(b'_') {
                return None;
            }
            self.func_spec_kind(param, 0)?;
            let payload = self.payload_node(&name)?;
            self.add(param, payload);
            return Some(());
        }
        if self.eat(b'g') {
            let name = self.identifier(depth + 1, Some(Kind::Identifier))?;
            if !self.eat(b'_') {
                return None;
            }
            self.func_spec_kind(param, 1)?;
            let payload = self.payload_node(&name)?;
            self.add(param, payload);
            return Some(());
        }
        if self.eat(b'i') {
            let text = self.read_until_underscore()?;
            self.func_spec_kind(param, 2)?;
            let payload = self.owned_node(Kind::Identifier, text)?;
            self.add(param, payload);
            return Some(());
        }
        if self.eat_str(b"fl") {
            let text = self.read_until_underscore()?;
            self.func_spec_kind(param, 3)?;
            let payload = self.owned_node(Kind::Identifier, text)?;
            self.add(param, payload);
            return Some(());
        }
        if self.eat_str(b"se") {
            let encoding = match self.take()? {
                b'0' => "u8",
                b'1' => "u16",
                _ => return None,
            };
            if !self.eat(b'v') {
                return None;
            }
            let name = self.identifier(depth + 1, Some(Kind::Identifier))?;
            if !self.eat(b'_') {
                return None;
            }
            self.func_spec_kind(param, 4)?;
            let payload = self.owned_node(Kind::Identifier, encoding)?;
            self.add(param, payload);
            let payload = self.payload_node(&name)?;
            self.add(param, payload);
            return Some(());
        }
        None
    }

    /// `closure-propagation ::= identifier type* '_'`
    fn closure_propagation(&mut self, param: &NodeRef<'a>, depth: u16) -> Option<()> {
        if Self::over_depth(depth) {
            return None;
        }
        let name = self.identifier(depth + 1, Some(Kind::Identifier))?;
        self.func_spec_kind(param, 5)?;
        let payload = self.payload_node(&name)?;
        self.add(param, payload);
        while self.peek() != Some(b'_') {
            let ty = self.ty(depth + 1)?;
            self.add(param, ty);
        }
        if !self.eat(b'_') {
            return None;
        }
        Some(())
    }

    /// Read the payload text up to (and including) the terminating `_`.
    fn read_until_underscore(&mut self) -> Option<&'a str> {
        let start = self.pos;
        while let Some(byte) = self.peek() {
            if byte == b'_' {
                break;
            }
            self.pos += 1;
        }
        if !self.eat(b'_') {
            return None;
        }
        core::str::from_utf8(self.text.get(start..self.pos - 1)?).ok()
    }

    /// A specialization payload: demangle it when it is a symbol.
    ///
    /// The reference printer re-demangles payload text, which can itself carry
    /// a payload. Both the payload length and the nesting depth are bounded.
    fn payload_node(&self, name: &NodeRef<'a>) -> Option<NodeRef<'a>> {
        let mut rendered = None;
        name.with_text(|text| {
            let Some(text) = text else { return };
            if text.len() > MAX_PAYLOAD_LEN || self.payload_depth >= MAX_PAYLOAD_DEPTH {
                return;
            }
            rendered = crate::swift::demangle_payload(text, self.payload_depth);
        });
        match rendered {
            Some(rendered) => self.owned_node(Kind::Identifier, &rendered),
            None => {
                let mut text = None;
                name.with_text(|value| text = value.map(alloc::string::String::from));
                self.owned_node(Kind::Identifier, &text?)
            }
        }
    }

    fn func_spec_kind(&self, param: &NodeRef<'a>, value: u32) -> Option<()> {
        let kind = self.value_node(Kind::FuncSpecKind, value)?;
        self.add(param, kind);
        Some(())
    }

    /// `generic-specialization ::= (type protocol-conformance* '_')+`
    fn generic_specialization(
        &mut self,
        specialization: NodeRef<'a>,
        depth: u16,
    ) -> Option<NodeRef<'a>> {
        if Self::over_depth(depth) {
            return None;
        }
        while !self.eat(b'_') {
            let param = self.node(Kind::GenericSpecializationParam)?;
            let ty = self.ty(depth + 1)?;
            self.add(&param, ty);
            while !self.eat(b'_') {
                let conformance = self.protocol_conformance(depth + 1)?;
                self.add(&param, conformance);
            }
            self.add(&specialization, param);
        }
        Some(specialization)
    }

    // ---- globals -------------------------------------------------------

    fn global(&mut self, depth: u16) -> Option<NodeRef<'a>> {
        if Self::over_depth(depth) || self.at_end() {
            return None;
        }

        if self.eat(b'M') {
            return self.metadata(depth);
        }

        if self.eat_str(b"PA") {
            let code = if self.eat(b'o') {
                Global::PartialApplyObjcForwarder
            } else {
                Global::PartialApplyForwarder
            };
            let forwarder = self.global_node(code)?;
            if self.eat_str(b"__T") {
                let inner = self.global(depth + 1)?;
                self.add(&forwarder, inner);
            }
            return Some(forwarder);
        }

        // A top-level type, for type-name consumers.
        if self.eat(b't') {
            let ty = self.ty(depth + 1)?;
            return self.with_children(Kind::TypeMangling, &[ty]);
        }

        if self.eat(b'w') {
            let name = value_witness_name(self.peek(), self.peek_at(1))?;
            let _ = self.take()?;
            let _ = self.take()?;
            let witness = self.text_node(Kind::ValueWitness, name)?;
            let ty = self.ty(depth + 1)?;
            self.add(&witness, ty);
            return Some(witness);
        }

        if self.eat(b'W') {
            return self.witness(depth);
        }

        if self.eat(b'T') {
            return self.thunk(depth);
        }

        self.entity(depth + 1)
    }

    /// `metadata ::= 'M' ('P' type | 'a' type | 'L' type | 'm' type |
    /// 'n' type | 'f' type | 'p' protocol-name | type)`
    fn metadata(&mut self, depth: u16) -> Option<NodeRef<'a>> {
        if self.eat(b'P') {
            let node = self.global_node(Global::TypeMetadataPattern)?;
            let ty = self.ty(depth + 1)?;
            self.add(&node, ty);
            return Some(node);
        }
        if self.eat(b'a') {
            let node = self.global_node(Global::TypeMetadataAccess)?;
            let ty = self.ty(depth + 1)?;
            self.add(&node, ty);
            return Some(node);
        }
        if self.eat(b'L') {
            let node = self.global_node(Global::TypeMetadataLazyCache)?;
            let ty = self.ty(depth + 1)?;
            self.add(&node, ty);
            return Some(node);
        }
        if self.eat(b'm') {
            let node = self.global_node(Global::ClassMetaclass)?;
            let ty = self.ty(depth + 1)?;
            self.add(&node, ty);
            return Some(node);
        }
        if self.eat(b'n') {
            let node = self.global_node(Global::NominalTypeDescriptor)?;
            let ty = self.ty(depth + 1)?;
            self.add(&node, ty);
            return Some(node);
        }
        if self.eat(b'f') {
            let node = self.global_node(Global::FullTypeMetadata)?;
            let ty = self.ty(depth + 1)?;
            self.add(&node, ty);
            return Some(node);
        }
        if self.eat(b'p') {
            let node = self.global_node(Global::ProtocolDescriptor)?;
            let proto = self.protocol_name(depth + 1)?;
            self.add(&node, proto);
            return Some(node);
        }
        let node = self.global_node(Global::TypeMetadata)?;
        let ty = self.ty(depth + 1)?;
        self.add(&node, ty);
        Some(node)
    }

    /// `witness ::= 'W' ('V' type | 'v' directness entity | 'P' conformance |
    /// 'G' conformance | 'I' conformance | 'l' type conformance |
    /// 'L' type conformance | 'a' conformance | 't' conformance decl-name |
    /// 'T' conformance decl-name protocol-name)`
    fn witness(&mut self, depth: u16) -> Option<NodeRef<'a>> {
        if self.eat(b'V') {
            let node = self.global_node(Global::ValueWitnessTable)?;
            let ty = self.ty(depth + 1)?;
            self.add(&node, ty);
            return Some(node);
        }
        if self.eat(b'v') {
            let indirect = match self.peek() {
                Some(b'd') => {
                    let _ = self.take()?;
                    false
                }
                Some(b'i') => {
                    let _ = self.take()?;
                    true
                }
                _ => return None,
            };
            let node = self.global_node(Global::FieldOffset)?;
            if indirect {
                node.set_global_value(1);
            }
            let entity = self.entity(depth + 1)?;
            self.add(&node, entity);
            return Some(node);
        }
        if self.eat(b'P') {
            let node = self.global_node(Global::ProtocolWitnessTable)?;
            let conformance = self.protocol_conformance(depth + 1)?;
            self.add(&node, conformance);
            return Some(node);
        }
        if self.eat(b'G') {
            let node = self.global_node(Global::GenericProtocolWitnessTable)?;
            let conformance = self.protocol_conformance(depth + 1)?;
            self.add(&node, conformance);
            return Some(node);
        }
        if self.eat(b'I') {
            let node = self.global_node(Global::GenericWitnessTableInstantiationFunction)?;
            let conformance = self.protocol_conformance(depth + 1)?;
            self.add(&node, conformance);
            return Some(node);
        }
        if self.eat(b'l') {
            let node = self.global_node(Global::LazyProtocolWitnessTableAccessor)?;
            let ty = self.ty(depth + 1)?;
            self.add(&node, ty);
            let conformance = self.protocol_conformance(depth + 1)?;
            self.add(&node, conformance);
            return Some(node);
        }
        if self.eat(b'L') {
            let node = self.global_node(Global::LazyProtocolWitnessTableCache)?;
            let ty = self.ty(depth + 1)?;
            self.add(&node, ty);
            let conformance = self.protocol_conformance(depth + 1)?;
            self.add(&node, conformance);
            return Some(node);
        }
        if self.eat(b'a') {
            let node = self.global_node(Global::ProtocolWitnessTableAccessor)?;
            let conformance = self.protocol_conformance(depth + 1)?;
            self.add(&node, conformance);
            return Some(node);
        }
        if self.eat(b't') {
            let node = self.global_node(Global::AssociatedTypeMetadataAccessor)?;
            let conformance = self.protocol_conformance(depth + 1)?;
            self.add(&node, conformance);
            let name = self.decl_name(depth + 1)?;
            self.add(&node, name);
            return Some(node);
        }
        if self.eat(b'T') {
            let node = self.global_node(Global::AssociatedTypeWitnessTableAccessor)?;
            let conformance = self.protocol_conformance(depth + 1)?;
            self.add(&node, conformance);
            let name = self.decl_name(depth + 1)?;
            self.add(&node, name);
            let proto = self.protocol_name(depth + 1)?;
            self.add(&node, proto);
            return Some(node);
        }
        None
    }

    /// `thunk ::= 'T' ('R' reabstract-signature | 'r' reabstract-signature |
    /// 'W' protocol-conformance entity)`
    fn thunk(&mut self, depth: u16) -> Option<NodeRef<'a>> {
        if self.eat(b'R') {
            let thunk = self.global_node(Global::ReabstractionThunkHelper)?;
            self.reabstract_signature(&thunk, depth + 1)?;
            return Some(thunk);
        }
        if self.eat(b'r') {
            let thunk = self.global_node(Global::ReabstractionThunk)?;
            self.reabstract_signature(&thunk, depth + 1)?;
            return Some(thunk);
        }
        if self.eat(b'W') {
            // The entity is mangled in its own generic context.
            let thunk = self.global_node(Global::ProtocolWitnessThunk)?;
            let conformance = self.protocol_conformance(depth + 1)?;
            self.add(&thunk, conformance);
            let entity = self.entity(depth + 1)?;
            self.add(&thunk, entity);
            return Some(thunk);
        }
        None
    }

    /// `reabstract-signature ::= generic-signature? type type`
    ///
    /// The printer renders the second type as the source and the first as the
    /// destination, matching the reference node order.
    fn reabstract_signature(&mut self, signature: &NodeRef<'a>, depth: u16) -> Option<()> {
        if self.eat(b'G') {
            let generics = self.generic_signature(depth + 1)?;
            self.add(signature, generics);
        }
        let src = self.ty(depth + 1)?;
        self.add(signature, src);
        let dest = self.ty(depth + 1)?;
        self.add(signature, dest);
        Some(())
    }

    // ---- entities ------------------------------------------------------

    /// `entity ::= 'Z'? (entity-kind context entity-name | nominal-type)`
    fn entity(&mut self, depth: u16) -> Option<NodeRef<'a>> {
        if Self::over_depth(depth) {
            return None;
        }
        let is_static = self.eat(b'Z');

        let basic = if self.eat(b'F') {
            BasicEntity::Function
        } else if self.eat(b'v') {
            BasicEntity::Variable
        } else if self.eat(b'I') {
            BasicEntity::Initializer
        } else if self.eat(b'i') {
            BasicEntity::Subscript
        } else {
            return self.nominal_type(depth + 1);
        };

        let context = self.context(depth + 1)?;

        // The accessor kinds wrap the storage entity the printer renders.
        let accessor = if basic == BasicEntity::Function {
            self.accessor_kind()
        } else {
            None
        };
        if let Some(accessor) = accessor {
            let name = self.decl_name(depth + 1)?;
            let entity = self.storage_entity(accessor, context, name, depth + 1)?;
            return self.finish_entity(entity, is_static);
        }

        let mut has_type = true;
        let mut name = None;
        // Deinitializer variants share one node kind and differ by code.
        let mut deinitializer_code = None;
        let kind = if basic == BasicEntity::Initializer {
            if self.eat(b'A') {
                let index = self.index()?;
                let entity = self.node(Kind::DefaultArgumentInitializer)?;
                entity.set_entity_index(index);
                self.add(&entity, context);
                return self.finish_entity(entity, is_static);
            }
            if !self.eat(b'i') {
                return None;
            }
            has_type = false;
            Kind::Initializer
        } else if let Some(code) = self.deinitializer_kind() {
            has_type = false;
            deinitializer_code = Some(code);
            Kind::Deinitializer
        } else if self.eat(b'e') {
            has_type = false;
            Kind::IVarInitializer
        } else if self.eat(b'E') {
            has_type = false;
            Kind::IVarDestroyer
        } else if self.eat(b'C') {
            Kind::Allocator
        } else if self.eat(b'c') {
            Kind::Constructor
        } else if let Some(kind) = self.closure_kind() {
            let index = self.index()?;
            let entity = self.node(kind)?;
            entity.set_entity_index(index);
            self.add(&entity, context);
            let ty = self.ty(depth + 1)?;
            self.add(&entity, ty);
            return self.finish_entity(entity, is_static);
        } else {
            name = Some(self.decl_name(depth + 1)?);
            match basic {
                BasicEntity::Function => Kind::Function,
                BasicEntity::Variable => Kind::Variable,
                BasicEntity::Subscript => Kind::Subscript,
                BasicEntity::Initializer => unreachable!(),
            }
        };

        let entity = self.context.alloc(kind);
        if let Some(code) = deinitializer_code {
            entity.set_deinitializer_code(code);
        }
        self.add(&entity, context);
        if let Some(name) = name {
            self.add(&entity, name);
        }
        if has_type {
            let ty = self.ty(depth + 1)?;
            self.add(&entity, ty);
        }
        self.finish_entity(entity, is_static)
    }

    /// Consume an explicit (`U`) or implicit (`u`) closure marker.
    fn closure_kind(&mut self) -> Option<Kind> {
        let kind = match self.peek()? {
            b'U' => Kind::ExplicitClosure,
            b'u' => Kind::ImplicitClosure,
            _ => return None,
        };
        let _ = self.take()?;
        Some(kind)
    }

    /// Consume a deinitializer name and return its printer code: `deinit`,
    /// `__deallocating_deinit` or `__isolated_deallocating_deinit`.
    fn deinitializer_kind(&mut self) -> Option<u8> {
        let code = match self.peek()? {
            b'd' => 0,
            b'D' => 1,
            b'Z' => 2,
            _ => return None,
        };
        let _ = self.take()?;
        Some(code)
    }

    fn finish_entity(&self, entity: NodeRef<'a>, is_static: bool) -> Option<NodeRef<'a>> {
        if is_static {
            let node = self.node(Kind::Static)?;
            self.add(&node, entity);
            Some(node)
        } else {
            Some(entity)
        }
    }

    /// Old accessor letters, mapped onto the shared printer's accessor codes.
    fn accessor_kind(&mut self) -> Option<Accessor> {
        let byte = self.peek()?;
        let accessor = match byte {
            b'g' => Accessor::Getter,
            b'G' => Accessor::GlobalGetter,
            b's' => Accessor::Setter,
            b'm' => Accessor::MaterializeForSet,
            b'w' => Accessor::WillSet,
            b'W' => Accessor::DidSet,
            b'r' => Accessor::Read,
            b'M' => Accessor::Modify,
            b'a' => {
                let _ = self.take()?;
                return match self.take()? {
                    b'O' => Some(Accessor::OwningMutableAddressor),
                    b'o' => Some(Accessor::NativeOwningMutableAddressor),
                    b'p' => Some(Accessor::NativePinningMutableAddressor),
                    b'u' => Some(Accessor::MutableAddressor),
                    _ => None,
                };
            }
            b'l' => {
                let _ = self.take()?;
                return match self.take()? {
                    b'O' => Some(Accessor::OwningAddressor),
                    b'o' => Some(Accessor::NativeOwningAddressor),
                    b'p' => Some(Accessor::NativePinningAddressor),
                    b'u' => Some(Accessor::NonMutableAddressor),
                    _ => None,
                };
            }
            _ => return None,
        };
        let _ = self.take()?;
        Some(accessor)
    }

    /// Build the storage entity an accessor refers to.
    ///
    /// The old grammar nests a `Variable`/`Subscript` under the accessor; the
    /// shared printer instead carries the accessor code on the storage node.
    fn storage_entity(
        &mut self,
        accessor: Accessor,
        context: NodeRef<'a>,
        name: NodeRef<'a>,
        depth: u16,
    ) -> Option<NodeRef<'a>> {
        // Old subscripts are named `subscript`; the printer emits that word
        // itself and expects no name child.
        let is_subscript = self.context.kind(&name) == Kind::Identifier
            && name.text_eq("subscript");
        let node = self.context.alloc(if is_subscript {
            Kind::Subscript
        } else {
            Kind::Variable
        });
        node.set_accessor_code(accessor);
        self.add(&node, context);
        if !is_subscript {
            self.add(&node, name);
        }
        let ty = self.ty(depth + 1)?;
        self.add(&node, ty);
        Some(node)
    }

    // ---- contexts and names --------------------------------------------

    /// `context ::= module | entity | 'E' module context |
    /// 'e' module generic-signature context | 'S' substitution | 'G'
    /// bound-generic-type`
    fn context(&mut self, depth: u16) -> Option<NodeRef<'a>> {
        if Self::over_depth(depth) || self.at_end() {
            return None;
        }
        if self.eat(b'E') {
            let ext = self.node(Kind::Extension)?;
            let module = self.module(depth + 1)?;
            self.add(&ext, module);
            let ty = self.context(depth + 1)?;
            self.add(&ext, ty);
            return Some(ext);
        }
        if self.eat(b'e') {
            let ext = self.node(Kind::Extension)?;
            let module = self.module(depth + 1)?;
            let signature = self.generic_signature(depth + 1)?;
            let ty = self.context(depth + 1)?;
            self.add(&ext, module);
            self.add(&ext, ty);
            self.add(&ext, signature);
            return Some(ext);
        }
        if self.eat(b'S') {
            return self.substitution_index(depth + 1);
        }
        if self.eat(b's') {
            return self.text_node(Kind::Module, STDLIB);
        }
        if self.eat(b'G') {
            return self.bound_generic_type(depth + 1);
        }
        if self.peek().is_some_and(is_start_of_entity) {
            return self.entity(depth + 1);
        }
        self.module(depth + 1)
    }

    /// `module ::= 's' | 'S' substitution | identifier`
    fn module(&mut self, depth: u16) -> Option<NodeRef<'a>> {
        if self.eat(b's') {
            return self.text_node(Kind::Module, STDLIB);
        }
        if self.eat(b'S') {
            let module = self.substitution_index(depth + 1)?;
            if self.context.kind(&module) != Kind::Module {
                return None;
            }
            return Some(module);
        }
        let module = self.identifier(depth + 1, Some(Kind::Module))?;
        self.sub_push(module.clone());
        Some(module)
    }

    /// `decl-name ::= local-decl-name | private-decl-name | identifier`
    fn decl_name(&mut self, depth: u16) -> Option<NodeRef<'a>> {
        if self.eat(b'L') {
            let discriminator = self.index_node(Kind::Number)?;
            let name = self.identifier(depth + 1, Some(Kind::Identifier))?;
            return self.with_children(Kind::LocalDeclName, &[discriminator, name]);
        }
        if self.eat(b'P') {
            let discriminator = self.identifier(depth + 1, Some(Kind::Identifier))?;
            let name = self.identifier(depth + 1, Some(Kind::Identifier))?;
            return self.with_children(Kind::PrivateDeclName, &[discriminator, name]);
        }
        self.identifier(depth + 1, Some(Kind::Identifier))
    }

    /// `identifier ::= 'X'? 'o' operator-mode? <length> <text>`
    fn identifier(&mut self, depth: u16, kind: Option<Kind>) -> Option<NodeRef<'a>> {
        if Self::over_depth(depth) || self.at_end() {
            return None;
        }
        let punycoded = self.eat(b'X');
        let mut kind = kind;
        let mut is_operator = false;
        if self.eat(b'o') {
            is_operator = true;
            // Operator identifiers only exist at the leaves of a name.
            if kind != Some(Kind::Identifier) {
                return None;
            }
            kind = Some(match self.take()? {
                b'p' => Kind::PrefixOperator,
                b'P' => Kind::PostfixOperator,
                b'i' => Kind::InfixOperator,
                _ => return None,
            });
        }
        let kind = kind?;
        let length = usize::try_from(self.natural()?).ok()?;
        if !self.has(length) {
            return None;
        }
        let slice = self.slice(length)?;
        self.advance(length);
        if !punycoded && !is_operator {
            let text = core::str::from_utf8(slice).ok()?;
            if text.is_empty() {
                return None;
            }
            return self.text_node(kind, text);
        }
        let mut decoded = [0_u8; 512];
        let text = if punycoded {
            let len = crate::swift::parser::punycode_decode(slice, &mut decoded)?;
            &decoded[..len]
        } else {
            decoded.get_mut(..slice.len())?.copy_from_slice(slice);
            &decoded[..slice.len()]
        };
        if text.is_empty() {
            return None;
        }
        if is_operator {
            let mut decoded_operator = [0_u8; 512];
            if text.len() > decoded_operator.len() {
                return None;
            }
            for (index, byte) in text.iter().copied().enumerate() {
                let symbol = operator_symbol(byte)?;
                decoded_operator[index] = symbol;
            }
            let text = core::str::from_utf8(&decoded_operator[..text.len()]).ok()?;
            return self.owned_node(kind, text);
        }
        let text = core::str::from_utf8(text).ok()?;
        self.owned_node(kind, text)
    }

    /// `substitution ::= 'S' ('o' | 'C' | <stdlib-code> | index)`
    fn substitution_index(&mut self, depth: u16) -> Option<NodeRef<'a>> {
        if Self::over_depth(depth) || self.at_end() {
            return None;
        }
        let code = self.take()?;
        if let Some(node) = standard_substitution(code) {
            return match node {
                StandardSubstitution::Module(text) => self.text_node(Kind::Module, text),
                StandardSubstitution::Type(kind, name) => self.swift_type(kind, name),
            };
        }
        let index = if code == b'_' {
            0
        } else if code.is_ascii_digit() {
            let mut value = u32::from(code - b'0');
            while let Some(byte) = self.peek() {
                if !byte.is_ascii_digit() {
                    break;
                }
                value = value
                    .saturating_mul(10)
                    .saturating_add(u32::from(byte - b'0'));
                self.pos += 1;
            }
            if !self.eat(b'_') {
                return None;
            }
            value.saturating_add(1)
        } else {
            return None;
        };
        self.sub(index)
    }

    fn swift_type(&self, kind: Kind, name: &'static str) -> Option<NodeRef<'a>> {
        let module = self.text_node(Kind::Module, STDLIB)?;
        let ident = self.text_node(Kind::Identifier, name)?;
        self.with_children(kind, &[module, ident])
    }

    /// `protocol-name ::= 'S' substitution | 's' decl-name | decl-name`
    fn protocol_name(&mut self, depth: u16) -> Option<NodeRef<'a>> {
        let proto = self.protocol_name_impl(depth)?;
        self.wrap_type(proto)
    }

    fn protocol_name_impl(&mut self, depth: u16) -> Option<NodeRef<'a>> {
        if Self::over_depth(depth) {
            return None;
        }
        if self.eat(b'S') {
            let sub = self.substitution_index(depth + 1)?;
            if self.context.kind(&sub) == Kind::Protocol {
                return Some(sub);
            }
            if self.context.kind(&sub) != Kind::Module {
                return None;
            }
            return self.protocol_name_given_context(sub, depth + 1);
        }
        if self.eat(b's') {
            let module = self.text_node(Kind::Module, STDLIB)?;
            return self.protocol_name_given_context(module, depth + 1);
        }
        self.declaration_name(Kind::Protocol, depth + 1)
    }

    fn protocol_name_given_context(
        &mut self,
        context: NodeRef<'a>,
        depth: u16,
    ) -> Option<NodeRef<'a>> {
        let name = self.decl_name(depth + 1)?;
        let proto = self.with_children(Kind::Protocol, &[context, name])?;
        self.sub_push(proto.clone());
        Some(proto)
    }

    /// `nominal-type ::= 'S' substitution | ('V'|'O'|'C'|'P') decl-name`
    fn nominal_type(&mut self, depth: u16) -> Option<NodeRef<'a>> {
        if self.eat(b'S') {
            return self.substitution_index(depth + 1);
        }
        let kind = match self.peek()? {
            b'V' => Kind::Structure,
            b'O' => Kind::Enum,
            b'C' => Kind::Class,
            b'P' => Kind::Protocol,
            _ => return None,
        };
        let _ = self.take()?;
        self.declaration_name(kind, depth + 1)
    }

    fn declaration_name(&mut self, kind: Kind, depth: u16) -> Option<NodeRef<'a>> {
        let context = self.context(depth + 1)?;
        let name = self.decl_name(depth + 1)?;
        let decl = self.with_children(kind, &[context, name])?;
        self.sub_push(decl.clone());
        Some(decl)
    }

    /// `bound-generic-type ::= 'G' nominal-type (args '_')+`
    fn bound_generic_type(&mut self, depth: u16) -> Option<NodeRef<'a>> {
        let nominal = self.nominal_type(depth + 1)?;
        self.bound_generic_args(nominal, depth + 1)
    }

    fn bound_generic_args(
        &mut self,
        mut nominal: NodeRef<'a>,
        depth: u16,
    ) -> Option<NodeRef<'a>> {
        if Self::over_depth(depth) || self.context.child_count(&nominal) == 0 {
            return None;
        }
        // Generic arguments for the outermost type come first.
        let parent = self.context.child(&nominal, 0)?;
        let parent_kind = self.context.kind(&parent);
        if !matches!(parent_kind, Kind::Module | Kind::Function | Kind::Extension) {
            let rebuilt_parent = self.bound_generic_args(parent, depth + 1)?;
            let kind = self.context.kind(&nominal);
            let rebuilt = self.node(kind)?;
            self.add(&rebuilt, rebuilt_parent);
            for index in 1..self.context.child_count(&nominal) {
                let child = self.context.child(&nominal, index)?;
                self.add(&rebuilt, child);
            }
            nominal = rebuilt;
        }

        let args = self.node(Kind::TypeList)?;
        while !self.eat(b'_') {
            let ty = self.ty(depth + 1)?;
            self.add(&args, ty);
            if self.at_end() {
                return None;
            }
        }
        if self.context.child_count(&args) == 0 {
            return Some(nominal);
        }
        let kind = match self.context.kind(&nominal) {
            Kind::Class => Kind::BoundGenericClass,
            Kind::Structure => Kind::BoundGenericStructure,
            Kind::Enum => Kind::BoundGenericEnum,
            _ => return None,
        };
        // The shared printer expects the unbound nominal as the first child
        // and the argument list as the second.
        self.with_children(kind, &[nominal, args])
    }

    /// `protocol-conformance ::= type protocol-name context`
    fn protocol_conformance(&mut self, depth: u16) -> Option<NodeRef<'a>> {
        let ty = self.ty(depth + 1)?;
        let proto = self.protocol_name(depth + 1)?;
        let context = self.context(depth + 1)?;
        self.with_children(Kind::ProtocolConformance, &[ty, proto, context])
    }

    // ---- types ---------------------------------------------------------

    /// `type ::= type-impl` (wrapped in [`Kind::Type`])
    fn ty(&mut self, depth: u16) -> Option<NodeRef<'a>> {
        let inner = self.type_impl(depth)?;
        self.wrap_type(inner)
    }

    fn type_impl(&mut self, depth: u16) -> Option<NodeRef<'a>> {
        if Self::over_depth(depth) || self.at_end() {
            return None;
        }
        match self.take()? {
            b'B' => self.builtin_type(),
            b'a' => self.declaration_name(Kind::TypeAlias, depth + 1),
            b'b' => self.function_type(Kind::ObjCBlock, depth + 1),
            b'c' => self.function_type(Kind::CFunctionPointer, depth + 1),
            b'D' => {
                let ty = self.ty(depth + 1)?;
                self.with_children(Kind::DynamicSelf, &[ty])
            }
            b'E' => {
                if !self.eat(b'R') || !self.eat(b'R') {
                    return None;
                }
                self.node(Kind::ErrorType)
            }
            b'F' => self.function_type(Kind::FunctionType, depth + 1),
            // The old uncurried function type prints like a function type.
            b'f' => self.function_type(Kind::FunctionType, depth + 1),
            b'G' => self.bound_generic_type(depth + 1),
            b'K' => self.function_type(Kind::AutoClosureType, depth + 1),
            b'M' => {
                let ty = self.ty(depth + 1)?;
                self.with_children(Kind::Metatype, &[ty])
            }
            b'P' => {
                if self.eat(b'M') {
                    let ty = self.ty(depth + 1)?;
                    return self.with_children(Kind::ExistentialMetatype, &[ty]);
                }
                self.protocol_list(depth + 1)
            }
            b'Q' => {
                if self.eat(b'u') {
                    return self.node(Kind::OpaqueReturnType);
                }
                if self.eat(b'U') {
                    let index = self.index()?;
                    let node = self.node(Kind::OpaqueReturnType)?;
                    let ordinal = self.value_node(Kind::OpaqueReturnTypeIndex, index)?;
                    self.add(&node, ordinal);
                    return Some(node);
                }
                // Archetypes are not implemented.
                None
            }
            b'q' => self.dependent_type(depth + 1),
            b'x' => self.generic_param(0, 0),
            b'w' => self.associated_type_simple(depth + 1),
            b'W' => self.associated_type_compound(depth + 1),
            b'R' => {
                let inner = self.type_impl(depth + 1)?;
                self.with_children(Kind::InOut, &[inner])
            }
            b'k' => {
                let inner = self.type_impl(depth + 1)?;
                self.with_children(Kind::NoDerivative, &[inner])
            }
            b'S' => self.substitution_index(depth + 1),
            b'T' => self.tuple_type(false, depth + 1),
            b't' => self.tuple_type(true, depth + 1),
            b'u' => {
                let signature = self.generic_signature(depth + 1)?;
                let inner = self.ty(depth + 1)?;
                self.with_children(Kind::DependentGenericType, &[signature, inner])
            }
            b'X' => self.special_type(depth),
            byte if matches!(byte, b'C' | b'V' | b'O') => {
                let kind = match byte {
                    b'C' => Kind::Class,
                    b'V' => Kind::Structure,
                    _ => Kind::Enum,
                };
                self.declaration_name(kind, depth + 1)
            }
            _ => None,
        }
    }

    /// `X`-introduced types.
    ///
    /// `Xb`/`XB` (SIL box types), `XM`/`XP` metatype representations and `Xu`
    /// (unmanaged) are not implemented; returning `None` rejects the symbol
    /// rather than printing it without the modifier.
    fn special_type(&mut self, depth: u16) -> Option<NodeRef<'a>> {
        if self.eat(b'f') {
            return self.function_type(Kind::ThinFunctionType, depth + 1);
        }
        if self.eat(b'o') {
            let inner = self.ty(depth + 1)?;
            return self.with_children(Kind::Unowned, &[inner]);
        }
        if self.eat(b'w') {
            let inner = self.ty(depth + 1)?;
            return self.with_children(Kind::Weak, &[inner]);
        }
        if self.eat(b'F') {
            return self.impl_function_type(depth + 1);
        }
        None
    }

    fn builtin_type(&mut self) -> Option<NodeRef<'a>> {
        match self.take()? {
            b'b' => self.text_node(Kind::BuiltinTypeName, "Builtin.BridgeObject"),
            b'B' => self.text_node(Kind::BuiltinTypeName, "Builtin.UnsafeValueBuffer"),
            b'f' => {
                let size = self.builtin_size()?;
                let mut buf = [0_u8; 32];
                let text = format_builtin(&mut buf, "Builtin.FPIEEE", size)?;
                self.owned_node(Kind::BuiltinTypeName, text)
            }
            b'i' => {
                let size = self.builtin_size()?;
                let mut buf = [0_u8; 32];
                let text = format_builtin(&mut buf, "Builtin.Int", size)?;
                self.owned_node(Kind::BuiltinTypeName, text)
            }
            b'v' => {
                let count = self.natural()?;
                if count == 0 || count > MAX_BUILTIN_SIZE {
                    return None;
                }
                if !self.eat(b'B') {
                    return None;
                }
                let mut buf = [0_u8; 48];
                let prefix = format_builtin(&mut buf, "Builtin.Vec", count)?;
                let mut text = alloc::string::String::from(prefix);
                match self.take()? {
                    b'i' => {
                        let size = self.builtin_size()?;
                        let mut tail = [0_u8; 32];
                        text.push('x');
                        text.push_str(format_builtin(&mut tail, "Int", size)?);
                    }
                    b'f' => {
                        let size = self.builtin_size()?;
                        let mut tail = [0_u8; 32];
                        text.push('x');
                        text.push_str(format_builtin(&mut tail, "FPIEEE", size)?);
                    }
                    b'p' => text.push_str("xRawPointer"),
                    _ => return None,
                }
                self.owned_node(Kind::BuiltinTypeName, &text)
            }
            b'O' => self.text_node(Kind::BuiltinTypeName, "Builtin.UnknownObject"),
            b'o' => self.text_node(Kind::BuiltinTypeName, "Builtin.NativeObject"),
            b'p' => self.text_node(Kind::BuiltinTypeName, "Builtin.RawPointer"),
            b't' => self.text_node(Kind::BuiltinTypeName, "Builtin.SILToken"),
            b'w' => self.text_node(Kind::BuiltinTypeName, "Builtin.Word"),
            _ => None,
        }
    }

    fn builtin_size(&mut self) -> Option<u32> {
        let size = self.natural()?;
        if !self.eat(b'_') {
            return None;
        }
        if size == 0 || size > MAX_BUILTIN_SIZE {
            return None;
        }
        Some(size)
    }

    /// `function-type ::= effects type type`
    fn function_type(&mut self, kind: Kind, depth: u16) -> Option<NodeRef<'a>> {
        let mut throws = false;
        let mut concurrent = false;
        let mut is_async = false;
        if !self.at_end() {
            throws = self.eat(b'z');
            concurrent = self.eat(b'y');
            is_async = self.eat(b'Z');
            if self.eat(b'D') {
                // Differentiability kinds are not implemented.
                return None;
            }
            if self.eat(b'Y') {
                // Global-actor function types are not implemented.
                return None;
            }
        }
        let args = self.ty(depth + 1)?;
        let result = self.ty(depth + 1)?;
        let block = self.node(kind)?;
        if throws {
            let node = self.node(Kind::ThrowsAnnotation)?;
            self.add(&block, node);
        }
        if is_async {
            let node = self.node(Kind::AsyncAnnotation)?;
            self.add(&block, node);
        }
        if concurrent {
            let node = self.node(Kind::ConcurrentFunctionType)?;
            self.add(&block, node);
        }
        let arguments = self.with_children(Kind::ArgumentTuple, &[args])?;
        self.add(&block, arguments);
        let returns = self.with_children(Kind::ReturnType, &[result])?;
        self.add(&block, returns);
        Some(block)
    }

    /// `impl-function-type ::= impl-callee-convention impl-function-attribute*
    /// generic-signature? '_' impl-parameter* '_' impl-result* '_'`
    fn impl_function_type(&mut self, depth: u16) -> Option<NodeRef<'a>> {
        if Self::over_depth(depth) {
            return None;
        }
        let node = self.node(Kind::ImplFunctionType)?;
        self.impl_callee_convention(&node)?;
        if self.eat(b'C') {
            let convention = self.context.alloc(Kind::ImplFunctionConvention);
            let flag = match self.take()? {
                b'b' => b'B',
                b'c' => b'C',
                b'm' => b'M',
                b'O' => b'O',
                b'w' => b'W',
                _ => return None,
            };
            convention.set_impl_flag(flag);
            self.add(&node, convention);
        }
        if self.eat(b'h') {
            let attribute = self.node(Kind::ImplSendable)?;
            self.add(&node, attribute);
        }
        if self.eat(b'H') {
            let attribute = self.node(Kind::ImplAsync)?;
            self.add(&node, attribute);
        }
        if self.eat(b'G') || self.eat(b'g') {
            let signature = self.generic_signature(depth + 1)?;
            self.add(&node, signature);
        }
        if !self.eat(b'_') {
            return None;
        }
        while !self.eat(b'_') {
            let parameter = self.impl_parameter(Kind::ImplParameter, depth + 1)?;
            self.add(&node, parameter);
        }
        while !self.eat(b'_') {
            let result = self.impl_parameter(Kind::ImplResult, depth + 1)?;
            self.add(&node, result);
        }
        Some(node)
    }

    fn impl_callee_convention(&mut self, node: &NodeRef<'a>) -> Option<()> {
        let convention = self.context.alloc(Kind::ImplCalleeConvention);
        if self.eat(b't') {
            convention.set_impl_flag(b't');
        } else {
            let flag = self.impl_convention(CalleeContext::Callee)?;
            convention.set_impl_flag(flag);
        }
        self.add(node, convention);
        Some(())
    }

    fn impl_parameter(&mut self, mut kind: Kind, depth: u16) -> Option<NodeRef<'a>> {
        if self.eat(b'z') {
            if kind != Kind::ImplResult {
                return None;
            }
            kind = Kind::ImplErrorResult;
        }
        let context = if kind == Kind::ImplParameter {
            CalleeContext::Parameter
        } else {
            CalleeContext::Result
        };
        let flag = self.impl_convention(context)?;
        let ty = self.ty(depth + 1)?;
        let convention = self.context.alloc(Kind::ImplConvention);
        convention.set_impl_flag(flag);
        self.with_children(kind, &[convention, ty])
    }

    /// Map an old convention letter onto the printer's impl-convention code.
    fn impl_convention(&mut self, context: CalleeContext) -> Option<u8> {
        let byte = self.take()?;
        let flag = match (byte, context) {
            (b't', CalleeContext::Callee) => b't',
            (b'a', CalleeContext::Result) => b'a',
            (b'd', _) => b'y',
            (b'D', CalleeContext::Result) => b'u',
            (b'g', _) => b'g',
            (b'e', CalleeContext::Parameter) => b'e',
            (b'i', CalleeContext::Parameter) => b'i',
            (b'i', CalleeContext::Result) => b'r',
            (b'l', CalleeContext::Parameter) => b'l',
            (b'o', _) => b'x',
            _ => return None,
        };
        Some(flag)
    }

    /// `generic-signature ::= (count)* ('r' | 'R' requirement* 'r')`
    fn generic_signature(&mut self, depth: u16) -> Option<NodeRef<'a>> {
        if Self::over_depth(depth) {
            return None;
        }
        let signature = self.node(Kind::DependentGenericSignature)?;
        let mut count = None;
        while !matches!(self.peek(), Some(b'R' | b'r')) {
            let value = if self.eat(b'z') {
                0
            } else {
                self.index()?.saturating_add(1)
            };
            let node = self.value_node(Kind::DependentGenericParamCount, value)?;
            self.add(&signature, node);
            count = Some(value);
        }
        if count.is_none() {
            let node = self.value_node(Kind::DependentGenericParamCount, 1)?;
            self.add(&signature, node);
        }
        if self.eat(b'r') {
            return Some(signature);
        }
        if !self.eat(b'R') {
            return None;
        }
        while !self.eat(b'r') {
            let requirement = self.generic_requirement(depth + 1)?;
            self.add(&signature, requirement);
        }
        Some(signature)
    }

    fn generic_requirement(&mut self, depth: u16) -> Option<NodeRef<'a>> {
        let constrained = self.constrained_type(depth + 1)?;
        if self.eat(b'z') {
            let other = self.ty(depth + 1)?;
            return self.with_children(Kind::DependentGenericSameTypeRequirement, &[constrained, other]);
        }
        if self.eat(b'l') {
            return self.layout_requirement(constrained, depth + 1);
        }
        let constraint = match self.peek()? {
            b'C' => self.ty(depth + 1)?,
            b'S' => {
                let _ = self.take()?;
                let sub = self.substitution_index(depth + 1)?;
                let type_name = match self.context.kind(&sub) {
                    Kind::Protocol | Kind::Class => sub,
                    Kind::Module => self.protocol_name_given_context(sub, depth + 1)?,
                    _ => return None,
                };
                self.wrap_type(type_name)?
            }
            _ => self.protocol_name(depth + 1)?,
        };
        self.with_children(
            Kind::DependentGenericConformanceRequirement,
            &[constrained, constraint],
        )
    }

    fn layout_requirement(
        &mut self,
        constrained: NodeRef<'a>,
        depth: u16,
    ) -> Option<NodeRef<'a>> {
        if Self::over_depth(depth) {
            return None;
        }
        let code = self.take()?;
        let (size, alignment) = match code {
            b'U' | b'R' | b'N' | b'T' | b'B' => (None, None),
            b'E' | b'M' => {
                let size = self.natural()?;
                if !self.eat(b'_') {
                    return None;
                }
                let alignment = self.natural()?;
                (Some(size), Some(alignment))
            }
            b'e' | b'm' => (Some(self.natural()?), None),
            _ => return None,
        };
        let node = self.context.alloc(Kind::DependentGenericLayoutRequirement);
        node.set_layout_code(code);
        self.add(&node, constrained);
        if let Some(size) = size {
            let size = self.value_node(Kind::Number, size)?;
            self.add(&node, size);
        }
        if let Some(alignment) = alignment {
            let alignment = self.value_node(Kind::Number, alignment)?;
            self.add(&node, alignment);
        }
        Some(node)
    }

    /// `constrained-type ::= generic-param | 'w' associated-type |
    /// 'W' associated-type-chain`
    fn constrained_type(&mut self, depth: u16) -> Option<NodeRef<'a>> {
        let inner = if self.eat(b'w') {
            self.associated_type_simple(depth + 1)?
        } else if self.eat(b'W') {
            self.associated_type_compound(depth + 1)?
        } else {
            self.generic_param_index(depth + 1)?
        };
        self.wrap_type(inner)
    }

    /// `generic-param ::= 'd' index index | 'x' | index`
    fn generic_param_index(&mut self, depth: u16) -> Option<NodeRef<'a>> {
        if Self::over_depth(depth) {
            return None;
        }
        if self.eat(b'd') {
            let param_depth = self.index()?.saturating_add(1);
            let index = self.index()?;
            return self.generic_param(param_depth, index);
        }
        if self.eat(b'x') {
            return self.generic_param(0, 0);
        }
        let index = self.index()?.saturating_add(1);
        self.generic_param(0, index)
    }

    fn generic_param(&self, depth: u32, index: u32) -> Option<NodeRef<'a>> {
        let node = self.node(Kind::DependentGenericParamType)?;
        node.set_number_value(depth);
        node.set_number_len(index);
        Some(node)
    }

    fn dependent_type(&mut self, depth: u16) -> Option<NodeRef<'a>> {
        if Self::over_depth(depth) || self.at_end() {
            return None;
        }
        let byte = self.peek()?;
        if byte != b'd' && byte != b'_' && !byte.is_ascii_digit() {
            let base = self.ty(depth + 1)?;
            return self.dependent_member_type_name(base, depth + 1);
        }
        self.generic_param_index(depth + 1)
    }

    fn dependent_member_type_name(
        &mut self,
        base: NodeRef<'a>,
        depth: u16,
    ) -> Option<NodeRef<'a>> {
        let assoc = if self.eat(b'S') {
            let assoc = self.substitution_index(depth + 1)?;
            if self.context.kind(&assoc) != Kind::DependentAssociatedTypeRef {
                return None;
            }
            assoc
        } else {
            let protocol = if self.eat(b'P') {
                Some(self.protocol_name(depth + 1)?)
            } else {
                None
            };
            let name = self.identifier(depth + 1, Some(Kind::Identifier))?;
            let assoc = self.with_children_opt(
                Kind::DependentAssociatedTypeRef,
                &[Some(name), protocol],
            )?;
            self.sub_push(assoc.clone());
            assoc
        };
        self.with_children(Kind::DependentMemberType, &[base, assoc])
    }

    fn associated_type_simple(&mut self, depth: u16) -> Option<NodeRef<'a>> {
        let base = self.generic_param_index(depth + 1)?;
        let base = self.wrap_type(base)?;
        self.dependent_member_type_name(base, depth + 1)
    }

    fn associated_type_compound(&mut self, depth: u16) -> Option<NodeRef<'a>> {
        let mut base = self.generic_param_index(depth + 1)?;
        while !self.eat(b'_') {
            let node_type = self.wrap_type(base)?;
            base = self.dependent_member_type_name(node_type, depth + 1)?;
        }
        Some(base)
    }

    /// `protocol-list ::= 'P' protocol-name* '_'`
    ///
    /// Built as [`Kind::ProtocolListType`] so the shared printer renders
    /// `Any` for the empty list and `A & B` otherwise.
    fn protocol_list(&mut self, depth: u16) -> Option<NodeRef<'a>> {
        let list = self.node(Kind::TypeList)?;
        while !self.eat(b'_') {
            let proto = self.protocol_name(depth + 1)?;
            self.add(&list, proto);
        }
        self.with_children(Kind::ProtocolListType, &[list])
    }

    /// `tuple ::= 'T' tuple-element* '_'` (variadic with `'t'`)
    fn tuple_type(&mut self, variadic: bool, depth: u16) -> Option<NodeRef<'a>> {
        let tuple = self.node(Kind::Tuple)?;
        let mut last = None;
        while !self.eat(b'_') {
            if self.at_end() {
                return None;
            }
            let element = self.node(Kind::TupleElement)?;
            if self.peek().is_some_and(is_start_of_identifier) {
                let label = self.identifier(depth + 1, Some(Kind::TupleElementName))?;
                self.add(&element, label);
            }
            let ty = self.ty(depth + 1)?;
            self.add(&element, ty);
            self.add(&tuple, element.clone());
            last = Some(element);
        }
        if variadic && let Some(element) = last {
            // The printer renders the variadic marker only as the first child.
            let children: Vec<_> = element.children().iter().cloned().collect();
            let rebuilt = self.node(Kind::TupleElement)?;
            let marker = self.node(Kind::VariadicMarker)?;
            self.add(&rebuilt, marker);
            for child in children {
                self.add(&rebuilt, child);
            }
            element.replace_children(&rebuilt);
        }
        Some(tuple)
    }
}

/// Which convention position an impl-function entry occupies.
#[derive(Clone, Copy, PartialEq, Eq)]
enum CalleeContext {
    Callee,
    Parameter,
    Result,
}

/// Basic entity kinds the old grammar distinguishes before the entity name.
#[derive(Clone, Copy, PartialEq, Eq)]
enum BasicEntity {
    Function,
    Variable,
    Initializer,
    Subscript,
}

/// Standard-library substitution codes (old table).
enum StandardSubstitution {
    Module(&'static str),
    Type(Kind, &'static str),
}

fn standard_substitution(code: u8) -> Option<StandardSubstitution> {
    let sub = match code {
        b'o' => StandardSubstitution::Module(OBJC_MODULE),
        b'C' => StandardSubstitution::Module(CLANG_MODULE),
        b'a' => StandardSubstitution::Type(Kind::Structure, "Array"),
        b'b' => StandardSubstitution::Type(Kind::Structure, "Bool"),
        b'c' => StandardSubstitution::Type(Kind::Structure, "UnicodeScalar"),
        b'd' => StandardSubstitution::Type(Kind::Structure, "Double"),
        b'f' => StandardSubstitution::Type(Kind::Structure, "Float"),
        b'i' => StandardSubstitution::Type(Kind::Structure, "Int"),
        b'V' => StandardSubstitution::Type(Kind::Structure, "UnsafeRawPointer"),
        b'v' => StandardSubstitution::Type(Kind::Structure, "UnsafeMutableRawPointer"),
        b'P' => StandardSubstitution::Type(Kind::Structure, "UnsafePointer"),
        b'p' => StandardSubstitution::Type(Kind::Structure, "UnsafeMutablePointer"),
        b'q' => StandardSubstitution::Type(Kind::Enum, "Optional"),
        b'Q' => StandardSubstitution::Type(Kind::Enum, "ImplicitlyUnwrappedOptional"),
        b'R' => StandardSubstitution::Type(Kind::Structure, "UnsafeBufferPointer"),
        b'r' => StandardSubstitution::Type(Kind::Structure, "UnsafeMutableBufferPointer"),
        b'S' => StandardSubstitution::Type(Kind::Structure, "String"),
        b'u' => StandardSubstitution::Type(Kind::Structure, "UInt"),
        _ => return None,
    };
    Some(sub)
}

fn is_start_of_identifier(byte: u8) -> bool {
    byte.is_ascii_digit() || byte == b'o'
}

fn is_start_of_entity(byte: u8) -> bool {
    matches!(byte, b'F' | b'I' | b'v' | b'P' | b's' | b'Z' | b'C' | b'V' | b'O')
}

/// Decode one operator-name letter (`a`-`z`) into its symbol.
fn operator_symbol(byte: u8) -> Option<u8> {
    const TABLE: &[u8; 26] = b"& @/= >    <*!|+?%-~   ^ .";
    if byte.is_ascii_lowercase() {
        let symbol = *TABLE.get(usize::from(byte - b'a'))?;
        if symbol == b' ' { None } else { Some(symbol) }
    } else if byte >= 0x80 {
        Some(byte)
    } else {
        None
    }
}

/// Value-witness kind names, keyed by their two-letter mangling code.
fn value_witness_name(first: Option<u8>, second: Option<u8>) -> Option<&'static str> {
    let code = [first?, second?];
    let name = match &code {
        b"al" => "allocateBuffer",
        b"ca" => "assignWithCopy",
        b"ta" => "assignWithTake",
        b"de" => "deallocateBuffer",
        b"xx" => "destroy",
        b"XX" => "destroyBuffer",
        b"Xx" => "destroyArray",
        b"CP" => "initializeBufferWithCopyOfBuffer",
        b"Cp" => "initializeBufferWithCopy",
        b"cp" => "initializeWithCopy",
        b"Tk" => "initializeBufferWithTake",
        b"tk" => "initializeWithTake",
        b"pr" => "projectBuffer",
        b"TK" => "initializeBufferWithTakeOfBuffer",
        b"Cc" => "initializeArrayWithCopy",
        b"Tt" => "initializeArrayWithTakeFrontToBack",
        b"tT" => "initializeArrayWithTakeBackToFront",
        b"xs" => "storeExtraInhabitant",
        b"xg" => "getExtraInhabitantIndex",
        b"ug" => "getEnumTag",
        b"up" => "destructiveProjectEnumData",
        b"ui" => "destructiveInjectEnumTag",
        b"et" => "getEnumTagSinglePayload",
        b"st" => "storeEnumTagSinglePayload",
        _ => return None,
    };
    Some(name)
}

/// Format `prefix + decimal` into `buffer`.
fn format_builtin<'b>(buffer: &'b mut [u8], prefix: &str, value: u32) -> Option<&'b str> {
    let prefix_len = prefix.len();
    if prefix_len >= buffer.len() {
        return None;
    }
    buffer[..prefix_len].copy_from_slice(prefix.as_bytes());
    let mut len = prefix_len;
    let mut digits = [0_u8; 10];
    let mut count = 0_usize;
    let mut value = value;
    loop {
        digits[count] = b'0' + u8::try_from(value % 10).ok()?;
        value /= 10;
        count += 1;
        if value == 0 {
            break;
        }
    }
    for index in (0..count).rev() {
        *buffer.get_mut(len)? = digits[index];
        len += 1;
    }
    core::str::from_utf8(&buffer[..len]).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn render(symbol: &str) -> Option<alloc::string::String> {
        let mut out = alloc::string::String::new();
        crate::swift::print(symbol, &mut out).ok()?;
        Some(out)
    }

    #[test]
    fn value_witness_codes_are_known() {
        assert_eq!(value_witness_name(Some(b'a'), Some(b'l')), Some("allocateBuffer"));
        assert_eq!(value_witness_name(Some(b'a'), Some(b'x')), None);
    }

    #[test]
    fn legacy_types_render() {
        assert_eq!(render("_TtSi").as_deref(), Some("Swift.Int"));
        assert_eq!(render("_TtGSaSS_").as_deref(), Some("[Swift.String]"));
        assert_eq!(
            render("_TtGSqGSaC5sugar7MyClass__").as_deref(),
            Some("[sugar.MyClass]?")
        );
    }

    #[test]
    fn legacy_accessors_use_storage_entities() {
        // The old grammar nests the storage under an accessor node; the shared
        // printer carries the accessor code on the storage node instead.
        assert_eq!(
            render("_TF3fooau3barSi").as_deref(),
            Some("foo.bar.unsafeMutableAddressor : Swift.Int")
        );
        assert_eq!(
            render("_TiC4Meow5MyCls9subscriptFT1iSi_Sf").as_deref(),
            Some("Meow.MyCls.subscript(i: Swift.Int) -> Swift.Float")
        );
    }

    #[test]
    fn legacy_globals_map_to_shared_codes() {
        assert_eq!(
            render("_TWvdvC3foo3bar3basSi").as_deref(),
            Some("direct field offset for foo.bar.bas : Swift.Int")
        );
        assert_eq!(
            render("_TwalC3foo3bar").as_deref(),
            Some("allocateBuffer value witness for foo.bar")
        );
        assert_eq!(
            render("_TMnC3foo3bar").as_deref(),
            Some("nominal type descriptor for foo.bar")
        );
    }

    #[test]
    fn unsupported_forms_are_rejected() {
        // Malformed and unimplemented legacy forms must not print.
        for symbol in ["_TtZZ", "_TtB", "_Tw", "_TTSgS"] {
            assert_eq!(render(symbol), None, "{symbol}");
        }
    }
}
