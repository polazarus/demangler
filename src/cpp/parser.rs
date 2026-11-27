//! Itanium C++ grammar parser.
//!
//! The parser builds immutable [`Node`] handles directly. Substitution entries
//! are stored as `Rc` handles, so a back-reference shares the referenced node
//! instead of copying it.

use alloc::{borrow::Cow, rc::Rc, vec::Vec};

use crate::parse::{Cursor, State};

use super::node::{Kind, Node, NodeRef};
use super::{
    Ast, MIN_WORK, ParseError, ParseErrorKind, ParseLimits, WORK_PER_BYTE, cursor_error, decode,
};

/// Maximum nesting depth for recursive productions.
///
/// This is the compiled-in depth budget; a [`crate::Budget`] may lower it but
/// not raise it, because the fixed print envelope bounds the nesting a
/// retained tree may have.
pub const MAX_DEPTH: u32 = 256;
/// Maximum nodes one parse may allocate.
///
/// This is the compiled-in node budget; a [`crate::Budget`] may lower or raise
/// it up to the scheme's hard ceiling.
pub const MAX_NODES: u32 = 1 << 14;

/// Recursive-descent parser over one mangled symbol.
#[expect(
    clippy::struct_excessive_bools,
    reason = "independent grammar context flags of the Itanium parser"
)]
pub struct Parser<'a, 's> {
    input: Cursor<'a, 's>,
    bytes: &'a [u8],
    subs: Vec<NodeRef<'a>>,
    limits: ParseLimits,
    depth: u32,
    nodes: u32,
    work: usize,
    work_limit: usize,
    /// Depth of the enclosing `Z <encoding> E` forms.
    ///
    /// A nested encoding ends at the closing `E` instead of at the end of the
    /// symbol, so its bare-function-type stops there.
    nested_encoding: u32,
    /// False while parsing a conversion operator's target type.
    ///
    /// A conversion operator's `<template-args>` belong to the name, not to a
    /// trailing template-parameter type, so `T_ I...E` must stop after `T_`.
    try_parse_template_args: bool,
    /// True while parsing the constraint name of a `Tk` declaration.
    ///
    /// Enclosing template parameter levels are not tracked well enough to
    /// substitute references there, so they print their encoded numbering.
    incomplete_template_params: bool,
    /// True when the name just parsed carries the `H` marker.
    ///
    /// An `N H ... E` name declares an explicit object parameter, so the
    /// encoding's first parameter prints as `this <type>`. The flag is set by
    /// the outermost nested name of an encoding and read immediately after the
    /// encoding parses its name.
    explicit_object: bool,
    /// True when the symbol carries LLVM's allocation-token decoration.
    ///
    /// The decoration is a prefix, so it is not part of the encoding the parser
    /// reads; the flag makes the finished encoding report it.
    alloc_token: bool,
    /// True when a `Kind::Lambda` node was built.
    has_lambda: bool,
    has_template_param: bool,
}

impl<'a, 's> Parser<'a, 's> {
    /// Create a parser positioned after the mangling prefix.
    pub(crate) fn new(
        bytes: &'a [u8],
        prefix: usize,
        state: &'s mut State,
        limits: ParseLimits,
    ) -> Result<Self, ParseError> {
        if bytes.len() > limits.input() {
            return Err(ParseError::new(0, ParseErrorKind::TooLong));
        }
        // The allowance is derived from the symbol length and capped by the
        // resolved work budget, so raising the input budget alone does not
        // raise the work allowance.
        let work_limit = bytes
            .len()
            .saturating_mul(WORK_PER_BYTE)
            .saturating_add(MIN_WORK)
            .min(limits.work());
        let mut input = Cursor::new(bytes, 0, state).map_err(cursor_error)?;
        let _ = input.advance(prefix).map_err(cursor_error)?;
        Ok(Self {
            input,
            bytes,
            subs: Vec::new(),
            limits,
            depth: 0,
            nodes: 0,
            work: 0,
            work_limit,
            nested_encoding: 0,
            try_parse_template_args: true,
            incomplete_template_params: false,
            explicit_object: false,
            alloc_token: false,
            has_lambda: false,
            has_template_param: false,
        })
    }

    /// Create a parser for a symbol that carries LLVM's allocation-token
    /// decoration.
    pub(crate) fn new_alloc_token(
        bytes: &'a [u8],
        prefix: usize,
        state: &'s mut State,
        limits: ParseLimits,
    ) -> Result<Self, ParseError> {
        let mut parser = Self::new(bytes, prefix, state, limits)?;
        parser.alloc_token = true;
        Ok(parser)
    }

