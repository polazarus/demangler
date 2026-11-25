#[cfg(any(feature = "encoding", feature = "d"))]
use alloc::string::String;
use core::fmt;

#[cfg(feature = "encoding")]
use crate::encoding::{Encoding, EncodingError};
#[cfg(feature = "encoding")]
use crate::error::Error;
use crate::scheme::{Language, Scheme};

/// The demangled text of a symbol.
///
/// Implements [`fmt::Display`] so it can be written to any sink. C++, Rust,
/// Swift, Go, Objective-C, Kotlin/Native, JNI, Ada, Fortran, Delphi and OCaml
/// retain their parsed representation and render lazily; D stores validated
/// rendered text.
///
/// # Rendering cost
///
/// A retained AST is rendered again on every [`fmt::Display`] call; the crate
/// does not cache the text. Store a `to_string()` result, or use a caller-owned
/// cache (for example an LRU), if the same value is printed repeatedly. D
/// stores validated text and does not re-render.
///
/// # Thread safety
///
/// C++, Rust and Swift retain `Rc`-owned node graphs, so `Demangled` is
/// `!Send + !Sync`. Demangle again on the target thread instead of moving a
/// value across threads.
#[derive(Debug)]
pub struct Demangled<'a> {
    scheme: Scheme,
    text: Text<'a>,
    #[cfg(feature = "coff")]
    coff_decoration: Option<crate::coff::Decoration<'a>>,
}

