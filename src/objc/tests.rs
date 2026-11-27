use std::string::{String, ToString};
use std::vec::Vec;

use super::*;

fn text(symbol: &str) -> String {
    parse_bytes_with_budget(symbol.as_bytes(), DEFAULT_BUDGET)
        .unwrap_or_else(|error| panic!("{symbol}: {error}"))
        .to_string()
}

#[test]
fn prints_apple_metadata() {
    for (symbol, expected) in [
        ("_OBJC_CLASS_$_NSString", "Objective-C class NSString"),
        (
            "_OBJC_METACLASS_$_NSString",
            "Objective-C metaclass NSString",
        ),
        (
            "__OBJC_$_CATEGORY_NSString_$_Drawing",
            "Objective-C category NSString (Drawing)",
        ),
        (
            "__OBJC_$_INSTANCE_METHODS_Foo",
            "Objective-C instance methods Foo",
        ),
        (
            "__OBJC_$_CLASS_METHODS_Foo",
            "Objective-C class methods Foo",
        ),
        ("__OBJC_$_PROP_LIST_Foo", "Objective-C property list Foo"),
        (
            "__OBJC_$_INSTANCE_VARIABLES_Foo",
            "Objective-C instance variables Foo",
        ),
        (
            "__OBJC_PROTOCOL_$_NSCopying",
            "Objective-C protocol NSCopying",
        ),
        (
            "__OBJC_PROTOCOL_REFERENCE_$_NSCopying",
            "Objective-C protocol ref NSCopying",
        ),
        (
            "__OBJC_LABEL_PROTOCOL_$_NSCopying",
            "Objective-C protocol label NSCopying",
        ),
        (
            "__OBJC_$_PROTOCOL_INSTANCE_METHODS_NSCopying",
            "Objective-C protocol instance methods NSCopying",
        ),
        (
            "__OBJC_$_PROTOCOL_CLASS_METHODS_NSCopying",
            "Objective-C protocol class methods NSCopying",
        ),
        (
            "__OBJC_$_PROTOCOL_INSTANCE_METHODS_OPT_NSCopying",
            "Objective-C protocol optional instance methods NSCopying",
        ),
        (
            "__OBJC_$_PROTOCOL_CLASS_METHODS_OPT_NSCopying",
            "Objective-C protocol optional class methods NSCopying",
        ),
        (
            "__OBJC_$_PROTOCOL_PROPERTIES_NSCopying",
            "Objective-C protocol property list NSCopying",
        ),
        (
            "__OBJC_$_PROTOCOL_REFS_NSCopying",
            "Objective-C protocol refs NSCopying",
        ),
        (
            "__OBJC_$_PROTOCOL_METHOD_TYPES_NSCopying",
            "Objective-C protocol method types NSCopying",
        ),
        (
            "__OBJC_$_CATEGORY_INSTANCE_METHODS_Foo_$_Bar",
            "Objective-C category instance methods Foo (Bar)",
        ),
        (
            "__OBJC_$_CATEGORY_CLASS_METHODS_Foo_$_Bar",
            "Objective-C category class methods Foo (Bar)",
        ),
        (
            "__OBJC_$_CATEGORY_PROP_LIST_Foo_$_Bar",
            "Objective-C category property list Foo (Bar)",
        ),
        (
            "__OBJC_CATEGORY_PROTOCOLS_$_Foo_$_Bar",
            "Objective-C category protocols Foo (Bar)",
        ),
        (
            "__OBJC_CLASS_PROTOCOLS_$_Foo",
            "Objective-C class protocols Foo",
        ),
        ("__OBJC_CLASS_RO_$_Foo", "Objective-C class data Foo"),
        (
            "__OBJC_METACLASS_RO_$_Foo",
            "Objective-C metaclass data Foo",
        ),
        ("_OBJC_IVAR_$_Foo._ivar", "Objective-C ivar Foo._ivar"),
        ("_OBJC_METH_VAR_NAME_0", "Objective-C method name 0"),
        ("_OBJC_METH_VAR_TYPE_12", "Objective-C method type 12"),
    ] {
        assert_eq!(text(symbol), expected, "{symbol}");
    }
}

