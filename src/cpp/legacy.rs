//! GNU v2 (old GNU, cfront-era) C++ demangling.
//!
//! GNU v2 is the mangling GCC used before the Itanium C++ ABI. A legacy name
//! carries no version prefix: the name is spelled out and the signature
//! follows a `__` separator, as in `foo__Fiv` (`foo(int, void)`) or
//! `bar__3foo` (`foo::bar(void)`). The separator is not unique — `a__b__c`
//! demangles as `a__b(char)`, not `b::a(...)` — so the parser tries every
//! `__` split point and accepts the first one that consumes the whole symbol.
//! A prefix match alone never selects the dialect.
//!
//! # Supported grammar
//!
//! - `__`-separated function names, the empty-name constructor form
//!   (`__3foo`) and the `_$_` destructor form;
//! - unqualified names, `Q<n>`-qualified names, template-ids and the encoded
//!   `static`/`const`/`volatile`/`__restrict` function qualifiers;
//! - operator names (`__pl`, `op$pl`, `op$assign_pl`) and conversion
//!   operators (`__op<type>`, `type$<type>`);
//! - `F`-suffixed parameter lists, back-references (`T<n>`, `N<count><n>`)
//!   and the trailing `e` ellipsis;
//! - builtin types with `const`/`volatile`/`__restrict`, `unsigned`,
//!   `signed` and `__complex`; pointers, references, rvalue references,
//!   arrays, function types and member-function pointers;
//! - template argument lists with type arguments (`Z<type>`) and integral
//!   value arguments;
//! - the `__vt_`/`_vt$` virtual table, `__ti`/`__tf` type information and
//!   `__thunk_` thunk special names, and `_<name><marker><variable>` static
//!   data members.
//!
//! Unsupported productions are rejected rather than approximated: squangling
//! (`K`, `B`, `n`), member templates (`H`), template template parameters,
//! `X`/`Y` template parameter references, `_GLOBAL_$I$`/`_GLOBAL_$D$` linker
//! bookkeeping names, and the HP/ARM/Lucid extensions.
//!
//! # Spelling
//!
//! The text follows the legacy `c++filt` conventions: an empty parameter list
//! prints as `void`, cv-qualifiers follow the type they qualify
//! (`int const *`), virtual tables print as `<type> virtual table`, type
//! information as `<type> type_info node`, and thunks as
//! `virtual function thunk (delta:<n>) for <symbol>`. Two historical
//! spellings are defects rather than conventions and are not reproduced: an
//! unnamed function-pointer parameter prints as `void (*)(void)` instead of
//! ` (*)(void)`, and a member-function pointer prints as
//! `void (foo::*)(void)` instead of ` (foo::)(void)`.
//!
//! # Budgets
//!
//! Parsing is bounded by the shared C++ input limit and by explicit depth,
//! node and work budgets. Printing is bounded by depth, visit and output-byte
//! budgets. A symbol that exceeds a budget is rejected, never truncated.

use alloc::borrow::Cow;
use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;
use core::fmt::{self, Write};

use super::node::{Kind, Node, NodeRef};
use super::{Ast, MAX_INPUT_LEN, ParseError, ParseErrorKind};

/// Maximum nesting depth for recursive productions.
const MAX_DEPTH: u32 = 64;
/// Maximum nodes one legacy parse may allocate.
const MAX_NODES: u32 = 1 << 12;
/// Minimum work allowance for very short symbols.
const MIN_WORK: usize = 1 << 12;
/// Work allowance per input byte.
const WORK_PER_BYTE: usize = 32;
/// Maximum number of `__` split points tried for one symbol.
const MAX_SPLITS: usize = 64;

/// Maximum print recursion depth.
const MAX_PRINT_DEPTH: u32 = 128;
/// Maximum node visits for one print run.
const MAX_PRINT_VISITS: usize = 1 << 16;
/// Maximum bytes written for one symbol.
const MAX_OUTPUT_BYTES: usize = 1 << 20;

/// The `CPLUS_MARKER` characters GNU v2 accepts inside special names.
const MARKERS: [u8; 2] = *b"$.";

/// True when `symbol` is a complete GNU v2 mangled name.
///
/// The legacy grammar has no unambiguous prefix, so this validates the whole
/// symbol with a bounded parse instead of matching a prefix. It is the
/// predicate scheme detection uses to tell a legacy C++ name from a plain C
/// name.
#[must_use]
pub(crate) fn is_legacy_name(symbol: &str) -> bool {
    looks_like_legacy(symbol) && parse(symbol).is_ok()
}

/// Cheap pre-filter: every legacy name has a `__` separator or a legacy
/// special-name marker.
pub(crate) fn looks_like_legacy(symbol: &str) -> bool {
    if symbol.len() < 3 || symbol.len() > MAX_INPUT_LEN {
        return false;
    }
    if symbol.starts_with("_Z") || symbol.starts_with('?') {
        return false;
    }
    symbol.contains("__") || (symbol.starts_with('_') && symbol.contains(['$', '.']))
}

/// Parse a GNU v2 mangled name into the shared C++ AST.
pub(crate) fn parse(symbol: &str) -> Result<Ast<'_>, ParseError> {
    if symbol.len() > MAX_INPUT_LEN {
        return Err(ParseError::new(0, ParseErrorKind::TooLong));
    }
    Parser::new(symbol).parse_ast()
}

/// Recursive-descent parser over one legacy symbol.
struct Parser<'a> {
    symbol: &'a str,
    bytes: &'a [u8],
    pos: usize,
    depth: u32,
    nodes: u32,
    work: usize,
    work_limit: usize,
    /// Types remembered for `T<n>`/`N<count><n>` back-references, in the
    /// order the demangler records them.
    types: Vec<NodeRef<'a>>,
    /// Non-zero inside a nested parameter list, where types are not
    /// remembered.
    forgetting: u32,
}

impl<'a> Parser<'a> {
    fn new(symbol: &'a str) -> Self {
        let work_limit = symbol
            .len()
            .saturating_mul(WORK_PER_BYTE)
            .saturating_add(MIN_WORK);
        Self {
            symbol,
            bytes: symbol.as_bytes(),
            pos: 0,
            depth: 0,
            nodes: 0,
            work: 0,
            work_limit,
            types: Vec::new(),
            forgetting: 0,
        }
    }

    // -- parser plumbing ---------------------------------------------------

    fn error(&self, kind: ParseErrorKind) -> ParseError {
        ParseError::new(self.pos, kind)
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }

    fn at(&self, index: usize) -> Option<u8> {
        self.bytes.get(index).copied()
    }

