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
/// Implements [`fmt::Display`] so it can be written to any sink. Every backend
/// retains its parsed tree and renders lazily; `Demangled::ast` borrows it.
///
/// # Alternate form
///
/// `{:#}` asks for a backend's compact form where it has one. Only Rust has
/// one: it omits the crate hash, as `rustc-demangle` does
/// (`main::main::h0123456789abcdef` becomes `main::main`). Every other
/// backend prints the same text for `{}` and `{:#}`.
///
/// # Rendering cost
///
/// A retained AST is rendered again on every [`fmt::Display`] call; the crate
/// does not cache the text. Store a `to_string()` result, or use a caller-owned
/// cache (for example an LRU), if the same value is printed repeatedly.
///
/// # Thread safety
///
/// C++, Rust and Swift retain `Rc`-owned node graphs, so `Demangled` is
/// `!Send + !Sync`. Demangle again on the target thread instead of moving a
/// value across threads.
#[derive(Debug, Clone)]
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
#[allow(
    clippy::large_enum_variant,
    reason = "fires only for feature subsets with one large AST; see above"
)]
#[derive(Debug, Clone)]
enum Text<'a> {
    /// The symbol is already readable (C, unknown, or pass-through).
    Plain(&'a str),
    /// Non-UTF-8 bytes passed through with escaped invalid bytes.
    Bytes(&'a [u8]),
    /// Parsed Rust tree, rendered lazily.
    #[cfg(feature = "rust")]
    Rust(crate::rust::Ast<'a>),
    /// A parsed D tree, rendered lazily.
    #[cfg(feature = "d")]
    D(crate::d::Ast<'a>),
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

    /// A parsed D tree.
    #[cfg(feature = "d")]
    pub(crate) const fn d(scheme: Scheme, parsed: crate::d::Ast<'a>) -> Self {
        Self::with(scheme, Text::D(parsed))
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
    /// [`Encoding::decode_identifier_escapes`]. See [`Encoding`](crate::Encoding) for
    /// the supported encodings and their scope.
    ///
    /// # Errors
    ///
    /// Returns [`EncodingError::Demangle`] if a retained tree cannot be
    /// rendered, [`EncodingError::InvalidEscape`] for a malformed escape, and
    /// [`EncodingError::InvalidEncoding`] when the escaped bytes are not valid
    /// in `encoding`.
    pub fn to_string_with_encoding(&self, encoding: Encoding) -> Result<String, EncodingError> {
        if let Text::Plain(text) = &self.text {
            return Ok(String::from(*text));
        }
        let mut rendered = String::new();
        fmt::Write::write_fmt(&mut rendered, format_args!("{self}")).map_err(|_| {
            EncodingError::Demangle(Error::parse(self.scheme, crate::ParseErrorKind::Invalid))
        })?;
        encoding.decode_identifier_escapes(&rendered)
    }

    /// Decoded PE/COFF decoration metadata, when the input was decorated.
    #[cfg(feature = "coff")]
    #[must_use]
    pub const fn coff_decoration(&self) -> Option<&crate::coff::Decoration<'a>> {
        self.coff_decoration.as_ref()
    }

    #[cfg(feature = "coff")]
    pub(crate) const fn with_coff_decoration(
        mut self,
        decoration: crate::coff::Decoration<'a>,
    ) -> Self {
        self.coff_decoration = Some(decoration);
        self
    }

    /// The detected mangling scheme.
    #[must_use]
    pub const fn scheme(&self) -> Scheme {
        self.scheme
    }

    /// The language associated with the detected scheme.
    #[must_use]
    pub const fn language(&self) -> Language {
        self.scheme.language()
    }

    /// The parsed tree, or `None` for plain C, unknown and pass-through names.
    #[cfg(any(
        feature = "cpp-itanium",
        feature = "cpp-msvc",
        feature = "cpp-gnu-v2",
        feature = "rust",
        feature = "swift",
        feature = "go",
        feature = "objc",
        feature = "kotlin-native",
        feature = "d",
        feature = "jni",
        feature = "ada",
        feature = "fortran",
        feature = "delphi",
        feature = "ocaml"
    ))]
    #[must_use]
    pub const fn ast(&self) -> Option<AstRef<'_, 'a>> {
        match &self.text {
            Text::Plain(_) | Text::Bytes(_) => None,
            #[cfg(any(feature = "cpp-itanium", feature = "cpp-msvc", feature = "cpp-gnu-v2"))]
            Text::Cpp(ast) => Some(AstRef::Cpp(ast)),
            #[cfg(feature = "rust")]
            Text::Rust(ast) => Some(AstRef::Rust(ast)),
            #[cfg(feature = "swift")]
            Text::Swift(ast) => Some(AstRef::Swift(ast)),
            #[cfg(feature = "go")]
            Text::Go(ast) => Some(AstRef::Go(ast)),
            #[cfg(feature = "objc")]
            Text::ObjC(ast) => Some(AstRef::ObjC(ast)),
            #[cfg(feature = "kotlin-native")]
            Text::KotlinNative(ast) => Some(AstRef::KotlinNative(ast)),
            #[cfg(feature = "d")]
            Text::D(ast) => Some(AstRef::D(ast)),
            #[cfg(feature = "jni")]
            Text::Jni(ast) => Some(AstRef::Jni(ast)),
            #[cfg(feature = "ada")]
            Text::Ada(ast) => Some(AstRef::Ada(ast)),
            #[cfg(feature = "fortran")]
            Text::Fortran(ast) => Some(AstRef::Fortran(ast)),
            #[cfg(feature = "delphi")]
            Text::Delphi(ast) => Some(AstRef::Delphi(ast)),
            #[cfg(feature = "ocaml")]
            Text::OCaml(ast) => Some(AstRef::OCaml(ast)),
        }
    }

    /// The C++ tree, when this is a C++ result.
    #[cfg(any(feature = "cpp-itanium", feature = "cpp-msvc", feature = "cpp-gnu-v2"))]
    #[must_use]
    pub const fn as_cpp(&self) -> Option<&crate::cpp::Ast<'a>> {
        match &self.text {
            Text::Cpp(ast) => Some(ast),
            _ => None,
        }
    }

    /// The Rust tree, when this is a Rust result.
    #[cfg(feature = "rust")]
    #[must_use]
    pub const fn as_rust(&self) -> Option<&crate::rust::Ast<'a>> {
        match &self.text {
            Text::Rust(ast) => Some(ast),
            _ => None,
        }
    }

    /// The Swift tree, when this is a Swift result.
    #[cfg(feature = "swift")]
    #[must_use]
    pub const fn as_swift(&self) -> Option<&crate::swift::Ast<'a>> {
        match &self.text {
            Text::Swift(ast) => Some(ast),
            _ => None,
        }
    }

    /// The Go tree, when this is a Go result.
    #[cfg(feature = "go")]
    #[must_use]
    pub const fn as_go(&self) -> Option<&crate::go::Ast<'a>> {
        match &self.text {
            Text::Go(ast) => Some(ast),
            _ => None,
        }
    }

    /// The Objective-C tree, when this is a Objective-C result.
    #[cfg(feature = "objc")]
    #[must_use]
    pub const fn as_objc(&self) -> Option<&crate::objc::Ast<'a>> {
        match &self.text {
            Text::ObjC(ast) => Some(ast),
            _ => None,
        }
    }

    /// The Kotlin/Native tree, when this is a Kotlin/Native result.
    #[cfg(feature = "kotlin-native")]
    #[must_use]
    pub const fn as_kotlin_native(&self) -> Option<&crate::kotlin_native::Ast<'a>> {
        match &self.text {
            Text::KotlinNative(ast) => Some(ast),
            _ => None,
        }
    }

    /// The D tree, when this is a D result.
    #[cfg(feature = "d")]
    #[must_use]
    pub const fn as_d(&self) -> Option<&crate::d::Ast<'a>> {
        match &self.text {
            Text::D(ast) => Some(ast),
            _ => None,
        }
    }

    /// The JNI tree, when this is a JNI result.
    #[cfg(feature = "jni")]
    #[must_use]
    pub const fn as_jni(&self) -> Option<&crate::jni::Ast<'a>> {
        match &self.text {
            Text::Jni(ast) => Some(ast),
            _ => None,
        }
    }

    /// The Ada tree, when this is a Ada result.
    #[cfg(feature = "ada")]
    #[must_use]
    pub const fn as_ada(&self) -> Option<&crate::ada::Ast<'a>> {
        match &self.text {
            Text::Ada(ast) => Some(ast),
            _ => None,
        }
    }

    /// The Fortran tree, when this is a Fortran result.
    #[cfg(feature = "fortran")]
    #[must_use]
    pub const fn as_fortran(&self) -> Option<&crate::fortran::Ast<'a>> {
        match &self.text {
            Text::Fortran(ast) => Some(ast),
            _ => None,
        }
    }

    /// The Delphi tree, when this is a Delphi result.
    #[cfg(feature = "delphi")]
    #[must_use]
    pub const fn as_delphi(&self) -> Option<&crate::delphi::Ast<'a>> {
        match &self.text {
            Text::Delphi(ast) => Some(ast),
            _ => None,
        }
    }

    /// The OCaml tree, when this is a OCaml result.
    #[cfg(feature = "ocaml")]
    #[must_use]
    pub const fn as_ocaml(&self) -> Option<&crate::ocaml::Ast<'a>> {
        match &self.text {
            Text::OCaml(ast) => Some(ast),
            _ => None,
        }
    }
}

