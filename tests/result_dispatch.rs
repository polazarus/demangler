//! Focused result and dispatch contract tests.
//!
//! These pin the shared `Demangled` construction path, the single PE/COFF
//! decoration peel used by both byte entry points, forced-scheme equivalence,
//! and the stable rendered text of the C++/D/Rust/Swift owned-text backends.

#![cfg(any(
    feature = "coff",
    feature = "cpp-itanium",
    feature = "d",
    feature = "rust"
))]

use demangler::{Demangler, Scheme};

#[cfg(feature = "coff")]
#[test]
fn coff_decoration_is_reported_by_both_detected_byte_paths() {
    use demangler::coff::CallingConvention;

    let demangler = Demangler::new();
    for (symbol, inner, convention, stack) in [
        (
            "_coff_stdcall@8",
            "coff_stdcall",
            CallingConvention::Stdcall,
            Some(8),
        ),
        (
            "@coff_fastcall@8",
            "coff_fastcall",
            CallingConvention::Fastcall,
            Some(8),
        ),
        (
            "__imp__imported_stdcall@8",
            "imported_stdcall",
            CallingConvention::Stdcall,
            Some(8),
        ),
    ] {
        // The UTF-8 and raw-byte entry points both detect the language and
        // attach the decoration exactly once.
        for demangled in [
            demangler.demangle(symbol).unwrap(),
            demangler.demangle_bytes(symbol.as_bytes()).unwrap(),
        ] {
            assert_eq!(demangled.scheme(), Scheme::C, "{symbol}");
            assert_eq!(demangled.to_string(), inner, "{symbol}");
            let decoration = demangled
                .coff_decoration()
                .unwrap_or_else(|| panic!("{symbol}"));
            assert_eq!(decoration.symbol(), inner.as_bytes(), "{symbol}");
            assert_eq!(decoration.convention(), convention, "{symbol}");
            assert_eq!(decoration.stack_bytes(), stack, "{symbol}");
        }
    }
}

#[cfg(feature = "coff")]
#[test]
fn forced_scheme_shares_the_coff_peel_and_pass_through() {
    let demangler = Demangler::new();

    // A forced pass-through scheme still reports the decoration that was
    // peeled from the raw input.
    let unknown = demangler
        .demangle_with_scheme(Scheme::Unknown, "_foo@8")
        .expect("forced pass-through with a stdcall decoration");
    assert_eq!(unknown.scheme(), Scheme::Unknown);
    assert_eq!(unknown.to_string(), "foo");
    assert_eq!(
        unknown.coff_decoration().unwrap().stack_bytes(),
        Some(8),
        "forced decoration metadata"
    );

    // The detected and forced paths must agree on the decoration metadata.
    let detected = demangler.demangle("__imp__foo@8").unwrap();
    let forced = demangler
        .demangle_bytes_with_scheme(Scheme::C, b"__imp__foo@8")
        .unwrap();
    assert_eq!(forced.scheme(), detected.scheme());
    assert_eq!(forced.to_string(), detected.to_string());
    assert_eq!(
        forced.coff_decoration().unwrap().symbol(),
        detected.coff_decoration().unwrap().symbol()
    );
    assert_eq!(
        forced.coff_decoration().unwrap().import(),
        detected.coff_decoration().unwrap().import()
    );
}

#[cfg(all(feature = "coff", feature = "cpp-itanium"))]
#[test]
fn forced_cpp_keeps_a_peeled_decoration_and_cpp_text() {
    let demangler = Demangler::new();
    let forced = demangler
        .demangle_bytes_with_scheme(Scheme::CppItanium, b"__imp__Z3foov")
        .expect("forced C++ with an import decoration");
    assert_eq!(forced.scheme(), Scheme::CppItanium);
    assert_eq!(forced.to_string(), "foo()");
    let decoration = forced.coff_decoration().expect("decoration metadata");
    assert_eq!(decoration.symbol(), b"_Z3foov");
    assert!(decoration.import());
}

