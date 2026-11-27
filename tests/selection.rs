//! Scheme/language selection tests.
//!
//! [`Demangler::selection`] narrows the demangler to an explicit set of
//! schemes or languages instead of relying on detection. These tests pin the
//! selection semantics documented in the crate: exactness of one allowed
//! scheme, language groups, preference order, the pass-through-last rule,
//! empty-selection parity with [`Demangler::demangle`], disabled features,
//! COFF decorations, the two failure shapes and `Selection` reuse.

use demangler::{Demangler, Error, Language, Scheme, Selector};

/// Names that exercise every backend, so the parity test also covers the
/// feature-reduced builds where a name stays plain text.
const SEEDS: &[&str] = &[
    "_ZN3Foo3barEv",
    "?x@@3HA",
    "foo__Fiv",
    "__ct__3fooFv",
    "_RNvCs1234_4test4main",
    "_ZN4test4main17h1234567890abcdefE",
    "$s4main3fooyySiF",
    "_TtSi",
    "fmt.Println",
    "_Dmain",
    "Java_com_example_Foo_bar__I",
    "_OBJC_CLASS_$_NSString",
    "kclass:com.example.Box",
    "ada__exceptions__triggered_by_abort",
    "__fprobe_MOD_add",
    "DELPHI_PROBE_$$_NOARGS",
    "camlFoo.bar_1",
    "_foo@8",
    "main",
    "not-a-symbol!",
    "",
];

#[test]
fn empty_selection_matches_the_detected_path() {
    let demangler = Demangler::new();
    let selection = demangler.selection();
    for &symbol in SEEDS {
        let detected = demangler
            .demangle(symbol)
            .map(|demangled| (demangled.scheme(), demangled.to_string()));
        let selected = selection
            .demangle(symbol)
            .map(|demangled| (demangled.scheme(), demangled.to_string()));
        assert_eq!(selected, detected, "{symbol}: empty selection diverged");

        let detected = demangler
            .demangle_bytes(symbol.as_bytes())
            .map(|demangled| (demangled.scheme(), demangled.to_string()));
        let selected = selection
            .demangle_bytes(symbol.as_bytes())
            .map(|demangled| (demangled.scheme(), demangled.to_string()));
        assert_eq!(selected, detected, "{symbol}: byte form diverged");
    }
}

#[test]
fn empty_and_duplicate_selectors_are_harmless() {
    let demangler = Demangler::new();
    let empty = demangler
        .selection()
        .allow_schemes(&[])
        .allow_languages(&[])
        .prefer_schemes(&[])
        .prefer_languages(&[]);
    assert_eq!(
        empty.demangle("_ZN3Foo3barEv").map(|d| d.scheme()),
        demangler.demangle("_ZN3Foo3barEv").map(|d| d.scheme()),
    );

    let duplicated = demangler
        .selection()
        .allow(Scheme::C)
        .allow(Scheme::C)
        .prefer(Scheme::C);
    let demangled_symbol = duplicated.demangle("main").expect("plain C name");
    assert_eq!(demangled_symbol.scheme(), Scheme::C);
    assert_eq!(demangled_symbol.to_string(), "main");
}

#[test]
fn selectors_convert_from_schemes_and_languages() {
    assert_eq!(Selector::from(Scheme::C), Selector::Scheme(Scheme::C));
    assert_eq!(
        Selector::from(Language::Go),
        Selector::Language(Language::Go)
    );
}

#[cfg(feature = "cpp-itanium")]
#[test]
fn allow_scheme_selects_exactly_that_scheme() {
    let demangler = Demangler::new();
    let selection = demangler.selection().allow(Scheme::CppItanium);
    let demangled_symbol = selection.demangle("_Z3foov").expect("Itanium name");
    assert_eq!(demangled_symbol.scheme(), Scheme::CppItanium);
    assert_eq!(demangled_symbol.language(), Language::Cpp);
    assert_eq!(demangled_symbol.to_string(), "foo()");

    // The raw-byte twin renders identically and escapes invalid bytes.
    let bytes = selection
        .demangle_bytes(b"_ZN4F\xffooE")
        .expect("Itanium bytes");
    assert_eq!(bytes.scheme(), Scheme::CppItanium);
    assert_eq!(bytes.to_string(), "F\\xffoo");
}

#[cfg(all(feature = "cpp-itanium", feature = "cpp-msvc"))]
#[test]
fn one_allowed_scheme_reports_itself_when_it_fails() {
    let demangler = Demangler::new();
    let error = demangler
        .selection()
        .allow(Scheme::CppMsvc)
        .demangle("_Z3foov")
        .expect_err("an Itanium name is not MSVC");
    assert_eq!(error, Error::InvalidSymbol(Scheme::CppMsvc));
    assert_eq!(error.scheme(), Scheme::CppMsvc);
}

