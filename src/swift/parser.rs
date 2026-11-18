//! Swift mangling parser.
//!
//! Mirrors the reference demangler's operator machine: a postfix stream of
//! operators, each popping operands from a small stack and pushing a node.
//! No allocation: nodes live in a fixed arena, identifiers in a fixed buffer.

use crate::swift::node::{Arena, Global, Kind, NIL, NodeId, SUB_CAP, accessor_code};

/// Maximum operator stack depth.
const STACK_CAP: usize = 128;

/// Maximum recursion depth.
const DEPTH_CAP: u8 = 96;

/// Parsing failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ParseError;

/// Result alias for the parser.
pub(crate) type PResult<T = NodeId> = Result<T, ParseError>;

/// Swift demangling parser over a mangled name (prefix already stripped).
pub(crate) struct Parser<'a> {
    text: &'a [u8],
    pos: usize,
    pub(crate) arena: Arena,
    stack: [NodeId; STACK_CAP],
    stack_len: usize,
    depth: u8,
    /// Remaining work budget (guards against pathological inputs).
    budget: u32,
    /// Byte offset where parsing failed (diagnostics only).
    pub(crate) fail_pos: usize,
    /// Operator byte that failed (diagnostics only).
    pub(crate) fail_byte: u8,
    /// `_T` prefixes use the pre-stable function type layout, where parameter
    /// labels are embedded in tuple elements and no label list is popped.
    old_function_type: bool,
}

impl<'a> Parser<'a> {
    /// Create a parser for `text`, which must not contain the mangling prefix.
    pub(crate) fn new(text: &'a [u8], old_function_type: bool) -> Self {
        Self {
            text,
            pos: 0,
            arena: Arena::new(),
            stack: [NIL; STACK_CAP],
            stack_len: 0,
            depth: 0,
            budget: 200_000,
            fail_pos: 0,
            fail_byte: 0,
            old_function_type,
        }
    }

    /// Parse a complete mangled name and assemble its top-level node.
    pub(crate) fn parse(&mut self) -> PResult {
        let mut last_pos = usize::MAX;
        let mut stalled = 0_u8;
        while self.pos < self.text.len() {
            if self.pos == last_pos {
                stalled = stalled.saturating_add(1);
                if stalled > 4 {
                    return Err(ParseError);
                }
            } else {
                stalled = 0;
                last_pos = self.pos;
            }
            if self.peek() == Some(b'.') {
                let rest = core::str::from_utf8(&self.text[self.pos..]).map_err(|_| ParseError)?;
                let node = self
                    .arena
                    .alloc_text(Kind::Suffix, rest)
                    .ok_or(ParseError)?;
                self.push(node)?;
                self.pos = self.text.len();
                break;
            }
            let node = self.operator()?;
            self.push(node)?;
        }
        self.finish()
    }

    /// Wrap the nodes left on the stack in a top-level node.
    fn finish(&mut self) -> PResult {
        let suffix = self.pop_kind(Kind::Suffix);
        let top = self.alloc(Kind::TopLevel)?;
        let mut parent = top;
        let mut attributes = [NIL; 16];
        let mut attribute_count = 0_usize;
        while self.stack_len > 0 {
            let id = self.stack[self.stack_len - 1];
            let kind = self.arena.kind(id);
            let is_attribute = kind == Kind::Attribute
                || kind == Kind::GenericSpecialization
                || kind == Kind::FunctionSignatureSpecialization;
            if !is_attribute && !self.is_function_attr(id) {
                break;
            }
            self.stack_len -= 1;
            if attribute_count >= attributes.len() {
                return Err(ParseError);
            }
            attributes[attribute_count] = id;
            attribute_count += 1;
        }
        for &attribute in attributes.iter().take(attribute_count) {
            self.arena.add_child(parent, attribute);
            if self.arena.kind(attribute) == Kind::Global
                && matches!(
                    Global::from_code(self.arena.node(attribute).flags),
                    Some(Global::PartialApplyForwarder | Global::PartialApplyObjcForwarder)
                )
            {
                parent = attribute;
            }
        }
        let mut index = 0_usize;
        while index < self.stack_len {
            let node = self.stack[index];
            index += 1;
            if self.arena.kind(node) == Kind::Type {
                let child = self.arena.child(node, 0);
                self.arena.add_child(parent, child);
            } else {
                self.arena.add_child(parent, node);
            }
        }
        if let Some(suffix) = suffix {
            self.arena.add_child(top, suffix);
        }
        if self.arena.child_count(top) == 0 {
            return Err(ParseError);
        }
        Ok(top)
    }

    /// True for nodes that print as prefixes of the symbol that follows them.
    fn is_function_attr(&self, id: NodeId) -> bool {
        match self.arena.kind(id) {
            Kind::Attribute
            | Kind::GenericSpecialization
            | Kind::FunctionSignatureSpecialization => true,
            Kind::Global => matches!(
                Global::from_code(self.arena.node(id).flags),
                Some(
                    Global::PartialApplyForwarder
                        | Global::PartialApplyObjcForwarder
                        | Global::OutlinedVariable
                        | Global::OutlinedReadOnlyObject
                        | Global::OutlinedBridgedMethod
                        | Global::MergedFunction
                        | Global::DistributedThunk
                        | Global::DistributedAccessor
                        | Global::DynamicallyReplaceableFunctionVar
                        | Global::DynamicallyReplaceableFunctionKey
                        | Global::DynamicallyReplaceableFunctionImpl
                        | Global::AsyncFunctionPointer
                        | Global::CoroFunctionPointer
                        | Global::BackDeploymentThunk
                        | Global::BackDeploymentFallback
                        | Global::BackDeploymentCopy
                        | Global::HasSymbolQuery
                        | Global::ResumePartialFunction
                        | Global::DirectMethodReferenceThunk
                        | Global::DefaultOverride
                )
            ),
            _ => false,
        }
    }

    // ---- input helpers -------------------------------------------------

    fn peek(&self) -> Option<u8> {
        self.text.get(self.pos).copied()
    }

    fn next_byte(&mut self) -> Option<u8> {
        self.budget = self.budget.checked_sub(1)?;
        let byte = self.peek()?;
        self.pos += 1;
        Some(byte)
    }

