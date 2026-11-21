//! Phase-0 Itanium C++ grammar parser.
//!
//! The parser builds immutable [`Node`] handles directly. Substitution entries
//! are stored as `Rc` handles, so a back-reference shares the referenced node
//! instead of copying it.

use alloc::{borrow::Cow, rc::Rc, vec::Vec};

use super::node::{Kind, Node, NodeRef};
use super::{Ast, MAX_INPUT_LEN, ParseError, ParseErrorKind};

/// Maximum nesting depth for recursive productions.
const MAX_DEPTH: u32 = 256;
/// Maximum nodes one parse may allocate.
const MAX_NODES: u32 = 1 << 14;
/// Minimum work allowance for very short symbols.
const MIN_WORK: usize = 1 << 12;
/// Work allowance per input byte.
const WORK_PER_BYTE: usize = 32;

/// Recursive-descent parser over one mangled symbol.
pub(crate) struct Parser<'a> {
    symbol: &'a str,
    bytes: &'a [u8],
    pos: usize,
    subs: Vec<NodeRef<'a>>,
    depth: u32,
    nodes: u32,
    work: usize,
    work_limit: usize,
    /// Depth of the enclosing `Z <encoding> E` forms.
    ///
    /// A nested encoding ends at the closing `E` instead of at the end of the
    /// symbol, so its bare-function-type stops there.
    nested_encoding: u32,
}

impl<'a> Parser<'a> {
    /// Create a parser positioned after the mangling prefix.
    pub(crate) fn new(symbol: &'a str, prefix: usize) -> Result<Self, ParseError> {
        if symbol.len() > MAX_INPUT_LEN {
            return Err(ParseError::new(0, ParseErrorKind::TooLong));
        }
        let work_limit = symbol
            .len()
            .saturating_mul(WORK_PER_BYTE)
            .saturating_add(MIN_WORK);
        Ok(Self {
            symbol,
            bytes: symbol.as_bytes(),
            pos: prefix,
            subs: Vec::new(),
            depth: 0,
            nodes: 0,
            work: 0,
            work_limit,
            nested_encoding: 0,
        })
    }

