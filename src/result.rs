#[cfg(feature = "encoding")]
use alloc::string::String;
use core::fmt;

#[cfg(feature = "encoding")]
use crate::encoding::{Encoding, EncodingError};
#[cfg(feature = "encoding")]
use crate::error::Error;
use crate::scheme::{Language, Scheme};

/// The demangled text of a symbol.
///
/// Implements [`fmt::Display`] so it can be written to any sink. Rust, Swift,
/// C++, Go, Objective-C, Kotlin/Native, D, JNI, Ada, Fortran, Delphi and OCaml
/// retain their parsed representation and render lazily.
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
    #[cfg(feature = "coff")]
    coff_decoration: Option<crate::coff::Decoration<'a>>,
}

/// Backing text representation, chosen per backend.
#[derive(Debug)]
enum Text<'a> {
    /// The symbol is already readable (C, unknown, or pass-through).
    Plain(&'a str),
    /// Non-UTF-8 bytes passed through with escaped invalid bytes.
    Bytes(&'a [u8]),
    /// Parsed Rust tree, rendered lazily.
    #[cfg(feature = "rust")]
    Rust(crate::rust::Ast<'a>),
    /// A parsed C++ tree, rendered lazily.
    #[cfg(any(
        feature = "cpp-itanium",
        feature = "cpp-msvc",
        feature = "cpp-gnu-v2"
    ))]
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
    /// A parsed D symbol, rendered lazily.
    #[cfg(feature = "d")]
    D(crate::d::Ast<'a>),
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
    /// Plain, non-mangled text.
    #[allow(clippy::single_call_fn)]
    pub(crate) const fn plain(scheme: Scheme, text: &'a str) -> Self {
        Self {
            scheme,
            text: Text::Plain(text),
            #[cfg(feature = "coff")]
            coff_decoration: None,
        }
    }

    /// Raw non-UTF-8 bytes; rendered with invalid bytes escaped.
    #[allow(clippy::single_call_fn)]
    pub(crate) const fn bytes(scheme: Scheme, bytes: &'a [u8]) -> Self {
        Self {
            scheme,
            text: Text::Bytes(bytes),
            #[cfg(feature = "coff")]
            coff_decoration: None,
        }
    }

    /// A parsed Rust tree.
    #[cfg(feature = "rust")]
    #[allow(clippy::single_call_fn)]
    pub(crate) const fn rust(scheme: Scheme, text: crate::rust::Ast<'a>) -> Self {
        Self {
            scheme,
            text: Text::Rust(text),
            #[cfg(feature = "coff")]
            coff_decoration: None,
        }
    }

    /// A parsed C++ tree.
    #[cfg(any(
        feature = "cpp-itanium",
        feature = "cpp-msvc",
        feature = "cpp-gnu-v2"
    ))]
    #[allow(clippy::single_call_fn)]
    pub(crate) const fn cpp(scheme: Scheme, text: crate::cpp::Ast<'a>) -> Self {
        Self {
            scheme,
            text: Text::Cpp(text),
            #[cfg(feature = "coff")]
            coff_decoration: None,
        }
    }

    /// A parsed Swift tree.
    #[cfg(feature = "swift")]
    #[allow(clippy::single_call_fn)]
    pub(crate) const fn swift(scheme: Scheme, parsed: crate::swift::Ast<'a>) -> Self {
        Self {
            scheme,
            text: Text::Swift(parsed),
            #[cfg(feature = "coff")]
            coff_decoration: None,
        }
    }

    /// A parsed Go tree.
    #[cfg(feature = "go")]
    #[allow(clippy::single_call_fn)]
    pub(crate) const fn go(scheme: Scheme, parsed: crate::go::Ast<'a>) -> Self {
        Self {
            scheme,
            text: Text::Go(parsed),
            #[cfg(feature = "coff")]
            coff_decoration: None,
        }
    }

    /// A parsed Objective-C symbol.
    #[cfg(feature = "objc")]
    #[allow(clippy::single_call_fn)]
    pub(crate) const fn objc(scheme: Scheme, parsed: crate::objc::Ast<'a>) -> Self {
        Self {
            scheme,
            text: Text::ObjC(parsed),
            #[cfg(feature = "coff")]
            coff_decoration: None,
        }
    }

    /// A parsed Kotlin/Native symbol.
    #[cfg(feature = "kotlin-native")]
    #[allow(clippy::single_call_fn)]
    pub(crate) const fn kotlin_native(
        scheme: Scheme,
        parsed: crate::kotlin_native::Ast<'a>,
    ) -> Self {
        Self {
            scheme,
            text: Text::KotlinNative(parsed),
            #[cfg(feature = "coff")]
            coff_decoration: None,
        }
    }

    /// A parsed D tree.
    #[cfg(feature = "d")]
    #[allow(clippy::single_call_fn)]
    pub(crate) const fn d(scheme: Scheme, text: crate::d::Ast<'a>) -> Self {
        Self {
            scheme,
            text: Text::D(text),
            #[cfg(feature = "coff")]
            coff_decoration: None,
        }
    }

    /// A parsed JNI symbol.
    #[cfg(feature = "jni")]
    #[allow(clippy::single_call_fn)]
    pub(crate) const fn jni(scheme: Scheme, text: crate::jni::Ast<'a>) -> Self {
        Self {
            scheme,
            text: Text::Jni(text),
            #[cfg(feature = "coff")]
            coff_decoration: None,
        }
    }

    /// A parsed Ada symbol.
    #[cfg(feature = "ada")]
    #[allow(clippy::single_call_fn)]
    pub(crate) const fn ada(scheme: Scheme, text: crate::ada::Ast<'a>) -> Self {
        Self {
            scheme,
            text: Text::Ada(text),
            #[cfg(feature = "coff")]
            coff_decoration: None,
        }
    }

    /// A parsed Fortran symbol.
    #[cfg(feature = "fortran")]
    #[allow(clippy::single_call_fn)]
    pub(crate) const fn fortran(scheme: Scheme, text: crate::fortran::Ast<'a>) -> Self {
        Self {
            scheme,
            text: Text::Fortran(text),
            #[cfg(feature = "coff")]
            coff_decoration: None,
        }
    }

    /// A parsed Delphi/Object Pascal symbol.
    #[cfg(feature = "delphi")]
    #[allow(clippy::single_call_fn)]
    pub(crate) const fn delphi(scheme: Scheme, text: crate::delphi::Ast<'a>) -> Self {
        Self {
            scheme,
            text: Text::Delphi(text),
            #[cfg(feature = "coff")]
            coff_decoration: None,
        }
    }

    /// A parsed OCaml symbol.
    #[cfg(feature = "ocaml")]
    #[allow(clippy::single_call_fn)]
    pub(crate) const fn ocaml(scheme: Scheme, text: crate::ocaml::Ast<'a>) -> Self {
        Self {
            scheme,
            text: Text::OCaml(text),
            #[cfg(feature = "coff")]
            coff_decoration: None,
        }
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
            Text::Rust(text) => write!(f, "{text}"),
            #[cfg(any(
                feature = "cpp-itanium",
                feature = "cpp-msvc",
                feature = "cpp-gnu-v2"
            ))]
            Text::Cpp(text) => write!(f, "{text}"),
            #[cfg(feature = "swift")]
            Text::Swift(parsed) => parsed.print(f),
            #[cfg(feature = "go")]
            Text::Go(parsed) => parsed.print(f),
            #[cfg(feature = "objc")]
            Text::ObjC(parsed) => parsed.print(f),
            #[cfg(feature = "kotlin-native")]
            Text::KotlinNative(parsed) => parsed.print(f),
            #[cfg(feature = "d")]
            Text::D(parsed) => write!(f, "{parsed}"),
            #[cfg(feature = "jni")]
            Text::Jni(parsed) => write!(f, "{parsed}"),
            #[cfg(feature = "ada")]
            Text::Ada(parsed) => write!(f, "{parsed}"),
            #[cfg(feature = "fortran")]
            Text::Fortran(parsed) => write!(f, "{parsed}"),
            #[cfg(feature = "delphi")]
            Text::Delphi(parsed) => write!(f, "{parsed}"),
            #[cfg(feature = "ocaml")]
            Text::OCaml(parsed) => write!(f, "{parsed}"),
        }
    }
}
