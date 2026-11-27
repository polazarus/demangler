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
        let demangled_symbol = demangler
            .demangle(symbol)
            .unwrap_or_else(|error| panic!("{symbol}: {error}"));
        assert_eq!(demangled_symbol.scheme(), Scheme::ObjC, "{symbol}");
        assert_eq!(
            demangled_symbol.language(),
            Language::ObjectiveC,
            "{symbol}"
        );
        assert_eq!(demangled_symbol.to_string(), expected, "{symbol}");
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
        let demangled_symbol = demangler
            .demangle(symbol)
            .unwrap_or_else(|error| panic!("{symbol}: {error}"));
        assert_eq!(demangled_symbol.scheme(), Scheme::KotlinNative, "{symbol}");
        assert_eq!(
            demangled_symbol.language(),
            Language::KotlinNative,
            "{symbol}"
        );
        assert_eq!(demangled_symbol.to_string(), expected, "{symbol}");
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
    assert_ne!(
        Scheme::detect("kfun:com.example#main(){}"),
        Scheme::KotlinNative
    );
    assert_ne!(
        Scheme::detect("kclass:com.example.Box"),
        Scheme::KotlinNative
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

#[cfg(all(feature = "cpp-itanium", feature = "cpp-msvc", feature = "cpp-gnu-v2"))]
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
        let demangled_symbol = demangler
            .demangle(symbol)
            .unwrap_or_else(|error| panic!("{symbol}: {error}"));
        assert_eq!(demangled_symbol.scheme(), scheme, "{symbol}");
        assert_eq!(demangled_symbol.language(), Language::Cpp, "{symbol}");
        assert_eq!(demangled_symbol.to_string(), expected, "{symbol}");
    }
}

#[cfg(feature = "cpp-gnu-v2")]
#[test]
fn demangles_legacy_cfront_dialects() {
    let demangler = Demangler::new();
    for (symbol, scheme, expected) in [
        ("__ct__3fooFv", Scheme::CppArm, "foo::foo(void)"),
        ("__dt__3foo", Scheme::CppArm, "foo::~foo"),
        (
            "__vtbl__3foo__3bar",
            Scheme::CppArm,
            "bar::foo virtual table",
        ),
        (
            "__sti__foo",
            Scheme::CppArm,
            "global constructors keyed to foo",
        ),
        ("foo__pt__2_i", Scheme::CppArm, "foo<int>"),
        ("foo__Fi_v", Scheme::CppEdg, "foo(int)"),
        ("foo__tm__2_i", Scheme::CppEdg, "foo<int>"),
    ] {
        let demangled_symbol = demangler
            .demangle(symbol)
            .unwrap_or_else(|error| panic!("{symbol}: {error}"));
        assert_eq!(demangled_symbol.scheme(), scheme, "{symbol}");
        assert_eq!(demangled_symbol.language(), Language::Cpp, "{symbol}");
        assert_eq!(demangled_symbol.to_string(), expected, "{symbol}");
    }
}