    /// Parse the encoding and require the whole symbol to be consumed.
    ///
    /// Two extensions wrap the encoding: `___Z <encoding> _block_invoke`
    /// (with optional digits) names a block's invocation function, and a
    /// trailing `.suffix` is not part of the mangling at all.
    pub(crate) fn parse(mut self) -> Result<Ast<'a>, ParseError> {
        let start = self.input.pos();
        // Only the three- and four-underscore prefixes introduce a block
        // invocation.
        let block_prefix = self.input.pos() >= 4;
        let mut encoding = self.parse_encoding()?;
        if block_prefix && self.starts_with(b"_block_invoke") {
            self.bump("_block_invoke".len())?;
            self.tick()?;
            let require_number = self.eat(b'_');
            let mut has_number = false;
            while matches!(self.peek(), Some(b'0'..=b'9')) {
                self.bump(1)?;
                self.tick()?;
                has_number = true;
            }
            if require_number && !has_number {
                return Err(self.error(ParseErrorKind::UnexpectedByte));
            }
            // A `.suffix` on a block invocation is not part of the name.
            while self.peek().is_some() {
                self.bump(1)?;
                self.tick()?;
            }
            encoding = self.make(Kind::BlockInvoke, None, start, alloc::vec![encoding])?;
        } else if self.peek() == Some(b'.') {
            let suffix_start = self.input.pos();
            while self.peek().is_some() {
                self.bump(1)?;
                self.tick()?;
            }
            let text = self.consumed(suffix_start, self.input.pos());
            encoding = self.make(
                Kind::DotSuffix,
                Some(decode(text)),
                start,
                alloc::vec![encoding],
            )?;
        }
        if self.input.pos() != self.bytes.len() {
            return Err(self.error(ParseErrorKind::UnexpectedByte));
        }
        if self.alloc_token {
            encoding = self.make(
                Kind::AllocToken,
                Some(Cow::Borrowed(".alloc_token")),
                start,
                alloc::vec![encoding],
            )?;
        }
        let root = self.make(Kind::Encoding, None, start, alloc::vec![encoding])?;
        Ok(Ast::from_root(
            root,
            self.has_lambda,
            self.has_template_param,
        ))
    }

    const fn error(&self, kind: ParseErrorKind) -> ParseError {
        ParseError::new(self.input.pos(), kind)
    }

    /// Borrow input bytes the cursor has already consumed.
    ///
    /// Both ends come from cursor positions, so the range is always in bounds;
    /// an out-of-range request yields an empty slice rather than a panic.
    fn consumed(&self, start: usize, end: usize) -> &'a [u8] {
        self.bytes.get(start..end).unwrap_or_default()
    }

    fn peek(&self) -> Option<u8> {
        self.input.peek()
    }

    fn peek_at(&self, offset: usize) -> Option<u8> {
        self.input.peek_at(offset)
    }

    fn starts_with(&self, tag: &[u8]) -> bool {
        self.input.starts_with(tag)
    }

    /// Advance past `count` bytes, preserving the parser's own work budget.
    fn bump(&mut self, count: usize) -> Result<(), ParseError> {
        self.input.advance(count).map(|_| ()).map_err(cursor_error)
    }

    fn eat(&mut self, byte: u8) -> bool {
        if self.peek() == Some(byte) {
            let _ = self.input.advance(1);
            true
        } else {
            false
        }
    }

    fn expect(&mut self, byte: u8) -> Result<(), ParseError> {
        if self.eat(byte) {
            Ok(())
        } else if self.peek().is_none() {
            Err(self.error(ParseErrorKind::UnexpectedEnd))
        } else {
            Err(self.error(ParseErrorKind::UnexpectedByte))
        }
    }

    const fn tick(&mut self) -> Result<(), ParseError> {
        self.work += 1;
        if self.work > self.work_limit {
            return Err(self.error(ParseErrorKind::TooMuchWork));
        }
        Ok(())
    }

    fn enter(&mut self) -> Result<(), ParseError> {
        self.tick()?;
        if self.depth >= self.limits.depth() {
            return Err(self.error(ParseErrorKind::TooDeep));
        }
        self.depth += 1;
        Ok(())
    }

    const fn leave(&mut self) {
        self.depth -= 1;
    }

    fn make(
        &mut self,
        kind: Kind,
        text: Option<Cow<'a, str>>,
        start: usize,
        children: Vec<NodeRef<'a>>,
    ) -> Result<NodeRef<'a>, ParseError> {
        if kind == Kind::Lambda {
            self.has_lambda = true;
        }
        if kind == Kind::TemplateParam {
            self.has_template_param = true;
        }
        self.nodes = self
            .nodes
            .checked_add(1)
            .ok_or_else(|| self.error(ParseErrorKind::TooManyNodes))?;
        if self.nodes > self.limits.nodes() {
            return Err(self.error(ParseErrorKind::TooManyNodes));
        }
        let end = self.input.pos();
        let span = if start <= end && end <= self.bytes.len() {
            u32::try_from(start)
                .ok()
                .zip(u32::try_from(end).ok())
                .map(|(start, end)| start..end)
        } else {
            None
        };
        Ok(Node::new(kind, text, span, children))
    }

    fn span_start(node: &NodeRef<'a>) -> Option<usize> {
        node.span().map(|range| range.start as usize)
    }

    fn push_sub(&mut self, node: NodeRef<'a>) {
        self.subs.push(node);
    }

    // -- encodings ---------------------------------------------------------

    fn parse_encoding(&mut self) -> Result<NodeRef<'a>, ParseError> {
        self.enter()?;
        let result = self.parse_encoding_inner();
        self.leave();
        result
    }

    fn parse_encoding_inner(&mut self) -> Result<NodeRef<'a>, ParseError> {
        self.tick()?;
        let start = self.input.pos();
        if matches!(self.peek(), Some(b'T' | b'G')) {
            return self.parse_special_name(start);
        }
        let name = self.parse_name()?;
        let explicit_object = core::mem::replace(&mut self.explicit_object, false);
        if self.at_encoding_end() {
            return self.make(Kind::Data, None, start, alloc::vec![name]);
        }
        let attributes = self.parse_function_attributes()?;
        let return_type = if name_ends_with_template(&name)
            && !name_ends_with_ctor(&name)
            && !name_ends_with_conversion_operator(&name)
        {
            Some(self.parse_type()?)
        } else {
            None
        };
        let parameters = self.parse_bare_function_type(explicit_object)?;
        let mut children = Vec::with_capacity(5);
        children.push(name);
        if let Some(return_type) = return_type {
            children.push(return_type);
        }
        children.push(parameters);
        children.extend(attributes);
        if let Some(clause) = self.parse_requires_clause()? {
            children.push(clause);
        }
        self.make(Kind::Function, None, start, children)
    }

    /// Parse an optional `Q <expression>` requires-clause.
    ///
    /// The clause is attached to a function encoding, to a lambda signature and
    /// to a template argument list. Enclosing template parameter levels are not
    /// tracked inside a constraint, so parameter references there print their
    /// encoded numbering, matching the reference demangler.
    fn parse_requires_clause(&mut self) -> Result<Option<NodeRef<'a>>, ParseError> {
        if self.peek() != Some(b'Q') {
            return Ok(None);
        }
        let start = self.input.pos();
        self.bump(1)?;
        self.tick()?;
        let saved = core::mem::replace(&mut self.incomplete_template_params, true);
        let constraint = self.parse_expression();
        self.incomplete_template_params = saved;
        let constraint = constraint?;
        Ok(Some(self.make(
            Kind::RequiresClause,
            None,
            start,
            alloc::vec![constraint],
        )?))
    }

    /// Parse an optional `Ua9enable_ifI <template-arg>* E` function attribute.
    fn parse_function_attributes(&mut self) -> Result<Option<NodeRef<'a>>, ParseError> {
        let start = self.input.pos();
        if !self.starts_with(b"Ua9enable_ifI") {
            return Ok(None);
        }
        self.bump("Ua9enable_ifI".len())?;
        self.tick()?;
        let mut arguments = Vec::new();
        while !self.eat(b'E') {
            if self.peek().is_none() {
                return Err(self.error(ParseErrorKind::UnexpectedEnd));
            }
            arguments.push(self.parse_template_arg()?);
        }
        Ok(Some(self.make(
            Kind::FunctionAttribute,
            Some(Cow::Borrowed("enable_if")),
            start,
            arguments,
        )?))
    }

    /// Parse the parameter list of an encoding.
    ///
    /// `explicit_object` marks an encoding whose name carried the `H` marker:
    /// its first parameter is the explicit object parameter.
    fn parse_bare_function_type(
        &mut self,
        explicit_object: bool,
    ) -> Result<NodeRef<'a>, ParseError> {
        let start = self.input.pos();
        let mut parameters = Vec::new();
        if self.peek() == Some(b'v') && self.ends_bare_function_type(1) {
            self.bump(1)?;
            self.tick()?;
        } else {
            while !self.at_encoding_end() {
                let ty = self.parse_type()?;
                let ty = if explicit_object && parameters.is_empty() {
                    let ty_start = Self::span_start(&ty).unwrap_or_else(|| self.input.pos());
                    self.make(
                        Kind::ExplicitObjectParameter,
                        None,
                        ty_start,
                        alloc::vec![ty],
                    )?
                } else {
                    ty
                };
                parameters.push(self.make_parameter(ty)?);
            }
        }
        self.make(Kind::ParameterList, None, start, parameters)
    }

    /// True when the byte at `offset` ends a bare-function-type.
    fn ends_bare_function_type(&self, offset: usize) -> bool {
        match self.peek_at(offset) {
            None | Some(b'Q' | b'_' | b'.') => true,
            Some(b'E') => self.nested_encoding > 0,
            Some(_) => false,
        }
    }

    /// True at the end of an encoding: the symbol, the closing `E` of a nested
    /// encoding, a requires-clause, or a byte that cannot start a type
    /// (`_block_invoke`, a `.` suffix).
    fn at_encoding_end(&self) -> bool {
        match self.peek() {
            None | Some(b'Q' | b'_' | b'.') => true,
            Some(b'E') => self.nested_encoding > 0,
            Some(_) => false,
        }
    }

    fn make_parameter(&mut self, ty: NodeRef<'a>) -> Result<NodeRef<'a>, ParseError> {
        let start = Self::span_start(&ty).unwrap_or_else(|| self.input.pos());
        self.make(Kind::Parameter, None, start, alloc::vec![ty])
    }

    // -- special names -----------------------------------------------------

    /// Parse a `<special-name>`, the third shape an encoding can take.
    fn parse_special_name(&mut self, start: usize) -> Result<NodeRef<'a>, ParseError> {
        self.tick()?;
        if self.starts_with(b"TV") {
            self.bump(2)?;
            let ty = self.parse_type()?;
            return self.make(Kind::VTable, None, start, alloc::vec![ty]);
        }
        if self.starts_with(b"TT") {
            self.bump(2)?;
            let ty = self.parse_type()?;
            return self.make(Kind::Vtt, None, start, alloc::vec![ty]);
        }
        if self.starts_with(b"TI") {
            self.bump(2)?;
            let ty = self.parse_type()?;
            return self.make(Kind::TypeInfo, None, start, alloc::vec![ty]);
        }
        if self.starts_with(b"TS") {
            self.bump(2)?;
            let ty = self.parse_type()?;
            return self.make(Kind::TypeInfoName, None, start, alloc::vec![ty]);
        }
        if self.starts_with(b"TH") {
            self.bump(2)?;
            let name = self.parse_name()?;
            return self.make(Kind::TlsInit, None, start, alloc::vec![name]);
        }
        if self.starts_with(b"TW") {
            self.bump(2)?;
            let name = self.parse_name()?;
            return self.make(Kind::TlsWrapper, None, start, alloc::vec![name]);
        }
        if self.starts_with(b"GV") {
            self.bump(2)?;
            let name = self.parse_name()?;
            return self.make(Kind::GuardVariable, None, start, alloc::vec![name]);
        }
        if self.starts_with(b"GR") {
            self.bump(2)?;
            let name = self.parse_name()?;
            let index = if self.eat(b'_') {
                0
            } else if matches!(self.peek(), Some(b'0'..=b'9' | b'A'..=b'Z')) {
                let index = self
                    .parse_seq_id_digits()?
                    .checked_add(1)
                    .ok_or_else(|| self.error(ParseErrorKind::InvalidLength))?;
                self.expect(b'_')?;
                index
            } else {
                0
            };
            let text = Cow::Owned(alloc::format!("{index}"));
            return self.make(
                Kind::ReferenceTemporary,
                Some(text),
                start,
                alloc::vec![name],
            );
        }
        if self.starts_with(b"GI") {
            self.bump(2)?;
            let module = self.parse_module_name()?;
            return self.make(Kind::ModuleInitializer, None, start, alloc::vec![module]);
        }
        if self.starts_with(b"TC") {
            self.bump(2)?;
            let complete = self.parse_type()?;
            let _offset = self.parse_number()?;
            self.expect(b'_')?;
            let base = self.parse_type()?;
            return self.make(Kind::CtorVtable, None, start, alloc::vec![complete, base]);
        }
        if self.starts_with(b"TA") {
            self.bump(2)?;
            let argument = self.parse_template_arg()?;
            return self.make(
                Kind::TemplateParamObject,
                None,
                start,
                alloc::vec![argument],
            );
        }
        if self.starts_with(b"Th") || self.starts_with(b"Tv") {
            self.bump(1)?;
            return self.parse_thunk(start, false);
        }
        if self.starts_with(b"Tc") {
            self.bump(2)?;
            return self.parse_thunk(start, true);
        }
        Err(self.error(ParseErrorKind::Unsupported))
    }

    /// Parse the call offsets and encoding of a thunk.
    ///
    /// `covariant` selects the `Tc` form, which carries a second offset.
    fn parse_thunk(&mut self, start: usize, covariant: bool) -> Result<NodeRef<'a>, ParseError> {
        let first = self.parse_call_offset()?;
        let text = if covariant {
            let second = self.parse_call_offset()?;
            Cow::Owned(alloc::format!("{first}, {second}"))
        } else {
            Cow::Owned(first)
        };
        let encoding = self.parse_encoding()?;
        self.make(Kind::Thunk, Some(text), start, alloc::vec![encoding])
    }

    /// Parse `h <number> _` or `v <number> _ <number> _`.
    fn parse_call_offset(&mut self) -> Result<alloc::string::String, ParseError> {
        self.tick()?;
        if self.eat(b'h') {
            let value = self.parse_signed_number()?;
            self.expect(b'_')?;
            return Ok(alloc::format!("offset({value})"));
        }
        if self.eat(b'v') {
            let first = self.parse_signed_number()?;
            self.expect(b'_')?;
            let second = self.parse_signed_number()?;
            self.expect(b'_')?;
            return Ok(alloc::format!("virtual offset({first}, {second})"));
        }
        Err(self.error(ParseErrorKind::Unsupported))
    }

    fn parse_signed_number(&mut self) -> Result<i64, ParseError> {
        let negative = self.eat(b'n');
        let value = self.parse_number()?;
        let value = i64::try_from(value).map_err(|_| self.error(ParseErrorKind::InvalidLength))?;
        Ok(if negative { -value } else { value })
    }

    // -- names -------------------------------------------------------------

    fn parse_name(&mut self) -> Result<NodeRef<'a>, ParseError> {
        self.enter()?;
        let result = self.parse_name_inner();
        self.leave();
        result
    }

    fn parse_name_inner(&mut self) -> Result<NodeRef<'a>, ParseError> {
        self.tick()?;
        match self.peek() {
            Some(b'N') => self.parse_nested_name(),
            Some(b'Z') => self.parse_local_name(),
            Some(_) => {
                self.explicit_object = false;
                self.parse_unscoped_name()
            }
            None => Err(self.error(ParseErrorKind::UnexpectedEnd)),
        }
    }

    /// Parse `Z <encoding> E <entity> [<discriminator>]`.
    ///
    /// The `Es` form names a string literal and the `Ed` form carries the
    /// number of the enclosing function parameter the local name belongs to.
    fn parse_local_name(&mut self) -> Result<NodeRef<'a>, ParseError> {
        let start = self.input.pos();
        self.expect(b'Z')?;
        self.nested_encoding += 1;
        let encoding = self.parse_encoding();
        self.nested_encoding -= 1;
        let encoding = encoding?;
        self.expect(b'E')?;
        if self.eat(b's') {
            self.tick()?;
            let entity = self.make(
                Kind::Identifier,
                Some(Cow::Borrowed("string literal")),
                start,
                Vec::new(),
            )?;
            let discriminator = self.parse_discriminator()?;
            return self.make(
                Kind::LocalName,
                discriminator,
                start,
                alloc::vec![encoding, entity],
            );
        }
        if self.peek() == Some(b'd') && matches!(self.peek_at(1), Some(b'_' | b'0'..=b'9')) {
            self.bump(1)?;
            self.tick()?;
            let number = if matches!(self.peek(), Some(b'0'..=b'9')) {
                let digits = self.parse_digits()?;
                Some(decode(digits))
            } else {
                None
            };
            self.expect(b'_')?;
            let entity = self.parse_name()?;
            return self.make(
                Kind::LocalName,
                number,
                start,
                alloc::vec![encoding, entity],
            );
        }
        let entity = self.parse_name()?;
        let discriminator = self.parse_discriminator()?;
        self.make(
            Kind::LocalName,
            discriminator,
            start,
            alloc::vec![encoding, entity],
        )
    }

    /// Parse an optional local-name discriminator.
    ///
    /// The ABI spells it `_ <digit>` or `__ <number> _`; GCC also emits bare
    /// digits at the end of the symbol, which the reference demangler accepts
    /// only when they run to the end of the input.
    fn parse_discriminator(&mut self) -> Result<Option<Cow<'a, str>>, ParseError> {
        match (self.peek(), self.peek_at(1)) {
            (Some(b'_'), Some(b'_')) => {
                self.bump(2)?;
                let digits_start = self.input.pos();
                while matches!(self.peek(), Some(b'0'..=b'9')) {
                    self.bump(1)?;
                    self.tick()?;
                }
                let digits_end = self.input.pos();
                self.expect(b'_')?;
                let digits = self.consumed(digits_start, digits_end);
                Ok(Some(decode(digits)))
            }
            (Some(b'_'), Some(b'0'..=b'9')) => {
                self.bump(1)?;
                let digits_start = self.input.pos();
                self.bump(1)?;
                self.tick()?;
                let digits = self.consumed(digits_start, self.input.pos());
                Ok(Some(decode(digits)))
            }
            (Some(b'0'..=b'9'), _) => {
                // Bare digits are only a discriminator when nothing follows:
                // a leading digit otherwise starts a source-name length.
                let start = self.input.pos();
                let checkpoint = self.input.checkpoint();
                while matches!(self.peek(), Some(b'0'..=b'9')) {
                    self.bump(1)?;
                    self.tick()?;
                }
                if self.input.pos() != self.bytes.len() {
                    self.input.reset(checkpoint);
                    return Ok(None);
                }
                let digits = self.consumed(start, self.input.pos());
                Ok(Some(decode(digits)))
            }
            _ => Ok(None),
        }
    }

    fn parse_unscoped_name(&mut self) -> Result<NodeRef<'a>, ParseError> {
        let start = self.input.pos();
        if self.starts_with(b"St") {
            self.bump(2)?;
            self.tick()?;
            let std = self.make(
                Kind::Substitution,
                Some(Cow::Borrowed("std")),
                start,
                Vec::new(),
            )?;
            let unqualified = self.parse_unqualified_name()?;
            let nested = self.make(Kind::NestedName, None, start, alloc::vec![std, unqualified])?;
            if self.peek() == Some(b'I') {
                self.push_sub(Rc::clone(&nested));
                let arguments = self.parse_template_args()?;
                return self.make(Kind::Name, None, start, alloc::vec![nested, arguments]);
            }
            return Ok(nested);
        }
        if self.peek() == Some(b'S') {
            let substitution = self.parse_substitution()?;
            let module = is_module_name(&substitution);
            let base = if module {
                self.combine_module_entity(substitution, start)?
            } else {
                substitution
            };
            if self.peek() == Some(b'I') {
                let arguments = self.parse_template_args()?;
                return self.make(Kind::Name, None, start, alloc::vec![base, arguments]);
            }
            return Ok(base);
        }
        if self.peek() == Some(b'W') {
            // Vendor extended qualifiers chain with the name they qualify:
            // `W3FOOW3BAR3Foo` is `Foo@FOO.BAR`. Each cumulative qualifier is a
            // substitution candidate in its own right.
            let (mut name, _entries) = self.parse_vendor_chain()?;
            if self.at_unqualified_name() {
                let unqualified = self.parse_unqualified_name()?;
                name = self.make(
                    Kind::NestedName,
                    None,
                    start,
                    alloc::vec![name, unqualified],
                )?;
            }
            if self.peek() == Some(b'I') {
                self.push_sub(Rc::clone(&name));
                let arguments = self.parse_template_args()?;
                return self.make(Kind::Name, None, start, alloc::vec![name, arguments]);
            }
            return Ok(name);
        }
        let base = self.parse_unqualified_name()?;
        if self.peek() == Some(b'I') {
            self.push_sub(Rc::clone(&base));
            let arguments = self.parse_template_args()?;
            return self.make(Kind::Name, None, start, alloc::vec![base, arguments]);
        }
        Ok(base)
    }

    fn parse_nested_name(&mut self) -> Result<NodeRef<'a>, ParseError> {
        self.enter()?;
        let result = self.parse_nested_name_inner();
        self.leave();
        result
    }

    #[expect(
        clippy::too_many_lines,
        reason = "one match arm per grammar production"
    )]
    fn parse_nested_name_inner(&mut self) -> Result<NodeRef<'a>, ParseError> {
        let start = self.input.pos();
        self.expect(b'N')?;
        let mut restrict = false;
        let mut volatile = false;
        let mut const_ = false;
        let mut ref_qual = 0_u8;
        // `N H ... E` declares an explicit object parameter instead of
        // carrying cv/ref qualifiers.
        let explicit_object = self.eat(b'H');
        if !explicit_object {
            if self.eat(b'r') {
                restrict = true;
            }
            if self.eat(b'V') {
                volatile = true;
            }
            if self.eat(b'K') {
                const_ = true;
            }
            if self.eat(b'R') {
                ref_qual = 1;
            } else if self.eat(b'O') {
                ref_qual = 2;
            }
        }
        let qualifier = name_qualifier_text(const_, volatile, restrict, ref_qual);

        let mut so_far: Option<NodeRef<'a>> = None;
        // Substitution entries the most recent component added. The last
        // component of a nested name is not substitutable, so they are dropped
        // when the closing `E` is reached.
        let mut pending_pops = 0_usize;
        loop {
            self.tick()?;
            if self.eat(b'E') {
                break;
            }
            pending_pops = 0;
            match self.peek() {
                None => return Err(self.error(ParseErrorKind::UnexpectedEnd)),
                Some(b'T') => {
                    if so_far.is_some() {
                        return Err(self.error(ParseErrorKind::UnexpectedByte));
                    }
                    so_far = Some(self.parse_template_param()?);
                }
                Some(b'I') => {
                    let base = so_far
                        .take()
                        .ok_or_else(|| self.error(ParseErrorKind::UnexpectedByte))?;
                    let base_start = Self::span_start(&base).unwrap_or(start);
                    let arguments = self.parse_template_args()?;
                    so_far = Some(self.make(
                        Kind::Name,
                        None,
                        base_start,
                        alloc::vec![base, arguments],
                    )?);
                }
                Some(b'S') => {
                    if self.starts_with(b"St") {
                        if so_far.is_some() {
                            return Err(self.error(ParseErrorKind::UnexpectedByte));
                        }
                        let std_start = self.input.pos();
                        self.bump(2)?;
                        self.tick()?;
                        so_far = Some(self.make(
                            Kind::Substitution,
                            Some(Cow::Borrowed("std")),
                            std_start,
                            Vec::new(),
                        )?);
                    } else {
                        // A substitution is a prefix component in its own
                        // right; `N S0_ S2_ 5Plugh E` names three components.
                        let component = self.parse_substitution()?;
                        so_far = Some(match so_far.take() {
                            Some(prefix) => self.make(
                                Kind::NestedName,
                                None,
                                start,
                                alloc::vec![prefix, component],
                            )?,
                            None => component,
                        });
                    }
                    let _ = self.eat(b'M');
                    continue;
                }
                Some(b'W') if matches!(self.peek_at(1), Some(b'0'..=b'9')) => {
                    // A vendor qualifier chain qualifies the component that
                    // follows it and is not a component of its own.
                    let (chain, entries) = self.parse_vendor_chain()?;
                    pending_pops = entries;
                    so_far = Some(match so_far.take() {
                        Some(prefix) => {
                            self.make(Kind::NestedName, None, start, alloc::vec![prefix, chain])?
                        }
                        None => chain,
                    });
                    let _ = self.eat(b'M');
                    continue;
                }
                Some(b'D') if matches!(self.peek_at(1), Some(b't' | b'T')) => {
                    return Err(self.error(ParseErrorKind::Unsupported));
                }
                Some(_) => {
                    let unqualified = self.parse_unqualified_name()?;
                    so_far = Some(match so_far.take() {
                        Some(prefix) => self.make(
                            Kind::NestedName,
                            None,
                            start,
                            alloc::vec![prefix, unqualified],
                        )?,
                        None => unqualified,
                    });
                }
            }
            let current = so_far
                .as_ref()
                .ok_or_else(|| self.error(ParseErrorKind::UnexpectedEnd))?;
            self.push_sub(Rc::clone(current));
            pending_pops = 1;
            let _ = self.eat(b'M');
        }
        for _ in 0..pending_pops {
            let _ = self.subs.pop();
        }
        let name = so_far.ok_or_else(|| self.error(ParseErrorKind::UnexpectedEnd))?;
        // The outermost nested name of an encoding reports the marker; a
        // nested name parsed inside it (a template argument, say) is not the
        // encoding's own name, so this assignment wins over any inner one.
        self.explicit_object = explicit_object;
        let Some(text) = qualifier else {
            return Ok(name);
        };
        let name_start = Self::span_start(&name).unwrap_or(start);
        if name.kind() == Kind::NestedName && name.text().is_none() {
            let children = name.children().to_vec();
            self.make(Kind::NestedName, Some(text), name_start, children)
        } else {
            self.make(Kind::NestedName, Some(text), name_start, alloc::vec![name])
        }
    }

    fn parse_unqualified_name(&mut self) -> Result<NodeRef<'a>, ParseError> {
        let leaf = self.parse_unqualified_name_leaf()?;
        self.parse_abi_tags(leaf)
    }

    #[expect(
        clippy::too_many_lines,
        reason = "one match arm per grammar production"
    )]
    fn parse_unqualified_name_leaf(&mut self) -> Result<NodeRef<'a>, ParseError> {
        let start = self.input.pos();
        self.tick()?;
        if self.eat(b'F') {
            // `F <source-name>` or `F <operator-name>`: a friend function
            // declared inside a class. The marker does not change the name.
            self.tick()?;
            if !self.at_unqualified_name() {
                return Err(self.error(ParseErrorKind::UnexpectedByte));
            }
            let inner = self.parse_unqualified_name_leaf()?;
            return self.make(Kind::FriendName, None, start, alloc::vec![inner]);
        }
        if self.peek() == Some(b'L') && matches!(self.peek_at(1), Some(b'0'..=b'9')) {
            // Internal-linkage marker. It does not change the name.
            self.bump(1)?;
            self.tick()?;
        }
        if self.starts_with(b"Ut") {
            self.bump(2)?;
            self.tick()?;
            let index = if matches!(self.peek(), Some(b'0'..=b'9')) {
                self.parse_number()?
            } else {
                0
            };
            self.expect(b'_')?;
            let text = Cow::Owned(alloc::format!("{}", index + 1));
            return self.make(Kind::UnnamedType, Some(text), start, Vec::new());
        }
        if self.starts_with(b"Ul") {
            return self.parse_lambda();
        }
        if self.starts_with(b"Ub") {
            self.bump(2)?;
            self.tick()?;
            if matches!(self.peek(), Some(b'0'..=b'9')) {
                let _ = self.parse_number()?;
            }
            self.expect(b'_')?;
            return self.make(
                Kind::Lambda,
                Some(Cow::Borrowed("block-literal")),
                start,
                Vec::new(),
            );
        }
        if self.starts_with(b"DC") {
            // A structured binding name: `DC <source-name>+ E`.
            self.bump(2)?;
            self.tick()?;
            let mut children = Vec::new();
            while !self.eat(b'E') {
                if self.peek().is_none() {
                    return Err(self.error(ParseErrorKind::UnexpectedEnd));
                }
                children.push(self.parse_source_name()?);
            }
            return self.make(Kind::StructuredBinding, None, start, children);
        }
        if self.peek() == Some(b'W') && matches!(self.peek_at(1), Some(b'0'..=b'9')) {
            return self.parse_vendor_qualifier();
        }
        if let Some(code) = self.ctor_dtor_code() {
            self.bump(code.len())?;
            self.tick()?;
            // Inheriting constructors carry the inherited class name. It is
            // not printed, but its template arguments are the constructor's
            // template parameter table and must stay reachable.
            let children = if code.starts_with("CI") {
                alloc::vec![self.parse_name()?]
            } else {
                Vec::new()
            };
            return self.make(
                Kind::CtorDtorName,
                Some(Cow::Borrowed(code)),
                start,
                children,
            );
        }
        if self.starts_with(b"cv") {
            self.bump(2)?;
            self.tick()?;
            // The trailing `I...E` belongs to the conversion operator's name,
            // not to a template-parameter type, so do not absorb it here.
            let saved = core::mem::replace(&mut self.try_parse_template_args, false);
            let target = self.parse_type();
            self.try_parse_template_args = saved;
            let target = target?;
            return self.make(
                Kind::OperatorName,
                Some(Cow::Borrowed("operator")),
                start,
                alloc::vec![target],
            );
        }
        if self.starts_with(b"li") {
            self.bump(2)?;
            self.tick()?;
            let suffix = self.parse_source_name()?;
            let text = alloc::format!("operator\"\" {}", suffix.text().unwrap_or(""));
            return self.make(
                Kind::OperatorName,
                Some(Cow::Owned(text)),
                start,
                alloc::vec![suffix],
            );
        }
        if let Some(spelling) = self.operator_spelling() {
            self.bump(2)?;
            self.tick()?;
            return self.make(
                Kind::OperatorName,
                Some(Cow::Borrowed(spelling)),
                start,
                Vec::new(),
            );
        }
        if matches!(self.peek(), Some(b'0'..=b'9')) {
            let identifier = self.parse_source_name()?;
            return self.make(Kind::UnqualifiedName, None, start, alloc::vec![identifier]);
        }
        Err(self.error(ParseErrorKind::UnexpectedByte))
    }

    /// True when the cursor starts an `<unqualified-name>`.
    fn at_unqualified_name(&self) -> bool {
        match self.peek() {
            Some(b'0'..=b'9') => true,
            Some(b'U') => matches!(self.peek_at(1), Some(b't' | b'l' | b'b')),
            Some(b'D') => self.peek_at(1) == Some(b'C'),
            Some(b'W' | b'L') => matches!(self.peek_at(1), Some(b'0'..=b'9')),
            Some(_) => {
                self.starts_with(b"cv")
                    || self.starts_with(b"li")
                    || self.ctor_dtor_code().is_some()
                    || self.operator_spelling().is_some()
            }
            None => false,
        }
    }

    /// Parse `W <source-name>`, the MIPS vendor extended qualifier.
    fn parse_vendor_qualifier(&mut self) -> Result<NodeRef<'a>, ParseError> {
        let start = self.input.pos();
        self.expect(b'W')?;
        self.tick()?;
        let text = self.parse_source_name_text()?;
        self.make(Kind::VendorQualifier, Some(decode(text)), start, Vec::new())
    }

    /// Parse a `W <source-name>` chain, one substitution entry per cumulative
    /// qualifier.
    ///
    /// A module name is a chain of sub-names: `W3FOOW3Bar` qualifies a name
    /// with `Foo.Bar`. The number of entries pushed is returned so a caller can
    /// drop them when the chain turns out not to be substitutable after all.
    fn parse_vendor_chain(&mut self) -> Result<(NodeRef<'a>, usize), ParseError> {
        let start = self.input.pos();
        let mut chain = self.parse_vendor_qualifier()?;
        self.push_sub(Rc::clone(&chain));
        let mut entries = 1;
        while self.peek() == Some(b'W') && matches!(self.peek_at(1), Some(b'0'..=b'9')) {
            self.extend_vendor_chain(&mut chain, start, entries)?;
            self.push_sub(Rc::clone(&chain));
            entries += 1;
        }
        Ok((chain, entries))
    }

    /// Append one `W <source-name>` sub-name to a vendor qualifier chain.
    ///
    /// The chain is a left-nested [`Kind::NestedName`] tree, so its length is a
    /// nesting depth: printing and inspecting it recurse once per sub-name.
    fn extend_vendor_chain(
        &mut self,
        chain: &mut NodeRef<'a>,
        start: usize,
        length: usize,
    ) -> Result<(), ParseError> {
        self.tick()?;
        if length >= self.limits.depth() as usize {
            return Err(self.error(ParseErrorKind::TooDeep));
        }
        let qualifier = self.parse_vendor_qualifier()?;
        *chain = self.make(
            Kind::NestedName,
            None,
            start,
            alloc::vec![Rc::clone(chain), qualifier],
        )?;
        Ok(())
    }

    /// Attach a following name to a module-name substitution.
    ///
    /// A vendor qualifier reached through a substitution (`S_`) is a module
    /// name that qualifies the name after it, so `S_ 3One` is the single
    /// substitutable entity `One@FOO`, not two template arguments. Every
    /// cumulative qualifier and the final qualified name take a substitution
    /// slot.
    fn combine_module_entity(
        &mut self,
        base: NodeRef<'a>,
        start: usize,
    ) -> Result<NodeRef<'a>, ParseError> {
        if !matches!(self.peek(), Some(b'0'..=b'9' | b'L' | b'U' | b'W')) {
            return Ok(base);
        }
        let mut name = base;
        let mut length = 1;
        while self.peek() == Some(b'W') && matches!(self.peek_at(1), Some(b'0'..=b'9')) {
            self.extend_vendor_chain(&mut name, start, length)?;
            self.push_sub(Rc::clone(&name));
            length += 1;
        }
        if self.at_unqualified_name() {
            let unqualified = self.parse_unqualified_name()?;
            name = self.make(
                Kind::NestedName,
                None,
                start,
                alloc::vec![name, unqualified],
            )?;
            self.push_sub(Rc::clone(&name));
        }
        Ok(name)
    }

    /// Parse `Ul <lambda-sig> E [<number>] _`.
    ///
    /// A lambda signature is a list of template parameter declarations
    /// followed by the parameter types.
    fn parse_lambda(&mut self) -> Result<NodeRef<'a>, ParseError> {
        let start = self.input.pos();
        self.expect(b'U')?;
        self.expect(b'l')?;
        let mut children = Vec::new();
        while is_template_param_decl(self.peek(), self.peek_at(1)) {
            children.push(self.parse_template_param_decl()?);
        }
        if self.peek() == Some(b'v') && self.peek_at(1) == Some(b'E') {
            self.bump(1)?;
            self.tick()?;
        } else {
            while self.peek() != Some(b'E') {
                if self.peek().is_none() {
                    return Err(self.error(ParseErrorKind::UnexpectedEnd));
                }
                // A template-head requires-clause belongs to the signature and
                // prints after the parameter declarations.
                if let Some(clause) = self.parse_requires_clause()? {
                    children.push(clause);
                    continue;
                }
                children.push(self.parse_type()?);
            }
        }
        self.expect(b'E')?;
        let number = if matches!(self.peek(), Some(b'0'..=b'9')) {
            Some(self.parse_number()?)
        } else {
            None
        };
        self.expect(b'_')?;
        // `Ul...E_` is closure #1; an explicit number is that value plus two.
        let index = match number {
            None => 1,
            Some(number) => number
                .checked_add(2)
                .ok_or_else(|| self.error(ParseErrorKind::InvalidLength))?,
        };
        let text = Cow::Owned(alloc::format!("{index}"));
        self.make(Kind::Lambda, Some(text), start, children)
    }

    /// Parse the `[<abi-tag>]` suffix of an unqualified name.
    fn parse_abi_tags(&mut self, mut name: NodeRef<'a>) -> Result<NodeRef<'a>, ParseError> {
        while self.peek() == Some(b'B') && matches!(self.peek_at(1), Some(b'0'..=b'9')) {
            self.bump(1)?;
            self.tick()?;
            let text = self.parse_source_name_text()?;
            let start = Self::span_start(&name).unwrap_or_else(|| self.input.pos());
            name = self.make(Kind::AbiTag, Some(decode(text)), start, alloc::vec![name])?;
        }
        Ok(name)
    }

    fn ctor_dtor_code(&self) -> Option<&'static str> {
        const CODES: &[&str] = &[
            "C1", "C2", "C3", "C4", "C5", "CI1", "CI2", "D0", "D1", "D2", "D4", "D5",
        ];
        CODES
            .iter()
            .copied()
            .find(|code| self.starts_with(code.as_bytes()))
    }

    /// Spell the `<operator-name>` code at the cursor without consuming it.
    ///
    /// Every operator code is exactly two ASCII bytes, so the code is matched
    /// directly instead of scanning a spelling table. A caller that accepts a
    /// hit advances exactly two bytes; a one-byte or unknown code yields
    /// `None`.
    fn operator_spelling(&self) -> Option<&'static str> {
        operator_code_spelling(self.peek()?, self.peek_at(1)?)
    }

    fn parse_source_name(&mut self) -> Result<NodeRef<'a>, ParseError> {
        let start = self.input.pos();
        let text = self.parse_source_name_text()?;
        self.make(Kind::Identifier, Some(decode(text)), start, Vec::new())
    }

    /// Parse `<source-name>` and return the identifier bytes it borrows.
    fn parse_source_name_text(&mut self) -> Result<&'a [u8], ParseError> {
        let length = self.parse_number()?;
        if length == 0 {
            return Err(self.error(ParseErrorKind::InvalidLength));
        }
        let text_start = self.input.pos();
        let text_end = text_start
            .checked_add(length)
            .ok_or_else(|| self.error(ParseErrorKind::InvalidLength))?;
        if text_end > self.bytes.len() {
            return Err(self.error(ParseErrorKind::UnexpectedEnd));
        }
        let bytes = self.consumed(text_start, text_end);
        self.bump(length)?;
        self.work = self.work.saturating_add(length);
        if self.work > self.work_limit {
            return Err(self.error(ParseErrorKind::TooMuchWork));
        }
        Ok(bytes)
    }

    // -- substitutions -----------------------------------------------------

    fn parse_substitution(&mut self) -> Result<NodeRef<'a>, ParseError> {
        let start = self.input.pos();
        self.expect(b'S')?;
        let well_known = match self.peek() {
            Some(b't') => Some("std"),
            Some(b'a') => Some("std::allocator"),
            Some(b'b') => Some("std::basic_string"),
            Some(b's') => Some("std::string"),
            Some(b'i') => Some("std::basic_istream<char, std::char_traits<char> >"),
            Some(b'o') => Some("std::ostream"),
            Some(b'd') => Some("std::basic_iostream<char, std::char_traits<char> >"),
            _ => None,
        };
        if let Some(text) = well_known {
            self.bump(1)?;
            self.tick()?;
            let node = self.make(
                Kind::Substitution,
                Some(Cow::Borrowed(text)),
                start,
                Vec::new(),
            )?;
            // An ABI tag on a well-known substitution is part of the
            // substituted entity: `SsB1X` is a substitutable
            // `std::string[abi:X]`.
            let tagged = self.parse_abi_tags(node.clone())?;
            if !Rc::ptr_eq(&tagged, &node) {
                self.push_sub(Rc::clone(&tagged));
            }
            return Ok(tagged);
        }
        let index = if self.eat(b'_') {
            0
        } else {
            self.parse_seq_id()?
                .checked_add(1)
                .ok_or_else(|| self.error(ParseErrorKind::InvalidLength))?
        };
        self.subs
            .get(index)
            .cloned()
            .ok_or_else(|| self.error(ParseErrorKind::UnknownSubstitution))
    }

    /// Parse a `<seq-id>` without its trailing `_`.
    fn parse_seq_id_digits(&mut self) -> Result<usize, ParseError> {
        let mut value = 0_usize;
        let mut any = false;
        while let Some(byte) = self.peek() {
            let digit = match byte {
                b'0'..=b'9' => byte - b'0',
                b'A'..=b'Z' => byte - b'A' + 10,
                _ => break,
            };
            value = value
                .checked_mul(36)
                .and_then(|current| current.checked_add(usize::from(digit)))
                .ok_or_else(|| self.error(ParseErrorKind::InvalidLength))?;
            self.bump(1)?;
            self.tick()?;
            any = true;
        }
        if !any {
            return Err(self.error(ParseErrorKind::UnexpectedByte));
        }
        Ok(value)
    }

    fn parse_seq_id(&mut self) -> Result<usize, ParseError> {
        let value = self.parse_seq_id_digits()?;
        self.expect(b'_')?;
        Ok(value)
    }

    /// Parse a `<module-name>`: one or more `W [P] <source-name>` sub-names.
    ///
    /// The chain is not added to the substitution table: the only caller is the
    /// `GI` special name, whose module name is not referenced again.
    fn parse_module_name(&mut self) -> Result<NodeRef<'a>, ParseError> {
        let start = self.input.pos();
        let mut children = Vec::new();
        let mut partitions = alloc::string::String::new();
        while self.eat(b'W') {
            self.tick()?;
            let partition = self.eat(b'P');
            partitions.push(if partition { '1' } else { '0' });
            let name = self.parse_source_name()?;
            children.push(self.make(Kind::UnqualifiedName, None, start, alloc::vec![name])?);
        }
        if children.is_empty() {
            return Err(self.error(ParseErrorKind::Unsupported));
        }
        self.make(
            Kind::ModuleName,
            Some(Cow::Owned(partitions)),
            start,
            children,
        )
    }

    // -- template arguments ------------------------------------------------

    fn parse_template_args(&mut self) -> Result<NodeRef<'a>, ParseError> {
        let start = self.input.pos();
        self.expect(b'I')?;
        let mut arguments: Vec<NodeRef<'a>> = Vec::new();
        // A `<template-param-decl>` qualifies the argument that follows it,
        // so it can neither end the list nor precede a requires-clause.
        let pending_decl = |arguments: &[NodeRef<'a>]| {
            arguments
                .last()
                .is_some_and(|last| last.kind() == Kind::TemplateParamDecl)
        };
        loop {
            self.tick()?;
            if self.peek() == Some(b'E') {
                if pending_decl(&arguments) {
                    return Err(self.error(ParseErrorKind::UnexpectedByte));
                }
                self.bump(1)?;
                break;
            }
            if self.peek().is_none() {
                return Err(self.error(ParseErrorKind::UnexpectedEnd));
            }
            // A template-head requires-clause sits among the arguments and is
            // not part of the name the arguments spell.
            if let Some(clause) = self.parse_requires_clause()? {
                if pending_decl(&arguments) {
                    return Err(self.error(ParseErrorKind::UnexpectedByte));
                }
                arguments.push(clause);
                continue;
            }
            arguments.push(self.parse_template_arg()?);
        }
        self.make(Kind::TemplateArgs, None, start, arguments)
    }

    fn parse_template_arg(&mut self) -> Result<NodeRef<'a>, ParseError> {
        self.enter()?;
        let result = self.parse_template_arg_inner();
        self.leave();
        result
    }

    fn parse_template_arg_inner(&mut self) -> Result<NodeRef<'a>, ParseError> {
        self.tick()?;
        match self.peek() {
            Some(b'J') => {
                let start = self.input.pos();
                self.bump(1)?;
                self.tick()?;
                let mut elements = Vec::new();
                loop {
                    self.tick()?;
                    if self.eat(b'E') {
                        break;
                    }
                    if self.peek().is_none() {
                        return Err(self.error(ParseErrorKind::UnexpectedEnd));
                    }
                    elements.push(self.parse_template_arg()?);
                }
                self.make(Kind::Pack, Some(Cow::Borrowed("J")), start, elements)
            }
            Some(b'L') => self.parse_literal(),
            Some(b'X') => self.parse_expression_template_arg(),
            Some(b'T') if is_template_param_decl(Some(b'T'), self.peek_at(1)) => {
                self.parse_template_param_decl()
            }
            Some(_) => self.parse_type(),
            None => Err(self.error(ParseErrorKind::UnexpectedEnd)),
        }
    }

    /// Parse a `<template-param-decl>`: `Ty`, `Tn <type>`, `Tk <name>
    /// [<template-args>]`, `Tt <decl>* E` or `Tp <decl>`.
    fn parse_template_param_decl(&mut self) -> Result<NodeRef<'a>, ParseError> {
        let start = self.input.pos();
        self.expect(b'T')?;
        self.tick()?;
        let keyword = match self.peek() {
            Some(b'y') => {
                self.bump(1)?;
                self.tick()?;
                "Ty"
            }
            Some(b'n') => {
                self.bump(1)?;
                self.tick()?;
                let ty = self.parse_type()?;
                return self.make(
                    Kind::TemplateParamDecl,
                    Some(Cow::Borrowed("Tn")),
                    start,
                    alloc::vec![ty],
                );
            }
            Some(b'k') => {
                self.bump(1)?;
                self.tick()?;
                // Enclosing template parameter levels are not tracked inside a
                // constraint, so parameter references print their numbering.
                let saved = core::mem::replace(&mut self.incomplete_template_params, true);
                let constraint = self.parse_name();
                self.incomplete_template_params = saved;
                let mut children = alloc::vec![constraint?];
                if self.peek() == Some(b'I') {
                    children.push(self.parse_template_args()?);
                }
                return self.make(
                    Kind::TemplateParamDecl,
                    Some(Cow::Borrowed("Tk")),
                    start,
                    children,
                );
            }
            Some(b't') => {
                self.bump(1)?;
                self.tick()?;
                let mut children = Vec::new();
                while !self.eat(b'E') {
                    if self.peek().is_none() {
                        return Err(self.error(ParseErrorKind::UnexpectedEnd));
                    }
                    // The parameter list may carry its own requires-clause.
                    if let Some(clause) = self.parse_requires_clause()? {
                        children.push(clause);
                        continue;
                    }
                    children.push(self.parse_template_param_decl()?);
                }
                return self.make(
                    Kind::TemplateParamDecl,
                    Some(Cow::Borrowed("Tt")),
                    start,
                    children,
                );
            }
            Some(b'p') => {
                self.bump(1)?;
                self.tick()?;
                let decl = self.parse_template_param_decl()?;
                return self.make(
                    Kind::TemplateParamDecl,
                    Some(Cow::Borrowed("Tp")),
                    start,
                    alloc::vec![decl],
                );
            }
            _ => return Err(self.error(ParseErrorKind::Unsupported)),
        };
        self.make(
            Kind::TemplateParamDecl,
            Some(Cow::Borrowed(keyword)),
            start,
            Vec::new(),
        )
    }

    /// Parse `<expr-primary> ::= L <type> [<value>] E | L <mangled-name> E`.
    ///
    /// `L Z <encoding> E` names an unnamed local entity, which prints as the
    /// entity's own name.
    fn parse_literal(&mut self) -> Result<NodeRef<'a>, ParseError> {
        let start = self.input.pos();
        self.expect(b'L')?;
        if self.peek() == Some(b'Z') {
            self.bump(1)?;
            self.tick()?;
            self.nested_encoding += 1;
            let encoding = self.parse_encoding();
            self.nested_encoding -= 1;
            let encoding = encoding?;
            self.expect(b'E')?;
            return self.make(Kind::Literal, None, start, alloc::vec![encoding]);
        }
        if self.starts_with(b"Ul") {
            // `L <lambda type> E`: a closure expression.
            let lambda = self.parse_lambda()?;
            self.expect(b'E')?;
            return self.make(Kind::Literal, None, start, alloc::vec![lambda]);
        }
        if self.peek() == Some(b'_') {
            if self.peek_at(1) != Some(b'Z') {
                return Err(self.error(ParseErrorKind::UnexpectedByte));
            }
            self.bump(2)?;
            self.nested_encoding += 1;
            let encoding = self.parse_encoding();
            self.nested_encoding -= 1;
            let encoding = encoding?;
            self.expect(b'E')?;
            return self.make(Kind::Literal, None, start, alloc::vec![encoding]);
        }
        let ty = self.parse_type()?;
        let value_start = self.input.pos();
        while self.peek().is_some_and(|byte| byte != b'E') {
            self.bump(1)?;
            self.tick()?;
        }
        let value_end = self.input.pos();
        self.expect(b'E')?;
        let text = if value_start == value_end {
            None
        } else {
            let value = self.consumed(value_start, value_end);
            Some(decode(value))
        };
        self.make(Kind::Literal, text, start, alloc::vec![ty])
    }

    /// Parse `X <expression> E`, an expression used as a template argument.
    ///
    /// The `X`/`E` pair is only a delimiter, so the expression node is returned
    /// directly.
    fn parse_expression_template_arg(&mut self) -> Result<NodeRef<'a>, ParseError> {
        self.expect(b'X')?;
        let expression = self.parse_expression()?;
        self.expect(b'E')?;
        Ok(expression)
    }

    fn parse_expression(&mut self) -> Result<NodeRef<'a>, ParseError> {
        self.enter()?;
        let result = self.parse_expression_inner();
        self.leave();
        result
    }

    fn parse_expression_inner(&mut self) -> Result<NodeRef<'a>, ParseError> {
        self.tick()?;
        let start = self.input.pos();
        // A leading `gs` puts the name that follows in the global namespace.
        let global = self.starts_with(b"gs");
        if global {
            self.bump(2)?;
            self.tick()?;
        }
        if self.starts_with(b"pp_") || self.starts_with(b"mm_") {
            // The `_` suffix selects the prefix form; plain `pp`/`mm` are
            // postfix and go through the operator table.
            let spelling = if self.starts_with(b"pp_") {
                "prefix++"
            } else {
                "prefix--"
            };
            self.bump(3)?;
            self.tick()?;
            let operand = self.parse_expression()?;
            return self.make(
                Kind::Expression,
                Some(Cow::Borrowed(spelling)),
                start,
                alloc::vec![operand],
            );
        }
        if self.starts_with(b"rq") || self.starts_with(b"rQ") {
            return self.parse_requires_expression(start);
        }
        if self.starts_with(b"sy") {
            return self.parse_pack_index(start);
        }
        if self.starts_with(b"mc") {
            return self.parse_member_cast(start);
        }
        if self.starts_with(b"cp") {
            // A call through a parenthesized callee: `cp <callee> <arg>* E`.
            self.bump(2)?;
            self.tick()?;
            let mut operands = alloc::vec![self.parse_expression()?];
            while !self.eat(b'E') {
                if self.peek().is_none() {
                    return Err(self.error(ParseErrorKind::UnexpectedEnd));
                }
                operands.push(self.parse_expression()?);
            }
            return self.make(
                Kind::Expression,
                Some(Cow::Borrowed("call-parenthesized")),
                start,
                operands,
            );
        }
        let Some(first) = self.peek() else {
            return Err(self.error(ParseErrorKind::UnexpectedEnd));
        };
        let second = self.peek_at(1);
        if let Some(second) = second {
            if let Some(form) = expression_form(first, second) {
                return self.parse_expression_form(start, form, [first, second], global);
            }
            if let Some(spelling) = self.operator_spelling() {
                let arity = expression_arity(first, second);
                self.bump(2)?;
                self.tick()?;
                let mut operands = Vec::with_capacity(arity as usize);
                for _ in 0..arity {
                    operands.push(self.parse_expression()?);
                }
                return self.make(
                    Kind::Expression,
                    Some(Cow::Borrowed(spelling)),
                    start,
                    operands,
                );
            }
        }
        if first == b'T' && !matches!(second, Some(b's' | b'u' | b'e' | b'L')) {
            let parameter = self.parse_template_param()?;
            return self.make(
                Kind::Expression,
                Some(Cow::Borrowed("template-param")),
                start,
                alloc::vec![parameter],
            );
        }
        if first == b'f' {
            // `fp`/`fpT`/`fL<digit>` name a function parameter; the other `f`
            // codes start a fold expression.
            if second == Some(b'p') || (second == Some(b'L') && self.is_digit(2)) {
                let parameter = self.parse_function_param()?;
                return Ok(parameter);
            }
            return self.parse_fold_expression(start);
        }
        if first == b'u' {
            return self.parse_vendor_expression(start);
        }
        if first == b'L' {
            return self.parse_literal();
        }
        if matches!(first, b'0'..=b'9' | b'o' | b'd' | b's' | b'D') {
            // An unresolved name: `x`, `on<operator>`, `dn<dtor>`, `sr...`.
            return self.parse_unresolved_name(global);
        }
        Err(self.error(ParseErrorKind::Unsupported))
    }

    fn is_digit(&self, offset: usize) -> bool {
        matches!(self.peek_at(offset), Some(b'0'..=b'9'))
    }

    /// Parse `<function-param>`: `fpT`, `fp <cv> _`, `fp <cv> <number> _`,
    /// `fL <number> p <cv> _` and `fL <number> p <cv> <number> _`.
    ///
    /// The parameter prints as `this` for `fpT` and as `fp` followed by the
    /// encoded index otherwise.
    fn parse_function_param(&mut self) -> Result<NodeRef<'a>, ParseError> {
        let start = self.input.pos();
        self.expect(b'f')?;
        self.tick()?;
        if self.eat(b'p') {
            self.parse_cv_qualifiers();
            if self.eat(b'T') {
                return self.make(
                    Kind::Identifier,
                    Some(Cow::Borrowed("this")),
                    start,
                    Vec::new(),
                );
            }
        } else {
            self.expect(b'L')?;
            let _level = self.parse_number()?;
            self.expect(b'p')?;
            self.parse_cv_qualifiers();
        }
        let index = if matches!(self.peek(), Some(b'0'..=b'9')) {
            Some(self.parse_digits()?)
        } else {
            None
        };
        self.expect(b'_')?;
        let text = index.map_or(Cow::Borrowed("fp"), |digits| {
            Cow::Owned(alloc::format!("fp{}", decode(digits)))
        });
        self.make(Kind::Identifier, Some(text), start, Vec::new())
    }

    /// Consume `<CV-qualifiers>` without recording them.
    fn parse_cv_qualifiers(&mut self) {
        let _ = self.eat(b'r');
        let _ = self.eat(b'V');
        let _ = self.eat(b'K');
    }

    /// Parse `rq <requirement>+ E` or `rQ <type>* _ <requirement>+ E`.
    ///
    /// The `rq` form is a requires-expression without a parameter list; the
    /// `rQ` form carries the parameter types before a `_` terminator.
    fn parse_requires_expression(&mut self, start: usize) -> Result<NodeRef<'a>, ParseError> {
        let with_parameters = self.peek_at(1) == Some(b'Q');
        self.bump(2)?;
        self.tick()?;
        let mut children = Vec::new();
        if with_parameters {
            let list_start = self.input.pos();
            let mut parameters = Vec::new();
            while !self.eat(b'_') {
                if self.peek().is_none() {
                    return Err(self.error(ParseErrorKind::UnexpectedEnd));
                }
                let ty = self.parse_type()?;
                parameters.push(self.make_parameter(ty)?);
            }
            children.push(self.make(Kind::ParameterList, None, list_start, parameters)?);
        }
        while !self.eat(b'E') {
            if self.peek().is_none() {
                return Err(self.error(ParseErrorKind::UnexpectedEnd));
            }
            children.push(self.parse_requirement()?);
        }
        self.make(Kind::RequiresExpression, None, start, children)
    }

    fn parse_requirement(&mut self) -> Result<NodeRef<'a>, ParseError> {
        self.enter()?;
        let result = self.parse_requirement_inner();
        self.leave();
        result
    }

    /// Parse one `<requirement>`: `X <expression> [N] [R <type>]`,
    /// `T <type>` or `Q <expression>`.
    fn parse_requirement_inner(&mut self) -> Result<NodeRef<'a>, ParseError> {
        self.tick()?;
        let start = self.input.pos();
        match self.peek() {
            Some(b'T') => {
                self.bump(1)?;
                self.tick()?;
                let ty = self.parse_type()?;
                self.make(
                    Kind::Requirement,
                    Some(Cow::Borrowed("typename")),
                    start,
                    alloc::vec![ty],
                )
            }
            Some(b'Q') => {
                self.bump(1)?;
                self.tick()?;
                let constraint = self.parse_expression()?;
                self.make(
                    Kind::Requirement,
                    Some(Cow::Borrowed("requires")),
                    start,
                    alloc::vec![constraint],
                )
            }
            _ => {
                // The requirement's expression comes first; a trailing `N`
                // marks it `noexcept` and a trailing `R` adds a return-type
                // constraint.
                self.expect(b'X')?;
                let expression = self.parse_expression()?;
                let noexcept = self.eat(b'N');
                if noexcept {
                    self.tick()?;
                }
                let return_type = if self.eat(b'R') {
                    self.tick()?;
                    // The operand is a `<type-constraint>`, a name rather than a
                    // full type, so it takes the substitution slots a name takes.
                    Some(self.parse_name()?)
                } else {
                    None
                };
                let text = match (noexcept, return_type.is_some()) {
                    (false, false) => "simple",
                    (false, true) => "compound",
                    (true, _) => "compound noexcept",
                };
                let mut children = alloc::vec![expression];
                children.extend(return_type);
                self.make(
                    Kind::Requirement,
                    Some(Cow::Borrowed(text)),
                    start,
                    children,
                )
            }
        }
    }

    /// Parse `sy <pack> <index>`, a C++26 pack subscript expression.
    fn parse_pack_index(&mut self, start: usize) -> Result<NodeRef<'a>, ParseError> {
        self.bump(2)?;
        self.tick()?;
        let pack = self.parse_pack_index_operand()?;
        let index = self.parse_expression()?;
        self.make(
            Kind::Expression,
            Some(Cow::Borrowed("pack-index")),
            start,
            alloc::vec![pack, index],
        )
    }

    /// Parse the pack operand of a pack subscript.
    ///
    /// A template parameter is kept as a [`Kind::TemplateParam`] node so that
    /// the printer can resolve it to the argument pack it names.
    fn parse_pack_index_operand(&mut self) -> Result<NodeRef<'a>, ParseError> {
        if self.peek() == Some(b'T') && !matches!(self.peek_at(1), Some(b's' | b'u' | b'e' | b'L'))
        {
            return self.parse_template_param();
        }
        self.parse_expression()
    }

    /// Parse `mc <type> <expression> <offset> E`.
    ///
    /// The production encodes a pointer-to-member conversion: the type is the
    /// target member-pointer type, the expression the converted pointer and the
    /// offset the base-class adjustment the conversion applies. The reference
    /// demangler prints only the cast and drops the adjustment.
    fn parse_member_cast(&mut self, start: usize) -> Result<NodeRef<'a>, ParseError> {
        self.bump(2)?;
        self.tick()?;
        let ty = self.parse_type()?;
        let expression = self.parse_expression()?;
        let _offset = self.parse_offset()?;
        self.expect(b'E')?;
        self.make(
            Kind::Expression,
            Some(Cow::Borrowed("member-cast")),
            start,
            alloc::vec![ty, expression],
        )
    }

    /// Parse `fL`/`fR`/`fl`/`fr <binary-operator> <expression>*`.
    fn parse_fold_expression(&mut self, start: usize) -> Result<NodeRef<'a>, ParseError> {
        self.expect(b'f')?;
        self.tick()?;
        let Some(mode @ (b'L' | b'R' | b'l' | b'r')) = self.peek() else {
            return Err(self.error(ParseErrorKind::Unsupported));
        };
        self.bump(1)?;
        self.tick()?;
        let Some((first, second)) = self.peek().zip(self.peek_at(1)) else {
            return Err(self.error(ParseErrorKind::UnexpectedEnd));
        };
        let Some(spelling) = self.operator_spelling() else {
            return Err(self.error(ParseErrorKind::Unsupported));
        };
        // A fold takes a binary operator or a pointer-to-member operator.
        let member_operator = matches!((first, second), (b'd', b's') | (b'p', b'm'));
        if (!matches!(expression_arity(first, second), 2)
            || expression_form(first, second).is_some())
            && !member_operator
        {
            return Err(self.error(ParseErrorKind::Unsupported));
        }
        let operator_start = self.input.pos();
        self.bump(2)?;
        self.tick()?;
        let operator = self.make(
            Kind::OperatorName,
            Some(Cow::Borrowed(spelling)),
            operator_start,
            Vec::new(),
        )?;
        let mut pack = self.parse_expression()?;
        let mut init = if matches!(mode, b'L' | b'R') {
            Some(self.parse_expression()?)
        } else {
            None
        };
        // A left fold encodes the initializer before the pack.
        if matches!(mode, b'L')
            && let Some(initializer) = init.take()
        {
            init = Some(core::mem::replace(&mut pack, initializer));
        }
        let text = if matches!(mode, b'L' | b'l') {
            "fold-left"
        } else {
            "fold-right"
        };
        let mut children = alloc::vec![operator, pack];
        children.extend(init);
        self.make(Kind::Expression, Some(Cow::Borrowed(text)), start, children)
    }

    /// Parse `u <source-name> <template-arg>* E`, a vendor extended expression.
    ///
    /// `__uuidof` is the one form whose operand is encoded as a bare type
    /// (`t <type>`) or expression (`z <expression>`) instead of a template
    /// argument list.
    fn parse_vendor_expression(&mut self, start: usize) -> Result<NodeRef<'a>, ParseError> {
        self.expect(b'u')?;
        self.tick()?;
        let name = self.parse_source_name()?;
        let mut children = alloc::vec![name];
        if children[0].text() == Some("__uuidof") {
            if self.eat(b't') {
                self.tick()?;
                children.push(self.parse_type()?);
                return self.make(
                    Kind::Expression,
                    Some(Cow::Borrowed("call")),
                    start,
                    children,
                );
            }
            if self.eat(b'z') {
                self.tick()?;
                children.push(self.parse_expression()?);
                return self.make(
                    Kind::Expression,
                    Some(Cow::Borrowed("call")),
                    start,
                    children,
                );
            }
        }
        while !self.eat(b'E') {
            if self.peek().is_none() {
                return Err(self.error(ParseErrorKind::UnexpectedEnd));
            }
            children.push(self.parse_template_arg()?);
        }
        self.make(
            Kind::Expression,
            Some(Cow::Borrowed("call")),
            start,
            children,
        )
    }

    #[expect(
        clippy::too_many_lines,
        reason = "one match arm per grammar production"
    )]
    fn parse_expression_form(
        &mut self,
        start: usize,
        form: ExpressionForm,
        op: [u8; 2],
        global: bool,
    ) -> Result<NodeRef<'a>, ParseError> {
        self.bump(2)?;
        self.tick()?;
        let mut operands = Vec::new();
        match form {
            ExpressionForm::Rethrow => {}
            ExpressionForm::Call => {
                if self.peek().is_none() {
                    return Err(self.error(ParseErrorKind::UnexpectedEnd));
                }
                while !self.eat(b'E') {
                    if self.peek().is_none() {
                        return Err(self.error(ParseErrorKind::UnexpectedEnd));
                    }
                    operands.push(self.parse_expression()?);
                }
            }
            ExpressionForm::Conversion => {
                operands.push(self.parse_type()?);
                if self.eat(b'_') {
                    while !self.eat(b'E') {
                        if self.peek().is_none() {
                            return Err(self.error(ParseErrorKind::UnexpectedEnd));
                        }
                        operands.push(self.parse_expression()?);
                    }
                } else {
                    operands.push(self.parse_expression()?);
                }
            }
            ExpressionForm::TypeList => {
                operands.push(self.parse_type()?);
                while !self.eat(b'E') {
                    if self.peek().is_none() {
                        return Err(self.error(ParseErrorKind::UnexpectedEnd));
                    }
                    operands.push(self.parse_braced_expression()?);
                }
            }
            ExpressionForm::InitializerList => {
                while !self.eat(b'E') {
                    if self.peek().is_none() {
                        return Err(self.error(ParseErrorKind::UnexpectedEnd));
                    }
                    operands.push(self.parse_braced_expression()?);
                }
            }
            ExpressionForm::MemberAccess
            | ExpressionForm::PointerMemberAccess
            | ExpressionForm::MemberPointer
            | ExpressionForm::PointerToMember => {
                operands.push(self.parse_expression()?);
                operands.push(self.parse_expression()?);
            }
            ExpressionForm::SizeofType
            | ExpressionForm::AlignofType
            | ExpressionForm::TypeidType => {
                operands.push(self.parse_type()?);
            }
            ExpressionForm::SizeofExpr
            | ExpressionForm::AlignofExpr
            | ExpressionForm::Noexcept
            // `tw <expression>`: the thrown operand is an expression.
            | ExpressionForm::Throw
            | ExpressionForm::TypeidExpr
            | ExpressionForm::PackExpansion => {
                operands.push(self.parse_expression()?);
            }
            ExpressionForm::Cast => {
                operands.push(self.parse_type()?);
                operands.push(self.parse_expression()?);
            }
            ExpressionForm::SizeofPack => {
                if self.peek() == Some(b'T') {
                    operands.push(self.parse_template_param()?);
                } else {
                    operands.push(self.parse_function_param()?);
                }
            }
            ExpressionForm::SizeofPackList => {
                while !self.eat(b'E') {
                    if self.peek().is_none() {
                        return Err(self.error(ParseErrorKind::UnexpectedEnd));
                    }
                    operands.push(self.parse_template_arg()?);
                }
            }
            ExpressionForm::Subobject => {
                let ty = self.parse_type()?;
                let expression = self.parse_expression()?;
                let offset = self.parse_offset()?;
                while self.eat(b'_') {
                    if matches!(self.peek(), Some(b'0'..=b'9')) {
                        let _ = self.parse_number()?;
                    }
                }
                let _ = self.eat(b'p');
                self.expect(b'E')?;
                let text = Cow::Owned(offset);
                return self.make(
                    Kind::Subobject,
                    Some(text),
                    start,
                    alloc::vec![ty, expression],
                );
            }
            ExpressionForm::New { array } => {
                let placement = self.parse_expression_list(b'_')?;
                let ty = self.parse_type()?;
                let inits = if self.starts_with(b"pi") {
                    self.bump(2)?;
                    self.tick()?;
                    self.parse_expression_list(b'E')?
                } else {
                    self.expect(b'E')?;
                    self.make(Kind::ParameterList, None, self.input.pos(), Vec::new())?
                };
                let text = new_expression_text("new", array, global);
                return self.make(
                    Kind::Expression,
                    Some(text),
                    start,
                    alloc::vec![placement, ty, inits],
                );
            }
            ExpressionForm::Delete { array } => {
                let operand = self.parse_expression()?;
                let text = new_expression_text("delete", array, global);
                return self.make(Kind::Expression, Some(text), start, alloc::vec![operand]);
            }
        }
        let text = match form {
            ExpressionForm::Cast => match op {
                [b'd', b'c'] => "dynamic_cast",
                [b's', b'c'] => "static_cast",
                [b'c', b'c'] => "const_cast",
                [b'r', b'c'] => "reinterpret_cast",
                _ => form.text(),
            },
            _ => form.text(),
        };
        self.make(Kind::Expression, Some(Cow::Borrowed(text)), start, operands)
    }

    /// Parse a `terminator`-delimited list of expressions.
    fn parse_expression_list(&mut self, terminator: u8) -> Result<NodeRef<'a>, ParseError> {
        let start = self.input.pos();
        let mut operands = Vec::new();
        while !self.eat(terminator) {
            if self.peek().is_none() {
                return Err(self.error(ParseErrorKind::UnexpectedEnd));
            }
            operands.push(self.parse_expression()?);
        }
        self.make(Kind::ParameterList, None, start, operands)
    }

    /// Parse a subobject's byte offset and render it.
    fn parse_offset(&mut self) -> Result<alloc::string::String, ParseError> {
        let negative = self.eat(b'n');
        let start = self.input.pos();
        if !matches!(self.peek(), Some(b'0'..=b'9')) {
            return Ok(alloc::string::String::from("0"));
        }
        let _ = self.parse_number()?;
        let digits = self.consumed(start, self.input.pos());
        let digits = decode(digits);
        Ok(if negative {
            alloc::format!("-{digits}")
        } else {
            alloc::string::String::from(digits.as_ref())
        })
    }

    /// Parse a `<braced-expression>`: an expression or a designated member
    /// initializer.
    fn parse_braced_expression(&mut self) -> Result<NodeRef<'a>, ParseError> {
        self.enter()?;
        let result = self.parse_braced_expression_inner();
        self.leave();
        result
    }

    fn parse_braced_expression_inner(&mut self) -> Result<NodeRef<'a>, ParseError> {
        if self.peek() == Some(b'd') {
            let start = self.input.pos();
            match self.peek_at(1) {
                Some(b'i') => {
                    self.bump(2)?;
                    self.tick()?;
                    let field = self.parse_source_name()?;
                    let init = self.parse_braced_expression()?;
                    return self.make(
                        Kind::Expression,
                        Some(Cow::Borrowed("braced-field")),
                        start,
                        alloc::vec![field, init],
                    );
                }
                Some(b'x') => {
                    self.bump(2)?;
                    self.tick()?;
                    let index = self.parse_expression()?;
                    let init = self.parse_braced_expression()?;
                    return self.make(
                        Kind::Expression,
                        Some(Cow::Borrowed("braced-index")),
                        start,
                        alloc::vec![index, init],
                    );
                }
                Some(b'X') => {
                    self.bump(2)?;
                    self.tick()?;
                    let first = self.parse_expression()?;
                    let last = self.parse_expression()?;
                    let init = self.parse_braced_expression()?;
                    return self.make(
                        Kind::Expression,
                        Some(Cow::Borrowed("braced-range")),
                        start,
                        alloc::vec![first, last, init],
                    );
                }
                _ => {}
            }
        }
        self.parse_expression()
    }

    // -- unresolved names --------------------------------------------------

    /// Parse an `<unresolved-name>`: a name that is not resolved through the
    /// substitution table.
    ///
    /// `global` records a `gs` prefix, which only the `sr <level>+ E` form and
    /// the bare base form print.
    fn parse_unresolved_name(&mut self, global: bool) -> Result<NodeRef<'a>, ParseError> {
        let start = self.input.pos();
        if self.starts_with(b"srN") {
            self.bump(3)?;
            self.tick()?;
            let mut so_far = self.parse_unresolved_type()?;
            if self.peek() == Some(b'I') {
                let arguments = self.parse_template_args()?;
                so_far = self.make(Kind::Name, None, start, alloc::vec![so_far, arguments])?;
            }
            while !self.eat(b'E') {
                if self.peek().is_none() {
                    return Err(self.error(ParseErrorKind::UnexpectedEnd));
                }
                let level = self.parse_simple_id()?;
                so_far = self.make(Kind::NestedName, None, start, alloc::vec![so_far, level])?;
            }
            let base = self.parse_base_unresolved_name()?;
            return self.make(Kind::NestedName, None, start, alloc::vec![so_far, base]);
        }
        if !self.starts_with(b"sr") {
            let base = self.parse_base_unresolved_name()?;
            if global {
                return self.make(Kind::GlobalName, None, start, alloc::vec![base]);
            }
            return Ok(base);
        }
        self.bump(2)?;
        self.tick()?;
        let so_far = if matches!(self.peek(), Some(b'0'..=b'9')) {
            let mut so_far: Option<NodeRef<'a>> = None;
            loop {
                let level = self.parse_simple_id()?;
                so_far = Some(match so_far {
                    Some(prefix) => {
                        self.make(Kind::NestedName, None, start, alloc::vec![prefix, level])?
                    }
                    None if global => {
                        self.make(Kind::GlobalName, None, start, alloc::vec![level])?
                    }
                    None => level,
                });
                if self.eat(b'E') {
                    break;
                }
                if self.peek().is_none() {
                    return Err(self.error(ParseErrorKind::UnexpectedEnd));
                }
            }
            so_far.ok_or_else(|| self.error(ParseErrorKind::UnexpectedEnd))?
        } else {
            let mut so_far = self.parse_unresolved_type()?;
            if self.peek() == Some(b'I') {
                let arguments = self.parse_template_args()?;
                so_far = self.make(Kind::Name, None, start, alloc::vec![so_far, arguments])?;
            }
            so_far
        };
        let base = self.parse_base_unresolved_name()?;
        self.make(Kind::NestedName, None, start, alloc::vec![so_far, base])
    }

    /// Parse `<unresolved-type>`: a template parameter, a `decltype` or a
    /// substitution.
    fn parse_unresolved_type(&mut self) -> Result<NodeRef<'a>, ParseError> {
        match self.peek() {
            Some(b'T') => {
                let parameter = self.parse_template_param()?;
                self.push_sub(Rc::clone(&parameter));
                Ok(parameter)
            }
            Some(b'D') if matches!(self.peek_at(1), Some(b't' | b'T')) => {
                let start = self.input.pos();
                self.bump(2)?;
                self.tick()?;
                let expression = self.parse_expression()?;
                self.expect(b'E')?;
                let node = self.make(Kind::Decltype, None, start, alloc::vec![expression])?;
                self.push_sub(Rc::clone(&node));
                Ok(node)
            }
            Some(b'S') => self.parse_substitution(),
            _ => Err(self.error(ParseErrorKind::Unsupported)),
        }
    }

    /// Parse a `<simple-id>`: a source name with optional template arguments.
    fn parse_simple_id(&mut self) -> Result<NodeRef<'a>, ParseError> {
        let start = self.input.pos();
        let identifier = self.parse_source_name()?;
        let base = self.make(Kind::UnqualifiedName, None, start, alloc::vec![identifier])?;
        if self.peek() == Some(b'I') {
            let arguments = self.parse_template_args()?;
            return self.make(Kind::Name, None, start, alloc::vec![base, arguments]);
        }
        Ok(base)
    }

    /// Parse a `<base-unresolved-name>`: a simple-id, a destructor name or an
    /// operator-function-id.
    fn parse_base_unresolved_name(&mut self) -> Result<NodeRef<'a>, ParseError> {
        let start = self.input.pos();
        if matches!(self.peek(), Some(b'0'..=b'9')) {
            return self.parse_simple_id();
        }
        if self.starts_with(b"dn") {
            self.bump(2)?;
            self.tick()?;
            let target = if matches!(self.peek(), Some(b'0'..=b'9')) {
                self.parse_simple_id()?
            } else {
                self.parse_unresolved_type()?
            };
            return self.make(Kind::DestructorName, None, start, alloc::vec![target]);
        }
        if self.starts_with(b"on") {
            self.bump(2)?;
            self.tick()?;
        }
        let operator = self.parse_unqualified_name_leaf()?;
        if self.peek() == Some(b'I') {
            let arguments = self.parse_template_args()?;
            return self.make(Kind::Name, None, start, alloc::vec![operator, arguments]);
        }
        Ok(operator)
    }

    // -- types -------------------------------------------------------------

    fn parse_type(&mut self) -> Result<NodeRef<'a>, ParseError> {
        self.enter()?;
        let result = self.parse_type_inner();
        self.leave();
        result
    }

    #[expect(
        clippy::too_many_lines,
        reason = "one match arm per grammar production"
    )]
    fn parse_type_inner(&mut self) -> Result<NodeRef<'a>, ParseError> {
        self.tick()?;
        let start = self.input.pos();
        let Some(byte) = self.peek() else {
            return Err(self.error(ParseErrorKind::UnexpectedEnd));
        };
        match byte {
            b'v' | b'w' | b'b' | b'c' | b'a' | b'h' | b's' | b't' | b'i' | b'j' | b'l' | b'm'
            | b'x' | b'y' | b'n' | b'o' | b'f' | b'd' | b'e' | b'g' | b'z' => {
                let name =
                    builtin_name(byte).ok_or_else(|| self.error(ParseErrorKind::Unsupported))?;
                self.bump(1)?;
                self.tick()?;
                self.make(
                    Kind::BuiltinType,
                    Some(Cow::Borrowed(name)),
                    start,
                    Vec::new(),
                )
            }
            b'D' if matches!(self.peek_at(1), Some(b't' | b'T')) => {
                self.bump(2)?;
                self.tick()?;
                let expression = self.parse_expression()?;
                self.expect(b'E')?;
                let node = self.make(Kind::Decltype, None, start, alloc::vec![expression])?;
                self.push_sub(Rc::clone(&node));
                Ok(node)
            }
            b'D' if matches!(self.peek_at(1), Some(b'o' | b'O' | b'w' | b'x')) => {
                self.parse_function_type(start)
            }
            b'D' if self.peek_at(1) == Some(b'y') => {
                // `Dy <pack> <index>`: the type of a C++26 pack subscript.
                self.bump(2)?;
                self.tick()?;
                let pack = self.parse_type()?;
                let index = self.parse_expression()?;
                let node = self.make(Kind::PackIndexType, None, start, alloc::vec![pack, index])?;
                self.push_sub(Rc::clone(&node));
                Ok(node)
            }
            b'D' => self.parse_d_type(start),
            b'u' => {
                self.bump(1)?;
                self.tick()?;
                let identifier = self.parse_source_name()?;
                let vendor = self.make(Kind::VendorType, None, start, alloc::vec![identifier])?;
                if self.peek() == Some(b'I') {
                    let arguments = self.parse_template_args()?;
                    let node =
                        self.make(Kind::Name, None, start, alloc::vec![vendor, arguments])?;
                    self.push_sub(Rc::clone(&node));
                    Ok(node)
                } else {
                    self.push_sub(Rc::clone(&vendor));
                    Ok(vendor)
                }
            }
            b'P' => {
                self.bump(1)?;
                self.tick()?;
                let inner = self.parse_type()?;
                let node = self.make(Kind::PointerType, None, start, alloc::vec![inner])?;
                self.push_sub(Rc::clone(&node));
                Ok(node)
            }
            b'R' => {
                self.bump(1)?;
                self.tick()?;
                let inner = self.parse_type()?;
                let node = self.make(Kind::ReferenceType, None, start, alloc::vec![inner])?;
                self.push_sub(Rc::clone(&node));
                Ok(node)
            }
            b'O' => {
                self.bump(1)?;
                self.tick()?;
                let inner = self.parse_type()?;
                let node = self.make(Kind::RvalueReferenceType, None, start, alloc::vec![inner])?;
                self.push_sub(Rc::clone(&node));
                Ok(node)
            }
            b'K' | b'V' | b'r' => {
                if self.qualifiers_start_function_type() {
                    self.parse_qualified_function_type(start)
                } else {
                    self.parse_qualified_type(start)
                }
            }
            b'U' if matches!(self.peek_at(1), Some(b'0'..=b'9')) => {
                let node = self.parse_vendor_extended_qualifier(start)?;
                self.push_sub(Rc::clone(&node));
                Ok(node)
            }
            b'U' => {
                // `Ul`/`Ut`/`Ub`: a lambda or unnamed type name.
                let node = self.parse_name()?;
                self.push_sub(Rc::clone(&node));
                Ok(node)
            }
            b'A' => self.parse_array_type(start),
            b'F' => self.parse_function_type(start),
            b'M' => self.parse_member_pointer(start),
            b'T' => {
                if matches!(self.peek_at(1), Some(b's' | b'u' | b'e')) {
                    // `Ts`/`Tu`/`Te <name>`: an elaborated type specifier.
                    let tag = match self.peek_at(1) {
                        Some(b'u') => "union",
                        Some(b'e') => "enum",
                        _ => "struct",
                    };
                    self.bump(2)?;
                    self.tick()?;
                    let name = self.parse_name()?;
                    let node = self.make(
                        Kind::ElaboratedType,
                        Some(Cow::Borrowed(tag)),
                        start,
                        alloc::vec![name],
                    )?;
                    self.push_sub(Rc::clone(&node));
                    return Ok(node);
                }
                let base = self.parse_template_param()?;
                if self.try_parse_template_args && self.peek() == Some(b'I') {
                    self.push_sub(Rc::clone(&base));
                    let arguments = self.parse_template_args()?;
                    let node = self.make(Kind::Name, None, start, alloc::vec![base, arguments])?;
                    self.push_sub(Rc::clone(&node));
                    Ok(node)
                } else {
                    self.push_sub(Rc::clone(&base));
                    Ok(base)
                }
            }
            b'S' => {
                if self.peek_at(1) == Some(b't') {
                    let node = self.parse_name()?;
                    self.push_sub(Rc::clone(&node));
                    Ok(node)
                } else {
                    let substitution = self.parse_substitution()?;
                    let module = is_module_name(&substitution);
                    let base = if module {
                        self.combine_module_entity(substitution, start)?
                    } else {
                        substitution
                    };
                    if self.peek() == Some(b'I') && (module || self.try_parse_template_args) {
                        let arguments = self.parse_template_args()?;
                        let node =
                            self.make(Kind::Name, None, start, alloc::vec![base, arguments])?;
                        self.push_sub(Rc::clone(&node));
                        Ok(node)
                    } else {
                        Ok(base)
                    }
                }
            }
            b'N' | b'Z' | b'L' | b'W' | b'0'..=b'9' => {
                let node = self.parse_name()?;
                self.push_sub(Rc::clone(&node));
                Ok(node)
            }
            _ => Err(self.error(ParseErrorKind::Unsupported)),
        }
    }

    #[expect(
        clippy::too_many_lines,
        reason = "one match arm per grammar production"
    )]
    fn parse_d_type(&mut self, start: usize) -> Result<NodeRef<'a>, ParseError> {
        self.expect(b'D')?;
        let Some(byte) = self.peek() else {
            return Err(self.error(ParseErrorKind::UnexpectedEnd));
        };
        match byte {
            b's' => self.builtin_d(start, "char16_t"),
            b'i' => self.builtin_d(start, "char32_t"),
            b'u' => self.builtin_d(start, "char8_t"),
            b'n' => self.builtin_d(start, "std::nullptr_t"),
            b'a' => self.builtin_d(start, "auto"),
            b'c' => self.builtin_d(start, "decltype(auto)"),
            b'h' => self.builtin_d(start, "half"),
            b'd' => self.builtin_d(start, "decimal64"),
            b'e' => self.builtin_d(start, "decimal128"),
            b'f' => self.builtin_d(start, "decimal32"),
            b'F' => {
                self.bump(1)?;
                self.tick()?;
                let digits_start = self.input.pos();
                let _width = self.parse_number()?;
                let digits_end = self.input.pos();
                let digits = self.consumed(digits_start, digits_end);
                let text = if self.eat(b'b') {
                    Cow::Borrowed("std::bfloat16_t")
                } else {
                    self.expect(b'_')?;
                    Cow::Owned(alloc::format!("_Float{}", decode(digits)))
                };
                let node = self.make(Kind::BuiltinType, Some(text), start, Vec::new())?;
                // libc++abi records parametrized binary-float builtins in the
                // substitution table; follow it so `S<n>_` indices match.
                self.push_sub(Rc::clone(&node));
                Ok(node)
            }
            b'B' | b'U' => {
                self.bump(1)?;
                self.tick()?;
                let unsigned = byte == b'U' || self.eat(b'u');
                let name = if unsigned {
                    "unsigned _BitInt"
                } else {
                    "_BitInt"
                };
                if !matches!(self.peek(), Some(b'0'..=b'9')) {
                    // An instantiation-dependent size: `DB <expression> _`.
                    let size = self.parse_expression()?;
                    self.expect(b'_')?;
                    let node = self.make(
                        Kind::BuiltinType,
                        Some(Cow::Borrowed(name)),
                        start,
                        alloc::vec![size],
                    )?;
                    self.push_sub(Rc::clone(&node));
                    return Ok(node);
                }
                let digits_start = self.input.pos();
                let _bits = self.parse_number()?;
                let digits_end = self.input.pos();
                self.expect(b'_')?;
                let digits = self.consumed(digits_start, digits_end);
                let text = alloc::format!("{name}({})", decode(digits));
                let node =
                    self.make(Kind::BuiltinType, Some(Cow::Owned(text)), start, Vec::new())?;
                self.push_sub(Rc::clone(&node));
                Ok(node)
            }
            b'k' | b'K' => {
                // `Dk <type-constraint>`: a constrained `auto` placeholder.
                let suffix = if byte == b'k' {
                    " auto"
                } else {
                    " decltype(auto)"
                };
                self.bump(1)?;
                self.tick()?;
                let constraint = self.parse_name()?;
                let node = self.make(
                    Kind::PostfixQualifiedType,
                    Some(Cow::Borrowed(suffix)),
                    start,
                    alloc::vec![constraint],
                )?;
                self.push_sub(Rc::clone(&node));
                Ok(node)
            }
            b'v' => {
                self.bump(1)?;
                self.tick()?;
                let node = self.parse_vector_type(start)?;
                self.push_sub(Rc::clone(&node));
                Ok(node)
            }
            b'A' | b'R' => self.fixed_point_type(start, false),
            b'S' => {
                self.bump(1)?;
                self.tick()?;
                self.expect(b'D')?;
                self.fixed_point_type(start, true)
            }
            b'p' => {
                self.bump(1)?;
                self.tick()?;
                let pattern = self.parse_type()?;
                let node = self.make(
                    Kind::Pack,
                    Some(Cow::Borrowed("Dp")),
                    start,
                    alloc::vec![pattern],
                )?;
                self.push_sub(Rc::clone(&node));
                Ok(node)
            }
            _ => Err(self.error(ParseErrorKind::Unsupported)),
        }
    }

    /// Parse `Dv <dimension> _ <element>`, a vector type.
    ///
    /// The leading `Dv` is consumed by [`Self::parse_d_type`].
    fn parse_vector_type(&mut self, start: usize) -> Result<NodeRef<'a>, ParseError> {
        if self.eat(b'_') {
            let element = self.parse_type()?;
            return self.make(Kind::VectorType, None, start, alloc::vec![element]);
        }
        if matches!(self.peek(), Some(b'1'..=b'9')) {
            let digits = self.parse_digits()?;
            self.expect(b'_')?;
            let element = self.parse_type()?;
            return self.make(
                Kind::VectorType,
                Some(decode(digits)),
                start,
                alloc::vec![element],
            );
        }
        let dimension = self.parse_expression()?;
        self.expect(b'_')?;
        let element = self.parse_type()?;
        self.make(
            Kind::VectorType,
            None,
            start,
            alloc::vec![dimension, element],
        )
    }

    /// Parse `U <source-name> [<template-args>] <type>`, a vendor extended type
    /// qualifier.
    ///
    /// The qualifier's type prints first and the qualifier after it. A
    /// qualifier whose name starts with `objcproto` carries an Objective-C
    /// protocol name in the rest of the source name.
    fn parse_vendor_extended_qualifier(&mut self, start: usize) -> Result<NodeRef<'a>, ParseError> {
        self.enter()?;
        let result = self.parse_vendor_extended_qualifier_inner(start);
        self.leave();
        result
    }

    fn parse_vendor_extended_qualifier_inner(
        &mut self,
        start: usize,
    ) -> Result<NodeRef<'a>, ParseError> {
        self.expect(b'U')?;
        self.tick()?;
        let name = self.parse_source_name_text()?;
        if let Some(protocol) = name.strip_prefix(b"objcproto") {
            let protocol = decode_source_name(protocol)
                .ok_or_else(|| self.error(ParseErrorKind::InvalidLength))?;
            let inner = self.parse_vendor_extended_qualifier_target()?;
            return self.make(
                Kind::ObjCProtocolType,
                Some(decode(protocol)),
                start,
                alloc::vec![inner],
            );
        }
        let arguments = if self.peek() == Some(b'I') {
            Some(self.parse_template_args()?)
        } else {
            None
        };
        let inner = self.parse_vendor_extended_qualifier_target()?;
        let mut children = alloc::vec![inner];
        children.extend(arguments);
        self.make(
            Kind::VendorExtQualifier,
            Some(decode(name)),
            start,
            children,
        )
    }

    /// Parse the type a vendor extended qualifier applies to.
    ///
    /// The whole `U ...` chain records one substitution slot, taken by the
    /// enclosing type production, so the target is parsed without one.
    fn parse_vendor_extended_qualifier_target(&mut self) -> Result<NodeRef<'a>, ParseError> {
        match self.peek() {
            Some(b'K' | b'V' | b'r') => {
                let start = self.input.pos();
                if self.qualifiers_start_function_type() {
                    return self.parse_qualified_function_type(start);
                }
                self.parse_qualified_type_node(start)
            }
            Some(b'U') => self.parse_vendor_extended_qualifier(self.input.pos()),
            _ => self.parse_type(),
        }
    }

    /// Parse the `A`/`R` part of a `DA...`/`DR...` fixed-point type.
    ///
    /// The leading `D` is consumed by [`Self::parse_d_type`]; `DS` consumes its
    /// own `D` before calling this.
    fn fixed_point_type(
        &mut self,
        start: usize,
        saturated: bool,
    ) -> Result<NodeRef<'a>, ParseError> {
        let Some(kind_byte) = self.peek() else {
            return Err(self.error(ParseErrorKind::UnexpectedEnd));
        };
        let kind = match kind_byte {
            b'A' => "_Accum",
            b'R' => "_Fract",
            _ => return Err(self.error(ParseErrorKind::Unsupported)),
        };
        self.bump(1)?;
        self.tick()?;
        let Some(size_byte) = self.peek() else {
            return Err(self.error(ParseErrorKind::UnexpectedEnd));
        };
        let size = match size_byte {
            b's' => "short ",
            b't' => "unsigned short ",
            b'i' => "",
            b'j' => "unsigned ",
            b'l' => "long ",
            b'm' => "unsigned long ",
            _ => return Err(self.error(ParseErrorKind::Unsupported)),
        };
        self.bump(1)?;
        self.tick()?;
        let text = if saturated {
            alloc::format!("_Sat {size}{kind}")
        } else {
            alloc::format!("{size}{kind}")
        };
        self.make(Kind::BuiltinType, Some(Cow::Owned(text)), start, Vec::new())
    }

    fn builtin_d(&mut self, start: usize, name: &'static str) -> Result<NodeRef<'a>, ParseError> {
        self.bump(1)?;
        self.tick()?;
        self.make(
            Kind::BuiltinType,
            Some(Cow::Borrowed(name)),
            start,
            Vec::new(),
        )
    }

    fn parse_qualified_type(&mut self, start: usize) -> Result<NodeRef<'a>, ParseError> {
        let node = self.parse_qualified_type_node(start)?;
        self.push_sub(Rc::clone(&node));
        Ok(node)
    }

    /// Parse `<qualifiers> <type>` without taking a substitution slot.
    ///
    /// A vendor extended qualifier's target is parsed this way: the whole
    /// qualified chain takes the single slot the enclosing type production
    /// records.
    fn parse_qualified_type_node(&mut self, start: usize) -> Result<NodeRef<'a>, ParseError> {
        let mut restrict = false;
        let mut volatile = false;
        let mut const_ = false;
        if self.eat(b'r') {
            restrict = true;
        }
        if self.eat(b'V') {
            volatile = true;
        }
        if self.eat(b'K') {
            const_ = true;
        }
        if !restrict && !volatile && !const_ {
            return Err(self.error(ParseErrorKind::UnexpectedByte));
        }
        let inner = self.parse_type()?;
        let text = qualifier_text(const_, volatile, restrict);
        self.make(
            Kind::QualifiedType,
            Some(Cow::Borrowed(text)),
            start,
            alloc::vec![inner],
        )
    }

    /// True when the CV-qualifiers at the cursor belong to a function type.
    ///
    /// `K F...`, `V F...` and `r F...` are a cv-qualified function type, not a
    /// qualified wrapper around one, and the qualifiers may precede an
    /// exception specification (`K Do F...`). The two forms print the same but
    /// take one substitution slot in the reference demangler, so the parser
    /// must not count the inner function type separately.
    fn qualifiers_start_function_type(&self) -> bool {
        let mut offset = 0;
        if self.peek_at(offset) == Some(b'r') {
            offset += 1;
        }
        if self.peek_at(offset) == Some(b'V') {
            offset += 1;
        }
        if self.peek_at(offset) == Some(b'K') {
            offset += 1;
        }
        match self.peek_at(offset) {
            Some(b'F') => true,
            Some(b'D') => matches!(self.peek_at(offset + 1), Some(b'o' | b'O' | b'w' | b'x')),
            _ => false,
        }
    }

    fn parse_qualified_function_type(&mut self, start: usize) -> Result<NodeRef<'a>, ParseError> {
        let mut restrict = false;
        let mut volatile = false;
        let mut const_ = false;
        if self.eat(b'r') {
            restrict = true;
        }
        if self.eat(b'V') {
            volatile = true;
        }
        if self.eat(b'K') {
            const_ = true;
        }
        let function = self.parse_function_type_node(start)?;
        let text = qualifier_text(const_, volatile, restrict);
        let node = self.make(
            Kind::QualifiedType,
            Some(Cow::Borrowed(text)),
            start,
            alloc::vec![function],
        )?;
        self.push_sub(Rc::clone(&node));
        Ok(node)
    }

    fn parse_array_type(&mut self, start: usize) -> Result<NodeRef<'a>, ParseError> {
        self.expect(b'A')?;
        if self.eat(b'_') {
            let element = self.parse_type()?;
            let node = self.make(Kind::ArrayType, None, start, alloc::vec![element])?;
            self.push_sub(Rc::clone(&node));
            return Ok(node);
        }
        if matches!(self.peek(), Some(b'0'..=b'9')) {
            let digits = self.parse_digits()?;
            self.expect(b'_')?;
            let element = self.parse_type()?;
            let node = self.make(
                Kind::ArrayType,
                Some(decode(digits)),
                start,
                alloc::vec![element],
            )?;
            self.push_sub(Rc::clone(&node));
            return Ok(node);
        }
        // A dependent bound: `A <expression> _ <type>`.
        let bound = self.parse_expression()?;
        self.expect(b'_')?;
        let element = self.parse_type()?;
        let node = self.make(Kind::ArrayType, None, start, alloc::vec![bound, element])?;
        self.push_sub(Rc::clone(&node));
        Ok(node)
    }

    fn parse_function_type(&mut self, start: usize) -> Result<NodeRef<'a>, ParseError> {
        let node = self.parse_function_type_node(start)?;
        self.push_sub(Rc::clone(&node));
        Ok(node)
    }

    /// Parse the function type itself without adding it to the substitution
    /// table; the caller decides which node takes the slot.
    ///
    /// The production is `[<exception-spec>] [Dx] F [Y] <return> <params>
    /// [R|O] E`; the cv-qualifiers that may precede it are parsed by the
    /// caller.
    fn parse_function_type_node(&mut self, start: usize) -> Result<NodeRef<'a>, ParseError> {
        let exception_spec = self.parse_exception_spec(start)?;
        if self.starts_with(b"Dx") {
            // A transaction-safe function type.
            self.bump(2)?;
            self.tick()?;
        }
        self.expect(b'F')?;
        let extern_c = self.eat(b'Y');
        let return_type = self.parse_type()?;
        let parameters = self.parse_function_parameters()?;
        let ref_qualifier = if self.peek() == Some(b'R') && self.peek_at(1) == Some(b'E') {
            self.bump(1)?;
            self.tick()?;
            Some("&")
        } else if self.peek() == Some(b'O') && self.peek_at(1) == Some(b'E') {
            self.bump(1)?;
            self.tick()?;
            Some("&&")
        } else {
            None
        };
        self.expect(b'E')?;
        let text = match (extern_c, ref_qualifier) {
            (false, None) => None,
            (true, None) => Some(Cow::Borrowed("extern \"C\"")),
            (false, Some(qualifier)) => Some(Cow::Borrowed(qualifier)),
            (true, Some(qualifier)) => Some(Cow::Owned(alloc::format!("extern \"C\" {qualifier}"))),
        };
        let mut children = alloc::vec![return_type, parameters];
        children.extend(exception_spec);
        self.make(Kind::FunctionType, text, start, children)
    }

    /// Parse an optional function type exception specification.
    ///
    /// `Do` is a plain `noexcept`, `DO <expression> E` carries its operand and
    /// `Dw <type>* E` lists the thrown types.
    fn parse_exception_spec(&mut self, start: usize) -> Result<Option<NodeRef<'a>>, ParseError> {
        if self.starts_with(b"Do") {
            self.bump(2)?;
            self.tick()?;
            return Ok(Some(self.make(
                Kind::ExceptionSpec,
                Some(Cow::Borrowed("noexcept")),
                start,
                Vec::new(),
            )?));
        }
        if self.starts_with(b"DO") {
            self.bump(2)?;
            self.tick()?;
            let operand = self.parse_expression()?;
            self.expect(b'E')?;
            return Ok(Some(self.make(
                Kind::ExceptionSpec,
                Some(Cow::Borrowed("noexcept")),
                start,
                alloc::vec![operand],
            )?));
        }
        if self.starts_with(b"Dw") {
            self.bump(2)?;
            self.tick()?;
            let mut types = Vec::new();
            while !self.eat(b'E') {
                if self.peek().is_none() {
                    return Err(self.error(ParseErrorKind::UnexpectedEnd));
                }
                types.push(self.parse_type()?);
            }
            return Ok(Some(self.make(
                Kind::ExceptionSpec,
                Some(Cow::Borrowed("throw")),
                start,
                types,
            )?));
        }
        Ok(None)
    }

    fn parse_function_parameters(&mut self) -> Result<NodeRef<'a>, ParseError> {
        let start = self.input.pos();
        let mut parameters = Vec::new();
        if self.peek() == Some(b'v') && self.void_parameter_ends_function_type() {
            self.bump(1)?;
            self.tick()?;
        } else {
            while !self.at_function_type_end() {
                let ty = self.parse_type()?;
                parameters.push(self.make_parameter(ty)?);
            }
        }
        self.make(Kind::ParameterList, None, start, parameters)
    }

    fn void_parameter_ends_function_type(&self) -> bool {
        match self.peek_at(1) {
            Some(b'E') => true,
            Some(b'R' | b'O') => self.peek_at(2) == Some(b'E'),
            _ => false,
        }
    }

    fn at_function_type_end(&self) -> bool {
        self.peek().is_none()
            || self.peek() == Some(b'E')
            || (matches!(self.peek(), Some(b'R' | b'O')) && self.peek_at(1) == Some(b'E'))
    }

    fn parse_member_pointer(&mut self, start: usize) -> Result<NodeRef<'a>, ParseError> {
        self.expect(b'M')?;
        let class = self.parse_type()?;
        let member = self.parse_type()?;
        let node = self.make(
            Kind::MemberPointerType,
            None,
            start,
            alloc::vec![class, member],
        )?;
        self.push_sub(Rc::clone(&node));
        Ok(node)
    }

    fn parse_template_param(&mut self) -> Result<NodeRef<'a>, ParseError> {
        let start = self.input.pos();
        self.expect(b'T')?;
        if self.eat(b'L') {
            // `TL <level> _ <index> _`: the level is not modelled, only kept in
            // the text.
            self.tick()?;
            let _level = self.parse_number()?;
            self.expect(b'_')?;
        }
        if !self.eat(b'_') {
            let _number = self.parse_number()?;
            self.expect(b'_')?;
        }
        if self.incomplete_template_params {
            // Print the encoded numbering without its trailing `_`, matching
            // the reference demangler's placeholder for an untracked level.
            let text = self.consumed(start, self.input.pos() - 1);
            return self.make(Kind::Identifier, Some(decode(text)), start, Vec::new());
        }
        let text = self.consumed(start, self.input.pos());
        self.make(Kind::TemplateParam, Some(decode(text)), start, Vec::new())
    }

    fn parse_number(&mut self) -> Result<usize, ParseError> {
        let mut value = 0_usize;
        let mut any = false;
        while let Some(byte @ b'0'..=b'9') = self.peek() {
            let digit = usize::from(byte - b'0');
            value = value
                .checked_mul(10)
                .and_then(|current| current.checked_add(digit))
                .ok_or_else(|| self.error(ParseErrorKind::InvalidLength))?;
            self.bump(1)?;
            self.tick()?;
            any = true;
        }
        if any {
            Ok(value)
        } else {
            Err(self.error(ParseErrorKind::UnexpectedByte))
        }
    }

    /// Parse a decimal number and return its digits as written.
    fn parse_digits(&mut self) -> Result<&'a [u8], ParseError> {
        let start = self.input.pos();
        let _ = self.parse_number()?;
        Ok(self.consumed(start, self.input.pos()))
    }
}

