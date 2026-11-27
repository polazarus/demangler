use alloc::format;
use alloc::string::ToString;
use alloc::vec;
use alloc::vec::Vec;

use super::*;
use crate::rust::{DEFAULT_BUDGET, parse_bytes_with_budget};

#[test]
fn alternate_mode_changes_three_things() {
    // Crate-root hash.
    assert_eq!(
        parse_bytes_with_budget(b"_RCs_3foo", DEFAULT_BUDGET)
            .unwrap()
            .to_string(),
        "foo[1]"
    );
    assert_eq!(
        format!(
            "{:#}",
            parse_bytes_with_budget(b"_RCs_3foo", DEFAULT_BUDGET).unwrap()
        ),
        "foo"
    );
    // Legacy trailing hash.
    let legacy = parse_bytes_with_budget(b"_ZN3foo17h05af221e174051e9E", DEFAULT_BUDGET).unwrap();
    assert_eq!(legacy.to_string(), "foo::h05af221e174051e9");
    assert_eq!(format!("{legacy:#}"), "foo");
    // Integer const type suffix.
    let konst = parse_bytes_with_budget(b"_RIC0Kanb_E", DEFAULT_BUDGET).unwrap();
    assert_eq!(konst.to_string(), "::<-11i8>");
    assert_eq!(format!("{konst:#}"), "::<-11>");
}

#[test]
fn suffix_is_appended_in_both_modes() {
    let ast = parse_bytes_with_budget(
        b"_RNvNtC4rand4fork23FORK_HANDLER_REGISTERED.0.0",
        DEFAULT_BUDGET,
    )
    .unwrap();
    assert_eq!(ast.suffix(), ".0.0");
    assert_eq!(ast.to_string(), "rand::fork::FORK_HANDLER_REGISTERED.0.0");
    assert_eq!(
        format!("{ast:#}"),
        "rand::fork::FORK_HANDLER_REGISTERED.0.0"
    );
}

#[test]
fn num_buf_renders_decimal_and_hex() {
    assert_eq!(NumBuf::decimal(0).as_str(), "0");
    assert_eq!(NumBuf::decimal(1235).as_str(), "1235");
    assert_eq!(NumBuf::decimal(u64::MAX).as_str(), "18446744073709551615");
    assert_eq!(NumBuf::hex(0).as_str(), "0");
    assert_eq!(NumBuf::hex(0x4d3).as_str(), "4d3");
    assert_eq!(NumBuf::hex(u64::MAX).as_str(), "ffffffffffffffff");
}

#[test]
fn hex_digits_are_lowercase_only() {
    assert_eq!(hex_digit(b'0'), Some(0));
    assert_eq!(hex_digit(b'f'), Some(15));
    assert_eq!(hex_digit(b'A'), None);
    assert_eq!(hex_digit(b'g'), None);
}

/// Identifier text is escaped reversibly: `\` doubles, a non-UTF-8 byte
/// becomes `\xNN` and ASCII is unchanged.
#[test]
fn identifier_text_is_escaped_reversibly() {
    let ast = parse_bytes_with_budget(b"_ZN5a\\b\xffcE", DEFAULT_BUDGET).unwrap();
    assert_eq!(ast.to_string(), "a\\\\b\\xffc");
    // The three spellings that must stay distinguishable: an escaped
    // backslash, an escaped byte and the raw ASCII digit `A`.
    let ast = parse_bytes_with_budget(b"_ZN3A\\AE", DEFAULT_BUDGET).unwrap();
    assert_eq!(ast.to_string(), "A\\\\A");
    assert_ne!(ast.to_string(), "A\\x41A");
}

/// Escaping multiplies the output by at most four, so a symbol inside the
/// input budget always stays inside the print budget.
#[test]
fn escaped_output_stays_within_the_print_budget() {
    let bytes = vec![0xff_u8; 60 * 1024];
    let mut symbol = Vec::from(&b"_ZN"[..]);
    symbol.extend_from_slice(bytes.len().to_string().as_bytes());
    symbol.extend_from_slice(&bytes);
    symbol.push(b'E');
    let ast =
        parse_bytes_with_budget(&symbol, DEFAULT_BUDGET).expect("a long escaped name is valid");
    let rendered = ast.to_string();
    assert_eq!(rendered.len(), 4 * bytes.len());
}