    fn next_if(&mut self, byte: u8) -> bool {
        if self.peek() == Some(byte) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn back(&mut self) {
        self.pos = self.pos.saturating_sub(1);
    }

    fn natural(&mut self) -> Option<u32> {
        let start = self.pos;
        let mut value: u32 = 0;
        while let Some(byte) = self.peek() {
            if !byte.is_ascii_digit() {
                break;
            }
            value = value.checked_mul(10)?.checked_add(u32::from(byte - b'0'))?;
            self.pos += 1;
        }
        if self.pos == start { None } else { Some(value) }
    }

    /// Index encoding: `_` = 0, `N_` = N + 1.
    fn index(&mut self) -> Option<u32> {
        if self.next_if(b'_') {
            return Some(0);
        }
        let value = self.natural()?;
        if self.next_if(b'_') {
            value.checked_add(1)
        } else {
            None
        }
    }

    // ---- stack helpers -------------------------------------------------

    fn push(&mut self, id: NodeId) -> PResult<()> {
        if self.stack_len >= STACK_CAP || id == NIL {
            return Err(ParseError);
        }
        self.stack[self.stack_len] = id;
        self.stack_len += 1;
        Ok(())
    }

    fn pop_any(&mut self) -> Option<NodeId> {
        if self.stack_len == 0 {
            return None;
        }
        self.stack_len -= 1;
        Some(self.stack[self.stack_len])
    }

    fn pop_kind(&mut self, kind: Kind) -> Option<NodeId> {
        if self.stack_len == 0 {
            return None;
        }
        let id = self.stack[self.stack_len - 1];
        if self.arena.kind(id) == kind {
            self.stack_len -= 1;
            Some(id)
        } else {
            None
        }
    }

    fn pop_type(&mut self) -> Option<NodeId> {
        self.pop_kind(Kind::Type)
    }

    fn pop_type_child(&mut self) -> Option<NodeId> {
        let ty = self.pop_type()?;
        let child = self.arena.child(ty, 0);
        (child != NIL).then_some(child)
    }

    fn pop_any_generic(&mut self) -> Option<NodeId> {
        let ty = self.pop_type()?;
        let child = self.arena.child(ty, 0);
        if child != NIL && self.arena.kind(child).is_nominal() {
            Some(child)
        } else {
            None
        }
    }

    fn pop_module(&mut self) -> Option<NodeId> {
        if let Some(id) = self.pop_kind(Kind::Identifier) {
            return self.arena.change_kind(id, Kind::Module);
        }
        self.pop_kind(Kind::Module)
    }

    fn pop_context(&mut self) -> Option<NodeId> {
        if let Some(module) = self.pop_module() {
            return Some(module);
        }
        if let Some(id) = self.pop_any() {
            let kind = self.arena.kind(id);
            if kind == Kind::Type {
                let child = self.arena.child(id, 0);
                if child != NIL && self.arena.kind(child).is_context() {
                    return Some(child);
                }
                self.stack_len += 1;
                return None;
            }
            if kind.is_context() {
                return Some(id);
            }
            self.stack_len += 1;
        }
        None
    }

    fn pop_context_any(&mut self) -> Option<NodeId> {
        if let Some(module) = self.pop_module() {
            return Some(module);
        }
        if let Some(id) = self.pop_any() {
            let kind = self.arena.kind(id);
            if kind == Kind::Type {
                let child = self.arena.child(id, 0);
                if child != NIL && self.arena.kind(child).is_context() {
                    return Some(child);
                }
                self.stack_len += 1;
                return None;
            }
            if kind.is_context() {
                return Some(id);
            }
            self.stack_len += 1;
        }
        None
    }

    /// Pop an associated type name (`<protocol>.<name>` or bare name).
    fn pop_assoc_type_name(&mut self) -> Option<NodeId> {
        let protocol = if let Some(ty) = self.pop_type() {
            let child = self.arena.child(ty, 0);
            if child != NIL && self.arena.kind(child) == Kind::Protocol {
                Some(ty)
            } else {
                self.stack_len += 1;
                None
            }
        } else {
            None
        };
        let name = self.pop_kind(Kind::Identifier)?;
        let node = self
            .node_with(Kind::DependentAssociatedTypeRef, &[name, protocol.unwrap_or(NIL)])
            .ok()?;
        Some(node)
    }

    fn pop_assoc_type_path(&mut self) -> Option<NodeId> {
        let path = self.alloc(Kind::AssocTypePath).ok()?;
        let mut elements = [NIL; 16];
        let mut count = 0_usize;
        loop {
            let first = self.pop_kind(Kind::FirstElementMarker).is_some();
            let assoc = self.pop_assoc_type_name()?;
            if count >= elements.len() {
                return None;
            }
            elements[count] = assoc;
            count += 1;
            if first {
                break;
            }
        }
        for index in (0..count).rev() {
            self.arena.add_child(path, elements[index]);
        }
        Some(path)
    }

    fn pop_assoc_conformance_subject(&mut self) -> Option<NodeId> {
        if let Some(ty) = self.pop_type() {
            let child = self.arena.child(ty, 0);
            if child != NIL
                && matches!(
                    self.arena.kind(child),
                    Kind::DependentGenericParamType | Kind::DependentMemberType
                )
            {
                return Some(ty);
            }
            self.stack_len += 1;
        }
        self.pop_assoc_type_path()
    }

    /// Pop a protocol nominally, accepting an already-built protocol type.
    fn pop_protocol(&mut self) -> Option<NodeId> {
        if let Some(id) = self.pop_any() {
            if self.arena.kind(id) == Kind::Type {
                let child = self.arena.child(id, 0);
                if child != NIL && self.arena.kind(child) == Kind::Protocol {
                    return Some(id);
                }
                self.stack_len += 1;
                return None;
            }
            if self.arena.kind(id).is_nominal() {
                return Some(id);
            }
            self.stack_len += 1;
        }
        let name = self.pop_decl_name()?;
        let context = self.pop_context()?;
        let protocol = self
            .node_with(Kind::Protocol, &[context, name])
            .ok()?;
        self.wrap_type(protocol).ok()
    }

    /// Pop a protocol conformance record.
    fn pop_protocol_conformance(&mut self) -> Option<NodeId> {
        let signature = self.pop_kind(Kind::DependentGenericSignature);
        let module = self.pop_module()?;
        let protocol = self.pop_protocol()?;
        let ty = self.pop_type()?;
        let node = self.alloc(Kind::ProtocolConformance).ok()?;
        if let Some(signature) = signature {
            let signature = self
                .typed(Kind::DependentGenericType, &[signature, ty])
                .ok()?;
            self.arena.add_child(node, signature);
        } else {
            self.arena.add_child(node, ty);
        }
        self.arena.add_child(node, protocol);
        self.arena.add_child(node, module);
        Some(node)
    }

    fn wrap_type(&mut self, child: NodeId) -> PResult {
        let ty = self.arena.alloc(Kind::Type).ok_or(ParseError)?;
        self.arena.add_child(ty, child);
        Ok(ty)
    }

    fn alloc(&mut self, kind: Kind) -> PResult {
        self.arena.alloc(kind).ok_or(ParseError)
    }

    fn alloc_text(&mut self, kind: Kind, text: &str) -> PResult {
        self.arena.alloc_text(kind, text).ok_or(ParseError)
    }

    fn node_with(&mut self, kind: Kind, children: &[NodeId]) -> PResult {
        let node = self.alloc(kind)?;
        for &child in children {
            if child != NIL {
                self.arena.add_child(node, child);
            }
        }
        Ok(node)
    }

    fn typed(&mut self, kind: Kind, children: &[NodeId]) -> PResult {
        let node = self.node_with(kind, children)?;
        self.wrap_type(node)
    }

    fn with_children(&mut self, kind: Kind, children: &[NodeId]) -> PResult {
        self.node_with(kind, children)
    }

    // ---- operators -----------------------------------------------------

    fn enter(&mut self) -> PResult<()> {
        self.depth = self.depth.checked_add(1).ok_or(ParseError)?;
        if self.depth > DEPTH_CAP {
            return Err(ParseError);
        }
        Ok(())
    }

    fn leave(&mut self) {
        self.depth = self.depth.saturating_sub(1);
    }

    fn operator(&mut self) -> PResult {
        self.enter()?;
        let start = self.pos;
        let result = self.operator_inner();
        if result.is_err() {
            self.fail_pos = start;
            self.fail_byte = self.text.get(start).copied().unwrap_or(0);
        }
        self.leave();
        result
    }

    fn operator_inner(&mut self) -> PResult {
        let Some(byte) = self.next_byte() else {
            return Err(ParseError);
        };
        match byte {
            b'A' => self.multi_substitutions(),
            b'B' => self.builtin_type(),
            b'C' => self.any_generic_type(Kind::Class),
            b'D' => self.type_mangling(),
            b'E' => self.extension_context(),
            b'F' => self.plain_function(),
            b'G' => self.bound_generic_type(),
            b'H' => self.runtime_record(),
            b'I' => self.impl_function_type(),
            b'K' => Ok(self.alloc(Kind::ThrowsAnnotation)?),
            b'L' => self.local_identifier(),
            b'R' => self.generic_requirement(),
            b'l' => self.generic_signature(false),
            b'r' => self.generic_signature(true),
            b'M' => self.metatype_global(),
            b'N' => self.type_metadata(),
            b'O' => self.any_generic_type(Kind::Enum),
            b'P' => self.any_generic_type(Kind::Protocol),
            b'Q' => self.archetype(),
            b'S' => self.standard_substitution(),
            b'T' => self.thunk_or_specialization(),
            b'V' => self.any_generic_type(Kind::Structure),
            b'W' => self.witness(),
            b'X' => self.special_type(),
            b'Y' => self.type_annotation(),
            b'Z' => self.static_entity(),
            b'a' => self.any_generic_type(Kind::TypeAlias),
            b'c' => self.function_type(Kind::FunctionType),
            b'd' => Ok(self.alloc(Kind::VariadicMarker)?),
            b'f' => self.function_entity(),
            b'h' => self.type_wrapper(Kind::Shared),
            b'i' => self.subscript(),
            b'm' => self.metatype_type(),
            b'n' => self.type_wrapper(Kind::Owned),
            b'o' => self.operator_identifier(),
            b'p' => self.protocol_list_type(),
            b'q' => self.generic_param_typed(),
            b's' => self.alloc_text(Kind::Module, "Swift"),
            b't' => self.tuple_type(),
            b'u' => self.generic_type(),
            b'v' => self.variable(),
            b'w' => self.value_witness(),
            b'x' => self.builtin_generic_param(0, 0),
            b'y' => Ok(self.alloc(Kind::EmptyList)?),
            b'z' => self.type_wrapper(Kind::InOut),
            b'_' => Ok(self.alloc(Kind::FirstElementMarker)?),
            b'$' => self.integer_type(),
            _ => {
                self.back();
                self.identifier()
            }
        }
    }

    fn identifier(&mut self) -> PResult {
        let first = self.peek().ok_or(ParseError)?;
        if !first.is_ascii_digit() {
            return Err(ParseError);
        }
        let mut word_substs = false;
        let mut punycoded = false;
        if self.next_if(b'0') {
            if self.next_if(b'0') {
                punycoded = true;
            } else {
                word_substs = true;
            }
        }
        if punycoded {
            // Punycode decoding is not implemented yet.
            return Err(ParseError);
        }
        let mut assembled: [u8; 512] = [0; 512];
        let mut len = 0_usize;
        loop {
            while word_substs {
                let Some(byte) = self.peek() else { break };
                if !byte.is_ascii_alphabetic() {
                    break;
                }
                self.pos += 1;
                let (index, last) = if byte.is_ascii_lowercase() {
                    (usize::from(byte - b'a'), false)
                } else {
                    (usize::from(byte - b'A'), true)
                };
                if index >= self.arena.word_count() {
                    return Err(ParseError);
                }
                let word = self.arena.word(index);
                let bytes = word.as_bytes();
                let end = len.checked_add(bytes.len()).ok_or(ParseError)?;
                if end > assembled.len() {
                    return Err(ParseError);
                }
                assembled[len..end].copy_from_slice(bytes);
                len = end;
                if last {
                    word_substs = false;
                }
            }
            if self.next_if(b'0') {
                break;
            }
            let count = self.natural().ok_or(ParseError)?;
            if count == 0 {
                return Err(ParseError);
            }
            let count = usize::try_from(count).map_err(|_| ParseError)?;
            let end = self.pos.checked_add(count).ok_or(ParseError)?;
            if end > self.text.len() {
                return Err(ParseError);
            }
            let slice = &self.text[self.pos..end];
            self.pos = end;
            let dst = len.checked_add(slice.len()).ok_or(ParseError)?;
            if dst > assembled.len() {
                return Err(ParseError);
            }
            assembled[len..dst].copy_from_slice(slice);
            len = dst;
            // Record words from the literal slice.
            self.collect_words(slice)?;
            if !word_substs {
                break;
            }
        }
        if len == 0 {
            return Err(ParseError);
        }
        let text = core::str::from_utf8(&assembled[..len]).map_err(|_| ParseError)?;
        let node = self.alloc_text(Kind::Identifier, text)?;
        self.arena.add_sub(node);
        Ok(node)
    }

    fn collect_words(&mut self, slice: &[u8]) -> Result<(), ParseError> {
        // A word is alphanumeric; a new word starts after '_' or at the start,
        // or at an uppercase letter whose predecessor is not uppercase.
        let mut start: Option<usize> = None;
        for (index, &byte) in slice.iter().enumerate() {
            let previous_upper = index > 0 && slice[index - 1].is_ascii_uppercase();
            let starts_word = byte.is_ascii_alphanumeric()
                && match start {
                    None => true,
                    Some(_) => {
                        byte.is_ascii_uppercase() && !previous_upper && slice[index - 1] != b'_'
                    }
                };
            if starts_word && start.is_none() {
                start = Some(index);
            }
            if let Some(begin) = start {
                let ends = !byte.is_ascii_alphanumeric()
                    || (index + 1 < slice.len()
                        && slice[index + 1].is_ascii_uppercase()
                        && !byte.is_ascii_uppercase());
                if ends {
                    let word_len = index - begin;
                    if word_len >= 2 {
                        let (offset, l) = self
                            .arena
                            .push_text(&slice[begin..index])
                            .ok_or(ParseError)?;
                        self.arena.add_word(offset, l);
                    }
                    start = None;
                }
            }
        }
        if let Some(begin) = start {
            let word_len = slice.len() - begin;
            if word_len >= 2 {
                let (offset, l) = self.arena.push_text(&slice[begin..]).ok_or(ParseError)?;
                self.arena.add_word(offset, l);
            }
        }
        Ok(())
    }

    fn multi_substitutions(&mut self) -> PResult {
        let mut repeat: Option<u32> = None;
        loop {
            let byte = self.next_byte().ok_or(ParseError)?;
            if byte.is_ascii_lowercase() {
                let index = usize::from(byte - b'a');
                let node = self.substitution(index)?;
                self.push_repeated(node, repeat.take())?;
                continue;
            }
            if byte.is_ascii_uppercase() {
                let index = usize::from(byte - b'A');
                let node = self.substitution(index)?;
                // The terminating substitution is pushed by the caller; only
                // repetitions are pushed here.
                let times = match repeat.take() {
                    Some(count) if count > 1 => count - 1,
                    _ => 0,
                };
                for _ in 0..times {
                    self.push(node)?;
                }
                return Ok(node);
            }
            if byte == b'_' {
                let encoded = repeat.take().ok_or(ParseError)?;
                let index = usize::try_from(encoded + 27).map_err(|_| ParseError)?;
                return self.substitution(index);
            }
            self.back();
            repeat = Some(self.natural().ok_or(ParseError)?);
        }
    }

    fn push_repeated(&mut self, node: NodeId, repeat: Option<u32>) -> PResult<()> {
        let times = match repeat {
            Some(count) if count > 1 => count,
            _ => 1,
        };
        for _ in 0..times {
            self.push(node)?;
        }
        Ok(())
    }

    fn substitution(&self, index: usize) -> PResult {
        if index >= SUB_CAP {
            return Err(ParseError);
        }
        self.arena.sub(index).ok_or(ParseError)
    }

    fn standard_substitution(&mut self) -> PResult {
        match self.next_byte().ok_or(ParseError)? {
            b'o' => self.alloc_text(Kind::Module, "__C"),
            b'C' => self.alloc_text(Kind::Module, "__C_Synthesized"),
            b'g' => {
                let optional = self.swift_type(Kind::BoundGenericEnum, "Optional")?;
                let list = self.alloc(Kind::TypeList)?;
                if let Some(ty) = self.pop_type() {
                    self.arena.add_child(list, ty);
                }
                let node = self.with_children(Kind::BoundGenericEnum, &[optional, list])?;
                let ty = self.wrap_type(node)?;
                self.arena.add_sub(ty);
                Ok(ty)
            }
            _ => {
                self.back();
                let repeat = self.natural().unwrap_or(1).max(1);
                let second = self.next_if(b'c');
                let kind = self.next_byte().ok_or(ParseError)?;
                let name = standard_type_name(kind, second).ok_or(ParseError)?;
                let (kind, name) = name;
                let node = self.swift_type(kind, name)?;
                let ty = self.wrap_type(node)?;
                self.arena.add_sub(ty);
                for _ in 1..repeat {
                    self.push(ty)?;
                }
                Ok(ty)
            }
        }
    }

    fn swift_type(&mut self, kind: Kind, name: &str) -> PResult {
        let module = self.alloc_text(Kind::Module, "Swift")?;
        let ident = self.alloc_text(Kind::Identifier, name)?;
        self.with_children(kind, &[module, ident])
    }

    fn builtin_type(&mut self) -> PResult {
        let code = self.next_byte().ok_or(ParseError)?;
        match code {
            b'f' | b'i' => {
                let width = self.natural().ok_or(ParseError)?;
                if !self.next_if(b'_') {
                    return Err(ParseError);
                }
                let mut buf = [0_u8; 40];
                let prefix = if code == b'f' {
                    "Builtin.FPIEEE"
                } else {
                    "Builtin.Int"
                };
                let text = format_builtin(&mut buf, prefix, width);
                let node = self.alloc_text(Kind::BuiltinTypeName, text)?;
                self.wrap_type(node)
            }
            b'v' => {
                let count = self.natural().ok_or(ParseError)?;
                if !self.next_if(b'_') {
                    return Err(ParseError);
                }
                let element = self.pop_type_child().ok_or(ParseError)?;
                let mut buf = [0_u8; 40];
                let text = format_builtin(&mut buf, "Builtin.Vec", count);
                let node = self.alloc_text(Kind::BuiltinVector, text)?;
                self.arena.add_child(node, element);
                self.wrap_type(node)
            }
            b'V' => {
                let element = self.pop_type_child().ok_or(ParseError)?;
                let count = self.pop_type_child().ok_or(ParseError)?;
                let node = self.alloc(Kind::BuiltinVector)?;
                self.arena.add_child(node, count);
                self.arena.add_child(node, element);
                self.wrap_type(node)
            }
            _ => {
                let name = builtin_name(code).ok_or(ParseError)?;
                let node = self.alloc_text(Kind::BuiltinTypeName, name)?;
                self.wrap_type(node)
            }
        }
    }

    fn any_generic_type(&mut self, kind: Kind) -> PResult {
        let name = self.pop_decl_name().ok_or(ParseError)?;
        let context = self.pop_context().ok_or(ParseError)?;
        let node = self.typed(kind, &[context, name])?;
        self.arena.add_sub(node);
        Ok(node)
    }

    fn type_mangling(&mut self) -> PResult {
        let ty = self.pop_type().ok_or(ParseError)?;
        let node = self.node_with(Kind::TypeMangling, &[ty])?;
        Ok(node)
    }

    fn bound_generic_type(&mut self) -> PResult {
        // Collect type lists: 'y' terminates a level; '_' separates levels.
        let mut lists: [NodeId; 8] = [NIL; 8];
        let mut list_count = 0_usize;
        loop {
            let list = self.alloc(Kind::TypeList)?;
            let mut elements: [NodeId; 32] = [NIL; 32];
            let mut count = 0_usize;
            while let Some(ty) = self.pop_type() {
                if count >= elements.len() {
                    return Err(ParseError);
                }
                elements[count] = ty;
                count += 1;
            }
            // Elements were popped in reverse order; append reversed.
            for index in (0..count).rev() {
                self.arena.add_child(list, elements[index]);
            }
            if list_count >= lists.len() {
                return Err(ParseError);
            }
            lists[list_count] = list;
            list_count += 1;
            if self.pop_kind(Kind::EmptyList).is_some() {
                break;
            }
            if self.pop_kind(Kind::FirstElementMarker).is_none() {
                return Err(ParseError);
            }
        }
        let nominal = self.pop_any_generic().ok_or(ParseError)?;
        if list_count == 0 {
            return Err(ParseError);
        }
        let bound = self.bind_generic_args(nominal, &lists[..list_count], 0)?;
        let ty = self.wrap_type(bound)?;
        self.arena.add_sub(ty);
        Ok(ty)
    }

    /// Apply generic argument lists to nested nominal contexts.
    ///
    /// Arguments for the outermost type come first; each list is attached to
    /// the nominal it belongs to, rebuilding contexts from the outside in.
    fn bind_generic_args(
        &mut self,
        nominal: NodeId,
        lists: &[NodeId],
        mut index: usize,
    ) -> PResult {
        if index >= lists.len() {
            return Err(ParseError);
        }
        let kind = self.arena.kind(nominal);
        let args = lists[index];
        // Nominal types always consume their argument list.
        index += 1;
        let context = self.arena.child(nominal, 0);
        let name = self.arena.child(nominal, 1);
        let current = if index < lists.len() {
            if context == NIL {
                return Err(ParseError);
            }
            let parent = self.bind_generic_args(context, lists, index)?;
            let rebuilt = self.alloc(kind)?;
            self.arena.add_child(rebuilt, parent);
            self.arena.add_child(rebuilt, name);
            rebuilt
        } else {
            nominal
        };
        let bound_kind = match kind {
            Kind::Structure => Kind::BoundGenericStructure,
            Kind::Class => Kind::BoundGenericClass,
            Kind::Enum => Kind::BoundGenericEnum,
            Kind::Protocol => Kind::BoundGenericProtocol,
            _ => Kind::BoundGenericOtherNominal,
        };
        self.with_children(bound_kind, &[current, args])
    }

    fn function_type(&mut self, kind: Kind) -> PResult {
        self.function_type_node(kind)
    }

    fn function_type_node(&mut self, kind: Kind) -> PResult {
        // Components are pushed in the reverse of the mangling order.
        let sending = self.pop_kind(Kind::SendingResultFunctionType);
        let isolated = self.pop_any_of(&[
            Kind::IsolatedAnyFunctionType,
            Kind::NonIsolatedCallerFunctionType,
            Kind::GlobalActorFunctionType,
        ]);
        let differentiable = self.pop_kind(Kind::DifferentiableFunctionType);
        let throws = self.pop_any_of(&[Kind::ThrowsAnnotation, Kind::TypedThrowsAnnotation]);
        let sendable = self.pop_kind(Kind::ConcurrentFunctionType);
        let is_async = self.pop_kind(Kind::AsyncAnnotation);
        let params = self.function_params(Kind::ArgumentTuple)?;
        let result = self.function_params(Kind::ReturnType)?;
        let node = self.with_children(
            kind,
            &[
                sending.unwrap_or(NIL),
                isolated.unwrap_or(NIL),
                differentiable.unwrap_or(NIL),
                throws.unwrap_or(NIL),
                sendable.unwrap_or(NIL),
                is_async.unwrap_or(NIL),
                params,
                result,
            ],
        )?;
        self.wrap_type(node)
    }

    fn function_params(&mut self, kind: Kind) -> PResult {
        let param = if self.pop_kind(Kind::EmptyList).is_some() {
            let tuple = self.alloc(Kind::Tuple)?;
            self.wrap_type(tuple)?
        } else {
            self.pop_type().ok_or(ParseError)?
        };
        self.node_with(kind, &[param])
    }

    fn pop_any_of(&mut self, kinds: &[Kind]) -> Option<NodeId> {
        if self.stack_len == 0 {
            return None;
        }
        let id = self.stack[self.stack_len - 1];
        let kind = self.arena.kind(id);
        if kinds.contains(&kind) {
            self.stack_len -= 1;
            Some(id)
        } else {
            None
        }
    }

    fn plain_function(&mut self) -> PResult {
        let generic = self.pop_kind(Kind::DependentGenericSignature);
        let ty = self.function_type_node(Kind::FunctionType)?;
        let labels = self.pop_function_param_labels(ty)?;
        let ty = if let Some(generic) = generic {
            self.typed(Kind::DependentGenericType, &[generic, ty])?
        } else {
            ty
        };
        let name = self.pop_decl_name().ok_or(ParseError)?;
        let context = self.pop_context_any().ok_or(ParseError)?;
        self.with_children(Kind::Function, &[context, name, labels.unwrap_or(NIL), ty])
    }

    /// Pop the parameter labels of a newly parsed function type.
    ///
    /// New-style manglings encode labels as operands pushed before the type;
    /// they are popped here and stored in a [`Kind::LabelList`]. Old-style
    /// (`_T`) manglings embed the labels in the tuple elements instead.
    fn pop_function_param_labels(&mut self, ty: NodeId) -> PResult<Option<NodeId>> {
        if self.old_function_type {
            return Ok(None);
        }
        if self.pop_kind(Kind::EmptyList).is_some() {
            return Ok(Some(self.alloc(Kind::LabelList)?));
        }
        let mut func = self.arena.child(ty, 0);
        if func != NIL && self.arena.kind(func) == Kind::DependentGenericType {
            let inner = self.arena.child(func, 1);
            func = if self.arena.kind(inner) == Kind::Type {
                self.arena.child(inner, 0)
            } else {
                inner
            };
        }
        if func == NIL {
            return Ok(None);
        }
        let args = self.find_child(func, Kind::ArgumentTuple).ok_or(ParseError)?;
        let params = self.arena.child(args, 0);
        let params = if self.arena.kind(params) == Kind::Type {
            self.arena.child(params, 0)
        } else {
            params
        };
        let count = if self.arena.kind(params) == Kind::Tuple {
            self.arena.child_count(params)
        } else {
            1
        };
        if count == 0 {
            return Ok(None);
        }
        let mut labels = [NIL; 64];
        let mut n = 0_usize;
        while n < count {
            if n >= labels.len() {
                return Err(ParseError);
            }
            let label = self.pop_any().ok_or(ParseError)?;
            match self.arena.kind(label) {
                Kind::Identifier | Kind::FirstElementMarker => {}
                _ => return Err(ParseError),
            }
            labels[n] = label;
            n += 1;
        }
        let list = self.alloc(Kind::LabelList)?;
        let has_labels = labels[..count]
            .iter()
            .any(|&label| self.arena.kind(label) == Kind::Identifier);
        if !has_labels {
            return Ok(Some(list));
        }
        for index in (0..count).rev() {
            self.arena.add_child(list, labels[index]);
        }
        Ok(Some(list))
    }

    fn find_child(&self, parent: NodeId, kind: Kind) -> Option<NodeId> {
        let mut child = self.arena.node(parent).first;
        let mut guard = 0_usize;
        while child != NIL {
            guard += 1;
            if guard > crate::swift::node::NODE_CAP {
                return None;
            }
            if self.arena.kind(child) == kind {
                return Some(child);
            }
            child = self.arena.node(child).next;
        }
        None
    }

    fn pop_decl_name(&mut self) -> Option<NodeId> {
        let id = self.pop_any()?;
        if self.arena.kind(id).is_decl_name() {
            Some(id)
        } else {
            self.stack_len += 1;
            None
        }
    }

    fn extension_context(&mut self) -> PResult {
        let generic = self.pop_kind(Kind::DependentGenericSignature);
        let module = self.pop_module().ok_or(ParseError)?;
        let ty = self.pop_any_generic().ok_or(ParseError)?;
        let node = if let Some(generic) = generic {
            self.with_children(Kind::Extension, &[module, ty, generic])?
        } else {
            self.with_children(Kind::Extension, &[module, ty])?
        };
        Ok(node)
    }

    fn generic_type(&mut self) -> PResult {
        let ty = self.pop_type().ok_or(ParseError)?;
        if self.pop_kind(Kind::EmptyList).is_some() {
            return Ok(ty);
        }
        let node = self.typed(Kind::Type, &[ty])?;
        Ok(node)
    }

    fn generic_signature(&mut self, has_counts: bool) -> PResult {
        let sig = self.alloc(Kind::DependentGenericSignature)?;
        if has_counts {
            while !self.next_if(b'l') {
                let count = if self.next_if(b'z') {
                    0
                } else {
                    self.index().ok_or(ParseError)?.saturating_add(1)
                };
                let node = self
                    .arena
                    .alloc_value(Kind::DependentGenericParamCount, count);
                self.arena.add_child(sig, node.ok_or(ParseError)?);
            }
        } else {
            let node = self
                .arena
                .alloc_value(Kind::DependentGenericParamCount, 1)
                .ok_or(ParseError)?;
            self.arena.add_child(sig, node);
        }
        // Requirements are already on the stack in mangling order; move them
        // into the signature in reverse to restore source order.
        let mut requirements = [NIL; 32];
        let mut count = 0_usize;
        while let Some(node) = self.pop_requirement() {
            if count >= requirements.len() {
                return Err(ParseError);
            }
            requirements[count] = node;
            count += 1;
        }
        for index in (0..count).rev() {
            self.arena.add_child(sig, requirements[index]);
        }
        Ok(sig)
    }

    fn pop_requirement(&mut self) -> Option<NodeId> {
        if self.stack_len == 0 {
            return None;
        }
        let id = self.stack[self.stack_len - 1];
        if matches!(
            self.arena.kind(id),
            Kind::DependentGenericConformanceRequirement
                | Kind::DependentGenericSameTypeRequirement
                | Kind::DependentGenericBaseClassRequirement
                | Kind::DependentGenericLayoutRequirement
        ) {
            self.stack_len -= 1;
            Some(id)
        } else {
            None
        }
    }

    fn generic_requirement(&mut self) -> PResult {
        let code = self.next_byte().ok_or(ParseError)?;
        let (kind, param_ty, other) = match code {
            b's' => {
                let other = self.pop_type().ok_or(ParseError)?;
                let param = self.generic_param()?;
                (Kind::DependentGenericSameTypeRequirement, param, other)
            }
            b'b' => {
                let other = self.pop_type().ok_or(ParseError)?;
                let param = self.generic_param()?;
                (Kind::DependentGenericBaseClassRequirement, param, other)
            }
            b'l' => {
                let layout = self.next_byte().ok_or(ParseError)?;
                let param = self.generic_param()?;
                let node =
                    self.with_children(Kind::DependentGenericLayoutRequirement, &[param, NIL])?;
                self.arena.node_mut(node).flags = layout;
                return Ok(node);
            }
            b'h' => {
                let param = self.generic_param()?;
                let node =
                    self.with_children(Kind::DependentGenericBaseClassRequirement, &[param, NIL])?;
                return Ok(node);
            }
            _ => {
                self.back();
                let protocol = self
                    .pop_any_generic()
                    .or_else(|| self.pop_kind(Kind::Protocol))
                    .ok_or(ParseError)?;
                let param = self.generic_param()?;
                (
                    Kind::DependentGenericConformanceRequirement,
                    param,
                    protocol,
                )
            }
        };
        self.with_children(kind, &[param_ty, other])
    }

    fn generic_param_typed(&mut self) -> PResult {
        let node = self.generic_param()?;
        self.wrap_type(node)
    }

    fn generic_param(&mut self) -> PResult {
        if self.next_if(b'd') {
            let depth = self.index().ok_or(ParseError)?.saturating_add(1);
            let index = self.index().ok_or(ParseError)?;
            return self.builtin_generic_param(depth, index);
        }
        if self.next_if(b'z') {
            return self.builtin_generic_param(0, 0);
        }
        if self.next_if(b's') {
            return Err(ParseError);
        }
        let index = self.index().ok_or(ParseError)?.saturating_add(1);
        self.builtin_generic_param(0, index)
    }

    fn builtin_generic_param(&mut self, depth: u32, index: u32) -> PResult {
        let node = self.alloc(Kind::DependentGenericParamType)?;
        self.arena.node_mut(node).value = depth;
        self.arena.node_mut(node).len = u16::try_from(index.min(u32::from(u16::MAX))).unwrap_or(0);
        self.wrap_type(node)
    }

    fn tuple_type(&mut self) -> PResult {
        let root = self.alloc(Kind::Tuple)?;
        if self.pop_kind(Kind::EmptyList).is_none() {
            let mut elements: [NodeId; 64] = [NIL; 64];
            let mut count = 0_usize;
            loop {
                let first = self.pop_kind(Kind::FirstElementMarker).is_some();
                let element = self.alloc(Kind::TupleElement)?;
                let variadic = self.pop_kind(Kind::VariadicMarker);
                self.arena.add_child(element, variadic.unwrap_or(NIL));
                if let Some(name) = self.pop_kind(Kind::Identifier) {
                    let (offset, len) = {
                        let node = self.arena.node(name);
                        (node.value, node.len)
                    };
                    let name_node = self.alloc(Kind::TupleElementName)?;
                    {
                        let node = self.arena.node_mut(name_node);
                        node.value = offset;
                        node.len = len;
                    }
                    self.arena.add_child(element, name_node);
                }
                let ty = self.pop_type().ok_or(ParseError)?;
                self.arena.add_child(element, ty);
                if count >= elements.len() {
                    return Err(ParseError);
                }
                elements[count] = element;
                count += 1;
                if first {
                    break;
                }
            }
            for index in (0..count).rev() {
                self.arena.add_child(root, elements[index]);
            }
        }
        self.wrap_type(root)
    }

    fn protocol_list_type(&mut self) -> PResult {
        let list = self.alloc(Kind::ProtocolList)?;
        if self.pop_kind(Kind::EmptyList).is_none() {
            let mut elements = [NIL; 16];
            let mut count = 0_usize;
            loop {
                let first = self.pop_kind(Kind::FirstElementMarker).is_some();
                let protocol = self
                    .pop_any_generic()
                    .or_else(|| self.pop_kind(Kind::Protocol))
                    .ok_or(ParseError)?;
                if count >= elements.len() {
                    return Err(ParseError);
                }
                elements[count] = protocol;
                count += 1;
                if first {
                    break;
                }
            }
            for index in (0..count).rev() {
                self.arena.add_child(list, elements[index]);
            }
        }
        self.typed(Kind::ProtocolListType, &[list])
    }

    fn metatype_type(&mut self) -> PResult {
        let ty = self.pop_type().ok_or(ParseError)?;
        let node = self.typed(Kind::Metatype, &[ty])?;
        Ok(node)
    }

    fn type_wrapper(&mut self, kind: Kind) -> PResult {
        let ty = self.pop_type_child().ok_or(ParseError)?;
        let node = self.typed(kind, &[ty])?;
        Ok(node)
    }

    fn static_entity(&mut self) -> PResult {
        let entity = self.pop_any().ok_or(ParseError)?;
        let node = self.with_children(Kind::Static, &[entity])?;
        Ok(node)
    }

    fn local_identifier(&mut self) -> PResult {
        if self.next_if(b'L') {
            let discriminator = self.pop_kind(Kind::Identifier).ok_or(ParseError)?;
            let name = self.pop_decl_name().ok_or(ParseError)?;
            return self.with_children(Kind::PrivateDeclName, &[discriminator, name]);
        }
        if self.next_if(b'l') {
            let discriminator = self.pop_kind(Kind::Identifier).ok_or(ParseError)?;
            return self.with_children(Kind::PrivateDeclName, &[discriminator]);
        }
        if let Some(byte) = self.peek()
            && byte.is_ascii_alphabetic()
        {
            let related = [byte];
            self.pos += 1;
            let text = core::str::from_utf8(&related).map_err(|_| ParseError)?;
            let kind = self.alloc_text(Kind::Identifier, text)?;
            let name = self.pop_any().ok_or(ParseError)?;
            return self.with_children(Kind::RelatedEntityDeclName, &[kind, name]);
        }
        let discriminator = self.index().ok_or(ParseError)?;
        let number = self
            .arena
            .alloc_value(Kind::Number, discriminator)
            .ok_or(ParseError)?;
        let name = self.pop_decl_name().ok_or(ParseError)?;
        self.with_children(Kind::LocalDeclName, &[number, name])
    }

    fn variable(&mut self) -> PResult {
        let ty = self.pop_type().ok_or(ParseError)?;
        let _labels = self.pop_kind(Kind::EmptyList);
        let name = self.pop_kind(Kind::Identifier).ok_or(ParseError)?;
        let accessor = self.next_accessor();
        let context = self.pop_context_any().ok_or(ParseError)?;
        let node = self.with_children(Kind::Variable, &[context, name, ty])?;
        if let Some(code) = accessor {
            self.arena.node_mut(node).flags = accessor_code(code);
        }
        Ok(node)
    }

    fn next_accessor(&mut self) -> Option<u8> {
        let byte = self.next_byte()?;
        match byte {
            b'g' | b'G' | b's' | b'm' | b'r' | b'M' | b'w' | b'W' | b'x' | b'y' | b'b' | b'z'
            | b'p' => Some(byte),
            b'a' | b'l' => {
                let _ = self.next_byte();
                Some(byte)
            }
            _ => {
                self.back();
                None
            }
        }
    }

    fn subscript(&mut self) -> PResult {
        let ty = self.pop_type().ok_or(ParseError)?;
        let _labels = self.pop_kind(Kind::EmptyList);
        let accessor = self.next_accessor();
        let context = self.pop_context_any().ok_or(ParseError)?;
        let node = self.with_children(Kind::Subscript, &[context, ty])?;
        if let Some(code) = accessor {
            self.arena.node_mut(node).flags = accessor_code(code);
        }
        Ok(node)
    }

    fn function_entity(&mut self) -> PResult {
        let code = self.next_byte().ok_or(ParseError)?;
        match code {
            b'C' | b'c' => {
                let ty = self.pop_type().ok_or(ParseError)?;
                let _labels = self.pop_kind(Kind::EmptyList);
                let name = self.pop_decl_name().ok_or(ParseError)?;
                let context = self.pop_context_any().ok_or(ParseError)?;
                self.with_children(Kind::Initializer, &[context, name, ty])
            }
            b'D' | b'd' => {
                let name = self.pop_decl_name().ok_or(ParseError)?;
                let context = self.pop_context_any().ok_or(ParseError)?;
                self.with_children(Kind::Deinitializer, &[context, name])
            }
            b'U' | b'u' => {
                let index = self.index().ok_or(ParseError)?;
                let ty = self.pop_type().ok_or(ParseError)?;
                let context = self.pop_context_any().ok_or(ParseError)?;
                let node = self.with_children(
                    if code == b'U' {
                        Kind::ExplicitClosure
                    } else {
                        Kind::ImplicitClosure
                    },
                    &[context, ty],
                )?;
                self.arena.node_mut(node).value = index;
                Ok(node)
            }
            b'A' => {
                let index = self.index().ok_or(ParseError)?;
                let context = self.pop_context_any().ok_or(ParseError)?;
                let node = self.with_children(Kind::DefaultArgumentInitializer, &[context])?;
                self.arena.node_mut(node).value = index;
                Ok(node)
            }
            b'i' => {
                let context = self.pop_context_any().ok_or(ParseError)?;
                self.with_children(Kind::Variable, &[context])
            }
            _ => Err(ParseError),
        }
    }

    fn type_metadata(&mut self) -> PResult {
        let inner = self.pop_any().ok_or(ParseError)?;
        self.global(Global::TypeMetadata, inner)
    }

    fn global(&mut self, code: Global, inner: NodeId) -> PResult {
        let node = self.alloc(Kind::Global)?;
        self.arena.node_mut(node).flags = code as u8;
        self.arena.add_child(node, inner);
        Ok(node)
    }

    fn global_index(&mut self, code: Global, inner: NodeId, index: u32) -> PResult {
        let node = self.global(code, inner)?;
        self.arena.node_mut(node).value = index;
        Ok(node)
    }

    fn impl_function_type(&mut self) -> PResult {
        if self.next_if(b's') || self.next_if(b'I') {
            // Pattern/invocation substitutions are not supported yet.
            return Err(ParseError);
        }
        let signature = self.pop_kind(Kind::DependentGenericSignature);
        let node = self.alloc(Kind::ImplFunctionType)?;
        if let Some(signature) = signature {
            let _pseudo = self.next_if(b'P');
            self.arena.add_child(node, signature);
        }

        if self.next_if(b'e') {
            let child = self.alloc(Kind::ImplEscaping)?;
            self.arena.add_child(node, child);
        }
        if self.next_if(b'A') {
            let child = self.alloc(Kind::ImplErasedIsolation)?;
            self.arena.add_child(node, child);
        }
        if self.next_if(b'N') {
            let child = self.alloc(Kind::ImplNonisolatedNonsending)?;
            self.arena.add_child(node, child);
        }
        if self.next_if(b'O') {
            let callable = self.next_if(b'o');
            let child = self.alloc(Kind::ImplCalledOnce)?;
            self.arena.node_mut(child).flags = u8::from(callable);
            self.arena.add_child(node, child);
        }
        if matches!(self.peek(), Some(b'd' | b'l' | b'f' | b'r')) {
            let code = self.next_byte().ok_or(ParseError)?;
            let child = self.alloc(Kind::ImplDifferentiabilityKind)?;
            self.arena.node_mut(child).flags = code;
            self.arena.add_child(node, child);
        }
        let callee = self.next_byte().ok_or(ParseError)?;
        if !matches!(callee, b'y' | b'g' | b'x' | b't') {
            return Err(ParseError);
        }
        let child = self.alloc(Kind::ImplCalleeConvention)?;
        self.arena.node_mut(child).flags = callee;
        self.arena.add_child(node, child);

        // Optional function representation.
        let representation = self.peek().ok_or(ParseError)?;
        let mut with_clang_type = false;
        let representation = match representation {
            b'B' | b'C' | b'M' | b'O' | b'J' | b'K' | b'V' | b'W' => {
                self.pos += 1;
                Some(representation)
            }
            b'z' => {
                self.pos += 1;
                let inner = self.next_byte().ok_or(ParseError)?;
                match inner {
                    b'B' | b'C' => {
                        with_clang_type = true;
                        Some(inner)
                    }
                    _ => {
                        self.pos -= 2;
                        None
                    }
                }
            }
            _ => None,
        };
        if let Some(code) = representation {
            if with_clang_type {
                return Err(ParseError);
            }
            let child = self.alloc(Kind::ImplFunctionConvention)?;
            self.arena.node_mut(child).flags = code;
            self.arena.add_child(node, child);
        }

        if matches!(self.peek(), Some(b'A' | b'I' | b'G')) {
            let code = self.next_byte().ok_or(ParseError)?;
            let child = self.alloc(Kind::ImplCoroutineKind)?;
            self.arena.node_mut(child).flags = code;
            self.arena.add_child(node, child);
        }
        if self.next_if(b'h') {
            let child = self.alloc(Kind::ImplSendable)?;
            self.arena.add_child(node, child);
        }
        if self.next_if(b'H') {
            let child = self.alloc(Kind::ImplAsync)?;
            self.arena.add_child(node, child);
        }
        if self.next_if(b'T') {
            let child = self.alloc(Kind::ImplSendingResult)?;
            self.arena.add_child(node, child);
        }

        let mut conventions = [NIL; 32];
        let mut count = 0_usize;
        while let Some(code) = self.peek_param_convention() {
            if count >= conventions.len() {
                return Err(ParseError);
            }
            let param = self.impl_entry(Kind::ImplParameter, code)?;
            conventions[count] = param;
            count += 1;
            self.arena.add_child(node, param);
        }
        while let Some(code) = self.peek_result_convention() {
            if count >= conventions.len() {
                return Err(ParseError);
            }
            let result = self.impl_entry(Kind::ImplResult, code)?;
            conventions[count] = result;
            count += 1;
            self.arena.add_child(node, result);
        }
        while self.next_if(b'Y') {
            if count >= conventions.len() {
                return Err(ParseError);
            }
            let code = self.next_byte().ok_or(ParseError)?;
            let param = self.impl_entry(Kind::ImplYield, code)?;
            conventions[count] = param;
            count += 1;
            self.arena.add_child(node, param);
        }
        if self.next_if(b'z') {
            if count >= conventions.len() {
                return Err(ParseError);
            }
            let code = self.next_byte().ok_or(ParseError)?;
            let result = self.impl_entry(Kind::ImplErrorResult, code)?;
            conventions[count] = result;
            count += 1;
            self.arena.add_child(node, result);
        }
        if !self.next_if(b'_') {
            return Err(ParseError);
        }
        // Attach the mangled types in reverse order.
        for index in (0..count).rev() {
            let ty = self.pop_type().ok_or(ParseError)?;
            self.arena.add_child(conventions[index], ty);
        }
        self.wrap_type(node)
    }

    fn peek_param_convention(&self) -> Option<u8> {
        let byte = self.text.get(self.pos).copied()?;
        matches!(
            byte,
            b'i' | b'c'
                | b'l'
                | b'b'
                | b'n'
                | b'X'
                | b'x'
                | b'g'
                | b'e'
                | b'y'
                | b'v'
                | b'p'
                | b'm'
        )
        .then_some(byte)
    }

    fn peek_result_convention(&self) -> Option<u8> {
        let byte = self.text.get(self.pos).copied()?;
        matches!(
            byte,
            b'r' | b'o' | b'd' | b'u' | b'a' | b'k' | b'l' | b'g' | b'm'
        )
        .then_some(byte)
    }

    fn impl_entry(&mut self, kind: Kind, convention: u8) -> PResult {
        self.pos += 1;
        let entry = self.alloc(kind)?;
        let conv = self.alloc(Kind::ImplConvention)?;
        self.arena.node_mut(conv).flags = convention;
        self.arena.add_child(entry, conv);
        // Optional differentiability, sending, isolation and implicit-leading
        // markers, in that order.
        let mut markers = [NIL; 4];
        let mut marker_count = 0_usize;
        for marker in 0..4 {
            let code = match marker {
                0 => b'w',
                1 => b'T',
                2 => b'I',
                _ => b'L',
            };
            if self.next_if(code) {
                let node = self.alloc(match marker {
                    0 => Kind::ImplDifferentiability,
                    1 => Kind::ImplSending,
                    2 => Kind::ImplIsolated,
                    _ => Kind::ImplImplicitLeading,
                })?;
                markers[marker_count] = node;
                marker_count += 1;
                self.arena.add_child(entry, node);
            }
        }
        Ok(entry)
    }

    fn runtime_record(&mut self) -> PResult {
        let code = self.next_byte().ok_or(ParseError)?;
        match code {
            b'n' => {
                let inner = self.pop_type().ok_or(ParseError)?;
                self.global(Global::NominalTypeDescriptorRecord, inner)
            }
            b'c' => {
                let inner = self.pop_protocol_conformance().ok_or(ParseError)?;
                self.global(Global::ProtocolConformanceDescriptorRecord, inner)
            }
            b'r' => {
                let inner = self.pop_protocol().ok_or(ParseError)?;
                self.global(Global::ProtocolDescriptorRecord, inner)
            }
            b'o' => {
                let inner = self.pop_any().ok_or(ParseError)?;
                self.global(Global::OpaqueTypeDescriptorRecord, inner)
            }
            b'F' => self.global(Global::AccessibleFunctionRecord, NIL),
            _ => Err(ParseError),
        }
    }

    fn metatype_global(&mut self) -> PResult {
        let code = self.next_byte().ok_or(ParseError)?;
        match code {
            b'f' => {
                let inner = self.pop_any().ok_or(ParseError)?;
                self.global(Global::FullTypeMetadata, inner)
            }
            b'P' => {
                let inner = self.pop_any().ok_or(ParseError)?;
                self.global(Global::TypeMetadataPattern, inner)
            }
            b'a' => {
                let inner = self.pop_any().ok_or(ParseError)?;
                self.global(Global::TypeMetadataAccess, inner)
            }
            b'A' => {
                let inner = self.pop_protocol_conformance().ok_or(ParseError)?;
                self.global(Global::ReflectionMetadataAssocTypeDescriptor, inner)
            }
            b'c' => {
                let inner = self.pop_protocol_conformance().ok_or(ParseError)?;
                self.global(Global::ProtocolConformanceDescriptor, inner)
            }
            b'S' => {
                let inner = self.pop_protocol().ok_or(ParseError)?;
                self.global(Global::ProtocolSelfConformanceDescriptor, inner)
            }
            b'n' => {
                let inner = self.pop_any().ok_or(ParseError)?;
                self.global(Global::NominalTypeDescriptor, inner)
            }
            b'p' => {
                let inner = self.pop_protocol().ok_or(ParseError)?;
                self.global(Global::ProtocolDescriptor, inner)
            }
            b'I' => {
                let inner = self.pop_any().ok_or(ParseError)?;
                self.global(Global::TypeMetadataInstantiationCache, inner)
            }
            b'i' => {
                let inner = self.pop_any().ok_or(ParseError)?;
                self.global(Global::TypeMetadataInstantiationFunction, inner)
            }
            b'l' => {
                let inner = self.pop_any().ok_or(ParseError)?;
                self.global(Global::TypeMetadataSingletonInitCache, inner)
            }
            b'L' => {
                let inner = self.pop_any().ok_or(ParseError)?;
                self.global(Global::TypeMetadataLazyCache, inner)
            }
            b'D' | b'd' => {
                let inner = self.pop_any().ok_or(ParseError)?;
                self.global(Global::TypeMetadataDemanglingCache, inner)
            }
            b'R' => {
                let inner = self.pop_any().ok_or(ParseError)?;
                self.global(Global::TypeMetadataMangledNameRef, inner)
            }
            b'm' => {
                let inner = self.pop_any().ok_or(ParseError)?;
                self.global(Global::ClassMetaclass, inner)
            }
            b'u' => {
                let inner = self.pop_any().ok_or(ParseError)?;
                self.global(Global::ClassMethodLookupFunction, inner)
            }
            b'U' => {
                let inner = self.pop_any().ok_or(ParseError)?;
                self.global(Global::ObjcMetadataUpdate, inner)
            }
            b's' => {
                let inner = self.pop_any().ok_or(ParseError)?;
                self.global(Global::ObjcResilientClassStub, inner)
            }
            b't' => {
                let inner = self.pop_any().ok_or(ParseError)?;
                self.global(Global::FullObjcResilientClassStub, inner)
            }
            b'o' => {
                let inner = self.pop_any().ok_or(ParseError)?;
                self.global(Global::ClassMetadataBaseOffset, inner)
            }
            b'r' => {
                let inner = self.pop_any().ok_or(ParseError)?;
                self.global(Global::GenericTypeCompletionFunction, inner)
            }
            b'X' => {
                let sub = self.next_byte().ok_or(ParseError)?;
                let inner = self.pop_any().ok_or(ParseError)?;
                let global = match sub {
                    b'M' => Global::ModuleDescriptor,
                    b'E' => Global::ExtensionDescriptor,
                    b'X' => Global::AnonymousContextDescriptor,
                    _ => return Err(ParseError),
                };
                self.global(global, inner)
            }
            _ => Err(ParseError),
        }
    }

    fn witness(&mut self) -> PResult {
        let code = self.next_byte().ok_or(ParseError)?;
        match code {
            b'P' => {
                let inner = self.pop_protocol_conformance().ok_or(ParseError)?;
                self.global(Global::ProtocolWitnessTable, inner)
            }
            b'a' => {
                let inner = self.pop_any().ok_or(ParseError)?;
                self.global(Global::ProtocolWitnessTableAccessor, inner)
            }
            b'p' => {
                let inner = self.pop_any().ok_or(ParseError)?;
                self.global(Global::ProtocolWitnessTablePattern, inner)
            }
            b'I' => {
                let inner = self.pop_any().ok_or(ParseError)?;
                self.global(Global::GenericWitnessTableInstantiationFunction, inner)
            }
            b'G' => {
                let inner = self.pop_any().ok_or(ParseError)?;
                self.global(Global::GenericProtocolWitnessTable, inner)
            }
            b'r' => {
                let inner = self.pop_any().ok_or(ParseError)?;
                self.global(Global::ResilientProtocolWitnessTable, inner)
            }
            b'L' => {
                let inner = self.pop_any().ok_or(ParseError)?;
                self.global(Global::LazyProtocolWitnessTableCache, inner)
            }
            b'l' => {
                let inner = self.pop_any().ok_or(ParseError)?;
                self.global(Global::LazyProtocolWitnessTableAccessor, inner)
            }
            b'T' => {
                let inner = self.pop_any().ok_or(ParseError)?;
                self.global(Global::AssociatedTypeWitnessTableAccessor, inner)
            }
            b'b' => {
                let inner = self.pop_any().ok_or(ParseError)?;
                self.global(Global::BaseProtocolWitnessTableAccessor, inner)
            }
            b'S' => {
                let inner = self.pop_any().ok_or(ParseError)?;
                self.global(Global::ProtocolSelfConformanceWitnessTable, inner)
            }
            b'V' => {
                let inner = self.pop_any().ok_or(ParseError)?;
                self.global(Global::ValueWitnessTable, inner)
            }
            b'v' => {
                let sub = self.next_byte().ok_or(ParseError)?;
                let inner = self.pop_any().ok_or(ParseError)?;
                let global = if sub == b'd' {
                    Global::FieldOffset
                } else {
                    Global::ValueWitnessTable
                };
                self.global(global, inner)
            }
            b'C' => {
                let inner = self.pop_any().ok_or(ParseError)?;
                self.global(Global::EnumTagIndex, inner)
            }
            b'O' => {
                let inner = self.pop_any().ok_or(ParseError)?;
                let op = self.next_byte().ok_or(ParseError)?;
                let global = match op {
                    b'y' => Global::OutlinedCopy,
                    b'e' => Global::OutlinedConsume,
                    b'r' => Global::OutlinedRetain,
                    b's' => Global::OutlinedRelease,
                    b'B' | b'b' => Global::OutlinedInitWithTake,
                    b'C' | b'c' => Global::OutlinedInitWithCopy,
                    b'D' | b'd' => Global::OutlinedAssignWithTake,
                    b'F' | b'f' => Global::OutlinedAssignWithCopy,
                    b'H' | b'h' => Global::OutlinedDestroy,
                    b'i' => Global::OutlinedEnumTagStore,
                    b'j' => Global::OutlinedEnumProjectData,
                    b'g' => Global::OutlinedEnumGetTag,
                    _ => return Err(ParseError),
                };
                self.global(global, inner)
            }
            b'Z' => {
                let inner = self.pop_any().ok_or(ParseError)?;
                self.global(Global::GlobalVariableOnceFunction, inner)
            }
            b'z' => {
                let inner = self.pop_any().ok_or(ParseError)?;
                self.global(Global::GlobalVariableOnceToken, inner)
            }
            b'J' => Err(ParseError),
            _ => Err(ParseError),
        }
    }

    fn thunk_or_specialization(&mut self) -> PResult {
        let code = self.next_byte().ok_or(ParseError)?;
        match code {
            b'o' => self.attribute(0),
            b'O' => self.attribute(1),
            b'D' => self.attribute(2),
            b'd' => self.attribute(3),
            b'g' | b'G' | b'B' | b's' | b'i' => self.generic_specialization(code),
            b'f' => self.function_specialization(),
            b't' => self.generic_specialization_with_dropped_args(),
            b'V' => {
                let base = self.pop_any().ok_or(ParseError)?;
                let derived = self.pop_any().ok_or(ParseError)?;
                self.with_children(Kind::VTableThunk, &[derived, base])
            }
            b'C' => {
                let ty = self.pop_type().ok_or(ParseError)?;
                self.with_children(Kind::CoroutineContinuationPrototype, &[ty])
            }
            b'A' => self.global(Global::PartialApplyForwarder, NIL),
            b'a' => self.global(Global::PartialApplyObjcForwarder, NIL),
            b'R' | b'r' => {
                let code = if code == b'R' {
                    Global::ReabstractionThunkHelper
                } else {
                    Global::ReabstractionThunk
                };
                let signature = self.pop_kind(Kind::DependentGenericSignature);
                let first = self.pop_type().ok_or(ParseError)?;
                let second = self.pop_type().ok_or(ParseError)?;
                let node = self.global(code, NIL)?;
                if let Some(signature) = signature {
                    self.arena.add_child(node, signature);
                }
                self.arena.add_child(node, first);
                self.arena.add_child(node, second);
                Ok(node)
            }
            b'j' => {
                let inner = self.pop_any().ok_or(ParseError)?;
                self.global(Global::DispatchThunk, inner)
            }
            b'q' => {
                let inner = self.pop_any().ok_or(ParseError)?;
                self.global(Global::MethodDescriptor, inner)
            }
            b'u' => self.global(Global::AsyncFunctionPointer, NIL),
            b'X' => self.global(Global::DynamicallyReplaceableFunctionVar, NIL),
            b'x' => self.global(Global::DynamicallyReplaceableFunctionKey, NIL),
            b'I' => self.global(Global::DynamicallyReplaceableFunctionImpl, NIL),
            b'm' => self.global(Global::MergedFunction, NIL),
            b'E' => self.global(Global::DistributedThunk, NIL),
            b'F' => self.global(Global::DistributedAccessor, NIL),
            b'W' => {
                let entity = self.pop_any().ok_or(ParseError)?;
                let conformance = self.pop_protocol_conformance().ok_or(ParseError)?;
                let node = self.global(Global::ProtocolWitnessThunk, conformance)?;
                self.arena.add_child(node, entity);
                Ok(node)
            }
            b'S' => {
                let inner = self.pop_any().ok_or(ParseError)?;
                self.global(Global::ProtocolSelfConformanceWitnessThunk, inner)
            }
            b'w' => {
                let sub = self.next_byte().ok_or(ParseError)?;
                let inner = self.pop_any().ok_or(ParseError)?;
                let global = match sub {
                    b'a' => Global::BackDeploymentCopy,
                    b'b' => Global::BackDeploymentThunk,
                    b'B' => Global::BackDeploymentFallback,
                    b'S' => Global::HasSymbolQuery,
                    b'c' => Global::CoroFunctionPointer,
                    b'd' => Global::DefaultOverride,
                    _ => return Err(ParseError),
                };
                self.global(global, inner)
            }
            b'Y' | b'Q' => {
                let index = self.index().ok_or(ParseError)?;
                let node = self.global_index(Global::ResumePartialFunction, NIL, index)?;
                self.arena.node_mut(node).len = u16::from(code);
                Ok(node)
            }
            b'v' => {
                let index = self.index().ok_or(ParseError)?;
                let read_only = self.next_if(b'r');
                let global = if read_only {
                    Global::OutlinedReadOnlyObject
                } else {
                    Global::OutlinedVariable
                };
                self.global_index(global, NIL, index)
            }
            b'e' => {
                let mut params = [0_u8; 4];
                let mut len = 0_usize;
                if !self.next_if(b'_') {
                    let kind = self.next_byte().ok_or(ParseError)?;
                    if !matches!(kind, b'o' | b'p' | b'a' | b'm') {
                        return Err(ParseError);
                    }
                    params[0] = kind;
                    len = 1;
                    loop {
                        if self.next_if(b'_') {
                            break;
                        }
                        let byte = self.next_byte().ok_or(ParseError)?;
                        if !matches!(byte, b'n' | b'b' | b'g') || len >= params.len() {
                            return Err(ParseError);
                        }
                        params[len] = byte;
                        len += 1;
                    }
                }
                let text = core::str::from_utf8(&params[..len]).map_err(|_| ParseError)?;
                let node = self.alloc_text(Kind::Global, text)?;
                self.arena.node_mut(node).flags = Global::OutlinedBridgedMethod as u8;
                Ok(node)
            }
            b'l' => {
                let assoc = self.pop_assoc_type_name().ok_or(ParseError)?;
                self.global(Global::AssociatedTypeDescriptor, assoc)
            }
            b'L' => {
                let protocol = self.pop_protocol().ok_or(ParseError)?;
                self.global(Global::ProtocolRequirementsBaseDescriptor, protocol)
            }
            b'M' => {
                let assoc = self.pop_assoc_type_name().ok_or(ParseError)?;
                self.global(Global::DefaultAssociatedTypeMetadataAccessor, assoc)
            }
            b'n' | b'N' => {
                let requirement = self.pop_protocol().ok_or(ParseError)?;
                let subject = self.pop_assoc_conformance_subject().ok_or(ParseError)?;
                let proto_ty = self.pop_type().ok_or(ParseError)?;
                let code = if code == b'n' {
                    Global::AssociatedConformanceDescriptor
                } else {
                    Global::DefaultAssociatedConformanceAccessor
                };
                let node = self.global(code, proto_ty)?;
                self.arena.add_child(node, subject);
                self.arena.add_child(node, requirement);
                Ok(node)
            }
            b'b' => {
                let requirement = self.pop_protocol().ok_or(ParseError)?;
                let proto_ty = self.pop_type().ok_or(ParseError)?;
                let node = self.global(Global::BaseConformanceDescriptor, proto_ty)?;
                self.arena.add_child(node, requirement);
                Ok(node)
            }
            b'J' | b'K' | b'k' => Err(ParseError),
            _ => Err(ParseError),
        }
    }

    fn function_specialization(&mut self) -> PResult {
        let _serialized = self.next_if(b'q');
        let _async_removed = self.next_if(b'a');
        let representation_changed = self.next_if(b'r');
        let pass = self.next_byte().ok_or(ParseError)?;
        if !pass.is_ascii_digit() {
            return Err(ParseError);
        }
        let spec = self.alloc(Kind::FunctionSignatureSpecialization)?;
        if representation_changed {
            self.arena.node_mut(spec).flags = 1;
            return Ok(spec);
        }
        let mut params = [NIL; 16];
        let mut count = 0_usize;
        while self.peek().is_some_and(|b| b != b'_') {
            if count >= params.len() {
                return Err(ParseError);
            }
            params[count] = self.func_spec_param(false)?;
            count += 1;
        }
        if !self.next_if(b'_') {
            return Err(ParseError);
        }
        if !self.next_if(b'n') {
            if count >= params.len() {
                return Err(ParseError);
            }
            params[count] = self.func_spec_param(true)?;
            count += 1;
        }
        // Attach payloads from the operand stack in reverse parameter order,
        // then store the parameters in source order.
        for index in (0..count).rev() {
            self.func_spec_attach(params[index])?;
        }
        for &param in params.iter().take(count) {
            self.arena.add_child(spec, param);
        }
        Ok(spec)
    }

    fn func_spec_param(&mut self, is_return: bool) -> PResult {
        let param = self.alloc(Kind::FuncSpecParam)?;
        if is_return {
            self.arena.node_mut(param).flags = 1;
        }
        let code = self.next_byte().ok_or(ParseError)?;
        match code {
            b'n' => {}
            b'c' | b'E' => self.func_spec_kind(param, if code == b'c' { 5 } else { 12 })?,
            b'C' => {
                let index = self.natural().ok_or(ParseError)?;
                self.func_spec_kind(param, 11)?;
                let payload = self
                    .arena
                    .alloc_value(Kind::FuncSpecNumber, index)
                    .ok_or(ParseError)?;
                self.arena.add_child(param, payload);
            }
            b'p' => self.func_spec_propagation(param)?,
            b'e' | b'd' | b'g' | b'o' => {
                let mut value = 0_u32;
                if code == b'e' {
                    value |= 1 << 10;
                }
                if matches!(code, b'd' | b'e') {
                    value |= 1 << 6;
                }
                if matches!(code, b'g' | b'd' | b'e') {
                    value |= 1 << 7;
                }
                if matches!(code, b'o' | b'd' | b'e') {
                    value |= 1 << 9;
                }
                if self.next_if(b'X') {
                    value |= 1 << 8;
                }
                self.func_spec_kind(param, value)?;
            }
            b'x' => self.func_spec_kind(param, 1 << 8)?,
            b'i' => self.func_spec_kind(param, 6)?,
            b's' => self.func_spec_kind(param, 7)?,
            b'r' => self.func_spec_kind(param, 8)?,
            _ => return Err(ParseError),
        }
        Ok(param)
    }

    fn func_spec_kind(&mut self, param: NodeId, code: u32) -> PResult<()> {
        let node = self
            .arena
            .alloc_value(Kind::FuncSpecKind, code)
            .ok_or(ParseError)?;
        self.arena.add_child(param, node);
        Ok(())
    }

    fn func_spec_propagation(&mut self, param: NodeId) -> PResult<()> {
        match self.next_byte().ok_or(ParseError)? {
            b'S' => self.func_spec_kind(param, 10),
            b'f' => self.func_spec_kind(param, 0),
            b'g' => self.func_spec_kind(param, 1),
            b'i' | b'd' => {
                let kind = if self.text[self.pos - 1] == b'i' {
                    2
                } else {
                    3
                };
                self.func_spec_kind(param, kind)?;
                let start = self.pos;
                while self.peek().is_some_and(|b| b.is_ascii_digit()) {
                    self.pos += 1;
                }
                if self.pos == start {
                    return Err(ParseError);
                }
                let text =
                    core::str::from_utf8(&self.text[start..self.pos]).map_err(|_| ParseError)?;
                let node = self.alloc_text(Kind::Identifier, text)?;
                self.arena.add_child(param, node);
                Ok(())
            }
            b's' => {
                let encoding = self.next_byte().ok_or(ParseError)?;
                self.func_spec_kind(param, 4)?;
                let text = match encoding {
                    b'b' => "u8",
                    b'w' => "u16",
                    b'c' => "objc",
                    _ => return Err(ParseError),
                };
                let payload = self.alloc_text(Kind::Identifier, text)?;
                self.arena.add_child(param, payload);
                Ok(())
            }
            b'k' => self.func_spec_kind(param, 9),
            _ => Err(ParseError),
        }
    }

    /// Pop the payloads a parameter needs from the operand stack and append
    /// them after its kind tag, in reference print order.
    fn func_spec_attach(&mut self, param: NodeId) -> PResult<()> {
        let kind = self.arena.child(param, 0);
        if kind == NIL {
            return Ok(());
        }
        let raw = self.arena.node(kind).value;
        match raw {
            5 | 12 => {
                let mut types = [NIL; 8];
                let mut type_count = 0_usize;
                while let Some(ty) = self.pop_type() {
                    if type_count < types.len() {
                        types[type_count] = ty;
                        type_count += 1;
                    }
                }
                let ident = self.pop_kind(Kind::Identifier).ok_or(ParseError)?;
                self.arena.add_child(param, ident);
                for index in (0..type_count).rev() {
                    self.arena.add_child(param, types[index]);
                }
            }
            0 | 1 | 10 => {
                let ident = self.pop_kind(Kind::Identifier).ok_or(ParseError)?;
                self.arena.add_child(param, ident);
            }
            4 => {
                let ident = self.pop_kind(Kind::Identifier).ok_or(ParseError)?;
                self.arena.add_child(param, ident);
            }
            9 => {
                let ty = self.pop_type().ok_or(ParseError)?;
                self.arena.add_child(param, ty);
                let ty = self.pop_type().ok_or(ParseError)?;
                self.arena.add_child(param, ty);
                let ident = self.pop_kind(Kind::Identifier).ok_or(ParseError)?;
                self.arena.add_child(param, ident);
            }
            _ => {}
        }
        Ok(())
    }

    fn attribute(&mut self, code: u8) -> PResult {
        let node = self.alloc(Kind::Attribute)?;
        self.arena.node_mut(node).flags = code;
        Ok(node)
    }

    fn generic_specialization(&mut self, code: u8) -> PResult {
        let node = self.alloc(Kind::GenericSpecialization)?;
        self.arena.node_mut(node).flags = code;
        self.generic_specialization_body(node)
    }

    fn generic_specialization_with_dropped_args(&mut self) -> PResult {
        let node = self.alloc(Kind::GenericSpecialization)?;
        while self.next_if(b't') {
            let value = self.natural().map_or(0, |v| v.saturating_add(1));
            let dropped = self
                .arena
                .alloc_value(Kind::DroppedArgument, value)
                .ok_or(ParseError)?;
            self.arena.add_child(node, dropped);
        }
        let code = self.next_byte().ok_or(ParseError)?;
        if !matches!(code, b'g' | b'G' | b'B') {
            return Err(ParseError);
        }
        self.arena.node_mut(node).flags = code;
        self.generic_specialization_body(node)
    }

    fn generic_specialization_body(&mut self, node: NodeId) -> PResult {
        let _serialized = self.next_if(b'q');
        let _async_removed = self.next_if(b'a');
        let _representation_changed = self.next_if(b'r');
        let pass = self.next_byte().ok_or(ParseError)?;
        if !pass.is_ascii_digit() {
            return Err(ParseError);
        }
        if self.pop_kind(Kind::EmptyList).is_none() {
            let mut types = [NIL; 16];
            let mut count = 0_usize;
            loop {
                let first = self.pop_kind(Kind::FirstElementMarker).is_some();
                let ty = self.pop_type().ok_or(ParseError)?;
                if count >= types.len() {
                    return Err(ParseError);
                }
                types[count] = ty;
                count += 1;
                if first {
                    break;
                }
            }
            for index in (0..count).rev() {
                let ty = types[index];
                let param = self.with_children(Kind::GenericSpecializationParam, &[ty])?;
                self.arena.add_child(node, param);
            }
        }
        Ok(node)
    }

    fn value_witness(&mut self) -> PResult {
        let inner = self.pop_any().ok_or(ParseError)?;
        let code = self.next_byte().ok_or(ParseError)?;
        let node = self.global(Global::ValueWitnessTable, inner)?;
        self.arena.node_mut(node).flags = code;
        Ok(node)
    }

    fn special_type(&mut self) -> PResult {
        let code = self.next_byte().ok_or(ParseError)?;
        match code {
            b'F' => Err(ParseError),
            b'x' => {
                let inner = self.pop_type().ok_or(ParseError)?;
                let node = self.typed(Kind::Type, &[inner])?;
                Ok(node)
            }
            b'o' => self.type_wrapper(Kind::Owned),
            b'u' => self.type_wrapper(Kind::Owned),
            b'w' => self.type_wrapper(Kind::Shared),
            b'b' => Err(ParseError),
            b'M' => {
                let _repr = self.next_byte().ok_or(ParseError)?;
                self.metatype_type()
            }
            b'm' => {
                let _repr = self.next_byte().ok_or(ParseError)?;
                let ty = self.pop_type().ok_or(ParseError)?;
                let node = self.typed(Kind::ExistentialMetatype, &[ty])?;
                Ok(node)
            }
            b'p' => {
                let ty = self.pop_type().ok_or(ParseError)?;
                let node = self.typed(Kind::ExistentialMetatype, &[ty])?;
                Ok(node)
            }
            b'e' => Err(ParseError),
            b'C' => self.function_type(Kind::CFunctionPointer),
            b'E' => self.function_type(Kind::NoEscapeFunctionType),
            b'K' => self.function_type(Kind::AutoClosureType),
            b'A' => self.function_type(Kind::EscapingAutoClosureType),
            b'B' => self.function_type(Kind::ObjCBlock),
            b'L' => self.function_type(Kind::EscapingObjCBlock),
            b'O' => {
                let once = self.next_if(b'o');
                self.function_type(if once {
                    Kind::CalledAtMostOnceFunctionType
                } else {
                    Kind::CalledOnceFunctionType
                })
            }
            b'f' => self.function_type(Kind::ThinFunctionType),
            b'l' => Err(ParseError),
            b'j' => Err(ParseError),
            b'X' => Err(ParseError),
            _ => Err(ParseError),
        }
    }

    fn archetype(&mut self) -> PResult {
        let code = self.next_byte().ok_or(ParseError)?;
        match code {
            b'a' => {
                let name = self.pop_kind(Kind::Identifier).ok_or(ParseError)?;
                let base = self.pop_type_child().ok_or(ParseError)?;
                let node = self.typed(Kind::DependentMemberType, &[base, name])?;
                self.arena.add_sub(node);
                Ok(node)
            }
            b'y' => {
                let param = self.generic_param()?;
                self.assoc_type_simple(param)
            }
            b'z' => {
                let param = self.builtin_generic_param(0, 0)?;
                self.assoc_type_simple(param)
            }
            b'x' => self.assoc_type_simple(NIL),
            b'r' => {
                let node = self.typed(Kind::DependentGenericParamType, &[])?;
                Ok(node)
            }
            _ => Err(ParseError),
        }
    }

    fn assoc_type_simple(&mut self, base: NodeId) -> PResult {
        let name = self.pop_kind(Kind::Identifier).ok_or(ParseError)?;
        let base = if base == NIL {
            self.typed(Kind::DependentGenericParamType, &[])?
        } else {
            base
        };
        let node = self.typed(Kind::DependentMemberType, &[base, name])?;
        self.arena.add_sub(node);
        Ok(node)
    }

    fn type_annotation(&mut self) -> PResult {
        let code = self.next_byte().ok_or(ParseError)?;
        let node = match code {
            b'a' => self.alloc(Kind::AsyncAnnotation)?,
            b'A' => self.alloc(Kind::IsolatedAnyFunctionType)?,
            b'b' => self.alloc(Kind::ConcurrentFunctionType)?,
            b'c' => {
                let ty = self.pop_type().ok_or(ParseError)?;
                self.with_children(Kind::GlobalActorFunctionType, &[ty])?
            }
            b'C' => self.alloc(Kind::NonIsolatedCallerFunctionType)?,
            b'i' => {
                let ty = self.pop_type_child().ok_or(ParseError)?;
                self.typed(Kind::Isolated, &[ty])?
            }
            b'j' => {
                let kind = self.next_byte().ok_or(ParseError)?;
                let node = self.alloc(Kind::DifferentiableFunctionType)?;
                self.arena.node_mut(node).flags = kind;
                node
            }
            b'k' => {
                let ty = self.pop_type_child().ok_or(ParseError)?;
                self.typed(Kind::NoDerivative, &[ty])?
            }
            b'K' => {
                let ty = self.pop_type().ok_or(ParseError)?;
                self.with_children(Kind::TypedThrowsAnnotation, &[ty])?
            }
            b't' => {
                let ty = self.pop_type_child().ok_or(ParseError)?;
                self.typed(Kind::CompileTimeLiteral, &[ty])?
            }
            b'g' => {
                let ty = self.pop_type_child().ok_or(ParseError)?;
                self.typed(Kind::ConstValue, &[ty])?
            }
            b'T' => self.alloc(Kind::SendingResultFunctionType)?,
            b'u' => {
                let ty = self.pop_type_child().ok_or(ParseError)?;
                self.typed(Kind::Sending, &[ty])?
            }
            _ => return Err(ParseError),
        };
        Ok(node)
    }

    fn operator_identifier(&mut self) -> PResult {
        let ident = self.pop_kind(Kind::Identifier).ok_or(ParseError)?;
        const TABLE: &[u8; 26] = b"& @/= >    <*!|+?%-~   ^ .";
        let mut text = [0_u8; 64];
        let source = self.arena.text(ident);
        if source.len() > text.len() {
            return Err(ParseError);
        }
        for (index, byte) in source.bytes().enumerate() {
            if !byte.is_ascii_lowercase() {
                return Err(ParseError);
            }
            let mapped = TABLE[usize::from(byte - b'a')];
            if mapped == b' ' {
                return Err(ParseError);
            }
            text[index] = mapped;
        }
        let text = core::str::from_utf8(&text[..source.len()]).map_err(|_| ParseError)?;
        let kind = match self.next_byte().ok_or(ParseError)? {
            b'i' => Kind::InfixOperator,
            b'p' => Kind::PrefixOperator,
            b'P' => Kind::PostfixOperator,
            _ => return Err(ParseError),
        };
        self.alloc_text(kind, text)
    }

    fn integer_type(&self) -> PResult {
        Err(ParseError)
    }
}

/// Format `prefix` + `value` into a stack buffer.
fn format_builtin<'b>(buf: &'b mut [u8; 40], prefix: &str, value: u32) -> &'b str {
    let bytes = prefix.as_bytes();
    let mut len = 0_usize;
    if bytes.len() + 10 > buf.len() {
        return "";
    }
    buf[..bytes.len()].copy_from_slice(bytes);
    len += bytes.len();
    let mut digits = [0_u8; 10];
    let mut count = 0_usize;
    let mut value = value;
    if value == 0 {
        digits[0] = b'0';
        count = 1;
    }
    while value > 0 {
        digits[count] = b'0' + u8::try_from(value % 10).unwrap_or(0);
        value /= 10;
        count += 1;
    }
    for index in (0..count).rev() {
        buf[len] = digits[index];
        len += 1;
    }
    core::str::from_utf8(&buf[..len]).unwrap_or("")
}