    fn starts_at(&self, index: usize, text: &[u8]) -> bool {
        self.bytes
            .get(index..)
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

    fn eat_marker(&mut self) -> bool {
        match self.peek() {
            Some(byte) if MARKERS.contains(&byte) => {
                self.pos += 1;
                true
            }
            _ => false,
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

    fn make_parameter(&mut self, ty: NodeRef<'a>) -> Result<NodeRef<'a>, ParseError> {
        let start = ty.span().map_or(self.pos, |range| range.start as usize);
        self.make(Kind::Parameter, None, start, alloc::vec![ty])
    }

    /// Remember a type for back-references, unless a nested parameter list
    /// turned remembering off.
    fn remember(&mut self, node: &NodeRef<'a>) {
        if self.forgetting == 0 {
            self.types.push(Rc::clone(node));
        }
    }

    // -- entry points ------------------------------------------------------

    fn parse_ast(mut self) -> Result<Ast<'a>, ParseError> {
        let shape = self.parse_symbol(0)?;
        if self.pos != self.bytes.len() {
            return Err(self.error(ParseErrorKind::UnexpectedByte));
        }
        let root = self.make(Kind::Encoding, None, 0, alloc::vec![shape])?;
        Ok(Ast::from_root_gnu_v2(root))
    }

    /// Parse one legacy entity starting at `start` and return its shape node.
    fn parse_symbol(&mut self, start: usize) -> Result<NodeRef<'a>, ParseError> {
        self.enter()?;
        let result = self.parse_symbol_inner(start);
        self.leave();
        result
    }

    fn parse_symbol_inner(&mut self, start: usize) -> Result<NodeRef<'a>, ParseError> {
        self.tick()?;
        self.pos = start;
        if let Some(shape) = self.parse_special(start)? {
            return Ok(shape);
        }
        // The constructor form spells no name: `__<class>` is the constructor
        // of `<class>`, and `__H...` is the unsupported member-template form.
        if self.starts_at(start, b"__") {
            match self.at(start + 2) {
                Some(b'H') => return Err(self.error(ParseErrorKind::Unsupported)),
                Some(b'0'..=b'9' | b'Q' | b't' | b'K') => {
                    self.pos = start + 2;
                    return self.parse_entity(start, None, true, false);
                }
                _ => {}
            }
        }
        self.parse_split(start)
    }

    /// Try every `__` split point; the first one that parses the whole
    /// symbol wins.
    fn parse_split(&mut self, start: usize) -> Result<NodeRef<'a>, ParseError> {
        let mut splits = [0_usize; MAX_SPLITS];
        let count = split_positions(&self.bytes[start..], start, &mut splits);
        if count == 0 {
            return Err(ParseError::new(start, ParseErrorKind::NotLegacy));
        }
        let mut first_error = None;
        for &split in &splits[..count] {
            self.pos = start;
            self.types.clear();
            self.forgetting = 0;
            let attempt = (|| {
                let name = self.parse_name_part(start, split)?;
                self.pos = split + 2;
                let shape = self.parse_entity(start, name, false, false)?;
                if self.pos != self.bytes.len() {
                    return Err(self.error(ParseErrorKind::UnexpectedByte));
                }
                Ok(shape)
            })();
            match attempt {
                Ok(shape) => return Ok(shape),
                Err(error) => {
                    if is_fatal(error.kind()) {
                        return Err(error);
                    }
                    if first_error.is_none() {
                        first_error = Some(error);
                    }
                }
            }
        }
        Err(first_error.unwrap_or_else(|| ParseError::new(start, ParseErrorKind::NotLegacy)))
    }

    // -- special names -----------------------------------------------------

