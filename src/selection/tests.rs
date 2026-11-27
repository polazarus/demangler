use std::string::ToString;
use std::vec::Vec;

use super::*;

/// Collect the candidate order of a selection.
fn order(selection: Selection) -> Vec<Scheme> {
    selection.candidates().collect()
}

#[test]
fn detection_order_covers_every_scheme_once() {
    assert_eq!(DETECTION_ORDER.len(), Scheme::COUNT);
    for scheme in Scheme::ALL {
        assert_eq!(
            DETECTION_ORDER
                .iter()
                .filter(|other| **other == scheme)
                .count(),
            1,
            "{scheme} is listed once"
        );
    }
    // The pass-through schemes are the fallback of last resort.
    assert_eq!(DETECTION_ORDER[Scheme::COUNT - 2], Scheme::C);
    assert_eq!(DETECTION_ORDER[Scheme::COUNT - 1], Scheme::Unknown);
}

#[test]
fn empty_selection_has_no_candidates() {
    assert_eq!(order(Demangler::new().selection()), []);
}

#[test]
fn allow_scheme_uses_detection_order() {
    let selection = Demangler::new()
        .selection()
        .allow(Scheme::Unknown)
        .allow(Scheme::CppMsvc)
        .allow(Scheme::RustV0);
    assert_eq!(
        order(selection),
        [Scheme::RustV0, Scheme::CppMsvc, Scheme::Unknown]
    );
}

#[test]
fn allow_language_expands_the_group() {
    let selection = Demangler::new().selection().allow(Language::Cpp);
    assert_eq!(
        order(selection),
        [
            Scheme::CppItanium,
            Scheme::CppMsvc,
            Scheme::CppGnuV2,
            Scheme::CppArm,
            Scheme::CppEdg,
            Scheme::CppHp,
        ]
    );
}

#[test]
fn prefer_moves_schemes_to_the_front() {
    let selection = Demangler::new()
        .selection()
        .allow(Language::Cpp)
        .prefer(Scheme::CppHp)
        .prefer(Scheme::CppMsvc);
    assert_eq!(
        order(selection)[..2],
        [Scheme::CppHp, Scheme::CppMsvc],
        "preference is in call order"
    );
    assert_eq!(order(selection).len(), 6, "duplicates are dropped");
}

#[test]
fn prefer_language_moves_the_group_to_the_front() {
    let selection = Demangler::new()
        .selection()
        .allow(Language::Cpp)
        .prefer(Language::Rust);
    assert_eq!(order(selection)[..2], [Scheme::RustLegacy, Scheme::RustV0]);
    assert_eq!(order(selection).len(), 8);
}

#[test]
fn prefer_c_and_unknown_override_the_tail_rule() {
    let selection = Demangler::new()
        .selection()
        .allow(Scheme::CppItanium)
        .allow(Scheme::C)
        .prefer(Scheme::C);
    assert_eq!(order(selection), [Scheme::C, Scheme::CppItanium]);

    let selection = Demangler::new()
        .selection()
        .allow(Scheme::CppItanium)
        .allow(Scheme::Unknown)
        .prefer(Scheme::Unknown);
    assert_eq!(order(selection), [Scheme::Unknown, Scheme::CppItanium]);
}

#[test]
fn empty_slices_are_no_ops() {
    let selection = Demangler::new()
        .selection()
        .allow_schemes(&[])
        .allow_languages(&[])
        .prefer_schemes(&[])
        .prefer_languages(&[]);
    assert_eq!(order(selection), []);
}

#[test]
fn plural_methods_match_their_singular_forms() {
    assert_eq!(
        order(
            Demangler::new()
                .selection()
                .allow_schemes(&[Scheme::Go, Scheme::CppItanium])
        ),
        order(
            Demangler::new()
                .selection()
                .allow(Scheme::Go)
                .allow(Scheme::CppItanium)
        )
    );
    assert_eq!(
        order(
            Demangler::new()
                .selection()
                .prefer_languages(&[Language::Rust, Language::C])
        ),
        order(
            Demangler::new()
                .selection()
                .prefer(Language::Rust)
                .prefer(Language::C)
        )
    );
    assert_eq!(
        order(
            Demangler::new()
                .selection()
                .prefer_schemes(&[Scheme::ObjC, Scheme::D])
        ),
        order(
            Demangler::new()
                .selection()
                .prefer(Scheme::ObjC)
                .prefer(Scheme::D)
        )
    );
}

#[cfg(all(feature = "cpp-itanium", feature = "rust"))]
#[test]
fn a_budget_error_is_not_reported_as_a_selection_failure() {
    let selection = Demangler::with_budget(crate::Budget::new().with_input_bytes(0))
        .selection()
        .allow(Scheme::CppItanium)
        .allow(Scheme::RustLegacy);
    assert!(matches!(
        selection.demangle("_Z3foov"),
        Err(Error::Budget { .. })
    ));
}

