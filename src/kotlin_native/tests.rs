use alloc::string::ToString;

use super::*;

#[test]
fn parse_is_display() {
    assert_eq!(
        parse_bytes_with_budget(b"kfun:com.example#main(){}", DEFAULT_BUDGET)
            .unwrap()
            .to_string(),
        "com.example.main()"
    );
}

#[test]
fn parse_errors_carry_the_offset() {
    // The bad byte inside a slice-validated path.
    let error = parse_bytes_with_budget(b"kclass:com.ex!mple.Box", DEFAULT_BUDGET).unwrap_err();
    assert_eq!(error.kind(), crate::ParseErrorKind::Invalid);
    assert_eq!(error.offset(), Some(13), "{error}");
    // Where the signature parser stopped.
    let error =
        parse_bytes_with_budget(b"kfun:com.example#foo(kotlin.Int", DEFAULT_BUDGET).unwrap_err();
    assert!(error.offset().is_some(), "{error}");
    let error = parse_bytes_with_budget(b"kfoo:bar", DEFAULT_BUDGET).unwrap_err();
    assert_eq!(error.kind(), crate::ParseErrorKind::NotThisScheme);
}

#[test]
fn ast_exposes_structure() {
    let ast =
        parse_bytes_with_budget(b"kfun:com.example.Box#get(){}kotlin.Int", DEFAULT_BUDGET).unwrap();
    let Symbol::Function(function) = ast.symbol() else {
        panic!("not a function");
    };
    assert_eq!(function.path(), "com.example.Box");
    assert_eq!(function.name(), "get");
    assert!(function.params().is_empty());
    let Some(Type::Class { name, .. }) = function.return_type() else {
        panic!("not a class type");
    };
    assert_eq!(name, b"kotlin.Int");
}

#[test]
fn depth_budget_reports_recursion() {
    // The shared cursor's depth budget is the only path to
    // `RecursedTooDeep` in this backend; nesting past `MAX_DEPTH` must not
    // fall through to a generic budget error.
    let mut symbol = alloc::string::String::from("ktype:kotlin.Array<");
    symbol.push_str(&"kotlin.Array<".repeat(MAX_DEPTH + 1));
    symbol.push('*');
    symbol.push_str(&">".repeat(MAX_DEPTH + 1));
    assert!(
        matches!(parse_bytes_with_budget(symbol.as_bytes(), DEFAULT_BUDGET).unwrap_err(), e if e.budget_kind() == Some(crate::BudgetKind::ParseDepth))
    );
}

#[test]
fn printer_is_bounded() {
    // Every accepted input renders, linear in its length.
    let symbol = "kfun:com.example#foo(kotlin.Int){}";
    let ast = parse_bytes_with_budget(symbol.as_bytes(), DEFAULT_BUDGET).unwrap();
    let mut out = alloc::string::String::new();
    ast.print(&mut out).unwrap();
    assert!(out.len() <= 4 * symbol.len());
}
