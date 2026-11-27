use alloc::string::{String, ToString};
use alloc::vec::Vec;

use super::*;

#[test]
fn parses_short_and_long_names() {
    for (symbol, expected) in [
        ("Java_com_example_Foo_plain", "com.example.Foo.plain"),
        ("Java_Foo_bar", "Foo.bar"),
        (
            "Java_com_example_Foo_bar___3Ljava_lang_String_2I",
            "com.example.Foo.bar(java.lang.String[], int)",
        ),
        ("Java_com_example_Foo_bar__", "com.example.Foo.bar()"),
        (
            "Java_com_example_Foo_under_1score__",
            "com.example.Foo.under_score()",
        ),
        ("Java_com_example_Weird_11_m__", "com.example.Weird_1.m()"),
    ] {
        assert_eq!(parse(symbol).unwrap().to_string(), expected, "{symbol}");
    }
}

#[test]
fn decodes_every_escape() {
    for (symbol, expected) in [
        // `_1` is a literal underscore, in the class and in the method.
        ("Java_a_1b_c_1d", "a_b.c_d"),
        // `_0xxxx` decodes to one UTF-16 code unit.
        ("Java_p_T_baz_000e9", "p.T.bazé"),
        // Uppercase hexadecimal digits are the same escape.
        ("Java_p_T_baz_0004A", "p.T.bazJ"),
        // A surrogate pair decodes to one supplementary character.
        ("Java_p_T_baz_0d835_0dd38", "p.T.baz𝔸"),
        // `_2` terminates an object type and `_3` is one array dimension.
        ("Java_p_T_baz__Lq_1w_x_2_3_3I", "p.T.baz(q_w.x, int[][])"),
        // A bare `_` in a signature is the binary-name separator.
        (
            "Java_p_T_baz__Ljava_lang_String_2",
            "p.T.baz(java.lang.String)",
        ),
    ] {
        assert_eq!(parse(symbol).unwrap().to_string(), expected, "{symbol}");
    }
}

#[test]
fn rejects_malformed_names() {
    for symbol in [
        "",
        "Java_",
        "Java_foo",
        "Java__bar",
        "Java_foo_",
        "Java_foo_bar_0",
        "Java_foo_bar_0zzzz",
        "Java_foo_bar_0d835",
        "Java_foo_bar_0dd38",
        "Java_foo_bar_0d835_0000",
        "Java_foo_bar_0d835_z",
        "Java_foo_2bar_baz",
        "Java_foo_3bar_baz",
        "Java_foo_4",
        "Java_foo_bar__X",
        "Java_foo_bar__V",
        "Java_foo_bar__(I",
        "Java_foo_bar__L_2",
        "Java_foo_bar__Ljava",
        "Java_foo_bar__Ljava_lang_String",
        "Java_foo_bar__Ix",
        "Java_foo_bar__[I",
        "Java_foo_bar__(int)",
        "Java_com_example_Foo_plain()",
        "JNI_OnLoad",
        "JNI_CreateJavaVM",
        "jni_onload",
        "_Java_com_example_Foo_plain",
    ] {
        assert_eq!(
            parse(symbol).map(|_| ()),
            Err(ParseError::Invalid),
            "{symbol}"
        );
        assert!(!is_jni(symbol.as_bytes()), "{symbol}");
    }
}

#[test]
fn exposes_the_parsed_structure() {
    let ast = parse("Java_com_example_Foo_bar___3Ljava_lang_String_2I").unwrap();
    assert_eq!(ast.class(), "com.example.Foo");
    assert_eq!(ast.method(), "bar");
    let descriptor = ast.descriptor().unwrap();
    assert_eq!(descriptor.args().len(), 2);

    let array = &descriptor.args()[0];
    assert_eq!(array.dimensions(), 1);
    assert_eq!(array.element(), &Element::Object("java.lang.String".into()));
    assert_eq!(array.to_string(), "java.lang.String[]");

    let int = &descriptor.args()[1];
    assert_eq!(int.dimensions(), 0);
    assert_eq!(int.element(), &Element::Primitive(Primitive::Int));
    assert_eq!(int.to_string(), "int");
    assert_eq!(Primitive::Int.java_name(), "int");
    assert_eq!(Primitive::Boolean.java_name(), "boolean");

    // A short name carries no descriptor.
    assert!(
        parse("Java_com_example_Foo_plain")
            .unwrap()
            .descriptor()
            .is_none()
    );
}

#[test]
fn budgets_are_enforced() {
    // An array nested deeper than MAX_DEPTH.
    let mut deep = String::from("Java_p_T_baz__");
    for _ in 0..=MAX_DEPTH {
        deep.push_str("_3");
    }
    deep.push('I');
    assert_eq!(parse(&deep).map(|_| ()), Err(ParseError::RecursedTooDeep));

    // One dimension less parses.
    let mut shallow = String::from("Java_p_T_baz__");
    for _ in 0..MAX_DEPTH {
        shallow.push_str("_3");
    }
    shallow.push('I');
    assert!(parse(&shallow).is_ok());

    // An input longer than MAX_INPUT_LEN.
    let mut long = String::from("Java_p_T_baz__");
    while long.len() <= MAX_INPUT_LEN {
        long.push('I');
    }
    assert_eq!(parse(&long).map(|_| ()), Err(ParseError::BudgetExceeded));
    assert!(!is_jni(long.as_bytes()));
}

#[test]
fn output_stays_within_the_budget() {
    // The widest expansion is a boolean argument (`Z` -> `boolean, `).
    let mut symbol = String::from("Java_p_T_baz__");
    while symbol.len() < MAX_INPUT_LEN {
        symbol.push('Z');
    }
    let printed = parse(&symbol).unwrap().to_string();
    assert!(printed.len() <= 16 * symbol.len());
    assert!(printed.starts_with("p.T.baz(boolean, boolean"));
}

#[test]
fn never_panics_on_truncated_names() {
    for name in [
        "Java_com_example_Foo_bar___3Ljava_lang_String_2I",
        "Java_com_example_Foo_under_1score__",
        "Java_p_T_baz_0d835_0dd38",
        "Java_p_T_baz__Lq_1w_x_2_3_3I",
        "Java_p__0d835_0dd38_C_m",
    ] {
        for end in 0..=name.len() {
            if let Some(prefix) = name.get(..end) {
                let _ = parse(prefix);
                let _ = is_jni(prefix.as_bytes());
            }
        }
    }
}

#[test]
fn never_panics_on_arbitrary_bytes() {
    const CHARSET: &[u8] = b"Jav_pTboZIBLV0123456789_()/;[]\xC3\x9F\xFF";
    let mut state = 0x1234_5678_u64;
    let mut next = || {
        state = state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        (state >> 33) as usize
    };
    for _ in 0..20_000 {
        let length = next() % 40;
        let mut bytes = Vec::with_capacity(length);
        for _ in 0..length {
            bytes.push(CHARSET[next() % CHARSET.len()]);
        }
        let _ = parse_bytes(&bytes);
        let _ = is_jni(&bytes);
    }
}

#[test]
fn errors_are_displayed() {
    assert_eq!(ParseError::Invalid.to_string(), "invalid syntax");
    assert_eq!(
        ParseError::RecursedTooDeep.to_string(),
        "recursion limit reached"
    );
    assert_eq!(ParseError::BudgetExceeded.to_string(), "size limit reached");
}
