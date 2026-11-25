#[cfg(any(
    feature = "encoding",
    feature = "d",
    feature = "cpp-itanium",
    feature = "cpp-msvc",
    feature = "cpp-gnu-v2",
    feature = "rust",
    feature = "swift"
))]
use alloc::string::String;

#[cfg(feature = "encoding")]
use crate::encoding::{Encoding, EncodingError};
use crate::error::Error;
use crate::result::Demangled;
use crate::scheme::{Language, Scheme};
use crate::selection::Selection;

/// Demangles symbol names using the appropriate demangling scheme.
///
/// Detects the scheme from the symbol name and dispatches to the appropriate
/// in-tree backend. No external demangler is invoked at runtime.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Demangler;

impl Demangler {
    /// Create a new [`Demangler`].
    pub const fn new() -> Self {
        Self
    }

    /// Start a [`Selection`] of candidate schemes.
    ///
    /// An empty selection behaves exactly like [`Self::demangle`]; add targets
    /// with [`Selection::allow`] or [`Selection::prefer`] to restrict or order
    /// the candidates.
    pub const fn selection(&self) -> Selection {
        Selection::new(*self)
    }

    /// Detect the scheme and demangle a UTF-8 symbol name.
    ///
    /// The returned [`Demangled`] implements [`core::fmt::Display`]. Swift
    /// retains the parsed node tree and prints it lazily; other backends
    /// borrow from `symbol` or store owned text.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidSymbol`] when the backend cannot demangle the
    /// input. [`Error::Unsupported`] is kept for API stability but is no
    /// longer produced: every detected scheme has an in-tree backend.
    pub fn demangle<'a>(&self, symbol: &'a str) -> Result<Demangled<'a>, Error> {
        self.demangle_bytes(symbol.as_bytes())
    }

    /// Demangle a UTF-8 symbol name with an explicitly selected scheme.
    ///
    /// This is the text twin of [`Self::demangle`]: it bypasses
    /// [`Scheme::detect`] while still validating the selected backend:
    /// `scheme` must name a backend compiled into this build, and a name the
    /// backend rejects returns [`Error::InvalidSymbol`]. The scheme is exact:
    /// a C++ dialect or a rustc/Swift encoding the parser did not produce is
    /// an [`Error::InvalidSymbol`], never a fallback to another scheme.
    /// It is the integration point for names whose automatic detection is
    /// intentionally conservative: general Ada external names (`pkg__subp`),
    /// gfortran external names (`foo_`), namespaced Free Pascal units and OCaml
    /// external names.
    ///
    /// [`Scheme::C`] and [`Scheme::Unknown`] are pass-through schemes and
    /// return the input as plain text. COFF decorations recognised by
    /// `crate::coff::parse_bytes` are peeled before dispatch, exactly as in
    /// [`Self::demangle_bytes`]; target-specific forms (a bare x86 `_name`, a
    /// `name@@N` vectorcall suffix or a `.weak.` wrapper) are decoded first
    /// with `crate::coff::parse_bytes_for`, and its `Decoration::symbol` is
    /// passed here.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidSymbol`] when the selected backend is not
    /// compiled in or cannot demangle the input.
    pub fn demangle_with_scheme<'a>(
        &self,
        scheme: Scheme,
        symbol: &'a str,
    ) -> Result<Demangled<'a>, Error> {
        self.demangle_bytes_with_scheme(scheme, symbol.as_bytes())
    }

    /// Demangle a raw symbol byte string with an explicitly selected scheme.
    ///
    /// Raw-byte twin of [`Self::demangle_with_scheme`]; see that method for
    /// the detection-bypass and error semantics. Identifier bytes that are not
    /// valid UTF-8 are retained and rendered as `\xNN`, as in
    /// [`Self::demangle_bytes`].
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidSymbol`] when the selected backend is not
    /// compiled in or cannot demangle the input.
    pub fn demangle_bytes_with_scheme<'a>(
        &self,
        scheme: Scheme,
        symbol: &'a [u8],
    ) -> Result<Demangled<'a>, Error> {
        self.demangle_bytes_selected(scheme, symbol, false)
    }

    /// Demangle a UTF-8 symbol name with every scheme of one language.
    ///
    /// The group is tried in [`Language::schemes`] order, each member through
    /// the exact-scheme path.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidSymbol`] when the language has one scheme and it
    /// rejects the symbol, and [`Error::NoMatchingScheme`] when several members
    /// were tried and all of them failed.
    pub fn demangle_with_language<'a>(
        &self,
        language: Language,
        symbol: &'a str,
    ) -> Result<Demangled<'a>, Error> {
        self.selection().allow(language).demangle(symbol)
    }

    /// Demangle a raw symbol byte string with every scheme of one language.
    ///
    /// Raw-byte twin of [`Self::demangle_with_language`].
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidSymbol`] when the language has one scheme and it
    /// rejects the symbol, and [`Error::NoMatchingScheme`] when several members
    /// were tried and all of them failed.
    pub fn demangle_bytes_with_language<'a>(
        &self,
        language: Language,
        symbol: &'a [u8],
    ) -> Result<Demangled<'a>, Error> {
        self.selection().allow(language).demangle_bytes(symbol)
    }

    /// Demangle a UTF-8 symbol name with every scheme of the given languages.
    ///
    /// Languages are tried in slice order; each language expands in
    /// [`Language::schemes`] order. An empty slice is an empty selection, which
    /// behaves exactly like [`Self::demangle`].
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidSymbol`] when exactly one scheme was a candidate
    /// and it rejected the symbol, and [`Error::NoMatchingScheme`] when several
    /// candidates were tried and all of them failed.
    pub fn demangle_with_languages<'a>(
        &self,
        languages: &[Language],
        symbol: &'a str,
    ) -> Result<Demangled<'a>, Error> {
        self.selection().prefer_languages(languages).demangle(symbol)
    }

    /// Demangle a raw symbol byte string with every scheme of the given
    /// languages.
    ///
    /// Raw-byte twin of [`Self::demangle_with_languages`].
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidSymbol`] when exactly one scheme was a candidate
    /// and it rejected the symbol, and [`Error::NoMatchingScheme`] when several
    /// candidates were tried and all of them failed.
    pub fn demangle_bytes_with_languages<'a>(
        &self,
        languages: &[Language],
        symbol: &'a [u8],
    ) -> Result<Demangled<'a>, Error> {
        self.selection()
            .prefer_languages(languages)
            .demangle_bytes(symbol)
    }

    /// Demangle a UTF-8 symbol name with the given schemes.
    ///
    /// Schemes are tried in slice order. An empty slice is an empty selection,
    /// which behaves exactly like [`Self::demangle`].
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidSymbol`] when exactly one scheme was a candidate
    /// and it rejected the symbol, and [`Error::NoMatchingScheme`] when several
    /// candidates were tried and all of them failed.
    pub fn demangle_with_schemes<'a>(
        &self,
        schemes: &[Scheme],
        symbol: &'a str,
    ) -> Result<Demangled<'a>, Error> {
        self.selection().prefer_schemes(schemes).demangle(symbol)
    }

    /// Demangle a raw symbol byte string with the given schemes.
    ///
    /// Raw-byte twin of [`Self::demangle_with_schemes`].
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidSymbol`] when exactly one scheme was a candidate
    /// and it rejected the symbol, and [`Error::NoMatchingScheme`] when several
    /// candidates were tried and all of them failed.
    pub fn demangle_bytes_with_schemes<'a>(
        &self,
        schemes: &[Scheme],
        symbol: &'a [u8],
    ) -> Result<Demangled<'a>, Error> {
        self.selection().prefer_schemes(schemes).demangle_bytes(symbol)
    }

    /// Peel PE/COFF decorations and dispatch to the selected backend.
    ///
    /// Both byte entry points share this path, so a decoration is decoded and
    /// reported exactly once. `allow_rust_fallback` keeps the detected path's
    /// legacy Rust fallback while a forced C++ scheme stays C++.
    fn demangle_bytes_selected<'a>(
        &self,
        scheme: Scheme,
        symbol: &'a [u8],
        allow_rust_fallback: bool,
    ) -> Result<Demangled<'a>, Error> {
        #[cfg(feature = "coff")]
        if let Some(decoration) = crate::coff::parse_bytes(symbol) {
            let inner =
                self.demangle_bytes_selected(scheme, decoration.symbol(), allow_rust_fallback)?;
            return Ok(inner.with_coff_decoration(decoration));
        }
        self.dispatch_bytes(scheme, symbol, allow_rust_fallback)
    }

    #[cfg(feature = "encoding")]
    /// Demangle a raw symbol byte string and decode its identifier escapes
    /// with an explicit character encoding.
    ///
    /// The symbol is demangled with [`Self::demangle_bytes`], then the
    /// rendered identifier text is decoded by
    /// [`Encoding::decode_identifier_escapes`]. The crate never transcodes
    /// inside a backend AST, and plain C/unknown names are already Unicode and
    /// are returned unchanged. See [`crate::encoding`] for the supported
    /// encodings and the rejected legacy code pages.
    ///
    /// # Errors
    ///
    /// Returns [`EncodingError::Demangle`] when the symbol cannot be
    /// demangled, and [`EncodingError::InvalidEscape`] or
    /// [`EncodingError::InvalidEncoding`] when the rendered escapes cannot be
    /// decoded.
    pub fn demangle_bytes_to_string_with_encoding(
        &self,
        symbol: &[u8],
        encoding: Encoding,
    ) -> Result<String, EncodingError> {
        self.demangle_bytes(symbol)?.to_string_with_encoding(encoding)
    }

    /// Demangle a symbol name read from a Mach-O symbol table.
    ///
    /// A Mach-O symbol table prefixes every external symbol with one
    /// underscore, so the C function `main` is stored as `_main`. The prefix
    /// is decoded only when no backend claims the name as written and the
    /// remainder is a plain C identifier:
    ///
    /// - `_main` is the C symbol `main`;
    /// - `__Z3foov` (Mach-O C++), `_$s...` (Mach-O Swift), `_OBJC_*` metadata
    ///   and `_RN...`/`_ZN...` Rust names keep their own scheme, because the
    ///   platform prefix is already part of their detection;
    /// - an assembly label (`_x.y`), a name whose remainder is not a plain C
    ///   identifier (`_123`, `__foo`) and a name with non-ASCII identifier
    ///   bytes are returned unchanged.
    ///
    /// [`Self::demangle`] and [`Self::demangle_bytes`] never strip the prefix,
    /// so an ordinary C name is never reclassified by accident. Linker
    /// fragment suffixes (`.cold`, `.part.N`) are not decoded here; declare a
    /// COFF target and use `crate::coff::parse_bytes_for` for those.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidSymbol`] when a detected backend cannot
    /// demangle the name; an unclaimed name is pass-through text.
    pub fn demangle_macho_bytes<'a>(&self, symbol: &'a [u8]) -> Result<Demangled<'a>, Error> {
        let detection = crate::detection::detect(symbol);
        if detection.scheme == Scheme::Unknown
            && let Some(rest) = symbol.strip_prefix(b"_")
            && crate::scheme::is_plain_c_name(rest)
            && let Ok(text) = core::str::from_utf8(rest)
        {
            return Ok(Demangled::plain(Scheme::C, text));
        }
        self.demangle_detection(detection, true)
    }

    /// Demangle a UTF-8 symbol name read from a Mach-O symbol table.
    ///
    /// UTF-8 wrapper around [`Self::demangle_macho_bytes`]; see that method for
    /// the decoded prefix and its limits.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidSymbol`] when a detected backend cannot
    /// demangle the name; an unclaimed name is pass-through text.
    pub fn demangle_macho<'a>(&self, symbol: &'a str) -> Result<Demangled<'a>, Error> {
        self.demangle_macho_bytes(symbol.as_bytes())
    }

    /// Demangle a raw symbol byte string.
    ///
    /// Object-file symbol names are byte strings, not UTF-8 strings. Every
    /// backend parses bytes directly. Invalid UTF-8 in identifier text is
    /// retained and rendered as `\xNN`; compiler payload text follows its
    /// documented verifier policy. A name that no backend recognizes is
    /// returned as escaped plain text with [`Scheme::Unknown`].
    pub fn demangle_bytes<'a>(&self, symbol: &'a [u8]) -> Result<Demangled<'a>, Error> {
        let detection = crate::detection::detect(symbol);
        self.demangle_detection(detection, true)
    }

    /// Dispatch an already selected scheme to its backend.
    /// Dispatch a detection result, reusing a tree that detection parsed.
    fn demangle_detection<'a>(
        &self,
        detection: crate::detection::Detection<'a>,
        allow_rust_fallback: bool,
    ) -> Result<Demangled<'a>, Error> {
        let crate::detection::Detection {
            scheme,
            symbol,
            parse_symbol,
            parsed,
            #[cfg(feature = "coff")]
            decoration,
        } = detection;
        let result = match parsed {
            Some(parsed) => {
                self.dispatch_parsed(parsed, parse_symbol, allow_rust_fallback)?
            }
            None => self.dispatch_bytes(scheme, symbol, allow_rust_fallback)?,
        };
        #[cfg(feature = "coff")]
        if let Some(decoration) = decoration {
            return Ok(result.with_coff_decoration(decoration));
        }
        Ok(result)
    }

    /// Render a parsed tree once into validated owned text.
    #[cfg(any(
        feature = "cpp-itanium",
        feature = "cpp-msvc",
        feature = "cpp-gnu-v2",
        feature = "d",
        feature = "rust",
        feature = "swift"
    ))]
    fn render_owned<'a, T: core::fmt::Display + ?Sized>(
        scheme: Scheme,
        parsed: &T,
    ) -> Result<Demangled<'a>, Error> {
        let mut sink = String::new();
        if core::fmt::Write::write_fmt(&mut sink, format_args!("{parsed}")).is_ok() {
            Ok(Demangled::owned(scheme, sink))
        } else {
            Err(Error::InvalidSymbol(scheme))
        }
    }

    /// Render a parsed Swift tree once into validated owned text.
    #[cfg(feature = "swift")]
    fn render_swift_owned<'a>(
        scheme: Scheme,
        parsed: &crate::swift::Ast<'_>,
    ) -> Result<Demangled<'a>, Error> {
        let mut sink = String::new();
        if parsed.print(&mut sink).is_ok() {
            Ok(Demangled::owned(scheme, sink))
        } else {
            Err(Error::InvalidSymbol(scheme))
        }
    }

    /// Dispatch a tree already parsed during detection.
    ///
    /// The legacy C++/Swift parses and the D/OCaml/Delphi/JNI prefilters
    /// retain their trees, so the detected path parses each symbol once. The
    /// AST is consumed without cloning; a failed C++ render keeps the
    /// detected-path Rust fallback on the same stripped byte view.
    fn dispatch_parsed<'a>(
        &self,
        parsed: crate::detection::Parsed<'a>,
        parse_symbol: &'a [u8],
        allow_rust_fallback: bool,
    ) -> Result<Demangled<'a>, Error> {
        #[cfg(not(any(
            feature = "cpp-itanium",
            feature = "cpp-msvc",
            feature = "cpp-gnu-v2"
        )))]
        let _ = (parse_symbol, allow_rust_fallback);
        match parsed {
            #[cfg(any(
                feature = "cpp-itanium",
                feature = "cpp-msvc",
                feature = "cpp-gnu-v2"
            ))]
            crate::detection::Parsed::Cpp(parsed) => {
                let scheme = crate::scheme::scheme_for_cpp_dialect(parsed.dialect());
                let mut sink = String::new();
                if core::fmt::Write::write_fmt(&mut sink, format_args!("{parsed}")).is_ok() {
                    return Ok(Demangled::owned(scheme, sink));
                }
                #[cfg(feature = "rust")]
                if allow_rust_fallback
                    && let Ok(parsed) = crate::rust::parse_bytes_unvalidated(parse_symbol)
                {
                    let scheme = crate::scheme::scheme_for_rust_encoding(parsed.encoding());
                    if let Ok(demangled) = Self::render_owned(scheme, &parsed) {
                        return Ok(demangled);
                    }
                }
                Err(Error::InvalidSymbol(scheme))
            }
            #[cfg(feature = "swift")]
            crate::detection::Parsed::Swift(parsed) => {
                let scheme = crate::scheme::scheme_for_swift_encoding(parsed.encoding());
                Self::render_swift_owned(scheme, &parsed)
            }
            #[cfg(feature = "d")]
            crate::detection::Parsed::D(parsed) => {
                // Back references can expand a name, so keep the render
                // validation until printer totality is proven.
                let mut sink = String::new();
                if core::fmt::Write::write_fmt(&mut sink, format_args!("{parsed}")).is_ok() {
                    Ok(Demangled::owned(Scheme::D, sink))
                } else {
                    Err(Error::InvalidSymbol(Scheme::D))
                }
            }
            #[cfg(feature = "ocaml")]
            crate::detection::Parsed::OCaml(parsed) => {
                Ok(Demangled::ocaml(Scheme::OCaml, parsed))
            }
            #[cfg(feature = "delphi")]
            crate::detection::Parsed::Delphi(parsed) => {
                Ok(Demangled::delphi(Scheme::Delphi, parsed))
            }
            #[cfg(feature = "jni")]
            crate::detection::Parsed::Jni(parsed) => Ok(Demangled::jni(Scheme::Jni, parsed)),
            #[allow(unreachable_patterns)]
            _ => Err(Error::InvalidSymbol(Scheme::Unknown)),
        }
    }

    /// Dispatch an already selected scheme to its backend.
    ///
    /// A scheme is exact: a C++ dialect, rustc encoding or Swift mangling the
    /// backend did not produce is [`Error::InvalidSymbol`]. The detected path
    /// (`allow_rust_fallback`) keeps the historical C++ to Rust fallback: when
    /// a C++ candidate fails, the same bytes are parsed as a rustc symbol and
    /// reported with the precise encoding that parsed.
    pub(crate) fn dispatch_bytes<'a>(
        &self,
        scheme: Scheme,
        symbol: &'a [u8],
        allow_rust_fallback: bool,
    ) -> Result<Demangled<'a>, Error> {
        #[cfg(not(all(
            feature = "rust",
            any(feature = "cpp-itanium", feature = "cpp-msvc", feature = "cpp-gnu-v2")
        )))]
        let _ = allow_rust_fallback;
        match scheme {
            Scheme::C | Scheme::Unknown => {
                core::str::from_utf8(symbol).map_or_else(
                    |_| Ok(Demangled::bytes(scheme, symbol)),
                    |text| Ok(Demangled::plain(scheme, text)),
                )
            }
            #[cfg(feature = "rust")]
            Scheme::RustLegacy | Scheme::RustV0 => {
                let stripped = crate::scheme::strip_decorations_bytes(symbol);
                match crate::rust::parse_bytes_unvalidated(stripped) {
                    Ok(parsed)
                        if crate::scheme::scheme_for_rust_encoding(parsed.encoding()) == scheme =>
                    {
                        Self::render_owned(scheme, &parsed)
                    }
                    _ => Err(Error::InvalidSymbol(scheme)),
                }
            }
            #[cfg(any(
                feature = "cpp-itanium",
                feature = "cpp-msvc",
                feature = "cpp-gnu-v2"
            ))]
            Scheme::CppItanium
            | Scheme::CppMsvc
            | Scheme::CppGnuV2
            | Scheme::CppArm
            | Scheme::CppEdg
            | Scheme::CppHp => {
                let stripped = crate::scheme::strip_thin_lto_bytes(symbol);
                let stripped = if stripped.starts_with(b"?") || stripped.starts_with(b".?") {
                    stripped
                } else {
                    crate::scheme::strip_version_suffix_bytes(stripped)
                };
                if let Ok(parsed) = crate::cpp::parse_bytes(stripped)
                    && crate::scheme::scheme_for_cpp_dialect(parsed.dialect()) == scheme
                {
                    // Keep the validation pass until printer totality is
                    // proven; a retained AST must not fail later in Display.
                    let mut sink = String::new();
                    if core::fmt::Write::write_fmt(&mut sink, format_args!("{parsed}")).is_ok() {
                        return Ok(Demangled::owned(scheme, sink));
                    }
                }
                #[cfg(feature = "rust")]
                if allow_rust_fallback
                    && let Ok(parsed) = crate::rust::parse_bytes_unvalidated(stripped)
                {
                    let scheme = crate::scheme::scheme_for_rust_encoding(parsed.encoding());
                    if let Ok(demangled) = Self::render_owned(scheme, &parsed) {
                        return Ok(demangled);
                    }
                }
                Err(Error::InvalidSymbol(scheme))
            }
            #[cfg(feature = "swift")]
            Scheme::SwiftStable | Scheme::SwiftLegacy => {
                let stripped = crate::scheme::strip_thin_lto_bytes(symbol);
                match crate::swift::parse_bytes_unvalidated(stripped) {
                    Ok(parsed)
                        if crate::scheme::scheme_for_swift_encoding(parsed.encoding()) == scheme =>
                    {
                        Self::render_swift_owned(scheme, &parsed)
                    }
                    _ => Err(Error::InvalidSymbol(scheme)),
                }
            }
            #[cfg(feature = "go")]
            Scheme::Go => crate::go::parse_bytes(symbol)
                .map(|parsed| Demangled::go(scheme, parsed))
                .map_err(|_| Error::InvalidSymbol(scheme)),
            #[cfg(feature = "kotlin-native")]
            Scheme::KotlinNative => {
                let stripped = crate::scheme::strip_thin_lto_bytes(symbol);
                crate::kotlin_native::parse_bytes(stripped)
                    .map(|parsed| Demangled::kotlin_native(scheme, parsed))
                    .map_err(|_| Error::InvalidSymbol(scheme))
            }
            #[cfg(feature = "d")]
            Scheme::D => {
                let stripped = crate::scheme::strip_decorations_bytes(symbol);
                let parsed =
                    crate::d::parse_bytes(stripped).map_err(|_| Error::InvalidSymbol(scheme))?;
                // Identifier back references expand an earlier name, so a
                // short symbol can exceed the printer's `MAX_OUTPUT_LEN`. Keep
                // the validation pass until printer totality is proven; a
                // retained AST must not fail later in Display.
                let mut sink = String::new();
                if core::fmt::Write::write_fmt(&mut sink, format_args!("{parsed}")).is_err() {
                    return Err(Error::InvalidSymbol(scheme));
                }
                Ok(Demangled::owned(scheme, sink))
            }
            #[cfg(feature = "jni")]
            Scheme::Jni => {
                let stripped = crate::scheme::strip_decorations_bytes(symbol);
                crate::jni::parse_bytes(stripped)
                    .map(|parsed| Demangled::jni(scheme, parsed))
                    .map_err(|_| Error::InvalidSymbol(scheme))
            }
            #[cfg(feature = "ada")]
            Scheme::Ada => crate::ada::parse_bytes(symbol)
                .map(|parsed| Demangled::ada(scheme, parsed))
                .map_err(|_| Error::InvalidSymbol(scheme)),
            #[cfg(feature = "fortran")]
            Scheme::Fortran => crate::fortran::parse_bytes(symbol)
                .map(|parsed| Demangled::fortran(scheme, parsed))
                .map_err(|_| Error::InvalidSymbol(scheme)),
            #[cfg(feature = "delphi")]
            Scheme::Delphi => crate::delphi::parse_bytes(symbol)
                .map(|parsed| Demangled::delphi(scheme, parsed))
                .map_err(|_| Error::InvalidSymbol(scheme)),
            #[cfg(feature = "ocaml")]
            Scheme::OCaml => crate::ocaml::parse_bytes(symbol)
                .map(|parsed| Demangled::ocaml(scheme, parsed))
                .map_err(|_| Error::InvalidSymbol(scheme)),
            #[cfg(feature = "objc")]
            Scheme::ObjC => {
                let stripped = crate::scheme::strip_decorations_bytes(symbol);
                let stripped = stripped
                    .iter()
                    .position(|byte| *byte == b'@')
                    .map_or(stripped, |index| &stripped[..index]);
                crate::objc::parse_bytes(stripped)
                    .map(|parsed| Demangled::objc(scheme, parsed))
                    .map_err(|_| Error::InvalidSymbol(scheme))
            }
            #[allow(unreachable_patterns)]
            _ => Err(Error::InvalidSymbol(scheme)),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::string::ToString;

    #[cfg(any(feature = "cpp-itanium", feature = "d"))]
    use std::string::String;

    use super::*;

    #[cfg(feature = "rust")]
    #[test]
    fn detects_rust() {
        assert_eq!(
            Scheme::detect("_RN4main6hello17he3c1b56a8e1b2df5E"),
            Scheme::RustV0
        );
    }

    #[cfg(feature = "cpp-itanium")]
    #[test]
    fn detects_cpp_itanium() {
        assert_eq!(Scheme::detect("_ZN3Foo3barEv"), Scheme::CppItanium);
    }

    #[cfg(feature = "swift")]
    #[test]
    fn detects_swift() {
        assert_eq!(Scheme::detect("$s4main3fooSi_yF"), Scheme::SwiftStable);
    }

    #[test]
    fn detects_c() {
        assert_eq!(Scheme::detect("main"), Scheme::C);
    }

    #[cfg(feature = "objc")]
    #[test]
    fn detects_objc() {
        for symbol in [
            "_OBJC_CLASS_$_NSString",
            "__OBJC_CLASS_$_NSString",
            "OBJC_CLASS_$_NSString",
            "_OBJC_METACLASS_$_NSString",
            "__OBJC_$_INSTANCE_METHODS_Foo",
            "__OBJC_$_CATEGORY_Foo_$_Bar",
            "__OBJC_PROTOCOL_$_NSCopying",
            "_OBJC_IVAR_$_Foo._ivar",
            "__objc_class_name_Foo",
            "_OBJC_CLASS_Foo",
            "-[Foo bar:baz:]",
            "+[Foo classThing]",
            "___block_descriptor_40_e8_32r_e5_v8?0l",
        ] {
            assert_eq!(Scheme::detect(symbol), Scheme::ObjC, "{symbol}");
            assert_eq!(
                Scheme::detect(symbol).language(),
                Language::ObjectiveC,
                "{symbol}"
            );
        }
    }

    #[cfg(feature = "objc")]
    #[test]
    fn objc_does_not_steal_other_schemes() {
        // Objective-C++ uses Itanium C++ mangling; `_Z` must stay C++.
        #[cfg(feature = "cpp-itanium")]
        {
            assert_eq!(Scheme::detect("_Z3foov"), Scheme::CppItanium);
            assert_eq!(Scheme::detect("__Z3foov"), Scheme::CppItanium);
        }
        #[cfg(feature = "rust")]
        assert_eq!(Scheme::detect("_RNvCs1234_4test4main"), Scheme::RustV0);
        #[cfg(feature = "swift")]
        assert_eq!(Scheme::detect("$s4main3fooSi_yF"), Scheme::SwiftStable);
        #[cfg(feature = "go")]
        assert_eq!(Scheme::detect("fmt.Println"), Scheme::Go);
        // Generic GNUstep runtime helpers are not metadata.
        assert_ne!(Scheme::detect("__objc_exec_class"), Scheme::ObjC);
        // An unknown Apple metadata kind is not claimed.
        assert_ne!(Scheme::detect("_OBJC_NOPE_$_Foo"), Scheme::ObjC);
        // A known prefix with a malformed body is claimed and then rejected.
        assert_eq!(Scheme::detect("_OBJC_CLASS_$_"), Scheme::ObjC);
    }

    #[cfg(feature = "objc")]
    #[test]
    fn demangles_objc() {
        let demangler = Demangler::new();
        for (symbol, expected) in [
            ("_OBJC_CLASS_$_NSString", "Objective-C class NSString"),
            (
                "__OBJC_$_CATEGORY_NSString_$_Drawing",
                "Objective-C category NSString (Drawing)",
            ),
            (
                "__OBJC_$_INSTANCE_METHODS_Foo",
                "Objective-C instance methods Foo",
            ),
            ("__objc_class_name_Foo", "Objective-C class Foo"),
            ("-[Foo bar:baz:]", "-[Foo bar:baz:]"),
        ] {
            let demangled = demangler
                .demangle(symbol)
                .unwrap_or_else(|error| panic!("{symbol}: {error}"));
            assert_eq!(demangled.scheme(), Scheme::ObjC, "{symbol}");
            assert_eq!(
                demangled.language(),
                Language::ObjectiveC,
                "{symbol}"
            );
            assert_eq!(demangled.to_string(), expected, "{symbol}");
        }
    }

    #[cfg(feature = "objc")]
    #[test]
    fn rejects_malformed_objc() {
        let demangler = Demangler::new();
        // A known prefix with a malformed body is a detected Objective-C name
        // that the backend refuses, not a plain-C pass-through.
        assert!(matches!(
            demangler.demangle("_OBJC_CLASS_$_"),
            Err(Error::InvalidSymbol(Scheme::ObjC))
        ));
        // An unknown kind under the Apple namespace is not claimed at all; it
        // stays unknown instead of being guessed by the legacy C++ dialects.
        assert_ne!(Scheme::detect("_OBJC_NOPE_$_Foo"), Scheme::ObjC);
        let unknown = demangler
            .demangle("_OBJC_NOPE_$_Foo")
            .expect("unknown namespace name is plain text");
        assert_eq!(unknown.scheme(), Scheme::Unknown);
        assert_eq!(unknown.to_string(), "_OBJC_NOPE_$_Foo");
        // A generic `__objc_*` helper is not metadata either.
        assert_ne!(Scheme::detect("__objc_exec_class"), Scheme::ObjC);
        assert_eq!(
            demangler.demangle("__objc_exec_class").unwrap().to_string(),
            "__objc_exec_class"
        );
    }

    #[cfg(feature = "kotlin-native")]
    #[test]
    fn detects_kotlin_native() {
        for symbol in [
            "kfun:com.example#main(){}",
            "ktype:com.example.Box<kotlin.Int>",
            "kclass:com.example.Box",
            "kproperty:com.example.Box#value",
            "kobj:com.example.Singleton",
            "kfield:com.example.Box.value",
            "ktypealias:com.example.IntList",
            "kpackage:com.example",
            "knamespace:com.example",
            "kvar:com.example.Singleton.$instance",
            "_kfun:com.example#main(){}",
            "__kfun:com.example#main(){}",
        ] {
            assert_eq!(Scheme::detect(symbol), Scheme::KotlinNative, "{symbol}");
            assert_eq!(
                Scheme::detect(symbol).language(),
                Language::KotlinNative,
                "{symbol}"
            );
        }
        // Unknown `k...` markers are not guessed.
        for symbol in ["kfoo:com.example", "kintf:com.example", "kclass"] {
            assert_ne!(Scheme::detect(symbol), Scheme::KotlinNative, "{symbol}");
        }
    }

    #[cfg(feature = "kotlin-native")]
    #[test]
    fn demangles_kotlin_native() {
        let demangler = Demangler::new();
        for (symbol, expected) in [
            (
                "kfun:com.example.foo#bar(kotlin.Int){}",
                "com.example.foo.bar(kotlin.Int)",
            ),
            (
                "kfun:com.example.Box#<init>(kotlin.Int){}",
                "com.example.Box.<init>(kotlin.Int)",
            ),
            ("kfun:com.example.Box#get(){}", "com.example.Box.get()"),
            ("kfun:com.example#topLevel(){}", "com.example.topLevel()"),
            (
                "ktype:com.example.Box<kotlin.Int>",
                "com.example.Box<kotlin.Int>",
            ),
        ] {
            let demangled = demangler
                .demangle(symbol)
                .unwrap_or_else(|error| panic!("{symbol}: {error}"));
            assert_eq!(demangled.scheme(), Scheme::KotlinNative, "{symbol}");
            assert_eq!(
                demangled.language(),
                Language::KotlinNative,
                "{symbol}"
            );
            assert_eq!(demangled.to_string(), expected, "{symbol}");
        }
    }

    #[cfg(feature = "kotlin-native")]
    #[test]
    fn rejects_malformed_kotlin_native() {
        let demangler = Demangler::new();
        for symbol in [
            "kfun:",
            "kfun:com.example#foo#nope(){}",
            "kfun:com.example#foo(kotlin.Int",
            "kclass:",
            "ktype:kotlin.Array<",
        ] {
            assert!(
                matches!(
                    demangler.demangle(symbol),
                    Err(Error::InvalidSymbol(Scheme::KotlinNative))
                ),
                "{symbol}"
            );
        }
    }

    #[cfg(feature = "kotlin-native")]
    #[test]
    fn kotlin_native_does_not_steal_other_schemes() {
        #[cfg(feature = "cpp-itanium")]
        assert_eq!(Scheme::detect("_Z3foov"), Scheme::CppItanium);
        #[cfg(feature = "rust")]
        assert_eq!(Scheme::detect("_RNvCs1234_4test4main"), Scheme::RustV0);
        #[cfg(feature = "swift")]
        assert_eq!(Scheme::detect("$s4main3fooSi_yF"), Scheme::SwiftStable);
        #[cfg(feature = "objc")]
        assert_eq!(Scheme::detect("_OBJC_CLASS_$_NSString"), Scheme::ObjC);
        #[cfg(feature = "go")]
        assert_eq!(Scheme::detect("fmt.Println"), Scheme::Go);
    }

    #[cfg(not(feature = "kotlin-native"))]
    #[test]
    fn kotlin_native_is_not_detected_without_its_feature() {
        assert_ne!(Scheme::detect("kfun:com.example#main(){}"), Scheme::KotlinNative);
        assert_ne!(Scheme::detect("kclass:com.example.Box"), Scheme::KotlinNative);
    }

    #[cfg(not(feature = "objc"))]
    #[test]
    fn objc_is_not_detected_without_its_feature() {
        for symbol in [
            "_OBJC_CLASS_$_NSString",
            "__OBJC_$_INSTANCE_METHODS_Foo",
            "__objc_class_name_Foo",
            "-[Foo bar:]",
        ] {
            assert_ne!(Scheme::detect(symbol), Scheme::ObjC, "{symbol}");
        }
    }

    #[cfg(feature = "cpp-gnu-v2")]
    #[test]
    fn detects_legacy_gnu_v2() {
        for symbol in [
            "foo__Fiv",
            "bar__3foo",
            "__3foo",
            "_$_3foo",
            "_3foo$bar",
            "_vt$foo",
            "__ti3foo",
        ] {
            assert_eq!(Scheme::detect(symbol), Scheme::CppGnuV2, "{symbol}");
        }
    }

    #[test]
    fn plain_names_with_separators_stay_c() {
        // A `__` alone does not make a name legacy C++: the legacy parser has
        // to consume the whole symbol before the dialect is claimed.
        for symbol in ["foo__bar", "foo_bar", "foo", "main"] {
            assert_eq!(Scheme::detect(symbol), Scheme::C, "{symbol}");
        }
        assert_ne!(Scheme::detect("__libc_start_main"), Scheme::CppGnuV2);
    }

    #[cfg(all(
        feature = "cpp-itanium",
        feature = "cpp-msvc",
        feature = "cpp-gnu-v2"
    ))]
    #[test]
    fn legacy_detection_keeps_itanium_and_msvc_precedence() {
        assert_eq!(Scheme::detect("_ZN3Foo3barEv"), Scheme::CppItanium);
        assert_eq!(Scheme::detect("?x@@3HA"), Scheme::CppMsvc);
        let demangler = Demangler::new();
        assert_eq!(
            demangler.demangle("_ZN3Foo3barEv").unwrap().to_string(),
            "Foo::bar()"
        );
        assert_eq!(demangler.demangle("?x@@3HA").unwrap().to_string(), "int x");
    }

    #[cfg(feature = "cpp-gnu-v2")]
    #[test]
    fn demangles_legacy_gnu_v2() {
        let demangler = Demangler::new();
        for (symbol, scheme, expected) in [
            ("foo__Fiv", Scheme::CppGnuV2, "foo(int, void)"),
            ("foo__FPCc", Scheme::CppGnuV2, "foo(char const *)"),
            ("_$_3foo", Scheme::CppGnuV2, "foo::~foo(void)"),
            ("_3foo$bar", Scheme::CppGnuV2, "foo::bar"),
        ] {
            let demangled = demangler
                .demangle(symbol)
                .unwrap_or_else(|error| panic!("{symbol}: {error}"));
            assert_eq!(demangled.scheme(), scheme, "{symbol}");
            assert_eq!(demangled.language(), Language::Cpp, "{symbol}");
            assert_eq!(demangled.to_string(), expected, "{symbol}");
        }
    }

    #[cfg(feature = "cpp-gnu-v2")]
    #[test]
    fn demangles_legacy_cfront_dialects() {
        let demangler = Demangler::new();
        for (symbol, scheme, expected) in [
            ("__ct__3fooFv", Scheme::CppArm, "foo::foo(void)"),
            ("__dt__3foo", Scheme::CppArm, "foo::~foo"),
            ("__vtbl__3foo__3bar", Scheme::CppArm, "bar::foo virtual table"),
            ("__sti__foo", Scheme::CppArm, "global constructors keyed to foo"),
            ("foo__pt__2_i", Scheme::CppArm, "foo<int>"),
            ("foo__Fi_v", Scheme::CppEdg, "foo(int)"),
            ("foo__tm__2_i", Scheme::CppEdg, "foo<int>"),
        ] {
            let demangled = demangler
                .demangle(symbol)
                .unwrap_or_else(|error| panic!("{symbol}: {error}"));
            assert_eq!(demangled.scheme(), scheme, "{symbol}");
            assert_eq!(demangled.language(), Language::Cpp, "{symbol}");
            assert_eq!(demangled.to_string(), expected, "{symbol}");
        }
    }

    #[test]
    fn decodes_macho_c_leading_underscore() {
        let demangler = Demangler::new();
        for (symbol, expected) in [("_main", "main"), ("_foo_bar", "foo_bar"), ("_x1", "x1")] {
            let demangled = demangler.demangle_macho(symbol).unwrap();
            assert_eq!(demangled.scheme(), Scheme::C, "{symbol}");
            assert_eq!(demangled.to_string(), expected, "{symbol}");
            assert_eq!(
                demangler
                    .demangle_macho_bytes(symbol.as_bytes())
                    .unwrap()
                    .to_string(),
                expected,
                "{symbol}"
            );
            // The global path never strips the platform prefix.
            let plain = demangler.demangle(symbol).unwrap();
            assert_ne!(plain.to_string(), expected, "{symbol}");
        }
    }

    #[test]
    fn macho_keeps_language_prefixes_and_ambiguous_names() {
        let demangler = Demangler::new();
        // A doubled underscore is a platform prefix plus a mangled name, so the
        // language backend keeps the name.
        #[cfg(feature = "cpp-itanium")]
        for symbol in ["__Z3foov", "_Z3foov"] {
            let demangled = demangler.demangle_macho(symbol).unwrap();
            assert_eq!(demangled.scheme(), Scheme::CppItanium, "{symbol}");
            assert_eq!(demangled.to_string(), "foo()", "{symbol}");
        }
        #[cfg(feature = "rust")]
        {
            let demangled = demangler.demangle_macho("_RNvCs1234_4test4main").unwrap();
            assert_eq!(demangled.scheme(), Scheme::RustV0);
        }
        #[cfg(feature = "swift")]
        {
            let demangled = demangler.demangle_macho("_$s4main3fooyySiF").unwrap();
            assert_eq!(demangled.scheme(), Scheme::SwiftStable);
        }
        #[cfg(feature = "objc")]
        {
            let demangled = demangler.demangle_macho("_OBJC_CLASS_$_NSString").unwrap();
            assert_eq!(demangled.scheme(), Scheme::ObjC);
        }
        // A remainder that is not a plain C identifier is left alone: the
        // text is unchanged and the name is never reclassified as C.
        for symbol in ["__foo", "_123", "_x.y", "_"] {
            let demangled = demangler.demangle_macho(symbol).unwrap();
            assert_ne!(demangled.scheme(), Scheme::C, "{symbol}");
            assert_eq!(demangled.to_string(), symbol, "{symbol}");
        }
        // Non-ASCII identifier bytes are not a plain C identifier either, so
        // the escaped pass-through text keeps the prefix.
        let bytes = demangler.demangle_macho_bytes(b"_f\xffoo").unwrap();
        assert_ne!(bytes.scheme(), Scheme::C);
        assert_eq!(bytes.to_string(), "_f\\xffoo");
    }

    #[test]
    fn detects_unknown() {
        assert_eq!(Scheme::detect("12345"), Scheme::Unknown);
        assert_eq!(Scheme::detect(""), Scheme::Unknown);
    }

    #[test]
    fn byte_api_accepts_utf8_and_escapes_invalid_bytes() {
        let demangler = Demangler::new();
        #[cfg(feature = "cpp-itanium")]
        {
            let valid = demangler.demangle_bytes(b"_ZN3Foo3barEv").unwrap();
            assert_eq!(valid.to_string(), "Foo::bar()");
            assert_eq!(Scheme::detect_bytes(b"_ZN3Foo3barEv"), Scheme::CppItanium);
        }

        #[cfg(feature = "cpp-itanium")]
        {
            let invalid = demangler.demangle_bytes(b"_ZN4F\xffooE").unwrap();
            assert_eq!(invalid.scheme(), Scheme::CppItanium);
            assert_eq!(invalid.to_string(), "F\\xffoo");
            assert_eq!(Scheme::detect_bytes(b"_ZN4F\xffooE"), Scheme::CppItanium);
        }

        let plain = demangler.demangle_bytes(b"plain\xff").unwrap();
        assert_eq!(plain.scheme(), Scheme::Unknown);
        assert_eq!(plain.to_string(), "plain\\xff");
        assert_eq!(Scheme::detect_bytes(b"plain\xff"), Scheme::Unknown);
    }

    #[test]
    fn plain_names_pass_through() {
        let demangler = Demangler::new();
        let demangled = demangler.demangle("main").unwrap();
        assert_eq!(demangled.scheme(), Scheme::C);
        assert_eq!(demangled.to_string(), "main");
    }

    #[cfg(feature = "cpp-itanium")]
    #[test]
    fn demangles_cpp_itanium() {
        let demangler = Demangler::new();
        let demangled = demangler.demangle("_ZN3Foo3barEv").unwrap();
        assert_eq!(demangled.scheme(), Scheme::CppItanium);
        assert_eq!(demangled.to_string(), "Foo::bar()");
    }

    #[cfg(feature = "cpp-itanium")]
    #[test]
    fn cpp_input_is_bounded() {
        let demangler = Demangler::new();
        let mut symbol = String::from("_Z");
        symbol.push_str(&"1".repeat(crate::cpp::MAX_INPUT_LEN));
        assert!(demangler.demangle(&symbol).is_err());
    }

    #[cfg(feature = "cpp-itanium")]
    #[test]
    fn cpp_keeps_full_signatures() {
        let demangler = Demangler::new();
        for (symbol, expected) in [
            ("_Z1fPFivE", "f(int (*)())"),
            ("_Z1fIiEvT_", "void f<int>(int)"),
            ("_Z1fA5_i", "f(int [5])"),
            ("_Z1fM1AFivE", "f(int (A::*)())"),
            ("_ZSt4cout", "std::cout"),
        ] {
            assert_eq!(
                demangler.demangle(symbol).unwrap().to_string(),
                expected,
                "{symbol}"
            );
        }
    }

    #[cfg(all(feature = "rust", feature = "cpp-itanium"))]
    #[test]
    fn rust_legacy_hash_is_not_parsed_as_cpp() {
        let demangler = Demangler::new();
        let demangled = demangler
            .demangle("_ZN4test4main17h1234567890abcdefE")
            .unwrap();
        assert_eq!(demangled.language(), Language::Rust);
        assert_eq!(demangled.to_string(), "test::main::h1234567890abcdef");
    }

    #[cfg(feature = "rust")]
    #[test]
    fn demangles_rust() {
        let demangler = Demangler::new();
        let demangled = demangler
            .demangle("_RNvCs1234_4test4main")
            .or_else(|_| demangler.demangle("_ZN4test4main17h1234567890abcdefE"))
            .unwrap();
        assert_eq!(demangled.language(), Language::Rust);
    }

    #[cfg(feature = "go")]
    #[test]
    fn demangles_go() {
        let demangler = Demangler::new();
        for (symbol, expected) in [
            ("fmt.Println", "fmt.Println"),
            (
                "runtime.(*mLockProfile).captureStack",
                "runtime.(*mLockProfile).captureStack",
            ),
            (
                "example.com/demangle/corpus/pkg%2ev1.Make",
                "example.com/demangle/corpus/pkg.v1.Make",
            ),
            ("runtime.memmove.abi0", "runtime.memmove"),
            ("type:.eq.main.Pair[string,int]", "type:.eq.main.Pair[string,int]"),
            ("go:itab.*os.File,io.Writer", "go:itab.*os.File,io.Writer"),
            ("go.shape.int", "go.shape.int"),
            ("cmp.Compare[go.shape.int]", "cmp.Compare[go.shape.int]"),
        ] {
            let demangled = demangler.demangle(symbol).unwrap();
            assert_eq!(demangled.scheme(), Scheme::Go, "{symbol}");
            assert_eq!(demangled.language(), Language::Go, "{symbol}");
            assert_eq!(demangled.to_string(), expected, "{symbol}");
        }
    }

    #[cfg(feature = "go")]
    #[test]
    fn rejects_malformed_go() {
        let demangler = Demangler::new();
        assert!(matches!(
            demangler.demangle("fmt.Println["),
            Err(Error::InvalidSymbol(Scheme::Go))
        ));
        // Detection is a prefilter: an ambiguous name stays out of Go and is
        // passed through as plain text.
        let ambiguous = demangler.demangle("fmt.Println(").unwrap();
        assert_eq!(ambiguous.scheme(), Scheme::Unknown);
    }

    #[cfg(feature = "swift")]
    #[test]
    fn demangles_swift() {
        let demangler = Demangler::new();
        let demangled = demangler.demangle("$s4main3fooyySiF").unwrap();
        assert_eq!(demangled.scheme(), Scheme::SwiftStable);
        assert_eq!(demangled.language(), Language::Swift);
        assert_eq!(demangled.to_string(), "main.foo(Swift.Int) -> ()");
    }

    #[cfg(feature = "swift")]
    #[test]
    fn rejects_swift_symbolic_references() {
        let demangler = Demangler::new();
        assert!(matches!(
            demangler.demangle("$s4main3foo\u{1}0123"),
            Err(Error::InvalidSymbol(Scheme::SwiftStable))
        ));
    }

    #[cfg(feature = "swift")]
    #[test]
    fn detects_legacy_swift() {
        for symbol in ["_TtBf32_", "_TF3foog3barSi", "_TWvdvC3foo3bar3basSi"] {
            assert_eq!(Scheme::detect(symbol), Scheme::SwiftLegacy, "{symbol}");
        }
    }

    #[cfg(feature = "swift")]
    #[test]
    fn demangles_legacy_swift() {
        let demangler = Demangler::new();
        for (symbol, expected) in [
            ("_TtBf32_", "Builtin.FPIEEE32"),
            ("_TtGSaSS_", "[Swift.String]"),
            ("_TF3foog3barSi", "foo.bar.getter : Swift.Int"),
            ("_TwalC3foo3bar", "allocateBuffer value witness for foo.bar"),
            (
                "_TWPC3foo3barS_8barrables",
                "protocol witness table for foo.bar : foo.barrable in Swift",
            ),
        ] {
            let demangled = demangler
                .demangle(symbol)
                .unwrap_or_else(|error| panic!("{symbol}: {error}"));
            assert_eq!(demangled.scheme(), Scheme::SwiftLegacy, "{symbol}");
            assert_eq!(demangled.language(), Language::Swift, "{symbol}");
            assert_eq!(demangled.to_string(), expected, "{symbol}");
        }
    }

    #[cfg(feature = "swift")]
    #[test]
    fn detects_mach_o_underscore_swift() {
        // Mach-O prefixes Swift symbols with an underscore.
        assert_eq!(Scheme::detect("_$s4main3fooSi_yF"), Scheme::SwiftStable);
        assert_eq!(Scheme::detect("_$S3foo3barC3bas3zimyAaEC_tFTo"), Scheme::SwiftStable);
    }

    /// Encode a positive base-26 D back-reference position.
    #[cfg(feature = "d")]
    fn d_backref(value: usize) -> String {
        let mut digits = std::vec![b'a' + u8::try_from(value % 26).unwrap()];
        let mut rest = value / 26;
        while rest > 0 {
            digits.push(b'A' + u8::try_from(rest % 26).unwrap());
            rest /= 26;
        }
        digits.reverse();
        String::from_utf8(digits).unwrap()
    }

    #[cfg(feature = "d")]
    #[test]
    fn detects_d() {
        for symbol in [
            "_Dmain",
            "_D3foo3barFZv",
            "_D4test3fooFiZi",
            "__D3foo3barFZv",
        ] {
            assert_eq!(Scheme::detect(symbol), Scheme::D, "{symbol}");
            assert_eq!(
                Scheme::detect_bytes(symbol.as_bytes()),
                Scheme::D,
                "{symbol}"
            );
            assert_eq!(
                Scheme::detect(symbol).language(),
                Language::D,
                "{symbol}"
            );
        }
    }

    #[cfg(feature = "d")]
    #[test]
    fn demangles_d() {
        let demangler = Demangler::new();
        for (symbol, expected) in [
            ("_Dmain", "D main"),
            ("_D3foo3barFZv", "foo.bar()"),
            ("_D4test3fooFiZi", "test.foo(int)"),
            ("__D3foo3barFZv", "foo.bar()"),
            ("_D3app__T3BoxTiZQh3getMFZi", "app.Box!(int).Box.get()"),
        ] {
            let demangled = demangler
                .demangle(symbol)
                .unwrap_or_else(|error| panic!("{symbol}: {error}"));
            assert_eq!(demangled.scheme(), Scheme::D, "{symbol}");
            assert_eq!(demangled.language(), Language::D, "{symbol}");
            assert_eq!(demangled.to_string(), expected, "{symbol}");
        }
    }

    #[cfg(feature = "d")]
    #[test]
    fn d_detection_requires_a_complete_symbol() {
        // A bare prefix, a truncated name and a name that merely starts with
        // `_D` are not claimed, so plain C names stay out of the backend.
        for symbol in ["_D", "_Dfoo", "_Debug", "_D3foo3bar", "_D3foo3barF"] {
            assert_ne!(Scheme::detect(symbol), Scheme::D, "{symbol}");
        }
    }

    #[cfg(feature = "d")]
    #[test]
    fn d_does_not_steal_other_schemes() {
        #[cfg(feature = "cpp-itanium")]
        assert_eq!(Scheme::detect("_Z3foov"), Scheme::CppItanium);
        #[cfg(feature = "rust")]
        assert_eq!(Scheme::detect("_RNvCs1234_4test4main"), Scheme::RustV0);
        #[cfg(feature = "rust")]
        assert_eq!(
            Scheme::detect("_ZN4test4main17h1234567890abcdefE"),
            Scheme::RustLegacy
        );
        #[cfg(feature = "objc")]
        assert_eq!(Scheme::detect("_OBJC_CLASS_$_NSString"), Scheme::ObjC);
        assert_eq!(Scheme::detect("main"), Scheme::C);
    }

    #[cfg(feature = "d")]
    #[test]
    fn d_strips_linker_decorations() {
        // Detection runs on the decoration-stripped name, so dispatch has to
        // hand the backend the same stripped name. `__D...@16` is the x86
        // stdcall spelling: COFF strips the decoration underscore, or the GNU
        // version stripper removes the `@` suffix, and both leave `_D...`.
        let demangler = Demangler::new();
        for symbol in [
            "_D3foo3barFZv.llvm.9D1C9369",
            "_D3foo3barFZv@GLIBC_2.2.5",
            "_D3foo3barFZv@@GLIBC_2.2.5",
            "__D3foo3barFZv@16",
        ] {
            assert_eq!(Scheme::detect(symbol), Scheme::D, "{symbol}");
            let demangled = demangler
                .demangle(symbol)
                .unwrap_or_else(|error| panic!("{symbol}: {error}"));
            assert_eq!(demangled.scheme(), Scheme::D, "{symbol}");
            assert_eq!(demangled.to_string(), "foo.bar()", "{symbol}");
        }
    }

    #[cfg(feature = "d")]
    #[test]
    fn d_output_cap_is_rejected_before_display() {
        // Identifier back references expand a 45 KiB symbol past the printer's
        // output ceiling. The dispatch arm validates the render, so this is a
        // rejection rather than a `Display` failure that `to_string()` would
        // turn into a panic.
        let name_len = 20_000;
        let mut symbol = String::from("_D20000");
        symbol.push_str(&"a".repeat(name_len));
        for _ in 0..5_000 {
            let value = symbol.len() - 2;
            symbol.push('Q');
            symbol.push_str(&d_backref(value));
        }
        symbol.push('i');

        assert!(matches!(
            Demangler::new().demangle(&symbol),
            Err(Error::InvalidSymbol(Scheme::D))
        ));
    }

    #[cfg(not(feature = "d"))]
    #[test]
    fn d_is_not_detected_without_its_feature() {
        assert_ne!(Scheme::detect("_D3foo3barFZv"), Scheme::D);
        assert_ne!(Scheme::detect("_Dmain"), Scheme::D);
    }

    #[cfg(feature = "jni")]
    #[test]
    fn jni_strips_linker_decorations() {
        let demangler = Demangler::new();
        for symbol in [
            "Java_com_example_Foo_bar__I.llvm.9D1C9369",
            "Java_com_example_Foo_bar__I@@JNI_1.0",
        ] {
            assert_eq!(Scheme::detect(symbol), Scheme::Jni, "{symbol}");
            let demangled = demangler
                .demangle(symbol)
                .unwrap_or_else(|error| panic!("{symbol}: {error}"));
            assert_eq!(demangled.scheme(), Scheme::Jni, "{symbol}");
            assert_eq!(demangled.language(), Language::Java, "{symbol}");
            assert_eq!(
                demangled.to_string(),
                "com.example.Foo.bar(int)",
                "{symbol}"
            );
        }
    }

    #[cfg(all(feature = "cpp-itanium", feature = "cpp-msvc"))]
    #[test]
    fn forced_cpp_scheme_rejects_another_dialect() {
        let demangler = Demangler::new();
        for (scheme, symbol) in [
            (Scheme::CppMsvc, "_Z3foov"),
            (Scheme::CppItanium, "?x@@3HA"),
        ] {
            assert_eq!(
                demangler.demangle_with_scheme(scheme, symbol).unwrap_err(),
                Error::InvalidSymbol(scheme),
                "{symbol}"
            );
        }
        // The matching scheme still parses the same symbol.
        assert_eq!(
            demangler
                .demangle_with_scheme(Scheme::CppItanium, "_Z3foov")
                .unwrap()
                .scheme(),
            Scheme::CppItanium
        );
        assert_eq!(
            demangler
                .demangle_with_scheme(Scheme::CppMsvc, "?x@@3HA")
                .unwrap()
                .scheme(),
            Scheme::CppMsvc
        );
    }

    #[cfg(feature = "cpp-gnu-v2")]
    #[test]
    fn forced_legacy_scheme_rejects_another_dialect() {
        let demangler = Demangler::new();
        for (scheme, symbol) in [
            (Scheme::CppGnuV2, "__ct__3fooFv"),
            (Scheme::CppArm, "foo__Fiv"),
        ] {
            assert_eq!(
                demangler.demangle_with_scheme(scheme, symbol).unwrap_err(),
                Error::InvalidSymbol(scheme),
                "{symbol}"
            );
        }
        assert_eq!(
            demangler
                .demangle_with_scheme(Scheme::CppArm, "__ct__3fooFv")
                .unwrap()
                .scheme(),
            Scheme::CppArm
        );
        assert_eq!(
            demangler
                .demangle_with_scheme(Scheme::CppGnuV2, "foo__Fiv")
                .unwrap()
                .scheme(),
            Scheme::CppGnuV2
        );
    }

    #[cfg(feature = "rust")]
    #[test]
    fn forced_rust_scheme_rejects_another_encoding() {
        let demangler = Demangler::new();
        for (scheme, symbol) in [
            (Scheme::RustLegacy, "_RNvCs1234_4test4main"),
            (Scheme::RustV0, "_ZN4test4main17h1234567890abcdefE"),
        ] {
            assert_eq!(
                demangler.demangle_with_scheme(scheme, symbol).unwrap_err(),
                Error::InvalidSymbol(scheme),
                "{symbol}"
            );
        }
        assert_eq!(
            demangler
                .demangle_with_scheme(Scheme::RustV0, "_RNvCs1234_4test4main")
                .unwrap()
                .scheme(),
            Scheme::RustV0
        );
        assert_eq!(
            demangler
                .demangle_with_scheme(Scheme::RustLegacy, "_ZN3fooE")
                .unwrap()
                .scheme(),
            Scheme::RustLegacy
        );
    }

    #[cfg(feature = "swift")]
    #[test]
    fn forced_swift_scheme_rejects_another_encoding() {
        let demangler = Demangler::new();
        for (scheme, symbol) in [
            (Scheme::SwiftLegacy, "$s4main3fooyySiF"),
            (Scheme::SwiftStable, "_TtSi"),
        ] {
            assert_eq!(
                demangler.demangle_with_scheme(scheme, symbol).unwrap_err(),
                Error::InvalidSymbol(scheme),
                "{symbol}"
            );
        }
        assert_eq!(
            demangler
                .demangle_with_scheme(Scheme::SwiftStable, "$s4main3fooyySiF")
                .unwrap()
                .scheme(),
            Scheme::SwiftStable
        );
        assert_eq!(
            demangler
                .demangle_with_scheme(Scheme::SwiftLegacy, "_TtSi")
                .unwrap()
                .scheme(),
            Scheme::SwiftLegacy
        );
    }

    #[cfg(all(feature = "rust", feature = "cpp-itanium"))]
    #[test]
    fn detected_cpp_falls_back_to_rust_with_its_encoding() {
        // A zero-length path element is invalid Itanium but valid in the
        // legacy rustc grammar, so the detected path retags the symbol.
        let demangler = Demangler::new();
        let demangled = demangler.demangle("_ZN0E").unwrap();
        assert_eq!(demangled.scheme(), Scheme::RustLegacy);
        assert_eq!(demangled.language(), Language::Rust);
        assert_eq!(
            demangler
                .demangle_bytes(b"_ZN1a0E")
                .unwrap()
                .scheme(),
            Scheme::RustLegacy
        );
        // A forced scheme and a selection are exact: no fallback.
        assert_eq!(
            demangler
                .demangle_with_scheme(Scheme::CppItanium, "_ZN0E")
                .unwrap_err(),
            Error::InvalidSymbol(Scheme::CppItanium)
        );
        assert_eq!(
            demangler
                .selection()
                .allow(Scheme::CppItanium)
                .demangle("_ZN0E")
                .unwrap_err(),
            Error::InvalidSymbol(Scheme::CppItanium)
        );
    }

    #[cfg(feature = "cpp-itanium")]
    #[test]
    fn language_and_scheme_convenience_methods() {
        let demangler = Demangler::new();
        assert_eq!(
            demangler
                .demangle_with_language(Language::Cpp, "_ZN3Foo3barEv")
                .unwrap()
                .scheme(),
            Scheme::CppItanium
        );
        assert_eq!(
            demangler
                .demangle_bytes_with_language(Language::Cpp, b"_ZN3Foo3barEv")
                .unwrap()
                .to_string(),
            "Foo::bar()"
        );
        assert_eq!(
            demangler
                .demangle_with_languages(&[Language::Cpp], "_ZN3Foo3barEv")
                .unwrap()
                .scheme(),
            Scheme::CppItanium
        );
        assert_eq!(
            demangler
                .demangle_bytes_with_languages(&[Language::Cpp], b"_ZN3Foo3barEv")
                .unwrap()
                .scheme(),
            Scheme::CppItanium
        );
        #[cfg(feature = "go")]
        {
            assert_eq!(
                demangler
                    .demangle_with_schemes(&[Scheme::Go, Scheme::CppItanium], "_ZN3Foo3barEv")
                    .unwrap()
                    .scheme(),
                Scheme::Go
            );
            assert_eq!(
                demangler
                    .demangle_with_schemes(&[Scheme::CppItanium, Scheme::Go], "_ZN3Foo3barEv")
                    .unwrap()
                    .scheme(),
                Scheme::CppItanium
            );
        }
        assert_eq!(
            demangler
                .demangle_bytes_with_schemes(&[Scheme::CppItanium], b"_ZN3Foo3barEv")
                .unwrap()
                .scheme(),
            Scheme::CppItanium
        );
        // An empty target list is an empty selection: exactly `demangle`.
        assert_eq!(
            demangler.demangle_with_schemes(&[], "main").unwrap().scheme(),
            Scheme::C
        );
    }
}
