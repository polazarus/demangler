//! Pure-Rust Kotlin/Native symbol demangling.
//!
//! Kotlin/Native is an LLVM backend with its own linker-symbol mangling,
//! unrelated to Kotlin/JVM descriptors and Kotlin/JS names. The compiler's
//! `KonanMangler` builds a declaration spelling from a qualified path, a `#`
//! member separator, value and type parameter lists, a return type and linkage
//! markers; the LLVM backend wraps it in a `k*:` prefix.
//!
//! No external demangler is used and there is no runtime dependency.
//!
//! # Supported forms
//!
//! - `kfun:` functions, constructors, property accessors, local functions,
//!   lambdas and compiler bridges;
//! - `ktype:` type symbols;
//! - `kclass:` classes, interfaces and objects;
//! - `kproperty:`, `kobj:`, `kfield:`, `ktypealias:`, `kvar:`;
//! - `kpackage:` and `knamespace:` package/namespace symbols;
//! - Mach-O leading-underscore variants (`_kfun:`, `__kfun:`);
//! - extension receivers (`@` in klib spellings, `__at__` in ELF symbols),
//!   `#static`, `#suspend`, `#companion`, `#internal`, `-impl`, `$` local and
//!   synthetic indexes, and `%` platform-specific suffixes;
//! - type parameters, bounds, type arguments, variance, nullable, star,
//!   vararg, flexible, `<dynamic>` and `<ERROR CLASS>` types.
//!
//! # Canonical output
//!
//! There is no standard Kotlin/Native demangler, so the printed text is this
//! crate's contract:
//!
//! | Input | Output |
//! |---|---|
//! | `kfun:com.example.foo#bar(kotlin.Int){}` | `com.example.foo.bar(kotlin.Int)` |
//! | `kfun:com.example.Box#<init>(kotlin.Int){}` | `com.example.Box.<init>(kotlin.Int)` |
//! | `kfun:com.example.Box#get(){}` | `com.example.Box.get()` |
//! | `kfun:com.example#topLevel(){}` | `com.example.topLevel()` |
//! | `ktype:com.example.Box<kotlin.Int>` | `com.example.Box<kotlin.Int>` |
//!
//! The spelling is a debugger name, not re-parsable Kotlin source:
//!
//! - `#` becomes `.`;
//! - `#static` and `#suspend` become `static ` and `suspend ` prefixes;
//! - `#internal` and `-impl` are linkage markers and are not printed;
//! - an extension receiver is printed as `name@receiver(...)`;
//! - type parameters are declared as `T<index>` and referenced as
//!   `T<container>:<index>`;
//! - variance is printed as `out ` / `in `, vararg as `vararg `, nullable as
//!   `?`, star as `*` and flexible types as `lower..upper`;
//! - a return type is appended as `: <type>`;
//! - a `%` platform suffix is appended as ` %<suffix>`.
//!
//! Unknown prefixes and unknown `#` markers are rejected, never guessed.
//!
//! # Raw bytes
//!
//! A symbol name is a byte string, so the parser matches the grammar on bytes
//! and never requires the whole name to be UTF-8. Text that is valid UTF-8
//! prints verbatim (so Unicode identifiers survive); bytes that are not valid
//! UTF-8 print escaped as `\xNN`. A backslash in a name prints as `\\`, and a
//! `%` platform suffix keeps its payload as written.
//!
//! # Detection
//!
//! [`crate::Scheme::detect`] claims only the known `k*:` prefixes (and their
//! leading-underscore variants) and tries them before Go and plain C. It does
//! not claim arbitrary `k...` names, and C++/Rust/Swift/Objective-C keep their
//! existing precedence.
//!
//! # Budgets
//!
//! Untrusted input stays bounded by `MAX_INPUT_LEN`,
//! `MAX_DEPTH` and
//! `MAX_WORK`. Exceeding a budget is an
//! error, never a partial result.
//!
//! The input, depth and work limits are runtime-configurable.
//! [`parse`](crate::kotlin_native::parse) keeps the compiled-in defaults, and a
//! [`crate::Demangler`] passes the configured budget, clamped to the hard
//! ceilings of 16 MiB input, 1024 nesting levels and 64M work units. Input and
//! work are independent: raising the input limit alone keeps the default work
//! allowance, so a caller that raises the input for a long symbol must raise
//! [`crate::Budget::work`] as well when the scan exceeds it. Every path
//! segment, parameter, type argument and type parameter consumes input, so the
//! input budget bounds those lists and there is no node dimension to
//! configure.
//!
//! Printing is not a budget: the printer walks each node once and the output
//! is linear in the parsed symbol, so `Display` is total except for a failing
//! sink.
//!
//! # Gaps
//!
//! - Kotlin/JVM descriptors, Kotlin/JS names and Kotlin metadata blobs are out
//!   of scope.
//! - Kotlin/Native internal table symbols (`kintf:`, `kifacetable:`,
//!   `kifacevtable:`, `kexttype:`, `kextoff:`, `kextname:`, `krefs:`) are not
//!   declarations and are left unclaimed.
//! - The `%` platform suffix is preserved verbatim; its ObjC-export payload is
//!   not decoded.
//! - `ktype:`, `kproperty:`, `kobj:`, `ktypealias:`, `kpackage:` and
//!   `knamespace:` are implemented from the mangler's marker constants; Kotlin
//!   2.4.21 executables emit only `kfun:`, `kclass:` and `kvar:` symbols, so
//!   those prefixes are covered by hand-authored compatibility cases rather
//!   than compiler output.
mod node;
mod parser;
mod printer;
mod text;

