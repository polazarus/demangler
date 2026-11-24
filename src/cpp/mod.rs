//! C++ ABI symbol parsing.
//!
//! This module exposes an immutable, inspectable syntax tree shared by the
//! Itanium and Microsoft C++ mangling schemes. It is independent of the text
//! backends: [`crate::Demangler`] parses C++ into this AST and prints it
//! in-tree.
//!
//! `parse` dispatches on the mangling prefix: `_Z` (and its underscore
//! variants) selects the Itanium parser, a name carrying LLVM's
//! `__alloc_token_` decoration is parsed as the Itanium name it decorates, `?`
//! selects the Microsoft parser, and everything else is tried as a legacy C++
//! name, which is accepted only when a legacy parser consumes the whole symbol.
//! `parse_msvc` parses a Microsoft name directly, including the `.?` RTTI type
//! descriptor name form.
//!
//! The Itanium, Microsoft and legacy printers render this AST as demangled
//! text. [`crate::Demangler`] uses them directly; `print_msvc` is also exposed
//! for callers that want Microsoft text from a parsed tree.
//!
//! The tree is `Rc`-owned and therefore `!Send + !Sync`. Nodes are immutable
//! after construction, children are sealed, and substitution references reuse
//! the same node handle, so the graph may be a DAG.
//!
//! # Supported grammar
//!
//! The parser covers the core Itanium productions:
//!
//! - prefixes `_Z`, `__Z`, `___Z`, `____Z`, plus LLVM's `__alloc_token_`
//!   instrumentation decoration;
//! - encodings, function parameter lists and data names;
//! - unscoped, nested, template-id and well-known substituted names, including
//!   friend function names and module (vendor qualifier) chains;
//! - source names, operator names, constructors/destructors and ABI tags;
//! - local names, lambdas and unnamed types;
//! - builtin, pointer, reference, rvalue-reference, cv-qualified, array,
//!   function, member-pointer, `decltype`, pack-index and vendor-qualified
//!   types;
//! - template arguments, template parameters and expression literals;
//! - C++20 requires-clauses on encodings, template heads and lambda
//!   signatures, and requires-expressions with their requirements;
//! - expression productions: calls, casts, `sizeof`/`alignof`, conversions,
//!   member access, initializer lists, fold and pack expansions, pack
//!   subscripts and pointer-to-member conversions;
//! - special names: vtables, VTTs, typeinfo, thunks, guard variables,
//!   reference temporaries and TLS wrappers/initializers;
//! - substitutions and back-references.
//!
//! Builtins include the single-letter codes plus parametrized `_BitInt`,
//! `_Float`, `bfloat16` and fixed-point types.
//!
//! The corpus gate in `tests/cpp_text_corpus.rs` requires every pair of the
//! vendored libc++abi corpus to reproduce the expected text. The corpus also
//! lists a few bare type manglings, which are not symbol names: `parse`
//! requires an encoding, so those are excluded there by name.
//!
//! Legacy GNU v2 names are covered by the `legacy` module: `__`-separated
//! function names, `Q<n>`-qualified names, operators, constructors and
//! destructors, templates, and the pointer/reference/array/function type forms.
//! The same module implements the ARM, EDG and HP aCC dialects, which share the
//! declarator grammar but spell special names, type back-references and
//! template arguments differently. See that module for the supported subsets
//! and the spelling they print.
//!
//! # Example
//!
//! ```rust
//! use demangle::cpp;
//!
//! assert_eq!(cpp::prefix_len("_Z1fv"), Some(2));
//! assert!(cpp::is_itanium("_ZN3Foo3barEv"));
//! ```

#[cfg(feature = "cpp-gnu-v2")]
pub(crate) mod legacy;
#[cfg(feature = "cpp-msvc")]
mod msvc;
#[cfg(feature = "cpp-msvc")]
mod msvc_printer;
mod node;
#[cfg(feature = "cpp-itanium")]
mod parser;
#[cfg(any(feature = "cpp-itanium", feature = "cpp-gnu-v2"))]
mod print_buffer;
#[cfg(feature = "cpp-itanium")]
mod printer;
mod text;

use alloc::rc::Rc;
use core::fmt;

pub(crate) use text::decode;
#[cfg(feature = "cpp-msvc")]
pub(crate) use text::owned_text;

pub use node::{Kind, Node, NodeRef};

