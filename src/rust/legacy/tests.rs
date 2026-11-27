use alloc::borrow::Cow;
use alloc::format;
use alloc::string::ToString;
use alloc::vec::Vec;

use super::*;
use crate::rust::{Encoding, Kind, NodeData, parse};

/// Assert both renderings, and cross-check them with the reference decoder.
fn check(symbol: &str, display: &str, alternate: &str) {
    let ast = parse(symbol).unwrap();
    assert_eq!(ast.encoding(), Encoding::Legacy);
    assert_eq!(ast.to_string(), display, "{symbol}");
    assert_eq!(format!("{ast:#}"), alternate, "{symbol}");
    check_oracle(symbol, display, alternate);
}

/// Compare with `rustc-demangle`, the executable oracle.
fn check_oracle(symbol: &str, display: &str, alternate: &str) {
    let oracle = rustc_demangle::try_demangle(symbol).unwrap();
    assert_eq!(format!("{oracle}"), display, "{symbol}");
    assert_eq!(format!("{oracle:#}"), alternate, "{symbol}");
}

/// Demangle raw symbol bytes, which the public `parse` cannot express.
fn check_bytes(symbol: &[u8], display: &str) {
    let ast = crate::rust::parse_bytes(symbol).unwrap();
    assert_eq!(ast.to_string(), display, "{symbol:?}");
}

#[test]
fn plain_paths() {
    check("_ZN4testE", "test", "test");
    check("_ZN3foo3barE", "foo::bar", "foo::bar");
    check("ZN3fooE", "foo", "foo");
    check("__ZN3fooE", "foo", "foo");
    // An empty path and an empty element both print nothing.
    check("_ZNE", "", "");
    check("_ZN0E", "", "");
}

#[test]
fn hash_is_omitted_in_alternate_mode() {
    check(
        "_ZN3foo17h05af221e174051e9E",
        "foo::h05af221e174051e9",
        "foo",
    );
    // Only a trailing `h<hex>` element is a hash, so `h05` counts too.
    check("_ZN3foo3h05E", "foo::h05", "foo");
    // A non-hex tail is an ordinary element.
    check("_ZN3foo4h05zE", "foo::h05z", "foo::h05z");
}

#[test]
fn escapes_are_decoded() {
    check("_ZN13_$LT$test$GT$E", "<test>", "<test>");
    check("_ZN8foo..barE", "foo::bar", "foo::bar");
    check("_ZN7foo.barE", "foo.bar", "foo.bar");
    check("_ZN12test$SP$testE", "test@test", "test@test");
    check("_ZN12test$BP$testE", "test*test", "test*test");
    check("_ZN12test$RF$testE", "test&test", "test&test");
    check("_ZN8$LP$$RP$E", "()", "()");
    check("_ZN8a$C$b.cdE", "a,b.cd", "a,b.cd");
    check("_ZN9$u20$testE", " test", " test");
    check("_ZN10$u7b$$u7d$E", "{}", "{}");
    check(
        "_ZN28_$u7b$$u7b$closure$u7d$$u7d$E",
        "{{closure}}",
        "{{closure}}",
    );
}

#[test]
fn unknown_escapes_stop_decoding() {
    // An unknown escape ends decoding and the rest is kept verbatim.
    check("_ZN7a$zz$bcE", "a$zz$bc", "a$zz$bc");
    // Control characters are not decoded.
    check("_ZN7a$u0$bcE", "a$u0$bc", "a$u0$bc");
    // Uppercase hex is not an escape.
    check("_ZN7a$uFF$bE", "a$uFF$b", "a$uFF$b");
    // An unterminated escape is kept verbatim.
    check("_ZN4a$bcE", "a$bc", "a$bc");
}

#[test]
fn elements_borrow_unless_rewritten() {
    let ast = parse("_ZN3fooE").unwrap();
    let element = ast.root().children().first().unwrap();
    assert_eq!(element.kind(), Kind::LegacyElement);
    assert_eq!(element.text(), Some(&b"foo"[..]));

    let ast = parse("_ZN8foo..barE").unwrap();
    let element = ast.root().children().first().unwrap();
    assert_eq!(element.text(), Some(&b"foo::bar"[..]));
}

