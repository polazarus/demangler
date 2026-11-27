//! Pure-Rust Java JNI native-method name demangling.
//!
//! A Java native method is bound to a C function whose name the JVM derives
//! from the method's declaring class and name. This module parses that name,
//! decodes the JNI escapes and renders a Java-like display form. There is no
//! canonical JNI demangler, so the spelling is this crate's contract.
//!
//! No external demangler is used and there is no runtime dependency.
//!
//! # Supported forms
//!
//! - short names: `Java_com_example_Foo_plain`, emitted when the method is not
//!   overloaded;
//! - long names: `Java_com_example_Foo_bar__I`, emitted for an overloaded
//!   method; the part after `__` is the mangled argument list, which is empty
//!   for the no-argument overload;
//! - every JNI escape: `_1` (`_`) in a name, `_2` (`;`) and `_3` (`[`) in an
//!   argument list, and `_0xxxx` (a UTF-16 code unit, including a surrogate
//!   pair for a supplementary character) in both;
//! - every descriptor type: the primitives `B C D F I J S Z`, object types
//!   (`L...;`) and arrays (`_3` per dimension).
//!
//! The mangled argument list is the descriptor's argument types with the
//! parentheses dropped: `([Ljava/lang/String;I)` becomes
//! `_3Ljava_lang_String_2I` and `()` becomes the empty string.
//!
//! # Output contract
//!
//! | Input | Output |
//! |---|---|
//! | `Java_com_example_Foo_plain` | `com.example.Foo.plain` |
//! | `Java_com_example_Foo_bar___3Ljava_lang_String_2I` | `com.example.Foo.bar(java.lang.String[], int)` |
//! | `Java_com_example_Foo_under_1score__` | `com.example.Foo.under_score()` |
//! | `Java_com_example_Weird_11_m__` | `com.example.Weird_1.m()` |
//!
//! The mangling writes both the package separator (`.`) and the binary-name
//! separator (`/`) as a bare `_`, and escapes a literal `_` as `_1`, so the
//! class path is recoverable: every unescaped `_` is a component boundary. The
//! last component before the descriptor delimiter is the method, and everything
//! before it is the class path, which is why the display form is a debugger
//! name rather than a Java source declaration.
//!
//! The long form encodes the argument types only: the JVM appends it to
//! disambiguate overloads, and Java forbids return-type-only overloading, so
//! the return type is redundant and absent from the symbol.
//!
//! # Raw bytes
//!
//! A symbol name is a byte string, so the parser matches the grammar on bytes
//! and never requires the input to be UTF-8. A byte that the mangling leaves
//! unescaped must be an ASCII alphanumeric, so a non-ASCII character can only
//! appear through a `_0xxxx` escape; decoded characters are printed as UTF-8.
//!
//! # Detection
//!
//! [`crate::Scheme::detect`] claims a name only when the whole symbol parses as
//! a short or long JNI name ([`is_jni`](crate::jni::is_jni)). The check runs before the legacy C++
//! dialects, which would otherwise claim a long name because it contains `__`.
//! `JNI_OnLoad`, `JNI_CreateJavaVM` and other `JNI_`-prefixed helpers are plain
//! C and stay [`Scheme::C`], as do registration names passed to
//! `RegisterNatives`: those are ordinary identifiers, not mangled names.
//!
//! # Budgets
//!
//! Untrusted input stays bounded by
//! `MAX_INPUT_LEN`,
//! `MAX_DEPTH` and
//! `MAX_WORK`. Exceeding a budget is an
//! error, never a partial result.
//!
//! The input, depth and work limits are runtime-configurable.
//! [`parse`](crate::jni::parse) keeps the compiled-in defaults, and a
//! [`crate::Demangler`] passes the configured budget, clamped to the hard
//! ceilings of 16 MiB input, 1024 array dimensions and 64M work units. A parsed
//! [`Type`](crate::jni::Type) stores its dimension count in a `u16`, comfortably
//! above the parser ceiling.
//! Input and work are independent: raising the input limit alone keeps the
//! default work allowance, so a caller that raises the input for a long symbol
//! must raise [`crate::Budget::work`] as well when the scan exceeds it. One
//! name holds one argument list, so there is no node dimension to configure.
//!
//! Printing is not a budget: the printer walks each node once and the output
//! is linear in the parsed name, so `Display` is total except for a failing
//! sink.
//!
//! # Gaps
//!
//! - The class/method boundary is the last unescaped `_`, so a symbol cannot
//!   distinguish a class whose path ends in several components from a method
//!   name; that ambiguity is inherent to the mangling and is resolved by the
//!   documented rule instead of a guess.
//! - A `_` followed by a digit is always an escape or a malformed escape: no
//!   Java identifier starts with a digit, so `_4` is rejected rather than read
//!   as a separator plus a name.
//! - Decoded characters are not validated as Java identifier characters; the
//!   mangling is defined on escape sequences, not on identifier syntax.
//! - The return type is absent from the symbol, so it is not printed.

