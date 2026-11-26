//! Legacy C++ demangling: GNU v2 (old GNU, cfront-era), ARM C++, EDG C++ and
//! HP aCC C++.
//!
//! GNU v2 is the mangling GCC used before the Itanium C++ ABI. A legacy name
//! carries no version prefix: the name is spelled out and the signature
//! follows a `__` separator, as in `foo__Fiv` (`foo(int, void)`) or
//! `bar__3foo` (`foo::bar(void)`). The separator is not unique — `a__b__c`
//! demangles as `a__b(char)`, not `b::a(...)` — so the parser tries every
//! `__` split point and accepts the first one that consumes the whole symbol.
//! A prefix match alone never selects the dialect.
//!
//! ARM C++ and EDG C++ are cfront-derived dialects with the same declarator
//! grammar. They differ from GNU v2 in four documented ways:
//!
//! - special names are spelled `__ct__<class>` (constructor),
//!   `__dt__<class>` (destructor), `__vtbl__<class>[__<class>...]` (virtual
//!   table), `__sti__<name>`/`__std__<name>` (static initialiser and
//!   destructor wrappers) and `<name>__pt__<count>_<args>` (ARM parameterized
//!   type; EDG also spells it `__tm__` and `__ps__`);
//! - `F` marks the argument list. A template component implies one for both
//!   dialects and EDG also implies one after a length-prefixed class name;
//!   without an argument list and without an implied one the symbol names a
//!   data object, so `bar__3foo` is `foo::bar` for both dialects while GNU v2
//!   reads it as `foo::bar(void)`;
//! - argument back-references (`T<n>`, `N<count><n>`) are numbered from one
//!   and only see types encoded after the `F` marker; back-references in type
//!   position stay zero-based as in GNU v2, which is what the reference
//!   demangler does;
//! - EDG may append the return type after the argument list (`foo__Fi_v`);
//!   the legacy printers drop it, as the reference demangler does.
//!
//! Those spellings are dialect markers: user code cannot legitimately define
//! an identifier such as `__ct`, so a symbol carrying one is parsed as ARM or
//! EDG first and falls back to GNU v2 when that parse is not complete. Every
//! other name keeps GNU v2 precedence, and a name is never claimed by a
//! dialect whose parser does not consume the whole symbol.
//!
//! HP aCC C++ is the third cfront-derived dialect. It shares the ARM
//! declarator grammar and the ARM special names (`__ct__`, `__dt__`,
//! `__vtbl__`, `__sti__`, `__std__`) and differs in three ways:
//!
//! - a template spec follows a name as `X<argument>*`, as in `foo__XSP5_Fv`
//!   (`foo<5>(void)`). An argument is `T<type>`, an `S`/`U` integral value
//!   (`S`/`U`, then `P`, `N` or `M`, then digits; `U` prints a trailing `U`
//!   and `M` is `-2147483648`) or an `A<count><name>` named literal
//!   (`&<name>`). The spec may follow any length-prefixed class name, so
//!   `foo__F3barXSP6` is `foo(bar<6>)`;
//! - an alternate entry point suffix `_<digits>` after a name or signature
//!   is dropped, as the reference demangler drops it (`foo__Fi_1` is
//!   `foo(int)`, `bar__3foo_1` is `foo::bar`);
//! - the parameterized-type form is not HP: the reference demangler rejects
//!   `__pt__` in HP mode, so it is rejected here too.
//!
//! HP is tried after the other legacy dialects, because every HP name that the
//! other dialects read the same way keeps their precedence; only a name whose
//! other readings fail is claimed as HP. A name is never claimed from a
//! prefix.
//!
//! # Supported grammar
//!
//! - `__`-separated function names, the empty-name constructor form
//!   (`__3foo`) and the `_$_` destructor form (GNU v2 only);
//! - unqualified names, `Q<n>`-qualified names, template-ids and the encoded
//!   `static`/`const`/`volatile`/`__restrict` function qualifiers;
//! - operator names (`__pl`, `op$pl`, `op$assign_pl`), method operators
//!   (`__as__<class>`, `__apl__<class>`) and conversion operators
//!   (`__op<type>`, `type$<type>`);
//! - `F`-suffixed parameter lists, back-references (`T<n>`, `N<count><n>`)
//!   and the trailing `e` ellipsis;
//! - builtin types with `const`/`volatile`/`__restrict`, `unsigned`,
//!   `signed` and `__complex`; pointers, references, rvalue references,
//!   arrays, function types and member-function pointers, including the
//!   optional `_<return type>` suffix of a function type;
//! - template argument lists with type arguments (`Z<type>`) and integral
//!   value arguments, including the `true`/`false` spelling of a `bool`;
//! - the `G` class marker;
//! - the `__vt_`/`_vt$` virtual table, `__ti`/`__tf` type information and
//!   `__thunk_` thunk special names, `_GLOBAL_$I$`/`_GLOBAL_$D$` (or
//!   `_GLOBAL_.I.`/`_GLOBAL_.D.`) constructor and destructor wrappers, and
//!   `_<name><marker><variable>` static data members (GNU v2 only);
//! - the ARM/EDG special names listed above;
//! - the HP aCC `X` template spec, its `S`/`U`/`A` arguments and the
//!   alternate entry point suffix listed above.
//!
//! Unsupported productions are rejected rather than approximated: squangling
//! (`K`, `B`, `n`), member templates (`H`), template template parameters,
//! `X`/`Y` template parameter references, the HP local-class `L` type and the
//! remaining HP cfront template extensions, EDG's recursive qualified-name
//! components, and the ARM/EDG local-variable and thunk forms.
//!
//! The vendor dialects outside the cfront family — Sun/Oracle Studio, IBM XL,
//! SGI MIPSpro, Metrowerks CodeWarrior, Watcom, Borland/Embarcadero and Digital
//! Mars — are not parsed at all. No permissively licensed corpus or trustworthy
//! oracle for them is available, and their spellings collide with schemes this
//! crate already claims (GNU v2, MSVC, PE/COFF and Delphi), so a guessed reading
//! would risk wrong output; `refs/cpp.md` records the evidence. Lucid needs no
//! dialect of its own: it is a subset of the ARM grammar.
//!
//! # Spelling
//!
//! The text follows the legacy `c++filt` conventions: an empty parameter list
//! prints as `void`, cv-qualifiers follow the type they qualify
//! (`int const *`), virtual tables print as `<type> virtual table`, type
//! information as `<type> type_info node`, thunks as
//! `virtual function thunk (delta:<n>) for <symbol>`, a `bool` template
//! value as `true`/`false`, and the static wrappers as
//! `global constructors keyed to <symbol>` and
//! `global destructors keyed to <symbol>`. Two historical spellings are
//! defects rather than conventions and are not reproduced: an unnamed
//! function-pointer parameter prints as `void (*)(void)` instead of
//! ` (*)(void)`, and a member-function pointer prints as
//! `void (foo::*)(void)` instead of ` (foo::)(void)`. Three further
//! normalisations apply: the `e` ellipsis is separated by `, `, the ARM/EDG
//! parameterized-type arguments are separated by `, ` instead of `,`, and a
//! constructor of a qualified class names the last component
//! (`bar::baz::baz`), where the EDG reference prints an empty name. Array
//! `new`/`delete` print as `operator new[]`/`operator delete[]` without the
//! historical space.
//!
//! The HP aCC template spec follows the same conventions: arguments are
//! separated by `, `, the base name is decoded where the reference appends the
//! raw encoding (`3baz__XSP5` is `baz<5>`, not `3baz<5>`), and no trailing
//! comma is printed where the reference emits one for a type argument that
//! carries its own template spec (`foo<bar<6>>`, not `foo<bar<6>,>`). An `A`
//! literal argument prints `&` followed by its payload demangled as a legacy
//! symbol, or by the payload verbatim when it is not one (`&bar`).
//!
//! # Budgets
//!
//! Parsing is bounded by the shared C++ input limit and by explicit depth,
//! node and work budgets. A symbol that needs several legacy candidates is
//! parsed once per candidate, so the work budget is charged per candidate and
//! the symbol is rejected when any candidate exhausts it. Printing is bounded
//! by depth, visit and output-byte budgets. A symbol that exceeds a budget is
//! rejected, never truncated.

