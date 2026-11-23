//! Explicit encoding decoding of reversible identifier escapes.
//!
//! The demangler keeps invalid identifier bytes as `\xNN` and literal
//! backslashes as `\\`. These tests pin the opt-in decoding layer for UTF-8,
//! Latin-1 and Windows-1252, including malformed and unrepresentable input.

use demangle::{Demangler, Encoding, EncodingError};

#[test]
fn latin1_decodes_escaped_bytes() {
    assert_eq!(
        Encoding::Latin1
            .decode_identifier_escapes("F\\xe9\\xffoo")
            .unwrap(),
        "Féÿoo"
    );
}

#[test]
fn cp1252_uses_the_windows_table() {
    assert_eq!(
        Encoding::Cp1252
            .decode_identifier_escapes("\\x80\\x92\\xe9")
            .unwrap(),
        "€’é"
    );
    // The same bytes are C1 controls in Latin-1.
    assert_eq!(
        Encoding::Latin1
            .decode_identifier_escapes("\\x80\\x92")
            .unwrap(),
        "\u{80}\u{92}"
    );
}

#[test]
fn utf8_decodes_valid_escaped_sequences() {
    assert_eq!(
        Encoding::Utf8
            .decode_identifier_escapes("F\\xc3\\xa9o")
            .unwrap(),
        "Féo"
    );
}

#[test]
fn literal_backslashes_are_preserved() {
    assert_eq!(
        Encoding::Latin1
            .decode_identifier_escapes("a\\\\x41b")
            .unwrap(),
        "a\\x41b"
    );
}

#[test]
fn invalid_escapes_are_rejected() {
    for text in ["\\", "\\q", "\\x", "\\x4", "\\xGG", "\\X41"] {
        assert!(
            matches!(
                Encoding::Latin1.decode_identifier_escapes(text),
                Err(EncodingError::InvalidEscape)
            ),
            "{text:?}"
        );
    }
}

#[test]
fn bytes_invalid_for_the_encoding_are_rejected() {
    assert!(matches!(
        Encoding::Utf8.decode_identifier_escapes("\\xff"),
        Err(EncodingError::InvalidEncoding)
    ));
    for text in ["\\x81", "\\x8d", "\\x8f", "\\x90", "\\x9d"] {
        assert!(
            matches!(
                Encoding::Cp1252.decode_identifier_escapes(text),
                Err(EncodingError::InvalidEncoding)
            ),
            "{text}"
        );
    }
}

#[test]
fn plain_text_is_not_decoded() {
    // A plain C/unknown name is already Unicode and is not escaped, so a
    // literal `\xNN` sequence must survive unchanged.
    let text = Demangler::new()
        .demangle_to_string_with_encoding(b"a\\x41b", Encoding::Latin1)
        .unwrap();
    assert_eq!(text, "a\\x41b");
}

#[test]
fn unknown_invalid_bytes_decode_after_demangling() {
    let text = Demangler::new()
        .demangle_to_string_with_encoding(b"plain\xff", Encoding::Latin1)
        .unwrap();
    assert_eq!(text, "plainÿ");
}

#[cfg(feature = "cpp-itanium")]
#[test]
fn cpp_identifier_escapes_decode_with_the_selected_encoding() {
    let demangler = Demangler::new();
    assert_eq!(
        demangler
            .demangle_to_string_with_encoding(b"_ZN4F\xe9ooE", Encoding::Latin1)
            .unwrap(),
        "Féoo"
    );
    assert_eq!(
        demangler
            .demangle_to_string_with_encoding(b"_ZN3F\x80oE", Encoding::Cp1252)
            .unwrap(),
        "F€o"
    );
    assert!(matches!(
        demangler.demangle_to_string_with_encoding(b"_ZN4F\xe9ooE", Encoding::Utf8),
        Err(EncodingError::InvalidEncoding)
    ));
}

#[cfg(feature = "ada")]
#[test]
fn forced_scheme_combines_with_encoding_decoding() {
    let demangled = Demangler::new()
        .demangle_as(demangle::Scheme::Ada, b"pkg__subp")
        .expect("forced Ada name");
    assert_eq!(
        demangled.to_string_with_encoding(Encoding::Utf8).unwrap(),
        "pkg.subp"
    );
}

#[cfg(feature = "ada")]
#[test]
fn demangle_errors_are_propagated() {
    assert!(matches!(
        Demangler::new().demangle_to_string_with_encoding(b"ada__Foo", Encoding::Latin1),
        Err(EncodingError::Demangle(demangle::Error::InvalidSymbol(
            demangle::Scheme::Ada
        )))
    ));
}