    /// Parse a GNU v2 special name, or return `None` when `start` does not
    /// begin one.
    fn parse_special(&mut self, start: usize) -> Result<Option<NodeRef<'a>>, ParseError> {
        if self.starts_at(start, b"__vt_")
            || self.starts_at(start, b"_vt$")
            || self.starts_at(start, b"_vt.")
        {
            let prefix = if self.starts_at(start, b"__vt_") {
                5
            } else {
                4
            };
            self.pos = start + prefix;
            let target = self.parse_dotted_name(start)?;
            return Ok(Some(self.make(
                Kind::VTable,
                None,
                start,
                alloc::vec![target],
            )?));
        }
        if self.starts_at(start, b"__ti") || self.starts_at(start, b"__tf") {
            let function = self.at(start + 3) == Some(b'f');
            self.pos = start + 4;
            let target = self.parse_type()?;
            let text = function.then_some(Cow::Borrowed("function"));
            return Ok(Some(self.make(
                Kind::TypeInfo,
                text,
                start,
                alloc::vec![target],
            )?));
        }
        if self.starts_at(start, b"__thunk_") {
            self.pos = start + 8;
            let delta = self.parse_count()?;
            self.expect(b'_')?;
            let shape = self.parse_symbol(self.pos)?;
            let text = Cow::Owned(alloc::format!("-{delta}"));
            return Ok(Some(self.make(
                Kind::Thunk,
                Some(text),
                start,
                alloc::vec![shape],
            )?));
        }
        if self.starts_at(start, b"_GLOBAL_") {
            // Linker bookkeeping for file-scope constructors and destructors.
            return Err(self.error(ParseErrorKind::Unsupported));
        }
        if let Some(shape) = self.parse_destructor(start)? {
            return Ok(Some(shape));
        }
        self.parse_static_data_member(start)
    }

    /// `_<marker>_<name>`: a destructor.
    fn parse_destructor(&mut self, start: usize) -> Result<Option<NodeRef<'a>>, ParseError> {
        let Some(marker) = self.at(start + 1) else {
            return Ok(None);
        };
        if self.at(start) != Some(b'_')
            || !MARKERS.contains(&marker)
            || self.at(start + 2) != Some(b'_')
        {
            return Ok(None);
        }
        self.pos = start + 3;
        Ok(Some(self.parse_entity(start, None, false, true)?))
    }

    /// `_<name><marker><variable>`: a static data member.
    fn parse_static_data_member(
        &mut self,
        start: usize,
    ) -> Result<Option<NodeRef<'a>>, ParseError> {
        if self.at(start) != Some(b'_') {
            return Ok(None);
        }
        let Some(kind) = self.at(start + 1) else {
            return Ok(None);
        };
        if !(kind.is_ascii_digit() || matches!(kind, b'Q' | b't' | b'K')) {
            return Ok(None);
        }
        if !self.bytes[start..]
            .iter()
            .any(|byte| MARKERS.contains(byte))
        {
            return Ok(None);
        }
        self.pos = start + 1;
        let class = match self.peek() {
            Some(b'Q') => self.parse_qualified_name()?,
            Some(b't') => self.parse_template()?,
            _ => self.parse_class_name()?,
        };
        if !self.eat_marker() {
            return Err(self.error(ParseErrorKind::UnexpectedByte));
        }
        let variable_start = self.pos;
        let variable = &self.symbol[variable_start..];
        if variable.is_empty() {
            return Err(self.error(ParseErrorKind::UnexpectedEnd));
        }
        self.pos = self.bytes.len();
        let leaf = self.make(
            Kind::Identifier,
            Some(Cow::Borrowed(variable)),
            variable_start,
            Vec::new(),
        )?;
        let name = self.make(Kind::NestedName, None, start + 1, alloc::vec![class, leaf])?;
        Ok(Some(self.make(
            Kind::Data,
            None,
            start,
            alloc::vec![name],
        )?))
    }

    /// Parse a `<marker>`-separated name, as used by the virtual table form.
    fn parse_dotted_name(&mut self, start: usize) -> Result<NodeRef<'a>, ParseError> {
        let mut components: Vec<NodeRef<'a>> = Vec::new();
        loop {
            if self.peek() == Some(b'_') {
                self.pos += 1;
            }
            let component = match self.peek() {
                Some(b'Q') => self.parse_qualified_name()?,
                Some(b't') => self.parse_template()?,
                Some(byte) if byte.is_ascii_digit() => self.parse_class_name()?,
                _ => self.parse_raw_name()?,
            };
            components.push(component);
            if self.pos == self.bytes.len() {
                break;
            }
            if !self.eat_marker() {
                return Err(self.error(ParseErrorKind::UnexpectedByte));
            }
        }
        self.nest(start, components)
    }

    /// Parse a run of literal name bytes.
    fn parse_raw_name(&mut self) -> Result<NodeRef<'a>, ParseError> {
        let start = self.pos;
        while let Some(byte) = self.peek() {
            if MARKERS.contains(&byte) || byte == b'_' && self.at(self.pos + 1) == Some(b'_') {
                break;
            }
            self.pos += 1;
        }
        if self.pos == start {
            return Err(self.error(ParseErrorKind::UnexpectedByte));
        }
        let text = &self.symbol[start..self.pos];
        let ident = self.make(
            Kind::Identifier,
            Some(Cow::Borrowed(text)),
            start,
            Vec::new(),
        )?;
        self.make(Kind::UnqualifiedName, None, start, alloc::vec![ident])
    }

    // -- names -------------------------------------------------------------

    /// Parse the name part of a `__`-separated legacy name.
    fn parse_name_part(
        &mut self,
        start: usize,
        end: usize,
    ) -> Result<Option<NodeRef<'a>>, ParseError> {
        if end <= start {
            return Ok(None);
        }
        let text = &self.symbol[start..end];
        if text.chars().any(|ch| ch.is_whitespace() || ch.is_control()) {
            return Err(self.error(ParseErrorKind::UnexpectedByte));
        }
        if let Some(spelling) = operator_spelling_from_name(text) {
            self.pos = end;
            let operator = self.make(
                Kind::OperatorName,
                Some(Cow::Borrowed(spelling)),
                start,
                Vec::new(),
            )?;
            return Ok(Some(self.make(
                Kind::UnqualifiedName,
                None,
                start,
                alloc::vec![operator],
            )?));
        }
        if let Some(offset) = conversion_operator_offset(text) {
            self.pos = start + offset;
            let target = self.parse_type()?;
            if self.pos != end {
                return Err(self.error(ParseErrorKind::UnexpectedByte));
            }
            let operator = self.make(
                Kind::OperatorName,
                Some(Cow::Borrowed("operator")),
                start,
                alloc::vec![target],
            )?;
            return Ok(Some(self.make(
                Kind::UnqualifiedName,
                None,
                start,
                alloc::vec![operator],
            )?));
        }
        self.pos = end;
        let ident = self.make(
            Kind::Identifier,
            Some(Cow::Borrowed(text)),
            start,
            Vec::new(),
        )?;
        Ok(Some(self.make(
            Kind::UnqualifiedName,
            None,
            start,
            alloc::vec![ident],
        )?))
    }

    /// Parse the signature of one entity and combine it with its name.
    fn parse_entity(
        &mut self,
        start: usize,
        name_part: Option<NodeRef<'a>>,
        constructor: bool,
        destructor: bool,
    ) -> Result<NodeRef<'a>, ParseError> {
        self.enter()?;
        let result = self.parse_entity_inner(start, name_part, constructor, destructor);
        self.leave();
        result
    }

    fn parse_entity_inner(
        &mut self,
        start: usize,
        name_part: Option<NodeRef<'a>>,
        constructor: bool,
        destructor: bool,
    ) -> Result<NodeRef<'a>, ParseError> {
        let mut static_function = false;
        let mut qualifiers: Vec<&'static str> = Vec::new();
        let mut component = None;
        loop {
            match self.peek() {
                Some(b'S') => {
                    self.pos += 1;
                    static_function = true;
                }
                Some(b'C') => {
                    self.pos += 1;
                    qualifiers.push("const");
                }
                Some(b'V') => {
                    self.pos += 1;
                    qualifiers.push("volatile");
                }
                Some(b'u') => {
                    self.pos += 1;
                    qualifiers.push("__restrict");
                }
                Some(b'K' | b'H' | b'B') => {
                    return Err(self.error(ParseErrorKind::Unsupported));
                }
                Some(b'Q') => {
                    component = Some(self.parse_qualified_name()?);
                    break;
                }
                Some(b't') => {
                    component = Some(self.parse_template()?);
                    break;
                }
                Some(byte) if byte.is_ascii_digit() => {
                    component = Some(self.parse_class_name()?);
                    break;
                }
                Some(b'_') => return Err(self.error(ParseErrorKind::UnexpectedByte)),
                _ => break,
            }
        }
        if let Some(component) = &component {
            self.remember(component);
        }
        if self.peek() == Some(b'F') {
            self.pos += 1;
        }
        let parameters = self.parse_args()?;
        let name = self.combine_name(start, component, name_part, constructor, destructor)?;
        let text = modifier_text(static_function, &qualifiers);
        self.make(Kind::Function, text, start, alloc::vec![name, parameters])
    }

    /// Combine the qualified prefix, the spelled name and a constructor or
    /// destructor marker into one name node.
    fn combine_name(
        &mut self,
        start: usize,
        component: Option<NodeRef<'a>>,
        name_part: Option<NodeRef<'a>>,
        constructor: bool,
        destructor: bool,
    ) -> Result<NodeRef<'a>, ParseError> {
        if constructor || destructor {
            let Some(class) = component else {
                return Err(self.error(ParseErrorKind::Unsupported));
            };
            let code = if destructor { "D1" } else { "C1" };
            let leaf = self.make(
                Kind::CtorDtorName,
                Some(Cow::Borrowed(code)),
                start,
                Vec::new(),
            )?;
            let leaf = self.make(Kind::UnqualifiedName, None, start, alloc::vec![leaf])?;
            return self.make(Kind::NestedName, None, start, alloc::vec![class, leaf]);
        }
        match (component, name_part) {
            (Some(prefix), Some(leaf)) => {
                self.make(Kind::NestedName, None, start, alloc::vec![prefix, leaf])
            }
            (None, Some(leaf)) => Ok(leaf),
            _ => Err(self.error(ParseErrorKind::Unsupported)),
        }
    }

    /// Fold name components into a left-nested qualified name.
    fn nest(
        &mut self,
        start: usize,
        components: Vec<NodeRef<'a>>,
    ) -> Result<NodeRef<'a>, ParseError> {
        let mut components = components.into_iter();
        let Some(mut name) = components.next() else {
            return Err(self.error(ParseErrorKind::UnexpectedEnd));
        };
        for component in components {
            name = self.make(Kind::NestedName, None, start, alloc::vec![name, component])?;
        }
        Ok(name)
    }

    /// Parse a `<length><characters>` source name.
    fn parse_class_name(&mut self) -> Result<NodeRef<'a>, ParseError> {
        let start = self.pos;
        let length = self.parse_count()?;
        if length == 0 {
            return Err(self.error(ParseErrorKind::InvalidLength));
        }
        let text_start = self.pos;
        let text_end = text_start
            .checked_add(length)
            .ok_or_else(|| self.error(ParseErrorKind::InvalidLength))?;
        let Some(text) = self.symbol.get(text_start..text_end) else {
            return Err(self.error(ParseErrorKind::InvalidLength));
        };
        self.pos = text_end;
        let ident = self.make(
            Kind::Identifier,
            Some(Cow::Borrowed(text)),
            text_start,
            Vec::new(),
        )?;
        self.make(Kind::UnqualifiedName, None, start, alloc::vec![ident])
    }

    /// Parse a `Q<n>_` qualified name.
    fn parse_qualified_name(&mut self) -> Result<NodeRef<'a>, ParseError> {
        let start = self.pos;
        self.expect(b'Q')?;
        let count = match self.peek() {
            Some(b'_') => {
                self.pos += 1;
                let count = self.parse_count()?;
                self.expect(b'_')?;
                count
            }
            Some(digit @ b'1'..=b'9') => {
                self.pos += 1;
                if self.peek() == Some(b'_') {
                    self.pos += 1;
                }
                usize::from(digit - b'0')
            }
            _ => return Err(self.error(ParseErrorKind::UnexpectedByte)),
        };
        let mut components = Vec::new();
        for _ in 0..count {
            if self.peek() == Some(b'_') {
                self.pos += 1;
            }
            let component = match self.peek() {
                Some(b't') => self.parse_template()?,
                _ => self.parse_type()?,
            };
            components.push(component);
        }
        self.nest(start, components)
    }

    /// Parse a `t<length><name><count><argument>*` template-id.
    fn parse_template(&mut self) -> Result<NodeRef<'a>, ParseError> {
        let start = self.pos;
        self.expect(b't')?;
        if self.peek() == Some(b'z') {
            // A template template parameter argument.
            return Err(self.error(ParseErrorKind::Unsupported));
        }
        let length = self.parse_count()?;
        if length == 0 {
            return Err(self.error(ParseErrorKind::InvalidLength));
        }
        let text_start = self.pos;
        let text_end = text_start
            .checked_add(length)
            .ok_or_else(|| self.error(ParseErrorKind::InvalidLength))?;
        let Some(text) = self.symbol.get(text_start..text_end) else {
            return Err(self.error(ParseErrorKind::InvalidLength));
        };
        self.pos = text_end;
        let ident = self.make(
            Kind::Identifier,
            Some(Cow::Borrowed(text)),
            text_start,
            Vec::new(),
        )?;
        let base = self.make(Kind::UnqualifiedName, None, start, alloc::vec![ident])?;
        let arguments_start = self.pos;
        let count = self.parse_get_count()?;
        let mut arguments = Vec::new();
        for _ in 0..count {
            match self.peek() {
                Some(b'Z') => {
                    self.pos += 1;
                    arguments.push(self.parse_type()?);
                }
                Some(b'z') => return Err(self.error(ParseErrorKind::Unsupported)),
                _ => {
                    let argument_start = self.pos;
                    let ty = self.parse_type()?;
                    let value = self.parse_integral_value()?;
                    let literal = self.make(
                        Kind::Literal,
                        Some(Cow::Owned(value)),
                        argument_start,
                        alloc::vec![ty],
                    )?;
                    arguments.push(literal);
                }
            }
        }
        let arguments = self.make(Kind::TemplateArgs, None, arguments_start, arguments)?;
        self.make(Kind::Name, None, start, alloc::vec![base, arguments])
    }

    /// Parse the value of an integral template argument.
    fn parse_integral_value(&mut self) -> Result<String, ParseError> {
        let start = self.pos;
        if self.peek() == Some(b'-') {
            self.pos += 1;
        }
        let digits_start = self.pos;
        while self.peek().is_some_and(|byte| byte.is_ascii_digit()) {
            self.pos += 1;
        }
        if self.pos == digits_start {
            return Err(self.error(ParseErrorKind::UnexpectedByte));
        }
        Ok(String::from(&self.symbol[start..self.pos]))
    }

    // -- types -------------------------------------------------------------

    fn parse_type(&mut self) -> Result<NodeRef<'a>, ParseError> {
        self.enter()?;
        let result = self.parse_type_inner();
        self.leave();
        result
    }

    fn parse_type_inner(&mut self) -> Result<NodeRef<'a>, ParseError> {
        let start = self.pos;
        match self.peek() {
            None => Err(self.error(ParseErrorKind::UnexpectedEnd)),
            Some(b'P' | b'p') => {
                self.pos += 1;
                let inner = self.parse_type()?;
                self.make(Kind::PointerType, None, start, alloc::vec![inner])
            }
            Some(b'R') => {
                self.pos += 1;
                let inner = self.parse_type()?;
                self.make(Kind::ReferenceType, None, start, alloc::vec![inner])
            }
            Some(b'O') => {
                self.pos += 1;
                let inner = self.parse_type()?;
                self.make(Kind::RvalueReferenceType, None, start, alloc::vec![inner])
            }
            Some(b'A') => {
                self.pos += 1;
                let dimension = if self.peek() == Some(b'_') {
                    None
                } else {
                    Some(Cow::Owned(self.parse_integral_value()?))
                };
                self.expect(b'_')?;
                let inner = self.parse_type()?;
                self.make(Kind::ArrayType, dimension, start, alloc::vec![inner])
            }
            Some(b'F') => {
                self.pos += 1;
                self.forgetting += 1;
                let parameters = self.parse_args();
                self.forgetting -= 1;
                let parameters = parameters?;
                if self.eat(b'_') {
                    self.tick()?;
                }
                self.make(Kind::FunctionType, None, start, alloc::vec![parameters])
            }
            Some(b'M') => {
                self.pos += 1;
                let class = match self.peek() {
                    Some(b'Q') => self.parse_qualified_name()?,
                    Some(b't') => self.parse_template()?,
                    Some(byte) if byte.is_ascii_digit() => self.parse_class_name()?,
                    _ => return Err(self.error(ParseErrorKind::Unsupported)),
                };
                let mut qualifiers: Vec<&'static str> = Vec::new();
                loop {
                    match self.peek() {
                        Some(b'C') => qualifiers.push("const"),
                        Some(b'V') => qualifiers.push("volatile"),
                        Some(b'u') => qualifiers.push("__restrict"),
                        _ => break,
                    }
                    self.pos += 1;
                }
                let function_start = self.pos;
                self.expect(b'F')?;
                self.forgetting += 1;
                let parameters = self.parse_args();
                self.forgetting -= 1;
                let parameters = parameters?;
                self.expect(b'_')?;
                let text = qualifier_text(&qualifiers);
                let function = self.make(
                    Kind::FunctionType,
                    text,
                    function_start,
                    alloc::vec![parameters],
                )?;
                self.make(
                    Kind::MemberPointerType,
                    None,
                    start,
                    alloc::vec![class, function],
                )
            }
            Some(b'T') => self.parse_backref(),
            Some(b'C' | b'V' | b'u') if self.qualifier_precedes_builtin() => self.parse_fund_type(),
            Some(b'C' | b'V' | b'u') => {
                let qualifier = match self.peek() {
                    Some(b'C') => "const",
                    Some(b'V') => "volatile",
                    _ => "__restrict",
                };
                self.pos += 1;
                let inner = self.parse_type()?;
                self.make(
                    Kind::QualifiedType,
                    Some(Cow::Borrowed(qualifier)),
                    start,
                    alloc::vec![inner],
                )
            }
            Some(b'K' | b'B' | b'X' | b'Y' | b'G') => Err(self.error(ParseErrorKind::Unsupported)),
            Some(b'Q') => self.parse_qualified_name(),
            Some(b't') => self.parse_template(),
            Some(byte) if byte.is_ascii_digit() => self.parse_class_name(),
            _ => self.parse_fund_type(),
        }
    }

    /// True when the qualifier at the cursor qualifies a builtin type, so it
    /// folds into the builtin spelling (`const unsigned int`) instead of
    /// wrapping the type.
    fn qualifier_precedes_builtin(&self) -> bool {
        let mut index = self.pos;
        while matches!(
            self.at(index),
            Some(b'C' | b'V' | b'u' | b'U' | b'S' | b'J')
        ) {
            index += 1;
        }
        matches!(
            self.at(index),
            Some(b'v' | b'x' | b'l' | b'i' | b's' | b'b' | b'c' | b'w' | b'r' | b'd' | b'f' | b'I')
        )
    }

    /// Parse a builtin type with its qualifiers and sign.
    fn parse_fund_type(&mut self) -> Result<NodeRef<'a>, ParseError> {
        let start = self.pos;
        let mut qualifiers: Vec<&'static str> = Vec::new();
        let mut signed = None;
        let mut complex = false;
        loop {
            match self.peek() {
                Some(b'C') => {
                    self.pos += 1;
                    qualifiers.push("const");
                }
                Some(b'V') => {
                    self.pos += 1;
                    qualifiers.push("volatile");
                }
                Some(b'u') => {
                    self.pos += 1;
                    qualifiers.push("__restrict");
                }
                Some(b'U') => {
                    self.pos += 1;
                    signed = Some("unsigned");
                }
                Some(b'S') => {
                    self.pos += 1;
                    signed = Some("signed");
                }
                Some(b'J') => {
                    self.pos += 1;
                    complex = true;
                }
                _ => break,
            }
        }
        let base = match self.peek() {
            Some(b'v') => "void",
            Some(b'x') => "long long",
            Some(b'l') => "long",
            Some(b'i') => "int",
            Some(b's') => "short",
            Some(b'b') => "bool",
            Some(b'c') => "char",
            Some(b'w') => "wchar_t",
            Some(b'r') => "long double",
            Some(b'd') => "double",
            Some(b'f') => "float",
            Some(b'I') => return self.parse_fixed_width_int(start, &qualifiers),
            _ => return Err(self.error(ParseErrorKind::Unsupported)),
        };
        self.pos += 1;
        let mut text = String::new();
        if let Some(signed) = signed {
            text.push_str(signed);
            text.push(' ');
        }
        if complex {
            text.push_str("__complex ");
        }
        text.push_str(base);
        let builtin = self.make(Kind::BuiltinType, Some(Cow::Owned(text)), start, Vec::new())?;
        let Some(text) = qualifier_text(&qualifiers) else {
            return Ok(builtin);
        };
        self.make(Kind::QualifiedType, Some(text), start, alloc::vec![builtin])
    }

    /// Parse the `I<hex>_` fixed-width integer form.
    fn parse_fixed_width_int(
        &mut self,
        start: usize,
        qualifiers: &[&'static str],
    ) -> Result<NodeRef<'a>, ParseError> {
        self.expect(b'I')?;
        let digits_start = self.pos;
        while self.peek().is_some_and(|byte| byte.is_ascii_hexdigit()) {
            self.pos += 1;
        }
        if self.pos == digits_start {
            return Err(self.error(ParseErrorKind::InvalidLength));
        }
        let digits = &self.symbol[digits_start..self.pos];
        let bits = u32::from_str_radix(digits, 16)
            .map_err(|_| self.error(ParseErrorKind::InvalidLength))?;
        self.expect(b'_')?;
        let text = alloc::format!("int{bits}_t");
        let builtin = self.make(Kind::BuiltinType, Some(Cow::Owned(text)), start, Vec::new())?;
        let Some(text) = qualifier_text(qualifiers) else {
            return Ok(builtin);
        };
        self.make(Kind::QualifiedType, Some(text), start, alloc::vec![builtin])
    }

    /// Parse a `T<n>` back-reference to a remembered type.
    fn parse_backref(&mut self) -> Result<NodeRef<'a>, ParseError> {
        self.expect(b'T')?;
        let index = self.parse_get_count()?;
        self.types
            .get(index)
            .cloned()
            .ok_or_else(|| self.error(ParseErrorKind::UnknownSubstitution))
    }

    // -- parameter lists ---------------------------------------------------

    fn parse_args(&mut self) -> Result<NodeRef<'a>, ParseError> {
        self.enter()?;
        let result = self.parse_args_inner();
        self.leave();
        result
    }

    fn parse_args_inner(&mut self) -> Result<NodeRef<'a>, ParseError> {
        let start = self.pos;
        let mut parameters = Vec::new();
        loop {
            match self.peek() {
                None | Some(b'_') => break,
                Some(b'e') => {
                    self.pos += 1;
                    let ellipsis = self.make(
                        Kind::BuiltinType,
                        Some(Cow::Borrowed("...")),
                        start,
                        Vec::new(),
                    )?;
                    parameters.push(self.make_parameter(ellipsis)?);
                    break;
                }
                Some(b'T') => {
                    let ty = self.parse_backref()?;
                    self.remember(&ty);
                    parameters.push(self.make_parameter(ty)?);
                }
                Some(b'N') => {
                    self.pos += 1;
                    let repeats = self.parse_get_count()?;
                    let index = self.parse_get_count()?;
                    let Some(ty) = self.types.get(index).cloned() else {
                        return Err(self.error(ParseErrorKind::UnknownSubstitution));
                    };
                    for _ in 0..repeats {
                        self.remember(&ty);
                        parameters.push(self.make_parameter(Rc::clone(&ty))?);
                    }
                }
                Some(_) => {
                    let ty = self.parse_type()?;
                    self.remember(&ty);
                    parameters.push(self.make_parameter(ty)?);
                }
            }
        }
        self.make(Kind::ParameterList, None, start, parameters)
    }

    // -- counts ------------------------------------------------------------

    /// Read a decimal count, consuming the whole digit run.
    fn parse_count(&mut self) -> Result<usize, ParseError> {
        let start = self.pos;
        let mut value = 0_usize;
        while let Some(byte) = self.peek() {
            let Some(digit) = byte.checked_sub(b'0').filter(|digit| *digit < 10) else {
                break;
            };
            value = value
                .checked_mul(10)
                .and_then(|value| value.checked_add(usize::from(digit)))
                .ok_or_else(|| self.error(ParseErrorKind::InvalidLength))?;
            self.pos += 1;
        }
        if self.pos == start {
            return Err(self.error(ParseErrorKind::UnexpectedByte));
        }
        Ok(value)
    }

    /// Read a count with the historical single-digit rule: a multi-digit run
    /// counts only when it is followed by `_`.
    fn parse_get_count(&mut self) -> Result<usize, ParseError> {
        let Some(first) = self.peek() else {
            return Err(self.error(ParseErrorKind::UnexpectedEnd));
        };
        let Some(digit) = first.checked_sub(b'0').filter(|digit| *digit < 10) else {
            return Err(self.error(ParseErrorKind::UnexpectedByte));
        };
        self.pos += 1;
        let mut value = usize::from(digit);
        if self.peek().is_some_and(|byte| byte.is_ascii_digit()) {
            let mut scan = self.pos;
            let mut wide = value;
            while let Some(byte) = self.at(scan) {
                let Some(digit) = byte.checked_sub(b'0').filter(|digit| *digit < 10) else {
                    break;
                };
                wide = wide
                    .checked_mul(10)
                    .and_then(|wide| wide.checked_add(usize::from(digit)))
                    .ok_or_else(|| self.error(ParseErrorKind::InvalidLength))?;
                scan += 1;
            }
            if self.at(scan) == Some(b'_') {
                value = wide;
                self.pos = scan + 1;
            }
        }
        Ok(value)
    }
}