use crate::error::BudgetKind;
use core::fmt;

use crate::budget::{EffectiveBudget, Limit};
use node::ParseLimits;
pub use node::{Ast, Function, Symbol, Type, TypeArgument, TypeParameter, Variance};
pub(crate) use node::{MAX_DEPTH, MAX_INPUT_LEN, MAX_WORK};
pub use parser::is_kotlin_native;
pub(crate) use parser::is_kotlin_native_bytes;

/// Why a Kotlin/Native symbol could not be parsed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub(crate) enum Failure {
    /// The symbol does not match a known Kotlin/Native form.
    Invalid,
    /// A type nests deeper than `MAX_DEPTH`.
    RecursedTooDeep,
    /// The input, work or list budget was exceeded.
    BudgetExceeded,
}

impl Failure {
    /// The parser budget dimension this failure belongs to, when it is a
    /// capacity failure rather than a malformed symbol.
    #[must_use]
    pub(crate) const fn budget_kind(self) -> Option<BudgetKind> {
        match self {
            Self::RecursedTooDeep => Some(BudgetKind::ParseDepth),
            Self::BudgetExceeded => Some(BudgetKind::Other),
            _ => None,
        }
    }
}

impl fmt::Display for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Invalid => "invalid syntax",
            Self::RecursedTooDeep => "recursion limit reached",
            Self::BudgetExceeded => "size limit reached",
        })
    }
}

impl core::error::Error for Failure {}

impl From<Failure> for crate::ParseError {
    fn from(failure: Failure) -> Self {
        Self::new(failure.budget_kind().map_or(
            crate::ParseErrorKind::Invalid,
            crate::ParseErrorKind::Budget,
        ))
    }
}

impl fmt::Display for Ast<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.print(f)
    }
}

impl Ast<'_> {
    /// Render the parsed symbol into `out`; the printer has no cap.
    pub(crate) fn print<W: fmt::Write + ?Sized>(&self, out: &mut W) -> fmt::Result {
        printer::print(&self.symbol, out)
    }
}

impl Ast<'_> {
    /// The scheme this tree was parsed from: always [`crate::Scheme::KotlinNative`].
    #[must_use]
    pub const fn scheme(&self) -> crate::Scheme {
        crate::Scheme::KotlinNative
    }
}

/// Parse a Kotlin/Native symbol into a reusable [`Ast`].
///
/// A Mach-O leading underscore (`_kfun:`) and a double underscore
/// (`__kfun:`) are accepted, as are klib `@` and ELF `__at__` extension
/// markers.
///
/// `budget` overrides the compiled-in limits per dimension, exactly as on a
/// [`Demangler`](crate::Demangler): [`Budget::default`](crate::Budget::default)
/// keeps them, and every value is clamped to
/// [`Scheme::hard_budget`](crate::Scheme::hard_budget).
///
/// # Errors
///
/// Returns [`ParseErrorKind::Invalid`](crate::ParseErrorKind::Invalid) when the prefix is unknown or a marker,
/// separator, list or type is malformed, a [`ParseErrorKind::Budget`](crate::ParseErrorKind::Budget) error of kind [`BudgetKind::ParseDepth`] when
/// a type nests deeper than `MAX_DEPTH`, and
/// [`ParseErrorKind::Budget`](crate::ParseErrorKind::Budget) when the input, work or list budget is
/// exceeded.
pub fn parse(symbol: &[u8], budget: crate::Budget) -> Result<Ast<'_>, crate::ParseError> {
    // A ThinLTO `.llvm.<hex>` ending is a linker suffix, not mangling.
    let symbol = crate::scheme::strip_thin_lto_bytes(symbol);
    parse_bytes_with_budget(symbol, budget.effective(crate::Scheme::KotlinNative))
        .map_err(crate::ParseError::from)
}

/// The budget the public entry point starts from.
///
/// [`parse`] uses it, so it keeps exactly the compiled-in limits; a
/// [`crate::Demangler`] resolves its configured [`crate::Budget`] for
/// [`Scheme::KotlinNative`](crate::Scheme::KotlinNative) instead.
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "the unit tests parse with the default budget")
)]
pub(crate) const DEFAULT_BUDGET: EffectiveBudget = EffectiveBudget {
    input_bytes: Some(Limit::Bounded(MAX_INPUT_LEN)),
    parse_depth: Some(Limit::Bounded(MAX_DEPTH)),
    work: Some(Limit::Bounded(MAX_WORK)),
    nodes: Some(Limit::Unbounded),
    expansion_visits: None,
    expansion_bytes: None,
};

/// Parse a Kotlin/Native symbol with one scheme's resolved budget.
///
/// [`parse`] wraps this with [`DEFAULT_BUDGET`]; a [`crate::Demangler`]
/// passes [`Demangler::effective_budget`](crate::Demangler::effective_budget)
/// for [`Scheme::KotlinNative`](crate::Scheme::KotlinNative), so a caller's
/// overrides reach the parser. The input, depth and work values are clamped to
/// this module's hard ceilings, and the grammar has no node dimension to
/// resolve.
pub(crate) fn parse_bytes_with_budget(
    symbol: &[u8],
    budget: EffectiveBudget,
) -> Result<Ast<'_>, Failure> {
    parser::parse_bytes(symbol, ParseLimits::from_effective(budget))
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod budget_tests;
