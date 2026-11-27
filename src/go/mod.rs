//! Pure-Rust Go symbol name demangling.
//!
//! Go does not mangle names in the C++ sense: the compiler emits
//! `package.Local` names and decorates them with compiler-generated
//! components. This module classifies those names, decodes the `%XX` package
//! path escapes that `cmd/internal/objabi.PathToPrefix` introduces, normalises
//! the `·` separator to `.`, and strips ABI markers (`.abi0`, `.abiinternal`,
//! `.abiwrap`), which are linker plumbing rather than part of the name.
//!
//! No external demangler is used and there is no runtime dependency.
//!
//! # Supported forms
//!
//! - package/function/method names: `fmt.Println`, `fmt.(*pp).doPrintln`,
//!   including methods promoted from another package
//!   (`(*Template).text/template/parse.action`);
//! - closures and wrappers: `.funcN`, `-rangeN`, `.gowrapN`, `.deferwrapN`,
//!   `-fm`, `·f`, nested closure indices, inlined function names and inlined
//!   methods such as `(*mheap).freeSpan`;
//! - generic instantiations and compiler shape types: `Sum[go.shape.int]`,
//!   `go.shape.struct { … }`, `..dict.T[int]`;
//! - runtime type descriptors: `type:`, `type:*`, `type:.eq.`, `type:.eqfunc`,
//!   `type:.hash.`, `type:.hashfunc`, `type:.gcprog.`, `type:.namedata.`,
//!   `type:.importpath.`, `type:noalg.`;
//! - linker specials: `go:itab.<T>,<I>`, `go:string.<payload>`, `go:info.<T>`,
//!   `go:buildid`, `go:func.*` and the other `go:` group symbols;
//! - compiler-generated data symbols: `..inittask`, `..interfaceSwitch.N`,
//!   `..typeAssert.N`, `..gobytes.N`, `..stmp_N`, `glob..funcN`,
//!   `gcbits.<hex>`, `gclocals.<base64>`;
//! - compiler-generated function suffixes: `.jumpN` jump tables, `.arginfoN`,
//!   `.stkobj` and the other internal suffixes;
//! - compiler constants: `$f32.`, `$f64.`, `$i32.`, `$i64.`, `$i128.`.
//!
//! Object files and export archives also carry a `<N>` symbol version that
//! `go tool nm` appends to static symbols; it is kept in the printed name and
//! exposed as [`crate::go::Ast::version`].
//!
//! # Normalisation
//!
//! The printed form is the name as a Go programmer writes it: the package
//! path is unescaped and `·` becomes `.`. Unescaping is lossy for one corner:
//! `%2e` inside the last package path segment becomes `.`, which is
//! indistinguishable from the package/local separator, so the printed text is
//! meant for humans rather than for re-parsing. [`crate::go::Package::decoded`] exposes
//! the same decoding structurally.
//!
//! `reflect.name` blobs (`type:.namedata.`) and `go:string.` payloads are
//! opaque: struct tags and string data may contain `%` and arbitrary bytes, so
//! they are printed verbatim.
//!
//! # Raw bytes
//!
//! A symbol name is a byte string, so the parser matches the grammar on bytes
//! and never requires the whole name to be UTF-8. Text that is valid UTF-8
//! prints verbatim (so `·` and Unicode type strings survive); bytes that are
//! not valid UTF-8 print escaped as `\xNN`, including bytes produced by a
//! `%XX` escape. A backslash in identifier text prints as `\\` so the
//! spelling stays reversible.
//!
//! # Budgets
//!
//! Untrusted input stays bounded by `MAX_INPUT_LEN`, `MAX_DEPTH` and
//! `MAX_WORK`. Exceeding a budget is an error, never a
//! partial result. Printed text is never longer than four times the input,
//! because every byte that is not valid UTF-8 costs four bytes as `\xNN`, so
//! the input budget also bounds printing.
//!
//! The input, depth and work limits are runtime-configurable. [`parse`](crate::go::parse)
//! keeps the compiled-in defaults, and a [`crate::Demangler`] passes the
//! configured budget, clamped to the hard ceilings of 16 MiB input, 1024
//! nesting levels and 64M work units. The three dimensions are independent:
//! raising the input limit alone keeps the default work allowance, so a caller
//! that raises the input for a long symbol must raise
//! [`crate::Budget::work`] as well when the scan exceeds it. A Go name is one
//! classified symbol, so there is no node dimension to configure; every
//! closure or wrapper suffix consumes input, so the input budget bounds the
//! suffix list.
//!
//! Printing is not a budget: the printer walks each node once, so the output is
//! linear in the parsed name and needs no cap of its own. The four-times-input
//! bound above is a property of the escaping, not a configurable limit.
//!
//! # Gaps
//!
//! - `.llvm.` suffixes are not Go and are left to the other backends.
//! - `%XX` escapes that do not decode to valid UTF-8 print as `\xNN` escapes.
//! - ABI markers are reported by [`crate::go::Ast::abi`] but not printed.
//! - Bare assembly and cgo names (`aeshashbody`, `_cgo_init`) carry no package
//!   and are indistinguishable from C, so detection leaves them out of Go.
//!
mod node;
mod parser;
mod printer;
mod text;

