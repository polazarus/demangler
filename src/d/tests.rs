use alloc::string::{String, ToString};

use super::*;

fn demangle(symbol: &str) -> String {
    parse_bytes_with_budget(symbol.as_bytes(), DEFAULT_BUDGET)
        .unwrap()
        .to_string()
}

#[test]
fn strips_linker_decorations() {
    for symbol in [
        "_D3foo3barFZv.llvm.9D1C9369",
        "_D3foo3barFZv@GLIBC_2.2.5",
        "_D3foo3barFZv@@GLIBC_2.2.5",
        "_D3foo3barFZv@16",
    ] {
        assert_eq!(demangle(symbol), "foo.bar()", "{symbol}");
        assert!(is_d(symbol.as_bytes()), "{symbol}");
    }
}

#[test]
fn parses_main() {
    assert_eq!(demangle("_Dmain"), "D main");
}

#[test]
fn parses_function_names() {
    assert_eq!(demangle("_D3foo3barFZv"), "foo.bar()");
    assert_eq!(demangle("_D4test3fooFiZi"), "test.foo(int)");
}

#[test]
fn parses_arrays_and_delegates() {
    assert_eq!(
        demangle("_D3foo3barFAiG5iHAyaiZi"),
        "foo.bar(int[], int[5], int[immutable(char)[]])"
    );
    assert_eq!(demangle("_D3foo3barFDFiZiZi"), "foo.bar(int(int) delegate)");
}

#[test]
fn parses_templates_and_backrefs() {
    assert_eq!(
        demangle("_D3app__T3BoxTiZQh3getMFZi"),
        "app.Box!(int).Box.get()"
    );
}

#[test]
fn parses_specials() {
    assert_eq!(demangle("_D4test3Foo6__initZ"), "initializer for test.Foo");
    assert_eq!(demangle("_D4test3Foo6__vtblZ"), "vtable for test.Foo");
    assert_eq!(demangle("_D4test3Foo7__ClassZ"), "ClassInfo for test.Foo");
    assert_eq!(demangle("_D4test12__ModuleInfoZ"), "ModuleInfo for test");
    assert_eq!(
        demangle("_D4test3Foo11__moduleRefZ"),
        "test.Foo.__moduleRef"
    );
}

#[test]
fn parses_parameter_marker_order() {
    // The mangler emits `M` (scope) and `Nk` (return) in either order and
    // the printed text follows the encoded order.
    assert_eq!(demangle("_D3foo3barFMiZv"), "foo.bar(scope int)");
    assert_eq!(demangle("_D3foo3barFNkiZv"), "foo.bar(return int)");
    assert_eq!(demangle("_D3foo3barFMNkiZv"), "foo.bar(scope return int)");
    assert_eq!(demangle("_D3foo3barFNkMiZv"), "foo.bar(return scope int)");
    assert_eq!(demangle("_D3foo3barFNkKiZv"), "foo.bar(return ref int)");
    assert_eq!(demangle("_D3foo3barFNkJiZv"), "foo.bar(return out int)");
    assert_eq!(
        demangle("_D3foo3barFMNkKiZv"),
        "foo.bar(scope return ref int)"
    );
    assert_eq!(
        demangle("_D3foo3barFNkMKiZv"),
        "foo.bar(return scope ref int)"
    );
}

#[test]
fn parses_thunk_wrappers() {
    // A this-adjusting thunk is named `_DThn<offset>_` plus the symbol it
    // thunks to; the wrapper does not change the name.
    assert_eq!(demangle("_DThn16_3foo3barFZv"), "foo.bar()");
    assert_eq!(demangle("__DThn24_3foo3barFiZi"), "foo.bar(int)");
    assert!(is_d(b"_DThn16_3foo3barFZv"));
    assert!(parse_bytes_with_budget(b"_DThn_3foo3barFZv", DEFAULT_BUDGET).is_err());
    assert!(parse_bytes_with_budget(b"_DThn16_", DEFAULT_BUDGET).is_err());
}

#[test]
fn parses_backreferenced_function_type() {
    // `M Q<ref>` stands for the whole function type of a nested function,
    // return type included, so no type follows it.
    assert_eq!(
        demangle("_D3foo3barFDFZvZ1aMQh"),
        "foo.bar(void() delegate).a()"
    );
    // A reference to a length digit is an identifier back reference, not
    // a function type, so the signature cannot start there.
    assert!(parse_bytes_with_budget(b"_D3foo3barFDFZvZ1aMQd", DEFAULT_BUDGET).is_err());
}

#[test]
fn rejects_bare_prefix_and_truncation() {
    assert!(parse_bytes_with_budget(b"_D", DEFAULT_BUDGET).is_err());
    assert!(parse_bytes_with_budget(b"_Dfoo", DEFAULT_BUDGET).is_err());
    assert!(parse_bytes_with_budget(b"_D3foo3bar", DEFAULT_BUDGET).is_err());
    assert!(!is_d(b"_D"));
    assert!(!is_d(b"_Dfoo"));
}