#[test]
fn prints_gnustep_metadata() {
    for (symbol, expected) in [
        ("__objc_class_name_Foo", "Objective-C class Foo"),
        ("__objc_class_ref_Foo", "Objective-C class ref Foo"),
        (
            "__objc_ivar_offset_Foo._ivar",
            "Objective-C ivar offset Foo._ivar",
        ),
        (
            "__objc_ivar_offset_value_Foo._ivar",
            "Objective-C ivar offset value Foo._ivar",
        ),
        ("_OBJC_CLASS_Foo", "Objective-C class Foo"),
        ("_OBJC_METACLASS_Foo", "Objective-C metaclass Foo"),
        ("_OBJC_REF_CLASS_Foo", "Objective-C class ref Foo"),
        ("_OBJC_INIT_CLASS_Foo", "Objective-C class init Foo"),
        ("_OBJC_PROTOCOL_P", "Objective-C protocol P"),
        ("_OBJC_ClassName_Object", "Objective-C class name Object"),
        ("_OBJC_Class_Object", "Objective-C class Object"),
        ("_OBJC_MetaClass_Object", "Objective-C metaclass Object"),
        (
            "_OBJC_InstanceMethods_Object",
            "Objective-C instance methods Object",
        ),
        (
            "_OBJC_ClassMethods_Object",
            "Objective-C class methods Object",
        ),
        (
            "_OBJC_InstanceIvars_Object",
            "Objective-C instance variables Object",
        ),
        (
            "_OBJC_ClassIvars_Object",
            "Objective-C class variables Object",
        ),
        ("_OBJC_Module", "Objective-C module"),
        ("_OBJC_SYMBOLS", "Objective-C symbols"),
        ("_OBJC_SELECTOR_TABLE", "Objective-C selector table"),
        (
            "_OBJC_SELECTOR_REFERENCES_",
            "Objective-C selector references",
        ),
    ] {
        assert_eq!(text(symbol), expected, "{symbol}");
    }
}

#[test]
fn prints_method_display_forms() {
    for symbol in [
        "-[NSObject init]",
        "+[Foo bar:baz:]",
        "-[Root(Cat) catMethod]",
        "-[Foo setX:]",
        "+[Foo _private]",
        "-[My.Module method]",
    ] {
        assert_eq!(text(symbol), symbol, "{symbol}");
    }
}

#[test]
fn prints_block_descriptors() {
    assert_eq!(
        text("__block_descriptor_40_8_32r_e5_v8?0l"),
        "Objective-C block descriptor 40_8_32r_e5_v8?0l"
    );
    assert_eq!(
        text("___block_descriptor_40_e8_32r_e5_v8?0l"),
        "Objective-C block descriptor 40_e8_32r_e5_v8?0l"
    );
    // Clang's real block signatures contain a `\x01` separator.
    assert_eq!(
        text("___block_descriptor_40_e8_32r_e5_v8\u{1}?0l"),
        "Objective-C block descriptor 40_e8_32r_e5_v8\u{1}?0l"
    );
}

#[test]
fn accepts_bare_and_single_underscore_spellings() {
    for (symbol, expected) in [
        ("_OBJC_CLASS_$_Foo", "Objective-C class Foo"),
        ("__OBJC_CLASS_$_Foo", "Objective-C class Foo"),
        ("OBJC_CLASS_$_Foo", "Objective-C class Foo"),
        (
            "_OBJC_$_INSTANCE_METHODS_Foo",
            "Objective-C instance methods Foo",
        ),
        (
            "__OBJC_$_INSTANCE_METHODS_Foo",
            "Objective-C instance methods Foo",
        ),
    ] {
        assert_eq!(text(symbol), expected, "{symbol}");
    }
}

