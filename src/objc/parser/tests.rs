use alloc::string::ToString;

use super::*;
use crate::objc::{DEFAULT_BUDGET, MAX_INPUT_LEN};

/// Parse with the compiled-in budget, the way the public entry point does.
fn parse_with(symbol: &[u8]) -> Result<Ast<'_>, ParseError> {
    parse_bytes(symbol, ParseLimits::from_effective(DEFAULT_BUDGET))
}

fn text(symbol: &str) -> alloc::string::String {
    parse_with(symbol.as_bytes())
        .unwrap_or_else(|error| panic!("{symbol}: {error}"))
        .to_string()
}

#[test]
fn byte_path_matches_the_text_path() {
    for symbol in [
        "_OBJC_CLASS_$_NSString",
        "__OBJC_$_CATEGORY_NSString_$_Drawing",
        "_OBJC_IVAR_$_Foo._ivar",
        "__objc_ivar_offset_Foo._ivar",
        "_OBJC_METH_VAR_NAME_0",
        "-[Foo bar:baz:]",
        "+[Root(Cat) catMethod]",
        "___block_descriptor_40_e8_32r_e5_v8?0l",
    ] {
        assert_eq!(
            text(symbol),
            crate::objc::parse_bytes_with_budget(symbol.as_bytes(), DEFAULT_BUDGET)
                .unwrap()
                .to_string()
        );
    }
}

#[test]
fn invalid_utf8_names_are_escaped() {
    // The byte form is what the linker actually stores.
    let bytes = b"_OBJC_CLASS_$_F\xffoo";
    assert_eq!(
        parse_with(bytes).unwrap().to_string(),
        "Objective-C class F\\xffoo"
    );
    assert_eq!(
        parse_with(b"-[Foo b\xffar]").unwrap().to_string(),
        "-[Foo b\\xffar]"
    );
}

#[test]
fn valid_utf8_names_stay_verbatim() {
    assert_eq!(
        parse_with("_OBJC_CLASS_$_Ünicode".as_bytes())
            .unwrap()
            .to_string(),
        "Objective-C class Ünicode"
    );
    assert_eq!(
        parse_with("-[Foo bär]".as_bytes()).unwrap().to_string(),
        "-[Foo bär]"
    );
}

#[test]
fn budgets_are_enforced_on_bytes() {
    let mut symbol = b"_OBJC_CLASS_$_".to_vec();
    symbol.resize(MAX_INPUT_LEN + 1, b'a');
    assert!(matches!(parse_with(&symbol).unwrap_err(), e if e.budget_kind().is_some()));
    assert!(
        matches!(parse_with(b"-[Foo bar[[]]]").unwrap_err(), e if e.budget_kind() == Some(BudgetKind::ParseDepth))
    );
}

#[test]
fn truncated_bytes_never_panic() {
    let symbol = b"__OBJC_$_CATEGORY_Foo_$_\xffBar";
    for end in 0..=symbol.len() {
        let _ = parse_with(&symbol[..end]);
    }
}
