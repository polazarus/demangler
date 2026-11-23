//! Microsoft C++ grammar parser.
//!
//! Microsoft does not publish a grammar for its decorated names, so this
//! parser follows LLVM's `MicrosoftDemangle.cpp` productions. It builds the
//! same immutable [`Node`] graph as the Itanium parser; the Microsoft-only
//! shapes are the [`Kind::Variable`], [`Kind::MemberFunction`],
//! [`Kind::Vftable`], [`Kind::Vbtable`], [`Kind::Rtti`],
//! [`Kind::DynamicInitializer`], [`Kind::DynamicAtexitDestructor`],
//! [`Kind::StringLiteral`], [`Kind::Md5Name`], [`Kind::VCallThunk`] and
//! [`Kind::TaggedType`] variants.
//!
//! Back-references share the referenced node handle, so the graph may be a
//! DAG. The parser rejects productions it cannot model instead of emitting
//! [`Kind::Opaque`] nodes.

use alloc::{borrow::Cow, rc::Rc, string::String, vec::Vec};

use crate::parse::{Cursor, Fail, FailKind, State};

use super::node::{Kind, Node, NodeRef};
use super::{Ast, MAX_INPUT_LEN, ParseError, ParseErrorKind, decode, owned_text};

/// Maximum nesting depth for recursive productions.
const MAX_DEPTH: u32 = 256;
/// Maximum nodes one parse may allocate.
const MAX_NODES: u32 = 1 << 14;
/// Minimum work allowance for very short symbols.
const MIN_WORK: usize = 1 << 12;
/// Work allowance per input byte.
const WORK_PER_BYTE: usize = 32;

/// Map a cursor failure back onto the public C++ parse error.
fn cursor_error(fail: Fail) -> ParseError {
    let kind = match fail.kind() {
        FailKind::EndOfInput => ParseErrorKind::UnexpectedEnd,
        FailKind::InputBudget => ParseErrorKind::TooLong,
        FailKind::LengthOverflow => ParseErrorKind::InvalidLength,
        FailKind::Work => ParseErrorKind::TooMuchWork,
        FailKind::Depth => ParseErrorKind::TooDeep,
        FailKind::Node => ParseErrorKind::TooManyNodes,
        FailKind::Invalid => ParseErrorKind::UnexpectedByte,
    };
    ParseError::new(fail.offset as usize, kind)
}

/// Maximum number of names or function parameter types that can be
/// back-referenced. Microsoft encodes references as a single decimal digit.
const MAX_BACKREFS: usize = 10;

/// Parse a Microsoft C++ mangled name into an inspectable AST.
///
/// Accepts the usual `?`-prefixed symbols and the `.`-prefixed RTTI type
/// descriptor name form.
///
/// # Errors
///
/// Returns [`ParseError`] when the symbol is not a Microsoft mangling, uses an
/// unsupported production, is malformed, or exceeds the parser's depth, node
/// or work budgets.
pub fn parse_msvc(symbol: &str) -> Result<Ast<'_>, ParseError> {
    parse_msvc_bytes(symbol.as_bytes())
}

/// Parse a Microsoft C++ mangled name from raw bytes.
pub(crate) fn parse_msvc_bytes(symbol: &[u8]) -> Result<Ast<'_>, ParseError> {
    if !(symbol.starts_with(b"?") || symbol.starts_with(b".?")) {
        return Err(ParseError::new(0, ParseErrorKind::NotMsvc));
    }
    let mut state = msvc_state(symbol.len());
    Parser::new(symbol, &mut state)?.parse()
}

/// Parser budget for the Microsoft parser.
///
/// The parser keeps its own depth, node and work accounting, so the cursor's
/// counters are only used to bound the input length.
fn msvc_state(input_len: usize) -> State {
    State::new(crate::parse::Limits {
        input: u32::try_from(input_len).unwrap_or(u32::MAX),
        depth: u16::MAX,
        work: u32::MAX,
    })
}

/// CV, extension and calling-convention qualifiers attached to a type.
#[derive(Clone, Copy, Default)]
struct Quals {
    const_: bool,
    volatile: bool,
    restrict: bool,
    unaligned: bool,
    ptrauth: Option<(u64, u64, u64)>,
}

impl Quals {
    fn is_empty(self) -> bool {
        !(self.const_ || self.volatile || self.restrict || self.unaligned) && self.ptrauth.is_none()
    }

    fn text(self) -> Option<String> {
        let mut parts: Vec<String> = Vec::new();
        if self.const_ {
            parts.push("const".into());
        }
        if self.volatile {
            parts.push("volatile".into());
        }
        if self.restrict {
            parts.push("__restrict".into());
        }
        if self.unaligned {
            parts.push("__unaligned".into());
        }
        if let Some((a, b, c)) = self.ptrauth {
            parts.push(alloc::format!("__ptrauth({a}, {b}, {c})"));
        }
        if parts.is_empty() {
            None
        } else {
            Some(parts.join(" "))
        }
    }

    fn union(self, other: Self) -> Self {
        Self {
            const_: self.const_ || other.const_,
            volatile: self.volatile || other.volatile,
            restrict: self.restrict || other.restrict,
            unaligned: self.unaligned || other.unaligned,
            ptrauth: self.ptrauth.or(other.ptrauth),
        }
    }
}

/// How an unqualified name should be memorized for back-references.
#[derive(Clone, Copy, PartialEq, Eq)]
enum NameBackref {
    /// Memorize a simple name.
    Simple,
    /// Memorize a complete template-id.
    Template,
}

/// A parsed qualified name, before the scope components are folded together.
struct ParsedName<'a> {
    base: NodeRef<'a>,
    /// Scope components in mangling order: nearest scope first.
    scope: Vec<NodeRef<'a>>,
}

/// A parsed function signature.
struct Signature<'a> {
    ret: Option<NodeRef<'a>>,
    params: NodeRef<'a>,
    calling_conv: &'static str,
    quals: Quals,
    ref_qual: Option<&'static str>,
}

/// Microsoft function storage/access/virtual classification.
#[derive(Clone, Copy, Default)]
struct FuncClass {
    access: Option<&'static str>,
    static_: bool,
    virtual_: bool,
    global: bool,
    no_param_list: bool,
    static_this_adjust: bool,
    virtual_this_adjust: bool,
    virtual_this_adjust_ex: bool,
}

/// Recursive-descent parser over one Microsoft mangled symbol.
struct Parser<'a, 's> {
    input: Cursor<'a, 's>,
    bytes: &'a [u8],
    names: Vec<NodeRef<'a>>,
    types: Vec<NodeRef<'a>>,
    depth: u32,
    nodes: u32,
    work: usize,
    work_limit: usize,
}

impl<'a, 's> Parser<'a, 's> {
    fn new(bytes: &'a [u8], state: &'s mut State) -> Result<Self, ParseError> {
        if bytes.len() > MAX_INPUT_LEN {
            return Err(ParseError::new(0, ParseErrorKind::TooLong));
        }
        let work_limit = bytes
            .len()
            .saturating_mul(WORK_PER_BYTE)
            .saturating_add(MIN_WORK);
        Ok(Self {
            input: Cursor::new(bytes, 0, state).map_err(cursor_error)?,
            bytes,
            names: Vec::new(),
            types: Vec::new(),
            depth: 0,
            nodes: 0,
            work: 0,
            work_limit,
        })
    }