#[test]
fn preserves_underscores_and_dollars() {
    assert_eq!(
        text("_OBJC_CLASS_$_My_Class$Sub"),
        "Objective-C class My_Class$Sub"
    );
    assert_eq!(
        text("__OBJC_$_CATEGORY_My_Class_$_My_Cat"),
        "Objective-C category My_Class (My_Cat)"
    );
    assert_eq!(
        text("__OBJC_$_PROP_LIST_My_Class_$_My_Cat"),
        "Objective-C property list My_Class (My_Cat)"
    );
}

#[test]
fn accepts_gnustep_abi2_decorations() {
    // Clang's GNUstep ABI 2 output decorates `_OBJC_` by object format:
    // ELF emits `.`, COFF emits `$`, and the i686 COFF C decoration gives
    // `_$_`. All three render like the undecorated record.
    for (symbol, expected) in [
        ("_OBJC_CLASS_Foo", "Objective-C class Foo"),
        ("._OBJC_CLASS_Foo", "Objective-C class Foo"),
        ("$_OBJC_CLASS_Foo", "Objective-C class Foo"),
        ("_$_OBJC_CLASS_Foo", "Objective-C class Foo"),
        ("._OBJC_METACLASS_Foo", "Objective-C metaclass Foo"),
        ("._OBJC_REF_CLASS_Foo", "Objective-C class ref Foo"),
        ("$_OBJC_REF_CLASS_Foo", "Objective-C class ref Foo"),
        ("_$_OBJC_REF_CLASS_Foo", "Objective-C class ref Foo"),
        ("._OBJC_INIT_CLASS_Foo", "Objective-C class init Foo"),
        ("._OBJC_PROTOCOL_P", "Objective-C protocol P"),
        ("$_OBJC_PROTOCOL_P", "Objective-C protocol P"),
        ("_$_OBJC_PROTOCOL_P", "Objective-C protocol P"),
    ] {
        assert_eq!(text(symbol), expected, "{symbol}");
        assert!(is_objc(symbol.as_bytes()), "{symbol}");
    }
    // The decoration only covers `_OBJC_`; local records, labels and the
    // Apple `__OBJC_` namespace are not decorated by any known target.
    for symbol in [
        ".objc_category_FooCat",
        ".L_OBJC_CLASS_Foo",
        "l_OBJC_LABEL_CLASS_$",
        ".__OBJC_$_INSTANCE_METHODS_Foo",
        "$_OBJC_NOPE_$_Foo",
    ] {
        assert!(
            parse_bytes_with_budget(symbol.as_bytes(), DEFAULT_BUDGET).is_err(),
            "{symbol}"
        );
        assert!(!is_objc(symbol.as_bytes()), "{symbol}");
    }
    // A decorated known prefix with a malformed body stays claimed, like
    // the undecorated spelling, so `parse` reports the failure.
    assert!(parse_bytes_with_budget(b"._OBJC_", DEFAULT_BUDGET).is_err());
    assert!(is_objc(b"._OBJC_"));
}