#[test]
fn decodes_macho_c_leading_underscore() {
    let demangler = Demangler::new();
    for (symbol, expected) in [("_main", "main"), ("_foo_bar", "foo_bar"), ("_x1", "x1")] {
        let demangled_symbol = demangler.demangle_macho(symbol).unwrap();
        assert_eq!(demangled_symbol.scheme(), Scheme::C, "{symbol}");
        assert_eq!(demangled_symbol.to_string(), expected, "{symbol}");
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
        let demangled_symbol = demangler.demangle_macho(symbol).unwrap();
        assert_eq!(demangled_symbol.scheme(), Scheme::CppItanium, "{symbol}");
        assert_eq!(demangled_symbol.to_string(), "foo()", "{symbol}");
    }
    #[cfg(feature = "rust")]
    {
        let demangled_symbol = demangler.demangle_macho("_RNvCs1234_4test4main").unwrap();
        assert_eq!(demangled_symbol.scheme(), Scheme::RustV0);
    }
    #[cfg(feature = "swift")]
    {
        let demangled_symbol = demangler.demangle_macho("_$s4main3fooyySiF").unwrap();
        assert_eq!(demangled_symbol.scheme(), Scheme::SwiftStable);
    }
    #[cfg(feature = "objc")]
    {
        let demangled_symbol = demangler.demangle_macho("_OBJC_CLASS_$_NSString").unwrap();
        assert_eq!(demangled_symbol.scheme(), Scheme::ObjC);
    }
    // A remainder that is not a plain C identifier is left alone: the
    // text is unchanged and the name is never reclassified as C.
    for symbol in ["__foo", "_123", "_x.y", "_"] {
        let demangled_symbol = demangler.demangle_macho(symbol).unwrap();
        assert_ne!(demangled_symbol.scheme(), Scheme::C, "{symbol}");
        assert_eq!(demangled_symbol.to_string(), symbol, "{symbol}");
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
    let demangled_symbol = demangler.demangle("main").unwrap();
    assert_eq!(demangled_symbol.scheme(), Scheme::C);
    assert_eq!(demangled_symbol.to_string(), "main");
}

#[cfg(feature = "cpp-itanium")]
#[test]
fn demangles_cpp_itanium() {
    let demangler = Demangler::new();
    let demangled_symbol = demangler.demangle("_ZN3Foo3barEv").unwrap();
    assert_eq!(demangled_symbol.scheme(), Scheme::CppItanium);
    assert_eq!(demangled_symbol.to_string(), "Foo::bar()");
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
    let demangled_symbol = demangler
        .demangle("_ZN4test4main17h1234567890abcdefE")
        .unwrap();
    assert_eq!(demangled_symbol.language(), Language::Rust);
    assert_eq!(
        demangled_symbol.to_string(),
        "test::main::h1234567890abcdef"
    );
}

#[cfg(feature = "rust")]
#[test]
fn rust_dispatch_honors_the_budget() {
    // The Rust paths all read the configured budget: the detected scheme,
    // the exact scheme and the C++ to Rust fallback.
    let tiny = Demangler::with_budget(Budget::new().with_input_bytes(4));
    for symbol in [
        "_RNvCs1234_4test4main",
        "_ZN4test4main17h1234567890abcdefE",
        // Valid legacy Rust and invalid Itanium: the detected path falls
        // back from C++ to Rust.
        "_ZN1a0E",
    ] {
        assert!(
            Demangler::new().demangle(symbol).is_ok(),
            "{symbol}: the default budget accepts"
        );
        assert!(
            matches!(tiny.demangle(symbol), Err(Error::Budget { .. })),
            "{symbol}: the tightened budget rejects"
        );
    }
}

#[cfg(feature = "rust")]
#[test]
fn rust_budget_is_clamped_per_scheme() {
    let demangler = Demangler::with_budget(Budget::new().with_input_bytes(usize::MAX));
    assert_eq!(
        demangler.effective_budget(Scheme::RustV0).input_bytes,
        Some(crate::budget::Limit::Bounded(1024 * 1024))
    );
    assert_eq!(
        demangler.effective_budget(Scheme::RustLegacy).input_bytes,
        Some(crate::budget::Limit::Bounded(16 * 1024 * 1024))
    );
}

#[cfg(feature = "rust")]
#[test]
fn demangles_rust() {
    let demangler = Demangler::new();
    let demangled_symbol = demangler
        .demangle("_RNvCs1234_4test4main")
        .or_else(|_| demangler.demangle("_ZN4test4main17h1234567890abcdefE"))
        .unwrap();
    assert_eq!(demangled_symbol.language(), Language::Rust);
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
        (
            "type:.eq.main.Pair[string,int]",
            "type:.eq.main.Pair[string,int]",
        ),
        ("go:itab.*os.File,io.Writer", "go:itab.*os.File,io.Writer"),
        ("go.shape.int", "go.shape.int"),
        ("cmp.Compare[go.shape.int]", "cmp.Compare[go.shape.int]"),
    ] {
        let demangled_symbol = demangler.demangle(symbol).unwrap();
        assert_eq!(demangled_symbol.scheme(), Scheme::Go, "{symbol}");
        assert_eq!(demangled_symbol.language(), Language::Go, "{symbol}");
        assert_eq!(demangled_symbol.to_string(), expected, "{symbol}");
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
    let demangled_symbol = demangler.demangle("$s4main3fooyySiF").unwrap();
    assert_eq!(demangled_symbol.scheme(), Scheme::SwiftStable);
    assert_eq!(demangled_symbol.language(), Language::Swift);
    assert_eq!(demangled_symbol.to_string(), "main.foo(Swift.Int) -> ()");
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
        let demangled_symbol = demangler
            .demangle(symbol)
            .unwrap_or_else(|error| panic!("{symbol}: {error}"));
        assert_eq!(demangled_symbol.scheme(), Scheme::SwiftLegacy, "{symbol}");
        assert_eq!(demangled_symbol.language(), Language::Swift, "{symbol}");
        assert_eq!(demangled_symbol.to_string(), expected, "{symbol}");
    }
}