/// Non-operator expression forms.
///
/// Each one has its own operand grammar; the generic operator table handles the
/// rest.
#[derive(Clone, Copy)]
enum ExpressionForm {
    Call,
    Conversion,
    TypeList,
    InitializerList,
    MemberAccess,
    PointerMemberAccess,
    MemberPointer,
    PointerToMember,
    SizeofType,
    SizeofExpr,
    AlignofType,
    AlignofExpr,
    Noexcept,
    Throw,
    Rethrow,
    TypeidType,
    TypeidExpr,
    Cast,
    PackExpansion,
    SizeofPack,
    SizeofPackList,
    Subobject,
    New { array: bool },
    Delete { array: bool },
}

impl ExpressionForm {
    /// The `text` of the [`Kind::Expression`] node this form produces.
    const fn text(self) -> &'static str {
        match self {
            Self::Call => "call",
            Self::Conversion => "conversion",
            Self::TypeList => "type-list",
            Self::InitializerList => "initializer-list",
            Self::MemberAccess => "member",
            Self::PointerMemberAccess => "pointer-member",
            Self::MemberPointer => "member-pointer",
            Self::PointerToMember => "pointer-to-member",
            Self::SizeofType | Self::SizeofExpr => "sizeof",
            Self::AlignofType | Self::AlignofExpr => "alignof",
            Self::Noexcept => "noexcept",
            Self::Throw | Self::Rethrow => "throw",
            Self::TypeidType | Self::TypeidExpr => "typeid",
            Self::Cast => "cast",
            Self::PackExpansion => "pack",
            Self::SizeofPack => "sizeof-pack",
            Self::SizeofPackList => "sizeof-pack-list",
            Self::Subobject => "subobject",
            // The new and delete forms carry their own text.
            Self::New { .. } | Self::Delete { .. } => "",
        }
    }
}