#[test]
fn hash_flag_is_recorded() {
    let ast = parse("_ZN3foo17h05af221e174051e9E").unwrap();
    let children = ast.root().children();
    assert_eq!(children.len(), 2);
    assert_eq!(children[0].data(), &NodeData::Hash(false));
    assert_eq!(children[1].data(), &NodeData::Hash(true));
}

#[test]
fn spans_cover_the_element_bytes() {
    let symbol = "_ZN3foo3barE";
    let ast = parse(symbol).unwrap();
    let first = ast.root().children().first().unwrap();
    let span = first.span().unwrap();
    assert_eq!(
        symbol.get(span.start as usize..span.end as usize),
        Some("foo")
    );
}

#[test]
fn malformed_symbols_are_rejected() {
    for symbol in [
        "_ZN",
        "_ZN3foo",
        "_ZNfooE",
        "_ZN3fooEv",
        "_ZN99999999999999999999fooE",
    ] {
        assert_eq!(parse(symbol).unwrap_err(), ParseError::Invalid, "{symbol}");
    }
}

/// A length is a byte count, so an element may hold bytes that are not
/// UTF-8; the printer escapes them instead of the parser rejecting them.
#[test]
fn invalid_utf8_elements_are_escaped() {
    check_bytes(b"_ZN4F\xffoo3barE", "F\\xffoo::bar");
    // A length that cuts a UTF-8 sequence in half is legal at this layer.
    check_bytes(b"_ZN1\xc3E", "\\xc3");
    // Valid UTF-8 keeps printing verbatim.
    check_bytes("_ZN5féstE".as_bytes(), "fést");
}

/// Truncating a valid symbol never panics and never escapes the budgets.
#[test]
fn truncations_are_rejected() {
    let symbol = b"_ZN4test4main17h1234567890abcdefE";
    for end in 0..symbol.len() {
        let _ = crate::rust::parse_bytes(&symbol[..end]);
    }
    // Dropping the terminator leaves the path unterminated.
    assert_eq!(
        crate::rust::parse_bytes(&symbol[..symbol.len() - 1]).unwrap_err(),
        ParseError::Invalid
    );
}

/// A length that runs past the end of the symbol is rejected at the
/// length prefix, not accepted as a short element.
#[test]
fn length_prefixes_must_fit() {
    for symbol in [&b"_ZN9fooE"[..], b"_ZN4fooE", b"_ZN2E"] {
        assert_eq!(
            crate::rust::parse_bytes(symbol).unwrap_err(),
            ParseError::Invalid,
            "{symbol:?}"
        );
    }
    // The same bytes with a matching length parse.
    assert!(crate::rust::parse_bytes(b"_ZN3fooE").is_ok());
}

#[test]
fn unescape_returns_borrowed_when_unchanged() {
    assert!(matches!(unescape(b"plain"), Cow::Borrowed(_)));
    assert!(matches!(unescape(b"a.b"), Cow::Borrowed(_)));
    assert!(matches!(unescape(b"a..b"), Cow::Owned(_)));
    assert!(matches!(unescape(b"$LT$"), Cow::Owned(_)));
    assert!(matches!(unescape(b"_$LT$"), Cow::Owned(_)));
}

/// Escapes decode to bytes, and non-UTF-8 bytes pass through decoding.
#[test]
fn unescape_keeps_raw_bytes() {
    assert_eq!(unescape(b"a\xffb").as_ref(), &b"a\xffb"[..]);
    assert_eq!(unescape(b"a\xff..b").as_ref(), &b"a\xff::b"[..]);
    assert_eq!(unescape(b"$u7b$\xff").as_ref(), &b"{\xff"[..]);
    assert_eq!(unescape(b"a$zz$\xff").as_ref(), &b"a$zz$\xff"[..]);
    let decoded: Vec<u8> = unescape(b"$u7b$$u7d$").into_owned();
    assert_eq!(decoded, b"{}");
}
