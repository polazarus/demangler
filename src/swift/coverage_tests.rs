use super::*;

#[test]
fn grammar_examples_parse() {
    for symbol in [
        "$s5MyKit5PointVMa",
        "$s5MyKit5PointVMaTwa",
        "$s5MyKit5PointVMn",
        "$s5MyKit5PointVMnTwa",
    ] {
        assert!(parse(symbol.as_bytes()).is_ok(), "{symbol}");
    }
}

#[test]
fn malformed_names_are_rejected() {
    for symbol in [
        "$s",
        "$s4",
        "$s4main999foo",
        "$s4main3",
        "$s4main3f",
        "$s4main3foo\x00",
    ] {
        assert!(
            parse(symbol.as_bytes()).is_err(),
            "{symbol:?} should be rejected"
        );
    }
}