/// The text of a new or delete expression, including the global qualifier.
fn new_expression_text(keyword: &str, array: bool, global: bool) -> Cow<'static, str> {
    let mut text = alloc::string::String::new();
    if global {
        text.push_str("::");
    }
    text.push_str(keyword);
    if array {
        text.push_str("[]");
    }
    Cow::Owned(text)
}

/// The expression form for a two-letter code, if it is not a plain operator.
const fn expression_form(first: u8, second: u8) -> Option<ExpressionForm> {
    Some(match (first, second) {
        (b'c', b'l') => ExpressionForm::Call,
        (b'c', b'v') => ExpressionForm::Conversion,
        (b't', b'l') => ExpressionForm::TypeList,
        (b'i', b'l') => ExpressionForm::InitializerList,
        (b'd', b't') => ExpressionForm::MemberAccess,
        (b'p', b't') => ExpressionForm::PointerMemberAccess,
        (b'd', b's') => ExpressionForm::MemberPointer,
        (b'p', b'm') => ExpressionForm::PointerToMember,
        (b's', b't') => ExpressionForm::SizeofType,
        (b's', b'z') => ExpressionForm::SizeofExpr,
        (b'a', b't') => ExpressionForm::AlignofType,
        (b'a', b'z') => ExpressionForm::AlignofExpr,
        (b'n', b'x') => ExpressionForm::Noexcept,
        (b't', b'w') => ExpressionForm::Throw,
        (b't', b'r') => ExpressionForm::Rethrow,
        (b't', b'i') => ExpressionForm::TypeidType,
        (b't', b'e') => ExpressionForm::TypeidExpr,
        (b'd' | b's' | b'c' | b'r', b'c') => ExpressionForm::Cast,
        (b's', b'p') => ExpressionForm::PackExpansion,
        (b's', b'Z') => ExpressionForm::SizeofPack,
        (b's', b'P') => ExpressionForm::SizeofPackList,
        (b's', b'o') => ExpressionForm::Subobject,
        (b'n', b'w') => ExpressionForm::New { array: false },
        (b'n', b'a') => ExpressionForm::New { array: true },
        (b'd', b'l') => ExpressionForm::Delete { array: false },
        (b'd', b'a') => ExpressionForm::Delete { array: true },
        _ => return None,
    })
}

