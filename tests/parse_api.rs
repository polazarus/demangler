//! The backend `parse` functions share one signature and one error type.

#![cfg(all(
    feature = "cpp-itanium",
    feature = "rust",
    feature = "swift",
    feature = "go",
    feature = "d"
))]

use demangler::{Budget, BudgetKind, ParseError, ParseErrorKind, Scheme};

/// One backend entry point, reduced to the scheme of what it parsed.
type Parse = fn(&[u8], Budget) -> Result<Scheme, ParseError>;

/// The default budget accepts what a tiny input budget rejects, in every
/// backend, and the rejection names the input dimension.
#[test]
fn the_budget_reaches_every_backend() {
    let tiny = Budget::new().with_input_bytes(4);
    let cases: [(&[u8], Parse); 5] = [
        (b"_ZN3foo3barEv", |s, b| {
            demangler::cpp::parse(s, b).map(|a| a.scheme())
        }),
        (b"_RNvCs1234_4main4main", |s, b| {
            demangler::rust::parse(s, b).map(|a| a.scheme())
        }),
        (b"$s4main3fooyySiF", |s, b| {
            demangler::swift::parse(s, b).map(|a| a.scheme())
        }),
        (b"fmt.Println", |s, b| {
            demangler::go::parse(s, b).map(|a| a.scheme())
        }),
        (b"_D4test3fooFZv", |s, b| {
            demangler::d::parse(s, b).map(|a| a.scheme())
        }),
    ];
    for (symbol, parse) in cases {
        let name = String::from_utf8_lossy(symbol);
        assert!(parse(symbol, Budget::default()).is_ok(), "{name}");
        let error = parse(symbol, tiny).unwrap_err();
        assert!(
            matches!(error.kind(), ParseErrorKind::Budget(_)),
            "{name}: {error}"
        );
    }
    assert_eq!(
        demangler::cpp::parse(b"_ZN3foo3barEv", tiny)
            .unwrap_err()
            .budget_kind(),
        Some(BudgetKind::Input)
    );
}

#[test]
fn errors_share_one_vocabulary() {
    let error = demangler::cpp::parse(b"main", Budget::default()).unwrap_err();
    assert_eq!(error.kind(), ParseErrorKind::NotThisScheme);
    let error = demangler::swift::parse(b"main", Budget::default()).unwrap_err();
    assert_eq!(error.kind(), ParseErrorKind::NotThisScheme);
    let error = demangler::go::parse(b"(", Budget::default()).unwrap_err();
    assert_eq!(error.kind(), ParseErrorKind::Invalid);
    assert_eq!(error.budget_kind(), None);
}

#[test]
fn cpp_errors_carry_an_offset() {
    let error = demangler::cpp::parse(b"_ZN3foo", Budget::default()).unwrap_err();
    assert_eq!(error.kind(), ParseErrorKind::UnexpectedEnd);
    assert!(error.offset().is_some());
    assert!(error.to_string().contains("at byte"), "{error}");
}

/// `cpp::parse` applies the demangler's print gates, so every tree it returns
/// prints.
#[test]
fn cpp_trees_always_print() {
    let error = demangler::cpp::parse(b"_Z1fIiEvT0_", Budget::default()).unwrap_err();
    assert_eq!(error.kind(), ParseErrorKind::Invalid);
    let ast = demangler::cpp::parse(b"_Z1fIiEvT_", Budget::default()).unwrap();
    assert_eq!(ast.to_string(), "void f<int>(int)");
}

/// Linker suffixes the demangler strips are stripped by the backends too,
/// except the ThinLTO ending, which the Itanium printer renders as LLVM does.
#[test]
fn linker_suffixes_match_the_demangler() {
    let cpp = demangler::cpp::parse(b"_Z3foov@@GLIBC_2.2", Budget::default()).unwrap();
    assert_eq!(cpp.to_string(), "foo()");
    let swift = demangler::swift::parse(b"$s4main3fooyySiF.llvm.12AB", Budget::default()).unwrap();
    assert_eq!(swift.to_string(), "main.foo(Swift.Int) -> ()");
    let clone = demangler::cpp::parse(b"_Z3foov.llvm.1234", Budget::default()).unwrap();
    assert_eq!(clone.to_string(), "foo() (.llvm.1234)");
}