#[test]
fn exposes_structure() {
    let ast = parse_bytes_with_budget(b"_OBJC_CLASS_$_Foo", DEFAULT_BUDGET).unwrap();
    let metadata = ast.metadata().expect("metadata");
    assert_eq!(metadata.kind(), MetadataKind::Class);
    assert_eq!(metadata.subject().as_deref(), Some("Foo"));
    assert_eq!(metadata.category(), None);
    assert_eq!(metadata.member(), None);

    let ast = parse_bytes_with_budget(b"__OBJC_$_CATEGORY_Foo_$_Bar", DEFAULT_BUDGET).unwrap();
    let metadata = ast.metadata().expect("metadata");
    assert_eq!(metadata.kind(), MetadataKind::Category);
    assert_eq!(metadata.subject().as_deref(), Some("Foo"));
    assert_eq!(metadata.category().as_deref(), Some("Bar"));

    let ast = parse_bytes_with_budget(b"_OBJC_IVAR_$_Foo._ivar", DEFAULT_BUDGET).unwrap();
    let metadata = ast.metadata().expect("metadata");
    assert_eq!(metadata.kind(), MetadataKind::Ivar);
    assert_eq!(metadata.subject().as_deref(), Some("Foo"));
    assert_eq!(metadata.member().as_deref(), Some("_ivar"));

    let ast = parse_bytes_with_budget(b"-[Foo bar:]", DEFAULT_BUDGET).unwrap();
    let method = ast.method().expect("method");
    assert_eq!(method.kind(), MethodKind::Instance);
    assert_eq!(method.class(), "Foo");
    assert_eq!(method.category(), None);
    assert_eq!(method.selector(), "bar:");

    let ast =
        parse_bytes_with_budget(b"__block_descriptor_40_8_32r_e5_v8?0l", DEFAULT_BUDGET).unwrap();
    assert_eq!(
        ast.block_descriptor().expect("descriptor").payload(),
        "40_8_32r_e5_v8?0l"
    );
}

#[test]
fn rejects_malformed_names() {
    for symbol in [
        "",
        "-",
        "+",
        "-[Foo]",
        "-[Foo ]",
        "-[ Foo bar]",
        "-[Foo bar",
        "+Foo bar]",
        "-[Foo bar baz]",
        "-[Foo bar::]",
        "-[Foo :bar]",
        "-[Foo bar:baz]",
        "-[Foo(bar baz]",
        "-[Foo() bar]",
        "_OBJC_",
        "__OBJC_",
        "OBJC_",
        "__OBJC_$_",
        "_OBJC_CLASS_$_",
        "_OBJC_NOPE_$_Foo",
        "__OBJC_$_NOPE_Foo",
        "__OBJC_$_CATEGORY_Foo",
        "__OBJC_$_CATEGORY__$_Bar",
        "__OBJC_$_CATEGORY_Foo_$_",
        "__OBJC_$_PROP_LIST_Foo_$_",
        "_OBJC_IVAR_$_Foo",
        "_OBJC_IVAR_$_._ivar",
        "_OBJC_METH_VAR_NAME_",
        "_OBJC_METH_VAR_NAME_x",
        "__objc_",
        "__objc_exec_class",
        "__objc_class_name_",
        "__objc_ivar_offset_Foo",
        "__objc_category_name_Foo_Bar",
        "__block_descriptor_",
        "___block_descriptor_",
    ] {
        assert!(
            parse_bytes_with_budget(symbol.as_bytes(), DEFAULT_BUDGET).is_err(),
            "{symbol}"
        );
    }
}

#[test]
fn is_objc_recognizes_namespaces() {
    for symbol in [
        "_OBJC_CLASS_$_Foo",
        "__OBJC_$_INSTANCE_METHODS_Foo",
        "__objc_class_name_Foo",
        "-[Foo bar:]",
        "__block_descriptor_40_8_32r_e5_v8?0l",
    ] {
        assert!(is_objc(symbol.as_bytes()), "{symbol}");
    }
    // The Apple namespace is claimed only for known kinds; a malformed
    // record under a known prefix is claimed so `parse` can reject it.
    assert!(!is_objc(b"_OBJC_NOPE_$_Foo"));
    assert!(parse_bytes_with_budget(b"_OBJC_NOPE_$_Foo", DEFAULT_BUDGET).is_err());
    assert!(is_objc(b"_OBJC_"));
    assert!(parse_bytes_with_budget(b"_OBJC_", DEFAULT_BUDGET).is_err());
    assert!(is_objc(b"_OBJC_CLASS_$_"));
    assert!(parse_bytes_with_budget(b"_OBJC_CLASS_$_", DEFAULT_BUDGET).is_err());
    for symbol in [
        "main",
        "_Z3foov",
        "__objc_exec_class",
        "__objc_category_name_Foo_Bar",
        "-[Foo",
    ] {
        assert!(!is_objc(symbol.as_bytes()), "{symbol}");
    }
}

