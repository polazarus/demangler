//! Swift mangling parser.
//!
//! Mirrors the reference demangler's operator machine: a postfix stream of
//! operators, each popping operands from a stack and pushing a node. Nodes own
//! their children as ordered slices. Input is read through [`Cursor`], which
//! owns the work and depth budgets; identifier text borrows the input bytes or
//! is stored as escaped `Cow` text.

use alloc::{borrow::Cow, vec::Vec};

use crate::parse::Parser as _;
use crate::parse::{Cursor, Escape, decimal_length};
use crate::swift::ParseLimits;
use crate::swift::node::{
    Accessor, AttributeKind, AutoDiffCode, Builder, Context, DeinitializerKind,
    GenericSpecializationKind, Global, GlobalOperand, KeyPathKind, Kind, LayoutCode, MacroRole,
    NodeData, NodeRef, ResumeStyle, accessor_code,
};
use crate::swift::text;

/// Parsing failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ParseError;

/// Generic requirement constraints (`R` operator).
#[derive(Clone, Copy)]
enum ReqConstraint {
    Protocol,
    SameType,
    BaseClass,
    Layout,
    SameShape,
    Inverse,
    ValueMarker,
    PackMarker,
}

/// Subject-kind part of a generic requirement code.
#[derive(Clone, Copy)]
enum ReqType {
    Generic,
    Assoc,
    CompoundAssoc,
    Substitution,
}

/// One function-signature specialization parameter under construction.
///
/// The parameter's payloads are popped from the operand stack only after every
/// parameter has been read, so the node is built once, in `func_spec_attach`.
struct ParamBuilder<'a> {
    is_return: bool,
    children: Vec<NodeRef<'a>>,
}

/// Result alias for the parser.
pub type PResult<'a, T = NodeRef<'a>> = Result<T, ParseError>;

/// Swift demangling parser over a mangled name (prefix already stripped).
pub struct Parser<'a, 's> {
    input: Cursor<'a, 's>,
    pub(crate) context: Context<'a>,
    stack: Vec<NodeRef<'a>>,
    /// The budget this parse enforces.
    limits: ParseLimits,
    /// Byte offset where parsing failed (diagnostics only).
    pub(crate) fail_pos: usize,
    /// Operator byte that failed (diagnostics only).
    pub(crate) fail_byte: u8,
    /// `_T` prefixes use the pre-stable function type layout, where parameter
    /// labels are embedded in tuple elements and no label list is popped.
    old_function_type: bool,
    /// Number of `Sg` optional layers built so far.
    optional_layers: u16,
    /// Number of iterative shared/owned/inout type wrappers built so far.
    wrapper_layers: u16,
}

impl<'a, 's> Parser<'a, 's> {
    /// Create a parser for the bytes after the mangling prefix.
    pub(crate) const fn new(
        input: Cursor<'a, 's>,
        old_function_type: bool,
        limits: ParseLimits,
    ) -> Self {
        Self {
            input,
            context: Context::new(limits.nodes()),
            stack: Vec::new(),
            limits,
            fail_pos: 0,
            fail_byte: 0,
            old_function_type,
            optional_layers: 0,
            wrapper_layers: 0,
        }
    }

    /// Parse a complete mangled name and assemble its top-level node.
    /// Nodes charged so far, and whether the node budget ran out.
    pub(crate) const fn node_usage(&self) -> (usize, bool) {
        (self.context.used(), self.context.failed())
    }

    /// Record on the cursor state that a budget stopped this parse.
    pub(crate) const fn exhaust(&mut self) {
        self.input.exhaust();
    }