/// True for errors that must abort the whole parse instead of trying the next
/// split point.
fn is_fatal(kind: ParseErrorKind) -> bool {
    matches!(
        kind,
        ParseErrorKind::TooDeep
            | ParseErrorKind::TooManyNodes
            | ParseErrorKind::TooMuchWork
            | ParseErrorKind::TooLong
    )
}

/// Collect the `__` split points of `bytes`, left to right.
///
/// A run of `_` bytes contributes its last pair, so `foo___bar` splits at the
/// second and third underscore. The signature must be non-empty.
fn split_positions(bytes: &[u8], base: usize, out: &mut [usize; MAX_SPLITS]) -> usize {
    let mut count = 0;
    let mut index = 0;
    while index + 1 < bytes.len() {
        if bytes[index] != b'_' || bytes[index + 1] != b'_' {
            index += 1;
            continue;
        }
        let run_start = index;
        let mut run_end = index;
        while run_end < bytes.len() && bytes[run_end] == b'_' {
            run_end += 1;
        }
        let split = run_start + (run_end - run_start - 2);
        if split + 2 < bytes.len() && count < MAX_SPLITS {
            out[count] = base + split;
            count += 1;
        }
        index = run_end;
    }
    count
}

/// The operator spelling for a legacy operator code, including the
/// `operator` keyword.
fn operator_spelling(code: &str) -> Option<&'static str> {
    Some(match code {
        "nw" | "new" => "operator new",
        "dl" | "delete" => "operator delete",
        "vn" => "operator new[]",
        "vd" => "operator delete[]",
        "as" => "operator=",
        "ne" => "operator!=",
        "eq" => "operator==",
        "ge" => "operator>=",
        "gt" => "operator>",
        "le" => "operator<=",
        "lt" => "operator<",
        "plus" | "pl" | "convert" => "operator+",
        "apl" => "operator+=",
        "minus" | "mi" | "negate" => "operator-",
        "ami" => "operator-=",
        "mult" | "ml" | "indirect" => "operator*",
        "amu" | "aml" => "operator*=",
        "trunc_mod" | "md" => "operator%",
        "amd" => "operator%=",
        "trunc_div" | "dv" => "operator/",
        "adv" => "operator/=",
        "truth_andif" | "aa" => "operator&&",
        "truth_orif" | "oo" => "operator||",
        "truth_not" | "nt" => "operator!",
        "postincrement" | "pp" => "operator++",
        "postdecrement" | "mm" => "operator--",
        "bit_ior" | "or" => "operator|",
        "aor" => "operator|=",
        "bit_xor" | "er" => "operator^",
        "aer" => "operator^=",
        "bit_and" | "ad" | "addr" => "operator&",
        "aad" => "operator&=",
        "bit_not" | "co" => "operator~",
        "call" | "cl" => "operator()",
        "alshift" | "ls" => "operator<<",
        "als" => "operator<<=",
        "arshift" | "rs" => "operator>>",
        "ars" => "operator>>=",
        "component" | "pt" | "rf" => "operator->",
        "method_call" => "operator->()",
        "array" | "vc" => "operator[]",
        "compound" | "cm" => "operator, ",
        "cond" | "cn" => "operator?:",
        "max" | "mx" => "operator>?",
        "min" | "mn" => "operator<?",
        "nop" => "operator",
        "rm" => "operator->*",
        "sz" => "operator sizeof ",
        _ => return None,
    })
}