use alloc::borrow::Cow;
use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;
use core::fmt::{self, Write};

use super::node::{Kind, Node, NodeRef};
use super::print_buffer::PrintBuffer;
use super::{
    Ast, DEFAULT_BUDGET, Dialect, MIN_WORK, ParseError, ParseErrorKind, ParseLimits, WORK_PER_BYTE,
    decode,
};
use crate::budget::EffectiveBudget;

/// Maximum nesting depth for recursive productions.
///
/// This is the compiled-in depth budget; a [`crate::Budget`] may lower it but
/// not raise it, because the fixed print envelope bounds the nesting a
/// retained tree may have.
pub(crate) const MAX_DEPTH: u32 = 64;
/// Maximum nodes one legacy parse may allocate.
///
/// This is the compiled-in node budget; a [`crate::Budget`] may lower or raise
/// it up to the scheme's hard ceiling.
pub(crate) const MAX_NODES: u32 = 1 << 12;

/// The `CPLUS_MARKER` characters GNU v2 accepts inside special names.
const MARKERS: [u8; 2] = *b"$.";

/// Special-name prefixes that identify an ARM, EDG or HP aCC symbol.
///
/// User code cannot define these names: an identifier starting with two
/// underscores is reserved to the implementation, and each of these prefixes is
/// the dialect's spelling of a compiler-generated entity.
const CFRONT_PREFIXES: [&str; 5] = ["__ct__", "__dt__", "__vtbl__", "__sti__", "__std__"];

/// Parameterized-type markers: ARM spells the form `__pt__`, EDG also accepts
/// `__tm__` and `__ps__`.
const CFRONT_TEMPLATE_MARKERS: [&str; 3] = ["__pt__", "__tm__", "__ps__"];

/// True when a byte string is a complete legacy C++ mangled name.
///
/// This predicate is the test-facing form of the same bounded parse.
#[cfg(test)]
pub(crate) fn is_legacy_name_bytes(symbol: &[u8]) -> bool {
    parse_if_legacy(symbol).is_some()
}

/// Parse `symbol` as a complete legacy C++ name and return its AST.
///
/// Detection uses the AST's dialect to report the precise scheme and keeps the
/// AST for dispatch, avoiding a second parse.
pub(crate) fn parse_if_legacy(symbol: &[u8]) -> Option<Ast<'_>> {
    parse_if_legacy_with(symbol, DEFAULT_BUDGET)
}

/// Parse a legacy C++ name with one resolved budget.
///
/// Detection uses this so its legacy prefilter sees the configured limits; the
/// default-budget wrapper keeps [`parse_if_legacy`] unchanged.
pub(crate) fn parse_if_legacy_with(symbol: &[u8], budget: EffectiveBudget) -> Option<Ast<'_>> {
    if !looks_like_legacy_bytes(symbol) {
        return None;
    }
    parse_bytes_with_budget(symbol, budget).ok()
}

/// Cheap byte pre-filter for a legacy C++ name.
///
/// The shape test only: the input cap is enforced by the parse itself, so a
/// raised input budget reaches a symbol longer than the default cap.
pub(crate) fn looks_like_legacy_bytes(symbol: &[u8]) -> bool {
    if symbol.len() < 3 {
        return false;
    }
    if symbol.starts_with(b"_Z") || symbol.starts_with(b"?") {
        return false;
    }
    contains_bytes(symbol, b"__")
        || (symbol.starts_with(b"_") && symbol.iter().any(|byte| matches!(byte, b'$' | b'.')))
}

/// True when `haystack` contains `needle` as a byte substring.
fn contains_bytes(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

/// Parse a legacy C++ mangled name with one resolved budget.
///
/// [`parse_bytes`] wraps this with [`DEFAULT_BUDGET`]; a [`crate::Demangler`]
/// passes the budget it resolved for the legacy scheme. Every dimension is
/// clamped to the scheme's hard ceiling, and the four dialects share one
/// resolution because they share one default and one ceiling.
///
/// The dialect is selected by full parse: GNU v2 first, then ARM, EDG and HP
/// aCC, except that a symbol carrying a cfront special-name marker is tried as
/// ARM, EDG and HP first and falls back to GNU v2. The first dialect whose
/// parser consumes the whole symbol wins; a partial parse never selects a
/// dialect.
pub(crate) fn parse_bytes_with_budget(
    symbol: &[u8],
    budget: EffectiveBudget,
) -> Result<Ast<'_>, ParseError> {
    let limits = ParseLimits::legacy(budget);
    if symbol.len() > limits.input() {
        return Err(ParseError::new(0, ParseErrorKind::TooLong));
    }
    let candidates = if has_cfront_marker(symbol) {
        [Dialect::Arm, Dialect::Edg, Dialect::Hp, Dialect::GnuV2]
    } else {
        [Dialect::GnuV2, Dialect::Arm, Dialect::Edg, Dialect::Hp]
    };
    let mut first_error = None;
    for dialect in candidates {
        match parse_with(symbol, dialect, limits) {
            Ok(ast) => return Ok(ast),
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
    Err(first_error.unwrap_or_else(|| ParseError::new(0, ParseErrorKind::NotLegacy)))
}

/// True when `symbol` carries a cfront special-name marker.
fn has_cfront_marker(symbol: &[u8]) -> bool {
    CFRONT_PREFIXES
        .iter()
        .any(|marker| symbol.starts_with(marker.as_bytes()))
        || CFRONT_TEMPLATE_MARKERS
            .iter()
            .any(|marker| contains_bytes(symbol, marker.as_bytes()))
}

/// True when `symbol` carries an EDG-only special-name marker.
fn has_edg_marker(symbol: &[u8]) -> bool {
    CFRONT_TEMPLATE_MARKERS[1..]
        .iter()
        .any(|marker| contains_bytes(symbol, marker.as_bytes()))
}

/// Parse `symbol` with one legacy dialect.
fn parse_with(symbol: &[u8], dialect: Dialect, limits: ParseLimits) -> Result<Ast<'_>, ParseError> {
    Parser::new(symbol, dialect, limits).parse_ast()
}

/// Recursive-descent parser over one legacy symbol.
struct Parser<'a> {
    bytes: &'a [u8],
    dialect: Dialect,
    pos: usize,
    limits: ParseLimits,
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
    /// Expansion visits charged by nested HP aCC literal renders.
    expanded_visits: usize,
    /// Rendered bytes charged by nested HP aCC literal renders.
    expanded_bytes: usize,
}

impl<'a> Parser<'a> {
    fn new(bytes: &'a [u8], dialect: Dialect, limits: ParseLimits) -> Self {
        // The allowance is derived from the symbol length and capped by the
        // resolved work budget, so raising the input budget alone does not
        // raise the work allowance.
        let work_limit = bytes
            .len()
            .saturating_mul(WORK_PER_BYTE)
            .saturating_add(MIN_WORK)
            .min(limits.work());
        Self {
            bytes,
            dialect,
            pos: 0,
            limits,
            depth: 0,
            nodes: 0,
            work: 0,
            work_limit,
            types: Vec::new(),
            forgetting: 0,
            expanded_visits: 0,
            expanded_bytes: 0,
        }
    }

    /// True for the cfront-derived dialects (ARM, EDG and HP aCC).
    const fn is_cfront(&self) -> bool {
        matches!(self.dialect, Dialect::Arm | Dialect::Edg | Dialect::Hp)
    }