/// Spell an Itanium `<operator-name>` code.
///
/// The caller has already read both bytes, so an unknown pair simply has no
/// spelling.
#[expect(clippy::match_same_arms, reason = "one arm per Itanium operator code")]
const fn operator_code_spelling(first: u8, second: u8) -> Option<&'static str> {
    Some(match (first, second) {
        (b'n', b'w') => "operator new",
        (b'n', b'a') => "operator new[]",
        (b'd', b'l') => "operator delete",
        (b'd', b'a') => "operator delete[]",
        (b'p', b's') => "operator+",
        (b'n', b'g') => "operator-",
        (b'a', b'd') => "operator&",
        (b'd', b'e') => "operator*",
        (b'd', b's') => "operator.*",
        (b'c', b'o') => "operator~",
        (b'p', b'l') => "operator+",
        (b'm', b'i') => "operator-",
        (b'm', b'l') => "operator*",
        (b'd', b'v') => "operator/",
        (b'r', b'm') => "operator%",
        (b'a', b'n') => "operator&",
        (b'o', b'r') => "operator|",
        (b'e', b'o') => "operator^",
        (b'a', b'S') => "operator=",
        (b'p', b'L') => "operator+=",
        (b'm', b'I') => "operator-=",
        (b'm', b'L') => "operator*=",
        (b'd', b'V') => "operator/=",
        (b'r', b'M') => "operator%=",
        (b'a', b'N') => "operator&=",
        (b'o', b'R') => "operator|=",
        (b'e', b'O') => "operator^=",
        (b'l', b's') => "operator<<",
        (b'r', b's') => "operator>>",
        (b'l', b'S') => "operator<<=",
        (b'r', b'S') => "operator>>=",
        (b'e', b'q') => "operator==",
        (b'n', b'e') => "operator!=",
        (b'l', b't') => "operator<",
        (b'g', b't') => "operator>",
        (b'l', b'e') => "operator<=",
        (b'g', b'e') => "operator>=",
        (b's', b's') => "operator<=>",
        (b'n', b't') => "operator!",
        (b'a', b'a') => "operator&&",
        (b'o', b'o') => "operator||",
        (b'p', b'p') => "operator++",
        (b'm', b'm') => "operator--",
        (b'c', b'm') => "operator,",
        (b'p', b'm') => "operator->*",
        (b'p', b't') => "operator->",
        (b'c', b'l') => "operator()",
        (b'i', b'x') => "operator[]",
        (b'q', b'u') => "operator?",
        (b's', b't') => "operator sizeof",
        (b's', b'z') => "operator sizeof...",
        (b'a', b't') => "operator alignof",
        (b'a', b'z') => "operator alignof...",
        (b'n', b'x') => "operator noexcept",
        (b'a', b'w') => "operator co_await",
        _ => return None,
    })
}