/// The operator spelling encoded directly in a legacy name part, if any.
fn operator_spelling_from_name(name: &str) -> Option<&'static str> {
    let rest = name
        .strip_prefix("op$")
        .or_else(|| name.strip_prefix("op."))?;
    if let Some(code) = rest.strip_prefix("assign_") {
        // An assignment operator appends `=`, which no table entry carries.
        return match operator_spelling(code)? {
            "operator+=" | "operator-=" | "operator*=" | "operator%=" | "operator/="
            | "operator|=" | "operator^=" | "operator&=" | "operator<<=" | "operator>>=" => {
                operator_spelling(code)
            }
            "operator+" => Some("operator+="),
            "operator-" => Some("operator-="),
            "operator*" => Some("operator*="),
            "operator%" => Some("operator%="),
            "operator/" => Some("operator/="),
            "operator|" => Some("operator|="),
            "operator^" => Some("operator^="),
            "operator&" => Some("operator&="),
            "operator<<" => Some("operator<<="),
            "operator>>" => Some("operator>>="),
            _ => None,
        };
    }
    operator_spelling(rest)
}

/// The byte offset of the conversion operator's target type inside a name
/// part, if the name part is a conversion operator.
fn conversion_operator_offset(name: &str) -> Option<usize> {
    if name.starts_with("__op") {
        return Some(4);
    }
    let rest = name
        .strip_prefix("type$")
        .or_else(|| name.strip_prefix("type."))?;
    if rest.is_empty() {
        return None;
    }
    Some(5)
}

