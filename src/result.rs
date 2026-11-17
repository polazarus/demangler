use core::fmt;

use crate::scheme::{Language, Scheme};

/// The demangled text of a symbol.
///
/// Implements [`fmt::Display`] so it can be written to any sink without
/// allocating: a `no_std` caller can write it into a fixed buffer, a `std`
/// caller can `to_string()` it.
#[derive(Debug)]
pub struct Demangled<'a> {
    scheme: Scheme,
    text: Text<'a>,
}

/// Backing text representation, chosen per backend.
#[derive(Debug)]
enum Text<'a> {
    /// The symbol is already readable (C, unknown, or pass-through).
    Plain(&'a str),
    /// `rustc-demangle` adapter: renders lazily, no allocation.
    #[cfg(feature = "rust")]
    Rust(rustc_demangle::Demangle<'a>),
    /// A backend that produced an owned string (`cpp` feature only).
    #[cfg(feature = "cpp")]
    Owned(alloc::string::String),
}

impl<'a> Demangled<'a> {
    /// Plain, non-mangled text.
    #[allow(clippy::single_call_fn)]
    pub(crate) const fn plain(scheme: Scheme, text: &'a str) -> Self {
        Self {
            scheme,
            text: Text::Plain(text),
        }
    }

    /// Lazy rustc-demangle adapter.
    #[cfg(feature = "rust")]
    #[allow(clippy::single_call_fn)]
    pub(crate) const fn rust(scheme: Scheme, text: rustc_demangle::Demangle<'a>) -> Self {
        Self {
            scheme,
            text: Text::Rust(text),
        }
    }

    /// Owned text from an `alloc`-based backend.
    #[cfg(feature = "cpp")]
    #[allow(clippy::single_call_fn)]
    pub(crate) const fn owned(scheme: Scheme, text: alloc::string::String) -> Self {
        Self {
            scheme,
            text: Text::Owned(text),
        }
    }

    /// The detected mangling scheme.
    pub const fn scheme(&self) -> Scheme {
        self.scheme
    }

    /// The language associated with the detected scheme.
    pub const fn language(&self) -> Language {
        self.scheme.language()
    }
}

impl fmt::Display for Demangled<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.text {
            Text::Plain(text) => f.write_str(text),
            #[cfg(feature = "rust")]
            Text::Rust(text) => write!(f, "{text}"),
            #[cfg(feature = "cpp")]
            Text::Owned(text) => f.write_str(text),
        }
    }
}