    fn parse(mut self) -> Result<Ast<'a>, ParseError> {
        let start = self.input.pos();
        let shape = self.parse_symbol_shape()?;
        // Like `llvm-undname`, ignore trailing characters after a complete
        // encoding. The RTTI type descriptor and typeinfo name parsers keep
        // their own exact end-of-input check, which rejects trailing text.
        let root = self.make(Kind::Encoding, None, start, alloc::vec![shape])?;
        Ok(Ast::from_root_msvc(root))
    }

    // -- primitives --------------------------------------------------------

    fn error(&self, kind: ParseErrorKind) -> ParseError {
        ParseError::new(self.input.pos(), kind)
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

    fn consume(&mut self, text: &[u8]) -> bool {
        if self.starts_with(text) {
            let _ = self.input.advance(text.len());
            self.tick_n(text.len());
            true
        } else {
            false
        }
    }

    fn expect_text(&mut self, text: &[u8]) -> Result<(), ParseError> {
        if self.consume(text) {
            Ok(())
        } else if self.input.pos() + text.len() > self.bytes.len() {
            Err(self.error(ParseErrorKind::UnexpectedEnd))
        } else {
            Err(self.error(ParseErrorKind::UnexpectedByte))
        }
    }

    fn tick_n(&mut self, count: usize) {
        self.work = self.work.saturating_add(count);
    }

    fn tick(&mut self) -> Result<(), ParseError> {
        self.tick_n(1);
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
        let end = self.input.pos();
        let span = if start <= end && end <= self.bytes.len() {
            Some(start as u32..end as u32)
        } else {
            None
        };
        Ok(Node::new(kind, text, span, children))
    }

    fn span_start(&self, node: &NodeRef<'a>) -> Option<usize> {
        node.span().map(|range| range.start as usize)
    }

    fn node_start(&self, node: &NodeRef<'a>) -> usize {
        self.span_start(node).unwrap_or_else(|| self.input.pos())
    }

    // -- budgets and back-references --------------------------------------

    fn memorize_name(&mut self, node: NodeRef<'a>) {
        if self.names.len() >= MAX_BACKREFS {
            return;
        }
        if self
            .names
            .iter()
            .any(|existing| Rc::ptr_eq(existing, &node))
        {
            return;
        }
        if let Some(key) = node_key(&node)
            && self
                .names
                .iter()
                .any(|existing| node_key(existing).as_deref() == Some(key.as_str()))
        {
            return;
        }
        self.names.push(node);
    }

    fn backref_name(&mut self) -> Result<NodeRef<'a>, ParseError> {
        let index = usize::from(
            self.peek()
                .ok_or_else(|| self.error(ParseErrorKind::UnexpectedEnd))?
                - b'0',
        );
        self.bump(1)?;
        self.tick()?;
        self.names
            .get(index)
            .cloned()
            .ok_or_else(|| self.error(ParseErrorKind::UnknownSubstitution))
    }

    fn backref_type(&mut self) -> Result<NodeRef<'a>, ParseError> {
        let index = usize::from(
            self.peek()
                .ok_or_else(|| self.error(ParseErrorKind::UnexpectedEnd))?
                - b'0',
        );
        self.bump(1)?;
        self.tick()?;
        self.types
            .get(index)
            .cloned()
            .ok_or_else(|| self.error(ParseErrorKind::UnknownSubstitution))
    }

    fn backref_parameter(&mut self) -> Result<NodeRef<'a>, ParseError> {
        self.backref_type()
    }

    /// Remember a parameter type for the first ten back-reference slots.
    ///
    /// Microsoft numbers parameter back-references from the same table as
    /// type back-references, and deduplicates equal types.
    fn memorize_type(&mut self, node: NodeRef<'a>) {
        if self.types.len() >= MAX_BACKREFS {
            return;
        }
        if self
            .types
            .iter()
            .any(|existing| Rc::ptr_eq(existing, &node))
        {
            return;
        }
        if let Some(key) = node_key(&node)
            && self
                .types
                .iter()
                .any(|existing| node_key(existing).as_deref() == Some(key.as_str()))
        {
            return;
        }
        self.types.push(node);
    }

    // -- numbers and strings ----------------------------------------------

    /// Decode a Microsoft `<number>`.
    ///
    /// A decimal digit `0`-`9` encodes `1`-`10`; a run of `A`-`P` hex digits
    /// terminated by `@` encodes any other value.
    fn read_number(&mut self) -> Result<(u64, bool), ParseError> {
        let negative = self.eat(b'?');
        if let Some(byte @ b'0'..=b'9') = self.peek() {
            self.bump(1)?;
            self.tick()?;
            return Ok((u64::from(byte - b'0') + 1, negative));
        }
        let mut value = 0_u64;
        let mut any = false;
        loop {
            match self.peek() {
                Some(b'@') => {
                    self.bump(1)?;
                    self.tick()?;
                    if !any {
                        return Ok((0, negative));
                    }
                    return Ok((value, negative));
                }
                Some(byte @ b'A'..=b'P') => {
                    value = value
                        .checked_mul(16)
                        .and_then(|current| current.checked_add(u64::from(byte - b'A')))
                        .ok_or_else(|| self.error(ParseErrorKind::InvalidLength))?;
                    self.bump(1)?;
                    self.tick()?;
                    any = true;
                }
                Some(_) => return Err(self.error(ParseErrorKind::InvalidLength)),
                None => return Err(self.error(ParseErrorKind::UnexpectedEnd)),
            }
        }
    }

    fn read_unsigned(&mut self) -> Result<u64, ParseError> {
        let (value, negative) = self.read_number()?;
        if negative {
            return Err(self.error(ParseErrorKind::InvalidLength));
        }
        Ok(value)
    }

    fn read_signed(&mut self) -> Result<i64, ParseError> {
        let (value, negative) = self.read_number()?;
        let value = i64::try_from(value).map_err(|_| self.error(ParseErrorKind::InvalidLength))?;
        Ok(if negative { -value } else { value })
    }

    /// Read a name component up to the next `@`.
    fn read_simple_string(&mut self) -> Result<&'a [u8], ParseError> {
        let start = self.input.pos();
        let Some(length) = self
            .bytes
            .get(start..)
            .and_then(|rest| rest.iter().position(|&byte| byte == b'@'))
        else {
            return Err(self.error(ParseErrorKind::UnexpectedEnd));
        };
        if length == 0 {
            return Err(self.error(ParseErrorKind::InvalidLength));
        }
        let end = start + length;
        let text = self
            .bytes
            .get(start..end)
            .ok_or_else(|| self.error(ParseErrorKind::InvalidLength))?;
        self.bump(length + 1)?;
        self.tick_n(length + 1);
        Ok(text)
    }

    // -- names -------------------------------------------------------------

    fn parse_symbol_shape(&mut self) -> Result<NodeRef<'a>, ParseError> {
        self.enter()?;
        let result = self.parse_symbol_inner();
        self.leave();
        result
    }

    fn parse_symbol_inner(&mut self) -> Result<NodeRef<'a>, ParseError> {
        let start = self.input.pos();
        if self.starts_with(b".?") {
            return self.parse_typeinfo_name(start);
        }
        if self.starts_with(b"??@") {
            return self.parse_md5_name(start);
        }
        self.expect(b'?')?;
        if let Some(node) = self.parse_special_intrinsic(start)? {
            return Ok(node);
        }
        if self.consume(b"$TSS") {
            return self.parse_thread_safe_guard(start);
        }
        self.parse_declarator(start)
    }

    fn parse_declarator(&mut self, start: usize) -> Result<NodeRef<'a>, ParseError> {
        let name = self.parse_fully_qualified_symbol_name()?;
        self.parse_encoded_symbol(name, start)
    }

    fn parse_fully_qualified_symbol_name(&mut self) -> Result<ParsedName<'a>, ParseError> {
        let base = self.parse_unqualified_name(NameBackref::Simple)?;
        let scope = self.parse_scope_chain()?;
        Ok(ParsedName { base, scope })
    }

    fn parse_fully_qualified_type_name(&mut self) -> Result<NodeRef<'a>, ParseError> {
        let base = self.parse_unqualified_type_name(NameBackref::Template)?;
        let scope = self.parse_scope_chain()?;
        let start = self.node_start(&base);
        self.fold_name(base, &scope, start)
    }

    fn parse_unqualified_name(&mut self, backref: NameBackref) -> Result<NodeRef<'a>, ParseError> {
        if matches!(self.peek(), Some(b'0'..=b'9')) {
            return self.backref_name();
        }
        if self.starts_with(b"?$") {
            return self.parse_template_instantiation(backref);
        }
        if self.starts_with(b"?") {
            return self.parse_special_name();
        }
        self.parse_simple_name(backref)
    }

    fn parse_unqualified_type_name(
        &mut self,
        backref: NameBackref,
    ) -> Result<NodeRef<'a>, ParseError> {
        if matches!(self.peek(), Some(b'0'..=b'9')) {
            return self.backref_name();
        }
        if self.starts_with(b"?$") {
            return self.parse_template_instantiation(backref);
        }
        // Type names are always memorized for back-references.
        self.parse_simple_name(NameBackref::Simple)
    }

    fn parse_simple_name(&mut self, backref: NameBackref) -> Result<NodeRef<'a>, ParseError> {
        let start = self.input.pos();
        let text = self.read_simple_string()?;
        let leaf = if let Some(inner) = unnamed_type_text(text) {
            self.make(Kind::UnnamedType, Some(decode(inner)), start, Vec::new())?
        } else {
            self.make(Kind::Identifier, Some(decode(text)), start, Vec::new())?
        };
        let name = self.make(Kind::UnqualifiedName, None, start, alloc::vec![leaf])?;
        if backref == NameBackref::Simple {
            self.memorize_name(Rc::clone(&name));
        }
        Ok(name)
    }

    fn parse_scope_chain(&mut self) -> Result<Vec<NodeRef<'a>>, ParseError> {
        let mut scope = Vec::new();
        loop {
            if self.eat(b'@') {
                break;
            }
            if self.peek().is_none() {
                return Err(self.error(ParseErrorKind::UnexpectedEnd));
            }
            scope.push(self.parse_scope_piece()?);
        }
        Ok(scope)
    }

    fn parse_scope_piece(&mut self) -> Result<NodeRef<'a>, ParseError> {
        if matches!(self.peek(), Some(b'0'..=b'9')) {
            return self.backref_name();
        }
        if self.starts_with(b"?$") {
            return self.parse_template_instantiation(NameBackref::Template);
        }
        if self.starts_with(b"?A") {
            return self.parse_anonymous_namespace();
        }
        if self.is_local_scope_pattern() {
            return self.parse_local_scope_piece();
        }
        self.parse_simple_name(NameBackref::Simple)
    }

    fn parse_anonymous_namespace(&mut self) -> Result<NodeRef<'a>, ParseError> {
        let start = self.input.pos();
        self.expect_text(b"?A")?;
        let key_start = self.input.pos();
        let Some(length) = self
            .bytes
            .get(key_start..)
            .and_then(|rest| rest.iter().position(|&byte| byte == b'@'))
        else {
            return Err(self.error(ParseErrorKind::UnexpectedEnd));
        };
        let key_end = key_start + length;
        let key = self
            .bytes
            .get(key_start..key_end)
            .ok_or_else(|| self.error(ParseErrorKind::InvalidLength))?;
        self.bump(length + 1)?;
        self.tick_n(length + 1);
        let leaf = self.make(
            Kind::Identifier,
            Some(Cow::Borrowed("`anonymous namespace'")),
            start,
            Vec::new(),
        )?;
        let name = self.make(Kind::UnqualifiedName, None, start, alloc::vec![leaf])?;
        if !key.is_empty() {
            // The namespace key is the value later back-references address.
            self.memorize_name(Rc::clone(&name));
        }
        Ok(name)
    }

    /// True for the `?<number>?<symbol>` local-scope form.
    fn is_local_scope_pattern(&self) -> bool {
        let Some(rest) = self.bytes.get(self.input.pos()..) else {
            return false;
        };
        if rest.first() != Some(&b'?') {
            return false;
        }
        let rest = &rest[1..];
        let Some(end) = rest.iter().position(|&byte| byte == b'?') else {
            return false;
        };
        let candidate = &rest[..end];
        if candidate.is_empty() {
            return false;
        }
        if candidate.len() == 1 {
            return candidate[0] == b'@' || candidate[0].is_ascii_digit();
        }
        if candidate.last() != Some(&b'@') {
            return false;
        }
        let number = &candidate[..candidate.len() - 1];
        (b'B'..=b'P').contains(&number[0])
            && number[1..].iter().all(|byte| (b'A'..=b'P').contains(byte))
    }

    fn parse_local_scope_piece(&mut self) -> Result<NodeRef<'a>, ParseError> {
        let start = self.input.pos();
        self.expect(b'?')?;
        let (discriminator, negative) = self.read_number()?;
        if negative {
            return Err(self.error(ParseErrorKind::InvalidLength));
        }
        self.expect(b'?')?;
        let shape = self.parse_symbol_shape()?;
        let encoding = self.make(Kind::Encoding, None, start, alloc::vec![shape])?;
        self.make(
            Kind::LocalName,
            Some(Cow::Owned(alloc::format!("{discriminator}"))),
            start,
            alloc::vec![encoding],
        )
    }

    fn parse_template_instantiation(
        &mut self,
        backref: NameBackref,
    ) -> Result<NodeRef<'a>, ParseError> {
        let start = self.input.pos();
        self.expect_text(b"?$")?;
        let saved_names = core::mem::take(&mut self.names);
        let saved_types = core::mem::take(&mut self.types);
        let base = self.parse_unqualified_name(NameBackref::Simple)?;
        if backref == NameBackref::Template && is_structor_or_conversion(&base) {
            return Err(self.error(ParseErrorKind::Unsupported));
        }
        let args = self.parse_template_args()?;
        self.names = saved_names;
        self.types = saved_types;
        let name = self.make(Kind::Name, None, start, alloc::vec![base, args])?;
        if backref == NameBackref::Template {
            self.memorize_name(Rc::clone(&name));
        }
        Ok(name)
    }

    fn parse_special_name(&mut self) -> Result<NodeRef<'a>, ParseError> {
        let start = self.input.pos();
        self.expect(b'?')?;
        let leaf = if self.eat(b'_') {
            if self.eat(b'_') {
                self.parse_double_under_operator(start)?
            } else {
                self.parse_under_operator(start)?
            }
        } else {
            self.parse_basic_operator(start)?
        };
        self.make(Kind::UnqualifiedName, None, start, alloc::vec![leaf])
    }

    fn parse_basic_operator(&mut self, start: usize) -> Result<NodeRef<'a>, ParseError> {
        let Some(code) = self.peek() else {
            return Err(self.error(ParseErrorKind::UnexpectedEnd));
        };
        self.bump(1)?;
        self.tick()?;
        if code == b'0' || code == b'1' {
            let text = if code == b'0' { "C1" } else { "D1" };
            let leaf = self.make(
                Kind::CtorDtorName,
                Some(Cow::Borrowed(text)),
                start,
                Vec::new(),
            )?;
            return Ok(leaf);
        }
        if code == b'B' {
            let leaf = self.make(
                Kind::OperatorName,
                Some(Cow::Borrowed("operator")),
                start,
                Vec::new(),
            )?;
            return Ok(leaf);
        }
        let spelling =
            basic_operator_spelling(code).ok_or_else(|| self.error(ParseErrorKind::Unsupported))?;
        let leaf = self.make(
            Kind::OperatorName,
            Some(Cow::Borrowed(spelling)),
            start,
            Vec::new(),
        )?;
        Ok(leaf)
    }

    fn parse_under_operator(&mut self, start: usize) -> Result<NodeRef<'a>, ParseError> {
        let Some(code) = self.peek() else {
            return Err(self.error(ParseErrorKind::UnexpectedEnd));
        };
        self.bump(1)?;
        self.tick()?;
        let spelling =
            under_operator_spelling(code).ok_or_else(|| self.error(ParseErrorKind::Unsupported))?;
        let leaf = self.make(
            Kind::OperatorName,
            Some(Cow::Borrowed(spelling)),
            start,
            Vec::new(),
        )?;
        Ok(leaf)
    }

    fn parse_double_under_operator(&mut self, start: usize) -> Result<NodeRef<'a>, ParseError> {
        let Some(code) = self.peek() else {
            return Err(self.error(ParseErrorKind::UnexpectedEnd));
        };
        self.bump(1)?;
        self.tick()?;
        if code == b'K' {
            let suffix_start = self.input.pos();
            let suffix = self.read_simple_string()?;
            let text = alloc::format!("operator \"\"{}", decode(suffix));
            let leaf = self.make(
                Kind::OperatorName,
                Some(Cow::Owned(text)),
                suffix_start,
                Vec::new(),
            )?;
            return Ok(leaf);
        }
        let spelling = double_under_operator_spelling(code)
            .ok_or_else(|| self.error(ParseErrorKind::Unsupported))?;
        let leaf = self.make(
            Kind::OperatorName,
            Some(Cow::Borrowed(spelling)),
            start,
            Vec::new(),
        )?;
        Ok(leaf)
    }

    fn fold_name(
        &mut self,
        base: NodeRef<'a>,
        scope: &[NodeRef<'a>],
        start: usize,
    ) -> Result<NodeRef<'a>, ParseError> {
        let mut prefix: Option<NodeRef<'a>> = None;
        for entry in scope.iter().rev() {
            prefix = Some(match prefix {
                None => Rc::clone(entry),
                Some(current) => self.make(
                    Kind::NestedName,
                    None,
                    start,
                    alloc::vec![current, Rc::clone(entry)],
                )?,
            });
        }
        match prefix {
            None => Ok(base),
            Some(prefix) => self.make(Kind::NestedName, None, start, alloc::vec![prefix, base]),
        }
    }

    fn fold_scope_entries(
        &mut self,
        scope: &[NodeRef<'a>],
        start: usize,
    ) -> Result<Option<NodeRef<'a>>, ParseError> {
        let mut prefix: Option<NodeRef<'a>> = None;
        for entry in scope.iter().rev() {
            prefix = Some(match prefix {
                None => Rc::clone(entry),
                Some(current) => self.make(
                    Kind::NestedName,
                    None,
                    start,
                    alloc::vec![current, Rc::clone(entry)],
                )?,
            });
        }
        Ok(prefix)
    }

    // -- template arguments -----------------------------------------------

    fn parse_template_args(&mut self) -> Result<NodeRef<'a>, ParseError> {
        let start = self.input.pos();
        let mut args = Vec::new();
        loop {
            if self.eat(b'@') {
                break;
            }
            if self.peek().is_none() {
                return Err(self.error(ParseErrorKind::UnexpectedEnd));
            }
            if self.consume(b"$$$V")
                || self.consume(b"$$V")
                || self.consume(b"$$Z")
                || self.consume(b"$S")
            {
                // Parameter pack separator: it does not denote an argument.
                continue;
            }
            let auto_nttp = self.consume(b"$M");
            if auto_nttp {
                let _ignored = self.parse_type(TypeMode::Drop)?;
            }
            let arg_start = self.input.pos();
            let arg = if self.consume(b"$$Y") {
                self.parse_fully_qualified_type_name()?
            } else if self.consume(b"$$B") {
                self.parse_type(TypeMode::Drop)?
            } else if self.consume(b"$$C") {
                self.parse_type(TypeMode::Mangle)?
            } else if self.starts_member_pointer_arg(auto_nttp) {
                self.parse_member_pointer_arg(auto_nttp)?
            } else if self.consume(b"$E") {
                let symbol = self.parse_symbol_shape()?;
                self.memorize_symbol_identifier(&symbol);
                self.make(
                    Kind::TemplateParam,
                    Some(Cow::Borrowed("&symbol")),
                    start,
                    alloc::vec![symbol],
                )?
            } else if self.starts_data_member_pointer_arg(auto_nttp) {
                self.parse_data_member_pointer_arg(auto_nttp)?
            } else if self.starts_integral_arg(auto_nttp) {
                self.parse_integral_arg(auto_nttp)?
            } else {
                self.parse_type(TypeMode::Drop)?
            };
            // Template arguments are type back-reference candidates in the
            // enclosing template context. A later digit can therefore refer
            // to an earlier argument, even one nested in a function type.
            if is_template_type_arg(&arg) && self.input.pos() - arg_start > 1 {
                self.memorize_type(Rc::clone(&arg));
            }
            args.push(arg);
        }
        self.make(Kind::TemplateArgs, None, start, args)
    }

    fn starts_member_pointer_arg(&self, auto_nttp: bool) -> bool {
        if auto_nttp {
            matches!(self.peek(), Some(b'1' | b'H' | b'I' | b'J'))
        } else {
            self.starts_with(b"$1")
                || self.starts_with(b"$H")
                || self.starts_with(b"$I")
                || self.starts_with(b"$J")
        }
    }

    fn parse_member_pointer_arg(&mut self, auto_nttp: bool) -> Result<NodeRef<'a>, ParseError> {
        let start = self.input.pos();
        if !auto_nttp {
            self.expect(b'$')?;
        }
        let specifier = self
            .peek()
            .ok_or_else(|| self.error(ParseErrorKind::UnexpectedEnd))?;
        self.bump(1)?;
        self.tick()?;
        let mut children = Vec::new();
        if self.starts_with(b"?") {
            let symbol = self.parse_symbol_shape()?;
            self.memorize_symbol_identifier(&symbol);
            children.push(symbol);
        }
        let offsets = match specifier {
            b'1' => 0,
            b'H' => 1,
            b'I' => 2,
            b'J' => 3,
            _ => return Err(self.error(ParseErrorKind::Unsupported)),
        };
        for _ in 0..offsets {
            let _ = self.read_signed()?;
        }
        self.make(
            Kind::TemplateParam,
            Some(Cow::Owned(alloc::format!("${}", char::from(specifier)))),
            start,
            children,
        )
    }

    fn starts_data_member_pointer_arg(&self, auto_nttp: bool) -> bool {
        if auto_nttp {
            matches!(self.peek(), Some(b'F' | b'G'))
        } else {
            self.starts_with(b"$F") || self.starts_with(b"$G")
        }
    }

    fn parse_data_member_pointer_arg(
        &mut self,
        auto_nttp: bool,
    ) -> Result<NodeRef<'a>, ParseError> {
        let start = self.input.pos();
        if !auto_nttp {
            self.expect(b'$')?;
        }
        let specifier = self
            .peek()
            .ok_or_else(|| self.error(ParseErrorKind::UnexpectedEnd))?;
        self.bump(1)?;
        self.tick()?;
        let offsets = match specifier {
            b'F' => 2,
            b'G' => 3,
            _ => return Err(self.error(ParseErrorKind::Unsupported)),
        };
        for _ in 0..offsets {
            let _ = self.read_signed()?;
        }
        self.make(
            Kind::TemplateParam,
            Some(Cow::Owned(alloc::format!("${}", char::from(specifier)))),
            start,
            Vec::new(),
        )
    }

    fn starts_integral_arg(&self, auto_nttp: bool) -> bool {
        if auto_nttp {
            self.peek() == Some(b'0')
        } else {
            self.starts_with(b"$0")
        }
    }

    fn parse_integral_arg(&mut self, auto_nttp: bool) -> Result<NodeRef<'a>, ParseError> {
        let start = self.input.pos();
        if !auto_nttp {
            self.expect_text(b"$0")?;
        } else {
            self.expect(b'0')?;
        }
        let (value, negative) = self.read_number()?;
        let text = if negative {
            alloc::format!("-{value}")
        } else {
            alloc::format!("{value}")
        };
        self.make(Kind::Literal, Some(Cow::Owned(text)), start, Vec::new())
    }

    // -- special intrinsics -----------------------------------------------

    fn parse_special_intrinsic(&mut self, start: usize) -> Result<Option<NodeRef<'a>>, ParseError> {
        if self.consume(b"?_7") {
            return self.parse_table(start, TableKind::Vftable, false).map(Some);
        }
        if self.consume(b"?_8") {
            return self.parse_table(start, TableKind::Vbtable, false).map(Some);
        }
        if self.consume(b"?_S") {
            return self.parse_table(start, TableKind::Vftable, true).map(Some);
        }
        if self.consume(b"?_R4") {
            return self
                .parse_table(start, TableKind::RttiCompleteObjectLocator, false)
                .map(Some);
        }
        if self.consume(b"?_9") {
            return self.parse_vcall_thunk(start).map(Some);
        }
        if self.consume(b"?_C") {
            return self.parse_string_literal(start).map(Some);
        }
        if self.consume(b"?_B") {
            return self.parse_local_static_guard(start, false).map(Some);
        }
        if self.consume(b"?__J") {
            return self.parse_local_static_guard(start, true).map(Some);
        }
        if self.consume(b"?_R0") {
            return self.parse_rtti_type_descriptor(start).map(Some);
        }
        if self.consume(b"?_R1") {
            return self.parse_rtti_base_class_descriptor(start).map(Some);
        }
        if self.consume(b"?_R2") {
            return self
                .parse_rtti_untyped(start, "base-class-array", "`RTTI Base Class Array'")
                .map(Some);
        }
        if self.consume(b"?_R3") {
            return self
                .parse_rtti_untyped(
                    start,
                    "class-hierarchy-descriptor",
                    "`RTTI Class Hierarchy Descriptor'",
                )
                .map(Some);
        }
        if self.consume(b"?__E") {
            return self.parse_dynamic_structor(start, false).map(Some);
        }
        if self.consume(b"?__F") {
            return self.parse_dynamic_structor(start, true).map(Some);
        }
        if self.starts_with(b"?_A") || self.starts_with(b"?_P") {
            return Err(self.error(ParseErrorKind::Unsupported));
        }
        Ok(None)
    }

    fn parse_thread_safe_guard(&mut self, start: usize) -> Result<NodeRef<'a>, ParseError> {
        let mut index = 0_u64;
        let mut any = false;
        while let Some(byte @ b'0'..=b'9') = self.peek() {
            index = index
                .checked_mul(10)
                .and_then(|current| current.checked_add(u64::from(byte - b'0')))
                .ok_or_else(|| self.error(ParseErrorKind::InvalidLength))?;
            self.bump(1)?;
            self.tick()?;
            any = true;
        }
        if !any {
            return Err(self.error(ParseErrorKind::InvalidLength));
        }
        self.expect(b'@')?;
        let mut scope = alloc::vec![self.parse_scope_piece()?];
        scope.extend(self.parse_scope_chain()?);
        let name = self
            .fold_scope_entries(&scope, start)?
            .ok_or_else(|| self.error(ParseErrorKind::UnexpectedEnd))?;
        self.expect_text(b"4HA")?;
        self.make(
            Kind::GuardVariable,
            Some(Cow::Owned(alloc::format!("{index}"))),
            start,
            alloc::vec![name],
        )
    }

    fn parse_table(
        &mut self,
        start: usize,
        table: TableKind,
        local: bool,
    ) -> Result<NodeRef<'a>, ParseError> {
        let (name_text, kind) = match table {
            TableKind::Vftable => ("`vftable'", Kind::Vftable),
            TableKind::Vbtable => ("`vbtable'", Kind::Vbtable),
            TableKind::RttiCompleteObjectLocator => ("`RTTI Complete Object Locator'", Kind::Rtti),
        };
        let base = self.make_identifier(name_text, start)?;
        let scope = self.parse_scope_chain()?;
        let name = self.fold_name(base, &scope, start)?;
        let marker = self
            .peek()
            .ok_or_else(|| self.error(ParseErrorKind::UnexpectedEnd))?;
        if marker != b'6' && marker != b'7' {
            return Err(self.error(ParseErrorKind::UnexpectedByte));
        }
        self.bump(1)?;
        self.tick()?;
        let (quals, _member) = self.read_qualifiers()?;
        let mut targets = Vec::new();
        loop {
            if self.eat(b'@') {
                break;
            }
            if self.peek().is_none() {
                break;
            }
            targets.push(self.parse_fully_qualified_type_name()?);
        }
        let mut text_parts = Vec::new();
        if local {
            text_parts.push("local");
        }
        if quals.const_ {
            text_parts.push("const");
        }
        let text = if text_parts.is_empty() {
            None
        } else {
            Some(Cow::Owned(text_parts.join(" ")))
        };
        let mut children = Vec::with_capacity(targets.len() + 1);
        children.push(name);
        children.extend(targets);
        if matches!(table, TableKind::RttiCompleteObjectLocator) {
            let text = text.map_or(Cow::Borrowed("complete-object-locator"), |text| {
                Cow::Owned(alloc::format!("complete-object-locator ({text})"))
            });
            return self.make(Kind::Rtti, Some(text), start, children);
        }
        self.make(kind, text, start, children)
    }

    fn parse_vcall_thunk(&mut self, start: usize) -> Result<NodeRef<'a>, ParseError> {
        let base = self.make_identifier("`vcall'", start)?;
        let scope = self.parse_scope_chain()?;
        let name = self.fold_name(base, &scope, start)?;
        self.expect_text(b"$B")?;
        let offset = self.read_unsigned()?;
        self.expect(b'A')?;
        let calling_conv = self.read_calling_conv()?;
        let text = Cow::Owned(alloc::format!("vcall{{{offset}, {calling_conv}}}"));
        self.make(Kind::VCallThunk, Some(text), start, alloc::vec![name])
    }

    fn parse_local_static_guard(
        &mut self,
        start: usize,
        thread: bool,
    ) -> Result<NodeRef<'a>, ParseError> {
        let name_text = if thread {
            "`local static thread guard'"
        } else {
            "`local static guard'"
        };
        let base = self.make_identifier(name_text, start)?;
        let scope = self.parse_scope_chain()?;
        let qualified = self.fold_name(base, &scope, start)?;
        if !self.consume(b"4IA") && !self.consume(b"5") {
            return Err(self.error(ParseErrorKind::UnexpectedByte));
        }
        let index = if self.peek().is_some() {
            self.read_unsigned()?
        } else {
            0
        };
        let text = Cow::Owned(alloc::format!("{index}"));
        self.make(
            Kind::GuardVariable,
            Some(text),
            start,
            alloc::vec![qualified],
        )
    }

    fn parse_string_literal(&mut self, start: usize) -> Result<NodeRef<'a>, ParseError> {
        self.expect_text(b"@_")?;
        let wide = match self.peek() {
            Some(b'0') => {
                self.bump(1)?;
                self.tick()?;
                false
            }
            Some(b'1') => {
                self.bump(1)?;
                self.tick()?;
                true
            }
            Some(_) => return Err(self.error(ParseErrorKind::UnexpectedByte)),
            None => return Err(self.error(ParseErrorKind::UnexpectedEnd)),
        };
        let byte_length = self.read_unsigned()?;
        if byte_length < if wide { 2 } else { 1 } {
            return Err(self.error(ParseErrorKind::InvalidLength));
        }
        if wide && byte_length % 2 != 0 {
            return Err(self.error(ParseErrorKind::InvalidLength));
        }
        // The CRC is always eight hexadecimal characters plus a terminator.
        let crc_start = self.input.pos();
        let Some(crc_len) = self
            .bytes
            .get(crc_start..)
            .and_then(|rest| rest.iter().position(|&byte| byte == b'@'))
        else {
            return Err(self.error(ParseErrorKind::UnexpectedEnd));
        };
        if crc_len == 0 {
            return Err(self.error(ParseErrorKind::InvalidLength));
        }
        self.bump(crc_len + 1)?;
        self.tick_n(crc_len + 1);
        if self.peek().is_none() {
            return Err(self.error(ParseErrorKind::UnexpectedEnd));
        }
        let mut decoded: Vec<u8> = Vec::new();
        let mut units_read = 0_u64;
        loop {
            if self.eat(b'@') {
                break;
            }
            if wide {
                if units_read >= 64 || units_read >= byte_length {
                    return Err(self.error(ParseErrorKind::InvalidLength));
                }
                let high = self.read_char_literal()?;
                let low = self.read_char_literal()?;
                units_read += 2;
                let code = (u16::from(high) << 8) | u16::from(low);
                let ch = char::from_u32(u32::from(code)).unwrap_or(char::REPLACEMENT_CHARACTER);
                let mut buffer = [0_u8; 4];
                decoded.extend_from_slice(ch.encode_utf8(&mut buffer).as_bytes());
            } else {
                if decoded.len() >= 128 {
                    return Err(self.error(ParseErrorKind::TooMuchWork));
                }
                let byte = self.read_char_literal()?;
                decoded.push(byte);
            }
        }
        if wide && byte_length <= 64 && units_read != byte_length {
            return Err(self.error(ParseErrorKind::InvalidLength));
        }
        while decoded.last() == Some(&0) {
            let _ = decoded.pop();
        }
        let text = owned_text(&decoded);
        self.make(
            Kind::StringLiteral,
            Some(Cow::Owned(text)),
            start,
            Vec::new(),
        )
    }

    fn read_char_literal(&mut self) -> Result<u8, ParseError> {
        let byte = self
            .peek()
            .ok_or_else(|| self.error(ParseErrorKind::UnexpectedEnd))?;
        if byte != b'?' {
            self.bump(1)?;
            self.tick()?;
            return Ok(byte);
        }
        self.bump(1)?;
        self.tick()?;
        match self.peek() {
            Some(b'$') => {
                self.bump(1)?;
                self.tick()?;
                let high = self
                    .peek()
                    .ok_or_else(|| self.error(ParseErrorKind::UnexpectedEnd))?;
                let low = self
                    .peek_at(1)
                    .ok_or_else(|| self.error(ParseErrorKind::UnexpectedEnd))?;
                let high =
                    rebased_hex(high).ok_or_else(|| self.error(ParseErrorKind::UnexpectedByte))?;
                let low =
                    rebased_hex(low).ok_or_else(|| self.error(ParseErrorKind::UnexpectedByte))?;
                self.bump(2)?;
                self.tick_n(2);
                Ok((high << 4) | low)
            }
            Some(byte @ b'0'..=b'9') => {
                self.bump(1)?;
                self.tick()?;
                const LOOKUP: &[u8] = b",/\\:. \n\t'-";
                Ok(LOOKUP[usize::from(byte - b'0')])
            }
            Some(byte @ b'a'..=b'z') => {
                self.bump(1)?;
                self.tick()?;
                Ok(0xE1 + (byte - b'a'))
            }
            Some(byte @ b'A'..=b'Z') => {
                self.bump(1)?;
                self.tick()?;
                Ok(0xC1 + (byte - b'A'))
            }
            _ => Err(self.error(ParseErrorKind::UnexpectedByte)),
        }
    }

    fn parse_rtti_type_descriptor(&mut self, start: usize) -> Result<NodeRef<'a>, ParseError> {
        let ty = self.parse_type(TypeMode::Result)?;
        self.expect_text(b"@8")?;
        if self.input.pos() != self.bytes.len() {
            return Err(self.error(ParseErrorKind::UnexpectedByte));
        }
        self.make(
            Kind::Rtti,
            Some(Cow::Borrowed("type-descriptor")),
            start,
            alloc::vec![ty],
        )
    }

    fn parse_rtti_base_class_descriptor(
        &mut self,
        start: usize,
    ) -> Result<NodeRef<'a>, ParseError> {
        let nv_offset = self.read_unsigned()?;
        let vbptr_offset = self.read_signed()?;
        let vbtable_offset = self.read_unsigned()?;
        let flags = self.read_unsigned()?;
        let base = self.make_identifier("`RTTI Base Class Descriptor'", start)?;
        let scope = self.parse_scope_chain()?;
        let name = self.fold_name(base, &scope, start)?;
        let _ = self.eat(b'8');
        let text = Cow::Owned(alloc::format!(
            "base-class-descriptor ({nv_offset}, {vbptr_offset}, {vbtable_offset}, {flags})"
        ));
        self.make(Kind::Rtti, Some(text), start, alloc::vec![name])
    }

    fn parse_rtti_untyped(
        &mut self,
        start: usize,
        text: &'static str,
        base_text: &'static str,
    ) -> Result<NodeRef<'a>, ParseError> {
        let base = self.make_identifier(base_text, start)?;
        let scope = self.parse_scope_chain()?;
        let name = self.fold_name(base, &scope, start)?;
        self.expect(b'8')?;
        self.make(
            Kind::Rtti,
            Some(Cow::Borrowed(text)),
            start,
            alloc::vec![name],
        )
    }

    fn parse_dynamic_structor(
        &mut self,
        start: usize,
        destructor: bool,
    ) -> Result<NodeRef<'a>, ParseError> {
        let known_static_member = self.eat(b'?');
        let entity = self.parse_declarator(start)?;
        let kind = if destructor {
            Kind::DynamicAtexitDestructor
        } else {
            Kind::DynamicInitializer
        };
        if entity.kind() == Kind::Variable {
            let at_count = if known_static_member { 2 } else { 1 };
            for _ in 0..at_count {
                self.expect(b'@')?;
            }
            let name = self.make_identifier("`dynamic initializer'", start)?;
            let signature = self.parse_function_encoding(
                ParsedName {
                    base: name,
                    scope: Vec::new(),
                },
                start,
            )?;
            let _ = signature;
            return self.make(kind, None, start, alloc::vec![entity]);
        }
        if known_static_member {
            return Err(self.error(ParseErrorKind::UnexpectedByte));
        }
        self.make(kind, None, start, alloc::vec![entity])
    }

    fn parse_md5_name(&mut self, start: usize) -> Result<NodeRef<'a>, ParseError> {
        self.expect_text(b"??@")?;
        let digest_start = self.input.pos();
        let mut length = 0_usize;
        while let Some(byte) = self.peek() {
            if byte == b'@' {
                break;
            }
            if !byte.is_ascii_hexdigit() {
                return Err(self.error(ParseErrorKind::UnexpectedByte));
            }
            self.bump(1)?;
            self.tick()?;
            length += 1;
        }
        if length != 32 {
            return Err(self.error(ParseErrorKind::InvalidLength));
        }
        let digest = self
            .bytes
            .get(digest_start..digest_start + length)
            .ok_or_else(|| self.error(ParseErrorKind::InvalidLength))?;
        self.expect(b'@')?;
        let _ = self.consume(b"??_R4@");
        self.make(Kind::Md5Name, Some(decode(digest)), start, Vec::new())
    }

    fn parse_typeinfo_name(&mut self, start: usize) -> Result<NodeRef<'a>, ParseError> {
        self.expect(b'.')?;
        let ty = self.parse_type(TypeMode::Result)?;
        if self.input.pos() != self.bytes.len() {
            return Err(self.error(ParseErrorKind::UnexpectedByte));
        }
        self.make(
            Kind::Rtti,
            Some(Cow::Borrowed("type-descriptor-name")),
            start,
            alloc::vec![ty],
        )
    }

    // -- encodings ---------------------------------------------------------

    fn parse_encoded_symbol(
        &mut self,
        name: ParsedName<'a>,
        start: usize,
    ) -> Result<NodeRef<'a>, ParseError> {
        if matches!(self.peek(), Some(b'0'..=b'4')) {
            return self.parse_variable(name, start);
        }
        self.parse_function_encoding(name, start)
    }

    fn parse_variable(
        &mut self,
        name: ParsedName<'a>,
        start: usize,
    ) -> Result<NodeRef<'a>, ParseError> {
        let storage = self.read_variable_storage_class()?;
        let ty = self.parse_type(TypeMode::Drop)?;
        let ty = self.apply_variable_qualifiers(ty)?;
        let name = self.fold_name(name.base, &name.scope, start)?;
        self.make(
            Kind::Variable,
            Some(Cow::Borrowed(storage)),
            start,
            alloc::vec![name, ty],
        )
    }

    fn apply_variable_qualifiers(&mut self, ty: NodeRef<'a>) -> Result<NodeRef<'a>, ParseError> {
        let pointer_kind = pointer_like_kind(&ty);
        let ext = if pointer_kind.is_some() {
            self.read_pointer_ext_qualifiers()
        } else {
            Quals::default()
        };
        let (quals, _member) = self.read_qualifiers()?;
        if pointer_kind == Some(Kind::MemberPointerType) {
            let _ = self.parse_fully_qualified_type_name()?;
        }
        let quals = ext.union(quals);
        if quals.is_empty() {
            return Ok(ty);
        }
        // The trailing qualifiers repeat the qualifiers of the pointed-to
        // type. Applying them to the pointer itself would turn
        // `char const *` into `char const *const`, so wrap the pointee.
        self.apply_pointee_quals(ty, quals)
    }

    /// Apply a variable's trailing qualifiers to the type they repeat.
    ///
    /// The qualifiers belong to the pointed-to type, the array element or the
    /// member, not to the pointer, array or member pointer itself.
    fn apply_pointee_quals(
        &mut self,
        node: NodeRef<'a>,
        quals: Quals,
    ) -> Result<NodeRef<'a>, ParseError> {
        match node.kind() {
            Kind::QualifiedType => {
                let inner = self.apply_pointee_quals(child_of(&node, 0)?, quals)?;
                self.replace_child(node, 0, inner)
            }
            Kind::PointerType | Kind::ReferenceType | Kind::RvalueReferenceType => {
                let pointee = self.apply_quals(child_of(&node, 0)?, quals)?;
                self.replace_child(node, 0, pointee)
            }
            Kind::ArrayType => {
                let element = self.apply_quals(child_of(&node, 0)?, quals)?;
                self.replace_child(node, 0, element)
            }
            Kind::MemberPointerType => {
                let member = child_of(&node, 1)?;
                if member.kind() == Kind::FunctionType {
                    let member = self.qualify_function_type(member, quals)?;
                    return self.replace_child(node, 1, member);
                }
                let member = self.apply_quals(member, quals)?;
                self.replace_child(node, 1, member)
            }
            _ => self.apply_quals(node, quals),
        }
    }

    /// Attach trailing cv qualifiers to a member function type, which keeps
    /// them after the parameter list rather than in the return type.
    fn qualify_function_type(
        &mut self,
        function: NodeRef<'a>,
        quals: Quals,
    ) -> Result<NodeRef<'a>, ParseError> {
        let mut text = function.text().map_or_else(String::new, String::from);
        if let Some(quals) = quals.text() {
            if !text.is_empty() {
                text.push(' ');
            }
            text.push_str(&quals);
        }
        let start = self.node_start(&function);
        self.make(
            Kind::FunctionType,
            Some(Cow::Owned(text)),
            start,
            function.children().to_vec(),
        )
    }

    fn replace_child(
        &mut self,
        node: NodeRef<'a>,
        index: usize,
        child: NodeRef<'a>,
    ) -> Result<NodeRef<'a>, ParseError> {
        let start = self.node_start(&node);
        let text = node.text().map(|text| Cow::Owned(String::from(text)));
        let mut children = node.children().to_vec();
        let slot = children
            .get_mut(index)
            .ok_or_else(|| self.error(ParseErrorKind::UnexpectedEnd))?;
        *slot = child;
        self.make(node.kind(), text, start, children)
    }

    fn parse_function_encoding(
        &mut self,
        mut name: ParsedName<'a>,
        start: usize,
    ) -> Result<NodeRef<'a>, ParseError> {
        let extern_c = self.consume(b"$$J0");
        let func_class = self.read_func_class()?;
        let thunk = if func_class.static_this_adjust {
            let offset = self.read_signed()?;
            Some(alloc::format!("adjustor{{{offset}}}"))
        } else if func_class.virtual_this_adjust {
            let (vbptr, vboffset) = if func_class.virtual_this_adjust_ex {
                (self.read_signed()?, self.read_signed()?)
            } else {
                (0, 0)
            };
            let vtordisp = i64::from(self.read_signed()? as i32);
            let static_offset = i64::from(self.read_signed()? as i32);
            if func_class.virtual_this_adjust_ex {
                Some(alloc::format!(
                    "vtordispex{{{vbptr}, {vboffset}, {vtordisp}, {static_offset}}}"
                ))
            } else {
                Some(alloc::format!("vtordisp{{{vtordisp}, {static_offset}}}"))
            }
        } else {
            None
        };
        let signature = if func_class.no_param_list {
            Signature {
                ret: None,
                params: self.empty_parameter_list()?,
                calling_conv: "",
                quals: Quals::default(),
                ref_qual: None,
            }
        } else {
            let has_this = !(func_class.global || func_class.static_);
            self.parse_function_type(has_this)?
        };
        if let Some(ret) = &signature.ret
            && let Some(target) = conversion_target(&name.base)
        {
            name.base = self.make(
                Kind::OperatorName,
                Some(Cow::Borrowed("operator")),
                target,
                alloc::vec![Rc::clone(ret)],
            )?;
        }
        let name = self.fold_name(name.base, &name.scope, start)?;
        let text = modifier_text(&func_class, &signature, extern_c);
        let mut children = Vec::with_capacity(3);
        children.push(name);
        if let Some(ret) = signature.ret {
            children.push(ret);
        }
        children.push(signature.params);
        let kind = if func_class.global || func_class.no_param_list {
            Kind::Function
        } else {
            Kind::MemberFunction
        };
        let node = self.make(kind, text, start, children)?;
        match thunk {
            Some(text) => self.make(
                Kind::Thunk,
                Some(Cow::Owned(text)),
                start,
                alloc::vec![node],
            ),
            None => Ok(node),
        }
    }

    fn parse_function_type(&mut self, has_this: bool) -> Result<Signature<'a>, ParseError> {
        let mut quals = Quals::default();
        let mut ref_qual = None;
        if has_this {
            quals = self.read_pointer_ext_qualifiers();
            ref_qual = self.read_ref_qualifier();
            let (member_quals, _member) = self.read_qualifiers()?;
            quals = quals.union(member_quals);
        }
        let calling_conv = self.read_calling_conv()?;
        let ret = if self.eat(b'@') {
            None
        } else {
            Some(self.parse_type(TypeMode::Result)?)
        };
        let params = self.parse_function_params()?;
        let _noexcept = self.read_throw_specification()?;
        Ok(Signature {
            ret,
            params,
            calling_conv,
            quals,
            ref_qual,
        })
    }

    fn parse_function_params(&mut self) -> Result<NodeRef<'a>, ParseError> {
        let start = self.input.pos();
        let mut params = Vec::new();
        if self.eat(b'X') {
            return self.make(Kind::ParameterList, None, start, params);
        }
        loop {
            match self.peek() {
                Some(b'@' | b'Z') | None => break,
                Some(b'0'..=b'9') => {
                    let ty = self.backref_parameter()?;
                    params.push(self.make_parameter(ty)?);
                }
                Some(_) => {
                    let before = self.input.pos();
                    let ty = self.parse_type(TypeMode::Drop)?;
                    let consumed = self.input.pos() - before;
                    params.push(self.make_parameter(Rc::clone(&ty))?);
                    if consumed > 1 {
                        self.memorize_type(ty);
                    }
                }
            }
        }
        let variadic = if self.eat(b'@') {
            false
        } else if self.eat(b'Z') {
            true
        } else {
            return Err(self.error(ParseErrorKind::UnexpectedEnd));
        };
        if variadic {
            let start = self.input.pos();
            let ellipsis = self.make(
                Kind::BuiltinType,
                Some(Cow::Borrowed("...")),
                start,
                Vec::new(),
            )?;
            params.push(self.make_parameter(ellipsis)?);
        }
        self.make(Kind::ParameterList, None, start, params)
    }

    fn empty_parameter_list(&mut self) -> Result<NodeRef<'a>, ParseError> {
        let start = self.input.pos();
        self.make(Kind::ParameterList, None, start, Vec::new())
    }

    fn make_parameter(&mut self, ty: NodeRef<'a>) -> Result<NodeRef<'a>, ParseError> {
        let start = self.node_start(&ty);
        self.make(Kind::Parameter, None, start, alloc::vec![ty])
    }

    fn read_throw_specification(&mut self) -> Result<bool, ParseError> {
        if self.consume(b"_E") {
            return Ok(true);
        }
        if self.eat(b'Z') {
            return Ok(false);
        }
        Err(self.error(ParseErrorKind::UnexpectedByte))
    }

    fn read_func_class(&mut self) -> Result<FuncClass, ParseError> {
        let Some(byte) = self.peek() else {
            return Err(self.error(ParseErrorKind::UnexpectedEnd));
        };
        self.bump(1)?;
        self.tick()?;
        let mut class = FuncClass::default();
        match byte {
            b'9' => {
                class.global = true;
                class.no_param_list = true;
                return Ok(class);
            }
            b'$' => {
                class.virtual_ = true;
                class.virtual_this_adjust = true;
                class.virtual_this_adjust_ex = self.eat(b'R');
                let Some(sub) = self.peek() else {
                    return Err(self.error(ParseErrorKind::UnexpectedEnd));
                };
                self.bump(1)?;
                self.tick()?;
                let (access, far) = match sub {
                    b'0' => ("private:", false),
                    b'1' => ("private:", true),
                    b'2' => ("protected:", false),
                    b'3' => ("protected:", true),
                    b'4' => ("public:", false),
                    b'5' => ("public:", true),
                    _ => return Err(self.error(ParseErrorKind::Unsupported)),
                };
                class.access = Some(access);
                let _ = far;
                return Ok(class);
            }
            _ => {}
        }
        let (access, static_, virtual_, far, global) = match byte {
            b'A' => ("private:", false, false, false, false),
            b'B' => ("private:", false, false, true, false),
            b'C' => ("private:", true, false, false, false),
            b'D' => ("private:", true, false, true, false),
            b'E' => ("private:", false, true, false, false),
            b'F' => ("private:", false, true, true, false),
            b'G' => ("private:", false, false, false, false),
            b'H' => ("private:", false, false, true, false),
            b'I' => ("protected:", false, false, false, false),
            b'J' => ("protected:", false, false, true, false),
            b'K' => ("protected:", true, false, false, false),
            b'L' => ("protected:", true, false, true, false),
            b'M' => ("protected:", false, true, false, false),
            b'N' => ("protected:", false, true, true, false),
            b'O' => ("protected:", false, true, false, false),
            b'P' => ("protected:", false, true, true, false),
            b'Q' => ("public:", false, false, false, false),
            b'R' => ("public:", false, false, true, false),
            b'S' => ("public:", true, false, false, false),
            b'T' => ("public:", true, false, true, false),
            b'U' => ("public:", false, true, false, false),
            b'V' => ("public:", false, true, true, false),
            b'W' => ("public:", false, true, false, false),
            b'X' => ("public:", false, true, true, false),
            b'Y' => ("", false, false, false, true),
            b'Z' => ("", false, false, true, true),
            _ => return Err(self.error(ParseErrorKind::Unsupported)),
        };
        class.access = if access.is_empty() {
            None
        } else {
            Some(access)
        };
        class.static_ = static_;
        class.virtual_ = virtual_;
        class.global = global;
        class.static_this_adjust = matches!(byte, b'G' | b'H' | b'O' | b'P' | b'W' | b'X');
        let _ = far;
        Ok(class)
    }

    fn read_calling_conv(&mut self) -> Result<&'static str, ParseError> {
        let Some(byte) = self.peek() else {
            return Err(self.error(ParseErrorKind::UnexpectedEnd));
        };
        self.bump(1)?;
        self.tick()?;
        match byte {
            b'A' | b'B' => Ok("__cdecl"),
            b'C' | b'D' => Ok("__pascal"),
            b'E' | b'F' => Ok("__thiscall"),
            b'G' | b'H' => Ok("__stdcall"),
            b'I' | b'J' => Ok("__fastcall"),
            b'M' | b'N' => Ok("__clrcall"),
            b'O' | b'P' => Ok("__eabi"),
            b'Q' => Ok("__vectorcall"),
            b'S' => Ok("__attribute__((__swiftcall__))"),
            b'W' => Ok("__attribute__((__swiftasynccall__))"),
            _ => Err(self.error(ParseErrorKind::Unsupported)),
        }
    }

    fn read_ref_qualifier(&mut self) -> Option<&'static str> {
        if self.eat(b'G') {
            Some("&")
        } else if self.eat(b'H') {
            Some("&&")
        } else {
            None
        }
    }

    fn read_qualifiers(&mut self) -> Result<(Quals, bool), ParseError> {
        let Some(byte) = self.peek() else {
            return Err(self.error(ParseErrorKind::UnexpectedEnd));
        };
        self.bump(1)?;
        self.tick()?;
        let quals = match byte {
            b'Q' => Quals::default(),
            b'R' => Quals {
                const_: true,
                ..Quals::default()
            },
            b'S' => Quals {
                volatile: true,
                ..Quals::default()
            },
            b'T' => Quals {
                const_: true,
                volatile: true,
                ..Quals::default()
            },
            b'A' => Quals::default(),
            b'B' => Quals {
                const_: true,
                ..Quals::default()
            },
            b'C' => Quals {
                volatile: true,
                ..Quals::default()
            },
            b'D' => Quals {
                const_: true,
                volatile: true,
                ..Quals::default()
            },
            _ => return Err(self.error(ParseErrorKind::UnexpectedByte)),
        };
        Ok((quals, matches!(byte, b'Q' | b'R' | b'S' | b'T')))
    }

    fn read_pointer_ext_qualifiers(&mut self) -> Quals {
        let mut quals = Quals::default();
        if self.eat(b'E') {
            // 64-bit pointer marker; not part of the shared AST vocabulary.
        }
        if self.eat(b'I') {
            quals.restrict = true;
        }
        if self.eat(b'F') {
            quals.unaligned = true;
        }
        quals
    }

    fn memorize_symbol_identifier(&mut self, symbol: &NodeRef<'a>) {
        let Some(name) = symbol.child(0) else {
            return;
        };
        let identifier = innermost_name(&name);
        self.memorize_name(identifier);
    }

    fn read_pointer_auth(&mut self) -> Result<Option<(u64, u64, u64)>, ParseError> {
        if !self.consume(b"__ptrauth") {
            return Ok(None);
        }
        let first = self.read_unsigned()?;
        let second = self.read_unsigned()?;
        let third = self.read_unsigned()?;
        Ok(Some((first, second, third)))
    }

    fn read_variable_storage_class(&mut self) -> Result<&'static str, ParseError> {
        let Some(byte) = self.peek() else {
            return Err(self.error(ParseErrorKind::UnexpectedEnd));
        };
        self.bump(1)?;
        self.tick()?;
        match byte {
            b'0' => Ok("private-static"),
            b'1' => Ok("protected-static"),
            b'2' => Ok("public-static"),
            b'3' => Ok("global"),
            b'4' => Ok("function-local-static"),
            _ => Err(self.error(ParseErrorKind::UnexpectedByte)),
        }
    }

    // -- types -------------------------------------------------------------

    fn parse_type(&mut self, mode: TypeMode) -> Result<NodeRef<'a>, ParseError> {
        self.enter()?;
        let result = self.parse_type_inner(mode);
        self.leave();
        result
    }

    fn parse_type_inner(&mut self, mode: TypeMode) -> Result<NodeRef<'a>, ParseError> {
        let leading = if mode == TypeMode::Mangle || (mode == TypeMode::Result && self.eat(b'?')) {
            self.read_qualifiers()?.0
        } else {
            Quals::default()
        };
        let ty = self.parse_type_unqualified()?;
        self.apply_quals(ty, leading)
    }

    fn parse_type_unqualified(&mut self) -> Result<NodeRef<'a>, ParseError> {
        if matches!(self.peek(), Some(b'0'..=b'9')) {
            return self.backref_type();
        }
        let Some(byte) = self.peek() else {
            return Err(self.error(ParseErrorKind::UnexpectedEnd));
        };
        if byte == b'T'
            || byte == b'U'
            || byte == b'V'
            || (byte == b'W' && self.peek_at(1) == Some(b'4'))
        {
            return self.parse_tagged_type();
        }
        if self.is_pointer_type() {
            if self.is_member_pointer() {
                return self.parse_member_pointer_type();
            }
            return self.parse_pointer_type();
        }
        if byte == b'Y' {
            return self.parse_array_type();
        }
        if self.starts_with(b"$$A8@@") || self.starts_with(b"$$A6") {
            return self.parse_function_type_node();
        }
        if byte == b'?' {
            return self.parse_custom_type();
        }
        self.parse_primitive_type()
    }

    fn parse_tagged_type(&mut self) -> Result<NodeRef<'a>, ParseError> {
        let start = self.input.pos();
        let tag = match self.peek() {
            Some(b'T') => "union",
            Some(b'U') => "struct",
            Some(b'V') => "class",
            Some(b'W') => {
                if !self.consume(b"W4") {
                    return Err(self.error(ParseErrorKind::Unsupported));
                }
                "enum"
            }
            _ => return Err(self.error(ParseErrorKind::Unsupported)),
        };
        if tag != "enum" {
            self.bump(1)?;
            self.tick()?;
        }
        let name = self.parse_fully_qualified_type_name()?;
        self.make(
            Kind::TaggedType,
            Some(Cow::Borrowed(tag)),
            start,
            alloc::vec![name],
        )
    }

    fn is_pointer_type(&self) -> bool {
        self.starts_with(b"$$Q") || matches!(self.peek(), Some(b'A' | b'P' | b'Q' | b'R' | b'S'))
    }

    fn is_member_pointer(&self) -> bool {
        if self.starts_with(b"$$Q") {
            return false;
        }
        let Some(byte) = self.peek() else {
            return false;
        };
        if !matches!(byte, b'P' | b'Q' | b'R' | b'S') {
            return false;
        }
        let mut index = self.input.pos() + 1;
        if let Some(next) = self.bytes.get(index).copied()
            && next.is_ascii_digit()
        {
            return next == b'8';
        }
        while let Some(byte) = self.bytes.get(index).copied() {
            if matches!(byte, b'E' | b'I' | b'F') {
                index += 1;
            } else {
                break;
            }
        }
        matches!(
            self.bytes.get(index).copied(),
            Some(b'Q' | b'R' | b'S' | b'T')
        )
    }

    fn parse_pointer_type(&mut self) -> Result<NodeRef<'a>, ParseError> {
        let start = self.input.pos();
        let (pointer_quals, affinity) = self.read_pointer_cv_qualifiers()?;
        if self.eat(b'6') {
            let signature = self.parse_function_type(false)?;
            let function = self.make_function_type_node(start, signature)?;
            let node = match affinity {
                PointerAffinity::Pointer => {
                    self.make(Kind::PointerType, None, start, alloc::vec![function])?
                }
                PointerAffinity::Reference => {
                    self.make(Kind::ReferenceType, None, start, alloc::vec![function])?
                }
                PointerAffinity::RvalueReference => self.make(
                    Kind::RvalueReferenceType,
                    None,
                    start,
                    alloc::vec![function],
                )?,
            };
            return self.apply_quals(node, pointer_quals);
        }
        let ext = self.read_pointer_ext_qualifiers();
        let auth = self.read_pointer_auth()?;
        let pointee = self.parse_type(TypeMode::Mangle)?;
        let node = match affinity {
            PointerAffinity::Pointer => {
                self.make(Kind::PointerType, None, start, alloc::vec![pointee])?
            }
            PointerAffinity::Reference => {
                self.make(Kind::ReferenceType, None, start, alloc::vec![pointee])?
            }
            PointerAffinity::RvalueReference => {
                self.make(Kind::RvalueReferenceType, None, start, alloc::vec![pointee])?
            }
        };
        self.apply_quals(
            node,
            Quals {
                ptrauth: auth,
                ..pointer_quals.union(ext)
            },
        )
    }

    fn parse_member_pointer_type(&mut self) -> Result<NodeRef<'a>, ParseError> {
        let start = self.input.pos();
        let (pointer_quals, _affinity) = self.read_pointer_cv_qualifiers()?;
        let ext = self.read_pointer_ext_qualifiers();
        let pointer_quals = pointer_quals.union(ext);
        if self.eat(b'8') {
            let class = self.parse_fully_qualified_type_name()?;
            let signature = self.parse_function_type(true)?;
            let function = self.make_function_type_node(start, signature)?;
            let node = self.make(
                Kind::MemberPointerType,
                None,
                start,
                alloc::vec![class, function],
            )?;
            return self.apply_quals(node, pointer_quals);
        }
        let (pointee_quals, _member) = self.read_qualifiers()?;
        let class = self.parse_fully_qualified_type_name()?;
        let pointee = self.parse_type(TypeMode::Drop)?;
        let pointee = self.apply_quals(pointee, pointee_quals)?;
        let node = self.make(
            Kind::MemberPointerType,
            None,
            start,
            alloc::vec![class, pointee],
        )?;
        self.apply_quals(node, pointer_quals)
    }

    fn read_pointer_cv_qualifiers(&mut self) -> Result<(Quals, PointerAffinity), ParseError> {
        if self.consume(b"$$Q") {
            return Ok((Quals::default(), PointerAffinity::RvalueReference));
        }
        let Some(byte) = self.peek() else {
            return Err(self.error(ParseErrorKind::UnexpectedEnd));
        };
        self.bump(1)?;
        self.tick()?;
        match byte {
            b'A' => Ok((Quals::default(), PointerAffinity::Reference)),
            b'P' => Ok((Quals::default(), PointerAffinity::Pointer)),
            b'Q' => Ok((
                Quals {
                    const_: true,
                    ..Quals::default()
                },
                PointerAffinity::Pointer,
            )),
            b'R' => Ok((
                Quals {
                    volatile: true,
                    ..Quals::default()
                },
                PointerAffinity::Pointer,
            )),
            b'S' => Ok((
                Quals {
                    const_: true,
                    volatile: true,
                    ..Quals::default()
                },
                PointerAffinity::Pointer,
            )),
            _ => Err(self.error(ParseErrorKind::UnexpectedByte)),
        }
    }

    fn parse_array_type(&mut self) -> Result<NodeRef<'a>, ParseError> {
        self.expect(b'Y')?;
        let rank = self.read_unsigned()?;
        if rank == 0 {
            return Err(self.error(ParseErrorKind::InvalidLength));
        }
        let mut dimensions = Vec::new();
        for _ in 0..rank {
            dimensions.push(self.read_unsigned()?);
        }
        let quals = if self.consume(b"$$C") {
            let (element_quals, member) = self.read_qualifiers()?;
            if member {
                return Err(self.error(ParseErrorKind::UnexpectedByte));
            }
            element_quals
        } else {
            Quals::default()
        };
        let mut node = self.parse_type(TypeMode::Drop)?;
        for dimension in dimensions.into_iter().rev() {
            let text = Cow::Owned(alloc::format!("{dimension}"));
            let node_start = self.node_start(&node);
            node = self.make(Kind::ArrayType, Some(text), node_start, alloc::vec![node])?;
        }
        self.apply_quals(node, quals)
    }

    fn parse_function_type_node(&mut self) -> Result<NodeRef<'a>, ParseError> {
        let start = self.input.pos();
        let has_this = if self.consume(b"$$A8@@") {
            true
        } else {
            self.expect_text(b"$$A6")?;
            false
        };
        let signature = self.parse_function_type(has_this)?;
        self.make_function_type_node(start, signature)
    }

    fn make_function_type_node(
        &mut self,
        start: usize,
        signature: Signature<'a>,
    ) -> Result<NodeRef<'a>, ParseError> {
        let text = type_modifier_text(&signature);
        let mut children = Vec::with_capacity(2);
        if let Some(ret) = signature.ret {
            children.push(ret);
        }
        children.push(signature.params);
        self.make(Kind::FunctionType, text, start, children)
    }

    fn parse_custom_type(&mut self) -> Result<NodeRef<'a>, ParseError> {
        self.expect(b'?')?;
        let name = self.parse_unqualified_type_name(NameBackref::Simple)?;
        self.expect(b'@')?;
        Ok(name)
    }

    fn parse_primitive_type(&mut self) -> Result<NodeRef<'a>, ParseError> {
        let start = self.input.pos();
        if self.consume(b"$$T") {
            return self.make(
                Kind::BuiltinType,
                Some(Cow::Borrowed("std::nullptr_t")),
                start,
                Vec::new(),
            );
        }
        let Some(byte) = self.peek() else {
            return Err(self.error(ParseErrorKind::UnexpectedEnd));
        };
        self.bump(1)?;
        self.tick()?;
        let name = match byte {
            b'X' => "void",
            b'D' => "char",
            b'C' => "signed char",
            b'E' => "unsigned char",
            b'F' => "short",
            b'G' => "unsigned short",
            b'H' => "int",
            b'I' => "unsigned int",
            b'J' => "long",
            b'K' => "unsigned long",
            b'M' => "float",
            b'N' => "double",
            b'O' => "long double",
            b'_' => {
                let Some(sub) = self.peek() else {
                    return Err(self.error(ParseErrorKind::UnexpectedEnd));
                };
                self.bump(1)?;
                self.tick()?;
                match sub {
                    b'N' => "bool",
                    b'J' => "__int64",
                    b'K' => "unsigned __int64",
                    b'L' => "__int128",
                    b'M' => "unsigned __int128",
                    b'W' => "wchar_t",
                    b'Q' => "char8_t",
                    b'S' => "char16_t",
                    b'U' => "char32_t",
                    b'P' => "auto",
                    b'T' => "decltype(auto)",
                    _ => return Err(self.error(ParseErrorKind::Unsupported)),
                }
            }
            _ => return Err(self.error(ParseErrorKind::Unsupported)),
        };
        self.make(
            Kind::BuiltinType,
            Some(Cow::Borrowed(name)),
            start,
            Vec::new(),
        )
    }

    fn apply_quals(&mut self, node: NodeRef<'a>, quals: Quals) -> Result<NodeRef<'a>, ParseError> {
        if quals.is_empty() {
            return Ok(node);
        }
        let Some(text) = quals.text() else {
            return Ok(node);
        };
        let start = self.node_start(&node);
        self.make(
            Kind::QualifiedType,
            Some(Cow::Owned(text)),
            start,
            alloc::vec![node],
        )
    }

    fn make_identifier(
        &mut self,
        text: &'static str,
        start: usize,
    ) -> Result<NodeRef<'a>, ParseError> {
        let leaf = self.make(
            Kind::Identifier,
            Some(Cow::Borrowed(text)),
            start,
            Vec::new(),
        )?;
        self.make(Kind::UnqualifiedName, None, start, alloc::vec![leaf])
    }
}

