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

use alloc::{borrow::Cow, vec::Vec};

use crate::parse::{Cursor, Escape, State};
use crate::swift::ParseLimits;
use crate::swift::node::{
    Accessor, AttributeKind, Builder, Context, DeinitializerKind, GenericSpecializationKind,
    Global, GlobalOperand, Kind, NodeRef,
};
use crate::swift::text;

/// Standard library module name.
const STDLIB: &str = "Swift";

/// Objective-C module name.
const OBJC_MODULE: &str = "__C";

/// Clang importer module name.
const CLANG_MODULE: &str = "__C_Synthesized";

/// Parse a legacy `_T` symbol.
///
/// `text` is the symbol bytes with the leading `_T` already removed; they need
/// not be UTF-8. Returns the assembled top-level node, or `None` when the
/// symbol is not a form this parser implements or the parse exceeds `limits`.
///
/// A budget failure is recorded on `state` ([`State::exhausted`]) and
/// `nodes` receives the nodes the parse charged, so a caller re-demangling a
/// payload can tell a budget failure from a malformed symbol and charge what
/// the nested parse spent.
pub fn parse<'a>(
    text: &'a [u8],
    state: &mut State,
    limits: ParseLimits,
    nodes: &mut usize,
) -> Option<NodeRef<'a>> {
    let input = Cursor::new(text, 0, state).ok()?;
    let mut parser = LegacyParser {
        input,
        context: Context::new(limits.nodes()),
        subs: Vec::new(),
        limits,
        payload_visits: 0,
        payload_bytes: 0,
    };
    let root = parser.top_level();
    *nodes = parser.context.used();
    if parser.context.failed() {
        parser.input.exhaust();
        return None;
    }
    root
}

/// Recursive-descent parser for the pre-stable grammar.
struct LegacyParser<'a, 's> {
    input: Cursor<'a, 's>,
    context: Context<'a>,
    subs: Vec<NodeRef<'a>>,
    /// The budget this parse enforces.
    limits: ParseLimits,
    /// Expansion visits spent rendering nested payload symbols.
    payload_visits: usize,
    /// Rendered bytes spent rendering nested payload symbols.
    payload_bytes: usize,
}