/// Join cv-qualifier spellings into a `QualifiedType` text.
fn qualifier_text(qualifiers: &[&'static str]) -> Option<Cow<'static, str>> {
    let mut text = String::new();
    for qualifier in qualifiers {
        if !text.is_empty() {
            text.push(' ');
        }
        text.push_str(qualifier);
    }
    (!text.is_empty()).then_some(Cow::Owned(text))
}

/// The encoded function modifiers, as the `Function` node's text.
fn modifier_text(static_function: bool, qualifiers: &[&'static str]) -> Option<Cow<'static, str>> {
    if !static_function && qualifiers.is_empty() {
        return None;
    }
    let mut text = String::new();
    if static_function {
        text.push_str("static");
    }
    for qualifier in qualifiers {
        if !text.is_empty() {
            text.push(' ');
        }
        text.push_str(qualifier);
    }
    Some(Cow::Owned(text))
}

// -- printer ---------------------------------------------------------------

/// Render a parsed GNU v2 symbol into `out`.
pub(crate) fn print<W: Write + ?Sized>(ast: &Ast<'_>, out: &mut W) -> fmt::Result {
    let mut printer = Printer::new();
    printer.symbol(&ast.root())?;
    out.write_str(&printer.out)
}

/// One pass over a legacy [`Ast`], writing text into an owned buffer.
struct Printer {
    out: String,
    depth: u32,
    visits: usize,
}

impl Printer {
    fn new() -> Self {
        Self {
            out: String::new(),
            depth: 0,
            visits: 0,
        }
    }

    fn enter(&mut self) -> fmt::Result {
        self.visits = self.visits.checked_add(1).ok_or(fmt::Error)?;
        if self.visits > MAX_PRINT_VISITS {
            return Err(fmt::Error);
        }
        self.depth = self.depth.checked_add(1).ok_or(fmt::Error)?;
        if self.depth > MAX_PRINT_DEPTH {
            self.depth -= 1;
            return Err(fmt::Error);
        }
        Ok(())
    }

    fn leave(&mut self) {
        self.depth -= 1;
    }

    fn write(&mut self, text: &str) -> fmt::Result {
        if self
            .out
            .len()
            .checked_add(text.len())
            .is_none_or(|len| len > MAX_OUTPUT_BYTES)
        {
            return Err(fmt::Error);
        }
        self.out.push_str(text);
        Ok(())
    }

    fn write_char(&mut self, ch: char) -> fmt::Result {
        if self.out.len() >= MAX_OUTPUT_BYTES {
            return Err(fmt::Error);
        }
        self.out.push(ch);
        Ok(())
    }

    fn last_char(&self) -> Option<char> {
        self.out.chars().next_back()
    }

    fn text<'n>(node: &'n NodeRef<'_>) -> Result<&'n str, fmt::Error> {
        node.text().ok_or(fmt::Error)
    }

    // -- shapes ------------------------------------------------------------

    fn symbol(&mut self, root: &NodeRef<'_>) -> fmt::Result {
        if root.kind() != Kind::Encoding {
            return Err(fmt::Error);
        }
        self.shape(&required(root, 0)?)
    }

    fn shape(&mut self, node: &NodeRef<'_>) -> fmt::Result {
        self.enter()?;
        let result = self.shape_inner(node);
        self.leave();
        result
    }

    fn shape_inner(&mut self, node: &NodeRef<'_>) -> fmt::Result {
        match node.kind() {
            Kind::Encoding => self.shape(&required(node, 0)?),
            Kind::Function => {
                self.name(&required(node, 0)?)?;
                self.write_char('(')?;
                self.parameters(&required(node, 1)?)?;
                self.write_char(')')?;
                if let Some(modifiers) = node.text() {
                    self.write_char(' ')?;
                    self.write(modifiers)?;
                }
                Ok(())
            }
            Kind::Data => self.name(&required(node, 0)?),
            Kind::VTable => {
                self.print_type(&required(node, 0)?)?;
                self.write(" virtual table")
            }
            Kind::TypeInfo => {
                self.print_type(&required(node, 0)?)?;
                if node.text() == Some("function") {
                    self.write(" type_info function")
                } else {
                    self.write(" type_info node")
                }
            }
            Kind::Thunk => {
                let delta = Self::text(node)?;
                self.write("virtual function thunk (delta:")?;
                self.write(delta)?;
                self.write(") for ")?;
                self.shape(&required(node, 0)?)
            }
            _ => Err(fmt::Error),
        }
    }

    fn parameters(&mut self, list: &NodeRef<'_>) -> fmt::Result {
        if list.child_count() == 0 {
            return self.write("void");
        }
        for (index, parameter) in list.children().iter().enumerate() {
            if index > 0 {
                self.write(", ")?;
            }
            let ty = match parameter.kind() {
                Kind::Parameter => required(parameter, 0)?,
                _ => Rc::clone(parameter),
            };
            self.print_type(&ty)?;
        }
        Ok(())
    }

    // -- names -------------------------------------------------------------

    fn name(&mut self, node: &NodeRef<'_>) -> fmt::Result {
        self.enter()?;
        let result = self.name_inner(node);
        self.leave();
        result
    }

    fn name_inner(&mut self, node: &NodeRef<'_>) -> fmt::Result {
        match node.kind() {
            Kind::Identifier => self.write(Self::text(node)?),
            Kind::UnqualifiedName => self.name(&required(node, 0)?),
            Kind::NestedName => self.nested_name(node),
            Kind::Name => {
                self.name(&required(node, 0)?)?;
                self.name(&required(node, 1)?)
            }
            Kind::TemplateArgs => {
                self.write_char('<')?;
                for (index, argument) in node.children().iter().enumerate() {
                    if index > 0 {
                        self.write(", ")?;
                    }
                    match argument.kind() {
                        Kind::Literal => {
                            let value = Self::text(argument)?;
                            self.write(value)?;
                        }
                        _ => self.print_type(argument)?,
                    }
                }
                self.write_char('>')
            }
            Kind::OperatorName => {
                let spelling = Self::text(node)?;
                self.write(spelling)?;
                if node.child_count() == 1 {
                    self.write_char(' ')?;
                    self.print_type(&required(node, 0)?)?;
                }
                Ok(())
            }
            Kind::CtorDtorName => Err(fmt::Error),
            _ => self.print_type(node),
        }
    }

    fn nested_name(&mut self, node: &NodeRef<'_>) -> fmt::Result {
        let prefix = required(node, 0)?;
        let leaf = required(node, 1)?;
        let ctor_dtor = match leaf.kind() {
            Kind::UnqualifiedName => match leaf.child(0) {
                Some(inner) if inner.kind() == Kind::CtorDtorName => Some(inner),
                _ => None,
            },
            _ => None,
        };
        match ctor_dtor {
            Some(inner) => {
                self.name(&prefix)?;
                self.write("::")?;
                if Self::text(&inner)?.starts_with('D') {
                    self.write_char('~')?;
                }
                self.name(&leaf_name(&prefix)?)
            }
            None => {
                self.name(&prefix)?;
                self.write("::")?;
                self.name(&leaf)
            }
        }
    }

    // -- types -------------------------------------------------------------

    fn print_type(&mut self, node: &NodeRef<'_>) -> fmt::Result {
        self.type_left(node)?;
        if has_right(node) {
            self.type_right(node)?;
        }
        Ok(())
    }

    fn type_left(&mut self, node: &NodeRef<'_>) -> fmt::Result {
        self.enter()?;
        let result = self.type_left_inner(node);
        self.leave();
        result
    }

    fn type_left_inner(&mut self, node: &NodeRef<'_>) -> fmt::Result {
        match node.kind() {
            Kind::BuiltinType => self.write(Self::text(node)?),
            Kind::QualifiedType => {
                let inner = required(node, 0)?;
                self.type_left(&inner)?;
                if !matches!(self.last_char(), Some('*' | '&')) {
                    self.write_char(' ')?;
                }
                self.write(Self::text(node)?)
            }
            Kind::PointerType | Kind::ReferenceType | Kind::RvalueReferenceType => {
                let inner = required(node, 0)?;
                self.type_left(&inner)?;
                if parenthesised(&inner) {
                    if self.last_char() != Some(' ') {
                        self.write_char(' ')?;
                    }
                    self.write_char('(')?;
                } else if matches!(self.last_char(), Some(ch) if ch.is_alphanumeric() || ch == '>')
                {
                    self.write_char(' ')?;
                }
                let declarator = match node.kind() {
                    Kind::PointerType => "*",
                    Kind::ReferenceType => "&",
                    _ => "&&",
                };
                self.write(declarator)
            }
            Kind::ArrayType => self.type_left(&required(node, 0)?),
            Kind::FunctionType => {
                // Legacy function types encode no return type.
                self.write("void ")
            }
            Kind::MemberPointerType => {
                let class = required(node, 0)?;
                let member = required(node, 1)?;
                self.type_left(&member)?;
                self.write_char('(')?;
                self.name(&class)?;
                self.write("::*")
            }
            Kind::Name | Kind::NestedName | Kind::UnqualifiedName | Kind::Identifier => {
                self.name(node)
            }
            _ => Err(fmt::Error),
        }
    }

    fn type_right(&mut self, node: &NodeRef<'_>) -> fmt::Result {
        self.enter()?;
        let result = self.type_right_inner(node);
        self.leave();
        result
    }

    fn type_right_inner(&mut self, node: &NodeRef<'_>) -> fmt::Result {
        match node.kind() {
            Kind::QualifiedType => self.type_right(&required(node, 0)?),
            Kind::PointerType | Kind::ReferenceType | Kind::RvalueReferenceType => {
                let inner = required(node, 0)?;
                if parenthesised(&inner) {
                    self.write_char(')')?;
                }
                self.type_right(&inner)
            }
            Kind::ArrayType => {
                let inner = required(node, 0)?;
                if !matches!(self.last_char(), Some(')' | ']')) {
                    self.write_char(' ')?;
                }
                self.write_char('[')?;
                if let Some(dimension) = node.text() {
                    self.write(dimension)?;
                }
                self.write_char(']')?;
                self.type_right(&inner)
            }
            Kind::FunctionType => {
                self.write_char('(')?;
                self.parameters(&required(node, 0)?)?;
                self.write_char(')')?;
                if let Some(qualifiers) = node.text() {
                    self.write_char(' ')?;
                    self.write(qualifiers)?;
                }
                Ok(())
            }
            Kind::MemberPointerType => {
                let member = required(node, 1)?;
                self.write_char(')')?;
                self.type_right(&member)
            }
            _ => Ok(()),
        }
    }
}

/// The child at `index`, or a print error when the node contract is broken.
fn required<'a>(node: &NodeRef<'a>, index: usize) -> Result<NodeRef<'a>, fmt::Error> {
    node.child(index).ok_or(fmt::Error)
}

/// True when a type needs declarator parentheses before a `*` or `&`.
fn parenthesised(node: &NodeRef<'_>) -> bool {
    matches!(
        node.kind(),
        Kind::ArrayType | Kind::FunctionType | Kind::MemberPointerType
    )
}

/// True when a type writes anything after its declarator name.
fn has_right(node: &NodeRef<'_>) -> bool {
    match node.kind() {
        Kind::ArrayType | Kind::FunctionType | Kind::MemberPointerType => true,
        Kind::QualifiedType
        | Kind::PointerType
        | Kind::ReferenceType
        | Kind::RvalueReferenceType => node.child(0).is_some_and(|inner| has_right(&inner)),
        _ => false,
    }
}

/// The innermost name component, used as a constructor or destructor name.
fn leaf_name<'a>(node: &NodeRef<'a>) -> Result<NodeRef<'a>, fmt::Error> {
    match node.kind() {
        Kind::Identifier | Kind::BuiltinType | Kind::OperatorName => Ok(Rc::clone(node)),
        Kind::UnqualifiedName | Kind::NestedName => {
            leaf_name(&node.child(node.child_count() - 1).ok_or(fmt::Error)?)
        }
        Kind::Name => leaf_name(&node.child(0).ok_or(fmt::Error)?),
        _ => Err(fmt::Error),
    }
}