#[test]
fn decorated_names_are_matched_on_bytes() {
    // The decoration is matched on raw bytes, so a subject that is not
    // UTF-8 still demangles and prints escaped.
    let ast = parse_bytes_with_budget(b"._OBJC_CLASS_F\xffo", DEFAULT_BUDGET).expect("demangles");
    assert_eq!(ast.to_string(), "Objective-C class F\\xffo");
    assert!(is_objc(b"$_OBJC_REF_CLASS_F\xffo"));
    assert!(is_objc(b"_$_OBJC_PROTOCOL_F\xffo"));
    assert!(!is_objc(b".objc_class_F\xffo"));
}

#[test]
fn input_budget_is_enforced() {
    let mut symbol = String::from("_OBJC_CLASS_$_");
    symbol.push_str(&"a".repeat(MAX_INPUT_LEN));
    assert_eq!(
        parse_bytes_with_budget(symbol.as_bytes(), DEFAULT_BUDGET).unwrap_err(),
        Failure::BudgetExceeded
    );
}

#[test]
fn depth_budget_is_enforced() {
    let symbol = "-[Foo bar[[]]]";
    assert_eq!(
        parse_bytes_with_budget(symbol.as_bytes(), DEFAULT_BUDGET).unwrap_err(),
        Failure::RecursedTooDeep
    );
}

#[test]
fn output_budget_is_respected() {
    for symbol in [
        "_OBJC_CLASS_$_Foo",
        "__OBJC_$_PROTOCOL_INSTANCE_METHODS_OPT_Foo",
        "__OBJC_$_CATEGORY_Foo_$_Bar",
        "__objc_ivar_offset_value_Foo._ivar",
        "-[Foo bar:baz:]",
        "___block_descriptor_40_e8_32r_e5_v8?0l",
    ] {
        let rendered = text(symbol);
        assert!(
            rendered.len() <= symbol.len() + 64,
            "{symbol}: {} > {}",
            rendered.len(),
            symbol.len() + 64
        );
    }
}

#[test]
fn errors_are_displayed() {
    assert_eq!(Failure::Invalid.to_string(), "invalid syntax");
    assert_eq!(
        Failure::RecursedTooDeep.to_string(),
        "recursion limit reached"
    );
    assert_eq!(Failure::BudgetExceeded.to_string(), "size limit reached");
}

#[test]
fn never_panics_on_truncated_names() {
    for symbol in [
        "_OBJC_CLASS_$_My_Class$Sub",
        "__OBJC_$_CATEGORY_Foo_$_Bar",
        "__OBJC_$_PROTOCOL_INSTANCE_METHODS_OPT_NSCopying",
        "__objc_ivar_offset_value_Foo._ivar",
        "-[Root(Cat) bar:baz:]",
        "___block_descriptor_40_e8_32r_e5_v8?0l",
    ] {
        for (end, _) in symbol.char_indices() {
            let _ = parse_bytes_with_budget(&symbol.as_bytes()[..end], DEFAULT_BUDGET);
            let _ = is_objc(&symbol.as_bytes()[..end]);
        }
        let _ = parse_bytes_with_budget(symbol.as_bytes(), DEFAULT_BUDGET);
    }
}

#[test]
fn never_panics_on_arbitrary_input() {
    const CHARSET: &[u8] = b"abcXYZ019._$()[]:? -+@";
    let mut state = 0x0B7C_1234_u64;
    let mut next = || {
        state = state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        (state >> 33) as usize
    };
    for _ in 0..10_000 {
        let len = next() % 48;
        let mut bytes = Vec::with_capacity(len);
        for _ in 0..len {
            bytes.push(CHARSET[next() % CHARSET.len()]);
        }
        if let Ok(text) = core::str::from_utf8(&bytes) {
            let _ = parse_bytes_with_budget(text.as_bytes(), DEFAULT_BUDGET);
            let _ = is_objc(text.as_bytes());
        }
    }
}