/// How leading qualifiers are encoded in a type production.
#[derive(Clone, Copy, PartialEq, Eq)]
enum TypeMode {
    /// `<type>` with a leading cv qualifier byte.
    Mangle,
    /// `<type>` preceded by an optional `?` and a qualifier byte.
    Result,
    /// `<type>` with no leading qualifier byte.
    Drop,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum PointerAffinity {
    Pointer,
    Reference,
    RvalueReference,
}

#[derive(Clone, Copy)]
enum TableKind {
    Vftable,
    Vbtable,
    RttiCompleteObjectLocator,
}

fn basic_operator_spelling(code: u8) -> Option<&'static str> {
    Some(match code {
        b'2' => "operator new",
        b'3' => "operator delete",
        b'4' => "operator=",
        b'5' => "operator>>",
        b'6' => "operator<<",
        b'7' => "operator!",
        b'8' => "operator==",
        b'9' => "operator!=",
        b'A' => "operator[]",
        b'C' => "operator->",
        b'D' => "operator*",
        b'E' => "operator++",
        b'F' => "operator--",
        b'G' => "operator-",
        b'H' => "operator+",
        b'I' => "operator&",
        b'J' => "operator->*",
        b'K' => "operator/",
        b'L' => "operator%",
        b'M' => "operator<",
        b'N' => "operator<=",
        b'O' => "operator>",
        b'P' => "operator>=",
        b'Q' => "operator,",
        b'R' => "operator()",
        b'S' => "operator~",
        b'T' => "operator^",
        b'U' => "operator|",
        b'V' => "operator&&",
        b'W' => "operator||",
        b'X' => "operator*=",
        b'Y' => "operator+=",
        b'Z' => "operator-=",
        _ => return None,
    })
}