    /// True for the HP aCC dialect.
    const fn is_hp(&self) -> bool {
        matches!(self.dialect, Dialect::Hp)
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
        if self.depth >= self.limits.depth() {
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
        if self.nodes > self.limits.nodes() {
            return Err(self.error(ParseErrorKind::TooManyNodes));
        }
        let end = self.pos;
        let span = if start <= end && end <= self.bytes.len() {
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
        let root = self.parse_root()?;
        Ok(Ast::from_root_legacy(root, self.dialect))
    }

    /// Parse the whole input into its root [`Kind::Encoding`] node.
    fn parse_root(&mut self) -> Result<NodeRef<'a>, ParseError> {
        let shape = self.parse_symbol(0)?;
        if self.pos != self.bytes.len() {
            return Err(self.error(ParseErrorKind::UnexpectedByte));
        }
        self.make(Kind::Encoding, None, 0, alloc::vec![shape])
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
        if self.is_cfront() {
            if self.dialect == Dialect::Arm && has_edg_marker(&self.bytes[start..]) {
                // `__tm__`/`__ps__` are the EDG spellings of the
                // parameterized-type form; ARM declines the name so that the
                // EDG candidate can read it.
                return Err(self.error(ParseErrorKind::Unsupported));
            }
            if let Some(shape) = self.parse_cfront_special(start)? {
                return Ok(shape);
            }
            return self.parse_split(start);
        }
        if let Some(shape) = self.parse_special(start)? {
            return Ok(shape);
        }
        // The constructor form spells no name: `__<class>` is the constructor
        // of `<class>`, and `__H...` is the unsupported member-template form.
        // ARM and EDG spell constructors `__ct__<class>`, so this form is GNU
        // v2 only.
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
    ///
    /// ARM and EDG encode a unique separator, so their front ends read only
    /// the first split point, as the reference demangler does. GNU v2 has no
    /// such guarantee (`a__b__c`), so it tries them all. Every attempt charges
    /// the one work counter of this parse, so the work budget bounds the
    /// total, however many split points the symbol has.
    fn parse_split(&mut self, start: usize) -> Result<NodeRef<'a>, ParseError> {
        let splits = split_positions(&self.bytes[start..], start);
        let mut count = splits.len();
        if count == 0 {
            return Err(ParseError::new(start, ParseErrorKind::NotLegacy));
        }
        if self.is_cfront() {
            count = 1;
        }
        let mut first_error = None;
        for &split in &splits[..count] {
            self.pos = start;
            self.types.clear();
            self.forgetting = 0;
            let attempt = (|| {
                let mut name = self.parse_name_part(start, split)?;
                self.pos = split + 2;
                if self.is_hp() && self.peek() == Some(b'X') {
                    name = Some(self.parse_hp_template_name(start, split)?);
                }
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
        let global_marker = self.at(start + 8);
        let global_kind = self.at(start + 9);
        if self.starts_at(start, b"_GLOBAL_")
            && matches!(global_marker, Some(b'$' | b'.'))
            && matches!(global_kind, Some(b'I' | b'D'))
            && self.at(start + 10) == global_marker
        {
            // Linker bookkeeping for file-scope constructors and destructors.
            // The payload is a complete legacy symbol, so it is parsed as one.
            let text = if global_kind == Some(b'I') {
                Cow::Borrowed("constructors")
            } else {
                Cow::Borrowed("destructors")
            };
            self.pos = start + 11;
            let shape = self.parse_symbol(self.pos)?;
            if self.pos != self.bytes.len() {
                return Err(self.error(ParseErrorKind::UnexpectedByte));
            }
            return Ok(Some(self.make(
                Kind::LegacySpecial,
                Some(text),
                start,
                alloc::vec![shape],
            )?));
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
        let variable = &self.bytes[variable_start..];
        if variable.is_empty() {
            return Err(self.error(ParseErrorKind::UnexpectedEnd));
        }
        self.pos = self.bytes.len();
        let leaf = self.make(
            Kind::Identifier,
            Some(decode(variable)),
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
        let text = &self.bytes[start..self.pos];
        let ident = self.make(Kind::Identifier, Some(decode(text)), start, Vec::new())?;
        self.make(Kind::UnqualifiedName, None, start, alloc::vec![ident])
    }

    // -- ARM/EDG special names ---------------------------------------------

    /// Parse an ARM or EDG special name, or return `None` when `start` does
    /// not begin one.
    fn parse_cfront_special(&mut self, start: usize) -> Result<Option<NodeRef<'a>>, ParseError> {
        if self.starts_at(start, b"__ct__") || self.starts_at(start, b"__dt__") {
            let destructor = self.at(start + 2) == Some(b'd');
            self.pos = start + 6;
            return Ok(Some(self.parse_entity(start, None, true, destructor)?));
        }
        if self.starts_at(start, b"__vtbl__") {
            return Ok(Some(self.parse_cfront_vtable(start)?));
        }
        if let Some(wrapper) = self.cfront_wrapper(start) {
            return Ok(Some(self.parse_cfront_wrapper(start, wrapper)?));
        }
        self.parse_parameterized_type(start)
    }

    /// The wrapper name for a `__sti__`/`__std__` static initialiser or
    /// destructor, if `start` begins one.
    fn cfront_wrapper(&self, start: usize) -> Option<&'static str> {
        if self.starts_at(start, b"__sti__") {
            Some("constructors")
        } else if self.starts_at(start, b"__std__") {
            Some("destructors")
        } else {
            None
        }
    }

    /// `__vtbl__<class>[__<class>...]`: a virtual table.
    ///
    /// Components are length-prefixed class names in innermost-first order, so
    /// `__vtbl__3foo__3bar` is the table of `bar::foo`.
    fn parse_cfront_vtable(&mut self, start: usize) -> Result<NodeRef<'a>, ParseError> {
        self.pos = start + 8;
        let mut components = Vec::new();
        loop {
            components.push(self.parse_class_name()?);
            if self.pos == self.bytes.len() {
                break;
            }
            if !self.starts_at(self.pos, b"__") {
                return Err(self.error(ParseErrorKind::UnexpectedByte));
            }
            self.pos += 2;
        }
        components.reverse();
        let name = self.nest(start, components)?;
        self.make(Kind::VTable, None, start, alloc::vec![name])
    }

    /// `__sti__<name>` / `__std__<name>`: a static initialiser or destructor.
    ///
    /// The payload is a legacy symbol when it carries a `__` separator
    /// (`__sti__bar__Fv`), otherwise one name production that runs to the end
    /// of the symbol (`__sti__foo`, `__sti__Q23foo3bar`). The reference
    /// demangler appends an undecodable payload verbatim; that fallback is not
    /// reproduced.
    fn parse_cfront_wrapper(
        &mut self,
        start: usize,
        wrapper: &'static str,
    ) -> Result<NodeRef<'a>, ParseError> {
        let payload = start + 7;
        if payload >= self.bytes.len() {
            return Err(self.error(ParseErrorKind::UnexpectedEnd));
        }
        let has_separator = self.bytes[payload..].windows(2).any(|pair| pair == b"__");
        let shape = if has_separator {
            self.parse_symbol(payload)?
        } else {
            let name = self.parse_encoded_name(payload, self.bytes.len())?;
            self.make(Kind::Data, None, payload, alloc::vec![name])?
        };
        self.make(
            Kind::LegacySpecial,
            Some(Cow::Borrowed(wrapper)),
            start,
            alloc::vec![shape],
        )
    }

    /// `<name>__pt__<count>_<args>`: an ARM or EDG parameterized type.
    ///
    /// `count` is the byte length of the `_`-introduced argument list, so the
    /// arguments always run to the end of the symbol. `name` is the template
    /// base: a qualified name, a template-id, a length-prefixed class name or
    /// a plain name.
    fn parse_parameterized_type(
        &mut self,
        start: usize,
    ) -> Result<Option<NodeRef<'a>>, ParseError> {
        let markers: &[&str] = match self.dialect {
            Dialect::Arm => &CFRONT_TEMPLATE_MARKERS[..1],
            // HP aCC reads the `X` template spec instead, and the reference
            // demangler rejects the parameterized-type form in HP mode.
            Dialect::Hp => &[],
            _ => &CFRONT_TEMPLATE_MARKERS,
        };
        let anchor = markers
            .iter()
            .filter_map(|marker| {
                self.bytes[start..]
                    .windows(marker.len())
                    .position(|window| window == marker.as_bytes())
                    .map(|offset| start + offset)
            })
            .min();
        let Some(anchor) = anchor else {
            return Ok(None);
        };
        if anchor == start {
            return Err(self.error(ParseErrorKind::Unsupported));
        }
        let base = self.parse_encoded_name(start, anchor)?;
        self.pos = anchor + 6;
        let length = self.parse_count()?;
        let args_start = self.pos;
        if length == 0 || self.bytes.len() - args_start != length {
            return Err(self.error(ParseErrorKind::InvalidLength));
        }
        self.expect(b'_')?;
        let mut arguments = Vec::new();
        while self.pos < self.bytes.len() {
            let before = self.pos;
            arguments.push(self.parse_type()?);
            if self.pos == before {
                return Err(self.error(ParseErrorKind::UnexpectedByte));
            }
        }
        let arguments = self.make(Kind::TemplateArgs, None, args_start, arguments)?;
        let name = self.make(Kind::Name, None, start, alloc::vec![base, arguments])?;
        Ok(Some(self.make(
            Kind::Data,
            None,
            start,
            alloc::vec![name],
        )?))
    }

    // -- HP aCC template specs ---------------------------------------------

    /// Decode the name part `start..split` and attach its HP aCC template spec.
    ///
    /// The spec belongs to the name, so the name part is decoded as one name
    /// production (`3baz__XSP5` is `baz<5>`) before the arguments are attached.
    fn parse_hp_template_name(
        &mut self,
        start: usize,
        split: usize,
    ) -> Result<NodeRef<'a>, ParseError> {
        let base = self.parse_encoded_name(start, split)?;
        self.pos = split + 2;
        let arguments = self.parse_hp_template_args()?;
        self.make(Kind::Name, None, start, alloc::vec![base, arguments])
    }