#[test]
fn empty_selection_delegates_to_detection() {
    let demangler = Demangler::new();
    let selection = demangler.selection();
    for symbol in [&b"main"[..], b"12345", b"plain\xff"] {
        let detected = demangler.demangle_bytes(symbol).unwrap();
        let selected = selection.demangle_bytes(symbol).unwrap();
        assert_eq!(selected.scheme(), detected.scheme(), "{symbol:?}");
        assert_eq!(selected.to_string(), detected.to_string(), "{symbol:?}");
    }
    assert_eq!(selection.demangle("main").unwrap().scheme(), Scheme::C);
}

#[cfg(feature = "cpp-itanium")]
#[test]
fn single_allowed_scheme_reports_invalid_symbol() {
    let error = Demangler::new()
        .selection()
        .allow(Scheme::CppItanium)
        .demangle("not a c++ symbol")
        .unwrap_err();
    assert_eq!(error, Error::InvalidSymbol(Scheme::CppItanium));
}

#[cfg(feature = "go")]
#[test]
fn one_member_language_behaves_like_one_scheme() {
    let error = Demangler::new()
        .selection()
        .allow(Language::Go)
        .demangle("fmt.Println[")
        .unwrap_err();
    assert_eq!(error.scheme(), Scheme::Go);
}

#[cfg(all(feature = "cpp-itanium", feature = "cpp-msvc"))]
#[test]
fn several_failed_candidates_report_no_matching_scheme() {
    let error = Demangler::new()
        .selection()
        .allow(Scheme::CppItanium)
        .allow(Scheme::CppMsvc)
        .demangle("not-a-symbol!")
        .unwrap_err();
    assert_eq!(error, Error::NoMatchingScheme);
    assert_eq!(error.scheme(), Scheme::Unknown);
}

#[cfg(feature = "cpp-itanium")]
#[test]
fn exact_scheme_rejects_another_cpp_dialect() {
    let demangler = Demangler::new();
    let error = demangler
        .selection()
        .allow(Scheme::CppItanium)
        .demangle("?x@@3HA")
        .unwrap_err();
    assert_eq!(error, Error::InvalidSymbol(Scheme::CppItanium));
}

#[cfg(feature = "cpp-msvc")]
#[test]
fn preferred_scheme_wins_over_detection_order() {
    let demangler = Demangler::new();
    let demangled_symbol = demangler
        .selection()
        .allow(Language::Cpp)
        .prefer(Scheme::CppMsvc)
        .demangle("?x@@3HA")
        .unwrap();
    assert_eq!(demangled_symbol.scheme(), Scheme::CppMsvc);
    assert_eq!(demangled_symbol.to_string(), "int x");
}

#[cfg(all(feature = "cpp-itanium", feature = "go"))]
#[test]
fn failing_candidate_falls_through_to_the_next() {
    let demangled = Demangler::new()
        .selection()
        .prefer(Scheme::CppItanium)
        .allow(Scheme::Go)
        .demangle("fmt.Println")
        .unwrap();
    assert_eq!(demangled.scheme(), Scheme::Go);
}

#[cfg(all(feature = "coff", feature = "cpp-itanium"))]
#[test]
fn coff_decoration_is_reported_on_the_winning_candidate() {
    let demangled = Demangler::new()
        .selection()
        .allow(Scheme::CppItanium)
        .demangle("__imp__Z3foov")
        .unwrap();
    assert_eq!(demangled.scheme(), Scheme::CppItanium);
    assert_eq!(demangled.to_string(), "foo()");
    assert!(
        demangled
            .coff_decoration()
            .is_some_and(crate::coff::Decoration::import)
    );
}

#[test]
fn selection_is_copy_and_reusable() {
    let selection = Demangler::new().selection().allow(Scheme::C);
    let copy = selection;
    assert_eq!(copy.demangle("main").unwrap().to_string(), "main");
    assert_eq!(selection.demangle("foo").unwrap().to_string(), "foo");
}

#[cfg(feature = "cpp-itanium")]
#[test]
fn selection_snapshots_the_demangler_budget() {
    let mut demangler = Demangler::new();
    let selection = demangler.selection().allow(Scheme::CppItanium);
    demangler.set_budget(crate::Budget::new().with_input_bytes(0));

    // The selection kept the budget it was built with.
    assert_eq!(selection.demangle("_Z3foov").unwrap().to_string(), "foo()");
    assert!(
        demangler
            .selection()
            .allow(Scheme::CppItanium)
            .demangle("_Z3foov")
            .is_err()
    );
}

#[test]
fn selector_converts_from_both_targets() {
    assert_eq!(Selector::from(Scheme::Go), Selector::Scheme(Scheme::Go));
    assert_eq!(
        Selector::from(Language::Java),
        Selector::Language(Language::Java)
    );
}