    /// Parse the encoding and require the whole symbol to be consumed.
    pub(crate) fn parse(mut self) -> Result<Ast<'a>, ParseError> {
        let start = self.pos;
        let encoding = self.parse_encoding()?;
        if self.pos != self.bytes.len() {
            return Err(self.error(ParseErrorKind::UnexpectedByte));
        }
        let root = self.make(Kind::Encoding, None, start, alloc::vec![encoding])?;
        Ok(Ast::from_root(root))
    }

    fn error(&self, kind: ParseErrorKind) -> ParseError {
        ParseError::new(self.pos, kind)
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }

    fn peek_at(&self, offset: usize) -> Option<u8> {
        self.pos
            .checked_add(offset)
            .and_then(|index| self.bytes.get(index))
            .copied()
    }

    fn starts_with(&self, text: &[u8]) -> bool {
        self.bytes
            .get(self.pos..)
            .is_some_and(|rest| rest.starts_with(text))
    }

    fn eat(&mut self, byte: u8) -> bool {
        if self.peek() == Some(byte) {
            self.pos += 1;
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

    fn tick(&mut self) -> Result<(), ParseError> {
        self.work += 1;
        if self.work > self.work_limit {
            return Err(self.error(ParseErrorKind::TooMuchWork));
        }
        Ok(())
    }

    fn enter(&mut self) -> Result<(), ParseError> {
        self.tick()?;
        if self.depth >= MAX_DEPTH {
            return Err(self.error(ParseErrorKind::TooDeep));
        }
        self.depth += 1;
        Ok(())
    }

    fn leave(&mut self) {
        self.depth -= 1;
    }

    fn make(
        &mut self,
        kind: Kind,
        text: Option<Cow<'a, str>>,
        start: usize,
        children: Vec<NodeRef<'a>>,
    ) -> Result<NodeRef<'a>, ParseError> {
        self.nodes = self
            .nodes
            .checked_add(1)
            .ok_or_else(|| self.error(ParseErrorKind::TooManyNodes))?;
        if self.nodes > MAX_NODES {
            return Err(self.error(ParseErrorKind::TooManyNodes));
        }
        let end = self.pos;
        let span = if start <= end && end <= self.symbol.len() {
            Some(start as u32..end as u32)
        } else {
            None
        };
        Ok(Node::new(kind, text, span, children))
    }

    fn span_start(&self, node: &NodeRef<'a>) -> Option<usize> {
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
        let start = self.pos;
        if matches!(self.peek(), Some(b'T' | b'G')) {
            return self.parse_special_name(start);
        }
        let name = self.parse_name()?;
        if self.at_encoding_end() {
            return self.make(Kind::Data, None, start, alloc::vec![name]);
        }
        let return_type = if name_ends_with_template(&name)
            && !name_ends_with_ctor(&name)
            && !name_ends_with_conversion_operator(&name)
        {
            Some(self.parse_type()?)
        } else {
            None
        };
        let parameters = self.parse_bare_function_type()?;
        let mut children = Vec::with_capacity(3);
        children.push(name);
        if let Some(return_type) = return_type {
            children.push(return_type);
        }
        children.push(parameters);
        self.make(Kind::Function, None, start, children)
    }

    fn parse_bare_function_type(&mut self) -> Result<NodeRef<'a>, ParseError> {
        let start = self.pos;
        let mut parameters = Vec::new();
        if self.peek() == Some(b'v') && self.ends_bare_function_type(1) {
            self.pos += 1;
            self.tick()?;
        } else {
            while !self.at_encoding_end() {
                let ty = self.parse_type()?;
                parameters.push(self.make_parameter(ty)?);
            }
        }
        self.make(Kind::ParameterList, None, start, parameters)
    }

    /// True when the byte at `offset` ends a bare-function-type.
    fn ends_bare_function_type(&self, offset: usize) -> bool {
        match self.peek_at(offset) {
            None => true,
            Some(b'E') => self.nested_encoding > 0,
            Some(_) => false,
        }
    }

    /// True at the end of an encoding: the symbol or the closing `E` of a
    /// nested encoding.
    fn at_encoding_end(&self) -> bool {
        match self.peek() {
            None => true,
            Some(b'E') => self.nested_encoding > 0,
            Some(_) => false,
        }
    }

    fn make_parameter(&mut self, ty: NodeRef<'a>) -> Result<NodeRef<'a>, ParseError> {
        let start = self.span_start(&ty).unwrap_or(self.pos);
        self.make(Kind::Parameter, None, start, alloc::vec![ty])
    }

    // -- special names -----------------------------------------------------

    /// Parse a `<special-name>`, the third shape an encoding can take.
    fn parse_special_name(&mut self, start: usize) -> Result<NodeRef<'a>, ParseError> {
        self.tick()?;
        if self.starts_with(b"TV") {
            self.pos += 2;
            let ty = self.parse_type()?;
            return self.make(Kind::VTable, None, start, alloc::vec![ty]);
        }
        if self.starts_with(b"TT") {
            self.pos += 2;
            let ty = self.parse_type()?;
            return self.make(Kind::Vtt, None, start, alloc::vec![ty]);
        }
        if self.starts_with(b"TI") {
            self.pos += 2;
            let ty = self.parse_type()?;
            return self.make(Kind::TypeInfo, None, start, alloc::vec![ty]);
        }
        if self.starts_with(b"TS") {
            self.pos += 2;
            let ty = self.parse_type()?;
            return self.make(Kind::TypeInfoName, None, start, alloc::vec![ty]);
        }
        if self.starts_with(b"TH") {
            self.pos += 2;
            let name = self.parse_name()?;
            return self.make(Kind::TlsInit, None, start, alloc::vec![name]);
        }
        if self.starts_with(b"TW") {
            self.pos += 2;
            let name = self.parse_name()?;
            return self.make(Kind::TlsWrapper, None, start, alloc::vec![name]);
        }
        if self.starts_with(b"GV") {
            self.pos += 2;
            let name = self.parse_name()?;
            return self.make(Kind::GuardVariable, None, start, alloc::vec![name]);
        }
        if self.starts_with(b"GR") {
            self.pos += 2;
            let name = self.parse_name()?;
            let index = if self.eat(b'_') {
                0
            } else if matches!(self.peek(), Some(b'0'..=b'9' | b'A'..=b'Z')) {
                let index = self
                    .parse_seq_id()?
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
        if self.starts_with(b"TC") {
            self.pos += 2;
            let complete = self.parse_type()?;
            let _offset = self.parse_number()?;
            self.expect(b'_')?;
            let base = self.parse_type()?;
            return self.make(Kind::CtorVtable, None, start, alloc::vec![complete, base]);
        }
        if self.starts_with(b"TA") {
            self.pos += 2;
            let argument = self.parse_template_arg()?;
            return self.make(
                Kind::TemplateParamObject,
                None,
                start,
                alloc::vec![argument],
            );
        }
        if self.starts_with(b"Th") || self.starts_with(b"Tv") {
            self.pos += 1;
            return self.parse_thunk(start, false);
        }
        if self.starts_with(b"Tc") {
            self.pos += 2;
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
            Some(_) => self.parse_unscoped_name(),
            None => Err(self.error(ParseErrorKind::UnexpectedEnd)),
        }
    }

    /// Parse `Z <encoding> E <entity> [<discriminator>]`.
    ///
    /// The `Es` form names a string literal and the `Ed` form carries the
    /// number of the enclosing function parameter the local name belongs to.
    fn parse_local_name(&mut self) -> Result<NodeRef<'a>, ParseError> {
        let start = self.pos;
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
            self.pos += 1;
            self.tick()?;
            let number = if matches!(self.peek(), Some(b'0'..=b'9')) {
                let digits = self.parse_digits()?;
                Some(Cow::Borrowed(digits))
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
    /// The ABI spells it `_ <number>` or `__ <number> _`; GCC also emits bare
    /// digits after the entity name, which the reference demangler accepts.
    fn parse_discriminator(&mut self) -> Result<Option<Cow<'a, str>>, ParseError> {
        match (self.peek(), self.peek_at(1)) {
            (Some(b'_'), Some(b'_')) => {
                self.pos += 2;
                let digits = self.parse_digits()?;
                self.expect(b'_')?;
                Ok(Some(Cow::Borrowed(digits)))
            }
            (Some(b'_'), Some(b'0'..=b'9')) => {
                self.pos += 1;
                let digits = self.parse_digits()?;
                Ok(Some(Cow::Borrowed(digits)))
            }
            (Some(b'0'..=b'9'), _) => {
                let digits = self.parse_digits()?;
                Ok(Some(Cow::Borrowed(digits)))
            }
            _ => Ok(None),
        }
    }

    fn parse_unscoped_name(&mut self) -> Result<NodeRef<'a>, ParseError> {
        let start = self.pos;
        if self.starts_with(b"St") {
            self.pos += 2;
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
            let base = self.parse_substitution()?;
            if self.peek() == Some(b'I') {
                let arguments = self.parse_template_args()?;
                return self.make(Kind::Name, None, start, alloc::vec![base, arguments]);
            }
            return Ok(base);
        }
        if self.peek() == Some(b'W') {
            // Vendor extended qualifiers chain with the name they qualify:
            // `W3FOOW3BAR3Foo` is `Foo@FOO.BAR`.
            let mut name = self.parse_vendor_qualifier()?;
            loop {
                if self.peek() == Some(b'W') && matches!(self.peek_at(1), Some(b'0'..=b'9')) {
                    let qualifier = self.parse_vendor_qualifier()?;
                    name =
                        self.make(Kind::NestedName, None, start, alloc::vec![name, qualifier])?;
                } else if matches!(self.peek(), Some(b'0'..=b'9' | b'L')) {
                    let unqualified = self.parse_unqualified_name()?;
                    name = self.make(
                        Kind::NestedName,
                        None,
                        start,
                        alloc::vec![name, unqualified],
                    )?;
                    break;
                } else {
                    break;
                }
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

    fn parse_nested_name_inner(&mut self) -> Result<NodeRef<'a>, ParseError> {
        let start = self.pos;
        self.expect(b'N')?;
        let mut restrict = false;
        let mut volatile = false;
        let mut const_ = false;
        let mut ref_qual = 0_u8;
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
        let qualifier = name_qualifier_text(const_, volatile, restrict, ref_qual);

        let mut so_far: Option<NodeRef<'a>> = None;
        let mut pushed_last = false;
        loop {
            self.tick()?;
            if self.eat(b'E') {
                break;
            }
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
                    let base_start = self.span_start(&base).unwrap_or(start);
                    let arguments = self.parse_template_args()?;
                    so_far = Some(self.make(
                        Kind::Name,
                        None,
                        base_start,
                        alloc::vec![base, arguments],
                    )?);
                }
                Some(b'S') => {
                    if so_far.is_some() {
                        return Err(self.error(ParseErrorKind::UnexpectedByte));
                    }
                    if self.starts_with(b"St") {
                        let std_start = self.pos;
                        self.pos += 2;
                        self.tick()?;
                        so_far = Some(self.make(
                            Kind::Substitution,
                            Some(Cow::Borrowed("std")),
                            std_start,
                            Vec::new(),
                        )?);
                    } else {
                        so_far = Some(self.parse_substitution()?);
                    }
                    pushed_last = false;
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
            pushed_last = true;
            let _ = self.eat(b'M');
        }
        if pushed_last {
            let _ = self.subs.pop();
        }
        let name = so_far.ok_or_else(|| self.error(ParseErrorKind::UnexpectedEnd))?;
        let Some(text) = qualifier else {
            return Ok(name);
        };
        let name_start = self.span_start(&name).unwrap_or(start);
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

    fn parse_unqualified_name_leaf(&mut self) -> Result<NodeRef<'a>, ParseError> {
        let start = self.pos;
        self.tick()?;
        if self.peek() == Some(b'L') && matches!(self.peek_at(1), Some(b'0'..=b'9')) {
            // Internal-linkage marker. It does not change the name.
            self.pos += 1;
            self.tick()?;
        }
        if self.starts_with(b"Ut") {
            self.pos += 2;
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
            self.pos += 2;
            self.tick()?;
            let _ = self.parse_number()?;
            self.expect(b'_')?;
            return self.make(
                Kind::Lambda,
                Some(Cow::Borrowed("block-literal")),
                start,
                Vec::new(),
            );
        }
        if self.peek() == Some(b'W') && matches!(self.peek_at(1), Some(b'0'..=b'9')) {
            return self.parse_vendor_qualifier();
        }
        if let Some(code) = self.ctor_dtor_code() {
            self.pos += code.len();
            self.tick()?;
            return self.make(
                Kind::CtorDtorName,
                Some(Cow::Borrowed(code)),
                start,
                Vec::new(),
            );
        }
        if self.starts_with(b"cv") {
            self.pos += 2;
            self.tick()?;
            let target = self.parse_type()?;
            return self.make(
                Kind::OperatorName,
                Some(Cow::Borrowed("operator")),
                start,
                alloc::vec![target],
            );
        }
        if self.starts_with(b"li") {
            self.pos += 2;
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
            self.pos += 2;
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

    /// Parse `W <source-name>`, the MIPS vendor extended qualifier.
    fn parse_vendor_qualifier(&mut self) -> Result<NodeRef<'a>, ParseError> {
        let start = self.pos;
        self.expect(b'W')?;
        self.tick()?;
        let text = self.parse_source_name_text()?;
        self.make(
            Kind::VendorQualifier,
            Some(Cow::Borrowed(text)),
            start,
            Vec::new(),
        )
    }

    /// Parse `Ul <lambda-sig> E [<number>] _`.
    ///
    /// A lambda signature is a list of template parameter declarations
    /// followed by the parameter types.
    fn parse_lambda(&mut self) -> Result<NodeRef<'a>, ParseError> {
        let start = self.pos;
        self.expect(b'U')?;
        self.expect(b'l')?;
        let mut children = Vec::new();
        while is_template_param_decl(self.peek(), self.peek_at(1)) {
            children.push(self.parse_template_param_decl()?);
        }
        if self.peek() == Some(b'v') && self.peek_at(1) == Some(b'E') {
            self.pos += 1;
            self.tick()?;
        } else {
            while self.peek() != Some(b'E') {
                if self.peek().is_none() {
                    return Err(self.error(ParseErrorKind::UnexpectedEnd));
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
            self.pos += 1;
            self.tick()?;
            let text = self.parse_source_name_text()?;
            let start = self.span_start(&name).unwrap_or(self.pos);
            name = self.make(
                Kind::AbiTag,
                Some(Cow::Borrowed(text)),
                start,
                alloc::vec![name],
            )?;
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

    fn operator_spelling(&self) -> Option<&'static str> {
        const OPERATORS: &[(&str, &str)] = &[
            ("nw", "operator new"),
            ("na", "operator new[]"),
            ("dl", "operator delete"),
            ("da", "operator delete[]"),
            ("ps", "operator+"),
            ("ng", "operator-"),
            ("ad", "operator&"),
            ("de", "operator*"),
            ("co", "operator~"),
            ("pl", "operator+"),
            ("mi", "operator-"),
            ("ml", "operator*"),
            ("dv", "operator/"),
            ("rm", "operator%"),
            ("an", "operator&"),
            ("or", "operator|"),
            ("eo", "operator^"),
            ("aS", "operator="),
            ("pL", "operator+="),
            ("mI", "operator-="),
            ("mL", "operator*="),
            ("dV", "operator/="),
            ("rM", "operator%="),
            ("aN", "operator&="),
            ("oR", "operator|="),
            ("eO", "operator^="),
            ("ls", "operator<<"),
            ("rs", "operator>>"),
            ("lS", "operator<<="),
            ("rS", "operator>>="),
            ("eq", "operator=="),
            ("ne", "operator!="),
            ("lt", "operator<"),
            ("gt", "operator>"),
            ("le", "operator<="),
            ("ge", "operator>="),
            ("ss", "operator<=>"),
            ("nt", "operator!"),
            ("aa", "operator&&"),
            ("oo", "operator||"),
            ("pp", "operator++"),
            ("mm", "operator--"),
            ("cm", "operator,"),
            ("pm", "operator->*"),
            ("pt", "operator->"),
            ("cl", "operator()"),
            ("ix", "operator[]"),
            ("qu", "operator?"),
            ("st", "operator sizeof"),
            ("sz", "operator sizeof..."),
            ("at", "operator alignof"),
            ("az", "operator alignof..."),
            ("nx", "operator noexcept"),
            ("aw", "operator co_await"),
        ];
        for &(code, spelling) in OPERATORS {
            if self.starts_with(code.as_bytes()) {
                return Some(spelling);
            }
        }
        None
    }

    fn parse_source_name(&mut self) -> Result<NodeRef<'a>, ParseError> {
        let start = self.pos;
        let text = self.parse_source_name_text()?;
        self.make(
            Kind::Identifier,
            Some(Cow::Borrowed(text)),
            start,
            Vec::new(),
        )
    }

    /// Parse `<source-name>` and return the identifier text it borrows.
    fn parse_source_name_text(&mut self) -> Result<&'a str, ParseError> {
        let length = self.parse_number()?;
        if length == 0 {
            return Err(self.error(ParseErrorKind::InvalidLength));
        }
        let text_start = self.pos;
        let text_end = text_start
            .checked_add(length)
            .ok_or_else(|| self.error(ParseErrorKind::InvalidLength))?;
        if text_end > self.bytes.len() {
            return Err(self.error(ParseErrorKind::UnexpectedEnd));
        }
        let symbol = self.symbol;
        let text = symbol
            .get(text_start..text_end)
            .ok_or_else(|| self.error(ParseErrorKind::InvalidLength))?;
        self.pos = text_end;
        self.work = self.work.saturating_add(length);
        if self.work > self.work_limit {
            return Err(self.error(ParseErrorKind::TooMuchWork));
        }
        Ok(text)
    }

    // -- substitutions -----------------------------------------------------

    fn parse_substitution(&mut self) -> Result<NodeRef<'a>, ParseError> {
        let start = self.pos;
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
            self.pos += 1;
            self.tick()?;
            return self.make(
                Kind::Substitution,
                Some(Cow::Borrowed(text)),
                start,
                Vec::new(),
            );
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

    fn parse_seq_id(&mut self) -> Result<usize, ParseError> {
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
            self.pos += 1;
            self.tick()?;
            any = true;
        }
        if !any {
            return Err(self.error(ParseErrorKind::UnexpectedByte));
        }
        self.expect(b'_')?;
        Ok(value)
    }

    // -- template arguments ------------------------------------------------

    fn parse_template_args(&mut self) -> Result<NodeRef<'a>, ParseError> {
        let start = self.pos;
        self.expect(b'I')?;
        let mut arguments = Vec::new();
        loop {
            self.tick()?;
            if self.eat(b'E') {
                break;
            }
            if self.peek().is_none() {
                return Err(self.error(ParseErrorKind::UnexpectedEnd));
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
                let start = self.pos;
                self.pos += 1;
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
                self.make(Kind::Pack, None, start, elements)
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
        let start = self.pos;
        self.expect(b'T')?;
        self.tick()?;
        let keyword = match self.peek() {
            Some(b'y') => {
                self.pos += 1;
                self.tick()?;
                "Ty"
            }
            Some(b'n') => {
                self.pos += 1;
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
                self.pos += 1;
                self.tick()?;
                let mut children = alloc::vec![self.parse_name()?];
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
                self.pos += 1;
                self.tick()?;
                let mut children = Vec::new();
                while !self.eat(b'E') {
                    if self.peek().is_none() {
                        return Err(self.error(ParseErrorKind::UnexpectedEnd));
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
                self.pos += 1;
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
    fn parse_literal(&mut self) -> Result<NodeRef<'a>, ParseError> {
        let start = self.pos;
        self.expect(b'L')?;
        if self.peek() == Some(b'_') {
            if self.peek_at(1) != Some(b'Z') {
                return Err(self.error(ParseErrorKind::UnexpectedByte));
            }
            self.pos += 2;
            self.nested_encoding += 1;
            let encoding = self.parse_encoding();
            self.nested_encoding -= 1;
            let encoding = encoding?;
            self.expect(b'E')?;
            return self.make(Kind::Literal, None, start, alloc::vec![encoding]);
        }
        let ty = self.parse_type()?;
        let value_start = self.pos;
        while self.peek().is_some_and(|byte| byte != b'E') {
            self.pos += 1;
            self.tick()?;
        }
        let value_end = self.pos;
        self.expect(b'E')?;
        let symbol = self.symbol;
        let text = if value_start == value_end {
            None
        } else {
            let value = symbol
                .get(value_start..value_end)
                .ok_or_else(|| self.error(ParseErrorKind::InvalidLength))?;
            Some(Cow::Borrowed(value))
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
        let start = self.pos;
        if self.starts_with(b"pp_") || self.starts_with(b"mm_") {
            let spelling = if self.starts_with(b"pp_") {
                "operator++"
            } else {
                "operator--"
            };
            self.pos += 3;
            self.tick()?;
            let operand = self.parse_expression()?;
            return self.make(
                Kind::Expression,
                Some(Cow::Borrowed(spelling)),
                start,
                alloc::vec![operand],
            );
        }
        let Some(first) = self.peek() else {
            return Err(self.error(ParseErrorKind::UnexpectedEnd));
        };
        let second = self.peek_at(1);
        if let Some(second) = second {
            if let Some(form) = expression_form(first, second) {
                return self.parse_expression_form(start, form);
            }
            if let Some(spelling) = self.operator_spelling() {
                let arity = expression_arity(first, second);
                self.pos += 2;
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
        if first == b'L' {
            return self.parse_literal();
        }
        Err(self.error(ParseErrorKind::Unsupported))
    }

    fn parse_expression_form(
        &mut self,
        start: usize,
        form: ExpressionForm,
    ) -> Result<NodeRef<'a>, ParseError> {
        self.pos += 2;
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
                    operands.push(self.parse_expression()?);
                }
            }
            ExpressionForm::InitializerList => {
                while !self.eat(b'E') {
                    if self.peek().is_none() {
                        return Err(self.error(ParseErrorKind::UnexpectedEnd));
                    }
                    operands.push(self.parse_expression()?);
                }
            }
            ExpressionForm::MemberAccess => {
                operands.push(self.parse_expression()?);
                operands.push(self.parse_name()?);
            }
            ExpressionForm::SizeofType
            | ExpressionForm::AlignofType
            | ExpressionForm::Throw
            | ExpressionForm::TypeidType => {
                operands.push(self.parse_type()?);
            }
            ExpressionForm::SizeofExpr
            | ExpressionForm::AlignofExpr
            | ExpressionForm::Noexcept
            | ExpressionForm::TypeidExpr
            | ExpressionForm::PackExpansion => {
                operands.push(self.parse_expression()?);
            }
            ExpressionForm::Cast => {
                operands.push(self.parse_type()?);
                operands.push(self.parse_expression()?);
            }
            ExpressionForm::SizeofPack => {
                let parameter = self.parse_template_param()?;
                operands.push(parameter);
            }
            ExpressionForm::Fold => {
                operands.push(self.parse_expression()?);
                operands.push(self.parse_expression()?);
            }
        }
        self.make(
            Kind::Expression,
            Some(Cow::Borrowed(form.text())),
            start,
            operands,
        )
    }

    // -- types -------------------------------------------------------------

    fn parse_type(&mut self) -> Result<NodeRef<'a>, ParseError> {
        self.enter()?;
        let result = self.parse_type_inner();
        self.leave();
        result
    }

    fn parse_type_inner(&mut self) -> Result<NodeRef<'a>, ParseError> {
        self.tick()?;
        let start = self.pos;
        let Some(byte) = self.peek() else {
            return Err(self.error(ParseErrorKind::UnexpectedEnd));
        };
        match byte {
            b'v' | b'w' | b'b' | b'c' | b'a' | b'h' | b's' | b't' | b'i' | b'j' | b'l' | b'm'
            | b'x' | b'y' | b'n' | b'o' | b'f' | b'd' | b'e' | b'g' | b'z' => {
                let name =
                    builtin_name(byte).ok_or_else(|| self.error(ParseErrorKind::Unsupported))?;
                self.pos += 1;
                self.tick()?;
                self.make(
                    Kind::BuiltinType,
                    Some(Cow::Borrowed(name)),
                    start,
                    Vec::new(),
                )
            }
            b'D' if matches!(self.peek_at(1), Some(b't' | b'T')) => {
                self.pos += 2;
                self.tick()?;
                let expression = self.parse_expression()?;
                self.expect(b'E')?;
                let node = self.make(Kind::Decltype, None, start, alloc::vec![expression])?;
                self.push_sub(Rc::clone(&node));
                Ok(node)
            }
            b'D' => self.parse_d_type(start),
            b'u' => {
                self.pos += 1;
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
                self.pos += 1;
                self.tick()?;
                let inner = self.parse_type()?;
                let node = self.make(Kind::PointerType, None, start, alloc::vec![inner])?;
                self.push_sub(Rc::clone(&node));
                Ok(node)
            }
            b'R' => {
                self.pos += 1;
                self.tick()?;
                let inner = self.parse_type()?;
                let node = self.make(Kind::ReferenceType, None, start, alloc::vec![inner])?;
                self.push_sub(Rc::clone(&node));
                Ok(node)
            }
            b'O' => {
                self.pos += 1;
                self.tick()?;
                let inner = self.parse_type()?;
                let node = self.make(Kind::RvalueReferenceType, None, start, alloc::vec![inner])?;
                self.push_sub(Rc::clone(&node));
                Ok(node)
            }
            b'K' | b'V' | b'r' => self.parse_qualified_type(start),
            b'A' => self.parse_array_type(start),
            b'F' => self.parse_function_type(start),
            b'M' => self.parse_member_pointer(start),
            b'T' => {
                if matches!(self.peek_at(1), Some(b's' | b'u' | b'e')) {
                    return Err(self.error(ParseErrorKind::Unsupported));
                }
                let base = self.parse_template_param()?;
                if self.peek() == Some(b'I') {
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
                    let base = self.parse_substitution()?;
                    if self.peek() == Some(b'I') {
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
            b'N' | b'Z' | b'L' | b'W' | b'U' | b'0'..=b'9' => {
                let node = self.parse_name()?;
                self.push_sub(Rc::clone(&node));
                Ok(node)
            }
            _ => Err(self.error(ParseErrorKind::Unsupported)),
        }
    }

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
                self.pos += 1;
                self.tick()?;
                let digits_start = self.pos;
                let _width = self.parse_number()?;
                let digits_end = self.pos;
                let symbol = self.symbol;
                let digits = symbol
                    .get(digits_start..digits_end)
                    .ok_or_else(|| self.error(ParseErrorKind::InvalidLength))?;
                let text = if self.eat(b'b') {
                    Cow::Borrowed("std::bfloat16_t")
                } else {
                    self.expect(b'_')?;
                    Cow::Owned(alloc::format!("_Float{digits}"))
                };
                let node = self.make(Kind::BuiltinType, Some(text), start, Vec::new())?;
                // libc++abi records parametrized binary-float builtins in the
                // substitution table; follow it so `S<n>_` indices match.
                self.push_sub(Rc::clone(&node));
                Ok(node)
            }
            b'B' | b'U' => {
                self.pos += 1;
                self.tick()?;
                let unsigned = byte == b'U' || self.eat(b'u');
                let digits_start = self.pos;
                let _bits = self.parse_number()?;
                let digits_end = self.pos;
                self.expect(b'_')?;
                let symbol = self.symbol;
                let digits = symbol
                    .get(digits_start..digits_end)
                    .ok_or_else(|| self.error(ParseErrorKind::InvalidLength))?;
                let text = if unsigned {
                    alloc::format!("unsigned _BitInt({digits})")
                } else {
                    alloc::format!("_BitInt({digits})")
                };
                let node =
                    self.make(Kind::BuiltinType, Some(Cow::Owned(text)), start, Vec::new())?;
                self.push_sub(Rc::clone(&node));
                Ok(node)
            }
            b'A' | b'R' => self.fixed_point_type(start, false),
            b'S' => {
                self.pos += 1;
                self.tick()?;
                self.expect(b'D')?;
                self.fixed_point_type(start, true)
            }
            b'p' => {
                self.pos += 1;
                self.tick()?;
                let pattern = self.parse_type()?;
                let node = self.make(Kind::Pack, None, start, alloc::vec![pattern])?;
                self.push_sub(Rc::clone(&node));
                Ok(node)
            }
            _ => Err(self.error(ParseErrorKind::Unsupported)),
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
        self.pos += 1;
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
        self.pos += 1;
        self.tick()?;
        let text = if saturated {
            alloc::format!("_Sat {size}{kind}")
        } else {
            alloc::format!("{size}{kind}")
        };
        self.make(Kind::BuiltinType, Some(Cow::Owned(text)), start, Vec::new())
    }

    fn builtin_d(&mut self, start: usize, name: &'static str) -> Result<NodeRef<'a>, ParseError> {
        self.pos += 1;
        self.tick()?;
        self.make(
            Kind::BuiltinType,
            Some(Cow::Borrowed(name)),
            start,
            Vec::new(),
        )
    }

    fn parse_qualified_type(&mut self, start: usize) -> Result<NodeRef<'a>, ParseError> {
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
        if self.peek() == Some(b'U') {
            return Err(self.error(ParseErrorKind::Unsupported));
        }
        let inner = self.parse_type()?;
        let text = qualifier_text(const_, volatile, restrict);
        let node = self.make(
            Kind::QualifiedType,
            Some(Cow::Borrowed(text)),
            start,
            alloc::vec![inner],
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
                Some(Cow::Borrowed(digits)),
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
        self.expect(b'F')?;
        let extern_c = self.eat(b'Y');
        if matches!(self.peek(), Some(b'D')) && matches!(self.peek_at(1), Some(b'o' | b'O' | b'w'))
        {
            // Exception specifications.
            return Err(self.error(ParseErrorKind::Unsupported));
        }
        let return_type = self.parse_type()?;
        let parameters = self.parse_function_parameters()?;
        let ref_qualifier = if self.peek() == Some(b'R') && self.peek_at(1) == Some(b'E') {
            self.pos += 1;
            self.tick()?;
            Some("&")
        } else if self.peek() == Some(b'O') && self.peek_at(1) == Some(b'E') {
            self.pos += 1;
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
        let node = self.make(
            Kind::FunctionType,
            text,
            start,
            alloc::vec![return_type, parameters],
        )?;
        self.push_sub(Rc::clone(&node));
        Ok(node)
    }

    fn parse_function_parameters(&mut self) -> Result<NodeRef<'a>, ParseError> {
        let start = self.pos;
        let mut parameters = Vec::new();
        if self.peek() == Some(b'v') && self.void_parameter_ends_function_type() {
            self.pos += 1;
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
        let start = self.pos;
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
        let symbol = self.symbol;
        let text = symbol
            .get(start..self.pos)
            .ok_or_else(|| self.error(ParseErrorKind::InvalidLength))?;
        self.make(
            Kind::TemplateParam,
            Some(Cow::Borrowed(text)),
            start,
            Vec::new(),
        )
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
            self.pos += 1;
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
    fn parse_digits(&mut self) -> Result<&'a str, ParseError> {
        let start = self.pos;
        let _ = self.parse_number()?;
        let end = self.pos;
        self.symbol
            .get(start..end)
            .ok_or_else(|| self.error(ParseErrorKind::InvalidLength))
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
    Fold,
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
            Self::SizeofType | Self::SizeofExpr => "sizeof",
            Self::AlignofType | Self::AlignofExpr => "alignof",
            Self::Noexcept => "noexcept",
            Self::Throw | Self::Rethrow => "throw",
            Self::TypeidType | Self::TypeidExpr => "typeid",
            Self::Cast => "cast",
            Self::PackExpansion => "pack",
            Self::SizeofPack => "sizeof-pack",
            Self::Fold => "fold",
        }
    }
}

/// The expression form for a two-letter code, if it is not a plain operator.
fn expression_form(first: u8, second: u8) -> Option<ExpressionForm> {
    Some(match (first, second) {
        (b'c', b'l') => ExpressionForm::Call,
        (b'c', b'v') => ExpressionForm::Conversion,
        (b't', b'l') => ExpressionForm::TypeList,
        (b'i', b'l') => ExpressionForm::InitializerList,
        (b'd', b't' | b's') => ExpressionForm::MemberAccess,
        (b's', b't') => ExpressionForm::SizeofType,
        (b's', b'z') => ExpressionForm::SizeofExpr,
        (b'a', b't') => ExpressionForm::AlignofType,
        (b'a', b'z') => ExpressionForm::AlignofExpr,
        (b'n', b'x') => ExpressionForm::Noexcept,
        (b't', b'w') => ExpressionForm::Throw,
        (b't', b'r') => ExpressionForm::Rethrow,
        (b't', b'i') => ExpressionForm::TypeidType,
        (b't', b'e') => ExpressionForm::TypeidExpr,
        (b'd', b'c') | (b's', b'c') | (b'c', b'c') | (b'r', b'c') => ExpressionForm::Cast,
        (b's', b'p') => ExpressionForm::PackExpansion,
        (b's', b'Z') => ExpressionForm::SizeofPack,
        (b'f', b'L' | b'R' | b'l' | b'r') => ExpressionForm::Fold,
        _ => return None,
    })
}

/// Number of operands an expression operator takes.
fn expression_arity(first: u8, second: u8) -> u8 {
    match (first, second) {
        (b'q', b'u') => 3,
        (b'p', b's')
        | (b'n', b'g')
        | (b'a', b'd')
        | (b'd', b'e')
        | (b'c', b'o')
        | (b'n', b't')
        | (b'p', b'p')
        | (b'm', b'm')
        | (b'd', b'l')
        | (b'd', b'a')
        | (b'n', b'w')
        | (b'n', b'a') => 1,
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

/// Builtin type names for the single-letter codes.
fn builtin_name(byte: u8) -> Option<&'static str> {
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
fn qualifier_text(const_: bool, volatile: bool, restrict: bool) -> &'static str {
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

    use super::super::{Kind, NodeRef, parse};
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
    fn rejects_remaining_unsupported_productions() {
        for symbol in [
            "_ZGIW3Foo",
            "_Z1fIXtl1BadsoiL_Z6nestedE_EEEEvv",
            "_ZN6test181fINS_1AEEEvNS_1SIXadsrT_plEEE",
        ] {
            let error = parse(symbol).unwrap_err();
            assert_eq!(error.kind(), ParseErrorKind::Unsupported, "{symbol}");
        }
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
