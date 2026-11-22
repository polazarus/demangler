use crate::error::Error;
use crate::result::Demangled;
use crate::scheme::Scheme;

/// Demangles symbol names using the appropriate demangling scheme.
///
/// Detects the scheme from the symbol name and delegates to the appropriate
/// backend. Delegation keeps the crate fast and correct today; hot paths can
/// be rewritten in-tree later without changing the public API.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Demangler;

impl Demangler {
    /// Create a new [`Demangler`].
    pub const fn new() -> Self {
        Self
    }

    /// Detect the scheme and demangle a symbol name.
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
        let scheme = Scheme::detect(symbol);
        match scheme {
            Scheme::C | Scheme::Unknown => Ok(Demangled::plain(scheme, symbol)),
            #[cfg(feature = "rust")]
            Scheme::Rust => demangle_rust(scheme, symbol).ok_or(Error::InvalidSymbol(scheme)),
            // `_Z...` also prefixes legacy rustc symbols; fall back to the
            // Rust backend when the C++ backends decline.
            #[cfg(any(
                feature = "cpp-itanium",
                feature = "cpp-msvc",
                feature = "cpp-gnu-v2"
            ))]
            Scheme::Cpp => {
                #[cfg(feature = "rust")]
                let rust_fallback = || demangle_rust(Scheme::Rust, symbol);
                #[cfg(not(feature = "rust"))]
                let rust_fallback = || None;
                demangle_cpp(scheme, symbol)
                    .or_else(rust_fallback)
                    .ok_or(Error::InvalidSymbol(scheme))
            }
            #[cfg(feature = "swift")]
            Scheme::Swift => crate::swift::parse(symbol)
                .map(|parsed| Demangled::swift(scheme, parsed))
                .map_err(|_| Error::InvalidSymbol(scheme)),
            #[cfg(feature = "go")]
            Scheme::Go => crate::go::parse(symbol)
                .map(|parsed| Demangled::go(scheme, parsed))
                .map_err(|_| Error::InvalidSymbol(scheme)),
            #[cfg(feature = "objc")]
            Scheme::ObjC => {
                let symbol = crate::scheme::strip_decorations(symbol);
                crate::objc::parse(symbol)
                    .map(|parsed| Demangled::objc(scheme, parsed))
                    .map_err(|_| Error::InvalidSymbol(scheme))
            }
            #[allow(unreachable_patterns)]
            _ => Err(Error::InvalidSymbol(scheme)),
        }
    }
}

/// In-tree Rust backend (`no_std` + `alloc`).
#[cfg(feature = "rust")]
#[allow(clippy::single_call_fn)]
fn demangle_rust<'a>(scheme: Scheme, symbol: &'a str) -> Option<Demangled<'a>> {
    let symbol = crate::scheme::strip_decorations(symbol);
    crate::rust::parse(symbol)
        .ok()
        .map(|parsed| Demangled::rust(scheme, parsed))
}

/// In-tree C++ backend (Itanium, Microsoft and the legacy dialects).
#[cfg(any(
    feature = "cpp-itanium",
    feature = "cpp-msvc",
    feature = "cpp-gnu-v2"
))]
#[allow(clippy::single_call_fn)]
fn demangle_cpp<'a>(scheme: Scheme, symbol: &'a str) -> Option<Demangled<'a>> {
    let symbol = crate::scheme::strip_thin_lto(symbol);
    // Microsoft names use `@` as a grammar separator, so a version suffix can
    // only be removed from the other dialects.
    let symbol = if symbol.starts_with('?') || symbol.starts_with(".?") {
        symbol
    } else {
        crate::scheme::strip_version_suffix(symbol)
    };
    if symbol.len() > crate::cpp::MAX_INPUT_LEN {
        return None;
    }
    let parsed = crate::cpp::parse(symbol).ok()?;
    // `Demangled::to_string` must not panic later. Reject a tree the printer
    // cannot render instead of returning a value whose Display fails.
    let mut sink = alloc::string::String::new();
    core::fmt::Write::write_fmt(&mut sink, format_args!("{parsed}")).ok()?;
    Some(Demangled::cpp(scheme, parsed))
}

#[cfg(test)]
mod tests {
    use std::string::ToString;

    #[cfg(feature = "cpp-itanium")]
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
}