    /// `X<argument>*`: an HP aCC template spec.
    ///
    /// The argument list ends at the first byte that does not start an
    /// argument, so it runs to the end of the symbol, to the `_` that
    /// separates it from a following `F`-marked argument list, or to a byte
    /// the caller continues with. A terminating `_` is consumed.
    fn parse_hp_template_args(&mut self) -> Result<NodeRef<'a>, ParseError> {
        self.expect(b'X')?;
        let args_start = self.pos;
        let mut arguments = Vec::new();
        loop {
            self.tick()?;
            match self.peek() {
                Some(b'T') => {
                    self.pos += 1;
                    arguments.push(self.parse_type()?);
                }
                Some(b'S' | b'U') => arguments.push(self.parse_hp_integral_value()?),
                Some(b'A') => arguments.push(self.parse_hp_literal()?),
                _ => break,
            }
        }
        let arguments = self.make(Kind::TemplateArgs, None, args_start, arguments)?;
        if self.peek() == Some(b'_') {
            self.pos += 1;
        }
        Ok(arguments)
    }

    /// `S`/`U` `P`/`N`/`M` `<digits>`: an HP aCC integral template argument.
    ///
    /// `S` is a signed value and `U` an unsigned one, printed with a trailing
    /// `U`. `P` prefixes a non-negative value, `N` a negated one and `M` is the
    /// special spelling of `-2147483648`, which prints without the `U` suffix.
    /// The digits are copied verbatim, as the reference demangler copies them.
    fn parse_hp_integral_value(&mut self) -> Result<NodeRef<'a>, ParseError> {
        let start = self.pos;
        let unsigned = self.peek() == Some(b'U');
        self.pos += 1;
        let mut text = String::new();
        let mut suffix_unsigned = unsigned;
        match self.peek() {
            Some(b'M') => {
                self.pos += 1;
                text.push_str("-2147483648");
                suffix_unsigned = false;
            }
            Some(sign @ (b'P' | b'N')) => {
                self.pos += 1;
                if sign == b'N' {
                    text.push('-');
                }
                let digits_start = self.pos;
                while self.peek().is_some_and(|byte| byte.is_ascii_digit()) {
                    self.pos += 1;
                }
                if self.pos == digits_start {
                    return Err(self.error(ParseErrorKind::UnexpectedByte));
                }
                text.push_str(&decode(&self.bytes[digits_start..self.pos]));
            }
            _ => return Err(self.error(ParseErrorKind::UnexpectedByte)),
        }
        if suffix_unsigned {
            text.push('U');
        }
        self.make(Kind::Literal, Some(Cow::Owned(text)), start, Vec::new())
    }

    /// `A<count><payload>`: an HP aCC named-literal template argument.
    ///
    /// The payload is an array or function name, so the argument prints `&`
    /// followed by the payload demangled as a legacy symbol when it is one, and
    /// by the payload verbatim when it is not. The reference demangler makes the
    /// same choice.
    fn parse_hp_literal(&mut self) -> Result<NodeRef<'a>, ParseError> {
        let start = self.pos;
        self.expect(b'A')?;
        let length = self.parse_count()?;
        if length == 0 {
            return Err(self.error(ParseErrorKind::InvalidLength));
        }
        let payload_start = self.pos;
        let payload_end = payload_start
            .checked_add(length)
            .ok_or_else(|| self.error(ParseErrorKind::InvalidLength))?;
        let Some(payload) = self.bytes.get(payload_start..payload_end) else {
            return Err(self.error(ParseErrorKind::InvalidLength));
        };
        self.pos = payload_end;
        let text = self.hp_literal_text(payload)?;
        self.make(Kind::Literal, Some(text), start, Vec::new())
    }

    /// The text of an `A` literal argument: `&` and the payload.
    ///
    /// A payload that parses as a legacy symbol is rendered at parse time. The
    /// nested parse is one more nesting level of this parse and shares its
    /// work and node counters, and the render is charged to this parse's
    /// expansion visit and byte budgets, so nested literals cannot escape the
    /// limits the caller configured. A budget failure rejects the whole
    /// symbol; a payload that is not a symbol prints verbatim.
    fn hp_literal_text(&mut self, payload: &'a [u8]) -> Result<Cow<'a, str>, ParseError> {
        self.enter()?;
        let mut nested = Parser::new(payload, self.dialect, self.limits);
        nested.depth = self.depth;
        nested.work = self.work;
        nested.work_limit = self.work_limit;
        nested.nodes = self.nodes;
        nested.expanded_visits = self.expanded_visits;
        nested.expanded_bytes = self.expanded_bytes;
        let parsed = nested.parse_root();
        self.work = nested.work;
        self.nodes = nested.nodes;
        self.expanded_visits = nested.expanded_visits;
        self.expanded_bytes = nested.expanded_bytes;
        self.leave();
        match parsed {
            Ok(root) => {
                let ast = Ast::from_root_legacy(root, self.dialect);
                self.charge_expansion(&ast)?;
                Ok(Cow::Owned(alloc::format!("&{ast}")))
            }
            Err(error) if is_fatal(error.kind()) => Err(self.error(error.kind())),
            Err(_) => Ok(Cow::Owned(alloc::format!("&{}", decode(payload)))),
        }
    }

    /// Charge one parse-time render of a nested `ast` to the expansion
    /// budgets, and reject a tree the printer could not render safely.
    fn charge_expansion(&mut self, ast: &Ast<'a>) -> Result<(), ParseError> {
        if !ast.printer_depth_ok() {
            return Err(self.error(ParseErrorKind::TooDeep));
        }
        self.expanded_visits = self.expanded_visits.saturating_add(ast.expanded_visits());
        if self.expanded_visits > self.limits.expansion_visits() {
            return Err(self.error(ParseErrorKind::TooManyExpansionVisits));
        }
        self.expanded_bytes = self.expanded_bytes.saturating_add(ast.expanded_bytes());
        if self.expanded_bytes > self.limits.expansion_bytes() {
            return Err(self.error(ParseErrorKind::TooManyExpansionBytes));
        }
        Ok(())
    }

    /// Skip an HP aCC alternate entry point suffix (`_<digits>`).
    ///
    /// The suffix names an alternate entry point of the function and is dropped,
    /// as the reference demangler drops it. An underscore with no digits is
    /// accepted, again as the reference accepts it.
    fn skip_hp_alternate_entry(&mut self) {
        while self.peek() == Some(b'_') {
            self.pos += 1;
            while self.peek().is_some_and(|byte| byte.is_ascii_digit()) {
                self.pos += 1;
            }
        }
    }

    /// Parse one name production that spans exactly `start..end`.
    ///
    /// Used for the template base before a parameterized-type marker, for the
    /// template base of an HP aCC template spec and for a `__sti__`/`__std__`
    /// payload that carries no `__` separator.
    fn parse_encoded_name(&mut self, start: usize, end: usize) -> Result<NodeRef<'a>, ParseError> {
        self.pos = start;
        let base = match self.peek() {
            Some(b'Q') => self.parse_qualified_name()?,
            Some(b't') => self.parse_template()?,
            Some(byte) if byte.is_ascii_digit() => self.parse_class_name()?,
            _ => self
                .parse_name_part(start, end)?
                .ok_or_else(|| self.error(ParseErrorKind::Unsupported))?,
        };
        if self.pos != end {
            return Err(self.error(ParseErrorKind::UnexpectedByte));
        }
        Ok(base)
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
        let text = &self.bytes[start..end];
        if has_blank(text) {
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
        let ident = self.make(Kind::Identifier, Some(decode(text)), start, Vec::new())?;
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
        let mut component_start = None;
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
                Some(byte @ (b'Q' | b't')) | Some(byte @ b'0'..=b'9') => {
                    component_start = Some(byte);
                    component = Some(match byte {
                        b'Q' => self.parse_qualified_name()?,
                        b't' => self.parse_template()?,
                        _ => self.parse_class_name()?,
                    });
                    break;
                }
                Some(b'_') => return Err(self.error(ParseErrorKind::UnexpectedByte)),
                _ => break,
            }
        }
        if let Some(component) = &component {
            self.remember(component);
        }
        // ARM and EDG encode the argument list with `F`, but a template
        // component implies one for both dialects, and EDG also implies one
        // after a length-prefixed class name. A qualified name never implies
        // one, so `bar__Q23foo3baz` names a data object in both dialects.
        let implicit_args = match component_start {
            Some(b't') => true,
            Some(byte) if byte.is_ascii_digit() => self.dialect == Dialect::Edg,
            _ => false,
        };
        let has_args = self.peek() == Some(b'F');
        if has_args {
            self.pos += 1;
            if self.is_cfront() {
                // ARM and EDG number argument back-references from one and
                // only count the types encoded after the `F` marker.
                self.types.clear();
            }
        } else if self.is_cfront() && !implicit_args {
            // Without an argument list the symbol names a data object.
            if self.is_hp() {
                self.skip_hp_alternate_entry();
            }
            let name = self.combine_name(start, component, name_part, constructor, destructor)?;
            let text = modifier_text(static_function, &qualifiers);
            return self.make(Kind::Data, text, start, alloc::vec![name]);
        }
        let parameters = self.parse_args()?;
        if has_args && self.dialect == Dialect::Edg && self.peek() == Some(b'_') {
            // EDG appends the return type after the argument list. The legacy
            // printers drop it, as the reference demangler does, so it is
            // parsed only to check that it is well formed.
            self.pos += 1;
            if self.pos < self.bytes.len() {
                let _return_type = self.parse_type()?;
            }
        }
        if self.is_hp() {
            self.skip_hp_alternate_entry();
        }
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
        let Some(text) = self.bytes.get(text_start..text_end) else {
            return Err(self.error(ParseErrorKind::InvalidLength));
        };
        self.pos = text_end;
        let ident = self.make(Kind::Identifier, Some(decode(text)), text_start, Vec::new())?;
        let name = self.make(Kind::UnqualifiedName, None, start, alloc::vec![ident])?;
        if self.is_hp() && self.peek() == Some(b'X') {
            // An HP aCC template spec may follow any length-prefixed class
            // name, so `3barXSP6` is `bar<6>`.
            let arguments = self.parse_hp_template_args()?;
            return self.make(Kind::Name, None, start, alloc::vec![name, arguments]);
        }
        Ok(name)
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
        let Some(text) = self.bytes.get(text_start..text_end) else {
            return Err(self.error(ParseErrorKind::InvalidLength));
        };
        self.pos = text_end;
        let ident = self.make(Kind::Identifier, Some(decode(text)), text_start, Vec::new())?;
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
        Ok(decode(&self.bytes[start..self.pos]).into_owned())
    }

    // -- types -------------------------------------------------------------

    fn parse_type(&mut self) -> Result<NodeRef<'a>, ParseError> {
        self.enter()?;
        let result = self.parse_type_inner();
        self.leave();
        result
    }

    /// Parse the optional `_<return type>` suffix of a function type.
    ///
    /// GNU v2 spells a function type as `F<arguments>[_<return type>]`. The
    /// historical demangler accepts a missing suffix and prints `void`; when
    /// the suffix is present but empty (a trailing `_`) it also prints `void`.
    fn parse_function_return_type(&mut self) -> Result<Option<NodeRef<'a>>, ParseError> {
        if self.eat(b'_') {
            self.parse_return_type()
        } else {
            Ok(None)
        }
    }

    /// Parse a return type after the `_` separator has been consumed.
    fn parse_return_type(&mut self) -> Result<Option<NodeRef<'a>>, ParseError> {
        self.tick()?;
        if self.pos == self.bytes.len() || self.peek() == Some(b'_') {
            return Ok(None);
        }
        self.parse_type().map(Some)
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
                let return_type = self.parse_function_return_type()?;
                let mut children = alloc::vec![parameters];
                if let Some(return_type) = return_type {
                    children.push(return_type);
                }
                self.make(Kind::FunctionType, None, start, children)
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
                let return_type = self.parse_return_type()?;
                let mut children = alloc::vec![parameters];
                if let Some(return_type) = return_type {
                    children.push(return_type);
                }
                let text = qualifier_text(&qualifiers);
                let function = self.make(Kind::FunctionType, text, function_start, children)?;
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
            Some(b'G') => {
                // `G` marks a class, struct or union type. The marker does not
                // change the spelling, but a primitive type after it is not a
                // valid use of the production.
                self.pos += 1;
                let inner = self.parse_type()?;
                if !is_class_like(&inner) {
                    return Err(self.error(ParseErrorKind::Unsupported));
                }
                Ok(inner)
            }
            Some(b'K' | b'B' | b'X' | b'Y') => Err(self.error(ParseErrorKind::Unsupported)),
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
        let digits = decode(&self.bytes[digits_start..self.pos]);
        let bits = u32::from_str_radix(digits.as_ref(), 16)
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
    ///
    /// This is the type-position form. ARM and EDG keep it zero-based even
    /// though their argument-position references are one-based, which is what
    /// the reference demangler does.
    fn parse_backref(&mut self) -> Result<NodeRef<'a>, ParseError> {
        self.expect(b'T')?;
        let index = self.parse_get_count()?;
        self.types
            .get(index)
            .cloned()
            .ok_or_else(|| self.error(ParseErrorKind::UnknownSubstitution))
    }

    /// Parse a `T<n>` back-reference in argument position.
    fn parse_arg_backref(&mut self) -> Result<NodeRef<'a>, ParseError> {
        self.expect(b'T')?;
        let count = self.parse_get_count()?;
        let index = self.backref_offset(count)?;
        self.types
            .get(index)
            .cloned()
            .ok_or_else(|| self.error(ParseErrorKind::UnknownSubstitution))
    }

    /// Translate an encoded argument back-reference index into a position in
    /// the remembered-type table: zero-based for GNU v2, one-based for ARM and
    /// EDG.
    fn backref_offset(&self, count: usize) -> Result<usize, ParseError> {
        if self.is_cfront() {
            count
                .checked_sub(1)
                .ok_or_else(|| self.error(ParseErrorKind::UnknownSubstitution))
        } else {
            Ok(count)
        }
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
                    let ty = self.parse_arg_backref()?;
                    self.remember(&ty);
                    parameters.push(self.make_parameter(ty)?);
                }
                Some(b'N') => {
                    self.pos += 1;
                    let repeats = self.parse_get_count()?;
                    let index = self.parse_get_count()?;
                    let index = self.backref_offset(index)?;
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
            | ParseErrorKind::TooManyExpansionVisits
            | ParseErrorKind::TooManyExpansionBytes
            | ParseErrorKind::TooLong
    )
}