#[cfg(test)]
mod tests {
    use alloc::string::String;

    use super::*;
    use crate::cpp::{self, Dialect};

    fn rendered(symbol: &str) -> String {
        let ast = parse(symbol).unwrap_or_else(|error| panic!("{symbol}: {error}"));
        alloc::format!("{ast}")
    }

    #[test]
    fn recognises_legacy_names() {
        for symbol in [
            "foo__Fiv",
            "bar__3foo",
            "__3foo",
            "_$_3foo",
            "_3foo$bar",
            "_vt$foo",
            "__ti3foo",
            "__thunk_4__$_7ostream",
            "a__b__c",
        ] {
            assert!(is_legacy_name(symbol), "{symbol} should be legacy C++");
        }
        for symbol in [
            "main",
            "foo",
            "foo__bar",
            "foo__",
            "__libc_start_main",
            "_ZN3Foo3barEv",
            "?x@@3HA",
            "foo_bar",
            "$s4main3foo",
        ] {
            assert!(!is_legacy_name(symbol), "{symbol} should not be legacy C++");
        }
    }

    #[test]
    fn parses_function_suffixes() {
        assert_eq!(rendered("foo__Fiv"), "foo(int, void)");
        assert_eq!(rendered("foo__Fv"), "foo(void)");
        assert_eq!(rendered("foo__F"), "foo(void)");
        assert_eq!(rendered("foo__Fi"), "foo(int)");
        assert_eq!(rendered("foo__Fii"), "foo(int, int)");
        assert_eq!(rendered("foo__Fve"), "foo(void, ...)");
        assert_eq!(rendered("foo_bar__Fv"), "foo_bar(void)");
        assert_eq!(rendered("foo__Fi3bar3baz"), "foo(int, bar, baz)");
    }

