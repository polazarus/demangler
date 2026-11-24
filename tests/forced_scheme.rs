//! Forced-scheme entry points.
//!
//! [`Demangler::demangle_with_scheme`] and
//! [`Demangler::demangle_bytes_with_scheme`] bypass detection for names whose
//! automatic detection is intentionally conservative. These tests pin the
//! backend-validation, pass-through, feature-gating and COFF-integration
//! semantics of the explicit entry points, plus the text/byte equivalence.

use demangler::{Demangler, Error, Scheme};

#[test]
fn forced_c_and_unknown_are_plain_pass_through() {
    let demangler = Demangler::new();
    for scheme in [Scheme::C, Scheme::Unknown] {
        let demangled = demangler
            .demangle_bytes_with_scheme(scheme, b"main")
            .unwrap();
        assert_eq!(demangled.scheme(), scheme);
        assert_eq!(demangled.to_string(), "main");

        let text = demangler.demangle_with_scheme(scheme, "main").unwrap();
        assert_eq!(text.scheme(), scheme);
        assert_eq!(text.to_string(), "main");
    }

    let escaped = demangler
        .demangle_bytes_with_scheme(Scheme::C, b"plain\xff")
        .unwrap();
    assert_eq!(escaped.scheme(), Scheme::C);
    assert_eq!(escaped.to_string(), "plain\\xff");
}

/// One name per backend the explicit entry point can reach, so the text and
/// byte forms are compared on the same backend set.
fn forced_text_cases() -> Vec<(Scheme, &'static str)> {
    // Some pushes are feature-gated, so the binding stays `mut` in
    // feature-reduced builds.
    #[allow(unused_mut)]
    let mut cases: Vec<(Scheme, &'static str)> = vec![
        (Scheme::C, "main"),
        (Scheme::Unknown, "main"),
        (Scheme::Unknown, "not-a-symbol!"),
    ];
    #[cfg(feature = "rust")]
    {
        cases.push((Scheme::RustV0, "_RNvC3foo3bar"));
        cases.push((Scheme::RustLegacy, "_ZN4test4main17h1234567890abcdefE"));
    }
    #[cfg(feature = "cpp-itanium")]
    cases.push((Scheme::CppItanium, "_ZN3Foo3barEv"));
    #[cfg(feature = "cpp-msvc")]
    cases.push((Scheme::CppMsvc, "?x@@3HA"));
    #[cfg(feature = "cpp-gnu-v2")]
    {
        cases.push((Scheme::CppGnuV2, "foo__Fiv"));
        cases.push((Scheme::CppArm, "__ct__3fooFv"));
        cases.push((Scheme::CppEdg, "foo__Fi_v"));
        cases.push((Scheme::CppHp, "foo__XSP5_Fv"));
    }
    #[cfg(feature = "swift")]
    {
        cases.push((Scheme::SwiftStable, "$s4main5helloSSyYaKF"));
        cases.push((Scheme::SwiftLegacy, "_TtGSaSS_"));
    }
    #[cfg(feature = "go")]
    cases.push((Scheme::Go, "runtime.(*mLockProfile).captureStack"));
    #[cfg(feature = "objc")]
    cases.push((Scheme::ObjC, "_OBJC_CLASS_$_NSString"));
    #[cfg(feature = "kotlin-native")]
    cases.push((
        Scheme::KotlinNative,
        "kfun:com.example.Foo#bar(kotlin.Int){}",
    ));
    #[cfg(feature = "d")]
    cases.push((Scheme::D, "_Dmain"));
    #[cfg(feature = "jni")]
    cases.push((Scheme::Jni, "Java_com_example_Foo_bar__I"));
    #[cfg(feature = "ada")]
    cases.push((Scheme::Ada, "pkg__subp"));
    #[cfg(feature = "fortran")]
    cases.push((Scheme::Fortran, "foo_"));
    #[cfg(feature = "delphi")]
    cases.push((Scheme::Delphi, "DELPHI_PROBE_$$_NOARGS"));
    #[cfg(feature = "ocaml")]
    cases.push((Scheme::OCaml, "camlFoo.bar_1"));
    cases
}

