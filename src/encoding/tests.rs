use super::*;

#[test]
fn decodes_latin1_and_cp1252() {
    assert_eq!(
        Encoding::Latin1
            .decode_identifier_escapes("F\\xe9\\xffoo")
            .unwrap(),
        "Féÿoo"
    );
    assert_eq!(
        Encoding::Windows1252
            .decode_identifier_escapes("\\x80\\x92\\xe9")
            .unwrap(),
        "€’é"
    );
    assert_eq!(
        Encoding::Latin1
            .decode_identifier_escapes("\\x80\\x92")
            .unwrap(),
        "\u{80}\u{92}"
    );
}

#[test]
fn decodes_utf8_runs() {
    assert_eq!(
        Encoding::Utf8
            .decode_identifier_escapes("F\\xc3\\xa9o")
            .unwrap(),
        "Féo"
    );
    assert_eq!(
        Encoding::Utf8
            .decode_identifier_escapes("\\xe9")
            .unwrap_err(),
        EncodingError::InvalidEncoding
    );
}

#[test]
fn preserves_literal_backslashes() {
    assert_eq!(
        Encoding::Latin1
            .decode_identifier_escapes("a\\\\x41b")
            .unwrap(),
        "a\\x41b"
    );
}

#[test]
fn rejects_malformed_escapes() {
    for text in ["\\", "\\q", "\\x", "\\x4", "\\xGG", "\\X41"] {
        assert_eq!(
            Encoding::Latin1
                .decode_identifier_escapes(text)
                .unwrap_err(),
            EncodingError::InvalidEscape,
            "{text:?}"
        );
    }
}

#[test]
fn rejects_undefined_cp1252_bytes() {
    for text in ["\\x81", "\\x8d", "\\x8f", "\\x90", "\\x9d"] {
        assert_eq!(
            Encoding::Windows1252
                .decode_identifier_escapes(text)
                .unwrap_err(),
            EncodingError::InvalidEncoding,
            "{text}"
        );
    }
}

#[test]
fn decodes_mac_roman() {
    // ASCII is unchanged, and the high half uses Apple's Roman table.
    assert_eq!(
        Encoding::MacRoman
            .decode_identifier_escapes("F\\x8e\\xa5o")
            .unwrap(),
        "Fé•o"
    );
    assert_eq!(
        Encoding::MacRoman
            .decode_identifier_escapes("\\x80\\xd5\\xf0")
            .unwrap(),
        "Ä’\u{f8ff}"
    );
    // The same bytes are different characters in Latin-1 and CP1252.
    assert_eq!(
        Encoding::Latin1
            .decode_identifier_escapes("\\x8e\\xa5")
            .unwrap(),
        "\u{8e}¥"
    );
}

#[test]
fn decodes_ebcdic_037() {
    assert_eq!(
        Encoding::Ebcdic037
            .decode_identifier_escapes("\\x81\\x82\\x83")
            .unwrap(),
        "abc"
    );
    assert_eq!(
        Encoding::Ebcdic037
            .decode_identifier_escapes("\\xc1\\xf0\\x40\\x4b")
            .unwrap(),
        "A0 ."
    );
    // An EBCDIC symbol name is byte-reversed ASCII: `main` is
    // `\x94\x81\x89\x95`.
    assert_eq!(
        Encoding::Ebcdic037
            .decode_identifier_escapes("\\x94\\x81\\x89\\x95")
            .unwrap(),
        "main"
    );
}

#[test]
fn cp1252_decodes_every_defined_byte() {
    use alloc::string::ToString;
    // Explicit Windows-1252 entries, independent of the implementation
    // helper. The five undefined bytes are checked separately.
    for (byte, expected) in [
        (0x80, '€'),
        (0x82, '‚'),
        (0x83, 'ƒ'),
        (0x84, '„'),
        (0x85, '…'),
        (0x86, '†'),
        (0x87, '‡'),
        (0x88, 'ˆ'),
        (0x89, '‰'),
        (0x8a, 'Š'),
        (0x8b, '‹'),
        (0x8c, 'Œ'),
        (0x8e, 'Ž'),
        (0x91, '‘'),
        (0x92, '’'),
        (0x93, '“'),
        (0x94, '”'),
        (0x95, '•'),
        (0x96, '–'),
        (0x97, '—'),
        (0x98, '˜'),
        (0x99, '™'),
        (0x9a, 'š'),
        (0x9b, '›'),
        (0x9c, 'œ'),
        (0x9e, 'ž'),
        (0x9f, 'Ÿ'),
    ] {
        let escape = alloc::format!("\\x{byte:02x}");
        assert_eq!(
            Encoding::Windows1252
                .decode_identifier_escapes(&escape)
                .unwrap(),
            expected.to_string(),
            "{byte:#04x}"
        );
    }
    for byte in 0x00..=0x7f_u8 {
        let escape = alloc::format!("\\x{byte:02x}");
        assert_eq!(
            Encoding::Windows1252
                .decode_identifier_escapes(&escape)
                .unwrap(),
            char::from(byte).to_string(),
            "{byte:#04x}"
        );
    }
    for byte in 0xa0..=0xff_u8 {
        let escape = alloc::format!("\\x{byte:02x}");
        assert_eq!(
            Encoding::Windows1252
                .decode_identifier_escapes(&escape)
                .unwrap(),
            char::from(byte).to_string(),
            "{byte:#04x}"
        );
    }
    for byte in [0x81, 0x8d, 0x8f, 0x90, 0x9d] {
        let escape = alloc::format!("\\x{byte:02x}");
        assert_eq!(
            Encoding::Windows1252
                .decode_identifier_escapes(&escape)
                .unwrap_err(),
            EncodingError::InvalidEncoding,
            "{byte:#04x}"
        );
    }
}

#[test]
fn uppercase_hex_digits_are_accepted() {
    assert_eq!(
        Encoding::Latin1.decode_identifier_escapes("\\xAF").unwrap(),
        "¯"
    );
}

#[test]
fn encoding_errors_display_and_chain() {
    use alloc::string::ToString;
    use core::error::Error as _;

    let demangle = EncodingError::Demangle(Error::InvalidSymbol(crate::Scheme::CppItanium));
    assert_eq!(demangle.to_string(), "invalid C++ symbol");
    assert!(demangle.source().is_some());
    assert_eq!(
        EncodingError::InvalidEscape.to_string(),
        "invalid \\xNN escape"
    );
    assert!(EncodingError::InvalidEscape.source().is_none());
    assert_eq!(
        EncodingError::InvalidEncoding.to_string(),
        "escaped bytes are not valid in the selected encoding"
    );
    assert!(EncodingError::InvalidEncoding.source().is_none());
}

#[test]
fn tables_are_total_and_injective() {
    let mut mac_roman_table = Vec::new();
    for byte in 0..=u8::MAX {
        mac_roman_table.push(mac_roman(byte));
    }
    let mut ebcdic = Vec::new();
    for byte in 0..=u8::MAX {
        ebcdic.push(EBCDIC_037[usize::from(byte)]);
    }
    for table in [&mac_roman_table, &ebcdic] {
        assert_eq!(table.len(), 256);
        for (index, character) in table.iter().enumerate() {
            assert!(
                !(0xd800..=0xdfff).contains(&u32::from(*character)),
                "surrogate at {index:#04x}"
            );
        }
        let unique: Vec<char> = {
            let mut sorted = table.clone();
            sorted.sort_unstable();
            sorted.dedup();
            sorted
        };
        assert_eq!(unique.len(), table.len(), "table is not injective");
    }
}