/// A borrowed view of the parsed tree behind a [`Demangled`].
///
/// One variant per backend module; [`Demangled::scheme`] gives the precise
/// scheme (C++ covers six, Rust and Swift two each). The enum is
/// `#[non_exhaustive]`: enabling another backend feature anywhere in the build
/// adds a variant, so a `match` needs a wildcard arm.
///
/// ```
/// # #[cfg(feature = "cpp-itanium")] {
/// use demangler::{AstRef, Demangler};
///
/// let demangled = Demangler::new().demangle("_ZN3foo3barEv").unwrap();
/// match demangled.ast() {
///     Some(AstRef::Cpp(ast)) => assert_eq!(ast.to_string(), "foo::bar()"),
///     _ => unreachable!(),
/// }
/// # }
/// ```
#[cfg(any(
    feature = "cpp-itanium",
    feature = "cpp-msvc",
    feature = "cpp-gnu-v2",
    feature = "rust",
    feature = "swift",
    feature = "go",
    feature = "objc",
    feature = "kotlin-native",
    feature = "d",
    feature = "jni",
    feature = "ada",
    feature = "fortran",
    feature = "delphi",
    feature = "ocaml"
))]
#[non_exhaustive]
#[derive(Debug, Clone, Copy)]
pub enum AstRef<'d, 'a> {
    /// A C++ tree.
    #[cfg(any(feature = "cpp-itanium", feature = "cpp-msvc", feature = "cpp-gnu-v2"))]
    Cpp(&'d crate::cpp::Ast<'a>),
    /// A Rust tree.
    #[cfg(feature = "rust")]
    Rust(&'d crate::rust::Ast<'a>),
    /// A Swift tree.
    #[cfg(feature = "swift")]
    Swift(&'d crate::swift::Ast<'a>),
    /// A Go tree.
    #[cfg(feature = "go")]
    Go(&'d crate::go::Ast<'a>),
    /// A Objective-C tree.
    #[cfg(feature = "objc")]
    ObjC(&'d crate::objc::Ast<'a>),
    /// A Kotlin/Native tree.
    #[cfg(feature = "kotlin-native")]
    KotlinNative(&'d crate::kotlin_native::Ast<'a>),
    /// A D tree.
    #[cfg(feature = "d")]
    D(&'d crate::d::Ast<'a>),
    /// A JNI tree.
    #[cfg(feature = "jni")]
    Jni(&'d crate::jni::Ast<'a>),
    /// A Ada tree.
    #[cfg(feature = "ada")]
    Ada(&'d crate::ada::Ast<'a>),
    /// A Fortran tree.
    #[cfg(feature = "fortran")]
    Fortran(&'d crate::fortran::Ast<'a>),
    /// A Delphi tree.
    #[cfg(feature = "delphi")]
    Delphi(&'d crate::delphi::Ast<'a>),
    /// A OCaml tree.
    #[cfg(feature = "ocaml")]
    OCaml(&'d crate::ocaml::Ast<'a>),
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
            Text::D(parsed) => fmt::Display::fmt(parsed, f),
            #[cfg(any(feature = "cpp-itanium", feature = "cpp-msvc", feature = "cpp-gnu-v2"))]
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