#[test]
fn text_and_byte_forms_produce_identical_output() {
    let demangler = Demangler::new();
    // The text form must route to the same backend and render the same
    // `Demangled` as the byte form.
    for (scheme, symbol) in forced_text_cases() {
        let from_text = demangler
            .demangle_with_scheme(scheme, symbol)
            .unwrap_or_else(|error| panic!("forced {scheme:?} text form failed: {error}"));
        let from_bytes = demangler
            .demangle_bytes_with_scheme(scheme, symbol.as_bytes())
            .unwrap_or_else(|error| panic!("forced {scheme:?} byte form failed: {error}"));

        assert_eq!(from_text.scheme(), from_bytes.scheme(), "{scheme:?}");
        assert_eq!(from_text.language(), from_bytes.language(), "{scheme:?}");
        assert_eq!(
            from_text.to_string(),
            from_bytes.to_string(),
            "{scheme:?} text and byte forms diverged"
        );
    }
}

#[cfg(feature = "ada")]
#[test]
fn forced_ada_accepts_general_external_names() {
    let demangler = Demangler::new();
    // Detection is deliberately precision-only for Ada, so a general
    // `pkg__subp` name is not auto-claimed.
    assert_ne!(Scheme::detect("pkg__subp"), Scheme::Ada);

    let demangled = demangler
        .demangle_bytes_with_scheme(Scheme::Ada, b"pkg__subp")
        .expect("forced Ada name");
    assert_eq!(demangled.scheme(), Scheme::Ada);
    assert_eq!(demangled.language(), demangler::Language::Ada);
    assert_eq!(demangled.to_string(), "pkg.subp");

    let nested = demangler
        .demangle_with_scheme(Scheme::Ada, "pkg__child__nested")
        .expect("forced Ada name from UTF-8");
    assert_eq!(nested.to_string(), "pkg.child.nested");
}

#[cfg(feature = "fortran")]
#[test]
fn forced_fortran_accepts_external_names() {
    let demangler = Demangler::new();
    // A trailing underscore is an ordinary C name to detection.
    assert_ne!(Scheme::detect("foo_"), Scheme::Fortran);

    let demangled = demangler
        .demangle_bytes_with_scheme(Scheme::Fortran, b"foo_")
        .expect("forced Fortran external name");
    assert_eq!(demangled.scheme(), Scheme::Fortran);
    assert_eq!(demangled.language(), demangler::Language::Fortran);
    assert_eq!(demangled.to_string(), "foo");

    let module = demangler
        .demangle_with_scheme(Scheme::Fortran, "__m_MOD_x")
        .expect("forced Fortran module name");
    assert_eq!(module.to_string(), "m.x");
}

#[cfg(feature = "delphi")]
#[test]
fn forced_delphi_routes_dotted_units() {
    let demangler = Demangler::new();
    let symbol = "NS.ESC$_$TFOO_BAR_$__$$_METHODTWO$LONGINT";
    // Detection now prefers Delphi over Go for dotted FPC names; forcing the
    // scheme must still reach the same backend.
    #[cfg(feature = "delphi")]
    assert_eq!(Scheme::detect(symbol), Scheme::Delphi);

    let demangled = demangler
        .demangle_bytes_with_scheme(Scheme::Delphi, symbol.as_bytes())
        .expect("forced Delphi name");
    assert_eq!(demangled.scheme(), Scheme::Delphi);
    assert_eq!(demangled.language(), demangler::Language::Delphi);
    assert_eq!(demangled.to_string(), "NS.ESC.TFOO_BAR.METHODTWO(LONGINT)");

    let routine = demangler
        .demangle_with_scheme(Scheme::Delphi, "DELPHI_PROBE_$$_NOARGS")
        .expect("forced Delphi routine");
    assert_eq!(routine.to_string(), "DELPHI_PROBE.NOARGS()");
}

#[cfg(feature = "ocaml")]
#[test]
fn forced_ocaml_is_validated_by_the_ocaml_backend() {
    let demangler = Demangler::new();
    // OCaml detection is precision-only. The forced entry point must route to
    // the OCaml backend and must never fall through to plain C.
    match demangler.demangle_bytes_with_scheme(Scheme::OCaml, b"camlFoo.bar_1") {
        Ok(demangled) => {
            assert_eq!(demangled.scheme(), Scheme::OCaml);
            assert_eq!(demangled.language(), demangler::Language::OCaml);
            assert_eq!(demangled.to_string(), "Foo.bar");
        }
        Err(error) => panic!("unexpected forced OCaml error: {error}"),
    }
}