#[cfg(all(feature = "cpp-itanium", feature = "cpp-msvc"))]
#[test]
fn allow_language_covers_every_scheme_in_the_group() {
    let demangler = Demangler::new();
    let selection = demangler.selection().allow(Language::Cpp);
    for (symbol, scheme) in [
        ("_Z3foov", Scheme::CppItanium),
        ("?x@@3HA", Scheme::CppMsvc),
    ] {
        let demangled_symbol = selection
            .demangle(symbol)
            .unwrap_or_else(|error| panic!("{symbol}: {error}"));
        assert_eq!(demangled_symbol.scheme(), scheme, "{symbol}");
        assert_eq!(demangled_symbol.language(), Language::Cpp, "{symbol}");
    }
}

#[cfg(all(feature = "cpp-itanium", feature = "rust"))]
#[test]
fn prefer_decides_between_two_matching_schemes() {
    let demangler = Demangler::new();
    // Both grammars accept this name and print the same text, so only the
    // reported scheme distinguishes them.
    let symbol = "_ZN4test4main17h1234567890abcdefE";
    let base = demangler
        .selection()
        .allow(Scheme::CppItanium)
        .allow(Scheme::RustLegacy);

    let rust = base
        .prefer(Scheme::RustLegacy)
        .demangle(symbol)
        .expect("legacy Rust name");
    assert_eq!(rust.scheme(), Scheme::RustLegacy);
    assert_eq!(rust.to_string(), "test::main::h1234567890abcdef");

    let cpp = base
        .prefer(Scheme::CppItanium)
        .demangle(symbol)
        .expect("Itanium name");
    assert_eq!(cpp.scheme(), Scheme::CppItanium);
    assert_eq!(cpp.to_string(), rust.to_string());
}

#[cfg(all(feature = "cpp-itanium", feature = "rust"))]
#[test]
fn prefer_language_moves_the_whole_group_to_the_front() {
    let demangler = Demangler::new();
    let selection = demangler
        .selection()
        .allow(Language::Cpp)
        .allow(Language::Rust)
        .prefer(Language::Rust);
    let demangled_symbol = selection
        .demangle("_ZN4test4main17h1234567890abcdefE")
        .expect("both groups accept the name");
    assert_eq!(demangled_symbol.scheme(), Scheme::RustLegacy);
    assert_eq!(demangled_symbol.language(), Language::Rust);
}

#[cfg(all(feature = "cpp-itanium", feature = "rust"))]
#[test]
fn plural_helpers_match_the_single_selectors() {
    let demangler = Demangler::new();
    let symbol = "_ZN4test4main17h1234567890abcdefE";
    let plural = demangler
        .selection()
        .allow_schemes(&[Scheme::CppItanium, Scheme::RustLegacy])
        .prefer_schemes(&[Scheme::RustLegacy]);
    let single = demangler
        .selection()
        .allow(Scheme::CppItanium)
        .allow(Scheme::RustLegacy)
        .prefer(Scheme::RustLegacy);
    assert_eq!(
        plural.demangle(symbol).map(|d| d.scheme()),
        single.demangle(symbol).map(|d| d.scheme()),
    );

    let plural = demangler
        .selection()
        .allow_languages(&[Language::Cpp, Language::Rust])
        .prefer_languages(&[Language::Rust]);
    assert_eq!(
        plural.demangle(symbol).map(|d| d.scheme()),
        Ok(Scheme::RustLegacy),
    );
}

#[cfg(all(feature = "cpp-itanium", feature = "rust"))]
#[test]
fn slice_entry_points_preserve_candidate_order() {
    let demangler = Demangler::new();
    let symbol = "_ZN4test4main17h1234567890abcdefE";
    let cpp_first = demangler
        .selection()
        .prefer_schemes(&[Scheme::CppItanium, Scheme::RustLegacy])
        .demangle(symbol)
        .expect("both grammars parse");
    assert_eq!(cpp_first.scheme(), Scheme::CppItanium);
    let rust_first = demangler
        .selection()
        .prefer_schemes(&[Scheme::RustLegacy, Scheme::CppItanium])
        .demangle(symbol)
        .expect("both grammars parse");
    assert_eq!(rust_first.scheme(), Scheme::RustLegacy);
}