#[cfg(feature = "cpp-itanium")]
#[test]
fn cpp_reports_the_budget_dimension() {
    for (budget, kind) in [
        (Budget::new().with_input_bytes(0), crate::BudgetKind::Input),
        (
            Budget::new().with_parse_depth(0),
            crate::BudgetKind::ParseDepth,
        ),
        (Budget::new().with_work(0), crate::BudgetKind::Work),
        (Budget::new().with_nodes(0), crate::BudgetKind::Nodes),
    ] {
        let error = Demangler::with_budget(budget)
            .demangle_with_scheme(Scheme::CppItanium, "_Z3foov")
            .unwrap_err();
        assert_eq!(
            error,
            Error::Budget {
                scheme: Scheme::CppItanium,
                kind
            }
        );
    }
}

#[cfg(feature = "swift")]
#[test]
fn swift_dispatch_honors_the_budget() {
    // Both grammars and every dispatch path read the configured budget: the
    // detected scheme, a forced scheme, the retained legacy tree that
    // detection parsed with the defaults, and a selection.
    let tiny = Demangler::with_budget(Budget::new().with_input_bytes(4));
    for symbol in ["$s4main3fooyySiF", "_TtC5sugar7MyClass"] {
        assert!(
            Demangler::new().demangle(symbol).is_ok(),
            "{symbol}: the default budget accepts"
        );
        assert!(
            matches!(tiny.demangle(symbol), Err(Error::InvalidSymbol(_))),
            "{symbol}: the tightened budget rejects"
        );
    }
    assert!(matches!(
        tiny.demangle_with_scheme(Scheme::SwiftStable, "$s4main3fooyySiF"),
        Err(Error::InvalidSymbol(Scheme::SwiftStable))
    ));
    assert!(matches!(
        tiny.demangle_with_scheme(Scheme::SwiftLegacy, "_TtC5sugar7MyClass"),
        Err(Error::InvalidSymbol(Scheme::SwiftLegacy))
    ));
    assert!(
        tiny.selection()
            .allow(Scheme::SwiftStable)
            .demangle("$s4main3fooyySiF")
            .is_err()
    );
}

#[cfg(feature = "swift")]
#[test]
fn swift_dispatch_accepts_a_raised_budget() {
    // A legacy `_T` name past the default input cap: detection's prefilter
    // parses with the defaults and fails, so dispatch re-parses the bytes
    // with the configured budget.
    let symbol = std::format!("_TtSi{}", "x".repeat(crate::swift::MAX_INPUT_LEN));
    assert!(
        Demangler::new().demangle(&symbol).is_err(),
        "the default input cap rejects"
    );
    let raised =
        Demangler::with_budget(Budget::new().with_input_bytes(2 * crate::swift::MAX_INPUT_LEN));
    let demangled = raised
        .demangle(&symbol)
        .unwrap_or_else(|error| panic!("raised budget: {error}"));
    assert_eq!(demangled.scheme(), Scheme::SwiftLegacy);
    assert!(
        demangled
            .to_string()
            .starts_with("Swift.Int with unmangled suffix \""),
        "the raised budget parses the whole symbol"
    );
}

#[cfg(all(
    feature = "go",
    feature = "objc",
    feature = "kotlin-native",
    feature = "jni"
))]
#[test]
fn linear_backend_dispatch_honors_the_budget() {
    // Go, Objective-C, Kotlin/Native and JNI resolve the configured budget
    // on every dispatch path: the detected scheme, a forced scheme and a
    // selection. Detection is shape-only for Go, Objective-C and
    // Kotlin/Native, and retains a JNI tree that dispatch re-parses when a
    // budget is configured.
    let tiny = Demangler::with_budget(Budget::new().with_input_bytes(4));
    for (scheme, symbol) in [
        (Scheme::Go, "fmt.Println"),
        (Scheme::ObjC, "_OBJC_CLASS_$_Foo"),
        (Scheme::KotlinNative, "kfun:com.example#main(){}"),
        (Scheme::Jni, "Java_com_example_Foo_plain"),
    ] {
        assert!(
            Demangler::new().demangle(symbol).is_ok(),
            "{scheme}: the default budget accepts {symbol}"
        );
        assert!(
            matches!(tiny.demangle(symbol), Err(Error::Budget { .. })),
            "{scheme}: the tightened budget rejects {symbol}"
        );
        assert!(
            matches!(
                tiny.demangle_with_scheme(scheme, symbol),
                Err(Error::Budget { .. })
            ),
            "{scheme}: the forced scheme rejects {symbol}"
        );
        assert!(
            tiny.selection().allow(scheme).demangle(symbol).is_err(),
            "{scheme}: the selection rejects {symbol}"
        );
    }
}

