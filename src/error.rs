use core::fmt;

use crate::scheme::Scheme;

/// Error returned when a symbol cannot be demangled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Error {
    /// The backend for `scheme` rejected the symbol: it is malformed, uses an
    /// unsupported production, or exceeds a [`crate::Budget`] dimension
    /// ([`ParseError::budget_kind`]).
    ///
    /// An offset in `error` is relative to the name the backend parsed: after
    /// a PE/COFF decoration, a ThinLTO suffix or a version suffix is removed.
    Parse {
        /// The scheme whose backend rejected the symbol.
        scheme: Scheme,
        /// Why the backend rejected it.
        error: ParseError,
    },

    /// Several candidate schemes were tried and none could demangle the
    /// symbol.
    ///
    /// A [`crate::Selection`] reports this instead of [`Error::Parse`] when
    /// more than one scheme was allowed, because no single scheme is
    /// responsible for the failure.
    NoMatchingScheme,
}

/// The parser dimension that rejected a symbol.
///
/// Every backend reports the exact dimension it ran out of.
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
        }
    }
}

impl Error {
    /// A parse error of `kind` with no offset, reported for `scheme`.
    pub(crate) const fn parse(scheme: Scheme, kind: ParseErrorKind) -> Self {
        Self::Parse {
            scheme,
            error: ParseError::new(kind),
        }
    }

    /// The scheme that failed to demangle, or `None` for
    /// [`Error::NoMatchingScheme`], which names no single scheme.
    #[must_use]
    pub const fn scheme(&self) -> Option<Scheme> {
        match self {
            Self::Parse { scheme, .. } => Some(*scheme),
            Self::NoMatchingScheme => None,
        }
    }

    /// The backend's parse error, when one scheme was tried.
    #[must_use]
    pub const fn parse_error(&self) -> Option<ParseError> {
        match self {
            Self::Parse { error, .. } => Some(*error),
            Self::NoMatchingScheme => None,
        }
    }

    /// The budget dimension, when a budget rejected the symbol.
    #[must_use]
    pub const fn budget_kind(&self) -> Option<BudgetKind> {
        match self {
            Self::Parse { error, .. } => error.budget_kind(),
            Self::NoMatchingScheme => None,
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
            Self::Parse { scheme, error } => match error.budget_kind() {
                Some(kind) => write!(
                    f,
                    "{} symbol exceeded its {kind} budget",
                    scheme.language().as_str()
                ),
                None => write!(f, "invalid {} symbol: {error}", scheme.language().as_str()),
            },
            Self::NoMatchingScheme => f.write_str("no selected scheme could demangle the symbol"),
        }
    }
}

impl core::error::Error for Error {}

/// Why a backend parser rejected a symbol.
///
/// Every backend `parse` returns this one type. [`ParseError::offset`] is the
/// byte offset in the input where the parser stopped: the offending byte, or
/// the end of the input for
/// [`UnexpectedEnd`](ParseErrorKind::UnexpectedEnd). A parser that backtracks
/// reports the alternative that failed last, so the offset is where it gave
/// up, which is not always where the mistake is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ParseError {
    kind: ParseErrorKind,
    /// The offset, or [`NO_OFFSET`]: a `u32` keeps the error as small as the
    /// parsers' internal failures, which every production returns.
    offset: u32,
}

/// The stored offset of a [`ParseError`] whose position is unknown.
const NO_OFFSET: u32 = u32::MAX;

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
    #[must_use]
    pub(crate) const fn new(kind: ParseErrorKind) -> Self {
        Self {
            kind,
            offset: NO_OFFSET,
        }
    }

    /// A parse error of `kind` at byte `offset`.
    ///
    /// Inputs are bounded by budgets that fit a `u32`, so saturating never
    /// changes a real offset.
    #[must_use]
    pub(crate) const fn at(kind: ParseErrorKind, offset: usize) -> Self {
        #[expect(
            clippy::cast_possible_truncation,
            reason = "the offset is checked against `NO_OFFSET` first"
        )]
        let offset = if offset >= NO_OFFSET as usize {
            NO_OFFSET - 1
        } else {
            offset as u32
        };
        Self { kind, offset }
    }

    /// A malformed-input error at byte `offset`.
    #[allow(dead_code, reason = "only the Go, JNI and Rust backends use it")]
    #[must_use]
    pub(crate) const fn invalid_at(offset: usize) -> Self {
        Self::at(ParseErrorKind::Invalid, offset)
    }

    /// A budget error of `kind` with no position.
    #[must_use]
    pub(crate) const fn budget(kind: BudgetKind) -> Self {
        Self::new(ParseErrorKind::Budget(kind))
    }

    /// What went wrong.
    #[must_use]
    pub const fn kind(&self) -> ParseErrorKind {
        self.kind
    }

    /// The byte offset in the input where the parser stopped.
    ///
    /// `None` when no single byte is at fault: no scheme prefix, an input
    /// longer than the budget, a tree outside an expansion gate or the node
    /// budget, or a symbol whose overall shape does not match (Fortran
    /// classifies whole names).
    #[must_use]
    pub const fn offset(&self) -> Option<usize> {
        if self.offset == NO_OFFSET {
            None
        } else {
            Some(self.offset as usize)
        }
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
        match self.offset() {
            Some(offset) => write!(f, "{} at byte {offset}", self.kind),
            None => fmt::Display::fmt(&self.kind, f),
        }
    }
}

impl core::error::Error for ParseError {}

#[cfg(test)]
mod tests;