#[cfg(feature = "cpp-itanium")]
#[test]
fn pass_through_schemes_are_tried_last() {
    let demangler = Demangler::new();

    // C is allowed first, but pass-through candidates run last, so the exact
    // backend still wins.
    let selection = demangler
        .selection()
        .allow(Scheme::C)
        .allow(Scheme::CppItanium);
    let demangled_symbol = selection.demangle("_Z3foov").expect("Itanium name");
    assert_eq!(demangled_symbol.scheme(), Scheme::CppItanium);
    assert_eq!(demangled_symbol.to_string(), "foo()");

    // The same rule applies to Unknown.
    let selection = demangler
        .selection()
        .allow(Scheme::Unknown)
        .allow(Scheme::CppItanium);
    assert_eq!(
        selection.demangle("_Z3foov").map(|d| d.scheme()),
        Ok(Scheme::CppItanium),
    );

    // A name no selected parser accepts reaches the pass-through candidate.
    let selection = demangler
        .selection()
        .allow(Scheme::CppItanium)
        .allow(Scheme::C);
    let plain = selection.demangle("not-a-symbol!").expect("pass-through");
    assert_eq!(plain.scheme(), Scheme::C);
    assert_eq!(plain.to_string(), "not-a-symbol!");
}

#[cfg(feature = "cpp-itanium")]
#[test]
fn prefer_overrides_the_pass_through_last_rule() {
    let demangler = Demangler::new();
    for scheme in [Scheme::C, Scheme::Unknown] {
        let selection = demangler
            .selection()
            .allow(Scheme::CppItanium)
            .prefer(scheme);
        let plain = selection
            .demangle("not-a-symbol!")
            .expect("preferred pass-through");
        assert_eq!(plain.scheme(), scheme);
        assert_eq!(plain.to_string(), "not-a-symbol!");

        // A name the backend accepts still loses to the preferred pass-through.
        let passthrough = selection
            .demangle("_Z3foov")
            .expect("preferred pass-through");
        assert_eq!(passthrough.scheme(), scheme);
        assert_eq!(passthrough.to_string(), "_Z3foov");
    }
}

#[cfg(not(feature = "d"))]
#[test]
fn a_disabled_scheme_fails_and_falls_through() {
    let demangler = Demangler::new();

    // A single disabled scheme is the only candidate, so the selection reports
    // that scheme.
    let error = demangler
        .selection()
        .allow(Scheme::D)
        .demangle("_Dmain")
        .expect_err("the D backend is not compiled in");
    assert_eq!(error, Error::InvalidSymbol(Scheme::D));

    // With a later candidate the selection falls through to it.
    let plain = demangler
        .selection()
        .allow(Scheme::D)
        .allow(Scheme::C)
        .demangle("_Dmain")
        .expect("the pass-through candidate accepts the name");
    assert_eq!(plain.scheme(), Scheme::C);
    assert_eq!(plain.to_string(), "_Dmain");
}

#[cfg(feature = "coff")]
#[test]
fn selection_peels_coff_decorations_once() {
    let demangler = Demangler::new();
    let demangled_symbol = demangler
        .selection()
        .allow(Scheme::C)
        .demangle("_foo@8")
        .expect("stdcall name");
    assert_eq!(demangled_symbol.scheme(), Scheme::C);
    assert_eq!(demangled_symbol.to_string(), "foo");
    let decoration = demangled_symbol
        .coff_decoration()
        .expect("decoration metadata");
    assert_eq!(decoration.symbol(), b"foo");
    assert_eq!(decoration.stack_bytes(), Some(8));

    #[cfg(feature = "cpp-itanium")]
    {
        let demangled_symbol = demangler
            .selection()
            .allow(Scheme::CppItanium)
            .demangle("__imp__Z3foov")
            .expect("imported Itanium name");
        assert_eq!(demangled_symbol.scheme(), Scheme::CppItanium);
        assert_eq!(demangled_symbol.to_string(), "foo()");
        assert!(
            demangled_symbol
                .coff_decoration()
                .expect("decoration")
                .import()
        );
    }
}

#[cfg(all(feature = "cpp-itanium", feature = "cpp-msvc"))]
#[test]
fn many_failing_schemes_report_no_matching_scheme() {
    let demangler = Demangler::new();
    let error = demangler
        .selection()
        .allow(Scheme::CppItanium)
        .allow(Scheme::CppMsvc)
        .demangle("not-a-symbol!")
        .expect_err("no selected C++ dialect accepts the name");
    assert_eq!(error, Error::NoMatchingScheme);
    assert_eq!(error.scheme(), Scheme::Unknown);
    assert_eq!(
        error.to_string(),
        "no selected scheme could demangle the symbol"
    );
}

