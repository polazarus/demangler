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

#[cfg(test)]
mod tests {
    use std::string::ToString;

    use super::*;

    #[test]
    fn reports_the_failing_scheme() {
        assert_eq!(Error::Unsupported(Scheme::Go).scheme(), Scheme::Go);
        assert_eq!(Error::InvalidSymbol(Scheme::Rust).scheme(), Scheme::Rust);
    }

    #[test]
    fn displays_both_error_variants() {
        assert_eq!(
            Error::Unsupported(Scheme::KotlinNative).to_string(),
            "no backend available for Kotlin/Native"
        );
        assert_eq!(
            Error::InvalidSymbol(Scheme::Cpp).to_string(),
            "invalid C++ symbol"
        );
    }
}