    pub(crate) fn parse(&mut self) -> PResult<'a> {
        let mut last_pos = usize::MAX;
        let mut stalled = 0_u8;
        while !self.input.remaining().is_empty() {
            let pos = self.input.pos();
            if pos == last_pos {
                stalled = stalled.saturating_add(1);
                if stalled > 4 {
                    return Err(ParseError);
                }
            } else {
                stalled = 0;
                last_pos = pos;
            }
            if self.peek() == Some(b'.') {
                // Trailing text is an unmangled compiler suffix.
                let node = self.alloc_cow(
                    Kind::Suffix,
                    text::cow(self.input.remaining(), Escape::Verbatim),
                )?;
                self.push(node.clone());
                let rest = self.input.remaining().len();
                self.advance(rest)?;
                break;
            }
            let node = self.operator()?;
            self.push(node.clone());
        }
        let root = self.finish()?;
        if self.context.failed() {
            return Err(ParseError);
        }
        Ok(root)
    }

    /// Wrap the nodes left on the stack in a top-level node.
    ///
    /// A `PartialApplyForwarder` (or its ObjC twin) among the attributes
    /// becomes the parent of everything after it, so the attribute list is
    /// rebuilt from the innermost forwarder outwards: the shared forwarder is
    /// copied with its children instead of being mutated.
    fn finish(&mut self) -> PResult<'a> {
        let suffix = self.pop_kind(Kind::Suffix);
        let mut attributes = Vec::new();
        while let Some(id) = self.stack.last().cloned() {
            let kind = id.kind();
            let is_attribute = kind == Kind::Attribute
                || kind == Kind::GenericSpecialization
                || kind == Kind::FunctionSignatureSpecialization;
            if !is_attribute && !Self::is_function_attr(&id) {
                break;
            }
            let _ = self.stack.pop();
            attributes.push(id);
        }
        if self.old_function_type && self.stack.len() > 1 {
            return Err(ParseError);
        }
        // Operands attach to the innermost forwarder, or to the top level.
        let mut children = Vec::new();
        for node in core::mem::take(&mut self.stack) {
            if node.kind() == Kind::Type {
                if let Some(child) = node.child(0) {
                    children.push(child);
                }
            } else {
                children.push(node);
            }
        }
        for attribute in attributes.into_iter().rev() {
            if Self::is_forwarder(&attribute) {
                let kind = attribute.kind();
                let forwarder = self
                    .context
                    .rebuild_with_children(&attribute, kind, children);
                children = alloc::vec![forwarder];
            } else {
                children.insert(0, attribute);
            }
        }
        if let Some(suffix) = suffix {
            children.push(suffix);
        }
        if children.is_empty() {
            return Err(ParseError);
        }
        let mut top = self.builder(Kind::TopLevel)?;
        for child in children {
            top.push(child);
        }
        Ok(top.build())
    }

    /// True for the global wrappers that print as a prefix of their child.
    fn is_forwarder(id: &NodeRef<'a>) -> bool {
        matches!(
            id.global_code(),
            Some(Global::PartialApplyForwarder | Global::PartialApplyObjcForwarder)
        )
    }

    /// True for nodes that print as prefixes of the symbol that follows them.
    fn is_function_attr(id: &NodeRef<'a>) -> bool {
        match id.kind() {
            Kind::Attribute
            | Kind::GenericSpecialization
            | Kind::FunctionSignatureSpecialization => true,
            Kind::Global => matches!(
                id.global_code(),
                Some(
                    Global::PartialApplyForwarder
                        | Global::PartialApplyObjcForwarder
                        | Global::OutlinedVariable
                        | Global::OutlinedReadOnlyObject
                        | Global::OutlinedBridgedMethod
                        | Global::AccessibleFunctionRecord
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
        self.input.peek()
    }

    /// Consume one byte, charging the work budget.
    fn next_byte(&mut self) -> Option<u8> {
        let bytes = self.input.advance(1).ok()?;
        bytes.first().copied()
    }

    fn next_if(&mut self, byte: u8) -> bool {
        if self.peek() != Some(byte) {
            return false;
        }
        self.input.advance(1).is_ok()
    }

    /// Consume `len` bytes.
    fn advance(&mut self, len: usize) -> PResult<'a, ()> {
        self.input.advance(len).map(|_| ()).map_err(|_| ParseError)
    }

    /// Read a decimal number, or fail on an absent or overflowing number.
    fn decimal(&mut self) -> PResult<'a, u32> {
        let value = decimal_length()
            .parse(&mut self.input)
            .map_err(|_| ParseError)?;
        u32::try_from(value).map_err(|_| ParseError)
    }

    /// Read a decimal number; `None` when absent or overflowing `u32`.
    fn natural(&mut self) -> Option<u32> {
        let start = self.input.pos();
        let mut value: u32 = 0;
        while let Some(byte) = self.peek() {
            if !byte.is_ascii_digit() {
                break;
            }
            value = value.checked_mul(10)?.checked_add(u32::from(byte - b'0'))?;
            if self.input.advance(1).is_err() {
                return None;
            }
        }
        if self.input.pos() == start {
            None
        } else {
            Some(value)
        }
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

    /// Push an operand. Every operand is a node the parse charged or a
    /// substitution reference consuming input, so the node and input budgets
    /// bound the stack.
    fn push(&mut self, id: NodeRef<'a>) {
        self.stack.push(id);
    }

    fn pop_any(&mut self) -> Option<NodeRef<'a>> {
        self.stack.pop()
    }

    fn pop_kind(&mut self, kind: Kind) -> Option<NodeRef<'a>> {
        let id = self.stack.last()?.clone();
        if id.kind() == kind {
            let _ = self.stack.pop();
            Some(id)
        } else {
            None
        }
    }

    fn pop_type(&mut self) -> Option<NodeRef<'a>> {
        self.pop_kind(Kind::Type)
    }

    fn pop_type_child(&mut self) -> Option<NodeRef<'a>> {
        let ty = self.pop_type()?;
        ty.child(0)
    }

    fn pop_any_generic(&mut self) -> Option<NodeRef<'a>> {
        let ty = self.pop_type()?;
        let child = ty.child(0)?;
        child.kind().is_any_generic().then_some(child)
    }

    fn pop_module(&mut self) -> Option<NodeRef<'a>> {
        if let Some(id) = self.pop_kind(Kind::Identifier) {
            return Some(self.context.relabel(&id, Kind::Module));
        }
        self.pop_kind(Kind::Module)
    }

    fn peek_context(&self) -> Option<Kind> {
        let id = self.stack.last()?;
        Some(id.kind())
    }

    fn pop_context_any(&mut self) -> Option<NodeRef<'a>> {
        if let Some(module) = self.pop_module() {
            return Some(module);
        }
        if let Some(id) = self.pop_any() {
            let kind = id.kind();
            if kind == Kind::Type {
                if let Some(child) = id.child(0)
                    && child.kind().is_context()
                {
                    return Some(child);
                }
                self.stack.push(id);
                return None;
            }
            if kind.is_context() {
                return Some(id);
            }
            self.stack.push(id);
        }
        None
    }

    /// Pop a `FirstElementMarker`-terminated list through `pop`, in the order
    /// the elements were popped (the reverse of the list's logical order).
    /// Callers attach the result with `into_iter().rev()`.
    fn pop_reverse_list(
        &mut self,
        pop: fn(&mut Self) -> Option<NodeRef<'a>>,
    ) -> Option<Vec<NodeRef<'a>>> {
        let mut elements = Vec::new();
        loop {
            let first = self.pop_kind(Kind::FirstElementMarker).is_some();
            let element = pop(self)?;
            elements.push(element);
            if first {
                break;
            }
        }
        Some(elements)
    }

    /// Pop an associated type name (`<protocol>.<name>` or bare name).
    fn pop_assoc_type_name(&mut self) -> Option<NodeRef<'a>> {
        let protocol = if let Some(ty) = self.pop_type() {
            let child = ty.child(0);
            if child
                .as_ref()
                .is_some_and(|child| child.kind() == Kind::Protocol)
            {
                Some(ty)
            } else {
                self.stack.push(ty);
                None
            }
        } else {
            None
        };
        let name = self.pop_kind(Kind::Identifier)?;
        self.node_with_opt(Kind::DependentAssociatedTypeRef, &[Some(name), protocol])
            .ok()
    }

    fn pop_assoc_type_path(&mut self) -> Option<NodeRef<'a>> {
        let mut path = self.builder(Kind::AssocTypePath).ok()?;
        let elements = self.pop_reverse_list(Self::pop_assoc_type_name)?;
        for element in elements.into_iter().rev() {
            path.push(element);
        }
        Some(path.build())
    }

    fn pop_assoc_conformance_subject(&mut self) -> Option<NodeRef<'a>> {
        if let Some(ty) = self.pop_type() {
            let child = ty.child(0);
            if child.as_ref().is_some_and(|child| {
                matches!(
                    child.kind(),
                    Kind::DependentGenericParamType | Kind::DependentMemberType
                )
            }) {
                return Some(ty);
            }
            self.stack.push(ty);
        }
        self.pop_assoc_type_path()
    }

    fn pop_any_protocol_conformance_list(&mut self) -> Option<NodeRef<'a>> {
        let mut list = self.builder(Kind::AnyProtocolConformanceList).ok()?;
        if self.pop_kind(Kind::EmptyList).is_some() {
            return Some(list.build());
        }
        let elements = self.pop_reverse_list(Self::pop_any)?;
        for element in elements.into_iter().rev() {
            list.push(element);
        }
        Some(list.build())
    }

    fn pop_retroactive_protocol_conformance_ref(&mut self) -> Option<NodeRef<'a>> {
        let module = self.pop_module()?;
        let protocol = self.pop_protocol()?;
        self.node_with(
            Kind::ProtocolConformanceRefInOtherModule,
            &[protocol, module],
        )
        .ok()
    }

    fn pop_concrete_protocol_conformance(&mut self) -> Option<NodeRef<'a>> {
        let conditionals = self.pop_any_protocol_conformance_list()?;
        let reference = if let Some(id) = self.pop_any() {
            if matches!(
                id.kind(),
                Kind::ProtocolConformanceRefInTypeModule
                    | Kind::ProtocolConformanceRefInProtocolModule
            ) {
                id
            } else {
                self.stack.push(id);
                self.pop_retroactive_protocol_conformance_ref()?
            }
        } else {
            self.pop_retroactive_protocol_conformance_ref()?
        };
        let ty = self.pop_type()?;
        self.node_with(
            Kind::ConcreteProtocolConformance,
            &[ty, reference, conditionals],
        )
        .ok()
    }

    /// Pop a protocol nominally, accepting an already-built protocol type.
    fn pop_protocol(&mut self) -> Option<NodeRef<'a>> {
        if let Some(id) = self.pop_any() {
            if id.kind() == Kind::Type {
                let child = id.child(0);
                if child
                    .as_ref()
                    .is_some_and(|child| child.kind() == Kind::Protocol)
                {
                    return Some(id);
                }
                self.stack.push(id);
                return None;
            }
            if id.kind().is_nominal() {
                return Some(id);
            }
            self.stack.push(id);
        }
        let name = self.pop_decl_name()?;
        let context = self.pop_context_any()?;
        let protocol = self.node_with(Kind::Protocol, &[context, name]).ok()?;
        self.wrap_type(protocol).ok()
    }

    /// Pop a protocol conformance record.
    fn pop_protocol_conformance(&mut self) -> Option<NodeRef<'a>> {
        let signature = self.pop_kind(Kind::DependentGenericSignature);
        let module = self.pop_module()?;
        let protocol = self.pop_protocol()?;
        let ty = self.pop_type()?;
        let mut node = self.builder(Kind::ProtocolConformance).ok()?;
        if let Some(signature) = signature {
            let signature = self
                .typed(Kind::DependentGenericType, &[signature, ty])
                .ok()?;
            node.push(signature);
        } else {
            node.push(ty);
        }
        node.push(protocol);
        node.push(module);
        Some(node.build())
    }

    fn wrap_type(&mut self, child: NodeRef<'a>) -> PResult<'a> {
        let mut ty = self.builder(Kind::Type)?;
        ty.push(child);
        Ok(ty.build())
    }

    /// Start a node; the caller pushes children and calls `Builder::build`.
    fn builder(&mut self, kind: Kind) -> PResult<'a, Builder<'a>> {
        let builder = self.context.builder(kind);
        self.check_budget(builder)
    }

    fn alloc(&mut self, kind: Kind) -> PResult<'a> {
        let builder = self.builder(kind)?;
        Ok(builder.build())
    }

    fn alloc_text(&mut self, kind: Kind, text: &'a str) -> PResult<'a> {
        let builder = self.context.builder_text(kind, text);
        Ok(self.check_budget(builder)?.build())
    }

    /// Allocate a text node from borrowed or escaped identifier text.
    fn alloc_cow(&mut self, kind: Kind, text: Cow<'a, str>) -> PResult<'a> {
        let builder = self.context.builder_text_cow(kind, text);
        Ok(self.check_budget(builder)?.build())
    }

    fn alloc_owned(&mut self, kind: Kind, text: &str) -> PResult<'a> {
        let builder = self.context.builder_owned(kind, text);
        Ok(self.check_budget(builder)?.build())
    }

    fn check_budget<T>(&self, value: T) -> PResult<'a, T> {
        if self.context.failed() {
            Err(ParseError)
        } else {
            Ok(value)
        }
    }

    /// Build a numeric node, charging it against the node budget.
    fn alloc_value(&mut self, kind: Kind, value: u32) -> PResult<'a> {
        let builder = self.context.builder_value(kind, value);
        Ok(self.check_budget(builder)?.build())
    }

    /// Start a global wrapper; the caller pushes children and builds it.
    fn global_builder(&mut self, code: Global, operand: GlobalOperand) -> PResult<'a, Builder<'a>> {
        let builder = self.context.builder_global(code, operand);
        self.check_budget(builder)
    }

    fn node_with(&mut self, kind: Kind, children: &[NodeRef<'a>]) -> PResult<'a> {
        let mut node = self.builder(kind)?;
        for child in children {
            node.push(child.clone());
        }
        Ok(node.build())
    }

    /// Like [`Self::node_with`], but absent children keep their position.
    fn node_with_opt(&mut self, kind: Kind, children: &[Option<NodeRef<'a>>]) -> PResult<'a> {
        let mut node = self.builder(kind)?;
        for child in children.iter().flatten() {
            node.push(child.clone());
        }
        Ok(node.build())
    }

    fn typed(&mut self, kind: Kind, children: &[NodeRef<'a>]) -> PResult<'a> {
        let node = self.node_with(kind, children)?;
        self.wrap_type(node)
    }

    fn with_children(&mut self, kind: Kind, children: &[NodeRef<'a>]) -> PResult<'a> {
        self.node_with(kind, children)
    }

    /// Like [`Self::with_children`], but absent children keep their position.
    fn with_children_opt(&mut self, kind: Kind, children: &[Option<NodeRef<'a>>]) -> PResult<'a> {
        self.node_with_opt(kind, children)
    }

    // ---- operators -----------------------------------------------------

    fn enter(&mut self) -> PResult<'a, ()> {
        self.input.enter().map_err(|_| ParseError)
    }

    const fn leave(&mut self) {
        self.input.leave();
    }

    fn operator(&mut self) -> PResult<'a> {
        self.enter()?;
        let start = self.input.pos();
        let start_byte = self.peek().unwrap_or(0);
        let result = self.operator_inner();
        if result.is_err() {
            self.fail_pos = start;
            self.fail_byte = start_byte;
        }
        self.leave();
        result
    }

    fn operator_inner(&mut self) -> PResult<'a> {
        let checkpoint = self.input.checkpoint();
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
            b'g' => self.retroactive_conformance(),
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
                self.input.reset(checkpoint);
                self.identifier()
            }
        }
    }

    fn identifier(&mut self) -> PResult<'a> {
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
        if !word_substs && !punycoded {
            let count = self.decimal()?;
            if count == 0 {
                return Err(ParseError);
            }
            let count = usize::try_from(count).map_err(|_| ParseError)?;
            let slice = self.input.advance(count).map_err(|_| ParseError)?;
            self.collect_words(slice);
            // Identifier bytes: a length can split a UTF-8 sequence, so text
            // is escaped rather than decoded.
            let node = self.alloc_cow(Kind::Identifier, text::cow(slice, Escape::Identifier))?;
            self.context.add_sub(node.clone());
            return Ok(node);
        }
        let mut assembled: [u8; 512] = [0; 512];
        let mut len = 0_usize;
        loop {
            while word_substs {
                let Some(byte) = self.peek() else { break };
                if !byte.is_ascii_alphabetic() {
                    break;
                }
                self.advance(1)?;
                let (index, last) = if byte.is_ascii_lowercase() {
                    (usize::from(byte - b'a'), false)
                } else {
                    (usize::from(byte - b'A'), true)
                };
                if index >= self.context.word_count() {
                    return Err(ParseError);
                }
                let bytes = self.context.word(index);
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
            let count = self.decimal()?;
            if count == 0 {
                return Err(ParseError);
            }
            let count = usize::try_from(count).map_err(|_| ParseError)?;
            if punycoded {
                // A Punycode payload that starts with a digit or `_` carries an
                // extra `_` between its length and the payload; the length
                // counts only the payload (Mangling.rst, "Identifiers").
                let _ = self.next_if(b'_');
            }
            let slice = self.input.advance(count).map_err(|_| ParseError)?;
            if punycoded {
                let mut decoded = [0_u8; 512];
                let decoded_len = punycode_decode(slice, &mut decoded).ok_or(ParseError)?;
                let dst = len.checked_add(decoded_len).ok_or(ParseError)?;
                if dst > assembled.len() {
                    return Err(ParseError);
                }
                assembled[len..dst].copy_from_slice(&decoded[..decoded_len]);
                len = dst;
            } else {
                let dst = len.checked_add(slice.len()).ok_or(ParseError)?;
                if dst > assembled.len() {
                    return Err(ParseError);
                }
                assembled[len..dst].copy_from_slice(slice);
                len = dst;
            }
            // Record words from the literal slice.
            self.collect_words(slice);
            if !word_substs {
                break;
            }
        }
        if len == 0 {
            return Err(ParseError);
        }
        let text = text::owned(&assembled[..len], Escape::Identifier);
        let node = self.alloc_cow(Kind::Identifier, Cow::Owned(text))?;
        self.context.add_sub(node.clone());
        Ok(node)
    }

    /// Record the word-substitution table for one identifier's bytes.
    ///
    /// Words stay raw bytes: substitution rebuilds the original identifier
    /// bytes, which are escaped once when the node's text is created.
    fn collect_words(&mut self, slice: &'a [u8]) {
        // Mirrors the reference demangler: a word starts at a non-digit,
        // non-underscore character and ends at `_`, end-of-slice, or an
        // uppercase letter that follows a non-uppercase one.
        let mut start: Option<usize> = None;
        for index in 0..=slice.len() {
            let byte = slice.get(index).copied().unwrap_or(0);
            if let Some(begin) = start {
                let previous = slice[index - 1];
                let ends = byte == b'_'
                    || byte == 0
                    || (!previous.is_ascii_uppercase() && byte.is_ascii_uppercase());
                if ends {
                    if index - begin >= 2 {
                        self.context.add_word(&slice[begin..index]);
                    }
                    start = None;
                }
            }
            if start.is_none() && byte != 0 && byte != b'_' && !byte.is_ascii_digit() {
                start = Some(index);
            }
        }
    }

    fn multi_substitutions(&mut self) -> PResult<'a> {
        let mut repeat: Option<u32> = None;
        loop {
            let checkpoint = self.input.checkpoint();
            let byte = self.next_byte().ok_or(ParseError)?;
            if byte.is_ascii_lowercase() {
                let index = usize::from(byte - b'a');
                let node = self.substitution(index)?;
                let times = match repeat.take() {
                    Some(count) if count > 1 => count,
                    _ => 1,
                };
                self.push_repeated(&node, times)?;
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
                self.push_repeated(&node, times)?;
                return Ok(node);
            }
            if byte == b'_' {
                let index = repeat
                    .take()
                    .map_or(26, |encoded| encoded.saturating_add(27));
                return self.substitution(index as usize);
            }
            self.input.reset(checkpoint);
            repeat = Some(self.natural().ok_or(ParseError)?);
        }
    }

    /// Push a substitution `times` times.
    ///
    /// The repeat count comes from the input, so the pushes are charged to the
    /// work budget: a short substitution cannot grow the stack without bound.
    fn push_repeated(&mut self, node: &NodeRef<'a>, times: u32) -> PResult<'a, ()> {
        if self
            .input
            .charge(usize::try_from(times).unwrap_or(usize::MAX))
            .is_err()
        {
            return Err(ParseError);
        }
        for _ in 0..times {
            self.push(node.clone());
        }
        Ok(())
    }

    fn substitution(&self, index: usize) -> PResult<'a> {
        self.context.sub(index).ok_or(ParseError)
    }

    fn standard_substitution(&mut self) -> PResult<'a> {
        let checkpoint = self.input.checkpoint();
        match self.next_byte().ok_or(ParseError)? {
            b'o' => self.alloc_text(Kind::Module, "__C"),
            b'C' => self.alloc_text(Kind::Module, "__C_Synthesized"),
            b'g' => {
                // Each optional layer becomes a nested AST node. Count all
                // `Sg` layers so a long chain cannot build a tree deep enough
                // to overflow the stack while being dropped or printed.
                self.optional_layers = self.optional_layers.saturating_add(1);
                if self.optional_layers > self.limits.depth() {
                    self.input.exhaust();
                    return Err(ParseError);
                }
                let optional = self.swift_type(Kind::BoundGenericEnum, "Optional")?;
                let mut list = self.builder(Kind::TypeList)?;
                if let Some(ty) = self.pop_type() {
                    list.push(ty);
                }
                let node = self.with_children(Kind::BoundGenericEnum, &[optional, list.build()])?;
                let ty = self.wrap_type(node)?;
                self.context.add_sub(ty.clone());
                Ok(ty)
            }
            _ => {
                self.input.reset(checkpoint);
                let repeat = self.natural().unwrap_or(1).max(1);
                let second = self.next_if(b'c');
                let kind = self.next_byte().ok_or(ParseError)?;
                let name = standard_type_name(kind, second).ok_or(ParseError)?;
                let (kind, name) = name;
                let node = self.swift_type(kind, name)?;
                let ty = self.wrap_type(node)?;
                for _ in 1..repeat {
                    self.push(ty.clone());
                }
                Ok(ty)
            }
        }
    }

    fn swift_type(&mut self, kind: Kind, name: &'static str) -> PResult<'a> {
        let module = self.alloc_text(Kind::Module, "Swift")?;
        let ident = self.alloc_text(Kind::Identifier, name)?;
        self.with_children(kind, &[module, ident])
    }

    fn builtin_type(&mut self) -> PResult<'a> {
        let code = self.next_byte().ok_or(ParseError)?;
        match code {
            b'f' | b'i' => {
                let width = self.natural().ok_or(ParseError)?;
                if !self.next_if(b'_') {
                    return Err(ParseError);
                }
                // A width is printed as a number, so its size costs no work.
                if width == 0 {
                    return Err(ParseError);
                }
                let mut buf = [0_u8; 40];
                let prefix = if code == b'f' {
                    "Builtin.FPIEEE"
                } else {
                    "Builtin.Int"
                };
                let text = format_builtin(&mut buf, prefix, width);
                let node = self.alloc_owned(Kind::BuiltinTypeName, text)?;
                self.wrap_type(node)
            }
            b'v' => {
                let count = self.natural().ok_or(ParseError)?;
                if !self.next_if(b'_') {
                    return Err(ParseError);
                }
                if count == 0 {
                    return Err(ParseError);
                }
                let element = self.pop_type_child().ok_or(ParseError)?;
                let mut buf = [0_u8; 40];
                let text = format_builtin(&mut buf, "Builtin.Vec", count);
                let builder = self.context.builder_owned(Kind::BuiltinVector, text);
                let mut node = self.check_budget(builder)?;
                node.push(element);
                self.wrap_type(node.build())
            }
            b'V' => {
                let element = self.pop_type().ok_or(ParseError)?;
                let size = self.pop_type().ok_or(ParseError)?;
                let mut node = self.builder(Kind::BuiltinFixedArray)?;
                node.push(size);
                node.push(element);
                self.wrap_type(node.build())
            }
            b'W' => {
                let referent = self.pop_type().ok_or(ParseError)?;
                let mut node = self.builder(Kind::BuiltinBorrow)?;
                node.push(referent);
                self.wrap_type(node.build())
            }
            _ => {
                let name = builtin_name(code).ok_or(ParseError)?;
                let node = self.alloc_text(Kind::BuiltinTypeName, name)?;
                self.wrap_type(node)
            }
        }
    }

    fn any_generic_type(&mut self, kind: Kind) -> PResult<'a> {
        let name = self.pop_decl_name().ok_or(ParseError)?;
        let context = self.pop_context_any().ok_or(ParseError)?;
        let node = self.typed(kind, &[context, name])?;
        self.context.add_sub(node.clone());
        Ok(node)
    }

    fn type_mangling(&mut self) -> PResult<'a> {
        let ty = self.pop_type().ok_or(ParseError)?;
        let node = self.node_with(Kind::TypeMangling, &[ty])?;
        Ok(node)
    }

    fn bound_generic_type(&mut self) -> PResult<'a> {
        let mut conformances = Vec::new();
        while let Some(id) = self.stack.last().cloned() {
            if id.kind() != Kind::RetroactiveConformance {
                break;
            }
            let _ = self.stack.pop();
            conformances.push(id);
        }
        let retroactive = if conformances.is_empty() {
            None
        } else {
            let mut list = self.builder(Kind::TypeList)?;
            for conformance in conformances.into_iter().rev() {
                list.push(conformance);
            }
            Some(list.build())
        };
        // Collect type lists: 'y' terminates a level; '_' separates levels.
        let mut lists: Vec<NodeRef<'a>> = Vec::new();
        loop {
            let mut list = self.builder(Kind::TypeList)?;
            let mut elements: Vec<NodeRef<'a>> = Vec::new();
            while let Some(ty) = self.pop_type() {
                elements.push(ty);
            }
            // Elements were popped in reverse order; append reversed.
            for element in elements.into_iter().rev() {
                list.push(element);
            }
            lists.push(list.build());
            if self.pop_kind(Kind::EmptyList).is_some() {
                break;
            }
            if self.pop_kind(Kind::FirstElementMarker).is_none() {
                return Err(ParseError);
            }
        }
        let nominal = self.pop_any_generic().ok_or(ParseError)?;
        if lists.is_empty() || lists.iter().all(|list| list.child_count() == 0) {
            return Err(ParseError);
        }
        let bound = self.bind_generic_args(nominal, &lists, 0)?;
        // The nominal may be shared through a substitution, so the retroactive
        // conformance is attached to a copy.
        let bound = match retroactive {
            Some(retroactive) => self.context.append(&bound, retroactive),
            None => bound,
        };
        let ty = self.wrap_type(bound)?;
        self.context.add_sub(ty.clone());
        Ok(ty)
    }

    /// Apply generic argument lists to nested nominal contexts.
    ///
    /// Arguments for the outermost type come first; each list is attached to
    /// the nominal it belongs to, rebuilding contexts from the outside in.
    fn bind_generic_args(
        &mut self,
        nominal: NodeRef<'a>,
        lists: &[NodeRef<'a>],
        index: usize,
    ) -> PResult<'a> {
        if index >= lists.len() {
            return Err(ParseError);
        }
        let kind = nominal.kind();
        let consumes = !matches!(
            kind,
            Kind::Variable
                | Kind::Subscript
                | Kind::ImplicitClosure
                | Kind::ExplicitClosure
                | Kind::DefaultArgumentInitializer
                | Kind::Initializer
                | Kind::PropertyWrapperBackingInitializer
                | Kind::PropertyWrappedFieldInitAccessor
                | Kind::PropertyWrapperInitFromProjectedValue
                | Kind::Static
        );
        let args = lists[index].clone();
        let mut next = if consumes { index + 1 } else { index };
        if kind == Kind::Subscript {
            while next < lists.len() && lists[next].child_count() == 0 {
                next += 1;
            }
        }
        let context = nominal.child(0);
        let current = if next < lists.len() {
            let Some(context) = context else {
                return Err(ParseError);
            };
            let parent = if context.kind() == Kind::Extension {
                let module = context.child(0);
                let inner = context.child(1).ok_or(ParseError)?;
                let signature = context.child(2);
                let bound_inner = self.bind_generic_args(inner, lists, next)?;
                let mut extension = self.builder(Kind::Extension)?;
                extension.push_opt(module);
                extension.push(bound_inner);
                extension.push_opt(signature);
                extension.build()
            } else {
                self.bind_generic_args(context, lists, next)?
            };
            let mut children = Vec::with_capacity(nominal.child_count());
            children.push(parent);
            children.extend(nominal.children().iter().skip(1).cloned());
            let rebuilt = self.context.rebuild_with_children(&nominal, kind, children);
            self.check_budget(rebuilt)?
        } else {
            nominal
        };
        if !consumes || args.child_count() == 0 {
            return Ok(current);
        }
        let bound_kind = match kind {
            Kind::Structure => Kind::BoundGenericStructure,
            Kind::Class => Kind::BoundGenericClass,
            Kind::Enum => Kind::BoundGenericEnum,
            Kind::Protocol => Kind::BoundGenericProtocol,
            Kind::Function | Kind::Constructor => Kind::BoundGenericFunction,
            _ => Kind::BoundGenericOtherNominal,
        };
        self.with_children(bound_kind, &[current, args])
    }

    fn function_type(&mut self, kind: Kind) -> PResult<'a> {
        self.function_type_node(kind)
    }

    fn function_type_node(&mut self, kind: Kind) -> PResult<'a> {
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
        let node = self.with_children_opt(
            kind,
            &[
                sending,
                isolated,
                differentiable,
                throws,
                sendable,
                is_async,
                Some(params),
                Some(result),
            ],
        )?;
        self.wrap_type(node)
    }

    fn function_params(&mut self, kind: Kind) -> PResult<'a> {
        let param = if self.pop_kind(Kind::EmptyList).is_some() {
            let tuple = self.alloc(Kind::Tuple)?;
            self.wrap_type(tuple)?
        } else {
            self.pop_type().ok_or(ParseError)?
        };
        self.node_with(kind, &[param])
    }

    fn pop_any_of(&mut self, kinds: &[Kind]) -> Option<NodeRef<'a>> {
        let id = self.stack.last()?.clone();
        let kind = id.kind();
        if kinds.contains(&kind) {
            let _ = self.stack.pop();
            Some(id)
        } else {
            None
        }
    }

    fn plain_function(&mut self) -> PResult<'a> {
        let generic = self.pop_kind(Kind::DependentGenericSignature);
        let ty = self.function_type_node(Kind::FunctionType)?;
        let labels = self.pop_function_param_labels(&ty)?;
        let ty = if let Some(generic) = generic {
            self.typed(Kind::DependentGenericType, &[generic, ty])?
        } else {
            ty
        };
        let name = self.pop_decl_name().ok_or(ParseError)?;
        let context = self.pop_context_any().ok_or(ParseError)?;
        self.with_children_opt(
            Kind::Function,
            &[Some(context), Some(name), labels, Some(ty)],
        )
    }

    /// Pop the parameter labels of a newly parsed function type.
    ///
    /// New-style manglings encode labels as operands pushed before the type;
    /// they are popped here and stored in a [`Kind::LabelList`]. Old-style
    /// (`_T`) manglings embed the labels in the tuple elements instead.
    fn pop_function_param_labels(&mut self, ty: &NodeRef<'a>) -> PResult<'a, Option<NodeRef<'a>>> {
        if !self.old_function_type && self.pop_kind(Kind::EmptyList).is_some() {
            return Ok(Some(self.alloc(Kind::LabelList)?));
        }
        let mut func = ty.child(0);
        if let Some(node) = func.clone()
            && node.kind() == Kind::DependentGenericType
        {
            let inner = node.child(1);
            func = match inner {
                Some(inner) if inner.kind() == Kind::Type => inner.child(0),
                other => other,
            };
        }
        let Some(func) = func else {
            return Ok(None);
        };
        let Some(args) = Self::find_child(&func, Kind::ArgumentTuple) else {
            return Ok(None);
        };
        let params = args.child(0);
        let params = match params {
            Some(params) if params.kind() == Kind::Type => params.child(0),
            other => other,
        };
        let count = match &params {
            Some(params) if params.kind() == Kind::Tuple => params.child_count(),
            _ => 1,
        };
        if count == 0 {
            return Ok(None);
        }
        if self.old_function_type {
            return self.old_function_param_labels(params);
        }
        let mut labels = Vec::new();
        while labels.len() < count {
            let label = self.pop_any().ok_or(ParseError)?;
            match label.kind() {
                Kind::Identifier | Kind::FirstElementMarker => {}
                _ => return Err(ParseError),
            }
            labels.push(label);
        }
        let mut list = self.builder(Kind::LabelList)?;
        let has_labels = labels.iter().any(|label| label.kind() == Kind::Identifier);
        if !has_labels {
            return Ok(Some(list.build()));
        }
        for label in labels.into_iter().rev() {
            list.push(label);
        }
        Ok(Some(list.build()))
    }

    /// The labels of an old-style (`_T0`) parameter tuple.
    ///
    /// Old manglings carry each label as the element's `TupleElementName`;
    /// like the reference, they are collected into a label list in which an
    /// unlabeled element is a [`Kind::FirstElementMarker`], so it prints as
    /// `_:`. A tuple without any label gets an empty list.
    fn old_function_param_labels(
        &mut self,
        params: Option<NodeRef<'a>>,
    ) -> PResult<'a, Option<NodeRef<'a>>> {
        let Some(params) = params.filter(|params| params.kind() == Kind::Tuple) else {
            return Ok(None);
        };
        let mut labels = Vec::with_capacity(params.child_count());
        for element in params.children() {
            let name = element
                .children()
                .iter()
                .find(|child| child.kind() == Kind::TupleElementName);
            labels.push(match name.and_then(|name| name.text_cow()) {
                Some(text) => self.alloc_cow(Kind::Identifier, text)?,
                None => self.alloc(Kind::FirstElementMarker)?,
            });
        }
        let mut list = self.builder(Kind::LabelList)?;
        if labels.iter().any(|label| label.kind() == Kind::Identifier) {
            for label in labels {
                list.push(label);
            }
        }
        Ok(Some(list.build()))
    }

    fn find_child(parent: &NodeRef<'a>, kind: Kind) -> Option<NodeRef<'a>> {
        parent
            .children()
            .iter()
            .find(|child| child.kind() == kind)
            .cloned()
    }

    fn pop_decl_name(&mut self) -> Option<NodeRef<'a>> {
        let id = self.pop_any()?;
        if id.kind().is_decl_name() {
            Some(id)
        } else {
            self.stack.push(id);
            None
        }
    }

    fn extension_context(&mut self) -> PResult<'a> {
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

    fn generic_type(&mut self) -> PResult<'a> {
        let signature = self
            .pop_kind(Kind::DependentGenericSignature)
            .ok_or(ParseError)?;
        let ty = self.pop_type().ok_or(ParseError)?;
        let node = self.node_with(Kind::DependentGenericType, &[signature, ty])?;
        self.wrap_type(node)
    }

    fn generic_signature(&mut self, has_counts: bool) -> PResult<'a> {
        let mut sig = self.builder(Kind::DependentGenericSignature)?;
        if has_counts {
            while !self.next_if(b'l') {
                let count = if self.next_if(b'z') {
                    0
                } else {
                    self.index().ok_or(ParseError)?.saturating_add(1)
                };
                let node = self.alloc_value(Kind::DependentGenericParamCount, count)?;
                sig.push(node);
            }
        } else {
            let node = self.alloc_value(Kind::DependentGenericParamCount, 1)?;
            sig.push(node);
        }
        // Requirements are already on the stack in mangling order; move them
        // into the signature in reverse to restore source order.
        let mut requirements = Vec::new();
        while let Some(node) = self.pop_requirement() {
            requirements.push(node);
        }
        for requirement in requirements.into_iter().rev() {
            sig.push(requirement);
        }
        Ok(sig.build())
    }

    fn pop_requirement(&mut self) -> Option<NodeRef<'a>> {
        let id = self.stack.last()?.clone();
        if matches!(
            id.kind(),
            Kind::DependentGenericConformanceRequirement
                | Kind::DependentGenericSameTypeRequirement
                | Kind::DependentGenericBaseClassRequirement
                | Kind::DependentGenericLayoutRequirement
                | Kind::DependentGenericInverseConformanceRequirement
        ) {
            let _ = self.stack.pop();
            Some(id)
        } else {
            None
        }
    }

    fn generic_requirement(&mut self) -> PResult<'a> {
        let checkpoint = self.input.checkpoint();
        let code = self.next_byte().ok_or(ParseError)?;
        let (constraint, subject_kind) = match code {
            b'V' => (ReqConstraint::ValueMarker, ReqType::Generic),
            b'v' => (ReqConstraint::PackMarker, ReqType::Generic),
            b'c' => (ReqConstraint::BaseClass, ReqType::Assoc),
            b'C' => (ReqConstraint::BaseClass, ReqType::CompoundAssoc),
            b'b' => (ReqConstraint::BaseClass, ReqType::Generic),
            b'B' => (ReqConstraint::BaseClass, ReqType::Substitution),
            b't' => (ReqConstraint::SameType, ReqType::Assoc),
            b'T' => (ReqConstraint::SameType, ReqType::CompoundAssoc),
            b's' => (ReqConstraint::SameType, ReqType::Generic),
            b'S' => (ReqConstraint::SameType, ReqType::Substitution),
            b'm' => (ReqConstraint::Layout, ReqType::Assoc),
            b'M' => (ReqConstraint::Layout, ReqType::CompoundAssoc),
            b'l' => (ReqConstraint::Layout, ReqType::Generic),
            b'L' => (ReqConstraint::Layout, ReqType::Substitution),
            b'p' => (ReqConstraint::Protocol, ReqType::Assoc),
            b'P' => (ReqConstraint::Protocol, ReqType::CompoundAssoc),
            b'Q' => (ReqConstraint::Protocol, ReqType::Substitution),
            b'h' => (ReqConstraint::SameShape, ReqType::Generic),
            b'i' => (ReqConstraint::Inverse, ReqType::Generic),
            b'I' => (ReqConstraint::Inverse, ReqType::Substitution),
            b'j' => (ReqConstraint::Inverse, ReqType::Assoc),
            b'J' => (ReqConstraint::Inverse, ReqType::CompoundAssoc),
            _ => {
                self.input.reset(checkpoint);
                (ReqConstraint::Protocol, ReqType::Generic)
            }
        };
        let inverse = if matches!(constraint, ReqConstraint::Inverse) {
            let index = self.index().ok_or(ParseError)?;
            Some(self.alloc_value(Kind::Number, index)?)
        } else {
            None
        };
        let subject = match subject_kind {
            ReqType::Generic => self.generic_param()?,
            ReqType::Assoc => {
                let base = self.generic_param()?;
                let ty = self.assoc_type_simple(Some(base))?;
                self.context.add_sub(ty.clone());
                ty
            }
            ReqType::CompoundAssoc => {
                let base = self.generic_param()?;
                let ty = self.assoc_type_compound(Some(base))?;
                self.context.add_sub(ty.clone());
                ty
            }
            ReqType::Substitution => self.pop_type().ok_or(ParseError)?,
        };
        match constraint {
            ReqConstraint::Protocol => {
                let protocol = self.pop_protocol().ok_or(ParseError)?;
                self.with_children(
                    Kind::DependentGenericConformanceRequirement,
                    &[subject, protocol],
                )
            }
            ReqConstraint::SameType
            | ReqConstraint::SameShape
            | ReqConstraint::ValueMarker
            | ReqConstraint::PackMarker => {
                let other = self.pop_type().ok_or(ParseError)?;
                self.with_children(Kind::DependentGenericSameTypeRequirement, &[subject, other])
            }
            ReqConstraint::BaseClass => {
                let other = self.pop_type().ok_or(ParseError)?;
                self.with_children(
                    Kind::DependentGenericBaseClassRequirement,
                    &[subject, other],
                )
            }
            ReqConstraint::Layout => {
                let byte = self.next_byte().ok_or(ParseError)?;
                let code = LayoutCode::from_byte(byte).ok_or(ParseError)?;
                let builder = self.context.builder_layout(byte).ok_or(ParseError)?;
                let mut node = self.check_budget(builder)?;
                node.push(subject);
                // `E`/`M` carry a size and an alignment, `e`/`m`/`S` a size.
                if code.takes_size() {
                    let size = self.index().ok_or(ParseError)?;
                    let size = self.alloc_value(Kind::Number, size)?;
                    node.push(size);
                    if code.takes_alignment() {
                        let alignment = self.index().ok_or(ParseError)?;
                        let alignment = self.alloc_value(Kind::Number, alignment)?;
                        node.push(alignment);
                    }
                }
                Ok(node.build())
            }
            ReqConstraint::Inverse => self.with_children_opt(
                Kind::DependentGenericInverseConformanceRequirement,
                &[Some(subject), inverse],
            ),
        }
    }

    fn generic_param_typed(&mut self) -> PResult<'a> {
        let node = self.generic_param()?;
        self.wrap_type(node)
    }

    fn generic_param(&mut self) -> PResult<'a> {
        if self.next_if(b'd') {
            let depth = self.index().ok_or(ParseError)?.saturating_add(1);
            let index = self.index().ok_or(ParseError)?;
            return self.builtin_generic_param(depth, index);
        }
        if self.next_if(b'z') {
            return self.builtin_generic_param(0, 0);
        }
        if self.next_if(b's') {
            let node = self.alloc(Kind::ConstrainedExistentialSelf)?;
            return self.wrap_type(node);
        }
        let index = self.index().ok_or(ParseError)?.saturating_add(1);
        self.builtin_generic_param(0, index)
    }

    fn builtin_generic_param(&mut self, depth: u32, index: u32) -> PResult<'a> {
        let mut node = self.builder(Kind::DependentGenericParamType)?;
        node.set_value(depth);
        node.set_len(index);
        self.wrap_type(node.build())
    }

    fn tuple_type(&mut self) -> PResult<'a> {
        let mut root = self.builder(Kind::Tuple)?;
        if self.pop_kind(Kind::EmptyList).is_none() {
            let mut elements: Vec<NodeRef<'a>> = Vec::new();
            loop {
                let first = self.pop_kind(Kind::FirstElementMarker).is_some();
                let mut element = self.builder(Kind::TupleElement)?;
                if let Some(variadic) = self.pop_kind(Kind::VariadicMarker) {
                    element.push(variadic);
                }
                if let Some(name) = self.pop_kind(Kind::Identifier) {
                    let name_node = self.context.relabel(&name, Kind::TupleElementName);
                    element.push(name_node);
                }
                let ty = self.pop_type().ok_or(ParseError)?;
                element.push(ty);
                elements.push(element.build());
                if first {
                    break;
                }
            }
            for element in elements.into_iter().rev() {
                root.push(element);
            }
        }
        self.wrap_type(root.build())
    }

    fn protocol_list(&mut self) -> PResult<'a, NodeRef<'a>> {
        let mut list = self.builder(Kind::ProtocolList)?;
        if self.pop_kind(Kind::EmptyList).is_none() {
            let elements = self
                .pop_reverse_list(Self::pop_protocol)
                .ok_or(ParseError)?;
            for element in elements.into_iter().rev() {
                list.push(element);
            }
        }
        Ok(list.build())
    }

    fn protocol_list_type(&mut self) -> PResult<'a> {
        let mut list = self.builder(Kind::ProtocolList)?;
        if self.pop_kind(Kind::EmptyList).is_none() {
            let elements = self
                .pop_reverse_list(Self::pop_protocol)
                .ok_or(ParseError)?;
            for element in elements.into_iter().rev() {
                list.push(element);
            }
        }
        self.typed(Kind::ProtocolListType, &[list.build()])
    }

    fn metatype_type(&mut self) -> PResult<'a> {
        let ty = self.pop_type().ok_or(ParseError)?;
        let node = self.typed(Kind::Metatype, &[ty])?;
        Ok(node)
    }

    fn type_wrapper(&mut self, kind: Kind) -> PResult<'a> {
        // These wrappers are built iteratively, so the cursor depth budget does
        // not grow. Count them explicitly: a rejected tree must not be deep
        // enough to overflow the stack while `Rc` teardown runs.
        self.wrapper_layers = self.wrapper_layers.saturating_add(1);
        if self.wrapper_layers > self.limits.depth() {
            self.input.exhaust();
            return Err(ParseError);
        }
        let ty = self.pop_type_child().ok_or(ParseError)?;
        let node = self.typed(kind, &[ty])?;
        Ok(node)
    }

    fn static_entity(&mut self) -> PResult<'a> {
        let entity = self.pop_any().ok_or(ParseError)?;
        let node = self.with_children(Kind::Static, &[entity])?;
        Ok(node)
    }

    fn local_identifier(&mut self) -> PResult<'a> {
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
            self.advance(1)?;
            // One ASCII letter: the conversion cannot fail.
            let text = core::str::from_utf8(&related).map_err(|_| ParseError)?;
            let kind = self.alloc_owned(Kind::Identifier, text)?;
            let name = self.pop_any().ok_or(ParseError)?;
            return self.with_children(Kind::RelatedEntityDeclName, &[kind, name]);
        }
        let discriminator = self.index().ok_or(ParseError)?;
        let number = self.alloc_value(Kind::Number, discriminator)?;
        let name = self.pop_decl_name().ok_or(ParseError)?;
        self.with_children(Kind::LocalDeclName, &[number, name])
    }

    fn variable(&mut self) -> PResult<'a> {
        let ty = self.pop_type().ok_or(ParseError)?;
        let _labels = self.pop_kind(Kind::EmptyList);
        let name = self.pop_decl_name().ok_or(ParseError)?;
        let accessor = self.next_accessor();
        let context = self.pop_context_any().ok_or(ParseError)?;
        let mut node = match accessor {
            Some(code) => {
                let accessor = Accessor::from_code(accessor_code(code)).ok_or(ParseError)?;
                let builder = self.context.builder_accessor(Kind::Variable, accessor);
                self.check_budget(builder)?
            }
            None => self.builder(Kind::Variable)?,
        };
        node.push(context);
        node.push(name);
        node.push(ty);
        Ok(node.build())
    }

    fn next_accessor(&mut self) -> Option<u8> {
        let checkpoint = self.input.checkpoint();
        let byte = self.next_byte()?;
        match byte {
            b'g' | b'G' | b's' | b'm' | b'r' | b'M' | b'w' | b'W' | b'x' | b'y' | b'b' | b'z'
            | b'p' => Some(byte),
            b'a' | b'l' => {
                let _ = self.next_byte();
                Some(byte)
            }
            _ => {
                self.input.reset(checkpoint);
                None
            }
        }
    }

    fn subscript(&mut self) -> PResult<'a> {
        let ty = self.pop_type().ok_or(ParseError)?;
        let labels = self.pop_function_param_labels(&ty)?;
        let accessor = self.next_accessor();
        let context = self.pop_context_any().ok_or(ParseError)?;
        let mut node = match accessor {
            Some(code) => {
                let accessor = Accessor::from_code(accessor_code(code)).ok_or(ParseError)?;
                let builder = self.context.builder_accessor(Kind::Subscript, accessor);
                self.check_budget(builder)?
            }
            None => self.builder(Kind::Subscript)?,
        };
        node.push(context);
        node.push_opt(labels);
        node.push(ty);
        Ok(node.build())
    }

    fn function_entity(&mut self) -> PResult<'a> {
        let code = self.next_byte().ok_or(ParseError)?;
        match code {
            b'C' | b'c' => {
                let private = self.pop_kind(Kind::PrivateDeclName);
                let ty = self.pop_type().ok_or(ParseError)?;
                let labels = self.pop_function_param_labels(&ty)?;
                let context = self.pop_context_any().ok_or(ParseError)?;
                let kind = if code == b'C' {
                    Kind::Allocator
                } else {
                    Kind::Constructor
                };
                self.with_children_opt(kind, &[Some(context), labels, Some(ty), private])
            }
            b'D' | b'd' => {
                let context = self.pop_context_any().ok_or(ParseError)?;
                let kind = if code == b'D' {
                    DeinitializerKind::Deallocating
                } else {
                    DeinitializerKind::Deinit
                };
                let builder = self.context.builder_deinit(kind);
                let mut node = self.check_budget(builder)?;
                node.push(context);
                Ok(node.build())
            }
            b'U' | b'u' => {
                let index = self.index().ok_or(ParseError)?;
                let ty = self.pop_type().ok_or(ParseError)?;
                let context = self.pop_context_any().ok_or(ParseError)?;
                let kind = if code == b'U' {
                    Kind::ExplicitClosure
                } else {
                    Kind::ImplicitClosure
                };
                let builder = self.context.builder_index(kind, index);
                let mut node = self.check_budget(builder)?;
                node.push(context);
                node.push(ty);
                Ok(node.build())
            }
            b'A' => {
                let index = self.index().ok_or(ParseError)?;
                let context = self.pop_context_any().ok_or(ParseError)?;
                let builder = self
                    .context
                    .builder_index(Kind::DefaultArgumentInitializer, index);
                let mut node = self.check_budget(builder)?;
                node.push(context);
                Ok(node.build())
            }
            b'i' => {
                let context = self.pop_context_any().ok_or(ParseError)?;
                self.with_children(Kind::Initializer, &[context])
            }
            b'P' => {
                let context = self.pop_context_any().ok_or(ParseError)?;
                self.with_children(Kind::PropertyWrapperBackingInitializer, &[context])
            }
            b'F' => {
                let context = self.pop_context_any().ok_or(ParseError)?;
                self.with_children(Kind::PropertyWrappedFieldInitAccessor, &[context])
            }
            b'W' => {
                let context = self.pop_context_any().ok_or(ParseError)?;
                self.with_children(Kind::PropertyWrapperInitFromProjectedValue, &[context])
            }
            b'p' => {
                let name = self.pop_decl_name().ok_or(ParseError)?;
                let context = self.pop_context_any().ok_or(ParseError)?;
                self.with_children(Kind::GenericTypeParamDecl, &[context, name])
            }
            b'm' => {
                let ty = self.pop_type().ok_or(ParseError)?;
                let labels = self.pop_function_param_labels(&ty)?;
                let name = self.pop_decl_name().ok_or(ParseError)?;
                let context = self.pop_context_any().ok_or(ParseError)?;
                self.with_children_opt(Kind::Macro, &[Some(context), Some(name), labels, Some(ty)])
            }
            b'M' => self.macro_expansion(),
            _ => Err(ParseError),
        }
    }

    fn macro_expansion(&mut self) -> PResult<'a> {
        let code = self.next_byte().ok_or(ParseError)?;
        if code == b'X' {
            let line = self.index().ok_or(ParseError)?;
            let column = self.index().ok_or(ParseError)?;
            let line_node = self.alloc_value(Kind::Number, line)?;
            let column_node = self.alloc_value(Kind::Number, column)?;
            let buffer = self.pop_kind(Kind::Identifier).ok_or(ParseError)?;
            let module = self.pop_kind(Kind::Identifier).ok_or(ParseError)?;
            return self.with_children(
                Kind::MacroExpansionLoc,
                &[module, buffer, line_node, column_node],
            );
        }
        let (kind, freestanding) = match code {
            b'f' => (Kind::FreestandingMacroExpansion, true),
            b'u' => (Kind::MacroExpansionUniqueName, false),
            _ => (Kind::AttachedMacroExpansion, false),
        };
        let role = if kind == Kind::AttachedMacroExpansion {
            Some(MacroRole::from_byte(code).ok_or(ParseError)?)
        } else {
            None
        };
        let macro_name = self.pop_kind(Kind::Identifier).ok_or(ParseError)?;
        let private = if freestanding {
            self.pop_kind(Kind::PrivateDeclName)
        } else {
            None
        };
        let attached_name = if role.is_some() {
            self.pop_decl_name()
        } else {
            None
        };
        let context = if self.peek_context().is_some_and(|kind| {
            matches!(
                kind,
                Kind::FreestandingMacroExpansion | Kind::MacroExpansionUniqueName
            )
        }) {
            self.pop_any().ok_or(ParseError)?
        } else {
            self.pop_context_any().ok_or(ParseError)?
        };
        let discriminator = self.index().ok_or(ParseError)?;
        let discriminator = self.alloc_value(Kind::Number, discriminator)?;
        let mut node = match role {
            Some(role) => {
                let builder = self.context.builder_macro_role(kind, role);
                self.check_budget(builder)?
            }
            None => self.builder(kind)?,
        };
        node.push(context);
        if let Some(attached_name) = attached_name {
            node.push(attached_name);
        }
        node.push(macro_name);
        node.push(discriminator);
        node.push_opt(private);
        Ok(node.build())
    }

    fn dependent_conformance_index(&mut self) -> Option<NodeRef<'a>> {
        let index = self.index()?;
        if index == 0 {
            return None;
        }
        if index == 1 {
            return self.alloc(Kind::Number).ok();
        }
        self.alloc_value(Kind::Number, index - 2).ok()
    }

    fn pop_dependent_conformance(&mut self) -> Option<NodeRef<'a>> {
        let id = self.pop_any()?;
        if matches!(
            id.kind(),
            Kind::DependentProtocolConformanceRoot
                | Kind::DependentProtocolConformanceInherited
                | Kind::DependentProtocolConformanceAssociated
                | Kind::DependentProtocolConformanceOpaque
        ) {
            Some(id)
        } else {
            self.stack.push(id);
            None
        }
    }

    fn retroactive_conformance(&mut self) -> PResult<'a> {
        let index = self.index().ok_or(ParseError)?;
        let index = self.alloc_value(Kind::Number, index)?;
        let conformance = self.pop_any().ok_or(ParseError)?;
        self.with_children(Kind::RetroactiveConformance, &[index, conformance])
    }

    fn type_metadata(&mut self) -> PResult<'a> {
        let inner = self.pop_any().ok_or(ParseError)?;
        self.global(Global::TypeMetadata, inner)
    }

    /// `context (decl-name '_')+ 'WZ'` or `'Wz'`: a global one-time
    /// initialization function or token for the listed variables.
    fn global_variable_once(&mut self, code: Global) -> PResult<'a> {
        let mut names = Vec::new();
        while self.pop_kind(Kind::FirstElementMarker).is_some() {
            names.push(self.pop_decl_name().ok_or(ParseError)?);
        }
        if names.is_empty() {
            return Err(ParseError);
        }
        let mut list = self.builder(Kind::GlobalVariableOnceDeclList)?;
        for name in names.into_iter().rev() {
            list.push(name);
        }
        let list = list.build();
        let context = self.pop_context_any().ok_or(ParseError)?;
        let mut node = self.global_builder(code, GlobalOperand::None)?;
        node.push(context);
        node.push(list);
        Ok(node.build())
    }

    fn global(&mut self, code: Global, inner: impl Into<Option<NodeRef<'a>>>) -> PResult<'a> {
        let mut node = self.global_builder(code, GlobalOperand::None)?;
        node.push_opt(inner.into());
        Ok(node.build())
    }

    fn global_index(
        &mut self,
        code: Global,
        inner: impl Into<Option<NodeRef<'a>>>,
        index: u32,
    ) -> PResult<'a> {
        let mut node = self.global_builder(code, GlobalOperand::Index(index))?;
        node.push_opt(inner.into());
        Ok(node.build())
    }

    #[expect(
        clippy::too_many_lines,
        reason = "one match arm per grammar production"
    )]
    fn impl_function_type(&mut self) -> PResult<'a> {
        let pattern = self.next_if(b's');
        let invocation = !pattern && self.next_if(b'I');
        let mut node = self.builder(Kind::ImplFunctionType)?;
        if pattern || invocation {
            let retroactive = self.pop_retroactive_conformances()?;
            let list = self.pop_bound_type_list()?;
            if pattern {
                let signature = self
                    .pop_kind(Kind::DependentGenericSignature)
                    .ok_or(ParseError)?;
                let _pseudo = self.next_if(b'P');
                let mut subs = self.builder(Kind::ImplPatternSubstitutions)?;
                subs.push(signature);
                subs.push(list);
                subs.push_opt(retroactive);
                let subs = subs.build();
                node.push(subs);
            } else {
                let mut subs = self.builder(Kind::ImplInvocationSubstitutions)?;
                subs.push(list);
                subs.push_opt(retroactive);
                let subs = subs.build();
                node.push(subs);
            }
        }
        let signature = self.pop_kind(Kind::DependentGenericSignature);
        if signature.is_some() {
            let _pseudo = self.next_if(b'P');
        }

        if self.next_if(b'e') {
            let child = self.alloc(Kind::ImplEscaping)?;
            node.push(child);
        }
        if self.next_if(b'A') {
            let child = self.alloc(Kind::ImplErasedIsolation)?;
            node.push(child);
        }
        if self.next_if(b'N') {
            let child = self.alloc(Kind::ImplNonisolatedNonsending)?;
            node.push(child);
        }
        if self.next_if(b'O') {
            let callable = self.next_if(b'o');
            node.push(self.impl_child(Kind::ImplCalledOnce, u8::from(callable))?);
        }
        if matches!(self.peek(), Some(b'd' | b'l' | b'f' | b'r')) {
            let code = self.next_byte().ok_or(ParseError)?;
            node.push(self.impl_child(Kind::ImplDifferentiabilityKind, code)?);
        }
        let callee = self.next_byte().ok_or(ParseError)?;
        if !matches!(callee, b'y' | b'g' | b'x' | b't') {
            return Err(ParseError);
        }
        node.push(self.impl_child(Kind::ImplCalleeConvention, callee)?);

        // Optional function representation.
        let representation_checkpoint = self.input.checkpoint();
        let representation = self.peek().ok_or(ParseError)?;
        let mut with_clang_type = false;
        let representation = match representation {
            b'B' | b'C' | b'M' | b'O' | b'J' | b'K' | b'V' | b'W' => {
                self.advance(1)?;
                Some(representation)
            }
            b'z' => {
                self.advance(1)?;
                let inner = self.next_byte().ok_or(ParseError)?;
                match inner {
                    b'B' | b'C' => {
                        with_clang_type = true;
                        Some(inner)
                    }
                    _ => {
                        self.input.reset(representation_checkpoint);
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
            node.push(self.impl_child(Kind::ImplFunctionConvention, code)?);
        }

        if matches!(self.peek(), Some(b'A' | b'I' | b'G')) {
            let code = self.next_byte().ok_or(ParseError)?;
            node.push(self.impl_child(Kind::ImplCoroutineKind, code)?);
        }
        if self.next_if(b'h') {
            let child = self.alloc(Kind::ImplSendable)?;
            node.push(child);
        }
        if self.next_if(b'H') {
            let child = self.alloc(Kind::ImplAsync)?;
            node.push(child);
        }
        if self.next_if(b'T') {
            let child = self.alloc(Kind::ImplSendingResult)?;
            node.push(child);
        }
        if let Some(signature) = signature {
            node.push(signature);
        }

        let mut conventions = Vec::new();
        while let Some(code) = self.peek_param_convention() {
            conventions.push(self.impl_entry(Kind::ImplParameter, code)?);
        }
        while let Some(code) = self.peek_result_convention() {
            conventions.push(self.impl_entry(Kind::ImplResult, code)?);
        }
        while self.next_if(b'Y') {
            let code = self.peek().ok_or(ParseError)?;
            conventions.push(self.impl_entry(Kind::ImplYield, code)?);
        }
        if self.next_if(b'z') {
            let code = self.peek().ok_or(ParseError)?;
            conventions.push(self.impl_entry(Kind::ImplErrorResult, code)?);
        }
        if !self.next_if(b'_') {
            return Err(ParseError);
        }
        // Attach the mangled types in reverse order.
        for entry in conventions.iter_mut().rev() {
            let ty = self.pop_type().ok_or(ParseError)?;
            entry.push(ty);
        }
        for entry in conventions {
            node.push(entry.build());
        }
        self.wrap_type(node.build())
    }

    /// A one-byte `Impl*` child.
    fn impl_child(&mut self, kind: Kind, code: u8) -> PResult<'a> {
        let builder = self.context.builder_impl(kind, code);
        Ok(self.check_budget(builder)?.build())
    }

    fn pop_retroactive_conformances(&mut self) -> PResult<'a, Option<NodeRef<'a>>> {
        let mut conformances = Vec::new();
        while let Some(id) = self.stack.last().cloned() {
            if id.kind() != Kind::RetroactiveConformance {
                break;
            }
            let _ = self.stack.pop();
            conformances.push(id);
        }
        if conformances.is_empty() {
            return Ok(None);
        }
        let mut list = self.builder(Kind::TypeList)?;
        for conformance in conformances.into_iter().rev() {
            list.push(conformance);
        }
        Ok(Some(list.build()))
    }

    fn pop_bound_type_list(&mut self) -> PResult<'a, NodeRef<'a>> {
        let mut list = self.builder(Kind::TypeList)?;
        let mut types = Vec::new();
        loop {
            while let Some(ty) = self.pop_type() {
                types.push(ty);
            }
            if self.pop_kind(Kind::EmptyList).is_some() {
                break;
            }
            if self.pop_kind(Kind::FirstElementMarker).is_none() {
                return Err(ParseError);
            }
        }
        for ty in types.into_iter().rev() {
            list.push(ty);
        }
        Ok(list.build())
    }

    fn peek_param_convention(&self) -> Option<u8> {
        let byte = self.peek()?;
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
        let byte = self.peek()?;
        matches!(
            byte,
            b'r' | b'o' | b'd' | b'u' | b'a' | b'k' | b'l' | b'g' | b'm'
        )
        .then_some(byte)
    }

    fn impl_entry(&mut self, kind: Kind, convention: u8) -> PResult<'a, Builder<'a>> {
        self.advance(1)?;
        let mut entry = self.builder(kind)?;
        entry.push(self.impl_child(Kind::ImplConvention, convention)?);
        // Optional differentiability, sending, isolation and implicit-leading
        // markers, in that order.
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
                entry.push(node);
            }
        }
        Ok(entry)
    }

    fn runtime_record(&mut self) -> PResult<'a> {
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
            b'F' => self.global(Global::AccessibleFunctionRecord, None),
            b'P' => {
                let protocol = self.pop_protocol().ok_or(ParseError)?;
                self.node_with(Kind::ProtocolConformanceRefInTypeModule, &[protocol])
            }
            b'p' => {
                let protocol = self.pop_protocol().ok_or(ParseError)?;
                self.node_with(Kind::ProtocolConformanceRefInProtocolModule, &[protocol])
            }
            b'C' => {
                let node = self.pop_concrete_protocol_conformance().ok_or(ParseError)?;
                Ok(node)
            }
            b'X' => {
                let list = self.pop_any_protocol_conformance_list().ok_or(ParseError)?;
                self.node_with(Kind::PackProtocolConformance, &[list])
            }
            b'D' => {
                let index = self.dependent_conformance_index().ok_or(ParseError)?;
                let protocol = self.pop_protocol().ok_or(ParseError)?;
                let ty = self.pop_type().ok_or(ParseError)?;
                self.node_with(
                    Kind::DependentProtocolConformanceRoot,
                    &[ty, protocol, index],
                )
            }
            b'I' => {
                let index = self.dependent_conformance_index().ok_or(ParseError)?;
                let protocol = self.pop_protocol().ok_or(ParseError)?;
                let nested = self.pop_dependent_conformance().ok_or(ParseError)?;
                self.node_with(
                    Kind::DependentProtocolConformanceInherited,
                    &[nested, protocol, index],
                )
            }
            b'A' => {
                let index = self.dependent_conformance_index().ok_or(ParseError)?;
                let protocol = self.pop_protocol().ok_or(ParseError)?;
                let ty = self.pop_type().ok_or(ParseError)?;
                let assoc =
                    self.node_with(Kind::DependentAssociatedConformance, &[ty, protocol])?;
                let nested = self.pop_dependent_conformance().ok_or(ParseError)?;
                self.node_with(
                    Kind::DependentProtocolConformanceAssociated,
                    &[nested, assoc, index],
                )
            }
            b'O' => {
                let ty = self.pop_type().ok_or(ParseError)?;
                let conformance = self.pop_dependent_conformance().ok_or(ParseError)?;
                self.node_with(Kind::DependentProtocolConformanceOpaque, &[conformance, ty])
            }
            _ => Err(ParseError),
        }
    }

    #[expect(
        clippy::too_many_lines,
        reason = "one match arm per grammar production"
    )]
    fn metatype_global(&mut self) -> PResult<'a> {
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
            b'V' => {
                let inner = self.pop_any().ok_or(ParseError)?;
                self.global(Global::PropertyDescriptor, inner)
            }
            b'Q' => {
                let inner = self.pop_any().ok_or(ParseError)?;
                self.global(Global::OpaqueTypeDescriptor, inner)
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

    #[expect(
        clippy::too_many_lines,
        reason = "one match arm per grammar production"
    )]
    fn witness(&mut self) -> PResult<'a> {
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
                let protocol_ty = self.pop_type().ok_or(ParseError)?;
                let conforming = self.pop_assoc_type_path().ok_or(ParseError)?;
                let conformance = self.pop_protocol_conformance().ok_or(ParseError)?;
                let mut node = self.global_builder(
                    Global::AssociatedTypeWitnessTableAccessor,
                    GlobalOperand::None,
                )?;
                node.push(conformance);
                node.push(conforming);
                node.push(protocol_ty);
                Ok(node.build())
            }
            b'b' => {
                let protocol_ty = self.pop_type().ok_or(ParseError)?;
                let conformance = self.pop_protocol_conformance().ok_or(ParseError)?;
                let mut node = self.global_builder(
                    Global::BaseProtocolWitnessTableAccessor,
                    GlobalOperand::None,
                )?;
                node.push(conformance);
                node.push(protocol_ty);
                Ok(node.build())
            }
            b'S' => {
                let inner = self.pop_protocol().ok_or(ParseError)?;
                self.global(Global::ProtocolSelfConformanceWitnessTable, inner)
            }
            b'V' => {
                let inner = self.pop_any().ok_or(ParseError)?;
                self.global(Global::ValueWitnessTable, inner)
            }
            b'v' => {
                let directness = match self.next_byte().ok_or(ParseError)? {
                    b'd' => false,
                    b'i' => true,
                    _ => return Err(ParseError),
                };
                let inner = self.pop_any().ok_or(ParseError)?;
                let mut node = self
                    .global_builder(Global::FieldOffset, GlobalOperand::Directness(directness))?;
                node.push(inner);
                Ok(node.build())
            }
            b'C' => {
                let inner = self.pop_any().ok_or(ParseError)?;
                self.with_children(Kind::EnumCase, &[inner])
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
            b'Z' => self.global_variable_once(Global::GlobalVariableOnceFunction),
            b'z' => self.global_variable_once(Global::GlobalVariableOnceToken),
            b'J' => self.differentiability_witness(),
            _ => Err(ParseError),
        }
    }

    #[expect(
        clippy::too_many_lines,
        reason = "one match arm per grammar production"
    )]
    fn thunk_or_specialization(&mut self) -> PResult<'a> {
        let code = self.next_byte().ok_or(ParseError)?;
        match code {
            b'o' => self.attribute(AttributeKind::Objc),
            b'O' => self.attribute(AttributeKind::NonObjc),
            b'D' => self.attribute(AttributeKind::Dynamic),
            b'd' => self.attribute(AttributeKind::Super),
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
            b'A' => self.global(Global::PartialApplyForwarder, None),
            b'a' => self.global(Global::PartialApplyObjcForwarder, None),
            b'R' | b'r' => {
                let code = if code == b'R' {
                    Global::ReabstractionThunkHelper
                } else {
                    Global::ReabstractionThunk
                };
                let signature = self.pop_kind(Kind::DependentGenericSignature);
                let first = self.pop_type().ok_or(ParseError)?;
                let second = self.pop_type().ok_or(ParseError)?;
                let mut node = self.global_builder(code, GlobalOperand::None)?;
                node.push_opt(signature);
                node.push(first);
                node.push(second);
                Ok(node.build())
            }
            b'j' => {
                let inner = self.pop_any().ok_or(ParseError)?;
                self.global(Global::DispatchThunk, inner)
            }
            b'q' => {
                let inner = self.pop_any().ok_or(ParseError)?;
                self.global(Global::MethodDescriptor, inner)
            }
            b'u' => self.global(Global::AsyncFunctionPointer, None),
            b'X' => self.global(Global::DynamicallyReplaceableFunctionVar, None),
            b'x' => self.global(Global::DynamicallyReplaceableFunctionKey, None),
            b'I' => self.global(Global::DynamicallyReplaceableFunctionImpl, None),
            b'm' => self.global(Global::MergedFunction, None),
            b'E' => self.global(Global::DistributedThunk, None),
            b'F' => self.global(Global::DistributedAccessor, None),
            b'W' => {
                let entity = self.pop_any().ok_or(ParseError)?;
                let conformance = self.pop_protocol_conformance().ok_or(ParseError)?;
                let mut node =
                    self.global_builder(Global::ProtocolWitnessThunk, GlobalOperand::None)?;
                node.push(conformance);
                node.push(entity);
                Ok(node.build())
            }
            b'S' => {
                let inner = self.pop_any().ok_or(ParseError)?;
                self.global(Global::ProtocolSelfConformanceWitnessThunk, inner)
            }
            b'w' => {
                let sub = self.next_byte().ok_or(ParseError)?;
                let global = match sub {
                    b'a' => Global::BackDeploymentCopy,
                    b'b' => Global::BackDeploymentThunk,
                    b'B' => Global::BackDeploymentFallback,
                    b'S' => Global::HasSymbolQuery,
                    b'c' => Global::CoroFunctionPointer,
                    b'd' => Global::DefaultOverride,
                    _ => return Err(ParseError),
                };
                self.global(global, None)
            }
            b'Y' | b'Q' => {
                let index = self.index().ok_or(ParseError)?;
                let style = if code == b'Y' {
                    ResumeStyle::Suspend
                } else {
                    ResumeStyle::Await
                };
                let operand = GlobalOperand::IndexAndResume { index, style };
                let node = self.global_builder(Global::ResumePartialFunction, operand)?;
                Ok(node.build())
            }
            b'v' => {
                let index = self.index().ok_or(ParseError)?;
                let read_only = self.next_if(b'r');
                let global = if read_only {
                    Global::OutlinedReadOnlyObject
                } else {
                    Global::OutlinedVariable
                };
                self.global_index(global, None, index)
            }
            b'e' => {
                let mut params = [0_u8; 8];
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
                // `o`/`p`/`a`/`m`/`n`/`b`/`g` bytes only: the conversion
                // cannot fail.
                let text = core::str::from_utf8(&params[..len]).map_err(|_| ParseError)?;
                let mut node =
                    self.global_builder(Global::OutlinedBridgedMethod, GlobalOperand::None)?;
                node.set_text(Some(Cow::Owned(text.into())));
                Ok(node.build())
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
                let mut node = self.global_builder(code, GlobalOperand::None)?;
                node.push(proto_ty);
                node.push(subject);
                node.push(requirement);
                Ok(node.build())
            }
            b'b' => {
                let requirement = self.pop_protocol().ok_or(ParseError)?;
                let proto_ty = self.pop_type().ok_or(ParseError)?;
                let mut node =
                    self.global_builder(Global::BaseConformanceDescriptor, GlobalOperand::None)?;
                node.push(proto_ty);
                node.push(requirement);
                Ok(node.build())
            }
            b'J' => self.auto_diff_thunk(),
            b'K' | b'k' => self.keypath_thunk(code),
            _ => Err(ParseError),
        }
    }

    /// Move every operand left on the stack into `builder`, preserving order.
    fn drain_stack_into(&mut self, builder: &mut Builder<'a>) {
        for id in core::mem::take(&mut self.stack) {
            builder.push(id);
        }
    }

    /// `AutoDiffFunctionKind` (`f`, `r`, `d`, `p`).
    fn auto_diff_kind(&mut self) -> PResult<'a> {
        let code =
            AutoDiffCode::from_byte(self.next_byte().ok_or(ParseError)?).ok_or(ParseError)?;
        if !matches!(
            code,
            AutoDiffCode::Forward
                | AutoDiffCode::Reverse
                | AutoDiffCode::Differential
                | AutoDiffCode::Pullback
        ) {
            return Err(ParseError);
        }
        let builder = self
            .context
            .builder_autodiff(Kind::AutoDiffFunctionKind, code);
        Ok(self.check_budget(builder)?.build())
    }

    fn index_subset(&mut self) -> PResult<'a, NodeRef<'a>> {
        let mut text = [0_u8; 64];
        let mut len = 0_usize;
        while let Some(byte) = self.peek() {
            if byte != b'S' && byte != b'U' {
                break;
            }
            if len >= text.len() {
                return Err(ParseError);
            }
            text[len] = byte;
            len += 1;
            self.advance(1)?;
        }
        if len == 0 {
            return Err(ParseError);
        }
        // `S`/`U` bytes only: the conversion cannot fail.
        let text = core::str::from_utf8(&text[..len]).map_err(|_| ParseError)?;
        self.alloc_owned(Kind::IndexSubset, text)
    }

    fn keypath_thunk(&mut self, code: u8) -> PResult<'a> {
        let mut flavor = 0_u8;
        if self.next_if(b'm') {
            if self.next_if(b'u') {
                flavor = 1;
            } else if self.next_if(b'M') && self.next_if(b'A') {
                flavor = 2;
            } else {
                return Err(ParseError);
            }
        }
        let serialized = self.next_if(b'q');
        let mut types = Vec::new();
        while let Some(ty) = self.pop_type() {
            types.push(ty);
        }
        let mut first = self.pop_any().ok_or(ParseError)?;
        let signature = if first.kind() == Kind::DependentGenericSignature {
            let decl = self.pop_any().ok_or(ParseError)?;
            let signature = first;
            first = decl;
            Some(signature)
        } else {
            None
        };
        // The reference printer prefers the method flavours over the setter
        // bit, so a `k`-prefixed `mu`/`mMA` thunk prints as a method.
        let kind = match flavor {
            1 => KeyPathKind::UnappliedMethod,
            2 => KeyPathKind::AppliedMethod,
            _ if code == b'k' => KeyPathKind::Setter,
            _ => KeyPathKind::Getter,
        };
        let builder = self.context.builder_keypath(kind);
        let mut node = self.check_budget(builder)?;
        node.push(first);
        node.push_opt(signature);
        for ty in types.into_iter().rev() {
            node.push(ty);
        }
        if serialized {
            let marker = self.alloc(Kind::IsSerialized)?;
            node.push(marker);
        }
        Ok(node.build())
    }

    fn auto_diff_thunk(&mut self) -> PResult<'a> {
        match self.peek() {
            Some(b'S') => {
                self.advance(1)?;
                let mut node = self.builder(Kind::AutoDiffSubsetParametersThunk)?;
                if self.stack.is_empty() {
                    // The printer requires at least one operand before the
                    // function kind; an empty prefix is not printable.
                    return Err(ParseError);
                }
                self.drain_stack_into(&mut node);
                let kind = self.auto_diff_kind()?;
                node.push(kind);
                let params = self.index_subset()?;
                node.push(params);
                if !self.next_if(b'p') {
                    return Err(ParseError);
                }
                let results = self.index_subset()?;
                node.push(results);
                if !self.next_if(b'r') {
                    return Err(ParseError);
                }
                let to_params = self.index_subset()?;
                node.push(to_params);
                if !self.next_if(b'P') {
                    return Err(ParseError);
                }
                Ok(node.build())
            }
            Some(b'O') => {
                self.advance(1)?;
                let mut node = self.builder(Kind::AutoDiffSelfReorderingReabstractionThunk)?;
                let signature = self.pop_kind(Kind::DependentGenericSignature);
                let first = self.pop_type().ok_or(ParseError)?;
                let second = self.pop_type().ok_or(ParseError)?;
                node.push(second);
                node.push(first);
                node.push_opt(signature);
                let kind = self.auto_diff_kind()?;
                node.push(kind);
                Ok(node.build())
            }
            _ => {
                let vtable = self.next_if(b'V');
                let kind = if vtable {
                    Kind::AutoDiffDerivativeVTableThunk
                } else {
                    Kind::AutoDiffFunction
                };
                let mut node = self.builder(kind)?;
                self.drain_stack_into(&mut node);
                let code = self.auto_diff_kind()?;
                node.push(code);
                let params = self.index_subset()?;
                node.push(params);
                if !self.next_if(b'p') {
                    return Err(ParseError);
                }
                let results = self.index_subset()?;
                node.push(results);
                if !self.next_if(b'r') {
                    return Err(ParseError);
                }
                Ok(node.build())
            }
        }
    }

    fn differentiability_witness(&mut self) -> PResult<'a> {
        let mut node = self.builder(Kind::DifferentiabilityWitness)?;
        let signature = self.pop_kind(Kind::DependentGenericSignature);
        self.drain_stack_into(&mut node);
        let kind = self.next_byte().ok_or(ParseError)?;
        // The printer reads the raw `DIFFERENTIABILITY-KIND` byte back.
        let raw_kind = match kind {
            b'f' | b'r' | b'd' | b'l' => kind,
            _ => return Err(ParseError),
        };
        let kind_node = self.alloc_value(Kind::Number, u32::from(raw_kind))?;
        node.push(kind_node);
        let params = self.index_subset()?;
        node.push(params);
        if !self.next_if(b'p') {
            return Err(ParseError);
        }
        let results = self.index_subset()?;
        node.push(results);
        if !self.next_if(b'r') {
            return Err(ParseError);
        }
        node.push_opt(signature);
        Ok(node.build())
    }

    fn function_specialization(&mut self) -> PResult<'a> {
        let _serialized = self.next_if(b'q');
        let _async_removed = self.next_if(b'a');
        let representation_changed = self.next_if(b'r');
        let pass = self.next_byte().ok_or(ParseError)?;
        if !pass.is_ascii_digit() {
            return Err(ParseError);
        }
        let builder = self
            .context
            .builder_function_signature_specialization(representation_changed);
        let mut spec = self.check_budget(builder)?;
        if representation_changed {
            return Ok(spec.build());
        }
        let mut params = Vec::new();
        while self.peek().is_some_and(|b| b != b'_') {
            params.push(self.func_spec_param(false)?);
        }
        if !self.next_if(b'_') {
            return Err(ParseError);
        }
        if !self.next_if(b'n') {
            params.push(self.func_spec_param(true)?);
        }
        // Attach payloads from the operand stack in reverse parameter order,
        // then store the parameters in source order.
        let mut attached = Vec::with_capacity(params.len());
        for param in params.into_iter().rev() {
            attached.push(self.func_spec_attach(param)?);
        }
        for param in attached.into_iter().rev() {
            spec.push(param);
        }
        Ok(spec.build())
    }

    fn func_spec_param(&mut self, is_return: bool) -> PResult<'a, ParamBuilder<'a>> {
        let mut param = ParamBuilder {
            is_return,
            children: Vec::new(),
        };
        let code = self.next_byte().ok_or(ParseError)?;
        match code {
            b'n' => {}
            b'c' | b'E' => self.func_spec_kind(&mut param, if code == b'c' { 5 } else { 12 })?,
            b'a' => self.func_spec_kind(&mut param, 13)?,
            b'C' => {
                let index = self.natural().ok_or(ParseError)?;
                self.func_spec_kind(&mut param, 11)?;
                let payload = self.alloc_value(Kind::FuncSpecNumber, index)?;
                param.children.push(payload);
            }
            b'p' => self.func_spec_propagation(&mut param)?,
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
                self.func_spec_kind(&mut param, value)?;
            }
            b'x' => self.func_spec_kind(&mut param, 1 << 8)?,
            b'i' => self.func_spec_kind(&mut param, 6)?,
            b's' => self.func_spec_kind(&mut param, 7)?,
            b'r' => self.func_spec_kind(&mut param, 8)?,
            _ => return Err(ParseError),
        }
        Ok(param)
    }

    fn func_spec_kind(&mut self, param: &mut ParamBuilder<'a>, code: u32) -> PResult<'a, ()> {
        let node = self.alloc_value(Kind::FuncSpecKind, code)?;
        param.children.push(node);
        Ok(())
    }

    fn func_spec_propagation(&mut self, param: &mut ParamBuilder<'a>) -> PResult<'a, ()> {
        loop {
            let checkpoint = self.input.checkpoint();
            let Some(byte) = self.next_byte() else { break };
            match byte {
                b'S' => self.func_spec_kind(param, 10)?,
                b'f' => self.func_spec_kind(param, 0)?,
                b'g' => self.func_spec_kind(param, 1)?,
                b'i' | b'd' => {
                    self.func_spec_kind(param, if byte == b'i' { 2 } else { 3 })?;
                    let start = self.input.pos();
                    while self.peek().is_some_and(|b| b.is_ascii_digit()) {
                        self.advance(1)?;
                    }
                    let end = self.input.pos();
                    if end == start {
                        return Err(ParseError);
                    }
                    // Decimal digits only: the conversion cannot fail.
                    let text = core::str::from_utf8(
                        self.input.bytes_between(start, end).ok_or(ParseError)?,
                    )
                    .map_err(|_| ParseError)?;
                    let node = self.alloc_owned(Kind::Identifier, text)?;
                    param.children.push(node);
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
                    let payload = self.alloc_owned(Kind::Identifier, text)?;
                    param.children.push(payload);
                }
                b'k' => self.func_spec_kind(param, 9)?,
                _ => {
                    self.input.reset(checkpoint);
                    break;
                }
            }
        }
        Ok(())
    }

    /// Pop the payloads a parameter needs from the operand stack and insert
    /// them after their kind tags, in reference print order.
    fn func_spec_attach(&mut self, param: ParamBuilder<'a>) -> PResult<'a> {
        let mut kinds = Vec::new();
        for child in &param.children {
            if child.kind() == Kind::FuncSpecKind {
                kinds.push(child.clone());
            }
        }
        let mut payloads: Vec<Vec<NodeRef<'a>>> = Vec::new();
        for kind in kinds.iter().rev() {
            let raw = kind.number_value();
            let mut buffer = Vec::new();
            match raw {
                5 | 12 => {
                    let mut types = Vec::new();
                    while let Some(ty) = self.pop_type() {
                        types.push(ty);
                    }
                    let ident = self.pop_kind(Kind::Identifier).ok_or(ParseError)?;
                    buffer.push(ident);
                    for ty in types.into_iter().rev() {
                        buffer.push(ty);
                    }
                }
                9 => {
                    let first = self.pop_type().ok_or(ParseError)?;
                    let second = self.pop_type().ok_or(ParseError)?;
                    let ident = self.pop_kind(Kind::Identifier).ok_or(ParseError)?;
                    buffer.push(ident);
                    buffer.push(second);
                    buffer.push(first);
                }
                10 => {
                    let ty = self.pop_type().ok_or(ParseError)?;
                    buffer.push(ty);
                }
                0 | 1 | 4 => {
                    let ident = self.pop_kind(Kind::Identifier).ok_or(ParseError)?;
                    buffer.push(ident);
                }
                13 => {
                    let mut pairs = Vec::new();
                    for _ in 0..3 {
                        pairs.push(self.pop_type().ok_or(ParseError)?);
                        pairs.push(self.pop_kind(Kind::Identifier).ok_or(ParseError)?);
                    }
                    for pair in pairs.into_iter().rev() {
                        buffer.push(pair);
                    }
                }
                _ => {}
            }
            payloads.push(buffer);
        }
        payloads.reverse();
        let builder = self.context.builder_function_spec(param.is_return);
        let mut node = self.check_budget(builder)?;
        let mut kind_index = 0_usize;
        let mut pending: Option<usize> = None;
        for child in param.children {
            if child.kind() == Kind::FuncSpecKind {
                if let Some(previous) = pending
                    && let Some(attached) = payloads.get(previous)
                {
                    for payload in attached {
                        node.push(payload.clone());
                    }
                }
                pending = Some(kind_index);
                kind_index += 1;
            }
            node.push(child);
        }
        if let Some(previous) = pending
            && let Some(attached) = payloads.get(previous)
        {
            for payload in attached {
                node.push(payload.clone());
            }
        }
        Ok(node.build())
    }

    fn attribute(&mut self, kind: AttributeKind) -> PResult<'a> {
        let builder = self.context.builder_attribute(kind);
        Ok(self.check_budget(builder)?.build())
    }

    fn generic_specialization(&mut self, code: u8) -> PResult<'a> {
        let kind = specialization_kind(code).ok_or(ParseError)?;
        let builder = self.context.builder_generic_specialization(kind, false);
        let mut node = self.check_budget(builder)?;
        self.generic_specialization_body(&mut node, kind)?;
        Ok(node.build())
    }

    fn generic_specialization_with_dropped_args(&mut self) -> PResult<'a> {
        let mut dropped = Vec::new();
        while self.next_if(b't') {
            let value = self.natural().map_or(0, |v| v.saturating_add(1));
            dropped.push(self.alloc_value(Kind::DroppedArgument, value)?);
        }
        let code = self.next_byte().ok_or(ParseError)?;
        let kind = specialization_kind(code).ok_or(ParseError)?;
        let builder = self.context.builder_generic_specialization(kind, false);
        let mut node = self.check_budget(builder)?;
        for dropped in dropped {
            node.push(dropped);
        }
        self.generic_specialization_body(&mut node, kind)?;
        Ok(node.build())
    }

    /// Parse the spec attributes and generic arguments of a `T`-specialization.
    fn generic_specialization_body(
        &mut self,
        node: &mut Builder<'a>,
        kind: GenericSpecializationKind,
    ) -> PResult<'a, ()> {
        let _serialized = self.next_if(b'q');
        let _async_removed = self.next_if(b'a');
        if self.next_if(b'r') {
            node.set_data(NodeData::GenericSpecialization {
                kind,
                representation_changed: true,
            });
        }
        let pass = self.next_byte().ok_or(ParseError)?;
        if !pass.is_ascii_digit() {
            return Err(ParseError);
        }
        if self.pop_kind(Kind::EmptyList).is_none() {
            let mut types = Vec::new();
            loop {
                let first = self.pop_kind(Kind::FirstElementMarker).is_some();
                let ty = self.pop_type().ok_or(ParseError)?;
                types.push(ty);
                if first {
                    break;
                }
            }
            for ty in types.into_iter().rev() {
                let param = self.with_children(Kind::GenericSpecializationParam, &[ty])?;
                node.push(param);
            }
        }
        Ok(())
    }

    fn value_witness(&mut self) -> PResult<'a> {
        let inner = self.pop_any().ok_or(ParseError)?;
        let _code = self.next_byte().ok_or(ParseError)?;
        self.global(Global::ValueWitnessTable, inner)
    }

    fn special_type(&mut self) -> PResult<'a> {
        let code = self.next_byte().ok_or(ParseError)?;
        match code {
            b'l' => {
                let list = self.protocol_list()?;
                let node = self.node_with(Kind::ProtocolListWithAnyObject, &[list])?;
                self.wrap_type(node)
            }
            b'Z' => {
                let types = self.pop_type_list()?;
                let name = self.pop_kind(Kind::Identifier).ok_or(ParseError)?;
                let parent = self.pop_context_any().ok_or(ParseError)?;
                self.with_children(Kind::AnonymousContext, &[name, parent, types])
            }
            b'P' => {
                let mut list = self.builder(Kind::ConstrainedExistentialRequirementList)?;
                let mut requirements = Vec::new();
                loop {
                    let first = self.pop_kind(Kind::FirstElementMarker).is_some();
                    let id = self.pop_any().ok_or(ParseError)?;
                    if !matches!(
                        id.kind(),
                        Kind::DependentGenericConformanceRequirement
                            | Kind::DependentGenericSameTypeRequirement
                            | Kind::DependentGenericBaseClassRequirement
                            | Kind::DependentGenericLayoutRequirement
                            | Kind::DependentGenericInverseConformanceRequirement
                    ) {
                        return Err(ParseError);
                    }
                    requirements.push(id);
                    if first {
                        break;
                    }
                }
                for requirement in requirements.into_iter().rev() {
                    list.push(requirement);
                }
                let base = self.pop_type().ok_or(ParseError)?;
                let node =
                    self.with_children(Kind::ConstrainedExistential, &[base, list.build()])?;
                self.wrap_type(node)
            }
            b'x' | b'X' => self.sil_box_type(code == b'X'),
            b'o' => self.type_wrapper(Kind::Unowned),
            b'u' => self.type_wrapper(Kind::Unmanaged),
            b'w' => self.type_wrapper(Kind::Weak),
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
            // `F`, `b`, `e`, `j` and every other byte are not a special type.
            _ => Err(ParseError),
        }
    }

    fn sil_box_type(&mut self, with_signature: bool) -> PResult<'a> {
        let signature = if with_signature {
            Some(
                self.pop_kind(Kind::DependentGenericSignature)
                    .ok_or(ParseError)?,
            )
        } else {
            None
        };
        let generic_args = if with_signature {
            Some(self.pop_type_list()?)
        } else {
            None
        };
        let fields = self.pop_type_list()?;
        let mut layout = self.builder(Kind::SILBoxLayout)?;
        for field in fields.children().iter().cloned() {
            let mut ty = field;
            let mut mutable = false;
            if ty.kind() == Kind::Type
                && let Some(inner) = ty.child(0)
                && inner.kind() == Kind::InOut
            {
                mutable = true;
                if let Some(value) = inner.child(0) {
                    ty = self.wrap_type(value)?;
                }
            }
            let mut entry = self.builder(if mutable {
                Kind::SILBoxMutableField
            } else {
                Kind::SILBoxImmutableField
            })?;
            entry.push(ty);
            layout.push(entry.build());
        }
        let mut node = self.builder(Kind::SILBoxTypeWithLayout)?;
        node.push(layout.build());
        node.push_opt(signature);
        node.push_opt(generic_args);
        self.wrap_type(node.build())
    }

    fn pop_type_list(&mut self) -> PResult<'a, NodeRef<'a>> {
        let mut list = self.builder(Kind::TypeList)?;
        if self.pop_kind(Kind::EmptyList).is_some() {
            return Ok(list.build());
        }
        let types = self.pop_reverse_list(Self::pop_type).ok_or(ParseError)?;
        for ty in types.into_iter().rev() {
            list.push(ty);
        }
        Ok(list.build())
    }

    fn archetype(&mut self) -> PResult<'a> {
        let code = self.next_byte().ok_or(ParseError)?;
        match code {
            b'a' => {
                let name = self.pop_kind(Kind::Identifier).ok_or(ParseError)?;
                let base = self.pop_type_child().ok_or(ParseError)?;
                let node = self.typed(Kind::DependentMemberType, &[base, name])?;
                self.context.add_sub(node.clone());
                Ok(node)
            }
            b'y' => {
                let param = self.generic_param()?;
                let ty = self.assoc_type_simple(Some(param))?;
                self.context.add_sub(ty.clone());
                Ok(ty)
            }
            b'Y' => {
                let param = self.generic_param()?;
                let ty = self.assoc_type_compound(Some(param))?;
                self.context.add_sub(ty.clone());
                Ok(ty)
            }
            b'z' => {
                let param = self.builtin_generic_param(0, 0)?;
                let ty = self.assoc_type_simple(Some(param))?;
                self.context.add_sub(ty.clone());
                Ok(ty)
            }
            b'Z' => {
                let param = self.builtin_generic_param(0, 0)?;
                let ty = self.assoc_type_compound(Some(param))?;
                self.context.add_sub(ty.clone());
                Ok(ty)
            }
            b'P' => {
                let mut pack = self.builder(Kind::Pack)?;
                if self.pop_kind(Kind::EmptyList).is_none() {
                    let elements = self.pop_reverse_list(Self::pop_type).ok_or(ParseError)?;
                    for element in elements.into_iter().rev() {
                        pack.push(element);
                    }
                }
                self.wrap_type(pack.build())
            }
            b'x' => {
                let ty = self.assoc_type_simple(None)?;
                self.context.add_sub(ty.clone());
                Ok(ty)
            }
            b'r' => {
                let node = self.alloc(Kind::OpaqueReturnType)?;
                self.wrap_type(node)
            }
            b'R' => {
                let ordinal = self.index().ok_or(ParseError)?;
                let index = self.alloc_value(Kind::OpaqueReturnTypeIndex, ordinal)?;
                let node = self.with_children(Kind::OpaqueReturnType, &[index])?;
                self.wrap_type(node)
            }
            b'O' => {
                let context = self.pop_context_any().ok_or(ParseError)?;
                self.with_children(Kind::OpaqueReturnTypeOf, &[context])
            }
            b'o' => {
                let index = self.index().ok_or(ParseError)?;
                // Consume bound generic lists and retroactive conformances.
                loop {
                    while self.pop_type().is_some() {}
                    if self.pop_kind(Kind::EmptyList).is_some() {
                        break;
                    }
                    if self.pop_kind(Kind::FirstElementMarker).is_none() {
                        return Err(ParseError);
                    }
                }
                while let Some(id) = self.stack.last().cloned() {
                    if id.kind() == Kind::RetroactiveConformance {
                        let _ = self.stack.pop();
                    } else {
                        break;
                    }
                }
                let name = self.pop_any().ok_or(ParseError)?;
                let index = self.alloc_value(Kind::Number, index)?;
                let node = self.with_children(Kind::OpaqueType, &[name, index])?;
                let ty = self.wrap_type(node)?;
                self.context.add_sub(ty.clone());
                Ok(ty)
            }
            _ => Err(ParseError),
        }
    }

    fn assoc_type_simple(&mut self, base: Option<NodeRef<'a>>) -> PResult<'a> {
        let name = self.pop_assoc_type_name().ok_or(ParseError)?;
        let base = if let Some(base) = base {
            base
        } else {
            self.typed(Kind::DependentGenericParamType, &[])?
        };
        let node = self.typed(Kind::DependentMemberType, &[base, name])?;
        Ok(node)
    }

    fn assoc_type_compound(&mut self, base: Option<NodeRef<'a>>) -> PResult<'a> {
        let names = self
            .pop_reverse_list(Self::pop_assoc_type_name)
            .ok_or(ParseError)?;
        let mut ty = if let Some(base) = base {
            base
        } else {
            self.pop_type().ok_or(ParseError)?
        };
        for name in names.into_iter().rev() {
            ty = self.typed(Kind::DependentMemberType, &[ty, name])?;
        }
        Ok(ty)
    }

    fn type_annotation(&mut self) -> PResult<'a> {
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
                let code = AutoDiffCode::from_byte(self.next_byte().ok_or(ParseError)?)
                    .ok_or(ParseError)?;
                // A function type only carries the `f`/`r`/`d`/`l` forms.
                if !matches!(
                    code,
                    AutoDiffCode::Forward
                        | AutoDiffCode::Reverse
                        | AutoDiffCode::Differential
                        | AutoDiffCode::Linear
                ) {
                    return Err(ParseError);
                }
                let builder = self
                    .context
                    .builder_autodiff(Kind::DifferentiableFunctionType, code);
                self.check_budget(builder)?.build()
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

    fn operator_identifier(&mut self) -> PResult<'a> {
        const TABLE: &[u8; 26] = b"& @/= >    <*!|+?%-~   ^ .";
        let ident = self.pop_kind(Kind::Identifier).ok_or(ParseError)?;
        let mut text = [0_u8; 64];
        let source = ident.text().unwrap_or("");
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
        let len = source.len();
        // Lowercase ASCII mapped through an ASCII table: the conversion cannot
        // fail.
        let text = core::str::from_utf8(&text[..len]).map_err(|_| ParseError)?;
        let kind = match self.next_byte().ok_or(ParseError)? {
            b'i' => Kind::InfixOperator,
            b'p' => Kind::PrefixOperator,
            b'P' => Kind::PostfixOperator,
            _ => return Err(ParseError),
        };
        self.alloc_owned(kind, text)
    }

    fn integer_type(&mut self) -> PResult<'a> {
        if self.next_if(b'n') {
            let value = self.index().ok_or(ParseError)?;
            let node = self.alloc_value(Kind::NegativeInteger, value)?;
            return self.wrap_type(node);
        }
        let value = self.index().ok_or(ParseError)?;
        let node = self.alloc_value(Kind::Integer, value)?;
        self.wrap_type(node)
    }
}