use crate::error::BudgetKind;
use core::fmt;

use crate::budget::{EffectiveBudget, Limit};
use node::ParseLimits;

pub use node::{
    Abi, Ast, Constant, ConstantClass, DataName, Function, Local, Name, Package, Receiver, Special,
    Suffix, Symbol, TypeDescriptor, TypeKind,
};
pub(crate) use node::{MAX_DEPTH, MAX_INPUT_LEN, MAX_WORK};

/// Why a symbol could not be parsed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ParseError {
    /// The symbol does not match a known Go name form.
    Invalid,
    /// The symbol nests `()`, `[]` or `{}` deeper than `MAX_DEPTH`.
    RecursedTooDeep,
    /// The symbol exceeds the input, work or suffix budget.
    BudgetExceeded,
}

impl ParseError {
    /// The parser budget dimension this failure belongs to, when it is a
    /// capacity failure rather than a malformed symbol.
    #[must_use]
    pub const fn budget_kind(&self) -> Option<BudgetKind> {
        match self {
            Self::RecursedTooDeep => Some(BudgetKind::ParseDepth),
            Self::BudgetExceeded => Some(BudgetKind::Other),
            _ => None,
        }
    }
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Invalid => "invalid syntax",
            Self::RecursedTooDeep => "recursion limit reached",
            Self::BudgetExceeded => "size limit reached",
        })
    }
}

impl core::error::Error for ParseError {}

impl fmt::Display for Ast<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.print(f)
    }
}

impl Ast<'_> {
    /// Render the parsed name into `out`.
    pub(crate) fn print<W: fmt::Write + ?Sized>(&self, out: &mut W) -> fmt::Result {
        printer::print(&self.name, out)?;
        printer::print_version(self.version, out)
    }
}

/// Parse a Go symbol name into a reusable [`Ast`].
///
/// # Errors
///
/// Returns [`ParseError::Invalid`] when the symbol matches no known Go name
/// form, [`ParseError::RecursedTooDeep`] when a receiver or generic argument
/// list nests too deeply, and [`ParseError::BudgetExceeded`] when the input,
/// work or suffix budget is exceeded.
pub fn parse(symbol: &str) -> Result<Ast<'_>, ParseError> {
    parse_bytes(symbol.as_bytes())
}

/// Parse a Go symbol name from raw bytes.
///
/// This is the entry point the other backends and the demangler dispatch to
/// once symbol bytes, not text, are the input. See [`parse`] for the accepted
/// forms; the only difference is that the name does not have to be UTF-8.
pub(crate) fn parse_bytes(symbol: &[u8]) -> Result<Ast<'_>, ParseError> {
    parse_bytes_with_budget(symbol, DEFAULT_BUDGET)
}

/// The budget the public entry point starts from.
///
/// [`parse`] uses it, so it keeps exactly the compiled-in limits; a
/// [`crate::Demangler`] resolves its configured [`crate::Budget`] for
/// [`Scheme::Go`](crate::Scheme::Go) instead.
pub(crate) const DEFAULT_BUDGET: EffectiveBudget = EffectiveBudget {
    input_bytes: Some(Limit::Bounded(MAX_INPUT_LEN)),
    parse_depth: Some(Limit::Bounded(MAX_DEPTH)),
    work: Some(Limit::Bounded(MAX_WORK)),
    nodes: Some(Limit::Unbounded),
    expansion_visits: None,
    expansion_bytes: None,
};

/// Parse a Go symbol name with one scheme's resolved budget.
///
/// [`parse_bytes`] wraps this with [`DEFAULT_BUDGET`]; a [`crate::Demangler`]
/// passes [`Demangler::effective_budget`](crate::Demangler::effective_budget)
/// for [`Scheme::Go`](crate::Scheme::Go), so a caller's overrides reach the
/// parser. The input, depth and work values are clamped to this module's hard
/// ceilings, and the grammar has no node dimension to resolve.
pub(crate) fn parse_bytes_with_budget(
    symbol: &[u8],
    budget: EffectiveBudget,
) -> Result<Ast<'_>, ParseError> {
    parser::parse_bytes(symbol, ParseLimits::from_effective(budget))
}