    #[test]
    fn parses_types_and_declarators() {
        for (symbol, expected) in [
            ("foo__FPi", "foo(int *)"),
            ("foo__FRi", "foo(int &)"),
            ("foo__FOi", "foo(int &&)"),
            ("foo__FPPc", "foo(char **)"),
            ("foo__FCi", "foo(int const)"),
            ("foo__FPCc", "foo(char const *)"),
            ("foo__FPCi", "foo(int const *)"),
            ("foo__FUi", "foo(unsigned int)"),
            ("foo__FSs", "foo(signed short)"),
            ("foo__FJc", "foo(__complex char)"),
            ("foo__FI2c_", "foo(int44_t)"),
            ("foo__FA5_i", "foo(int [5])"),
            ("foo__FA3_A4_c", "foo(char [3][4])"),
            ("foo__FPA3_c", "foo(char (*)[3])"),
            ("foo__FPFv", "foo(void (*)(void))"),
            ("foo__FPPFiv", "foo(void (**)(int, void))"),
            ("foo__FM3fooFv_", "foo(void (foo::*)(void))"),
            ("foo__FM3fooCFv_", "foo(void (foo::*)(void) const)"),
        ] {
            assert_eq!(rendered(symbol), expected, "{symbol}");
        }
    }

    #[test]
    fn parses_nested_and_qualified_names() {
        assert_eq!(rendered("bar__3foo"), "foo::bar(void)");
        assert_eq!(rendered("foo__3barFv"), "bar::foo(void)");
        assert_eq!(rendered("foo__C3barFv"), "bar::foo(void) const");
        assert_eq!(rendered("foo__S3barFv"), "bar::foo(void) static");
        assert_eq!(rendered("foo__Q23foo3barFv"), "foo::bar::foo(void)");
        assert_eq!(rendered("bar__Q23foo3bar"), "foo::bar::bar(void)");
        assert_eq!(rendered("foo__FiQ23foo3bar"), "foo(int, foo::bar)");
    }

    #[test]
    fn parses_constructors_destructors_and_operators() {
        assert_eq!(rendered("__3foo"), "foo::foo(void)");
        assert_eq!(rendered("__3fooFv"), "foo::foo(void)");
        assert_eq!(rendered("_$_3foo"), "foo::~foo(void)");
        assert_eq!(rendered("__Q23foo3bar"), "foo::bar::bar(void)");
        assert_eq!(rendered("_$_Q23foo3bar"), "foo::bar::~bar(void)");
        assert_eq!(rendered("op$pl__Fv"), "operator+(void)");
        assert_eq!(rendered("op$assign_pl__Fv"), "operator+=(void)");
        assert_eq!(rendered("op$eq__Fv"), "operator==(void)");
        assert_eq!(rendered("__opPc__Fv"), "operator char *(void)");
        assert_eq!(rendered("type$3foo__Fv"), "operator foo(void)");
    }

    #[test]
    fn parses_templates_and_back_references() {
        assert_eq!(rendered("foo__t6vector1Zii"), "vector<int>::foo(int)");
        assert_eq!(
            rendered("foo__t6vector2ZiZc"),
            "vector<int, char>::foo(void)"
        );
        assert_eq!(rendered("__t6vector1Zii"), "vector<int>::vector(int)");
        assert_eq!(rendered("foo__FiT0Pc"), "foo(int, int, char *)");
        assert_eq!(
            rendered("bar__3fooFPiN51"),
            "foo::bar(int *, int *, int *, int *, int *, int *)"
        );
    }

    #[test]
    fn parses_special_names() {
        assert_eq!(rendered("_3foo$bar"), "foo::bar");
        assert_eq!(rendered("_Q23foo3bar$baz"), "foo::bar::baz");
        assert_eq!(rendered("_vt$foo"), "foo virtual table");
        assert_eq!(rendered("_vt$foo$bar"), "foo::bar virtual table");
        assert_eq!(rendered("__vt_3foo"), "foo virtual table");
        assert_eq!(rendered("__ti3foo"), "foo type_info node");
        assert_eq!(
            rendered("__tft6vector1Zi"),
            "vector<int> type_info function"
        );
        assert_eq!(
            rendered("__thunk_4__$_7ostream"),
            "virtual function thunk (delta:-4) for ostream::~ostream(void)"
        );
    }

    #[test]
    fn reports_the_dialect_and_shape() {
        let ast = cpp::parse("foo__Fiv").expect("legacy name should parse");
        assert_eq!(ast.dialect(), Dialect::GnuV2);
        assert_eq!(ast.root().kind(), Kind::Encoding);
        let function = ast.root().child(0).expect("encoding shape");
        assert_eq!(function.kind(), Kind::Function);
        let parameters = function.child(1).expect("parameter list");
        assert_eq!(parameters.kind(), Kind::ParameterList);
        assert_eq!(parameters.child_count(), 2);
        assert_eq!(function.span(), Some(0..8));
        let name = function.child(0).expect("function name");
        let identifier = name.child(0).expect("identifier");
        assert_eq!(identifier.span(), Some(0..3));
        assert_eq!(identifier.text(), Some("foo"));
    }

    #[test]
    fn rejects_malformed_input() {
        for symbol in [
            "foo__",
            "__",
            "foo__Fzzz",
            "foo__FiA_",
            "foo__t6vector1Zi10",
            "foo__Q23foo",
            "_3foo$",
            "__ti",
            "__thunk_",
            "a__b__c__",
            "foo__FiT9",
            "foo__FiN31",
        ] {
            assert!(parse(symbol).is_err(), "{symbol} should be rejected");
        }
    }

    #[test]
    fn rejects_unsupported_productions() {
        for symbol in ["foo__H3bar", "foo__K3bar", "foo__B0", "_GLOBAL_$I$foo"] {
            let error = parse(symbol).unwrap_err();
            assert!(
                matches!(
                    error.kind(),
                    ParseErrorKind::Unsupported
                        | ParseErrorKind::UnexpectedByte
                        | ParseErrorKind::NotLegacy
                ),
                "{symbol}: {error:?}"
            );
        }
    }

    #[test]
    fn cpp_parse_reports_non_legacy_names() {
        let error = cpp::parse("main").unwrap_err();
        assert_eq!(error.kind(), ParseErrorKind::NotItanium);
        let error = cpp::parse("foo__Fzzz").unwrap_err();
        assert_ne!(error.kind(), ParseErrorKind::NotItanium);
    }

    #[test]
    fn truncations_never_panic() {
        for symbol in [
            "foo__Fiv",
            "bar__3fooFPiN51e",
            "__thunk_4__$_7ostream",
            "_vt$foo$bar",
            "foo__t6vector2ZiZc",
            "foo__FiM3fooCFv_",
        ] {
            for end in 0..symbol.len() {
                let _ = parse(&symbol[..end]);
            }
            for offset in 0..symbol.len() {
                let mut bytes = String::from(symbol);
                bytes.insert(offset, '_');
                let _ = parse(&bytes);
            }
        }
    }

    #[test]
    fn input_budget_is_enforced() {
        let mut symbol = String::from("foo__F");
        symbol.push_str(&"i".repeat(MAX_INPUT_LEN));
        let error = parse(&symbol).unwrap_err();
        assert_eq!(error.kind(), ParseErrorKind::TooLong);
    }

    #[test]
    fn depth_budget_is_enforced() {
        let mut symbol = String::from("foo__F");
        symbol.push_str(&"P".repeat(2 * MAX_DEPTH as usize));
        symbol.push('i');
        let error = parse(&symbol).unwrap_err();
        assert_eq!(error.kind(), ParseErrorKind::TooDeep);
    }

    #[test]
    fn node_budget_is_enforced() {
        // Every `__` is a candidate split point, and each attempt allocates
        // nodes before it fails, so the node budget bounds the whole parse.
        let mut symbol = String::new();
        for _ in 0..40 {
            symbol.push_str(&"i".repeat(100));
            symbol.push_str("__");
        }
        symbol.push_str(&"i".repeat(100));
        let error = parse(&symbol).unwrap_err();
        assert_eq!(error.kind(), ParseErrorKind::TooManyNodes);
    }

    #[test]
    fn long_valid_names_print() {
        let mut symbol = String::from("foo__F");
        for _ in 0..200 {
            symbol.push_str("Pi");
        }
        let text = rendered(&symbol);
        assert!(text.starts_with("foo(int *, int *"));
        assert_eq!(text.matches("int *").count(), 200);
        assert!(text.len() > 1_000);
    }
}