/// Map a `T`-operator specialization code to its typed kind.
///
/// `t` (dropped arguments) is a prefix handled by its own entry point and
/// `p`/`P` (partial specializations) are not implemented.
const fn specialization_kind(code: u8) -> Option<GenericSpecializationKind> {
    Some(match code {
        b'g' => GenericSpecializationKind::Generic,
        b'G' => GenericSpecializationKind::NotReabstracted,
        b'B' => GenericSpecializationKind::ResilienceDomain,
        b's' => GenericSpecializationKind::Prespecialized,
        b'i' => GenericSpecializationKind::Inlined,
        _ => return None,
    })
}

fn punycode_digit(byte: u8) -> Option<u32> {
    match byte {
        b'a'..=b'z' => Some(u32::from(byte - b'a')),
        b'A'..=b'J' => Some(u32::from(byte - b'A') + 26),
        _ => None,
    }
}

fn punycode_adapt(delta: i64, points: i64, first: bool) -> i32 {
    let mut delta = if first { delta / 700 } else { delta / 2 };
    delta += delta / points;
    let mut k = 0_i64;
    while delta > ((36 - 1) * 26) / 2 {
        delta /= 36 - 1;
        k += 36;
    }
    let value = k + ((36 - 1 + 1) * delta) / (delta + 38);
    i32::try_from(value).unwrap_or(0)
}