/// Backing text representation, chosen per backend.
///
/// The AST payloads are intentionally stored inline. Boxing a backend AST
/// would save enum size only for feature subsets that expose one large parser,
/// at the cost of an allocation on every successful demangle; all-features
/// builds already carry several large variants.
#[allow(clippy::large_enum_variant)]
#[derive(Debug)]
enum Text<'a> {
    /// The symbol is already readable (C, unknown, or pass-through).
    Plain(&'a str),
    /// Non-UTF-8 bytes passed through with escaped invalid bytes.
    Bytes(&'a [u8]),
    /// Parsed Rust tree, rendered lazily.
    #[cfg(feature = "rust")]
    Rust(crate::rust::Ast<'a>),
    /// Validated rendered D text.
    #[cfg(feature = "d")]
    Owned(String),
    /// A parsed C++ tree, rendered lazily.
    #[cfg(any(feature = "cpp-itanium", feature = "cpp-msvc", feature = "cpp-gnu-v2"))]
    Cpp(crate::cpp::Ast<'a>),
    /// A parsed Swift tree, rendered lazily.
    #[cfg(feature = "swift")]
    Swift(crate::swift::Ast<'a>),
    /// A parsed Go tree, rendered lazily.
    #[cfg(feature = "go")]
    Go(crate::go::Ast<'a>),
    /// A parsed Objective-C symbol, rendered lazily.
    #[cfg(feature = "objc")]
    ObjC(crate::objc::Ast<'a>),
    /// A parsed Kotlin/Native symbol, rendered lazily.
    #[cfg(feature = "kotlin-native")]
    KotlinNative(crate::kotlin_native::Ast<'a>),
    /// A parsed JNI symbol, rendered lazily.
    #[cfg(feature = "jni")]
    Jni(crate::jni::Ast<'a>),
    /// A parsed Ada symbol, rendered lazily.
    #[cfg(feature = "ada")]
    Ada(crate::ada::Ast<'a>),
    /// A parsed Fortran symbol, rendered lazily.
    #[cfg(feature = "fortran")]
    Fortran(crate::fortran::Ast<'a>),
    /// A parsed Delphi/Object Pascal symbol, rendered lazily.
    #[cfg(feature = "delphi")]
    Delphi(crate::delphi::Ast<'a>),
    /// A parsed OCaml symbol, rendered lazily.
    #[cfg(feature = "ocaml")]
    OCaml(crate::ocaml::Ast<'a>),
}

impl<'a> Demangled<'a> {
    /// Build a result from its scheme and backing text representation.
    const fn with(scheme: Scheme, text: Text<'a>) -> Self {
        Self {
            scheme,
            text,
            #[cfg(feature = "coff")]
            coff_decoration: None,
        }
    }

    /// Plain, non-mangled text.
    pub(crate) const fn plain(scheme: Scheme, text: &'a str) -> Self {
        Self::with(scheme, Text::Plain(text))
    }

    /// Raw non-UTF-8 bytes; rendered with invalid bytes escaped.
    pub(crate) const fn bytes(scheme: Scheme, bytes: &'a [u8]) -> Self {
        Self::with(scheme, Text::Bytes(bytes))
    }

    /// A parsed Rust tree.
    #[cfg(feature = "rust")]
    pub(crate) const fn rust(scheme: Scheme, text: crate::rust::Ast<'a>) -> Self {
        Self::with(scheme, Text::Rust(text))
    }

    /// Validated rendered D text.
    #[cfg(feature = "d")]
    pub(crate) const fn owned(scheme: Scheme, text: String) -> Self {
        Self::with(scheme, Text::Owned(text))
    }

    /// A parsed C++ tree.
    #[cfg(any(feature = "cpp-itanium", feature = "cpp-msvc", feature = "cpp-gnu-v2"))]
    pub(crate) const fn cpp(scheme: Scheme, parsed: crate::cpp::Ast<'a>) -> Self {
        Self::with(scheme, Text::Cpp(parsed))
    }

    /// A parsed Swift tree.
    #[cfg(feature = "swift")]
    pub(crate) const fn swift(scheme: Scheme, parsed: crate::swift::Ast<'a>) -> Self {
        Self::with(scheme, Text::Swift(parsed))
    }

    /// A parsed Go tree.
    #[cfg(feature = "go")]
    pub(crate) const fn go(scheme: Scheme, parsed: crate::go::Ast<'a>) -> Self {
        Self::with(scheme, Text::Go(parsed))
    }

    /// A parsed Objective-C symbol.
    #[cfg(feature = "objc")]
    pub(crate) const fn objc(scheme: Scheme, parsed: crate::objc::Ast<'a>) -> Self {
        Self::with(scheme, Text::ObjC(parsed))
    }

    /// A parsed Kotlin/Native symbol.
    #[cfg(feature = "kotlin-native")]
    pub(crate) const fn kotlin_native(
        scheme: Scheme,
        parsed: crate::kotlin_native::Ast<'a>,
    ) -> Self {
        Self::with(scheme, Text::KotlinNative(parsed))
    }

    /// A parsed JNI symbol.
    #[cfg(feature = "jni")]
    pub(crate) const fn jni(scheme: Scheme, text: crate::jni::Ast<'a>) -> Self {
        Self::with(scheme, Text::Jni(text))
    }

    /// A parsed Ada symbol.
    #[cfg(feature = "ada")]
    pub(crate) const fn ada(scheme: Scheme, text: crate::ada::Ast<'a>) -> Self {
        Self::with(scheme, Text::Ada(text))
    }

    /// A parsed Fortran symbol.
    #[cfg(feature = "fortran")]
    pub(crate) const fn fortran(scheme: Scheme, text: crate::fortran::Ast<'a>) -> Self {
        Self::with(scheme, Text::Fortran(text))
    }

    /// A parsed Delphi/Object Pascal symbol.
    #[cfg(feature = "delphi")]
    pub(crate) const fn delphi(scheme: Scheme, text: crate::delphi::Ast<'a>) -> Self {
        Self::with(scheme, Text::Delphi(text))
    }

    /// A parsed OCaml symbol.
    #[cfg(feature = "ocaml")]
    pub(crate) const fn ocaml(scheme: Scheme, text: crate::ocaml::Ast<'a>) -> Self {
        Self::with(scheme, Text::OCaml(text))
    }

    #[cfg(feature = "encoding")]
    /// Render the demangled text, decoding reversible `\xNN` identifier
    /// escapes with `encoding`.
    ///
    /// Plain C and unknown names are already Unicode and are returned
    /// unchanged. Parsed backends render their identifiers with the crate's
    /// reversible escape policy; the rendered text is then decoded by
    /// [`Encoding::decode_identifier_escapes`]. See [`crate::encoding`] for
    /// the supported encodings and their scope.
    ///
    /// # Errors
    ///
    /// Returns [`EncodingError::Demangle`] if a retained tree cannot be
    /// rendered, [`EncodingError::InvalidEscape`] for a malformed escape, and
    /// [`EncodingError::InvalidEncoding`] when the escaped bytes are not valid
    /// in `encoding`.
    pub fn to_string_with_encoding(&self, encoding: Encoding) -> Result<String, EncodingError> {
        match &self.text {
            Text::Plain(text) => Ok(String::from(*text)),
            #[cfg(feature = "d")]
            Text::Owned(text) => encoding.decode_identifier_escapes(text),
            _ => {
                let mut rendered = String::new();
                fmt::Write::write_fmt(&mut rendered, format_args!("{self}"))
                    .map_err(|_| EncodingError::Demangle(Error::InvalidSymbol(self.scheme)))?;
                encoding.decode_identifier_escapes(&rendered)
            }
        }
    }

    /// Decoded PE/COFF decoration metadata, when the input was decorated.
    #[cfg(feature = "coff")]
    #[must_use]
    pub const fn coff_decoration(&self) -> Option<&crate::coff::Decoration<'a>> {
        self.coff_decoration.as_ref()
    }

    #[cfg(feature = "coff")]
    pub(crate) fn with_coff_decoration(mut self, decoration: crate::coff::Decoration<'a>) -> Self {
        self.coff_decoration = Some(decoration);
        self
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
            Text::Bytes(bytes) => {
                crate::parse::write_escaped(f, bytes, crate::parse::Escape::Identifier)
            }
            #[cfg(feature = "rust")]
            Text::Rust(text) => fmt::Display::fmt(text, f),
            #[cfg(feature = "d")]
            Text::Owned(text) => f.write_str(text),
            #[cfg(any(
                feature = "cpp-itanium",
                feature = "cpp-msvc",
                feature = "cpp-gnu-v2"
            ))]
            Text::Cpp(parsed) => fmt::Display::fmt(parsed, f),
            #[cfg(feature = "swift")]
            Text::Swift(parsed) => parsed.print(f),
            #[cfg(feature = "go")]
            Text::Go(parsed) => parsed.print(f),
            #[cfg(feature = "objc")]
            Text::ObjC(parsed) => parsed.print(f),
            #[cfg(feature = "kotlin-native")]
            Text::KotlinNative(parsed) => parsed.print(f),
            #[cfg(feature = "jni")]
            Text::Jni(parsed) => fmt::Display::fmt(parsed, f),
            #[cfg(feature = "ada")]
            Text::Ada(parsed) => fmt::Display::fmt(parsed, f),
            #[cfg(feature = "fortran")]
            Text::Fortran(parsed) => fmt::Display::fmt(parsed, f),
            #[cfg(feature = "delphi")]
            Text::Delphi(parsed) => fmt::Display::fmt(parsed, f),
            #[cfg(feature = "ocaml")]
            Text::OCaml(parsed) => fmt::Display::fmt(parsed, f),
        }
    }
}