/// Collect the `__` split points of `bytes`, left to right.
///
/// A run of `_` bytes contributes its last pair, so `foo___bar` splits at the
/// second and third underscore. The signature must be non-empty.
fn split_positions(bytes: &[u8], base: usize) -> Vec<usize> {
    let mut out = Vec::new();
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
        if split + 2 < bytes.len() {
            out.push(base + split);
        }
        index = run_end;
    }
    out
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
fn operator_spelling_from_name(name: &[u8]) -> Option<&'static str> {
    let rest = name
        .strip_prefix(b"op$")
        .or_else(|| name.strip_prefix(b"op."))
        // GNU v2 spells a method operator as `__<code>__<class>...`, so the
        // name part before the second `__` carries the separator too.
        .or_else(|| name.strip_prefix(b"__"))?;
    if let Some(code) = rest.strip_prefix(b"assign_") {
        let code = core::str::from_utf8(code).ok()?;
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
    operator_spelling(core::str::from_utf8(rest).ok()?)
}

/// The byte offset of the conversion operator's target type inside a name
/// part, if the name part is a conversion operator.
fn conversion_operator_offset(name: &[u8]) -> Option<usize> {
    if name.starts_with(b"__op") {
        return Some(4);
    }
    let rest = name
        .strip_prefix(b"type$")
        .or_else(|| name.strip_prefix(b"type."))?;
    if rest.is_empty() {
        return None;
    }
    Some(5)
}