/// Forcing a scheme is exact: the backend validates the dialect or encoding it
/// parsed, so a name from another language or the other Rust encoding is an
/// error instead of a retag.
#[cfg(all(feature = "cpp-itanium", feature = "rust"))]
#[test]
fn forced_scheme_does_not_retag_another_language() {
    let demangler = Demangler::new();
    for (scheme, symbol) in [
        (Scheme::CppItanium, "_RNvCs1234_4test4main"),
        (Scheme::RustV0, "_ZN3Foo3barEv"),
        (Scheme::RustLegacy, "_RNvCs1234_4test4main"),
        (Scheme::RustV0, "_ZN4test4main17h1234567890abcdefE"),
    ] {
        let error = demangler
            .demangle_with_scheme(scheme, symbol)
            .expect_err("a forced scheme is exact");
        assert_eq!(error, Error::InvalidSymbol(scheme), "{scheme:?} {symbol}");
    }
}

/// The detected path keeps the C++ -> Rust fallback for an `_ZN` name that
/// only the legacy Rust grammar accepts; a forced scheme never retags.
#[cfg(all(feature = "cpp-itanium", feature = "rust"))]
#[test]
fn detected_cpp_falls_back_to_legacy_rust() {
    let demangler = Demangler::new();
    // An empty path and a zero-length element are valid legacy Rust and
    // invalid Itanium C++, so detection claims Itanium and the parse falls
    // back to Rust.
    for (symbol, expected) in [("_ZN1a0E", "a::"), ("_ZNE", "")] {
        assert_eq!(Scheme::detect(symbol), Scheme::CppItanium, "{symbol}");
        let demangled = demangler
            .demangle(symbol)
            .unwrap_or_else(|error| panic!("{symbol}: {error}"));
        assert_eq!(demangled.scheme(), Scheme::RustLegacy, "{symbol}");
        assert_eq!(demangled.language(), demangler::Language::Rust, "{symbol}");
        assert_eq!(demangled.to_string(), expected, "{symbol}");

        // The exact Itanium scheme and the other Rust encoding reject it.
        for scheme in [Scheme::CppItanium, Scheme::RustV0] {
            let error = demangler
                .demangle_with_scheme(scheme, symbol)
                .expect_err("a forced scheme is exact");
            assert_eq!(error, Error::InvalidSymbol(scheme), "{scheme:?} {symbol}");
        }
        let forced = demangler
            .demangle_with_scheme(Scheme::RustLegacy, symbol)
            .unwrap_or_else(|error| panic!("forced {symbol}: {error}"));
        assert_eq!(forced.scheme(), Scheme::RustLegacy, "{symbol}");
    }
}

/// Forcing one C++ dialect must not accept another dialect's name.
#[cfg(all(feature = "cpp-itanium", feature = "cpp-msvc"))]
#[test]
fn forced_cpp_dialects_are_exact() {
    let demangler = Demangler::new();
    for (scheme, symbol) in [
        (Scheme::CppMsvc, "_Z3foov"),
        (Scheme::CppItanium, "?x@@3HA"),
    ] {
        let error = demangler
            .demangle_with_scheme(scheme, symbol)
            .expect_err("a forced C++ dialect is exact");
        assert_eq!(error, Error::InvalidSymbol(scheme), "{scheme:?} {symbol}");
    }
}

/// Forcing one legacy dialect must not accept another legacy dialect's name.
#[cfg(feature = "cpp-gnu-v2")]
#[test]
fn forced_legacy_dialects_are_exact() {
    let demangler = Demangler::new();
    for (scheme, symbol) in [
        (Scheme::CppGnuV2, "__ct__3fooFv"),
        (Scheme::CppArm, "foo__Fiv"),
        (Scheme::CppEdg, "foo__XSP5_Fv"),
        (Scheme::CppHp, "foo__Fi_v"),
    ] {
        let error = demangler
            .demangle_with_scheme(scheme, symbol)
            .expect_err("a forced legacy dialect is exact");
        assert_eq!(error, Error::InvalidSymbol(scheme), "{scheme:?} {symbol}");
    }
}