#[cfg(feature = "cpp-msvc")]
pub use msvc::parse_msvc;
#[cfg(feature = "cpp-msvc")]
pub use msvc_printer::{PrintError, PrintErrorKind, print_msvc};

#[cfg(feature = "cpp-itanium")]
use crate::parse::{Limits, State};
#[cfg(feature = "cpp-itanium")]
use parser::Parser;

/// Maximum input length accepted by the C++ text and AST paths.
pub(crate) const MAX_INPUT_LEN: usize = 64 * 1024;

/// Length of a recognised Itanium mangling prefix.
///
/// The longest matching prefix wins: `____Z`, `___Z`, `__Z`, then `_Z`.
#[must_use]
pub fn prefix_len(symbol: &str) -> Option<usize> {
    prefix_len_bytes(symbol.as_bytes())
}

/// Length of a recognised Itanium mangling prefix in a byte string.
pub(crate) fn prefix_len_bytes(symbol: &[u8]) -> Option<usize> {
    if symbol.starts_with(b"____Z") {
        Some(5)
    } else if symbol.starts_with(b"___Z") {
        Some(4)
    } else if symbol.starts_with(b"__Z") {
        Some(3)
    } else if symbol.starts_with(b"_Z") {
        Some(2)
    } else {
        None
    }
}

/// True when `symbol` starts with an Itanium mangling prefix.
#[must_use]
pub fn is_itanium(symbol: &str) -> bool {
    prefix_len(symbol).is_some()
}

/// True when a byte string starts with an Itanium mangling prefix.
#[cfg(feature = "cpp-itanium")]
#[must_use]
pub(crate) fn is_itanium_bytes(symbol: &[u8]) -> bool {
    prefix_len_bytes(symbol).is_some()
}

/// True when `symbol` starts with a Microsoft C++ mangling prefix.
///
/// A Microsoft symbol starts with `?`. The RTTI type descriptor name form
/// starts with `.?` and is accepted by [`parse_msvc`] directly.
#[must_use]
pub fn is_msvc(symbol: &str) -> bool {
    symbol.starts_with('?') || symbol.starts_with(".?")
}

/// True when a byte string starts with a Microsoft C++ prefix.
#[cfg(feature = "cpp-msvc")]
#[must_use]
pub(crate) fn is_msvc_bytes(symbol: &[u8]) -> bool {
    symbol.starts_with(b"?") || symbol.starts_with(b".?")
}

/// The mangling scheme an [`Ast`] was parsed from.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Dialect {
    /// Itanium C++ ABI (`_Z...`).
    Itanium,
    /// Microsoft C++ (`?...`).
    Msvc,
    /// Legacy GNU v2 (`foo__Fiv`).
    GnuV2,
    /// ARM C++ (`__ct__3fooFv`, `__vtbl__3foo`, `__sti__foo`).
    Arm,
    /// EDG C++ (`__ct__3fooFv`, `foo__Fi_v`, `foo__tm__2_i`).
    Edg,
    /// HP aCC C++ (`foo__XSP5_Fv`, `foo__F3barXSP6`, `foo__Fi_1`).
    Hp,
}

/// A parsed C++ symbol with an immutable node graph.
///
/// The graph is `Rc`-owned, so `Ast` is `!Send + !Sync`. Clone the `Ast` to
/// share the graph without re-parsing.
#[derive(Clone)]
pub struct Ast<'a> {
    dialect: Dialect,
    root: NodeRef<'a>,
}

impl<'a> Ast<'a> {
    /// Wrap a parsed Itanium root node.
    #[cfg(feature = "cpp-itanium")]
    pub(crate) fn from_root(root: NodeRef<'a>) -> Self {
        Self {
            dialect: Dialect::Itanium,
            root,
        }
    }

    /// Wrap a parsed Microsoft root node.
    #[cfg(feature = "cpp-msvc")]
    pub(crate) fn from_root_msvc(root: NodeRef<'a>) -> Self {
        Self {
            dialect: Dialect::Msvc,
            root,
        }
    }

    /// Wrap a parsed legacy root node for `dialect`.
    ///
    /// `dialect` is one of [`Dialect::GnuV2`], [`Dialect::Arm`],
    /// [`Dialect::Edg`] or [`Dialect::Hp`]; the legacy parser is the only
    /// caller.
    #[cfg(feature = "cpp-gnu-v2")]
    pub(crate) fn from_root_legacy(root: NodeRef<'a>, dialect: Dialect) -> Self {
        debug_assert!(matches!(
            dialect,
            Dialect::GnuV2 | Dialect::Arm | Dialect::Edg | Dialect::Hp
        ));
        Self { dialect, root }
    }