#[cfg(feature = "d")]
#[test]
fn a_single_member_language_reports_its_scheme() {
    let demangler = Demangler::new();
    // A one-scheme language behaves like one allowed scheme, not like a group.
    let error = demangler
        .selection()
        .allow(Language::D)
        .demangle("not-a-symbol!")
        .expect_err("not a D name");
    assert_eq!(error, Error::InvalidSymbol(Scheme::D));
}

#[cfg(feature = "cpp-itanium")]
#[test]
fn a_selection_is_copy_and_reusable() {
    let demangler = Demangler::new();
    let base = demangler.selection().allow(Scheme::CppItanium);
    let with_c = base.allow(Scheme::C);

    // `allow` consumes a copy, so `base` stays usable.
    assert_eq!(
        base.demangle("_Z3foov").map(|d| d.scheme()),
        Ok(Scheme::CppItanium),
    );
    assert_eq!(
        base.demangle("not-a-symbol!").map(|d| d.scheme()),
        Err(Error::InvalidSymbol(Scheme::CppItanium)),
    );

    let copied = base;
    assert_eq!(
        copied.demangle("_Z3foov").map(|d| d.to_string()),
        Ok(String::from("foo()")),
    );
    assert_eq!(
        with_c.demangle("not-a-symbol!").map(|d| d.scheme()),
        Ok(Scheme::C),
    );
}

#[cfg(all(feature = "cpp-itanium", feature = "cpp-msvc"))]
#[test]
fn language_and_scheme_entry_points_are_exact() {
    let demangler = Demangler::new();

    let demangled_symbol = demangler
        .demangle_with_language(Language::Cpp, "_Z3foov")
        .expect("Itanium name");
    assert_eq!(demangled_symbol.scheme(), Scheme::CppItanium);

    let demangled_symbol = demangler
        .selection()
        .prefer_schemes(&[Scheme::CppItanium, Scheme::CppMsvc])
        .demangle("?x@@3HA")
        .expect("MSVC name");
    assert_eq!(demangled_symbol.scheme(), Scheme::CppMsvc);
    assert_eq!(demangled_symbol.to_string(), "int x");

    let from_bytes = demangler
        .selection()
        .prefer_schemes(&[Scheme::CppItanium, Scheme::CppMsvc])
        .demangle_bytes(b"?x@@3HA")
        .expect("MSVC bytes");
    assert_eq!(from_bytes.scheme(), demangled_symbol.scheme());
    assert_eq!(from_bytes.to_string(), demangled_symbol.to_string());
}

#[cfg(feature = "go")]
#[test]
fn languages_union_and_pick_the_only_match() {
    let demangler = Demangler::new();
    // Only the Go candidate accepts a dotted name, so the union is exact
    // regardless of the order the two groups are listed in.
    let demangled_symbol = demangler
        .selection()
        .prefer_languages(&[Language::Cpp, Language::Go])
        .demangle("fmt.Println")
        .expect("Go name");
    assert_eq!(demangled_symbol.scheme(), Scheme::Go);
    assert_eq!(demangled_symbol.to_string(), "fmt.Println");

    let from_bytes = demangler
        .selection()
        .prefer_languages(&[Language::Cpp, Language::Go])
        .demangle_bytes(b"fmt.Println")
        .expect("Go bytes");
    assert_eq!(from_bytes.scheme(), Scheme::Go);
}

#[cfg(all(feature = "cpp-itanium", feature = "rust"))]
#[test]
fn explicit_selection_does_not_use_the_detected_rust_fallback() {
    let demangler = Demangler::new();
    // The detected path retags this Rust-only `_ZN` name as legacy Rust.
    let detected = demangler.demangle("_ZN1a0E").expect("legacy Rust name");
    assert_eq!(detected.scheme(), Scheme::RustLegacy);
    assert_eq!(detected.to_string(), "a::");

    // An explicit selection is exact: no C++ dialect retags, and with several
    // failing candidates the selection reports no matching scheme.
    let error = demangler
        .selection()
        .allow(Scheme::CppItanium)
        .allow(Scheme::RustV0)
        .demangle("_ZN1a0E")
        .expect_err("neither selected scheme accepts the name");
    assert_eq!(error, Error::NoMatchingScheme);

    let rust = demangler
        .selection()
        .allow(Scheme::RustLegacy)
        .demangle("_ZN1a0E")
        .expect("the exact encoding accepts the name");
    assert_eq!(rust.scheme(), Scheme::RustLegacy);
}

#[test]
fn results_and_selections_compare_and_clone() {
    let demangler = Demangler::new();
    let selection = demangler.selection().allow(Language::C);
    assert_eq!(selection, demangler.selection().allow(Language::C));
    assert_ne!(selection, demangler.selection());
    let result = selection.demangle("main").unwrap();
    assert_eq!(result.clone().to_string(), result.to_string());
}
