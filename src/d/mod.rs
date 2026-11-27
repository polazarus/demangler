//! Pure-Rust D ABI symbol demangling.
//!
//! D mangles a symbol as `_D QualifiedName Type` (or `_D QualifiedName Z` for
//! internal symbols) so that functions, variables, classes and templates are
//! link-time type safe. This backend parses that grammar on bytes and prints
//! the same spelling as `c++filt -s dlang`.
//!
//! No external demangler is used and there is no runtime dependency.
//!
//! # Supported forms
//!
//! - qualified names and nested functions
//!   (`_D3app5outerFiZ5innerMFNaNbNiNfiZi` -> `app.outer(int).inner(int)`);
//! - primitives, `const`/`immutable`/`shared`/`inout`/`__vector`, pointers,
//!   dynamic, static and associative arrays, function and delegate types,
//!   class/struct/enum/typedef/identifier types, tuples and `typeof(*null)`;
//! - function parameters (`in`, `out`, `ref`, `lazy`, and the `scope`/`return`
//!   markers in either encoded order), function attributes and calling
//!   conventions;
//! - templates `__T`/`__U`, including nested instances, type, value, symbol
//!   and externally mangled arguments, specialized (`H`) arguments and
//!   template symbol parameters;
//! - identifier and type back references (`Q` + base-26), including the
//!   back-referenced `M Q<ref>` function type of a nested function;
//! - value literals: integers, characters, booleans, reals, complex numbers,
//!   strings, arrays, associative arrays, struct literals and function
//!   symbols;
//! - compiler specials: `_Dmain`, `__ctor`, `__dtor`, `__postblitMFZ`,
//!   `__initZ`, `__vtblZ`, `__ClassZ`, `__InterfaceZ`, `__ModuleInfoZ` and
//!   `__moduleRefZ`;
//! - `TypeInfo_*` names, which the ABI encodes as ordinary identifiers.
//!
//! A Mach-O leading underscore (`__D...`) is accepted as well as `_D...`. An
//! LDC this-adjusting thunk (`_DThn<offset>_` followed by the symbol it thunks
//! to) is accepted and prints the wrapped name; see `refs/d.md` for the
//! provenance of that non-ABI wrapper.
//!
//! # Canonical output
//!
//! The printed text matches `c++filt -s dlang` for the supported grammar. A
//! function's encoded return type is not printed at symbol level, because the
//! parameter list is the useful signature; a function *type* prints its return
//! type (`int(int) function`). Names are decoded as text: valid UTF-8 is
//! written verbatim, invalid bytes as `\xNN`, and a backslash in identifier
//! text as `\\`.
//!
//! # Detection
//!
//! `is_d` parses the whole symbol, so `_D` followed by malformed or
//! unsupported syntax is not claimed. This keeps plain C identifiers that
//! merely start with `_D` out of the D backend.
//!
//! # Budgets
//!
//! Untrusted input stays bounded by `MAX_INPUT_LEN`,
//! `MAX_DEPTH`, `MAX_WORK`,
//! `MAX_NODES` and
//! the parser limits. Back references re-scan
//! earlier input, so work is metered separately from input length. Exceeding a
//! budget is an error, never a partial result.
//!
//! The input, depth, work and node limits are runtime-configurable.
//! [`parse`](crate::d::parse) keeps the compiled-in defaults, and a
//! [`crate::Demangler`] passes the configured budget, clamped to the hard
//! ceilings of 16 MiB input, 1024 nesting levels, 64M work units and 1M
//! nodes. The four dimensions are independent: raising the input limit alone
//! keeps the default work allowance and the default node allowance, so a caller
//! that raises the input for a long symbol must raise [`crate::Budget::work`]
//! and [`crate::Budget::nodes`] as well when the parse needs more than they
//! allow.
//!
//! Printing is not a budget. The printer walks each owned node once and has no
//! output cap. The spelling is not linear in the input — an `immutable`
//! modifier prints ten bytes and a back reference re-parses an earlier part of
//! the symbol — so the parse charges back-reference expansion to the
//! `expansion_visits` and `expansion_bytes` gates instead.
//!
//! # Gaps
//!
//! - `X` externally mangled template arguments are printed verbatim; their
//!   payload is not demangled.
//! - Anonymous symbols (`0` components) are skipped; a symbol made only of
//!   anonymous components is rejected.
//! - A compiler special that prepends a phrase (`initializer for `, …) is only
//!   accepted as the final qualified-name component.
//! - The D ABI has no canonical demangler; `c++filt` is the generation-time
//!   oracle and its echo/placeholder output is recorded as an oracle gap in
//!   `refs/d.md`, not as coverage. The pinned corpus has no remaining gaps.