#[test]
fn accepts_macho_prefix() {
    assert_eq!(demangle("__D3foo3barFZv"), "foo.bar()");
}

#[test]
fn parses_raw_bytes() {
    let ast = parse_bytes_with_budget(b"_D8foo\xff\xfebarFZv", DEFAULT_BUDGET).unwrap();
    assert_eq!(ast.to_string(), "foo\\xff\\xfebar()");
}

#[test]
fn budgets_are_bounded() {
    let long = alloc::format!("_D{}", "A".repeat(MAX_INPUT_LEN));
    assert!(
        matches!(parse_bytes_with_budget(long.as_bytes(), DEFAULT_BUDGET), Err(e) if e.budget_kind().is_some())
    );

    let deep = alloc::format!("_D3foo3barF{}iZv", "P".repeat(MAX_DEPTH + 1));
    assert!(
        matches!(parse_bytes_with_budget(deep.as_bytes(), DEFAULT_BUDGET), Err(e) if e.budget_kind() == Some(crate::BudgetKind::ParseDepth))
    );
}

#[test]
fn work_budget_bounds_back_reference_rescans() {
    // Each parameter type is a tuple of two back references to the previous
    // parameter type, so one re-scan costs twice its target and the work
    // grows as 2^n while the input stays a few hundred bytes. The work
    // budget, not the input or depth budget, has to stop the re-scan.
    assert!(parse_bytes_with_budget(nested_tuple_params(4).as_bytes(), DEFAULT_BUDGET).is_ok());

    let symbol = nested_tuple_params(20);
    assert!(symbol.len() < MAX_INPUT_LEN);
    assert!(
        matches!(parse_bytes_with_budget(symbol.as_bytes(), DEFAULT_BUDGET), Err(e) if e.budget_kind().is_some())
    );
}

/// One function parameter per level; each level is `B2` plus two back
/// references to the level below it.
fn nested_tuple_params(levels: usize) -> String {
    let mut symbol = String::from("_D3foo3barF");
    let mut target = symbol.len();
    symbol.push('i');
    for _ in 0..levels {
        let start = symbol.len();
        let mut level = String::from("B2");
        for _ in 0..2 {
            let value = start + level.len() - target;
            level.push('Q');
            level.push_str(&backref(value));
        }
        symbol.push_str(&level);
        target = start;
    }
    symbol.push_str("Zv");
    symbol
}

/// Encode a positive base-26 back-reference position.
fn backref(value: usize) -> String {
    let mut digits = alloc::vec![b'a' + u8::try_from(value % 26).unwrap()];
    let mut rest = value / 26;
    while rest > 0 {
        digits.push(b'A' + u8::try_from(rest % 26).unwrap());
        rest /= 26;
    }
    digits.reverse();
    String::from_utf8(digits).unwrap()
}

#[test]
fn printer_has_no_output_budget() {
    let symbol = "_D3foo3barFZv";
    let ast = parse_bytes_with_budget(symbol.as_bytes(), DEFAULT_BUDGET).unwrap();
    let mut out = String::new();
    ast.print(&mut out).unwrap();
    assert_ne!(out, "");
}

#[test]
fn fake_parent_chain_is_stack_safe() {
    // `4__S0` is one length-prefixed fake parent. 12 000 of them fit in
    // the input budget and used to cost one `parse_identifier` frame each;
    // the chain is consumed in a loop, so it must parse on a small stack.
    let mut symbol = String::from("_D");
    for _ in 0..12_000 {
        symbol.push_str("4__S0");
    }
    symbol.push_str("1aZ");

    let rendered = std::thread::Builder::new()
        .stack_size(512 * 1024)
        .spawn(move || {
            parse_bytes_with_budget(symbol.as_bytes(), DEFAULT_BUDGET)
                .expect("fake-parent chain parses")
                .to_string()
        })
        .expect("spawn")
        .join()
        .expect("no stack overflow");
    assert_eq!(rendered, "a");
}

#[test]
fn parse_errors_carry_the_offset() {
    // `!` is not a parameter type at byte 12.
    let error = parse_bytes_with_budget(b"_D4test3fooF!Zv", DEFAULT_BUDGET).unwrap_err();
    assert_eq!(error.kind(), crate::ParseErrorKind::Invalid);
    assert_eq!(error.offset(), Some(12), "{error}");
    let error = parse_bytes_with_budget(b"_D4test3fooFZ", DEFAULT_BUDGET).unwrap_err();
    assert_eq!(
        error.kind(),
        crate::ParseErrorKind::UnexpectedEnd,
        "{error}"
    );
    let error = parse_bytes_with_budget(b"main", DEFAULT_BUDGET).unwrap_err();
    assert_eq!(error.kind(), crate::ParseErrorKind::NotThisScheme);
}