fn under_operator_spelling(code: u8) -> Option<&'static str> {
    Some(match code {
        b'0' => "operator/=",
        b'1' => "operator%=",
        b'2' => "operator>>=",
        b'3' => "operator<<=",
        b'4' => "operator&=",
        b'5' => "operator|=",
        b'6' => "operator^=",
        b'D' => "`vbase dtor'",
        b'E' => "`vector deleting dtor'",
        b'F' => "`default ctor closure'",
        b'G' => "`scalar deleting dtor'",
        b'H' => "`vector ctor iterator'",
        b'I' => "`vector dtor iterator'",
        b'J' => "`vector vbase ctor iterator'",
        b'K' => "`virtual displacement map'",
        b'L' => "`eh vector ctor iterator'",
        b'M' => "`eh vector dtor iterator'",
        b'N' => "`eh vector vbase ctor iterator'",
        b'O' => "`copy ctor closure'",
        b'T' => "`local vftable ctor closure'",
        b'U' => "operator new[]",
        b'V' => "operator delete[]",
        b'X' => "`placement delete closure'",
        b'Y' => "`placement delete[] closure'",
        _ => return None,
    })
}

fn double_under_operator_spelling(code: u8) -> Option<&'static str> {
    Some(match code {
        b'A' => "`managed vector ctor iterator'",
        b'B' => "`managed vector dtor iterator'",
        b'C' => "`eh vector copy ctor iterator'",
        b'D' => "`eh vector vbase copy ctor iterator'",
        b'G' => "`vector copy ctor iterator'",
        b'H' => "`vector vbase copy ctor iterator'",
        b'I' => "`managed vector vbase copy ctor iterator'",
        b'L' => "operator co_await",
        b'M' => "operator<=>",
        _ => return None,
    })
}

