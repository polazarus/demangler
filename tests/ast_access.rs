//! `Demangled::ast` and the `as_<backend>` shortcuts hand out the parsed tree.

use demangler::{Demangler, Scheme};

#[test]
fn plain_names_have_no_tree() {
    let demangled = Demangler::new().demangle("main").unwrap();
    assert_eq!(demangled.scheme(), Scheme::C);
    #[cfg(feature = "rust")]
    assert!(demangled.ast().is_none());
    #[cfg(feature = "rust")]
    assert!(demangled.as_rust().is_none());
}

#[cfg(feature = "cpp-itanium")]
#[test]
fn cpp_tree_matches_the_result() {
    use demangler::AstRef;

    let demangled = Demangler::new().demangle("_ZN3foo3barEv").unwrap();
    let Some(AstRef::Cpp(ast)) = demangled.ast() else {
        panic!("expected a C++ tree");
    };
    assert_eq!(ast.scheme(), demangled.scheme());
    assert_eq!(ast.to_string(), demangled.to_string());
    assert!(std::ptr::eq(ast, demangled.as_cpp().unwrap()));
    #[cfg(feature = "rust")]
    assert!(demangled.as_rust().is_none());
}

#[cfg(feature = "rust")]
#[test]
fn rust_tree_reports_its_scheme() {
    let demangled = Demangler::new().demangle("_RNvCs1234_4main4main").unwrap();
    let ast = demangled.as_rust().expect("a Rust tree");
    assert_eq!(ast.scheme(), Scheme::RustV0);
    assert_eq!(ast.to_string(), demangled.to_string());
}

#[cfg(feature = "swift")]
#[test]
fn swift_tree_prints_like_the_result() {
    let demangled = Demangler::new().demangle("$s4main3fooyySiF").unwrap();
    let ast = demangled.as_swift().expect("a Swift tree");
    assert_eq!(ast.scheme(), Scheme::SwiftStable);
    assert_eq!(ast.to_string(), demangled.to_string());
}

#[cfg(feature = "d")]
#[test]
fn d_keeps_its_tree() {
    let demangled = Demangler::new().demangle("_D4test3fooFZv").unwrap();
    let ast = demangled.as_d().expect("a D tree");
    assert_eq!(ast.scheme(), Scheme::D);
    assert_eq!(ast.to_string(), demangled.to_string());
}

#[cfg(feature = "go")]
#[test]
fn go_tree_is_reachable_through_ast() {
    use demangler::AstRef;

    let demangled = Demangler::new()
        .demangle_with_scheme(Scheme::Go, "fmt.Println")
        .unwrap();
    assert!(matches!(demangled.ast(), Some(AstRef::Go(_))));
    assert_eq!(demangled.as_go().unwrap().scheme(), Scheme::Go);
}

#[cfg(all(feature = "coff", feature = "cpp-msvc"))]
#[test]
fn a_decorated_symbol_exposes_the_inner_tree() {
    let demangled = Demangler::new().demangle("__imp_?x@@3HA").unwrap();
    assert!(demangled.coff_decoration().is_some());
    let ast = demangled.as_cpp().expect("the decorated C++ tree");
    assert_eq!(ast.scheme(), Scheme::CppMsvc);
}