    /// The mangling scheme this tree was parsed from.
    #[must_use]
    pub const fn dialect(&self) -> Dialect {
        self.dialect
    }

    /// The root [`Kind::Encoding`] node.
    #[must_use]
    pub fn root(&self) -> NodeRef<'a> {
        Rc::clone(&self.root)
    }
}

impl fmt::Debug for Ast<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Ast")
            .field("dialect", &self.dialect)
            .field("root", &self.root)
            .finish()
    }
}

impl fmt::Display for Ast<'_> {
    /// Render the parsed symbol as demangled C++ text.
    ///
    /// The text follows the LLVM demangler's conventions. Printing is bounded
    /// by depth, node-visit and output-byte budgets, so a symbol that parses
    /// but cannot be rendered is reported as an error.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.dialect {
            #[cfg(feature = "cpp-itanium")]
            Dialect::Itanium => printer::print(self, f),
            #[cfg(feature = "cpp-msvc")]
            Dialect::Msvc => print_msvc(self, f).map_err(|_| fmt::Error),
            #[cfg(feature = "cpp-gnu-v2")]
            Dialect::GnuV2 | Dialect::Arm | Dialect::Edg | Dialect::Hp => legacy::print(self, f),
            #[allow(unreachable_patterns)]
            _ => Err(fmt::Error),
        }
    }
}

/// Why a C++ symbol could not be parsed.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[non_exhaustive]
pub enum ParseErrorKind {
    /// The symbol does not start with a supported Itanium prefix.
    NotItanium,
    /// The symbol does not start with a supported Microsoft prefix.
    NotMsvc,
    /// The symbol is not a legacy C++ name.
    NotLegacy,
    /// The input ended in the middle of a production.
    UnexpectedEnd,
    /// A byte was not valid at this position.
    UnexpectedByte,
    /// A source-name length or sequence id was malformed or out of range.
    InvalidLength,
    /// A substitution referenced an entry that does not exist yet.
    UnknownSubstitution,
    /// The production is valid Itanium but not implemented yet.
    Unsupported,
    /// The nesting budget was exceeded.
    TooDeep,
    /// The node budget was exceeded.
    TooManyNodes,
    /// The work budget was exceeded.
    TooMuchWork,
    /// The symbol is too long for `u32` spans.
    TooLong,
}

impl fmt::Display for ParseErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::NotItanium => "not an Itanium mangled name",
            Self::NotMsvc => "not a Microsoft mangled name",
            Self::NotLegacy => "not a legacy C++ mangled name",
            Self::UnexpectedEnd => "unexpected end of input",
            Self::UnexpectedByte => "unexpected byte",
            Self::InvalidLength => "invalid length",
            Self::UnknownSubstitution => "unknown substitution",
            Self::Unsupported => "unsupported production",
            Self::TooDeep => "nesting budget exceeded",
            Self::TooManyNodes => "node budget exceeded",
            Self::TooMuchWork => "work budget exceeded",
            Self::TooLong => "symbol too long",
        };
        f.write_str(message)
    }
}

/// A parse failure with the byte offset where it was detected.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ParseError {
    position: usize,
    kind: ParseErrorKind,
}

impl ParseError {
    /// Create an error at `position`.
    pub(crate) const fn new(position: usize, kind: ParseErrorKind) -> Self {
        Self { position, kind }
    }

    /// Byte offset in the input where parsing failed.
    #[must_use]
    pub const fn position(&self) -> usize {
        self.position
    }

    /// What went wrong.
    #[must_use]
    pub const fn kind(&self) -> ParseErrorKind {
        self.kind
    }
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} at byte {}", self.kind, self.position)
    }
}

impl core::error::Error for ParseError {}

/// True when `symbol` carries LLVM's `__alloc_token_` decoration.
#[cfg(feature = "cpp-itanium")]
#[must_use]
pub(crate) fn is_alloc_token_bytes(symbol: &[u8]) -> bool {
    alloc_token_name_offset_bytes(symbol).is_some()
}