/// Number of operands an expression operator takes.
const fn expression_arity(first: u8, second: u8) -> u8 {
    match (first, second) {
        (b'q', b'u') => 3,
        (b'p', b's' | b'p')
        | (b'n', b'g' | b't' | b'w' | b'a')
        | (b'a', b'd')
        | (b'd', b'e' | b'l' | b'a')
        | (b'c', b'o')
        | (b'm', b'm') => 1,
        _ => 2,
    }
}

/// True when the two bytes start a `<template-param-decl>`.
///
/// The declaration keywords (`Ty`, `Tn`, `Tk`, `Tt`, `Tp`) are disjoint from
/// the template parameter reference forms (`T_`, `T0_`, `TL1_`).
fn is_template_param_decl(first: Option<u8>, second: Option<u8>) -> bool {
    first == Some(b'T') && matches!(second, Some(b'y' | b'n' | b'k' | b't' | b'p'))
}

/// Decode a `<source-name>` from an identifier that is already in hand.
///
/// The Objective-C protocol extension packs a length-prefixed protocol name
/// into the tail of the qualifier's own source name.
fn decode_source_name(text: &[u8]) -> Option<&[u8]> {
    let digits = text.iter().take_while(|byte| byte.is_ascii_digit()).count();
    if digits == 0 {
        return None;
    }
    let length = parse_decimal(text.get(..digits)?)?;
    let end = digits.checked_add(length)?;
    let body = text.get(digits..end)?;
    (end == text.len() && !body.is_empty()).then_some(body)
}

