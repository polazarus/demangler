use core::fmt;

use crate::scheme::Scheme;

/// Error returned when a symbol cannot be demangled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Error {
    /// The detected scheme has no enabled backend (missing feature or not yet
    /// implemented, e.g. Swift or Go).
    Unsupported(Scheme),

    /// A backend is enabled for the detected scheme but could not demangle
    /// the symbol.
    InvalidSymbol(Scheme),
}

impl Error {
    /// The scheme that failed to demangle.
    pub const fn scheme(&self) -> Scheme {
        match self {
            Self::Unsupported(scheme) | Self::InvalidSymbol(scheme) => *scheme,
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
        }
    }
}

impl core::error::Error for Error {}
