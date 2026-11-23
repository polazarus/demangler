//! Fortran backend (scaffold).
//!
//! The implementation is supplied by the feature worker.

use core::fmt;

/// Why a symbol could not be parsed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ParseError {
    /// The symbol does not match the grammar.
    Invalid,
    /// A parser budget was exceeded.
    BudgetExceeded,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Invalid => "invalid syntax",
            Self::BudgetExceeded => "size limit reached",
        })
    }
}

impl core::error::Error for ParseError {}

/// A parsed symbol (scaffold).
#[derive(Debug, Clone, Copy)]
pub struct Ast<'a> {
    symbol: &'a [u8],
}

impl fmt::Display for Ast<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        crate::parse::write_escaped(f, self.symbol, crate::parse::Escape::Identifier)
    }
}

/// Parse a symbol from UTF-8.
///
/// # Errors
///
/// Returns [] for unsupported syntax.
pub fn parse(symbol: &str) -> Result<Ast<'_>, ParseError> {
    parse_bytes(symbol.as_bytes())
}

/// Parse a symbol from raw bytes.
///
/// # Errors
///
/// Returns [] for unsupported syntax.
pub(crate) fn parse_bytes(_symbol: &[u8]) -> Result<Ast<'_>, ParseError> {
    Err(ParseError::Invalid)
}

/// True when a symbol has this scheme's high-confidence form.
#[must_use]
pub fn is_fortran(_symbol: &[u8]) -> bool {
    false
}