/// True when a name part contains whitespace or a control character.
///
/// Valid UTF-8 is checked character-wise, so a Unicode space is rejected as
/// before; bytes that are not UTF-8 are checked for the ASCII classes.
fn has_blank(text: &[u8]) -> bool {
    if let Ok(text) = core::str::from_utf8(text) {
        return text.chars().any(|ch| ch.is_whitespace() || ch.is_control());
    }
    text.iter()
        .any(|byte| byte.is_ascii_whitespace() || byte.is_ascii_control())
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
    out.write_str(printer.buf.as_str())
}

/// Render `ast` and return the printer's visit/depth/byte counts.
#[cfg(test)]
pub(crate) fn stats<W: Write + ?Sized>(
    ast: &Ast<'_>,
    out: &mut W,
) -> Result<(usize, u32, usize), fmt::Error> {
    let mut printer = Printer::new();
    printer.symbol(&ast.root())?;
    let (visits, depth, bytes) = (
        printer.buf.visits(),
        printer.buf.max_depth(),
        printer.buf.bytes(),
    );
    out.write_str(printer.buf.as_str())?;
    Ok((visits, depth, bytes))
}

/// One pass over a legacy [`Ast`], writing text into an owned buffer.
struct Printer {
    /// Output text, recursion depth and node-visit accounting.
    buf: PrintBuffer,
}

impl Printer {
    fn new() -> Self {
        Self {
            buf: PrintBuffer::new(),
        }
    }

    fn enter(&mut self) -> fmt::Result {
        self.buf.enter()
    }

    fn leave(&mut self) {
        self.buf.leave();
    }

    fn write(&mut self, text: &str) -> fmt::Result {
        self.buf.write(text)
    }

    fn write_char(&mut self, ch: char) -> fmt::Result {
        self.buf.write_char(ch)
    }

