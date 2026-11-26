use core::fmt;

use crate::scheme::Scheme;

/// Error returned when a symbol cannot be demangled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Error {
    /// The detected scheme has no enabled backend.
    ///
    /// Every scheme in [`Scheme`] has an in-tree backend, so this variant
    /// is not produced; it is kept for API stability.
    Unsupported(Scheme),

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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
            Self::Other => "parser",
        }
    }
}

impl Error {
    /// The scheme that failed to demangle.
    ///
    /// Returns [`Scheme::Unknown`] for [`Error::NoMatchingScheme`], which
    /// names no single scheme.
    pub const fn scheme(&self) -> Scheme {
        match self {
            Self::Unsupported(scheme) | Self::InvalidSymbol(scheme) | Self::Budget { scheme, .. } => {
                *scheme
            }
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

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unsupported(scheme) => {
                write!(f, "no backend available for {}", scheme.language().as_str())
            }
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

#[cfg(test)]
mod tests {
    use std::string::ToString;

    use super::*;

    #[test]
    fn reports_the_failing_scheme() {
        assert_eq!(Error::Unsupported(Scheme::Go).scheme(), Scheme::Go);
        assert_eq!(
            Error::InvalidSymbol(Scheme::RustV0).scheme(),
            Scheme::RustV0
        );
        assert_eq!(
            Error::Budget {
                scheme: Scheme::D,
                kind: BudgetKind::Input
            }
            .scheme(),
            Scheme::D
        );
        assert_eq!(Error::NoMatchingScheme.scheme(), Scheme::Unknown);
    }

    #[test]
    fn displays_both_error_variants() {
        assert_eq!(
            Error::Unsupported(Scheme::KotlinNative).to_string(),
            "no backend available for Kotlin/Native"
        );
        assert_eq!(
            Error::InvalidSymbol(Scheme::CppItanium).to_string(),
            "invalid C++ symbol"
        );
        assert_eq!(
            Error::Budget {
                scheme: Scheme::D,
                kind: BudgetKind::Work
            }
            .to_string(),
            "D symbol exceeded its work budget"
        );
        assert_eq!(
            Error::Budget {
                scheme: Scheme::Jni,
                kind: BudgetKind::Other
            }
            .budget_kind(),
            Some(BudgetKind::Other)
        );
        assert_eq!(Error::NoMatchingScheme.to_string(), "no selected scheme could demangle the symbol");
    }
}