/// Parse a decimal number from an identifier slice.
fn parse_decimal(text: &[u8]) -> Option<usize> {
    let mut value = 0_usize;
    for &byte in text {
        let digit = byte.checked_sub(b'0').filter(|digit| *digit < 10)?;
        value = value.checked_mul(10)?.checked_add(usize::from(digit))?;
    }
    Some(value)
}

/// Builtin type names for the single-letter codes.
const fn builtin_name(byte: u8) -> Option<&'static str> {
    Some(match byte {
        b'v' => "void",
        b'w' => "wchar_t",
        b'b' => "bool",
        b'c' => "char",
        b'a' => "signed char",
        b'h' => "unsigned char",
        b's' => "short",
        b't' => "unsigned short",
        b'i' => "int",
        b'j' => "unsigned int",
        b'l' => "long",
        b'm' => "unsigned long",
        b'x' => "long long",
        b'y' => "unsigned long long",
        b'n' => "__int128",
        b'o' => "unsigned __int128",
        b'f' => "float",
        b'd' => "double",
        b'e' => "long double",
        b'g' => "__float128",
        b'z' => "...",
        _ => return None,
    })
}

/// Canonical cv-qualifier string.
const fn qualifier_text(const_: bool, volatile: bool, restrict: bool) -> &'static str {
    match (const_, volatile, restrict) {
        (true, false, false) => "const",
        (false, true, false) => "volatile",
        (false, false, true) => "restrict",
        (true, true, false) => "const volatile",
        (true, false, true) => "const restrict",
        (false, true, true) => "volatile restrict",
        (true, true, true) => "const volatile restrict",
        (false, false, false) => "",
    }
}

/// Canonical nested-name qualifier string, including a ref-qualifier.
fn name_qualifier_text<'a>(
    const_: bool,
    volatile: bool,
    restrict: bool,
    ref_qual: u8,
) -> Option<Cow<'a, str>> {
    let base = qualifier_text(const_, volatile, restrict);
    match (base.is_empty(), ref_qual) {
        (true, 0) => None,
        (false, 0) => Some(Cow::Borrowed(base)),
        (true, 1) => Some(Cow::Borrowed("&")),
        (true, _) => Some(Cow::Borrowed("&&")),
        (false, 1) => Some(Cow::Owned(alloc::format!("{base} &"))),
        (false, _) => Some(Cow::Owned(alloc::format!("{base} &&"))),
    }
}

/// True for a vendor qualifier chain, which qualifies the name after it.
///
/// `W3FOO` is a single qualifier and `W3FOOW3BAR` a chain of two; both are
/// module names, and a substitution may refer to either.
fn is_module_name(node: &NodeRef<'_>) -> bool {
    match node.kind() {
        Kind::VendorQualifier => true,
        Kind::NestedName => node.children().iter().all(is_module_name),
        _ => false,
    }
}

/// True when a name's final component is a template-id.
fn name_ends_with_template(node: &NodeRef<'_>) -> bool {
    match node.kind() {
        Kind::Name => true,
        Kind::NestedName | Kind::LocalName => {
            node.children().last().is_some_and(name_ends_with_template)
        }
        _ => false,
    }
}

/// True when a name's final component is a conversion operator.
///
/// A conversion operator's result type is encoded in the name, so a templated
/// conversion operator does not also encode a return type.
fn name_ends_with_conversion_operator(node: &NodeRef<'_>) -> bool {
    match node.kind() {
        Kind::OperatorName => node.text() == Some("operator") && node.child_count() == 1,
        Kind::Name => node
            .child(0)
            .is_some_and(|base| name_ends_with_conversion_operator(&base)),
        Kind::NestedName | Kind::LocalName => node
            .children()
            .last()
            .is_some_and(name_ends_with_conversion_operator),
        _ => false,
    }
}