mod node;
mod parser;
mod printer;

use crate::error::BudgetKind;
use core::fmt;

use crate::budget::{EffectiveBudget, Limit};
use node::ParseLimits;
pub use node::{Ast, Descriptor, Element, Primitive, Type};
pub(crate) use node::{MAX_DEPTH, MAX_INPUT_LEN, MAX_WORK};

/// Why a symbol could not be parsed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ParseError {
    /// The symbol does not match the JNI name grammar.
    Invalid,
    /// A descriptor nests arrays deeper than `MAX_DEPTH`.
    RecursedTooDeep,
    /// The input, work or output budget was exceeded.
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
    /// Render the parsed name into `out`, without a print cap.
    pub(crate) fn print<W: fmt::Write + ?Sized>(&self, out: &mut W) -> fmt::Result {
        printer::print(self, out)
    }
}

impl Ast<'_> {
    /// The scheme this tree was parsed from: always [`crate::Scheme::Jni`].
    #[must_use]
    pub const fn scheme(&self) -> crate::Scheme {
        crate::Scheme::Jni
    }
}

/// Parse a JNI native-method name into a reusable [`Ast`].
///
/// # Errors
///
/// Returns [`ParseError::Invalid`] when the prefix is missing, an escape is
/// truncated or malformed, a surrogate is unpaired, or a descriptor byte is
/// unknown; [`ParseError::RecursedTooDeep`] when a descriptor nests arrays
/// deeper than `MAX_DEPTH`; and [`ParseError::BudgetExceeded`] when the input
/// or work budget is exceeded.
pub fn parse(symbol: &[u8]) -> Result<Ast<'_>, ParseError> {
    parse_bytes_with_budget(symbol, DEFAULT_BUDGET)
}

/// The budget the public entry point starts from.
///
/// [`parse`] uses it, so it keeps exactly the compiled-in limits; a
/// [`crate::Demangler`] resolves its configured [`crate::Budget`] for
/// [`Scheme::Jni`](crate::Scheme::Jni) instead.
pub(crate) const DEFAULT_BUDGET: EffectiveBudget = EffectiveBudget {
    input_bytes: Some(Limit::Bounded(MAX_INPUT_LEN)),
    parse_depth: Some(Limit::Bounded(MAX_DEPTH)),
    work: Some(Limit::Bounded(MAX_WORK)),
    nodes: Some(Limit::Unbounded),
    expansion_visits: None,
    expansion_bytes: None,
};

/// Parse a JNI native-method name with one scheme's resolved budget.
///
/// [`parse`] wraps this with [`DEFAULT_BUDGET`]; a [`crate::Demangler`]
/// passes [`Demangler::effective_budget`](crate::Demangler::effective_budget)
/// for [`Scheme::Jni`](crate::Scheme::Jni), so a caller's overrides reach the
/// parser. The input, depth and work values are clamped to this module's hard
/// ceilings, and the grammar has no node dimension to resolve.
pub(crate) fn parse_bytes_with_budget(
    symbol: &[u8],
    budget: EffectiveBudget,
) -> Result<Ast<'_>, ParseError> {
    parser::parse_bytes(symbol, ParseLimits::from_effective(budget))
}

/// True when `symbol` is a fully valid JNI native-method name.
///
/// Detection validates the whole symbol rather than accepting the `Java_`
/// prefix, because ordinary C functions share it: a name that does not parse is
/// not JNI and stays [`crate::Scheme::C`].
#[must_use]
pub fn is_jni(symbol: &[u8]) -> bool {
    parse(symbol).is_ok()
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod budget_tests;