impl<'a> LegacyParser<'a, '_> {
    // ---- input ---------------------------------------------------------

    fn peek(&self) -> Option<u8> {
        self.input.peek()
    }

    fn peek_at(&self, offset: usize) -> Option<u8> {
        self.input.peek_at(offset)
    }

    fn at_end(&self) -> bool {
        self.input.remaining().is_empty()
    }

    /// Consume one byte, charging the work budget.
    fn take(&mut self) -> Option<u8> {
        let bytes = self.input.advance(1).ok()?;
        bytes.first().copied()
    }

    fn eat(&mut self, byte: u8) -> bool {
        if self.peek() != Some(byte) {
            return false;
        }
        self.input.advance(1).is_ok()
    }

    fn eat_str(&mut self, bytes: &[u8]) -> bool {
        if !self.input.starts_with(bytes) {
            return false;
        }
        self.input.advance(bytes.len()).is_ok()
    }

    fn has(&self, len: usize) -> bool {
        self.input.remaining().len() >= len
    }

    /// Consume `len` bytes and return them.
    fn take_bytes(&mut self, len: usize) -> Option<&'a [u8]> {
        self.input.advance(len).ok()
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
            if self.input.advance(1).is_err() {
                return None;
            }
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

    /// Start a node; the caller pushes children and calls `Builder::build`.
    const fn builder(&mut self, kind: Kind) -> Option<Builder<'a>> {
        if self.context.failed() {
            return None;
        }
        Some(self.context.builder(kind))
    }

    const fn builder_text(&mut self, kind: Kind, text: &'a str) -> Option<Builder<'a>> {
        if self.context.failed() {
            return None;
        }
        Some(self.context.builder_text(kind, text))
    }

    /// A text builder from borrowed or escaped text.
    fn builder_text_cow(&mut self, kind: Kind, text: Cow<'a, str>) -> Option<Builder<'a>> {
        if self.context.failed() {
            return None;
        }
        Some(self.context.builder_text_cow(kind, text))
    }

    fn builder_owned(&mut self, kind: Kind, text: &str) -> Option<Builder<'a>> {
        if self.context.failed() {
            return None;
        }
        Some(self.context.builder_owned(kind, text))
    }

    const fn builder_value(&mut self, kind: Kind, value: u32) -> Option<Builder<'a>> {
        if self.context.failed() {
            return None;
        }
        Some(self.context.builder_value(kind, value))
    }

    const fn builder_index(&mut self, kind: Kind, index: u32) -> Option<Builder<'a>> {
        if self.context.failed() {
            return None;
        }
        Some(self.context.builder_index(kind, index))
    }

    const fn builder_accessor(&mut self, kind: Kind, code: Accessor) -> Option<Builder<'a>> {
        if self.context.failed() {
            return None;
        }
        Some(self.context.builder_accessor(kind, code))
    }

    const fn builder_impl(&mut self, kind: Kind, code: u8) -> Option<Builder<'a>> {
        if self.context.failed() {
            return None;
        }
        Some(self.context.builder_impl(kind, code))
    }

    const fn builder_global(
        &mut self,
        code: Global,
        operand: GlobalOperand,
    ) -> Option<Builder<'a>> {
        if self.context.failed() {
            return None;
        }
        Some(self.context.builder_global(code, operand))
    }

    const fn builder_deinit(&mut self, kind: DeinitializerKind) -> Option<Builder<'a>> {
        if self.context.failed() {
            return None;
        }
        Some(self.context.builder_deinit(kind))
    }

    const fn builder_attribute(&mut self, kind: AttributeKind) -> Option<Builder<'a>> {
        if self.context.failed() {
            return None;
        }
        Some(self.context.builder_attribute(kind))
    }

    const fn builder_generic_specialization(
        &mut self,
        kind: GenericSpecializationKind,
    ) -> Option<Builder<'a>> {
        if self.context.failed() {
            return None;
        }
        Some(self.context.builder_generic_specialization(kind, false))
    }

    /// The old grammar has no `r` spec attribute, so nothing is re-abstracted.
    const fn builder_function_signature_specialization(&mut self) -> Option<Builder<'a>> {
        if self.context.failed() {
            return None;
        }
        Some(
            self.context
                .builder_function_signature_specialization(false),
        )
    }

    /// Every old parameter is an argument; the return entry has no marker.
    const fn builder_function_spec(&mut self) -> Option<Builder<'a>> {
        if self.context.failed() {
            return None;
        }
        Some(self.context.builder_function_spec(false))
    }

    /// A layout requirement builder; unknown layout letters are rejected.
    fn builder_layout(&mut self, code: u8) -> Option<Builder<'a>> {
        if self.context.failed() {
            return None;
        }
        self.context.builder_layout(code)
    }

    /// Build a leaf node.
    fn node(&mut self, kind: Kind) -> Option<NodeRef<'a>> {
        Some(self.builder(kind)?.build())
    }

    fn text_node(&mut self, kind: Kind, text: &'a str) -> Option<NodeRef<'a>> {
        Some(self.builder_text(kind, text)?.build())
    }

    fn owned_node(&mut self, kind: Kind, text: &str) -> Option<NodeRef<'a>> {
        Some(self.builder_owned(kind, text)?.build())
    }

    /// A text node from borrowed or escaped text.
    fn text_cow_node(&mut self, kind: Kind, text: Cow<'a, str>) -> Option<NodeRef<'a>> {
        Some(self.builder_text_cow(kind, text)?.build())
    }

    fn value_node(&mut self, kind: Kind, value: u32) -> Option<NodeRef<'a>> {
        Some(self.builder_value(kind, value)?.build())
    }

    fn with_children(&mut self, kind: Kind, children: &[NodeRef<'a>]) -> Option<NodeRef<'a>> {
        let mut node = self.builder(kind)?;
        for child in children {
            node.push(child.clone());
        }
        Some(node.build())
    }

    fn with_children_opt(
        &mut self,
        kind: Kind,
        children: &[Option<NodeRef<'a>>],
    ) -> Option<NodeRef<'a>> {
        let mut node = self.builder(kind)?;
        for child in children.iter().flatten() {
            node.push(child.clone());
        }
        Some(node.build())
    }

    /// Record a substitution. Every entry is a node the parse already charged
    /// or a reference consuming input, so the node and input budgets bound
    /// the table.
    fn sub_push(&mut self, node: NodeRef<'a>) {
        self.subs.push(node);
    }

    fn sub(&self, index: u32) -> Option<NodeRef<'a>> {
        self.subs.get(usize::try_from(index).ok()?).cloned()
    }

    /// Wrap `inner` in a [`Kind::Type`], the old grammar's type marker.
    fn wrap_type(&mut self, inner: NodeRef<'a>) -> Option<NodeRef<'a>> {
        self.with_children(Kind::Type, &[inner])
    }

    /// True when `depth` exceeds the depth budget; records the exhaustion.
    const fn over_depth(&mut self, depth: u16) -> bool {
        if depth > self.limits.depth() {
            self.input.exhaust();
            return true;
        }
        false
    }

    // ---- top level -----------------------------------------------------

    /// `mangled-name ::= '_T' global`, plus the old attribute prefixes.
    fn top_level(&mut self) -> Option<NodeRef<'a>> {
        let mut top = self.builder(Kind::TopLevel)?;
        // Specialization prefixes are chained and each resets the
        // substitution table, exactly as in the reference implementation.
        if self.eat_str(b"TS") {
            loop {
                let attribute = self.specialized_attribute(0)?;
                top.push(attribute);
                self.subs.clear();
                if !self.eat_str(b"_TTS") {
                    break;
                }
            }
            if !self.eat_str(b"_T") {
                return None;
            }
        } else if self.eat_str(b"To") {
            top.push(self.attribute(AttributeKind::Objc)?);
        } else if self.eat_str(b"TO") {
            top.push(self.attribute(AttributeKind::NonObjc)?);
        } else if self.eat_str(b"TD") {
            top.push(self.attribute(AttributeKind::Dynamic)?);
        } else if self.eat_str(b"Td") {
            // `super` in the shared printer's attribute table.
            top.push(self.attribute(AttributeKind::Super)?);
        } else if self.eat_str(b"TV") {
            // `override` in the shared printer's attribute table.
            top.push(self.attribute(AttributeKind::Override)?);
        }

        let global = self.global(0)?;
        top.push(global);

        // Trailing text is an unmangled suffix, as the reference does.
        if !self.at_end() {
            // Compiler payload: bytes stay verbatim, invalid bytes are escaped.
            let rest = text::cow(self.input.remaining(), Escape::Verbatim);
            let suffix = self.text_cow_node(Kind::Suffix, rest)?;
            top.push(suffix);
            let remaining = self.input.remaining().len();
            let _ = self.input.advance(remaining);
        }
        Some(top.build())
    }

    fn attribute(&mut self, kind: AttributeKind) -> Option<NodeRef<'a>> {
        Some(self.builder_attribute(kind)?.build())
    }

    /// `specialized-attribute ::= generic-specialization | function-specialization`
    fn specialized_attribute(&mut self, depth: u16) -> Option<NodeRef<'a>> {
        if self.over_depth(depth) {
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
        // `r` selects the printer's "generic not re-abstracted
        // specialization" description; `g` the plain one.
        let kind = if not_reabstracted {
            GenericSpecializationKind::NotReabstracted
        } else {
            GenericSpecializationKind::Generic
        };
        let mut specialization = self.builder_generic_specialization(kind)?;
        if self.eat(b'q') {
            let serialized = self.node(Kind::IsSerialized)?;
            specialization.push(serialized);
        }
        // The specialization pass id is not printed.
        let _ = self.take()?;
        self.generic_specialization(&mut specialization, depth)?;
        Some(specialization.build())
    }

    /// `function-signature-specialization ::= (param)* '_'`
    ///
    /// Payload-carrying parameters (`cp*`, `cl*`) demangle a nested symbol in
    /// the reference printer; they are rejected here rather than printed
    /// without their payload.
    fn function_signature_specialization(&mut self, depth: u16) -> Option<NodeRef<'a>> {
        if self.over_depth(depth) {
            return None;
        }
        let mut specialization = self.builder_function_signature_specialization()?;
        if self.eat(b'q') {
            let serialized = self.node(Kind::IsSerialized)?;
            specialization.push(serialized);
        }
        // The specialization pass id is not printed.
        let _ = self.take()?;
        while !self.eat(b'_') {
            let mut param = self.builder_function_spec()?;
            if self.eat_str(b"n_") {
                // An empty parameter still occupies an argument slot.
            } else if self.eat_str(b"cp") {
                self.constant_propagation(&mut param, depth + 1)?;
            } else if self.eat_str(b"cl") {
                self.closure_propagation(&mut param, depth + 1)?;
            } else if self.eat_str(b"i_") {
                self.func_spec_kind(&mut param, 6)?;
            } else if self.eat_str(b"k_") {
                self.func_spec_kind(&mut param, 7)?;
            } else if self.eat_str(b"r_") {
                self.func_spec_kind(&mut param, 8)?;
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
                self.func_spec_kind(&mut param, value)?;
            }
            specialization.push(param.build());
        }
        Some(specialization.build())
    }

    /// `constant-propagation ::= 'fr' identifier '_' | 'g' identifier '_' |
    /// 'i' <digits> '_' | 'fl' <digits> '_' |
    /// 'se' ('0'|'1') 'v' identifier '_'`
    fn constant_propagation(&mut self, param: &mut Builder<'a>, depth: u16) -> Option<()> {
        if self.over_depth(depth) {
            return None;
        }
        if self.eat_str(b"fr") {
            let name = self.identifier(depth + 1, Some(Kind::Identifier))?;
            if !self.eat(b'_') {
                return None;
            }
            self.func_spec_kind(param, 0)?;
            let payload = self.payload_node(&name, depth)?;
            param.push(payload);
            return Some(());
        }
        if self.eat(b'g') {
            let name = self.identifier(depth + 1, Some(Kind::Identifier))?;
            if !self.eat(b'_') {
                return None;
            }
            self.func_spec_kind(param, 1)?;
            let payload = self.payload_node(&name, depth)?;
            param.push(payload);
            return Some(());
        }
        if self.eat(b'i') {
            let text = self.read_until_underscore()?;
            self.func_spec_kind(param, 2)?;
            let payload = self.text_cow_node(Kind::Identifier, text)?;
            param.push(payload);
            return Some(());
        }
        if self.eat_str(b"fl") {
            let text = self.read_until_underscore()?;
            self.func_spec_kind(param, 3)?;
            let payload = self.text_cow_node(Kind::Identifier, text)?;
            param.push(payload);
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
            param.push(payload);
            let payload = self.payload_node(&name, depth)?;
            param.push(payload);
            return Some(());
        }
        None
    }

    /// `closure-propagation ::= identifier type* '_'`
    fn closure_propagation(&mut self, param: &mut Builder<'a>, depth: u16) -> Option<()> {
        if self.over_depth(depth) {
            return None;
        }
        let name = self.identifier(depth + 1, Some(Kind::Identifier))?;
        self.func_spec_kind(param, 5)?;
        let payload = self.payload_node(&name, depth)?;
        param.push(payload);
        while self.peek() != Some(b'_') {
            let ty = self.ty(depth + 1)?;
            param.push(ty);
        }
        if !self.eat(b'_') {
            return None;
        }
        Some(())
    }

    /// Read the payload text up to (and including) the terminating `_`.
    ///
    /// The payload is compiler text: its bytes are kept verbatim, and only
    /// invalid UTF-8 bytes are escaped.
    fn read_until_underscore(&mut self) -> Option<Cow<'a, str>> {
        let start = self.input.pos();
        while let Some(byte) = self.peek() {
            if byte == b'_' {
                break;
            }
            let _ = self.take()?;
        }
        if !self.eat(b'_') {
            return None;
        }
        let end = self.input.pos().checked_sub(1)?;
        let bytes = self.input.bytes_between(start, end)?;
        Some(text::cow(bytes, Escape::Verbatim))
    }

    /// A specialization payload: demangle it when it is a symbol.
    ///
    /// The reference printer re-demangles payload text, which can itself carry
    /// a payload. The nested parse spends this parse's budget: it gets the
    /// depth left below `depth`, the remaining work and nodes, and the
    /// expansion visits and bytes not yet spent on earlier payloads, and what
    /// it uses is charged back here. A payload that runs out of budget rejects
    /// the whole name, so the result is never printed with undemangled payload
    /// text; a payload that is not a symbol prints verbatim.
    fn payload_node(&mut self, name: &NodeRef<'a>, depth: u16) -> Option<NodeRef<'a>> {
        let payload = name.text()?;
        if !crate::swift::is_swift(payload) {
            return self.owned_node(Kind::Identifier, payload);
        }
        let depth = depth.saturating_add(1);
        if self.over_depth(depth) {
            return None;
        }
        let allowance = crate::swift::PayloadAllowance {
            depth: self.limits.depth() - depth,
            work: self.limits.work().saturating_sub(self.input.work_used()),
            nodes: self.context.remaining(),
            expansion_visits: self
                .limits
                .expansion_visits()
                .saturating_sub(self.payload_visits),
            expansion_bytes: self
                .limits
                .expansion_bytes()
                .saturating_sub(self.payload_bytes),
        };
        let outcome = crate::swift::demangle_payload(payload.as_bytes(), self.limits, allowance);
        self.input.charge(outcome.work).ok()?;
        self.context.charge_many(outcome.nodes);
        self.payload_visits = self.payload_visits.saturating_add(outcome.expansion_visits);
        self.payload_bytes = self.payload_bytes.saturating_add(outcome.expansion_bytes);
        match outcome.text {
            crate::swift::PayloadText::Rendered(rendered) => {
                self.owned_node(Kind::Identifier, &rendered)
            }
            crate::swift::PayloadText::Verbatim => self.owned_node(Kind::Identifier, payload),
            crate::swift::PayloadText::Exhausted => {
                self.input.exhaust();
                None
            }
        }
    }

    fn func_spec_kind(&mut self, param: &mut Builder<'a>, value: u32) -> Option<()> {
        let kind = self.value_node(Kind::FuncSpecKind, value)?;
        param.push(kind);
        Some(())
    }

    /// `generic-specialization ::= (type protocol-conformance* '_')+`
    fn generic_specialization(
        &mut self,
        specialization: &mut Builder<'a>,
        depth: u16,
    ) -> Option<()> {
        if self.over_depth(depth) {
            return None;
        }
        while !self.eat(b'_') {
            let mut param = self.builder(Kind::GenericSpecializationParam)?;
            let ty = self.ty(depth + 1)?;
            param.push(ty);
            while !self.eat(b'_') {
                let conformance = self.protocol_conformance(depth + 1)?;
                param.push(conformance);
            }
            specialization.push(param.build());
        }
        Some(())
    }

    // ---- globals -------------------------------------------------------

    fn global(&mut self, depth: u16) -> Option<NodeRef<'a>> {
        if self.over_depth(depth) || self.at_end() {
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
            let mut forwarder = self.builder_global(code, GlobalOperand::None)?;
            if self.eat_str(b"__T") {
                let inner = self.global(depth + 1)?;
                forwarder.push(inner);
            }
            return Some(forwarder.build());
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
            let mut witness = self.builder_text(Kind::ValueWitness, name)?;
            let ty = self.ty(depth + 1)?;
            witness.push(ty);
            return Some(witness.build());
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
            let mut node = self.builder_global(Global::TypeMetadataPattern, GlobalOperand::None)?;
            let ty = self.ty(depth + 1)?;
            node.push(ty);
            return Some(node.build());
        }
        if self.eat(b'a') {
            let mut node = self.builder_global(Global::TypeMetadataAccess, GlobalOperand::None)?;
            let ty = self.ty(depth + 1)?;
            node.push(ty);
            return Some(node.build());
        }
        if self.eat(b'L') {
            let mut node =
                self.builder_global(Global::TypeMetadataLazyCache, GlobalOperand::None)?;
            let ty = self.ty(depth + 1)?;
            node.push(ty);
            return Some(node.build());
        }
        if self.eat(b'm') {
            let mut node = self.builder_global(Global::ClassMetaclass, GlobalOperand::None)?;
            let ty = self.ty(depth + 1)?;
            node.push(ty);
            return Some(node.build());
        }
        if self.eat(b'n') {
            let mut node =
                self.builder_global(Global::NominalTypeDescriptor, GlobalOperand::None)?;
            let ty = self.ty(depth + 1)?;
            node.push(ty);
            return Some(node.build());
        }
        if self.eat(b'f') {
            let mut node = self.builder_global(Global::FullTypeMetadata, GlobalOperand::None)?;
            let ty = self.ty(depth + 1)?;
            node.push(ty);
            return Some(node.build());
        }
        if self.eat(b'p') {
            let mut node = self.builder_global(Global::ProtocolDescriptor, GlobalOperand::None)?;
            let proto = self.protocol_name(depth + 1)?;
            node.push(proto);
            return Some(node.build());
        }
        let mut node = self.builder_global(Global::TypeMetadata, GlobalOperand::None)?;
        let ty = self.ty(depth + 1)?;
        node.push(ty);
        Some(node.build())
    }

    /// `witness ::= 'W' ('V' type | 'v' directness entity | 'P' conformance |
    /// 'G' conformance | 'I' conformance | 'l' type conformance |
    /// 'L' type conformance | 'a' conformance | 't' conformance decl-name |
    /// 'T' conformance decl-name protocol-name)`
    fn witness(&mut self, depth: u16) -> Option<NodeRef<'a>> {
        if self.eat(b'V') {
            let mut node = self.builder_global(Global::ValueWitnessTable, GlobalOperand::None)?;
            let ty = self.ty(depth + 1)?;
            node.push(ty);
            return Some(node.build());
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
            let mut node =
                self.builder_global(Global::FieldOffset, GlobalOperand::Directness(indirect))?;
            let entity = self.entity(depth + 1)?;
            node.push(entity);
            return Some(node.build());
        }
        if self.eat(b'P') {
            let mut node =
                self.builder_global(Global::ProtocolWitnessTable, GlobalOperand::None)?;
            let conformance = self.protocol_conformance(depth + 1)?;
            node.push(conformance);
            return Some(node.build());
        }
        if self.eat(b'G') {
            let mut node =
                self.builder_global(Global::GenericProtocolWitnessTable, GlobalOperand::None)?;
            let conformance = self.protocol_conformance(depth + 1)?;
            node.push(conformance);
            return Some(node.build());
        }
        if self.eat(b'I') {
            let mut node = self.builder_global(
                Global::GenericWitnessTableInstantiationFunction,
                GlobalOperand::None,
            )?;
            let conformance = self.protocol_conformance(depth + 1)?;
            node.push(conformance);
            return Some(node.build());
        }
        if self.eat(b'l') {
            let mut node = self.builder_global(
                Global::LazyProtocolWitnessTableAccessor,
                GlobalOperand::None,
            )?;
            let ty = self.ty(depth + 1)?;
            node.push(ty);
            let conformance = self.protocol_conformance(depth + 1)?;
            node.push(conformance);
            return Some(node.build());
        }
        if self.eat(b'L') {
            let mut node =
                self.builder_global(Global::LazyProtocolWitnessTableCache, GlobalOperand::None)?;
            let ty = self.ty(depth + 1)?;
            node.push(ty);
            let conformance = self.protocol_conformance(depth + 1)?;
            node.push(conformance);
            return Some(node.build());
        }
        if self.eat(b'a') {
            let mut node =
                self.builder_global(Global::ProtocolWitnessTableAccessor, GlobalOperand::None)?;
            let conformance = self.protocol_conformance(depth + 1)?;
            node.push(conformance);
            return Some(node.build());
        }
        if self.eat(b't') {
            let mut node =
                self.builder_global(Global::AssociatedTypeMetadataAccessor, GlobalOperand::None)?;
            let conformance = self.protocol_conformance(depth + 1)?;
            node.push(conformance);
            let name = self.decl_name(depth + 1)?;
            node.push(name);
            return Some(node.build());
        }
        if self.eat(b'T') {
            let mut node = self.builder_global(
                Global::AssociatedTypeWitnessTableAccessor,
                GlobalOperand::None,
            )?;
            let conformance = self.protocol_conformance(depth + 1)?;
            node.push(conformance);
            let name = self.decl_name(depth + 1)?;
            node.push(name);
            let proto = self.protocol_name(depth + 1)?;
            node.push(proto);
            return Some(node.build());
        }
        None
    }

    /// `thunk ::= 'T' ('R' reabstract-signature | 'r' reabstract-signature |
    /// 'W' protocol-conformance entity)`
    fn thunk(&mut self, depth: u16) -> Option<NodeRef<'a>> {
        if self.eat(b'R') {
            let mut thunk =
                self.builder_global(Global::ReabstractionThunkHelper, GlobalOperand::None)?;
            self.reabstract_signature(&mut thunk, depth + 1)?;
            return Some(thunk.build());
        }
        if self.eat(b'r') {
            let mut thunk = self.builder_global(Global::ReabstractionThunk, GlobalOperand::None)?;
            self.reabstract_signature(&mut thunk, depth + 1)?;
            return Some(thunk.build());
        }
        if self.eat(b'W') {
            // The entity is mangled in its own generic context.
            let mut thunk =
                self.builder_global(Global::ProtocolWitnessThunk, GlobalOperand::None)?;
            let conformance = self.protocol_conformance(depth + 1)?;
            thunk.push(conformance);
            let entity = self.entity(depth + 1)?;
            thunk.push(entity);
            return Some(thunk.build());
        }
        None
    }

    /// `reabstract-signature ::= generic-signature? type type`
    ///
    /// The printer renders the second type as the source and the first as the
    /// destination, matching the reference node order.
    fn reabstract_signature(&mut self, signature: &mut Builder<'a>, depth: u16) -> Option<()> {
        if self.eat(b'G') {
            let generics = self.generic_signature(depth + 1)?;
            signature.push(generics);
        }
        let src = self.ty(depth + 1)?;
        signature.push(src);
        let dest = self.ty(depth + 1)?;
        signature.push(dest);
        Some(())
    }

    // ---- entities ------------------------------------------------------

    /// `entity ::= 'Z'? (entity-kind context entity-name | nominal-type)`
    fn entity(&mut self, depth: u16) -> Option<NodeRef<'a>> {
        if self.over_depth(depth) {
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
        // Deinitializer variants share one node kind and differ by payload.
        let mut deinitializer = None;
        let kind = if basic == BasicEntity::Initializer {
            if self.eat(b'A') {
                let index = self.index()?;
                let mut entity = self.builder_index(Kind::DefaultArgumentInitializer, index)?;
                entity.push(context);
                return self.finish_entity(entity.build(), is_static);
            }
            if !self.eat(b'i') {
                return None;
            }
            has_type = false;
            Kind::Initializer
        } else if let Some(form) = self.deinitializer_kind() {
            has_type = false;
            deinitializer = Some(form);
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
            let mut entity = self.builder_index(kind, index)?;
            entity.push(context);
            let ty = self.ty(depth + 1)?;
            entity.push(ty);
            return self.finish_entity(entity.build(), is_static);
        } else {
            name = Some(self.decl_name(depth + 1)?);
            match basic {
                BasicEntity::Function => Kind::Function,
                BasicEntity::Variable => Kind::Variable,
                BasicEntity::Subscript => Kind::Subscript,
                BasicEntity::Initializer => unreachable!(),
            }
        };

        let mut entity = match deinitializer {
            Some(form) => self.builder_deinit(form)?,
            None => self.builder(kind)?,
        };
        entity.push(context);
        if let Some(name) = name {
            entity.push(name);
        }
        if has_type {
            let ty = self.ty(depth + 1)?;
            entity.push(ty);
        }
        self.finish_entity(entity.build(), is_static)
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

    /// Consume a deinitializer name and return its printer form: `deinit`,
    /// `__deallocating_deinit` or `__isolated_deallocating_deinit`.
    fn deinitializer_kind(&mut self) -> Option<DeinitializerKind> {
        let form = match self.peek()? {
            b'd' => DeinitializerKind::Deinit,
            b'D' => DeinitializerKind::Deallocating,
            b'Z' => DeinitializerKind::IsolatedDeallocating,
            _ => return None,
        };
        let _ = self.take()?;
        Some(form)
    }

    fn finish_entity(&mut self, entity: NodeRef<'a>, is_static: bool) -> Option<NodeRef<'a>> {
        if is_static {
            let mut node = self.builder(Kind::Static)?;
            node.push(entity);
            Some(node.build())
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
        let is_subscript = name.kind() == Kind::Identifier && name.text_eq("subscript");
        let mut node = self.builder_accessor(
            if is_subscript {
                Kind::Subscript
            } else {
                Kind::Variable
            },
            accessor,
        )?;
        node.push(context);
        if !is_subscript {
            node.push(name);
        }
        let ty = self.ty(depth + 1)?;
        node.push(ty);
        Some(node.build())
    }

    // ---- contexts and names --------------------------------------------

    /// `context ::= module | entity | 'E' module context |
    /// 'e' module generic-signature context | 'S' substitution | 'G'
    /// bound-generic-type`
    fn context(&mut self, depth: u16) -> Option<NodeRef<'a>> {
        if self.over_depth(depth) || self.at_end() {
            return None;
        }
        if self.eat(b'E') {
            let mut ext = self.builder(Kind::Extension)?;
            let module = self.module(depth + 1)?;
            ext.push(module);
            let ty = self.context(depth + 1)?;
            ext.push(ty);
            return Some(ext.build());
        }
        if self.eat(b'e') {
            let mut ext = self.builder(Kind::Extension)?;
            let module = self.module(depth + 1)?;
            let signature = self.generic_signature(depth + 1)?;
            let ty = self.context(depth + 1)?;
            ext.push(module);
            ext.push(ty);
            ext.push(signature);
            return Some(ext.build());
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
            if module.kind() != Kind::Module {
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
        if self.over_depth(depth) || self.at_end() {
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
        let slice = self.take_bytes(length)?;
        if !punycoded && !is_operator {
            if slice.is_empty() {
                return None;
            }
            // Identifier bytes: a length can split a UTF-8 sequence, so text
            // is escaped rather than decoded.
            return self.text_cow_node(kind, text::cow(slice, Escape::Identifier));
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
            // Operator table bytes are ASCII: the conversion cannot fail.
            let text = core::str::from_utf8(&decoded_operator[..text.len()]).ok()?;
            return self.owned_node(kind, text);
        }
        self.text_cow_node(kind, Cow::Owned(text::owned(text, Escape::Identifier)))
    }

    /// `substitution ::= 'S' ('o' | 'C' | <stdlib-code> | index)`
    fn substitution_index(&mut self, depth: u16) -> Option<NodeRef<'a>> {
        if self.over_depth(depth) || self.at_end() {
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
                let _ = self.take()?;
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

    fn swift_type(&mut self, kind: Kind, name: &'static str) -> Option<NodeRef<'a>> {
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
        if self.over_depth(depth) {
            return None;
        }
        if self.eat(b'S') {
            let sub = self.substitution_index(depth + 1)?;
            if sub.kind() == Kind::Protocol {
                return Some(sub);
            }
            if sub.kind() != Kind::Module {
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

    fn bound_generic_args(&mut self, mut nominal: NodeRef<'a>, depth: u16) -> Option<NodeRef<'a>> {
        if self.over_depth(depth) || nominal.child_count() == 0 {
            return None;
        }
        // Generic arguments for the outermost type come first.
        let parent = nominal.child(0)?;
        let parent_kind = parent.kind();
        if !matches!(parent_kind, Kind::Module | Kind::Function | Kind::Extension) {
            let rebuilt_parent = self.bound_generic_args(parent, depth + 1)?;
            let kind = nominal.kind();
            let mut rebuilt = self.builder(kind)?;
            rebuilt.push(rebuilt_parent);
            for index in 1..nominal.child_count() {
                let child = nominal.child(index)?;
                rebuilt.push(child);
            }
            nominal = rebuilt.build();
        }

        let mut args = self.builder(Kind::TypeList)?;
        let mut count = 0_usize;
        while !self.eat(b'_') {
            let ty = self.ty(depth + 1)?;
            args.push(ty);
            count += 1;
            if self.at_end() {
                return None;
            }
        }
        if count == 0 {
            return Some(nominal);
        }
        let args = args.build();
        let kind = match nominal.kind() {
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

    #[expect(clippy::match_same_arms, reason = "one arm per legacy type code")]
    fn type_impl(&mut self, depth: u16) -> Option<NodeRef<'a>> {
        if self.over_depth(depth) || self.at_end() {
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
                    let mut node = self.builder(Kind::OpaqueReturnType)?;
                    let ordinal = self.value_node(Kind::OpaqueReturnTypeIndex, index)?;
                    node.push(ordinal);
                    return Some(node.build());
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
                if count == 0 {
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
        if size == 0 {
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
        let mut block = self.builder(kind)?;
        if throws {
            let node = self.node(Kind::ThrowsAnnotation)?;
            block.push(node);
        }
        if is_async {
            let node = self.node(Kind::AsyncAnnotation)?;
            block.push(node);
        }
        if concurrent {
            let node = self.node(Kind::ConcurrentFunctionType)?;
            block.push(node);
        }
        let arguments = self.with_children(Kind::ArgumentTuple, &[args])?;
        block.push(arguments);
        let returns = self.with_children(Kind::ReturnType, &[result])?;
        block.push(returns);
        Some(block.build())
    }

    /// `impl-function-type ::= impl-callee-convention impl-function-attribute*
    /// generic-signature? '_' impl-parameter* '_' impl-result* '_'`
    fn impl_function_type(&mut self, depth: u16) -> Option<NodeRef<'a>> {
        if self.over_depth(depth) {
            return None;
        }
        let mut node = self.builder(Kind::ImplFunctionType)?;
        let callee = self.impl_callee_convention()?;
        node.push(callee);
        if self.eat(b'C') {
            let flag = match self.take()? {
                b'b' => b'B',
                b'c' => b'C',
                b'm' => b'M',
                b'O' => b'O',
                b'w' => b'W',
                _ => return None,
            };
            let convention = self.builder_impl(Kind::ImplFunctionConvention, flag)?;
            node.push(convention.build());
        }
        if self.eat(b'h') {
            let attribute = self.node(Kind::ImplSendable)?;
            node.push(attribute);
        }
        if self.eat(b'H') {
            let attribute = self.node(Kind::ImplAsync)?;
            node.push(attribute);
        }
        if self.eat(b'G') || self.eat(b'g') {
            let signature = self.generic_signature(depth + 1)?;
            node.push(signature);
        }
        if !self.eat(b'_') {
            return None;
        }
        while !self.eat(b'_') {
            let parameter = self.impl_parameter(Kind::ImplParameter, depth + 1)?;
            node.push(parameter);
        }
        while !self.eat(b'_') {
            let result = self.impl_parameter(Kind::ImplResult, depth + 1)?;
            node.push(result);
        }
        Some(node.build())
    }

    fn impl_callee_convention(&mut self) -> Option<NodeRef<'a>> {
        let flag = if self.eat(b't') {
            b't'
        } else {
            self.impl_convention(CalleeContext::Callee)?
        };
        Some(self.builder_impl(Kind::ImplCalleeConvention, flag)?.build())
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
        let convention = self.builder_impl(Kind::ImplConvention, flag)?;
        self.with_children(kind, &[convention.build(), ty])
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
        if self.over_depth(depth) {
            return None;
        }
        let mut signature = self.builder(Kind::DependentGenericSignature)?;
        let mut count = None;
        while !matches!(self.peek(), Some(b'R' | b'r')) {
            let value = if self.eat(b'z') {
                0
            } else {
                self.index()?.saturating_add(1)
            };
            let node = self.value_node(Kind::DependentGenericParamCount, value)?;
            signature.push(node);
            count = Some(value);
        }
        if count.is_none() {
            let node = self.value_node(Kind::DependentGenericParamCount, 1)?;
            signature.push(node);
        }
        if self.eat(b'r') {
            return Some(signature.build());
        }
        if !self.eat(b'R') {
            return None;
        }
        while !self.eat(b'r') {
            let requirement = self.generic_requirement(depth + 1)?;
            signature.push(requirement);
        }
        Some(signature.build())
    }

    fn generic_requirement(&mut self, depth: u16) -> Option<NodeRef<'a>> {
        let constrained = self.constrained_type(depth + 1)?;
        if self.eat(b'z') {
            let other = self.ty(depth + 1)?;
            return self.with_children(
                Kind::DependentGenericSameTypeRequirement,
                &[constrained, other],
            );
        }
        if self.eat(b'l') {
            return self.layout_requirement(constrained, depth + 1);
        }
        let constraint = match self.peek()? {
            b'C' => self.ty(depth + 1)?,
            b'S' => {
                let _ = self.take()?;
                let sub = self.substitution_index(depth + 1)?;
                let type_name = match sub.kind() {
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

    fn layout_requirement(&mut self, constrained: NodeRef<'a>, depth: u16) -> Option<NodeRef<'a>> {
        if self.over_depth(depth) {
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
        let mut node = self.builder_layout(code)?;
        node.push(constrained);
        if let Some(size) = size {
            let size = self.value_node(Kind::Number, size)?;
            node.push(size);
        }
        if let Some(alignment) = alignment {
            let alignment = self.value_node(Kind::Number, alignment)?;
            node.push(alignment);
        }
        Some(node.build())
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
        if self.over_depth(depth) {
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

    fn generic_param(&mut self, depth: u32, index: u32) -> Option<NodeRef<'a>> {
        let mut node = self.builder_value(Kind::DependentGenericParamType, depth)?;
        node.set_len(index);
        Some(node.build())
    }

    fn dependent_type(&mut self, depth: u16) -> Option<NodeRef<'a>> {
        if self.over_depth(depth) || self.at_end() {
            return None;
        }
        let byte = self.peek()?;
        if byte != b'd' && byte != b'_' && !byte.is_ascii_digit() {
            let base = self.ty(depth + 1)?;
            return self.dependent_member_type_name(base, depth + 1);
        }
        self.generic_param_index(depth + 1)
    }

    fn dependent_member_type_name(&mut self, base: NodeRef<'a>, depth: u16) -> Option<NodeRef<'a>> {
        let assoc = if self.eat(b'S') {
            let assoc = self.substitution_index(depth + 1)?;
            if assoc.kind() != Kind::DependentAssociatedTypeRef {
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
            let assoc =
                self.with_children_opt(Kind::DependentAssociatedTypeRef, &[Some(name), protocol])?;
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
        let mut list = self.builder(Kind::TypeList)?;
        while !self.eat(b'_') {
            let proto = self.protocol_name(depth + 1)?;
            list.push(proto);
        }
        let list = list.build();
        self.with_children(Kind::ProtocolListType, &[list])
    }

    /// `tuple ::= 'T' tuple-element* '_'` (variadic with `'t'`)
    fn tuple_type(&mut self, variadic: bool, depth: u16) -> Option<NodeRef<'a>> {
        let mut elements = Vec::new();
        while !self.eat(b'_') {
            if self.at_end() {
                return None;
            }
            let mut element = self.builder(Kind::TupleElement)?;
            if self.peek().is_some_and(is_start_of_identifier) {
                let label = self.identifier(depth + 1, Some(Kind::TupleElementName))?;
                element.push(label);
            }
            let ty = self.ty(depth + 1)?;
            element.push(ty);
            elements.push(element.build());
        }
        if variadic && let Some(element) = elements.last().cloned() {
            // The printer renders the variadic marker only as the first child.
            let mut rebuilt = self.builder(Kind::TupleElement)?;
            let marker = self.node(Kind::VariadicMarker)?;
            rebuilt.push(marker);
            for child in element.children() {
                rebuilt.push(child.clone());
            }
            let last = elements.len() - 1;
            elements[last] = rebuilt.build();
        }
        let mut tuple = self.builder(Kind::Tuple)?;
        for element in elements {
            tuple.push(element);
        }
        Some(tuple.build())
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

const fn standard_substitution(code: u8) -> Option<StandardSubstitution> {
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

const fn is_start_of_identifier(byte: u8) -> bool {
    byte.is_ascii_digit() || byte == b'o'
}

const fn is_start_of_entity(byte: u8) -> bool {
    matches!(
        byte,
        b'F' | b'I' | b'v' | b'P' | b's' | b'Z' | b'C' | b'V' | b'O'
    )
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
        assert_eq!(
            value_witness_name(Some(b'a'), Some(b'l')),
            Some("allocateBuffer")
        );
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

    #[test]
    fn typed_payloads_replace_the_old_codes() {
        use crate::swift::node::LayoutCode;

        fn payload_of<T>(symbol: &str, kind: Kind, read: impl Fn(NodeRef<'_>) -> T) -> Option<T> {
            let ast = crate::swift::parse(symbol).ok()?;
            let mut stack = alloc::vec![ast.root()];
            let mut found = None;
            while let Some(node) = stack.pop() {
                if node.kind() == kind {
                    found = Some(read(node.clone()));
                }
                stack.extend(node.children().iter().cloned());
            }
            found
        }

        for (symbol, expected) in [
            ("_TFC3foo3bard", DeinitializerKind::Deinit),
            ("_TFC3foo3barD", DeinitializerKind::Deallocating),
            ("_TFC3foo3barZ", DeinitializerKind::IsolatedDeallocating),
        ] {
            let found = payload_of(symbol, Kind::Deinitializer, |node| {
                node.deinitializer_kind()
            });
            assert_eq!(found, Some(expected), "{symbol}");
        }

        for (symbol, expected) in [
            ("_TtuRxlUrFxwx5Mince", LayoutCode::UnknownLayout),
            ("_TtuRxlRrFxwx5Mince", LayoutCode::RefCountedObject),
            ("_TtuRxlNrFxwx5Mince", LayoutCode::NativeRefCountedObject),
            ("_TtuRxle64rFxwx5Mince", LayoutCode::TrivialExactSize),
            (
                "_TtuRxlE64_16rFxwx5Mince",
                LayoutCode::TrivialExactSizeAndAlignment,
            ),
            ("_TtuRxlm64rFxwx5Mince", LayoutCode::TrivialAtMostSize),
            (
                "_TtuRxlM64_16rFxwx5Mince",
                LayoutCode::TrivialAtMostSizeAndAlignment,
            ),
        ] {
            let found = payload_of(symbol, Kind::DependentGenericLayoutRequirement, |node| {
                node.layout_code()
            });
            assert_eq!(found, Some(expected), "{symbol}");
        }

        // Letters the old grammar never accepted stay rejected, even though
        // the typed enum knows them.
        for symbol in ["_TtuRxlCrFxwx5Mince", "_TtuRxlDrFxwx5Mince"] {
            assert_eq!(render(symbol), None, "{symbol}");
        }
    }
}