/// Parse a C++ mangled name into an inspectable AST.
///
/// Itanium names (a `_Z` prefix, including the underscored variants) use the
/// Itanium parser; a name carrying LLVM's `__alloc_token_` decoration is parsed
/// as the Itanium name it decorates; names starting with `?` use the Microsoft
/// parser; anything else is parsed as a legacy C++ name and accepted only when
/// one of the legacy parsers consumes the whole symbol. The legacy dialects are
/// tried in priority order — GNU v2 first, then ARM, EDG and HP aCC — except
/// for the reserved cfront special-name markers, which select those dialects
/// first; the `cpp::legacy` module documents the selection rules in full.
///
/// # Errors
///
/// Returns [`ParseError`] when the symbol is not a recognised C++ mangling,
/// uses an unsupported production, is malformed, or exceeds the parser's
/// depth, node or work budgets.
pub fn parse(symbol: &str) -> Result<Ast<'_>, ParseError> {
    parse_bytes(symbol.as_bytes())
}

/// Parse a C++ mangled name from raw bytes.
///
/// This is the byte-native entry point: it performs the same dialect dispatch
/// as [`parse`] but never requires the symbol to be UTF-8. Identifier bytes
/// that are not valid UTF-8 are escaped at node construction (see
/// [`text`]), so every byte of the input is represented in the tree.
pub(crate) fn parse_bytes(symbol: &[u8]) -> Result<Ast<'_>, ParseError> {
    #[cfg(feature = "cpp-itanium")]
    if let Some(prefix) = prefix_len_bytes(symbol) {
        let mut state = itanium_state(symbol.len());
        return Parser::new(symbol, prefix, &mut state)?.parse();
    }
    #[cfg(feature = "cpp-itanium")]
    if let Some(offset) = alloc_token_name_offset_bytes(symbol) {
        // LLVM's allocation-token instrumentation prefixes the mangled name.
        let prefix = symbol
            .get(offset..)
            .and_then(prefix_len_bytes)
            .ok_or(ParseError::new(0, ParseErrorKind::NotItanium))?;
        let mut state = itanium_state(symbol.len());
        return Parser::new_alloc_token(symbol, offset + prefix, &mut state)?.parse();
    }
    #[cfg(feature = "cpp-msvc")]
    if symbol.starts_with(b"?") {
        return msvc::parse_msvc_bytes(symbol);
    }
    #[cfg(feature = "cpp-gnu-v2")]
    if legacy::looks_like_legacy_bytes(symbol) {
        return legacy::parse_bytes(symbol);
    }
    Err(ParseError::new(0, ParseErrorKind::NotItanium))
}

/// Parser budget for the Itanium parser.
///
/// The parser keeps its own depth, node and work accounting, so the cursor's
/// counters are only used to bound the input length.
#[cfg(feature = "cpp-itanium")]
fn itanium_state(input_len: usize) -> State {
    State::new(Limits {
        input: u32::try_from(input_len).unwrap_or(u32::MAX),
        depth: u16::MAX,
        work: u32::MAX,
    })
}