#[cfg(feature = "cpp-itanium")]
#[test]
fn cpp_owned_text_renders_stably() {
    let demangler = Demangler::new();
    for (symbol, expected) in [
        ("_ZN3Foo3barEv", "Foo::bar()"),
        ("_Z1fPFivE", "f(int (*)())"),
        ("_ZSt4cout", "std::cout"),
    ] {
        let demangled = demangler
            .demangle(symbol)
            .unwrap_or_else(|error| panic!("{symbol}: {error}"));
        assert_eq!(demangled.scheme(), Scheme::CppItanium, "{symbol}");
        let first = demangled.to_string();
        assert_eq!(first, expected, "{symbol}");
        // The validated rendered text is owned by the result, so repeated
        // Display calls cannot change it or fail.
        assert_eq!(demangled.to_string(), first, "{symbol}");
        assert_eq!(format!("{demangled}"), first, "{symbol}");

        let forced = demangler
            .demangle_with_scheme(Scheme::CppItanium, symbol)
            .unwrap_or_else(|error| panic!("forced {symbol}: {error}"));
        assert_eq!(forced.to_string(), first, "{symbol}");
    }
}

#[cfg(feature = "d")]
#[test]
fn d_owned_text_renders_stably() {
    let demangler = Demangler::new();
    for (symbol, expected) in [
        ("_Dmain", "D main"),
        ("_D3foo3barFZv", "foo.bar()"),
        ("_D4test3fooFiZi", "test.foo(int)"),
    ] {
        let demangled = demangler
            .demangle(symbol)
            .unwrap_or_else(|error| panic!("{symbol}: {error}"));
        assert_eq!(demangled.scheme(), Scheme::D, "{symbol}");
        let first = demangled.to_string();
        assert_eq!(first, expected, "{symbol}");
        assert_eq!(demangled.to_string(), first, "{symbol}");
        assert_eq!(format!("{demangled}"), first, "{symbol}");

        let forced = demangler
            .demangle_with_scheme(Scheme::D, symbol)
            .unwrap_or_else(|error| panic!("forced {symbol}: {error}"));
        assert_eq!(forced.to_string(), first, "{symbol}");
    }
}

#[cfg(feature = "rust")]
#[test]
fn rust_owned_text_renders_stably() {
    let demangler = Demangler::new();
    for (symbol, scheme) in [
        ("_RNvCs1234_4test4main", Scheme::RustV0),
        ("_ZN4test4main17h1234567890abcdefE", Scheme::RustLegacy),
    ] {
        let demangled = demangler
            .demangle(symbol)
            .unwrap_or_else(|error| panic!("{symbol}: {error}"));
        assert_eq!(demangled.scheme(), scheme, "{symbol}");
        let first = demangled.to_string();
        assert_eq!(demangled.to_string(), first, "{symbol}");
        assert_eq!(format!("{demangled}"), first, "{symbol}");
    }
}

#[cfg(feature = "swift")]
#[test]
fn swift_owned_text_renders_stably() {
    let demangler = Demangler::new();
    for (symbol, scheme, expected) in [
        (
            "$s4main3fooyySiF",
            Scheme::SwiftStable,
            "main.foo(Swift.Int) -> ()",
        ),
        ("_TtSi", Scheme::SwiftLegacy, "Swift.Int"),
    ] {
        let demangled = demangler
            .demangle(symbol)
            .unwrap_or_else(|error| panic!("{symbol}: {error}"));
        assert_eq!(demangled.scheme(), scheme, "{symbol}");
        assert_eq!(demangled.to_string(), expected, "{symbol}");
        assert_eq!(format!("{demangled}"), expected, "{symbol}");

        let forced = demangler
            .demangle_with_scheme(scheme, symbol)
            .unwrap_or_else(|error| panic!("forced {symbol}: {error}"));
        assert_eq!(forced.to_string(), expected, "{symbol}");
    }
}

#[test]
fn demangled_is_send_and_sync() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<demangler::Demangled<'static>>();
}
