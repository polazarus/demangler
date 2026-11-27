use core::fmt;

use crate::scheme::Scheme;

/// Error returned when a symbol cannot be demangled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Error {
    /// A backend is enabled for the detected scheme but could not demangle
    /// the symbol because the input is malformed or unsupported.
    InvalidSymbol(Scheme),

    /// A parser limit rejected the symbol.
    ///
    /// This is a capacity failure, not a malformed symbol: raise the relevant
    /// [`crate::Budget`] dimension (or use the backend default) to accept it.
    Budget {
        /// The scheme whose parser hit the limit.
        scheme: Scheme,
        /// The dimension that rejected the symbol.
        kind: BudgetKind,
    },

    /// Several candidate schemes were tried and none could demangle the
    /// symbol.
    ///
    /// A [`crate::Selection`] reports this instead of [`Error::InvalidSymbol`]
    /// when more than one scheme was allowed, because no single scheme is
    /// responsible for the failure.
    NoMatchingScheme,
}

/// The parser dimension that rejected a symbol.
///
/// Backends with fine-grained errors report the exact dimension. Some legacy
/// error types collapse input, work and node failures into one budget error;
/// they report [`BudgetKind::Other`] rather than guessing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum BudgetKind {
    /// The input byte limit.
    Input,
    /// The parser nesting-depth limit.
    ParseDepth,
    /// The parser work limit.
    Work,
    /// The AST node limit.
    Nodes,
    /// The parse-time expansion visit limit.
    ExpansionVisits,
    /// The parse-time expansion byte limit.
    ExpansionBytes,
    /// A budget failure whose dimension the backend does not distinguish.
    Other,
}

impl BudgetKind {
    /// A short name for the dimension.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Input => "input",
            Self::ParseDepth => "parse depth",
            Self::Work => "work",
            Self::Nodes => "nodes",
            Self::ExpansionVisits => "expansion visits",
            Self::ExpansionBytes => "expansion bytes",
            Self::Other => "parser",
        }
    }
}

impl Error {
    /// The scheme that failed to demangle.
    ///
    /// Returns [`Scheme::Unknown`] for [`Error::NoMatchingScheme`], which
    /// names no single scheme.
    #[must_use]
    pub const fn scheme(&self) -> Scheme {
        match self {
            Self::InvalidSymbol(scheme) | Self::Budget { scheme, .. } => *scheme,
            Self::NoMatchingScheme => Scheme::Unknown,
        }
    }

    /// The budget dimension, when this is [`Error::Budget`].
    #[must_use]
    pub const fn budget_kind(&self) -> Option<BudgetKind> {
        match self {
            Self::Budget { kind, .. } => Some(*kind),
            _ => None,
        }
    }
}

impl fmt::Display for BudgetKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidSymbol(scheme) => {
                write!(f, "invalid {} symbol", scheme.language().as_str())
            }
            Self::Budget { scheme, kind } => write!(
                f,
                "{} symbol exceeded its {} budget",
                scheme.language().as_str(),
                kind.as_str()
            ),
            Self::NoMatchingScheme => f.write_str("no selected scheme could demangle the symbol"),
        }
    }
}

impl core::error::Error for Error {}

/// Why a backend parser rejected a symbol.
///
/// Every backend `parse` returns this one type. [`ParseError::offset`] is the
/// byte offset in the parsed input where the parser stopped, when the backend
/// tracks it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ParseError {
    kind: ParseErrorKind,
    offset: Option<usize>,
}

/// The class of a [`ParseError`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ParseErrorKind {
    /// The input is not in this backend's scheme: no prefix, or the wrong
    /// overall shape.
    NotThisScheme,
    /// The input ended inside a production.
    UnexpectedEnd,
    /// The input is malformed: an invalid byte, length, substitution or back
    /// reference.
    Invalid,
    /// The input is well-formed but uses a production the backend rejects on
    /// purpose, such as an unsupported vendor form or a Swift symbolic
    /// reference.
    Unsupported,
    /// A budget dimension rejected the symbol.
    Budget(BudgetKind),
}

impl ParseError {
    /// A parse error of `kind` with no known offset.
    #[allow(dead_code, reason = "unused in a build without a backend")]
    #[must_use]
    pub(crate) const fn new(kind: ParseErrorKind) -> Self {
        Self { kind, offset: None }
    }

    /// A parse error of `kind` at byte `offset`.
    #[allow(dead_code, reason = "only the C++ backend tracks offsets")]
    #[must_use]
    pub(crate) const fn at(kind: ParseErrorKind, offset: usize) -> Self {
        Self {
            kind,
            offset: Some(offset),
        }
    }

    /// What went wrong.
    #[must_use]
    pub const fn kind(&self) -> ParseErrorKind {
        self.kind
    }

    /// The byte offset in the parsed input where the parser stopped, when the
    /// backend tracks it.
    #[must_use]
    pub const fn offset(&self) -> Option<usize> {
        self.offset
    }

    /// The budget dimension, when this is a budget failure.
    #[must_use]
    pub const fn budget_kind(&self) -> Option<BudgetKind> {
        match self.kind {
            ParseErrorKind::Budget(kind) => Some(kind),
            _ => None,
        }
    }
}

impl fmt::Display for ParseErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotThisScheme => f.write_str("not in this scheme"),
            Self::UnexpectedEnd => f.write_str("unexpected end of input"),
            Self::Invalid => f.write_str("invalid mangling"),
            Self::Unsupported => f.write_str("unsupported production"),
            Self::Budget(kind) => write!(f, "{kind} budget exceeded"),
        }
    }
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.offset {
            Some(offset) => write!(f, "{} at byte {offset}", self.kind),
            None => fmt::Display::fmt(&self.kind, f),
        }
    }
}

impl core::error::Error for ParseError {}

#[cfg(test)]
mod tests;