    fn last_char(&self) -> Option<char> {
        self.buf.last_char()
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
            Kind::Data => {
                self.name(&required(node, 0)?)?;
                if let Some(modifiers) = node.text() {
                    self.write_char(' ')?;
                    self.write(modifiers)?;
                }
                Ok(())
            }
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
            Kind::LegacySpecial => {
                self.write("global ")?;
                self.write(Self::text(node)?)?;
                self.write(" keyed to ")?;
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
                            let is_bool = argument.child(0).is_some_and(|ty| {
                                ty.kind() == Kind::BuiltinType
                                    && Self::text(&ty).is_ok_and(|text| text == "bool")
                            });
                            match (is_bool, value) {
                                (true, "0") => self.write("false")?,
                                (true, "1") => self.write("true")?,
                                _ => self.write(value)?,
                            }
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
                    if !matches!(self.last_char(), Some(' ' | '*' | '&')) {
                        self.write_char(' ')?;
                    }
                    self.write_char('(')?;
                } else if matches!(
                    self.last_char(),
                    Some(ch) if ch.is_alphanumeric() || matches!(ch, '>' | '_' | '$')
                ) {
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
                match node.child(1) {
                    Some(return_type) => self.type_left(&return_type)?,
                    // A function type with no encoded return type returns `void`.
                    None => self.write("void ")?,
                }
                Ok(())
            }
            Kind::MemberPointerType => {
                let class = required(node, 0)?;
                let member = required(node, 1)?;
                self.type_left(&member)?;
                if matches!(
                    self.last_char(),
                    Some(ch) if ch.is_alphanumeric() || matches!(ch, '>' | '_' | '$')
                ) {
                    self.write_char(' ')?;
                }
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
                if let Some(return_type) = node.child(1) {
                    self.type_right(&return_type)?;
                }
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

/// True when a type names a class, struct, union or template-id.
fn is_class_like(node: &NodeRef<'_>) -> bool {
    matches!(
        node.kind(),
        Kind::Name | Kind::NestedName | Kind::UnqualifiedName | Kind::Identifier
    )
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

    use super::super::MAX_INPUT_LEN;
    use super::*;
    use crate::cpp::{self, Dialect};

    fn parse(symbol: &str) -> Result<Ast<'_>, ParseError> {
        parse_bytes_with_budget(symbol.as_bytes(), DEFAULT_BUDGET)
    }

    /// The compiled-in legacy limits, for tests that pick a dialect directly.
    fn default_limits() -> ParseLimits {
        ParseLimits::legacy(DEFAULT_BUDGET)
    }

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
            "__ct__3fooFv",
            "__vtbl__3foo",
            "__sti__foo",
            "foo__pt__2_i",
            "foo__Fi_v",
            "foo__tm__2_i",
            "foo__XSP5_Fv",
            "foo__XT3bar_Fv",
            "foo__Fi_1",
            "foo__FG3bar",
            "_GLOBAL_$I$foo__Fv",
        ] {
            assert!(
                is_legacy_name_bytes(symbol.as_bytes()),
                "{symbol} should be legacy C++"
            );
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
            "__ct__3fooFzzz",
            "__sti__",
            "foo__Fi_ii",
        ] {
            assert!(
                !is_legacy_name_bytes(symbol.as_bytes()),
                "{symbol} should not be legacy C++"
            );
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
            ("foo__FPFi_v", "foo(void (*)(int))"),
            ("foo__FPFib_v", "foo(void (*)(int, bool))"),
            ("foo__FPFiPc_i", "foo(int (*)(int, char *))"),
            ("foo__FP4foo_", "foo(foo_ *)"),
            ("foo__FM3fooFv_", "foo(void (foo::*)(void))"),
            ("foo__FM3fooCFv_", "foo(void (foo::*)(void) const)"),
            ("foo__FM3fooFi_v", "foo(void (foo::*)(int))"),
            ("foo__FG3bar", "foo(bar)"),
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
        assert_eq!(rendered("__as__3fooR3foo"), "foo::operator=(foo &)");
        assert_eq!(rendered("__apl__3fooPCc"), "foo::operator+=(char const *)");
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
        assert_eq!(rendered("foo__t6vector1b1"), "vector<true>::foo(void)");
        assert_eq!(rendered("foo__t6vector1b0"), "vector<false>::foo(void)");
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
        assert_eq!(
            rendered("_GLOBAL_$I$foo__Fv"),
            "global constructors keyed to foo(void)"
        );
        assert_eq!(
            rendered("_GLOBAL_.D.foo__Fv"),
            "global destructors keyed to foo(void)"
        );
        assert_eq!(
            rendered("_GLOBAL_$D$__3foo"),
            "global destructors keyed to foo::foo(void)"
        );
    }

    #[test]
    fn reports_cfront_dialects() {
        for (symbol, dialect) in [
            ("foo__Fiv", Dialect::GnuV2),
            ("bar__3foo", Dialect::GnuV2),
            ("_$_3foo", Dialect::GnuV2),
            ("__ct__3fooFv", Dialect::Arm),
            ("__dt__3foo", Dialect::Arm),
            ("__vtbl__3foo__3bar", Dialect::Arm),
            ("__sti__foo", Dialect::Arm),
            ("__std__foo", Dialect::Arm),
            ("foo__pt__2_i", Dialect::Arm),
            ("foo__FiT1", Dialect::Arm),
            ("foo__Fi_v", Dialect::Edg),
            ("foo__F_i", Dialect::Edg),
            ("foo__tm__2_i", Dialect::Edg),
            ("foo__ps__3_ii", Dialect::Edg),
            ("__ct__3fooFi_v", Dialect::Edg),
            // HP aCC is tried last, so only its own productions reach it.
            ("foo__XSP5_Fv", Dialect::Hp),
            ("foo__XT3bar_Fv", Dialect::Hp),
            ("foo__XSP5", Dialect::Hp),
            ("bar__XSP5", Dialect::Hp),
            ("3baz__XSP5", Dialect::Hp),
            ("foo__F3barXSP5", Dialect::Hp),
            ("foo__Fi_1", Dialect::Hp),
            ("bar__3foo_1", Dialect::Hp),
            ("__ct__3fooXSP5_Fv", Dialect::Hp),
            ("__vtbl__3fooXSP5", Dialect::Hp),
        ] {
            let ast = cpp::parse(symbol).unwrap_or_else(|error| panic!("{symbol}: {error}"));
            assert_eq!(ast.dialect(), dialect, "{symbol}");
        }
    }

    #[test]
    fn parses_hp_template_specs() {
        assert_eq!(rendered("foo__XSP5_Fv"), "foo<5>(void)");
        assert_eq!(rendered("foo__XSP5"), "foo<5>");
        assert_eq!(rendered("foo__X"), "foo<>");
        assert_eq!(rendered("foo__XSP5_"), "foo<5>");
        assert_eq!(rendered("foo__XT3bar_Fv"), "foo<bar>(void)");
        assert_eq!(rendered("foo__XT3barT3baz_Fv"), "foo<bar, baz>(void)");
        assert_eq!(rendered("foo__XTP3bar_Fv"), "foo<bar *>(void)");
        assert_eq!(rendered("foo__XT3barXSP6_Fv"), "foo<bar<6>>(void)");
        assert_eq!(rendered("foo__XSP5T3bar"), "foo<5, bar>");
        // The base name is decoded like the parameterized-type base.
        assert_eq!(rendered("3baz__XSP5"), "baz<5>");
        assert_eq!(rendered("Q2_3foo3bar__XSP5_Fv"), "foo::bar<5>(void)");
        // A spec may follow a class name in type position too.
        assert_eq!(rendered("foo__F3barXSP5"), "foo(bar<5>)");
        assert_eq!(rendered("foo__FP3barXSP5"), "foo(bar<5> *)");
        assert_eq!(rendered("foo__FQ2_3bar3bazXSP5"), "foo(bar::baz<5>)");
        assert_eq!(rendered("foo__F3barXSP5_i"), "foo(bar<5>, int)");
        assert_eq!(rendered("__ct__3fooXSP5_Fv"), "foo<5>::foo(void)");
        assert_eq!(rendered("__dt__3fooXSP5_Fv"), "foo<5>::~foo(void)");
        assert_eq!(rendered("__vtbl__3fooXSP5"), "foo<5> virtual table");
    }

    #[test]
    fn parses_hp_integral_and_literal_arguments() {
        assert_eq!(rendered("foo__XSP0_Fv"), "foo<0>(void)");
        assert_eq!(rendered("foo__XSN0_Fv"), "foo<-0>(void)");
        assert_eq!(rendered("foo__XSN5_Fv"), "foo<-5>(void)");
        assert_eq!(rendered("foo__XUP7_Fv"), "foo<7U>(void)");
        assert_eq!(rendered("foo__XUN7_Fv"), "foo<-7U>(void)");
        assert_eq!(rendered("foo__XSM_Fv"), "foo<-2147483648>(void)");
        // `M` prints without the unsigned suffix, as the reference does.
        assert_eq!(rendered("foo__XUM_Fv"), "foo<-2147483648>(void)");
        assert_eq!(rendered("foo__XSP4294967295_Fv"), "foo<4294967295>(void)");
        assert_eq!(rendered("foo__XA3bar_Fv"), "foo<&bar>(void)");
        assert_eq!(rendered("foo__XA4_bar_Fv"), "foo<&_bar>(void)");
        // A payload that is itself a legacy symbol is demangled.
        assert_eq!(rendered("foo__XA10bar__3baz_Fv"), "foo<&baz::bar>(void)");
        assert_eq!(rendered("foo__XA9_3foo$bar_Fv"), "foo<&_3foo$bar>(void)");
    }

    #[test]
    fn hp_drops_alternate_entry_points() {
        assert_eq!(rendered("foo__Fi_1"), "foo(int)");
        assert_eq!(rendered("foo__Fi_12"), "foo(int)");
        assert_eq!(rendered("foo__Fi_1_2"), "foo(int)");
        assert_eq!(rendered("foo__Fi__"), "foo(int)");
        assert_eq!(rendered("foo__XSP5_Fv_1"), "foo<5>(void)");
        assert_eq!(rendered("bar__3foo_1"), "foo::bar");
        assert_eq!(
            rendered("__sti__bar__Fv_1"),
            "global constructors keyed to bar(void)"
        );
        // Only an underscore run and digits may follow the signature.
        assert!(parse_with(b"foo__Fi_1x", Dialect::Hp, default_limits()).is_err());
        assert!(parse("foo__F3barXSP5_1").is_err());
    }

    #[test]
    fn hp_rejects_productions_it_does_not_support() {
        // The reference demangler rejects the parameterized-type form in HP
        // mode, so HP never reads `__pt__` even though ARM does.
        assert!(parse_with(b"foo__pt__2_i", Dialect::Hp, default_limits()).is_err());
        assert!(parse_with(b"foo__tm__2_i", Dialect::Hp, default_limits()).is_err());
        assert_eq!(
            parse("foo__pt__2_i").map(|ast| ast.dialect()),
            Ok(Dialect::Arm)
        );
        for symbol in [
            "foo__FL1_3bar",
            "foo__XSP_Fv",
            "foo__XA0",
            "foo__XA3ba",
            "foo__XSP5XSP6_Fv",
            "foo__XTA3bar_Fv",
            "foo__FXT3bar",
            "foo__FPXSP5",
            "foo__XAP5_Fv",
            "__ct__XSP5_Fv",
        ] {
            assert!(parse(symbol).is_err(), "{symbol} should be rejected");
        }
    }

    fn nested_literal(levels: usize) -> String {
        let mut symbol = String::from("bar__3baz");
        for _ in 0..levels {
            let payload = symbol.clone();
            symbol = alloc::format!("foo__XA{}", payload.len());
            symbol.push_str(&payload);
        }
        symbol
    }

    #[test]
    fn hp_literal_nesting_renders_every_level() {
        let symbol = nested_literal(3);
        let ast = parse_with(symbol.as_bytes(), Dialect::Hp, default_limits())
            .expect("nested literals parse");
        let text = alloc::format!("{ast}");
        assert_eq!(text.matches("foo<&").count(), 3, "{text}");
        assert!(text.contains("baz::bar"), "{text}");
    }

    #[test]
    fn hp_literal_nesting_is_charged_to_the_parse_depth() {
        // Nested `A` payloads demangle with a nested parser that continues this
        // parse's depth counter, so a chain of payloads hits the depth budget
        // instead of falling back to undemangled text.
        let symbol = nested_literal(64);
        let error = parse_with(symbol.as_bytes(), Dialect::Hp, default_limits())
            .expect_err("nesting past the depth budget is rejected");
        assert_eq!(error.kind(), ParseErrorKind::TooDeep);
    }

    #[test]
    fn parses_arm_constructors_and_destructors() {
        assert_eq!(rendered("__ct__3foo"), "foo::foo");
        assert_eq!(rendered("__ct__3fooFv"), "foo::foo(void)");
        assert_eq!(rendered("__ct__3fooFi"), "foo::foo(int)");
        assert_eq!(rendered("__dt__3foo"), "foo::~foo");
        assert_eq!(rendered("__dt__3fooFv"), "foo::~foo(void)");
        assert_eq!(rendered("__ct__Q23bar3bazFv"), "bar::baz::baz(void)");
        assert_eq!(rendered("__ct__Q23bar3baz"), "bar::baz::baz");
        assert_eq!(rendered("__dt__Q23bar3bazFv"), "bar::baz::~baz(void)");
        assert_eq!(rendered("__ct__t6vector1Zii"), "vector<int>::vector(int)");
    }

    #[test]
    fn parses_arm_vtables_and_static_wrappers() {
        assert_eq!(rendered("__vtbl__3foo"), "foo virtual table");
        assert_eq!(rendered("__vtbl__3foo__3bar"), "bar::foo virtual table");
        assert_eq!(rendered("__sti__foo"), "global constructors keyed to foo");
        assert_eq!(rendered("__std__foo"), "global destructors keyed to foo");
        assert_eq!(
            rendered("__sti__bar__Fv"),
            "global constructors keyed to bar(void)"
        );
        assert_eq!(
            rendered("__std__bar__3foo"),
            "global destructors keyed to foo::bar"
        );
        assert_eq!(
            rendered("__sti__op$pl__Fv"),
            "global constructors keyed to operator+(void)"
        );
        assert_eq!(
            rendered("__sti__Q23bar3baz"),
            "global constructors keyed to bar::baz"
        );
        assert_eq!(
            rendered("__sti__bar__S3foo"),
            "global constructors keyed to foo::bar static"
        );
    }

    #[test]
    fn parses_parameterized_types() {
        assert_eq!(rendered("foo__pt__2_i"), "foo<int>");
        assert_eq!(rendered("foo__pt__3_ii"), "foo<int, int>");
        assert_eq!(rendered("foo__pt__4_iPc"), "foo<int, char *>");
        assert_eq!(rendered("foo__pt__2_v"), "foo<void>");
        assert_eq!(rendered("Q2_3foo3bar__pt__2_i"), "foo::bar<int>");
        assert_eq!(rendered("3baz__pt__2_i"), "baz<int>");
        assert_eq!(rendered("foo__tm__2_i"), "foo<int>");
        assert_eq!(rendered("foo__ps__3_ii"), "foo<int, int>");
        // The argument list length must match the encoded count exactly.
        for symbol in ["foo__pt__2_Pi", "foo__pt__6_PiPi", "__pt__2_i"] {
            assert!(
                !matches!(parse(symbol).map(|ast| ast.dialect()), Ok(Dialect::Arm)),
                "{symbol} should not be an ARM parameterized type"
            );
        }
    }

    #[test]
    fn cfront_back_references_are_one_based() {
        assert_eq!(rendered("foo__FiT1"), "foo(int, int)");
        assert_eq!(rendered("foo__FiT1T2"), "foo(int, int, int)");
        assert_eq!(rendered("foo__FiN21"), "foo(int, int, int)");
        assert_eq!(rendered("foo__FvN11"), "foo(void, void)");
        assert_eq!(rendered("foo__FPPiT1"), "foo(int **, int **)");
        assert_eq!(
            rendered("foo__FiR3fooT1T2T1T2"),
            "foo(int, foo &, foo &, foo &, foo &, foo &)"
        );
        // The cfront reading only applies when GNU v2 cannot parse the name.
        assert_eq!(
            rendered("__ct__3fooFiR3fooT1T2T1T2"),
            "foo::foo(int, foo &, int, foo &, int, foo &)"
        );
    }

    #[test]
    fn parses_edg_return_types() {
        assert_eq!(rendered("foo__Fi_v"), "foo(int)");
        assert_eq!(rendered("foo__FiPc_"), "foo(int, char *)");
        assert_eq!(rendered("foo__Fi_"), "foo(int)");
        assert_eq!(rendered("foo__Fv_"), "foo(void)");
        assert_eq!(rendered("foo__F_i"), "foo(void)");
        assert_eq!(rendered("__ct__3fooFi_v"), "foo::foo(int)");
        assert_eq!(rendered("__dt__3fooFi_v"), "foo::~foo(int)");
        // ARM has no return-type suffix, so EDG is the only legacy reading.
        for symbol in ["foo__Fi_ii", "__std__bar__3foo__Fi_v"] {
            assert!(parse(symbol).is_err(), "{symbol} should be rejected");
        }
    }

    #[test]
    fn cfront_markers_keep_the_gnu_v2_reading() {
        // A marker only selects a cfront dialect when that dialect consumes
        // the whole symbol; otherwise GNU v2 keeps its reading.
        assert_eq!(rendered("foo__pt__2_Pi"), "_P::foo__pt(int)");
        assert_eq!(rendered("__vtbl__Q23bar3baz"), "bar::baz::__vtbl(void)");
        assert_eq!(rendered("__vtbl__t6vector1Zii"), "vector<int>::__vtbl(int)");
        for symbol in ["__ct__3fooFzzz", "__vtbl__3foo__", "__sti__"] {
            assert!(parse(symbol).is_err(), "{symbol} should be rejected");
        }
    }

    #[test]
    fn cfront_truncations_never_panic() {
        for symbol in [
            "__ct__3fooFiT1",
            "__vtbl__3foo__3bar",
            "__sti__bar__Fv",
            "foo__pt__3_ii",
            "foo__Fi_v",
            "foo__tm__5_PiPc",
            "Q2_3foo3bar__pt__2_i",
            "foo__XSP5_Fv",
            "foo__XT3barXSP6_Fv",
            "foo__F3barXSP5_i",
            "foo__XA10bar__3baz_Fv",
            "__ct__3fooXSP5_Fv",
            "foo__Fi_1_2",
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
    fn cfront_budgets_are_enforced() {
        let mut symbol = String::from("__ct__3fooF");
        symbol.push_str(&"P".repeat(2 * MAX_DEPTH as usize));
        symbol.push('i');
        let error = parse(&symbol).unwrap_err();
        assert_eq!(error.kind(), ParseErrorKind::TooDeep);

        let mut symbol = String::from("foo__pt__");
        let arguments = "i".repeat(2 * MAX_NODES as usize);
        symbol.push_str(&alloc::format!("{}", arguments.len() + 1));
        symbol.push('_');
        symbol.push_str(&arguments);
        let error = parse(&symbol).unwrap_err();
        assert_eq!(error.kind(), ParseErrorKind::TooManyNodes);
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
            "foo__FGi",
            "_GLOBAL_$I$foo",
        ] {
            assert!(parse(symbol).is_err(), "{symbol} should be rejected");
        }
    }

    #[test]
    fn rejects_unsupported_productions() {
        for symbol in ["foo__H3bar", "foo__K3bar", "foo__B0"] {
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