/// Forcing one Swift encoding must not accept the other one's name.
#[cfg(feature = "swift")]
#[test]
fn forced_swift_encodings_are_exact() {
    let demangler = Demangler::new();
    for (scheme, symbol) in [
        (Scheme::SwiftLegacy, "$s4main3fooyySiF"),
        (Scheme::SwiftStable, "_TtSi"),
    ] {
        let error = demangler
            .demangle_with_scheme(scheme, symbol)
            .expect_err("a forced Swift encoding is exact");
        assert_eq!(error, Error::InvalidSymbol(scheme), "{scheme:?} {symbol}");
    }
}

#[cfg(feature = "cpp-itanium")]
#[test]
fn allocation_token_names_are_detected_as_itanium() {
    assert_eq!(Scheme::detect("__alloc_token__Znwm"), Scheme::CppItanium);
}

#[cfg(feature = "ada")]
#[test]
fn forced_scheme_still_validates_the_backend() {
    // Forcing a scheme bypasses detection, not parsing: a malformed Ada name
    // is an error rather than a C pass-through.
    assert!(matches!(
        Demangler::new().demangle_bytes_with_scheme(Scheme::Ada, b"ada__Foo"),
        Err(Error::InvalidSymbol(Scheme::Ada))
    ));
}

#[cfg(not(feature = "ada"))]
#[test]
fn forced_disabled_ada_scheme_is_rejected() {
    let demangler = Demangler::new();
    assert!(matches!(
        demangler.demangle_bytes_with_scheme(Scheme::Ada, b"pkg__subp"),
        Err(Error::InvalidSymbol(Scheme::Ada))
    ));
    assert!(matches!(
        demangler.demangle_with_scheme(Scheme::Ada, "pkg__subp"),
        Err(Error::InvalidSymbol(Scheme::Ada))
    ));
}

#[cfg(not(feature = "d"))]
#[test]
fn forced_disabled_d_scheme_is_rejected() {
    let demangler = Demangler::new();
    assert!(matches!(
        demangler.demangle_bytes_with_scheme(Scheme::D, b"_Dmain"),
        Err(Error::InvalidSymbol(Scheme::D))
    ));
    assert!(matches!(
        demangler.demangle_with_scheme(Scheme::D, "_Dmain"),
        Err(Error::InvalidSymbol(Scheme::D))
    ));
}

#[cfg(feature = "coff")]
#[test]
fn forced_scheme_peels_generic_coff_decorations() {
    let demangled = Demangler::new()
        .demangle_bytes_with_scheme(Scheme::C, b"_foo@8")
        .expect("forced C with a stdcall decoration");
    assert_eq!(demangled.scheme(), Scheme::C);
    assert_eq!(demangled.to_string(), "foo");
    let decoration = demangled.coff_decoration().expect("decoration metadata");
    assert_eq!(decoration.stack_bytes(), Some(8));

    // The text form must peel the same decoration.
    let from_text = Demangler::new()
        .demangle_with_scheme(Scheme::C, "_foo@8")
        .expect("forced C text with a stdcall decoration");
    assert_eq!(from_text.to_string(), "foo");
    assert_eq!(
        from_text
            .coff_decoration()
            .expect("decoration metadata")
            .stack_bytes(),
        Some(8)
    );
}

#[cfg(feature = "coff")]
#[test]
fn forced_scheme_accepts_target_decoded_coff_forms() {
    use demangler::coff::{self, DecorationKind, Target};

    // A bare x86 `_name` is ambiguous without the target, so the caller
    // decodes it first and passes the undecorated symbol to the forced API.
    let decoration = coff::parse_bytes_for(b"_foo", Target::X86).expect("x86 decoration");
    assert_eq!(decoration.kind(), DecorationKind::LeadingUnderscore);
    let demangled = Demangler::new()
        .demangle_bytes_with_scheme(Scheme::C, decoration.symbol())
        .expect("forced C name");
    assert_eq!(demangled.to_string(), "foo");
}