#[cfg(feature = "swift")]
#[test]
fn detects_mach_o_underscore_swift() {
    // Mach-O prefixes Swift symbols with an underscore.
    assert_eq!(Scheme::detect("_$s4main3fooSi_yF"), Scheme::SwiftStable);
    assert_eq!(
        Scheme::detect("_$S3foo3barC3bas3zimyAaEC_tFTo"),
        Scheme::SwiftStable
    );
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
        assert_eq!(Scheme::detect(symbol).language(), Language::D, "{symbol}");
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
        let demangled_symbol = demangler
            .demangle(symbol)
            .unwrap_or_else(|error| panic!("{symbol}: {error}"));
        assert_eq!(demangled_symbol.scheme(), Scheme::D, "{symbol}");
        assert_eq!(demangled_symbol.language(), Language::D, "{symbol}");
        assert_eq!(demangled_symbol.to_string(), expected, "{symbol}");
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
        let demangled_symbol = demangler
            .demangle(symbol)
            .unwrap_or_else(|error| panic!("{symbol}: {error}"));
        assert_eq!(demangled_symbol.scheme(), Scheme::D, "{symbol}");
        assert_eq!(demangled_symbol.to_string(), "foo.bar()", "{symbol}");
    }
}

#[cfg(feature = "d")]
#[test]
fn d_printer_has_no_output_cap() {
    // Identifier back references expand a 45 KiB symbol past the old fixed
    // output ceiling; the printer no longer has one.
    let name_len = 20_000;
    let mut symbol = String::from("_D20000");
    symbol.push_str(&"a".repeat(name_len));
    for _ in 0..5_000 {
        let value = symbol.len() - 2;
        symbol.push('Q');
        symbol.push_str(&d_backref(value));
    }
    symbol.push('i');

    let demangled = Demangler::new()
        .demangle(&symbol)
        .expect("the expanded D symbol is accepted");
    assert_ne!(demangled.to_string(), "");
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
        let demangled_symbol = demangler
            .demangle(symbol)
            .unwrap_or_else(|error| panic!("{symbol}: {error}"));
        assert_eq!(demangled_symbol.scheme(), Scheme::Jni, "{symbol}");
        assert_eq!(demangled_symbol.language(), Language::Java, "{symbol}");
        assert_eq!(
            demangled_symbol.to_string(),
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
    let demangled_symbol = demangler.demangle("_ZN0E").unwrap();
    assert_eq!(demangled_symbol.scheme(), Scheme::RustLegacy);
    assert_eq!(demangled_symbol.language(), Language::Rust);
    assert_eq!(
        demangler.demangle_bytes(b"_ZN1a0E").unwrap().scheme(),
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
        demangler
            .demangle_with_schemes(&[], "main")
            .unwrap()
            .scheme(),
        Scheme::C
    );
}

#[test]
fn demangler_carries_a_budget() {
    use crate::budget::Limit;

    assert_eq!(Demangler::new().budget(), Budget::default());
    assert_eq!(Demangler::default().budget(), Budget::default());

    let budget = Budget::new().with_input_bytes(1024).with_parse_depth(16);
    let demangler = Demangler::with_budget(budget);
    assert_eq!(demangler.budget(), budget);
    assert_eq!(
        demangler.effective_budget(Scheme::CppItanium),
        budget.effective(Scheme::CppItanium)
    );
    assert_eq!(
        demangler.effective_budget(Scheme::CppItanium).input_bytes,
        Some(Limit::Bounded(1024))
    );

    let mut demangler = Demangler::new();
    demangler.set_budget(budget);
    assert_eq!(demangler.budget(), budget);
    // C has no parser budget; the text is unchanged.
    assert_eq!(demangler.demangle("main").unwrap().to_string(), "main");
    assert_eq!(
        demangler
            .selection()
            .allow(Scheme::C)
            .demangle("main")
            .unwrap()
            .to_string(),
        "main"
    );
}

#[test]
fn demangler_construction_stays_const() {
    const BUDGET: Budget = Budget::new().with_work(4096);
    const DEMANGLER: Demangler = Demangler::with_budget(BUDGET);
    const CONFIGURED: Budget = DEMANGLER.budget();

    assert_eq!(CONFIGURED, BUDGET);
    assert_eq!(Demangler::new().budget(), Budget::default());
}
