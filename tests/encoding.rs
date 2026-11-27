//! Encoding-layer tests (compiled only with the `encoding` feature).

#![cfg(feature = "encoding")]

//! Explicit encoding decoding of reversible identifier escapes.
//!
//! The demangler keeps invalid identifier bytes as `\xNN` and literal
//! backslashes as `\\`. These tests pin the opt-in decoding layer for UTF-8,
//! Latin-1, Windows-1252, MacRoman and EBCDIC code page 037, including
//! malformed and unrepresentable input.

use std::fmt::Write as _;

use demangler::{Demangler, Encoding, EncodingError};

/// Demangle `symbol` and decode its identifier escapes with `encoding`.
fn decode(symbol: &[u8], encoding: Encoding) -> Result<String, EncodingError> {
    Demangler::new()
        .demangle_bytes(symbol)?
        .to_string_with_encoding(encoding)
}

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
        Encoding::Windows1252
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

#[cfg(feature = "ocaml")]
#[test]
fn ocaml_backslash_escape_decodes_reversibly() {
    let text =
        decode(b"camlProbe.a$5c$78$34$31_1", Encoding::Latin1).expect("OCaml escape should decode");
    assert_eq!(text, "Probe.a\\x41");
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
                Encoding::Windows1252.decode_identifier_escapes(text),
                Err(EncodingError::InvalidEncoding)
            ),
            "{text}"
        );
    }
}

#[test]
fn mac_roman_uses_the_apple_table() {
    assert_eq!(
        Encoding::MacRoman
            .decode_identifier_escapes("F\\x8e\\xa5oo")
            .unwrap(),
        "Fé•oo"
    );
    // `0xF0` is the Apple logo, a private-use code point.
    assert_eq!(
        Encoding::MacRoman
            .decode_identifier_escapes("\\xf0")
            .unwrap(),
        "\u{f8ff}"
    );
    // MacRoman is not Latin-1 above 0x7F.
    assert_eq!(
        Encoding::Latin1
            .decode_identifier_escapes("\\x8e\\xa5")
            .unwrap(),
        "\u{8e}¥"
    );
}

#[test]
fn ebcdic_037_uses_the_us_canada_page() {
    // An EBCDIC symbol table stores the same characters as EBCDIC bytes.
    assert_eq!(
        Encoding::Ebcdic037
            .decode_identifier_escapes("\\x94\\x81\\x89\\x95")
            .unwrap(),
        "main"
    );
    assert_eq!(
        Encoding::Ebcdic037
            .decode_identifier_escapes("\\xc1\\xf0\\x40\\x4b")
            .unwrap(),
        "A0 ."
    );
    // Every byte is defined, so no escape is rejected for this encoding.
    let all = (0..=u8::MAX).fold(String::new(), |mut all, byte| {
        let _ = write!(all, "\\x{byte:02x}");
        all
    });
    let decoded = Encoding::Ebcdic037
        .decode_identifier_escapes(&all)
        .expect("every EBCDIC 037 byte is defined");
    assert_eq!(decoded.chars().count(), 256);
}

#[cfg(feature = "cpp-itanium")]
#[test]
fn mac_roman_and_ebcdic_decode_demangled_identifiers() {
    // `F\x8eoo` is `Féoo` in MacRoman and `F\u{8e}oo` in Latin-1.
    assert_eq!(decode(b"_ZN4F\x8eooE", Encoding::MacRoman).unwrap(), "Féoo");
    // `\x94\x81\x89\x95` is `main` in EBCDIC 037.
    assert_eq!(
        decode(b"_ZN4\x94\x81\x89\x95E", Encoding::Ebcdic037).unwrap(),
        "main"
    );
}

#[test]
fn plain_text_is_not_decoded() {
    // A plain C/unknown name is already Unicode and is not escaped, so a
    // literal `\xNN` sequence must survive unchanged.
    let text = decode(b"a\\x41b", Encoding::Latin1).unwrap();
    assert_eq!(text, "a\\x41b");
}

#[test]
fn unknown_invalid_bytes_decode_after_demangling() {
    let text = decode(b"plain\xff", Encoding::Latin1).unwrap();
    assert_eq!(text, "plainÿ");
}

#[cfg(feature = "cpp-itanium")]
#[test]
fn cpp_identifier_escapes_decode_with_the_selected_encoding() {
    assert_eq!(decode(b"_ZN4F\xe9ooE", Encoding::Latin1).unwrap(), "Féoo");
    assert_eq!(
        decode(b"_ZN3F\x80oE", Encoding::Windows1252).unwrap(),
        "F€o"
    );
    assert!(matches!(
        decode(b"_ZN4F\xe9ooE", Encoding::Utf8),
        Err(EncodingError::InvalidEncoding)
    ));
}

#[cfg(feature = "ada")]
#[test]
fn forced_scheme_combines_with_encoding_decoding() {
    let demangled = Demangler::new()
        .demangle_bytes_with_scheme(demangler::Scheme::Ada, b"pkg__subp")
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
        decode(b"ada__Foo", Encoding::Latin1),
        Err(EncodingError::Demangle(demangler::Error::InvalidSymbol(
            demangler::Scheme::Ada
        )))
    ));
}
