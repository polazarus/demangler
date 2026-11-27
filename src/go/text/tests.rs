use alloc::string::ToString;

use super::*;

#[test]
fn decodes_path_escapes() {
    assert_eq!(decoded(b"pkg%2ev1"), "pkg.v1");
    assert_eq!(decoded(b"a%2fb"), "a/b");
    assert_eq!(decoded(b"%2e%2e"), "..");
    assert_eq!(decoded(b"%25"), "%");
}

#[test]
fn keeps_malformed_escapes() {
    assert_eq!(decoded(b"100%"), "100%");
    assert_eq!(decoded(b"%zz"), "%zz");
    assert_eq!(decoded(b"%2"), "%2");
    assert_eq!(decoded(b"a%2"), "a%2");
}

#[test]
fn borrows_when_unchanged() {
    assert!(matches!(
        text(b"fmt.Println", Escape::Identifier),
        Cow::Borrowed(_)
    ));
    assert!(matches!(unescape(b"%2e"), Cow::Owned(_)));
}

#[test]
fn invalid_bytes_are_escaped_not_dropped() {
    assert_eq!(decoded(b"pkg%ff.Foo"), "pkg\\xff.Foo");
    assert_eq!(text(b"a\xffb", Escape::Identifier), "a\\xffb");
    assert_eq!(text(b"a\\b", Escape::Identifier), "a\\\\b");
    assert_eq!(text(b"a\\b", Escape::Verbatim), "a\\b");
    assert_eq!(decoded(b"a\xffb").to_string(), "a\\xffb");
}