/// Builtin type names (`B` + kind) that carry no extra payload.
const fn builtin_name(code: u8) -> Option<&'static str> {
    match code {
        b'b' => Some("Builtin.BridgeObject"),
        b'B' => Some("Builtin.UnsafeValueBuffer"),
        b'c' => Some("Builtin.RawUnsafeContinuation"),
        b'D' => Some("Builtin.DefaultActorStorage"),
        b'd' => Some("Builtin.NonDefaultDistributedActorStorage"),
        b'I' => Some("Builtin.IntLiteral"),
        b'j' => Some("Builtin.Job"),
        b'P' => Some("Builtin.PackIndex"),
        b'O' => Some("Builtin.UnknownObject"),
        b'o' => Some("Builtin.NativeObject"),
        b'p' => Some("Builtin.RawPointer"),
        b't' => Some("Builtin.SILToken"),
        b'w' => Some("Builtin.Word"),
        b'A' => Some("Builtin.ImplicitActor"),
        _ => None,
    }
}

/// Standard substitution table: `S` + kind → Swift type.
const fn standard_type_name(code: u8, second: bool) -> Option<(Kind, &'static str)> {
    use Kind as K;
    if second {
        return match code {
            b'A' => Some((K::Protocol, "Actor")),
            b'C' => Some((K::Structure, "CheckedContinuation")),
            b'c' => Some((K::Structure, "UnsafeContinuation")),
            b'E' => Some((K::Structure, "CancellationError")),
            b'e' => Some((K::Protocol, "UnownedSerialExecutor")),
            b'F' => Some((K::Protocol, "Executor")),
            b'f' => Some((K::Protocol, "SerialExecutor")),
            b'G' => Some((K::Structure, "TaskGroup")),
            b'g' => Some((K::Structure, "ThrowingTaskGroup")),
            b'I' => Some((K::Protocol, "AsyncIteratorProtocol")),
            b'i' => Some((K::Protocol, "AsyncSequence")),
            b'J' => Some((K::Structure, "UnownedJob")),
            b'M' => Some((K::Structure, "MainActor")),
            b'P' => Some((K::Structure, "TaskPriority")),
            b'S' => Some((K::Structure, "AsyncStream")),
            b's' => Some((K::Structure, "AsyncThrowingStream")),
            b'T' => Some((K::Structure, "Task")),
            b't' => Some((K::Structure, "UnsafeCurrentTask")),
            _ => None,
        };
    }
    match code {
        b'A' => Some((K::Structure, "AutoreleasingUnsafeMutablePointer")),
        b'a' => Some((K::Structure, "Array")),
        b'B' => Some((K::Protocol, "BinaryFloatingPoint")),
        b'b' => Some((K::Structure, "Bool")),
        b'D' => Some((K::Structure, "Dictionary")),
        b'd' => Some((K::Structure, "Double")),
        b'E' => Some((K::Protocol, "Encodable")),
        b'e' => Some((K::Protocol, "Decodable")),
        b'F' => Some((K::Protocol, "FloatingPoint")),
        b'f' => Some((K::Structure, "Float")),
        b'G' => Some((K::Protocol, "RandomNumberGenerator")),
        b'H' => Some((K::Protocol, "Hashable")),
        b'h' => Some((K::Structure, "Set")),
        b'I' => Some((K::Structure, "DefaultIndices")),
        b'i' => Some((K::Structure, "Int")),
        b'J' => Some((K::Structure, "Character")),
        b'j' => Some((K::Protocol, "Numeric")),
        b'K' => Some((K::Protocol, "BidirectionalCollection")),
        b'k' => Some((K::Protocol, "RandomAccessCollection")),
        b'L' => Some((K::Protocol, "Comparable")),
        b'l' => Some((K::Protocol, "Collection")),
        b'M' => Some((K::Protocol, "MutableCollection")),
        b'm' => Some((K::Protocol, "RangeReplaceableCollection")),
        b'N' => Some((K::Structure, "ClosedRange")),
        b'n' => Some((K::Structure, "Range")),
        b'O' => Some((K::Structure, "ObjectIdentifier")),
        b'P' => Some((K::Structure, "UnsafePointer")),
        b'p' => Some((K::Structure, "UnsafeMutablePointer")),
        b'Q' => Some((K::Protocol, "Equatable")),
        b'q' => Some((K::Structure, "Optional")),
        b'R' => Some((K::Structure, "UnsafeBufferPointer")),
        b'r' => Some((K::Structure, "UnsafeMutableBufferPointer")),
        b'S' => Some((K::Structure, "String")),
        b's' => Some((K::Structure, "Substring")),
        b'T' => Some((K::Protocol, "Sequence")),
        b't' => Some((K::Protocol, "IteratorProtocol")),
        b'U' => Some((K::Protocol, "UnsignedInteger")),
        b'u' => Some((K::Structure, "UInt")),
        b'V' => Some((K::Structure, "UnsafeRawPointer")),
        b'v' => Some((K::Structure, "UnsafeMutableRawPointer")),
        b'W' => Some((K::Structure, "UnsafeRawBufferPointer")),
        b'w' => Some((K::Structure, "UnsafeMutableRawBufferPointer")),
        b'X' => Some((K::Structure, "RangeExpression")),
        b'x' => Some((K::Protocol, "Strideable")),
        b'Y' => Some((K::Protocol, "RawRepresentable")),
        b'y' => Some((K::Protocol, "StringProtocol")),
        b'Z' => Some((K::Protocol, "SignedInteger")),
        b'z' => Some((K::Protocol, "BinaryInteger")),
        _ => None,
    }
}

