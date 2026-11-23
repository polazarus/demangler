#[cfg(any(
    feature = "encoding",
    feature = "d",
    feature = "cpp-itanium",
    feature = "cpp-msvc",
    feature = "cpp-gnu-v2"
))]
use alloc::string::String;

#[cfg(feature = "encoding")]
use crate::encoding::{Encoding, EncodingError};
use crate::error::Error;
use crate::result::Demangled;
use crate::scheme::Scheme;

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
    /// backend rejects returns [`Error::InvalidSymbol`]. It is the integration
    /// point for names whose automatic detection is intentionally
    /// conservative: general Ada external names (`pkg__subp`), gfortran
    /// external names (`foo_`), namespaced Free Pascal units and OCaml
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
        #[cfg(feature = "coff")]
        if let Some(decoration) = crate::coff::parse_bytes(symbol) {
            let inner = self.demangle_bytes_with_scheme(scheme, decoration.symbol())?;
            return Ok(inner.with_coff_decoration(decoration));
        }
        self.dispatch_bytes(scheme, symbol)
    }

    /// Demangle a raw symbol byte string with an explicitly selected scheme.
    ///
    /// Deprecated alias for [`Self::demangle_bytes_with_scheme`].
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidSymbol`] when the selected backend is not
    /// compiled in or cannot demangle the input.
    #[deprecated(note = "use `demangle_bytes_with_scheme` instead")]
    pub fn demangle_as<'a>(
        &self,
        scheme: Scheme,
        symbol: &'a [u8],
    ) -> Result<Demangled<'a>, Error> {
        self.demangle_bytes_with_scheme(scheme, symbol)
    }

    /// Demangle a UTF-8 symbol name with an explicitly selected scheme.
    ///
    /// Deprecated alias for [`Self::demangle_with_scheme`].
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidSymbol`] when the selected backend is not
    /// compiled in or cannot demangle the input.
    #[deprecated(note = "use `demangle_with_scheme` instead")]
    pub fn demangle_as_str<'a>(
        &self,
        scheme: Scheme,
        symbol: &'a str,
    ) -> Result<Demangled<'a>, Error> {
        self.demangle_with_scheme(scheme, symbol)
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
    pub fn demangle_to_string_with_encoding(
        &self,
        symbol: &[u8],
        encoding: Encoding,
    ) -> Result<String, EncodingError> {
        self.demangle_bytes(symbol)?.to_string_with_encoding(encoding)
    }

    /// Demangle a raw symbol byte string.
    ///
    /// Object-file symbol names are byte strings, not UTF-8 strings. Every
    /// backend parses bytes directly. Invalid UTF-8 in identifier text is
    /// retained and rendered as `\xNN`; compiler payload text follows its
    /// documented verifier policy. A name that no backend recognizes is
    /// returned as escaped plain text with [`Scheme::Unknown`].
    pub fn demangle_bytes<'a>(&self, symbol: &'a [u8]) -> Result<Demangled<'a>, Error> {
        #[cfg(feature = "coff")]
        if let Some(decoration) = crate::coff::parse_bytes(symbol) {
            let inner = self.demangle_bytes(decoration.symbol())?;
            return Ok(inner.with_coff_decoration(decoration));
        }
        let scheme = Scheme::detect_bytes(symbol);
        self.dispatch_bytes(scheme, symbol)
    }

    /// Dispatch an already selected scheme to its backend.
    fn dispatch_bytes<'a>(&self, scheme: Scheme, symbol: &'a [u8]) -> Result<Demangled<'a>, Error> {
        match scheme {
            Scheme::C | Scheme::Unknown => {
                core::str::from_utf8(symbol).map_or_else(
                    |_| Ok(Demangled::bytes(scheme, symbol)),
                    |text| Ok(Demangled::plain(scheme, text)),
                )
            }
            #[cfg(feature = "rust")]
            Scheme::Rust => {
                let stripped = crate::scheme::strip_decorations_bytes(symbol);
                crate::rust::parse_bytes(stripped)
                    .map(|parsed| Demangled::rust(scheme, parsed))
                    .map_err(|_| Error::InvalidSymbol(scheme))
            }
            #[cfg(any(
                feature = "cpp-itanium",
                feature = "cpp-msvc",
                feature = "cpp-gnu-v2"
            ))]
            Scheme::Cpp => {
                let stripped = crate::scheme::strip_thin_lto_bytes(symbol);
                let stripped = if stripped.starts_with(b"?") || stripped.starts_with(b".?") {
                    stripped
                } else {
                    crate::scheme::strip_version_suffix_bytes(stripped)
                };
                if let Ok(parsed) = crate::cpp::parse_bytes(stripped) {
                    // Keep the validation pass until printer totality is
                    // proven; a retained AST must not fail later in Display.
                    let mut sink = String::new();
                    if core::fmt::Write::write_fmt(&mut sink, format_args!("{parsed}")).is_ok() {
                        return Ok(Demangled::cpp(scheme, parsed));
                    }
                }
                #[cfg(feature = "rust")]
                if let Ok(parsed) = crate::rust::parse_bytes(stripped) {
                    return Ok(Demangled::rust(Scheme::Rust, parsed));
                }
                Err(Error::InvalidSymbol(scheme))
            }
            #[cfg(feature = "swift")]
            Scheme::Swift => crate::swift::parse_bytes(symbol)
                .map(|parsed| Demangled::swift(scheme, parsed))
                .map_err(|_| Error::InvalidSymbol(scheme)),
            #[cfg(feature = "go")]
            Scheme::Go => crate::go::parse_bytes(symbol)
                .map(|parsed| Demangled::go(scheme, parsed))
                .map_err(|_| Error::InvalidSymbol(scheme)),
            #[cfg(feature = "kotlin-native")]
            Scheme::KotlinNative => crate::kotlin_native::parse_bytes(symbol)
                .map(|parsed| Demangled::kotlin_native(scheme, parsed))
                .map_err(|_| Error::InvalidSymbol(scheme)),
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
                Ok(Demangled::d(scheme, parsed))
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
            Scheme::Rust
        );
    }

    #[cfg(feature = "cpp-itanium")]
    #[test]
    fn detects_cpp_itanium() {
        assert_eq!(Scheme::detect("_ZN3Foo3barEv"), Scheme::Cpp);
    }

    #[cfg(feature = "swift")]
    #[test]
    fn detects_swift() {
        assert_eq!(Scheme::detect("$s4main3fooSi_yF"), Scheme::Swift);
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
                crate::Language::ObjectiveC,
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
            assert_eq!(Scheme::detect("_Z3foov"), Scheme::Cpp);
            assert_eq!(Scheme::detect("__Z3foov"), Scheme::Cpp);
        }
        #[cfg(feature = "rust")]
        assert_eq!(Scheme::detect("_RNvCs1234_4test4main"), Scheme::Rust);
        #[cfg(feature = "swift")]
        assert_eq!(Scheme::detect("$s4main3fooSi_yF"), Scheme::Swift);
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
                crate::Language::ObjectiveC,
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
                crate::Language::KotlinNative,
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
                crate::Language::KotlinNative,
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
        assert_eq!(Scheme::detect("_Z3foov"), Scheme::Cpp);
        #[cfg(feature = "rust")]
        assert_eq!(Scheme::detect("_RNvCs1234_4test4main"), Scheme::Rust);
        #[cfg(feature = "swift")]
        assert_eq!(Scheme::detect("$s4main3fooSi_yF"), Scheme::Swift);
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
            assert_eq!(Scheme::detect(symbol), Scheme::Cpp, "{symbol}");
        }
    }

    #[test]
    fn plain_names_with_separators_stay_c() {
        // A `__` alone does not make a name legacy C++: the legacy parser has
        // to consume the whole symbol before the dialect is claimed.
        for symbol in ["foo__bar", "foo_bar", "foo", "main"] {
            assert_eq!(Scheme::detect(symbol), Scheme::C, "{symbol}");
        }
        assert_ne!(Scheme::detect("__libc_start_main"), Scheme::Cpp);
    }

    #[cfg(all(
        feature = "cpp-itanium",
        feature = "cpp-msvc",
        feature = "cpp-gnu-v2"
    ))]
    #[test]
    fn legacy_detection_keeps_itanium_and_msvc_precedence() {
        assert_eq!(Scheme::detect("_ZN3Foo3barEv"), Scheme::Cpp);
        assert_eq!(Scheme::detect("?x@@3HA"), Scheme::Cpp);
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
        for (symbol, expected) in [
            ("foo__Fiv", "foo(int, void)"),
            ("foo__FPCc", "foo(char const *)"),
            ("_$_3foo", "foo::~foo(void)"),
            ("_3foo$bar", "foo::bar"),
        ] {
            let demangled = demangler
                .demangle(symbol)
                .unwrap_or_else(|error| panic!("{symbol}: {error}"));
            assert_eq!(demangled.scheme(), Scheme::Cpp, "{symbol}");
            assert_eq!(demangled.language(), crate::Language::Cxx, "{symbol}");
            assert_eq!(demangled.to_string(), expected, "{symbol}");
        }
    }

    #[cfg(feature = "cpp-gnu-v2")]
    #[test]
    fn demangles_legacy_cfront_dialects() {
        let demangler = Demangler::new();
        for (symbol, expected) in [
            ("__ct__3fooFv", "foo::foo(void)"),
            ("__dt__3foo", "foo::~foo"),
            ("__vtbl__3foo__3bar", "bar::foo virtual table"),
            ("__sti__foo", "global constructors keyed to foo"),
            ("foo__pt__2_i", "foo<int>"),
            ("foo__Fi_v", "foo(int)"),
            ("foo__tm__2_i", "foo<int>"),
        ] {
            let demangled = demangler
                .demangle(symbol)
                .unwrap_or_else(|error| panic!("{symbol}: {error}"));
            assert_eq!(demangled.scheme(), Scheme::Cpp, "{symbol}");
            assert_eq!(demangled.language(), crate::Language::Cxx, "{symbol}");
            assert_eq!(demangled.to_string(), expected, "{symbol}");
        }
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
            assert_eq!(Scheme::detect_bytes(b"_ZN3Foo3barEv"), Scheme::Cpp);
        }

        #[cfg(feature = "cpp-itanium")]
        {
            let invalid = demangler.demangle_bytes(b"_ZN4F\xffooE").unwrap();
            assert_eq!(invalid.scheme(), Scheme::Cpp);
            assert_eq!(invalid.to_string(), "F\\xffoo");
            assert_eq!(Scheme::detect_bytes(b"_ZN4F\xffooE"), Scheme::Cpp);
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
        assert_eq!(demangled.scheme(), Scheme::Cpp);
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
        assert_eq!(demangled.language(), crate::Language::Rust);
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
        assert_eq!(demangled.language(), crate::Language::Rust);
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
            assert_eq!(demangled.language(), crate::Language::Go, "{symbol}");
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
        assert_eq!(demangled.scheme(), Scheme::Swift);
        assert_eq!(demangled.language(), crate::Language::Swift);
        assert_eq!(demangled.to_string(), "main.foo(Swift.Int) -> ()");
    }

    #[cfg(feature = "swift")]
    #[test]
    fn rejects_swift_symbolic_references() {
        let demangler = Demangler::new();
        assert!(matches!(
            demangler.demangle("$s4main3foo\u{1}0123"),
            Err(Error::InvalidSymbol(Scheme::Swift))
        ));
    }

    #[cfg(feature = "swift")]
    #[test]
    fn detects_legacy_swift() {
        for symbol in ["_TtBf32_", "_TF3foog3barSi", "_TWvdvC3foo3bar3basSi"] {
            assert_eq!(Scheme::detect(symbol), Scheme::Swift, "{symbol}");
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
            assert_eq!(demangled.scheme(), Scheme::Swift, "{symbol}");
            assert_eq!(demangled.language(), crate::Language::Swift, "{symbol}");
            assert_eq!(demangled.to_string(), expected, "{symbol}");
        }
    }

    #[cfg(feature = "swift")]
    #[test]
    fn detects_mach_o_underscore_swift() {
        // Mach-O prefixes Swift symbols with an underscore.
        assert_eq!(Scheme::detect("_$s4main3fooSi_yF"), Scheme::Swift);
        assert_eq!(Scheme::detect("_$S3foo3barC3bas3zimyAaEC_tFTo"), Scheme::Swift);
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
                crate::Language::D,
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
            assert_eq!(demangled.language(), crate::Language::D, "{symbol}");
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
        assert_eq!(Scheme::detect("_Z3foov"), Scheme::Cpp);
        #[cfg(feature = "rust")]
        assert_eq!(Scheme::detect("_RNvCs1234_4test4main"), Scheme::Rust);
        #[cfg(feature = "rust")]
        assert_eq!(
            Scheme::detect("_ZN4test4main17h1234567890abcdefE"),
            Scheme::Rust
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
            assert_eq!(demangled.language(), crate::Language::Java, "{symbol}");
            assert_eq!(
                demangled.to_string(),
                "com.example.Foo.bar(int)",
                "{symbol}"
            );
        }
    }
}