mod node;
mod parser;
mod printer;

use crate::error::BudgetKind;
use core::fmt;

use crate::budget::{EffectiveBudget, Limit};
use node::ParseLimits;
use node::Root;
pub(crate) use node::{MAX_DEPTH, MAX_INPUT_LEN, MAX_NODES, MAX_WORK};

/// Why a D symbol could not be parsed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ParseError {
    /// The symbol does not match the D ABI grammar.
    Invalid,
    /// A type, qualified name or template nests deeper than `MAX_DEPTH`.
    RecursedTooDeep,
    /// The input, work or node budget was exceeded.
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

/// A parsed D symbol.
#[derive(Clone, Debug)]
pub struct Ast<'a> {
    root: Root<'a>,
}

impl fmt::Display for Ast<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.print(f)
    }
}

impl Ast<'_> {
    /// Render the parsed symbol.
    pub(crate) fn print<W: fmt::Write + ?Sized>(&self, out: &mut W) -> fmt::Result {
        printer::print(&self.root, out)
    }
}

impl Ast<'_> {
    /// The scheme this tree was parsed from: always [`crate::Scheme::D`].
    #[must_use]
    pub const fn scheme(&self) -> crate::Scheme {
        crate::Scheme::D
    }
}

/// Parse a D symbol from UTF-8.
///
/// # Errors
///
/// Returns [`ParseError::Invalid`] when the symbol does not match the D ABI
/// grammar, [`ParseError::RecursedTooDeep`] when nesting exceeds
/// `MAX_DEPTH`, and [`ParseError::BudgetExceeded`] when an input, work or
/// node budget is exceeded.
pub fn parse(symbol: &str) -> Result<Ast<'_>, ParseError> {
    parse_bytes(symbol.as_bytes())
}

/// Parse a D symbol from raw bytes.
///
/// This is the entry point the demangler dispatches to once symbol bytes, not
/// text, are the input. See [`parse`] for the accepted forms; the only
/// difference is that the symbol does not have to be UTF-8.
///
/// # Errors
///
/// See [`parse`].
pub(crate) fn parse_bytes(symbol: &[u8]) -> Result<Ast<'_>, ParseError> {
    parse_bytes_with_budget(symbol, DEFAULT_BUDGET)
}

/// The budget the public entry point starts from.
///
/// [`parse`] uses it, so it keeps exactly the compiled-in limits; a
/// [`crate::Demangler`] resolves its configured [`crate::Budget`] for
/// [`Scheme::D`](crate::Scheme::D) instead.
pub(crate) const DEFAULT_BUDGET: EffectiveBudget = EffectiveBudget {
    input_bytes: Some(Limit::Bounded(MAX_INPUT_LEN)),
    parse_depth: Some(Limit::Bounded(MAX_DEPTH)),
    work: Some(Limit::Bounded(MAX_WORK)),
    nodes: Some(Limit::Bounded(MAX_NODES)),
    // The policy-table expansion gates; no parser reads them yet.
    expansion_visits: Some(Limit::Bounded(1 << 18)),
    expansion_bytes: Some(Limit::Bounded(1 << 18)),
};

/// Parse a D symbol with one scheme's resolved budget.
///
/// The public [`parse`] wraps this with [`DEFAULT_BUDGET`]; a
/// [`crate::Demangler`] passes
/// [`Demangler::effective_budget`](crate::Demangler::effective_budget) for
/// [`Scheme::D`](crate::Scheme::D), so a caller's overrides reach the parser.
/// Every dimension is clamped to this module's hard ceiling and the input,
/// depth, work and node limits are independent of each other.
pub(crate) fn parse_bytes_with_budget(
    symbol: &[u8],
    budget: EffectiveBudget,
) -> Result<Ast<'_>, ParseError> {
    parser::parse_bytes(
        crate::scheme::strip_decorations_bytes(symbol),
        ParseLimits::from_effective(budget),
    )
}

/// True when `symbol` is a complete D ABI symbol.
///
/// This parses the whole symbol, so a bare `_D` prefix, a truncated name or a
/// grammar production this backend rejects is not claimed.
///
/// Detection classifies with the configured budget when it can; a
/// [`crate::Demangler`] re-parses the retained bytes with the configured
/// budget instead of reusing this tree.
#[must_use]
pub fn is_d(symbol: &[u8]) -> bool {
    parse_bytes(symbol).is_ok()
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod budget_tests;