/// True when a name's final component is a constructor or destructor.
fn name_ends_with_ctor(node: &NodeRef<'_>) -> bool {
    match node.kind() {
        Kind::CtorDtorName => true,
        Kind::UnqualifiedName => node
            .child(0)
            .is_some_and(|child| child.kind() == Kind::CtorDtorName),
        Kind::NestedName | Kind::LocalName => {
            node.children().last().is_some_and(name_ends_with_ctor)
        }
        Kind::Name => node.child(0).is_some_and(|base| name_ends_with_ctor(&base)),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use alloc::string::String;
    use alloc::vec::Vec;

    use super::super::{DEFAULT_BUDGET, Kind, MAX_INPUT_LEN, NodeRef, itanium_state, parse};
    use super::*;

    fn root_kind(symbol: &str) -> Kind {
        parse(symbol).unwrap().root().kind()
    }

    fn child_kinds(node: &NodeRef<'_>) -> Vec<Kind> {
        node.children().iter().map(|child| child.kind()).collect()
    }

    fn find_text<'a>(node: &NodeRef<'a>, text: &str) -> Option<NodeRef<'a>> {
        if node.text() == Some(text) {
            return Some(Rc::clone(node));
        }
        node.children()
            .iter()
            .find_map(|child| find_text(child, text))
    }

    /// Find the first node of `kind` in the graph.
    fn find_kind<'a>(node: &NodeRef<'a>, kind: Kind) -> Option<NodeRef<'a>> {
        if node.kind() == kind {
            return Some(Rc::clone(node));
        }
        node.children()
            .iter()
            .find_map(|child| find_kind(child, kind))
    }

    /// True when the graph holds a nested name qualified by a template-id.
    fn has_nested_template_id(node: &NodeRef<'_>) -> bool {
        (node.kind() == Kind::NestedName
            && node
                .children()
                .iter()
                .any(|child| child.kind() == Kind::Name && child.child_count() == 2))
            || node.children().iter().any(has_nested_template_id)
    }

    fn count_nodes(node: &NodeRef<'_>) -> usize {
        1 + node.children().iter().map(count_nodes).sum::<usize>()
    }

    #[test]
    fn parses_data_name() {
        let ast = parse("_Z1A").unwrap();
        let encoding = ast.root();
        assert_eq!(encoding.kind(), Kind::Encoding);
        let data = encoding.child(0).unwrap();
        assert_eq!(data.kind(), Kind::Data);
        let name = data.child(0).unwrap();
        assert_eq!(name.kind(), Kind::UnqualifiedName);
        assert_eq!(name.child(0).unwrap().kind(), Kind::Identifier);
        assert_eq!(name.child(0).unwrap().text(), Some("A"));
    }

    #[test]
    fn parses_function_with_parameters() {
        let ast = parse("_Z1A1B1C").unwrap();
        let function = ast.root().child(0).unwrap();
        assert_eq!(function.kind(), Kind::Function);
        assert_eq!(
            child_kinds(&function),
            alloc::vec![Kind::UnqualifiedName, Kind::ParameterList]
        );
        let parameters = function.child(1).unwrap();
        assert_eq!(parameters.child_count(), 2);
        assert_eq!(parameters.child(0).unwrap().kind(), Kind::Parameter);
        assert_eq!(
            parameters
                .child(0)
                .unwrap()
                .child(0)
                .unwrap()
                .child(0)
                .unwrap()
                .text(),
            Some("B")
        );
    }

    #[test]
    fn parses_void_parameter_list() {
        let ast = parse("_Z1Av").unwrap();
        let function = ast.root().child(0).unwrap();
        let parameters = function.child(1).unwrap();
        assert_eq!(parameters.kind(), Kind::ParameterList);
        assert!(parameters.is_leaf());
    }

    #[test]
    fn parses_nested_names() {
        let ast = parse("_ZN3Foo3barEv").unwrap();
        let function = ast.root().child(0).unwrap();
        let name = function.child(0).unwrap();
        assert_eq!(name.kind(), Kind::NestedName);
        assert_eq!(
            child_kinds(&name),
            alloc::vec![Kind::UnqualifiedName, Kind::UnqualifiedName]
        );
        assert_eq!(name.child(0).unwrap().child(0).unwrap().text(), Some("Foo"));
        assert_eq!(name.child(1).unwrap().child(0).unwrap().text(), Some("bar"));
    }

    #[test]
    fn parses_cv_and_ref_qualified_names() {
        let ast = parse("_ZNK1A1fEv").unwrap();
        let name = ast.root().child(0).unwrap().child(0).unwrap();
        assert_eq!(name.kind(), Kind::NestedName);
        assert_eq!(name.text(), Some("const"));

        let ast = parse("_ZNKO1A1hEv").unwrap();
        let name = ast.root().child(0).unwrap().child(0).unwrap();
        assert_eq!(name.text(), Some("const &&"));
    }

    #[test]
    fn parses_builtins_pointers_and_qualifiers() {
        let ast = parse("_Z1fPKc").unwrap();
        let parameters = ast.root().child(0).unwrap().child(1).unwrap();
        let parameter = parameters.child(0).unwrap();
        let pointer = parameter.child(0).unwrap();
        assert_eq!(pointer.kind(), Kind::PointerType);
        let qualified = pointer.child(0).unwrap();
        assert_eq!(qualified.kind(), Kind::QualifiedType);
        assert_eq!(qualified.text(), Some("const"));
        assert_eq!(qualified.child(0).unwrap().text(), Some("char"));
    }

    #[test]
    fn parses_arrays_and_function_types() {
        let ast = parse("_Z1fA5_i").unwrap();
        let array = ast
            .root()
            .child(0)
            .unwrap()
            .child(1)
            .unwrap()
            .child(0)
            .unwrap()
            .child(0)
            .unwrap();
        assert_eq!(array.kind(), Kind::ArrayType);
        assert_eq!(array.text(), Some("5"));
        assert_eq!(array.child(0).unwrap().text(), Some("int"));

        let ast = parse("_Z1fPFivE").unwrap();
        let pointer = ast
            .root()
            .child(0)
            .unwrap()
            .child(1)
            .unwrap()
            .child(0)
            .unwrap()
            .child(0)
            .unwrap();
        assert_eq!(pointer.kind(), Kind::PointerType);
        let function = pointer.child(0).unwrap();
        assert_eq!(function.kind(), Kind::FunctionType);
        assert_eq!(function.child(0).unwrap().text(), Some("int"));
        assert!(function.child(1).unwrap().is_leaf());
    }

    #[test]
    fn parses_member_pointer() {
        let ast = parse("_Z1fM1AFivE").unwrap();
        let member = ast
            .root()
            .child(0)
            .unwrap()
            .child(1)
            .unwrap()
            .child(0)
            .unwrap()
            .child(0)
            .unwrap();
        assert_eq!(member.kind(), Kind::MemberPointerType);
        assert_eq!(member.child(0).unwrap().child(0).unwrap().text(), Some("A"));
        assert_eq!(member.child(1).unwrap().kind(), Kind::FunctionType);
    }

    /// Every `<operator-name>` code and the text it must print.
    ///
    /// This table is the oracle for the parser's direct two-byte match: it
    /// pins both the accepted codes and their spelling, so a code cannot be
    /// dropped, invented or re-spelled unnoticed.
    const OPERATOR_SPELLINGS: &[(&[u8; 2], &str)] = &[
        (b"nw", "operator new"),
        (b"na", "operator new[]"),
        (b"dl", "operator delete"),
        (b"da", "operator delete[]"),
        (b"ps", "operator+"),
        (b"ng", "operator-"),
        (b"ad", "operator&"),
        (b"de", "operator*"),
        (b"ds", "operator.*"),
        (b"co", "operator~"),
        (b"pl", "operator+"),
        (b"mi", "operator-"),
        (b"ml", "operator*"),
        (b"dv", "operator/"),
        (b"rm", "operator%"),
        (b"an", "operator&"),
        (b"or", "operator|"),
        (b"eo", "operator^"),
        (b"aS", "operator="),
        (b"pL", "operator+="),
        (b"mI", "operator-="),
        (b"mL", "operator*="),
        (b"dV", "operator/="),
        (b"rM", "operator%="),
        (b"aN", "operator&="),
        (b"oR", "operator|="),
        (b"eO", "operator^="),
        (b"ls", "operator<<"),
        (b"rs", "operator>>"),
        (b"lS", "operator<<="),
        (b"rS", "operator>>="),
        (b"eq", "operator=="),
        (b"ne", "operator!="),
        (b"lt", "operator<"),
        (b"gt", "operator>"),
        (b"le", "operator<="),
        (b"ge", "operator>="),
        (b"ss", "operator<=>"),
        (b"nt", "operator!"),
        (b"aa", "operator&&"),
        (b"oo", "operator||"),
        (b"pp", "operator++"),
        (b"mm", "operator--"),
        (b"cm", "operator,"),
        (b"pm", "operator->*"),
        (b"pt", "operator->"),
        (b"cl", "operator()"),
        (b"ix", "operator[]"),
        (b"qu", "operator?"),
        (b"st", "operator sizeof"),
        (b"sz", "operator sizeof..."),
        (b"at", "operator alignof"),
        (b"az", "operator alignof..."),
        (b"nx", "operator noexcept"),
        (b"aw", "operator co_await"),
    ];

    /// The spelling `OPERATOR_SPELLINGS` expects for one byte pair.
    fn expected_operator_spelling(first: u8, second: u8) -> Option<&'static str> {
        OPERATOR_SPELLINGS
            .iter()
            .find(|(code, _)| code[0] == first && code[1] == second)
            .map(|&(_, spelling)| spelling)
    }

    #[test]
    fn operator_code_table_has_no_duplicate_codes() {
        for (index, (code, _)) in OPERATOR_SPELLINGS.iter().enumerate() {
            let earlier = OPERATOR_SPELLINGS.get(..index).unwrap_or(&[]);
            assert!(
                !earlier.iter().any(|(other, _)| other == code),
                "duplicate operator code {code:?}"
            );
        }
    }

    #[test]
    fn operator_code_table_is_exhaustive() {
        // Both bytes together enumerate every code the match may accept, so a
        // missing, invented or re-spelled code fails here.
        for first in 0..=u8::MAX {
            for second in 0..=u8::MAX {
                assert_eq!(
                    operator_code_spelling(first, second),
                    expected_operator_spelling(first, second),
                    "operator code {first:#04x} {second:#04x}"
                );
            }
        }
    }

    /// A parser over `bytes` positioned at `prefix`, with the compiled-in
    /// Itanium limits.
    fn itanium_parser<'a, 's>(
        bytes: &'a [u8],
        prefix: usize,
        state: &'s mut State,
    ) -> Parser<'a, 's> {
        Parser::new(bytes, prefix, state, ParseLimits::itanium(DEFAULT_BUDGET)).unwrap()
    }

    #[test]
    fn operator_lookup_reads_two_bytes_at_the_cursor() {
        for (code, spelling) in OPERATOR_SPELLINGS {
            // A code at the very end of the input is still a complete code.
            let mut state = itanium_state(code.len());
            let parser = itanium_parser(*code, 0, &mut state);
            assert_eq!(parser.operator_spelling(), Some(*spelling));

            // The lookup itself does not move the cursor; the accepting caller
            // advances exactly two bytes, leaving a following name in place.
            let mut bytes = code.to_vec();
            bytes.extend_from_slice(b"3foo");
            let mut state = itanium_state(bytes.len());
            let mut parser = itanium_parser(&bytes, 0, &mut state);
            assert_eq!(parser.operator_spelling(), Some(*spelling));
            assert_eq!(parser.input.pos(), 0, "lookup consumed input");
            let name = parser.parse_unqualified_name().unwrap();
            assert_eq!(name.kind(), Kind::OperatorName);
            assert_eq!(name.text(), Some(*spelling));
            assert_eq!(parser.input.pos(), 2, "{code:?} must consume two bytes");
        }
    }

    #[test]
    fn operator_lookup_rejects_truncated_and_unknown_codes() {
        // Empty input has no code at all.
        let mut state = itanium_state(0);
        let parser = itanium_parser(b"", 0, &mut state);
        assert_eq!(parser.operator_spelling(), None);

        // One byte of a real code is a truncated code, not a match.
        for (code, _) in OPERATOR_SPELLINGS {
            let mut state = itanium_state(1);
            let parser = itanium_parser(&code[..1], 0, &mut state);
            assert_eq!(parser.operator_spelling(), None, "truncated {code:?}");
        }

        // Two bytes that are not a code are rejected, including codes spelled
        // with the wrong case and bytes outside ASCII.
        for code in [b"zz", b"nW", b"NN", b"a ", b" w", b"\xff\xff"] {
            let mut state = itanium_state(code.len());
            let parser = itanium_parser(code, 0, &mut state);
            assert_eq!(parser.operator_spelling(), None, "unknown {code:?}");
        }

        // A code is only recognised at the cursor: `nw` after the first byte is
        // not at the current position.
        let mut state = itanium_state(3);
        let parser = itanium_parser(b"xnw", 0, &mut state);
        assert_eq!(parser.operator_spelling(), None);
        let mut state = itanium_state(3);
        let parser = itanium_parser(b"xnw", 1, &mut state);
        assert_eq!(parser.operator_spelling(), Some("operator new"));
        // A truncated code at the cursor stays rejected even when an earlier
        // byte would have completed it.
        let mut state = itanium_state(2);
        let parser = itanium_parser(b"xn", 1, &mut state);
        assert_eq!(parser.operator_spelling(), None);
    }

    #[test]
    fn templated_conversion_operator_has_no_return_type() {
        let ast = parse("_ZN1AcviIiEEv").unwrap();
        let function = ast.root().child(0).unwrap();
        assert_eq!(function.kind(), Kind::Function);
        assert_eq!(
            child_kinds(&function),
            alloc::vec![Kind::Name, Kind::ParameterList]
        );
    }

    #[test]
    fn parses_template_arguments_and_parameters() {
        let ast = parse("_Z3fooIiEvT_").unwrap();
        let function = ast.root().child(0).unwrap();
        let name = function.child(0).unwrap();
        assert_eq!(name.kind(), Kind::Name);
        assert_eq!(name.child(1).unwrap().kind(), Kind::TemplateArgs);
        assert_eq!(function.child(1).unwrap().text(), Some("void"));
        let parameter = function.child(2).unwrap().child(0).unwrap();
        assert_eq!(parameter.child(0).unwrap().kind(), Kind::TemplateParam);
        assert_eq!(parameter.child(0).unwrap().text(), Some("T_"));
    }

    #[test]
    fn parses_std_names() {
        let ast = parse("_ZSt4cout").unwrap();
        let name = ast.root().child(0).unwrap().child(0).unwrap();
        assert_eq!(name.kind(), Kind::NestedName);
        assert_eq!(name.child(0).unwrap().text(), Some("std"));
        assert_eq!(
            name.child(1).unwrap().child(0).unwrap().text(),
            Some("cout")
        );
    }

    #[test]
    fn substitutions_share_nodes() {
        let ast = parse("_Z1f1AS_S_").unwrap();
        let parameters = ast.root().child(0).unwrap().child(1).unwrap();
        let first = parameters.child(0).unwrap().child(0).unwrap();
        let second = parameters.child(1).unwrap().child(0).unwrap();
        assert!(Rc::ptr_eq(&first, &second));
    }

    #[test]
    fn nested_prefix_substitution_shares_nodes() {
        let ast = parse("_ZN1A1B1CES0_1D").unwrap();
        // The first parameter is the substituted `A::B` prefix.
        let parameters = ast.root().child(0).unwrap().child(1).unwrap();
        let first = parameters.child(0).unwrap().child(0).unwrap();
        assert_eq!(first.kind(), Kind::NestedName);
        assert_eq!(first.child(0).unwrap().child(0).unwrap().text(), Some("A"));
        assert_eq!(first.child(1).unwrap().child(0).unwrap().text(), Some("B"));
    }

    #[test]
    fn identifier_text_borrows_the_symbol() {
        let symbol = String::from("_ZN3Foo3barEv");
        let ast = parse(&symbol).unwrap();
        let foo = find_text(&ast.root(), "Foo").unwrap();
        let text = foo.text().unwrap();
        let symbol_start = symbol.as_ptr() as usize;
        let text_start = text.as_ptr() as usize;
        assert!(text_start >= symbol_start);
        assert!(text_start + text.len() <= symbol_start + symbol.len());
    }

    #[test]
    fn rejects_malformed_input() {
        for symbol in ["_Z", "_Z1", "_Z1fP", "_Z1fS_", "_Zabc", "_Z0", "_Z1fZ"] {
            assert!(parse(symbol).is_err(), "{symbol} should fail");
        }
        assert!(parse("_Z1f1A").is_ok());
    }

    #[test]
    fn parses_vector_types() {
        let ast = parse("_Z1fDv2_i").unwrap();
        let vector = ast
            .root()
            .child(0)
            .unwrap()
            .child(1)
            .unwrap()
            .child(0)
            .unwrap()
            .child(0)
            .unwrap();
        assert_eq!(vector.kind(), Kind::VectorType);
        assert_eq!(vector.text(), Some("2"));
        assert_eq!(vector.child(0).unwrap().text(), Some("int"));
    }

    #[test]
    fn parses_structured_binding_names() {
        let ast = parse("_ZDC1a1bE").unwrap();
        let name = ast.root().child(0).unwrap().child(0).unwrap();
        assert_eq!(name.kind(), Kind::StructuredBinding);
        assert_eq!(name.child_count(), 2);
        assert_eq!(name.child(0).unwrap().text(), Some("a"));
        assert_eq!(name.child(1).unwrap().text(), Some("b"));
    }

    #[test]
    fn parses_explicit_object_parameters() {
        let ast = parse("_ZNH3Foo3fooES_i").unwrap();
        let parameters = ast.root().child(0).unwrap().child(1).unwrap();
        let first = parameters.child(0).unwrap().child(0).unwrap();
        assert_eq!(first.kind(), Kind::ExplicitObjectParameter);
        assert_eq!(
            first.child(0).unwrap().child(0).unwrap().text(),
            Some("Foo")
        );
        let second = parameters.child(1).unwrap().child(0).unwrap();
        assert_eq!(second.kind(), Kind::BuiltinType);
    }

    #[test]
    fn parses_function_type_exception_specifications() {
        let ast = parse("_Z1bPDoFivE").unwrap();
        let function = ast
            .root()
            .child(0)
            .unwrap()
            .child(1)
            .unwrap()
            .child(0)
            .unwrap()
            .child(0)
            .unwrap()
            .child(0)
            .unwrap();
        assert_eq!(function.kind(), Kind::FunctionType);
        let spec = function.child(2).unwrap();
        assert_eq!(spec.kind(), Kind::ExceptionSpec);
        assert_eq!(spec.text(), Some("noexcept"));
        assert!(spec.is_leaf());
    }

    #[test]
    fn parses_unresolved_names_in_template_arguments() {
        // `srN <substitution> <template-id> E <base>`: the qualifier levels
        // build a nested name whose prefix is the `test8::meta<int>`
        // template-id.
        let ast = parse("_ZN5test81fIiEEvNS_5int_cIXsrNS_4metaIT_E4typeE5valueEEE").unwrap();
        assert!(has_nested_template_id(&ast.root()), "{:?}", ast.root());
        // The base of the unresolved name is a bare identifier.
        assert_eq!(
            find_text(&ast.root(), "value").unwrap().kind(),
            Kind::Identifier
        );
    }

    #[test]
    fn parses_block_invocation_functions() {
        let ast = parse("___Z10blocksNRVOv_block_invoke").unwrap();
        let invoke = ast.root().child(0).unwrap();
        assert_eq!(invoke.kind(), Kind::BlockInvoke);
        assert_eq!(invoke.child(0).unwrap().kind(), Kind::Function);
    }

    #[test]
    fn parses_module_initializers() {
        let ast = parse("_ZGIW3FooWP3BarW3Baz").unwrap();
        let initializer = ast.root().child(0).unwrap();
        assert_eq!(initializer.kind(), Kind::ModuleInitializer);
        let module = initializer.child(0).unwrap();
        assert_eq!(module.kind(), Kind::ModuleName);
        assert_eq!(module.text(), Some("010"));
        assert_eq!(module.child_count(), 3);
    }

    #[test]
    fn parses_friend_function_names() {
        let ast = parse("_ZN5test21AIiEF1fEzQ4TrueIT_E").unwrap();
        let friend = find_kind(&ast.root(), Kind::FriendName).expect("friend name");
        assert_eq!(friend.child_count(), 1);
        let identifier = find_kind(&friend, Kind::Identifier).expect("identifier");
        assert_eq!(identifier.text(), Some("f"));
    }

    #[test]
    fn parses_encoding_requires_clauses() {
        let ast = parse("_ZN5test21hIvEEvzQ4TrueITL0__E").unwrap();
        let function = ast.root().child(0).unwrap();
        let clause = function.child(function.child_count() - 1).unwrap();
        assert_eq!(clause.kind(), Kind::RequiresClause);
        // The parameter list is the last child before the clause.
        assert_eq!(
            function.child(function.child_count() - 2).unwrap().kind(),
            Kind::ParameterList
        );
        // Constraint parameters print their encoded numbering.
        assert_eq!(
            find_text(&clause, "TL0_").map(|node| node.kind()),
            Some(Kind::Identifier)
        );
    }

    #[test]
    fn parses_template_head_requires_clauses() {
        // `Tt Ty Tn TL0__ Q1CIS1_E E`: the clause sits inside the template
        // template parameter's own declaration list.
        let ast = parse("_ZN5test51gITtTyTnTL0__Q1CIS1_EENS_1XEEEvv").unwrap();
        let declaration = find_text(&ast.root(), "Tt").expect("template template parameter");
        assert_eq!(declaration.kind(), Kind::TemplateParamDecl);
        assert!(
            declaration
                .children()
                .iter()
                .any(|child| child.kind() == Kind::RequiresClause),
            "the clause belongs to the parameter list"
        );
    }

    #[test]
    fn parses_requires_expressions() {
        let ast = parse("_Z1gIiEviQrQT__XplfL0p_fp_E").unwrap();
        let expression = find_kind(&ast.root(), Kind::RequiresExpression).expect("requires");
        assert_eq!(expression.child(0).unwrap().kind(), Kind::ParameterList);
        let requirements: Vec<&NodeRef<'_>> = expression
            .children()
            .iter()
            .filter(|child| child.kind() == Kind::Requirement)
            .collect();
        assert_eq!(requirements.len(), 1);
        assert_eq!(requirements[0].text(), Some("simple"));
    }

    #[test]
    fn parses_pack_subscripts() {
        let ast = parse("_Z1gILi0EJciEEDyT0_T_v").unwrap();
        let index = find_kind(&ast.root(), Kind::PackIndexType).expect("pack index type");
        assert_eq!(index.child(0).unwrap().kind(), Kind::TemplateParam);
        assert_eq!(index.child(1).unwrap().kind(), Kind::Expression);

        let ast = parse("_Z3fooILi0ETpTnDaJLi1ELi2EEEDTsyT0_T_Ev").unwrap();
        let expression = find_kind(&ast.root(), Kind::Expression).expect("pack index");
        assert_eq!(expression.text(), Some("pack-index"));
        assert_eq!(expression.child(0).unwrap().kind(), Kind::TemplateParam);
    }

    #[test]
    fn parses_member_pointer_conversions() {
        let ast = parse("_Z1fIXtl1DmcM7DerivedKiadL_ZN11MoreDerived1zEEn8EEEEvv").unwrap();
        let cast = find_text(&ast.root(), "member-cast").expect("member cast");
        assert_eq!(cast.kind(), Kind::Expression);
        assert_eq!(cast.child(0).unwrap().kind(), Kind::MemberPointerType);
        // The base-class adjustment is encoded but not part of the AST.
        assert_eq!(cast.child_count(), 2);
    }

    #[test]
    fn parses_module_qualifier_chains() {
        // `W3FOO` and `W3FOO W3BAR` are the first two substitution entries; the
        // qualifier chain is reused through `S0_` and extended by `W3BAR`.
        let ast = parse("_ZW3FOOW3BAR3FooPS0_1APN3BobS0_1BE").unwrap();
        let parameters = ast.root().child(0).unwrap().child(1).unwrap();
        assert_eq!(parameters.child_count(), 2);
        let pointer = parameters.child(0).unwrap().child(0).unwrap();
        assert_eq!(pointer.kind(), Kind::PointerType);
        let target = pointer.child(0).unwrap();
        assert_eq!(target.kind(), Kind::NestedName);
        assert_eq!(
            find_kind(&target, Kind::Identifier).unwrap().text(),
            Some("A")
        );
    }

    #[test]
    fn parses_allocation_token_decorations() {
        let ast = parse("__alloc_token_123__Z3foov").unwrap();
        let decoration = ast.root().child(0).unwrap();
        assert_eq!(decoration.kind(), Kind::AllocToken);
        assert_eq!(decoration.child(0).unwrap().kind(), Kind::Function);

        let ast = parse("__alloc_token__Z3foov.llvm.1234").unwrap();
        let decoration = ast.root().child(0).unwrap();
        assert_eq!(decoration.kind(), Kind::AllocToken);
        assert_eq!(decoration.child(0).unwrap().kind(), Kind::DotSuffix);
    }

    #[test]
    fn rejects_symbols_that_are_not_encodings() {
        // A bare type mangling carries no encoding prefix.
        for symbol in ["i", "PKFvRiE"] {
            let error = parse(symbol).unwrap_err();
            assert_eq!(error.kind(), ParseErrorKind::NotItanium, "{symbol}");
        }
        // A decoration still has to introduce a mangled name.
        assert_eq!(
            parse("__alloc_token_0_Z3foov").unwrap_err().kind(),
            ParseErrorKind::NotItanium
        );
        assert_eq!(
            parse("__alloc_token__Z").unwrap_err().kind(),
            ParseErrorKind::UnexpectedEnd
        );
        // A requirement has to start with `X`, `T` or `Q`.
        assert_eq!(
            parse("_Z1fIiEviQrqZE").unwrap_err().kind(),
            ParseErrorKind::UnexpectedByte
        );
    }

    #[test]
    fn input_budget_is_enforced() {
        let mut symbol = String::from("_Z");
        symbol.push_str(&"1".repeat(MAX_INPUT_LEN));
        let error = parse(&symbol).unwrap_err();
        assert_eq!(error.kind(), ParseErrorKind::TooLong);
    }

    #[test]
    fn depth_budget_is_enforced() {
        let mut symbol = String::from("_Z1f");
        symbol.push_str(&"P".repeat(300));
        symbol.push('i');
        let error = parse(&symbol).unwrap_err();
        assert_eq!(error.kind(), ParseErrorKind::TooDeep);
    }

    #[test]
    fn expression_and_type_depth_budgets_are_enforced() {
        let mut braced = String::from("_Z1fIXtl1B");
        for _ in 0..400 {
            braced.push_str("di1a");
        }
        braced.push_str("Li1EEEEvv");
        assert_eq!(
            parse(&braced).unwrap_err().kind(),
            ParseErrorKind::TooDeep,
            "braced expressions"
        );

        let mut vendor = String::from("_Z1f");
        for _ in 0..400 {
            vendor.push_str("U1a");
        }
        vendor.push('i');
        assert_eq!(
            parse(&vendor).unwrap_err().kind(),
            ParseErrorKind::TooDeep,
            "vendor extended qualifiers"
        );

        // A module name is a left-nested chain, so its length is a depth.
        let mut chain = String::from("_Z1fW1a");
        for _ in 0..400 {
            chain.push_str("W1a");
        }
        chain.push_str("1b");
        assert_eq!(
            parse(&chain).unwrap_err().kind(),
            ParseErrorKind::TooDeep,
            "module name chains"
        );
    }

    #[test]
    fn node_budget_is_enforced() {
        let mut symbol = String::from("_Z1f");
        symbol.push_str(&"i".repeat(20_000));
        let error = parse(&symbol).unwrap_err();
        assert_eq!(error.kind(), ParseErrorKind::TooManyNodes);
    }

    #[test]
    fn root_kind_is_encoding() {
        assert_eq!(root_kind("_Z1A"), Kind::Encoding);
        assert_eq!(root_kind("_Z1fv"), Kind::Encoding);
    }

    #[test]
    fn node_count_is_bounded_by_input() {
        let ast = parse("_Z1fPKc").unwrap();
        assert!(count_nodes(&ast.root()) < 32);
    }
}