fn conversion_target(node: &NodeRef<'_>) -> Option<usize> {
    if node.kind() == Kind::UnqualifiedName
        && let Some(child) = node.child(0)
        && child.kind() == Kind::OperatorName
        && child.text() == Some("operator")
        && child.child_count() == 0
    {
        return node.span().map(|range| range.start as usize);
    }
    None
}

fn modifier_text(
    func_class: &FuncClass,
    signature: &Signature<'_>,
    extern_c: bool,
) -> Option<Cow<'static, str>> {
    let mut parts: Vec<String> = Vec::new();
    if extern_c {
        parts.push("extern \"C\"".into());
    }
    if let Some(access) = func_class.access {
        parts.push(access.into());
    }
    if func_class.static_ {
        parts.push("static".into());
    } else if func_class.virtual_ {
        parts.push("virtual".into());
    }
    if !signature.calling_conv.is_empty() {
        parts.push(signature.calling_conv.into());
    }
    if let Some(quals) = signature.quals.text() {
        parts.push(quals);
    }
    if let Some(reference) = signature.ref_qual {
        parts.push(reference.into());
    }
    if parts.is_empty() {
        None
    } else {
        Some(Cow::Owned(parts.join(" ")))
    }
}

fn type_modifier_text(signature: &Signature<'_>) -> Option<Cow<'static, str>> {
    let mut parts: Vec<String> = Vec::new();
    if !signature.calling_conv.is_empty() {
        parts.push(signature.calling_conv.into());
    }
    if let Some(quals) = signature.quals.text() {
        parts.push(quals);
    }
    if let Some(reference) = signature.ref_qual {
        parts.push(reference.into());
    }
    if parts.is_empty() {
        None
    } else {
        Some(Cow::Owned(parts.join(" ")))
    }
}

/// True when a name is a constructor, destructor or conversion operator.
fn is_structor_or_conversion(node: &NodeRef<'_>) -> bool {
    match node.kind() {
        Kind::UnqualifiedName => node.child(0).is_some_and(|child| {
            matches!(child.kind(), Kind::CtorDtorName)
                || (child.kind() == Kind::OperatorName
                    && child.text() == Some("operator")
                    && child.child_count() == 0)
        }),
        Kind::Name => node
            .child(0)
            .is_some_and(|child| is_structor_or_conversion(&child)),
        _ => false,
    }
}

/// The innermost unqualified component of a qualified name.
fn innermost_name<'a>(node: &NodeRef<'a>) -> NodeRef<'a> {
    match node.kind() {
        Kind::NestedName => node
            .child(1)
            .map_or_else(|| Rc::clone(node), |child| innermost_name(&child)),
        Kind::LocalName => node
            .child(1)
            .map_or_else(|| Rc::clone(node), |child| innermost_name(&child)),
        _ => Rc::clone(node),
    }
}

