use core::fmt;

use crate::scheme::{Language, Scheme};

/// The demangled text of a symbol.
///
/// Implements [`fmt::Display`] so it can be written to any sink. Rust, Swift,
/// C++ and Go retain their parsed representation and render lazily.
///
/// # Thread safety
///
/// Rust, Swift and C++ retain `Rc`-owned node graphs, so `Demangled` is
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
    /// Parsed Rust tree, rendered lazily.
    Rust(crate::rust::Ast<'a>),
    /// A parsed C++ tree, rendered lazily.
    Cpp(crate::cpp::Ast<'a>),
    /// A parsed Swift tree, rendered lazily.
    Swift(crate::swift::Ast<'a>),
    /// A parsed Go name, rendered lazily.
    Go(crate::go::Ast<'a>),
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

    /// A parsed Rust tree.
    #[allow(clippy::single_call_fn)]
    pub(crate) const fn rust(scheme: Scheme, text: crate::rust::Ast<'a>) -> Self {
        Self {
            scheme,
            text: Text::Rust(text),
        }
    }

    /// A parsed C++ tree.
    #[allow(clippy::single_call_fn)]
    pub(crate) const fn cpp(scheme: Scheme, text: crate::cpp::Ast<'a>) -> Self {
        Self {
            scheme,
            text: Text::Cpp(text),
        }
    }

    /// A parsed Swift tree.
    #[allow(clippy::single_call_fn)]
    pub(crate) const fn swift(scheme: Scheme, parsed: crate::swift::Ast<'a>) -> Self {
        Self {
            scheme,
            text: Text::Swift(parsed),
        }
    }

    /// A parsed Go name.
    #[allow(clippy::single_call_fn)]
    pub(crate) const fn go(scheme: Scheme, parsed: crate::go::Ast<'a>) -> Self {
        Self {
            scheme,
            text: Text::Go(parsed),
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
            Text::Rust(text) => write!(f, "{text}"),
            Text::Cpp(text) => write!(f, "{text}"),
            Text::Swift(parsed) => parsed.print(f),
            Text::Go(parsed) => parsed.print(f),
        }
    }
}