/// Byte offset of the mangled name in a symbol that carries LLVM's
/// allocation-token decoration.
///
/// The instrumentation pass spells the decoration `__alloc_token_`, optionally
/// followed by the token number and a `_` separator. It is not part of the
/// mangling.
#[cfg(feature = "cpp-itanium")]
fn alloc_token_name_offset_bytes(symbol: &[u8]) -> Option<usize> {
    const PREFIX: &[u8] = b"__alloc_token_";
    let rest = symbol.strip_prefix(PREFIX)?;
    let digits = rest.iter().take_while(|byte| byte.is_ascii_digit()).count();
    let mut offset = PREFIX.len() + digits;
    if digits > 0 {
        if symbol.get(offset) != Some(&b'_') {
            return None;
        }
        offset += 1;
    }
    Some(offset)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognises_prefixes() {
        assert_eq!(prefix_len("_Z1fv"), Some(2));
        assert_eq!(prefix_len("__Z1fv"), Some(3));
        assert_eq!(prefix_len("___Z1fv"), Some(4));
        assert_eq!(prefix_len("____Z1fv"), Some(5));
        assert_eq!(prefix_len("_Z"), Some(2));
        assert_eq!(prefix_len("main"), None);
        assert!(is_itanium("_ZN3Foo3barEv"));
        assert!(!is_itanium("$s4main3foo"));
    }

    #[test]
    fn rejects_non_itanium() {
        let error = parse("main").unwrap_err();
        assert_eq!(error.kind(), ParseErrorKind::NotItanium);
        assert_eq!(error.position(), 0);
    }

    #[cfg(feature = "cpp-itanium")]
    #[test]
    fn root_is_an_encoding() {
        let ast = parse("_Z1A").unwrap();
        assert_eq!(ast.root().kind(), Kind::Encoding);
        assert_eq!(ast.root().child_count(), 1);
        assert_eq!(ast.root().child(0).unwrap().kind(), Kind::Data);
    }

    #[cfg(feature = "cpp-itanium")]
    #[test]
    fn ast_clone_shares_the_root() {
        let ast = parse("_Z1fv").unwrap();
        let cloned = ast.clone();
        assert!(Rc::ptr_eq(&ast.root(), &cloned.root()));
    }

    #[cfg(feature = "cpp-itanium")]
    #[test]
    fn ast_debug_is_bounded() {
        let ast = parse("_Z1fPPPPi").unwrap();
        let rendered = alloc::format!("{ast:?}");
        assert!(rendered.contains("Ast"));
        assert!(rendered.contains("Encoding"));
        assert!(!rendered.contains("BuiltinType"));
    }

    /// Collect the text of every node carrying `kind`, bounded by the node
    /// budget so a shared DAG cannot loop.
    #[cfg(any(feature = "cpp-itanium", feature = "cpp-msvc", feature = "cpp-gnu-v2"))]
    fn texts_of(ast: &Ast<'_>, kind: Kind) -> alloc::vec::Vec<alloc::string::String> {
        let mut out = alloc::vec::Vec::new();
        let mut stack = alloc::vec![ast.root()];
        let mut visits = 0_usize;
        while let Some(node) = stack.pop() {
            visits += 1;
            assert!(visits < 1 << 16, "node walk is bounded");
            if node.kind() == kind
                && let Some(text) = node.text()
            {
                out.push(alloc::string::String::from(text));
            }
            stack.extend(node.children().iter().cloned());
        }
        out
    }

    #[cfg(feature = "cpp-itanium")]
    #[test]
    fn byte_path_accepts_invalid_utf8_in_source_names() {
        // The source name is length-prefixed by bytes, so the `\xff` byte is
        // part of the identifier even though it is not UTF-8.
        let ast = parse_bytes(b"_ZN4F\xffoo3barEv").unwrap();
        let texts = texts_of(&ast, Kind::Identifier);
        assert!(texts.iter().any(|text| text == "F\\xffoo"), "{texts:?}");
        assert!(texts.iter().any(|text| text == "bar"), "{texts:?}");

        // Spans address the supplied bytes.
        let mut stack = alloc::vec![ast.root()];
        let mut found = None;
        while let Some(node) = stack.pop() {
            if node.text() == Some("F\\xffoo") {
                found = node.span();
                break;
            }
            stack.extend(node.children().iter().cloned());
        }
        assert_eq!(found, Some(3..8));
    }

    #[cfg(feature = "cpp-gnu-v2")]
    #[test]
    fn byte_path_accepts_invalid_utf8_in_legacy_names() {
        let ast = parse_bytes(b"f\xffoo__Fiv").unwrap();
        assert_eq!(ast.dialect(), Dialect::GnuV2);
        let texts = texts_of(&ast, Kind::Identifier);
        assert!(texts.iter().any(|text| text == "f\\xffoo"), "{texts:?}");
    }

    #[cfg(feature = "cpp-msvc")]
    #[test]
    fn byte_path_accepts_invalid_utf8_in_msvc_names() {
        let ast = parse_bytes(b"?F\xffoo@@YAXXZ").unwrap();
        assert_eq!(ast.dialect(), Dialect::Msvc);
        let texts = texts_of(&ast, Kind::Identifier);
        assert!(texts.iter().any(|text| text == "F\\xffoo"), "{texts:?}");
    }

    #[cfg(feature = "cpp-itanium")]
    #[test]
    fn byte_path_keeps_valid_text_unchanged() {
        for ast in [
            parse_bytes(b"_ZN3Foo3barEv").unwrap(),
            parse("_ZN3Foo3barEv").unwrap(),
        ] {
            let texts = texts_of(&ast, Kind::Identifier);
            assert!(texts.iter().any(|text| text == "Foo"), "{texts:?}");
            assert!(texts.iter().any(|text| text == "bar"), "{texts:?}");
        }
    }
}