/// The pointer-like kind under any cv-qualifier wrappers.
fn pointer_like_kind(node: &NodeRef<'_>) -> Option<Kind> {
    match node.kind() {
        Kind::PointerType
        | Kind::ReferenceType
        | Kind::RvalueReferenceType
        | Kind::MemberPointerType => Some(node.kind()),
        Kind::QualifiedType => node.child(0).and_then(|child| pointer_like_kind(&child)),
        _ => None,
    }
}

/// A child of a node, reporting a malformed node instead of panicking.
fn child_of<'a>(node: &NodeRef<'a>, index: usize) -> Result<NodeRef<'a>, ParseError> {
    node.child(index).ok_or_else(|| {
        let position = node.span().map_or(0, |range| range.start as usize);
        ParseError::new(position, ParseErrorKind::UnexpectedEnd)
    })
}

/// True for a template argument that denotes a type and can therefore be
/// addressed by a type back-reference.
fn is_template_type_arg(node: &NodeRef<'_>) -> bool {
    matches!(
        node.kind(),
        Kind::BuiltinType
            | Kind::TaggedType
            | Kind::PointerType
            | Kind::ReferenceType
            | Kind::RvalueReferenceType
            | Kind::ArrayType
            | Kind::FunctionType
            | Kind::MemberPointerType
            | Kind::QualifiedType
            | Kind::Name
            | Kind::UnqualifiedName
            | Kind::NestedName
            | Kind::Identifier
            | Kind::UnnamedType
    )
}

