use core::fmt;

use crate::scheme::{Language, Scheme};

/// The demangled text of a symbol.
///
/// Implements [`fmt::Display`] so it can be written to any sink. Swift retains
/// its parsed node graph and renders it lazily; other backends either borrow or
/// store owned text (C++).
///
/// # Thread safety
///
/// Swift demangling retains an `Rc`-owned node graph, so `Demangled` is
/// `!Send + !Sync`. Demangle again on the target thread instead of moving a
/// value across threads.
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
    /// A backend that produced an owned string.
    Owned(alloc::string::String),
    /// A parsed Swift tree, rendered lazily.
    Swift(crate::swift::Parsed<'a>),
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
    #[allow(clippy::single_call_fn)]
    pub(crate) const fn owned(scheme: Scheme, text: alloc::string::String) -> Self {
        Self {
            scheme,
            text: Text::Owned(text),
        }
    }

    /// A parsed Swift tree.
    #[allow(clippy::single_call_fn)]
    pub(crate) const fn swift(scheme: Scheme, parsed: crate::swift::Parsed<'a>) -> Self {
        Self {
            scheme,
            text: Text::Swift(parsed),
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
            Text::Owned(text) => f.write_str(text),
            Text::Swift(parsed) => parsed.print(f),
        }
    }
}