pub fn punycode_decode(input: &[u8], out: &mut [u8]) -> Option<usize> {
    let mut points = [0_u32; 128];
    let mut count = 0_usize;
    let (basic, encoded) = input.iter().rposition(|&byte| byte == b'_').map_or_else(
        || (&[][..], input),
        |index| (&input[..index], &input[index + 1..]),
    );
    for &byte in basic {
        if byte > 0x7f || count >= points.len() {
            return None;
        }
        points[count] = u32::from(byte);
        count += 1;
    }
    let mut n = 128_u32;
    let mut i = 0_i64;
    let mut bias = 72_i32;
    let mut position = 0_usize;
    while position < encoded.len() {
        let old = i;
        let mut weight = 1_i64;
        let mut k = 36_i64;
        loop {
            let byte = *encoded.get(position)?;
            position += 1;
            let digit = i64::from(punycode_digit(byte)?);
            i = i.checked_add(digit.checked_mul(weight)?)?;
            let bias64 = i64::from(bias);
            let t = if k <= bias64 {
                1
            } else if k >= bias64 + 26 {
                26
            } else {
                k - bias64
            };
            if digit < t {
                break;
            }
            weight = weight.checked_mul(36 - t)?;
            k += 36;
        }
        let points_now = i64::try_from(count).ok()?.checked_add(1)?;
        bias = punycode_adapt(i - old, points_now, old == 0);
        let quotient = u32::try_from(i / points_now).ok()?;
        n = n.checked_add(quotient)?;
        i %= points_now;
        if n < 0x80 || count >= points.len() {
            return None;
        }
        let at = usize::try_from(i).ok()?;
        if at > count {
            return None;
        }
        points.copy_within(at..count, at + 1);
        points[at] = n;
        count += 1;
        i += 1;
    }
    let mut len = 0_usize;
    for point in &points[..count] {
        let mut point = *point;
        if (0xD800..0xD880).contains(&point) {
            point -= 0xD800;
        }
        let ch = char::from_u32(point)?;
        let mut buf = [0_u8; 4];
        let text = ch.encode_utf8(&mut buf);
        let end = len.checked_add(text.len())?;
        if end > out.len() {
            return None;
        }
        out[len..end].copy_from_slice(text.as_bytes());
        len = end;
    }
    Some(len)
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
        b'e' => Some("Builtin.Executor"),
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
            b'h' => Some((K::Protocol, "TaskExecutor")),
            b'g' => Some((K::Structure, "ThrowingTaskGroup")),
            b'I' => Some((K::Protocol, "AsyncIteratorProtocol")),
            b'i' => Some((K::Protocol, "AsyncSequence")),
            b'J' => Some((K::Structure, "UnownedJob")),
            b'M' => Some((K::Class, "MainActor")),
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
        b'X' => Some((K::Protocol, "RangeExpression")),
        b'x' => Some((K::Protocol, "Strideable")),
        b'Y' => Some((K::Protocol, "RawRepresentable")),
        b'y' => Some((K::Protocol, "StringProtocol")),
        b'Z' => Some((K::Protocol, "SignedInteger")),
        b'z' => Some((K::Protocol, "BinaryInteger")),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::{Limits, State};
    use crate::swift::DEFAULT_BUDGET;
    use alloc::rc::Rc;

    fn state() -> State {
        State::new(Limits {
            input: 64,
            depth: 4,
            work: 1024,
        })
    }

    fn parser<'a>(state: &mut State) -> Parser<'a, '_> {
        let cursor = Cursor::new(b"", 0, state).expect("empty input fits the budget");
        Parser::new(cursor, false, ParseLimits::from_effective(DEFAULT_BUDGET))
    }

    /// Render a stable Swift symbol, or `None` when it is rejected.
    fn render(symbol: &str) -> Option<std::string::String> {
        let mut out = std::string::String::new();
        crate::swift::print(symbol, &mut out).ok()?;
        Some(out)
    }

    /// Find the first node of `kind` in the parsed tree.
    fn find<'a>(ast: &crate::swift::Ast<'a>, kind: Kind) -> Option<NodeRef<'a>> {
        let mut stack = std::vec![ast.root()];
        while let Some(node) = stack.pop() {
            if node.kind() == kind {
                return Some(node);
            }
            stack.extend(node.children().iter().cloned());
        }
        None
    }

    #[test]
    fn macro_expansion_roles_include_preamble() {
        // `fMq` is an attached role the grammar defines; an unknown role letter
        // must reject instead of defaulting to a known one.
        let rendered = render("$s1a1b1cfMq0_").expect("preamble macro expands");
        assert!(rendered.contains("preamble macro"), "{rendered}");
        assert!(render("$s1a1b1cfMx0_").is_none());
    }

    #[test]
    fn layout_constraint_codes_are_validated() {
        let ast = crate::swift::parse("$s1a1bRlzE3_5_1c").expect("layout requirement parses");
        let node = find(&ast, Kind::DependentGenericLayoutRequirement).expect("layout node");
        assert_eq!(node.layout_code(), LayoutCode::TrivialExactSizeAndAlignment);
        assert_eq!(node.child_count(), 3, "subject, size and alignment");
        assert_eq!(node.child(1).map(|size| size.number_value()), Some(4));
        assert_eq!(
            node.child(2).map(|alignment| alignment.number_value()),
            Some(6)
        );

        let ast = crate::swift::parse("$s1a1bRlzS3_1c").expect("stride layout parses");
        let node = find(&ast, Kind::DependentGenericLayoutRequirement).expect("layout node");
        assert_eq!(node.layout_code(), LayoutCode::TrivialStride);
        assert_eq!(node.child_count(), 2, "subject and size");

        assert!(render("$s1a1bRlzx1c").is_none(), "unknown letters reject");
    }

    #[test]
    fn differentiable_function_type_codes_are_validated() {
        for (symbol, code) in [
            ("$sSiyYjfc", AutoDiffCode::Forward),
            ("$sSiyYjrc", AutoDiffCode::Reverse),
            ("$sSiyYjdc", AutoDiffCode::Differential),
            ("$sSiyYjlc", AutoDiffCode::Linear),
        ] {
            let ast = crate::swift::parse(symbol).expect("annotation parses");
            let node = find(&ast, Kind::DifferentiableFunctionType).expect("annotation node");
            assert_eq!(node.autodiff_code(), code);
        }
        // `p` belongs to `AutoDiffFunctionKind` only; other bytes are invalid.
        assert!(render("$sSiyYjpc").is_none());
        assert!(render("$sSiyYjXc").is_none());
    }

    #[test]
    fn specialization_kinds_are_closed() {
        assert_eq!(
            specialization_kind(b'g'),
            Some(GenericSpecializationKind::Generic)
        );
        assert_eq!(
            specialization_kind(b'G'),
            Some(GenericSpecializationKind::NotReabstracted)
        );
        assert_eq!(
            specialization_kind(b'B'),
            Some(GenericSpecializationKind::ResilienceDomain)
        );
        assert_eq!(
            specialization_kind(b's'),
            Some(GenericSpecializationKind::Prespecialized)
        );
        assert_eq!(
            specialization_kind(b'i'),
            Some(GenericSpecializationKind::Inlined)
        );
        for code in *b"tfpPx" {
            assert_eq!(specialization_kind(code), None, "{code:?}");
        }
    }

    #[test]
    fn representation_changed_flag_is_recorded() {
        let ast = crate::swift::parse("$s1a1b1cSiyTgr5").expect("specialization parses");
        let node = find(&ast, Kind::GenericSpecialization).expect("specialization node");
        assert_eq!(
            node.generic_specialization(),
            Some((GenericSpecializationKind::Generic, true))
        );

        let ast = crate::swift::parse("$s1a1b1cSiyTfr5").expect("specialization parses");
        let node = find(&ast, Kind::FunctionSignatureSpecialization).expect("spec node");
        assert!(node.function_signature_representation_changed());

        assert!(
            render("$s1a1b1cSiyTp5").is_none(),
            "partial specializations reject"
        );
    }

    /// Push `elements` nodes below `marker`, so the loop pops them in reverse.
    fn push_list(parser: &mut Parser<'_, '_>, elements: usize) {
        let bottom = parser.alloc(Kind::Identifier).unwrap();
        let marker = parser.alloc(Kind::FirstElementMarker).unwrap();
        parser.stack.push(bottom);
        parser.stack.push(marker);
        for _ in 1..elements {
            let node = parser.alloc(Kind::Identifier).unwrap();
            parser.stack.push(node);
        }
    }

    #[test]
    fn reverse_list_pops_elements_in_reverse_order() {
        let mut state = state();
        let mut parser = parser(&mut state);
        let first = parser.alloc(Kind::Identifier).unwrap();
        let marker = parser.alloc(Kind::FirstElementMarker).unwrap();
        let last = parser.alloc(Kind::Identifier).unwrap();
        parser.stack.push(first.clone());
        parser.stack.push(marker);
        parser.stack.push(last.clone());

        let elements = parser
            .pop_reverse_list(Parser::pop_any)
            .expect("well-formed list");
        assert_eq!(elements.len(), 2);
        assert!(Rc::ptr_eq(&elements[0], &last));
        assert!(Rc::ptr_eq(&elements[1], &first));
    }

    #[test]
    fn reverse_list_length_is_bounded_by_the_node_budget_alone() {
        let mut state = state();
        let mut parser = parser(&mut state);

        push_list(&mut parser, 100);
        let elements = parser
            .pop_reverse_list(Parser::pop_any)
            .expect("a list has no length cap of its own");
        assert_eq!(elements.len(), 100);
    }
}