fn unnamed_type_text(text: &[u8]) -> Option<&[u8]> {
    let inner = text.strip_prefix(b"<")?.strip_suffix(b">")?;
    if inner.starts_with(b"unnamed-type-")
        || inner.starts_with(b"lambda_")
        || inner.starts_with(b"unnamed-enum-")
        || inner == b"decltype-auto"
        || inner == b"auto"
    {
        Some(inner)
    } else {
        None
    }
}

fn rebased_hex(byte: u8) -> Option<u8> {
    match byte {
        b'A'..=b'J' => Some(byte - b'A'),
        b'K'..=b'P' => Some(10 + byte - b'K'),
        _ => None,
    }
}

/// A stable key for deduplicating memorized names, mirroring LLVM's
/// "render the name and compare the text" rule.
fn node_key(node: &NodeRef<'_>) -> Option<String> {
    node_key_inner(node, 0)
}

fn node_key_inner(node: &NodeRef<'_>, depth: u32) -> Option<String> {
    if depth > 16 {
        return None;
    }
    Some(match node.kind() {
        Kind::Identifier
        | Kind::UnnamedType
        | Kind::OperatorName
        | Kind::CtorDtorName
        | Kind::Substitution
        | Kind::TemplateParam
        | Kind::BuiltinType
        | Kind::Md5Name => node.text()?.into(),
        Kind::UnqualifiedName => node_key_inner(&node.child(0)?, depth + 1)?,
        Kind::NestedName => {
            let prefix = node_key_inner(&node.child(0)?, depth + 1)?;
            let inner = node_key_inner(&node.child(1)?, depth + 1)?;
            alloc::format!("{prefix}::{inner}")
        }
        Kind::Name => {
            let base = node_key_inner(&node.child(0)?, depth + 1)?;
            let args = node.child(1)?;
            let mut rendered = String::new();
            for child in args.children() {
                rendered.push_str(&node_key_inner(child, depth + 1).unwrap_or_default());
                rendered.push(',');
            }
            alloc::format!("{base}<{rendered}>")
        }
        Kind::LocalName => alloc::format!("local:{}", node.text().unwrap_or("")),
        Kind::TaggedType => alloc::format!(
            "{} {}",
            node.text().unwrap_or(""),
            node_key_inner(&node.child(0)?, depth + 1)?
        ),
        Kind::QualifiedType => alloc::format!(
            "{} {}",
            node.text().unwrap_or(""),
            node_key_inner(&node.child(0)?, depth + 1)?
        ),
        Kind::PointerType => alloc::format!("*{}", node_key_inner(&node.child(0)?, depth + 1)?),
        Kind::ReferenceType => {
            alloc::format!("&{}", node_key_inner(&node.child(0)?, depth + 1)?)
        }
        Kind::RvalueReferenceType => {
            alloc::format!("&&{}", node_key_inner(&node.child(0)?, depth + 1)?)
        }
        Kind::ArrayType => alloc::format!(
            "[{}]{}",
            node.text().unwrap_or(""),
            node_key_inner(&node.child(0)?, depth + 1)?
        ),
        Kind::FunctionType => {
            let mut rendered = String::from("fn(");
            for child in node.children() {
                rendered.push_str(&node_key_inner(child, depth + 1).unwrap_or_default());
                rendered.push(',');
            }
            rendered.push(')');
            rendered
        }
        Kind::MemberPointerType => alloc::format!(
            "{}::*{}",
            node_key_inner(&node.child(0)?, depth + 1)?,
            node_key_inner(&node.child(1)?, depth + 1)?
        ),
        _ => node.text()?.into(),
    })
}