/// True when `name` has the shape of a Go linker symbol name.
///
/// This is the cheap prefilter used by [`crate::Scheme::detect`]; it validates
/// the shape but not every name form, so [`parse`] may still reject a name
/// that this accepts.
///
/// Detection is a byte test: only ASCII whitespace and control bytes are
/// rejected directly, so a valid Unicode identifier is never refused merely
/// because a byte is `>= 0x80`.
///
/// The rule is:
///
/// - explicit prefixes `type:`, `go:` and `go.shape.` are Go;
/// - `$f32.`/`$f64.`/`$i32.`/`$i64.`/`$i128.` compiler constants are Go;
/// - names containing `·`, Go's identifier separator, are Go;
/// - otherwise the name must be dotted: the part before the package separator
///   must be a non-empty escaped import path and the local part must start
///   with an identifier, `(`, `[` or `.`;
/// - whitespace is only allowed in type strings (`go.shape.`, `..dict.`, or
///   inside a generic argument list);
/// - a `(` is only valid as a receiver opener (at the start of the local part
///   or after a `.`) or inside a bracketed type string;
/// - names handled by another scheme (`?`, `_Z`, `_RN`, `$`, `.llvm.`) are
///   never Go.
#[must_use]
pub fn is_go(name: &str) -> bool {
    is_go_bytes(name.as_bytes())
}

/// True when `name` has the shape of a Go linker symbol name, as raw bytes.
///
/// Detection is a byte test: only ASCII whitespace and control bytes are
/// rejected directly, so a valid Unicode identifier is never refused merely
/// because a byte is `>= 0x80`.
pub(crate) fn is_go_bytes(name: &[u8]) -> bool {
    /// The UTF-8 encoding of `·`, Go's identifier separator.
    const MIDDLE_DOT: &[u8] = "·".as_bytes();

    if name.is_empty() || name.len() > MAX_INPUT_LEN {
        return false;
    }
    if name.starts_with(b"type:") || name.starts_with(b"go:") || name.starts_with(b"go.shape.") {
        return true;
    }
    if name.starts_with(b"$f32.")
        || name.starts_with(b"$f64.")
        || name.starts_with(b"$i32.")
        || name.starts_with(b"$i64.")
        || name.starts_with(b"$i128.")
    {
        return true;
    }
    if name.starts_with(b"?")
        || name.starts_with(b"$")
        || name.starts_with(b"_Z")
        || name.starts_with(b"_RN")
        || contains(name, b".llvm.")
    {
        return false;
    }
    if name.iter().any(u8::is_ascii_control) {
        return false;
    }

    // A Go name is either explicitly prefixed (handled above) or carries a
    // package separator: `.` for dotted paths or `·` for assembly names.
    // Rejecting everything else here is the cheap prefilter for the common
    // plain-C symbol (`memcpy`, `_start`, ...).
    if !name.contains(&b'.') && !contains(name, MIDDLE_DOT) {
        return false;
    }

    // `·` is Go's identifier separator, used by assembly and by the compiler
    // for `gclocals·<hash>` and `pkg.func·f`.
    if contains(name, MIDDLE_DOT) && name.iter().any(u8::is_ascii_alphanumeric) {
        return true;
    }

    let (package, local) = parser::split_package(name);
    if package.is_empty() || local.is_empty() {
        return false;
    }

    // Type strings inside generic arguments and dictionaries can contain
    // spaces (`interface {}`, `struct { x int }`); nothing else can. Only ASCII
    // whitespace is a separator here, so Unicode identifiers pass through.
    if name.iter().any(u8::is_ascii_whitespace)
        && !(contains(name, b"go.shape.") || contains(name, b"..dict.") || local.contains(&b'['))
    {
        return false;
    }
    if !matches!(
        local.first(),
        Some(b'A'..=b'Z' | b'a'..=b'z' | b'_' | b'(' | b'[' | b'.')
    ) {
        return false;
    }

    // A `(` is either the receiver opener at the start of the local part or
    // part of a bracketed type string such as `F[go.shape.func()]`. Anything
    // else (`foo.bar(baz)`) is not a Go name.
    let mut depth = 0_usize;
    for (index, byte) in local.iter().enumerate() {
        match byte {
            b'(' if depth == 0 && index != 0 && local[index - 1] != b'.' => {
                return false;
            }
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => {
                if depth == 0 {
                    return false;
                }
                depth -= 1;
            }
            _ => {}
        }
    }
    true
}

/// True when `haystack` contains `needle`.
fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    crate::parse::find(haystack, needle).is_some()
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod budget_tests;
