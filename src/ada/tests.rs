use alloc::string::ToString;

use super::*;

#[test]
fn parse_errors_describe_themselves() {
    assert_eq!(Failure::Invalid.to_string(), "invalid syntax");
    assert_eq!(Failure::BudgetExceeded.to_string(), "size limit reached");
}

#[test]
fn parses_qualified_names() {
    for (symbol, expected) in [
        (
            "ada__exceptions__triggered_by_abort",
            "ada.exceptions.triggered_by_abort",
        ),
        ("pkg__child__nested", "pkg.child.nested"),
        ("ada__foo_bar", "ada.foo_bar"),
        ("pkg__x1_y2", "pkg.x1_y2"),
    ] {
        assert_eq!(
            parse_bytes_with_budget(symbol.as_bytes(), DEFAULT_BUDGET)
                .unwrap()
                .to_string(),
            expected,
            "{symbol}"
        );
    }
}

#[test]
fn strips_protected_suffixes() {
    for (symbol, expected) in [
        ("pkg__prot__setN", "pkg.prot.set"),
        ("pkg__prot__setP", "pkg.prot.set"),
        ("pkg__prot__getN", "pkg.prot.get"),
        ("pkg__prot__getP", "pkg.prot.get"),
        ("pkg__enumN", "pkg.enum"),
        ("pkg__set2N", "pkg.set2"),
    ] {
        assert_eq!(
            parse_bytes_with_budget(symbol.as_bytes(), DEFAULT_BUDGET)
                .unwrap()
                .to_string(),
            expected,
            "{symbol}"
        );
    }
}

#[test]
fn strips_entry_suffixes() {
    for (symbol, expected) in [
        ("pkg__prot__wait_B4s", "pkg.prot.wait"),
        ("pkg__prot__wait_E3s", "pkg.prot.wait"),
        ("pkg__prot__fam_B9s", "pkg.prot.fam"),
        ("pkg__prot__fam_E8s", "pkg.prot.fam"),
        ("pkg__prot__wait_Bs", "pkg.prot.wait"),
        ("pkg__prot__wait_B04s", "pkg.prot.wait"),
        ("ada__prot__wait_B4s", "ada.prot.wait"),
    ] {
        assert_eq!(
            parse_bytes_with_budget(symbol.as_bytes(), DEFAULT_BUDGET)
                .unwrap()
                .to_string(),
            expected,
            "{symbol}"
        );
    }
}

#[test]
fn strips_task_body_suffix() {
    for (symbol, expected) in [
        ("pkg__workerTKB", "pkg.worker"),
        ("ada_probe__singletonTKB", "ada_probe.singleton"),
    ] {
        assert_eq!(
            parse_bytes_with_budget(symbol.as_bytes(), DEFAULT_BUDGET)
                .unwrap()
                .to_string(),
            expected,
            "{symbol}"
        );
    }
}

#[test]
fn prints_elaboration_suffixes() {
    for (symbol, expected) in [
        ("pkg___elabb", "pkg'Elab_Body"),
        ("pkg___elabs", "pkg'Elab_Spec"),
        ("pkg__child___elabb", "pkg.child'Elab_Body"),
        ("ada__foo___elabs", "ada.foo'Elab_Spec"),
    ] {
        assert_eq!(
            parse_bytes_with_budget(symbol.as_bytes(), DEFAULT_BUDGET)
                .unwrap()
                .to_string(),
            expected,
            "{symbol}"
        );
    }
}

#[test]
fn strips_platform_underscores() {
    for symbol in ["_ada__foo", "__ada__foo"] {
        assert_eq!(
            parse_bytes_with_budget(symbol.as_bytes(), DEFAULT_BUDGET)
                .unwrap()
                .to_string(),
            "ada.foo",
            "{symbol}"
        );
    }
}

#[test]
fn strips_linker_decorations() {
    assert_eq!(
        parse_bytes_with_budget(b"ada__foo.llvm.1234", DEFAULT_BUDGET)
            .unwrap()
            .to_string(),
        "ada.foo"
    );
    assert_eq!(
        parse_bytes_with_budget(b"ada__foo@@GNAT_1", DEFAULT_BUDGET)
            .unwrap()
            .to_string(),
        "ada.foo"
    );
}

#[test]
fn rejects_unsupported_suffixes() {
    for symbol in [
        "pkg__workerTB",
        "pkg__workerVIP",
        "pkg__workerZ",
        "pkg__workerTKE",
        "pkg__workerTKVIP",
        "pkg__workerTKZ",
        "pkg__workerT1",
        "pkg_E",
        "pkg__child_E",
        "pkg__recIP",
        "pkg__protVIP",
        "pkg__protA",
        "pkg__prot__wait_B4",
        "pkg__prot__wait_Bx4s",
        "pkg__prot__wait_B4S",
        "pkg__prot__wait_B4s__2",
        "pkg__prot__waitN_B4s",
        "pkg__prot__wait_B4s.2",
        "pkg___master",
        "pkg___chain",
        "pkg__objP2sIP",
        "pkg__enumS",
        "pkg__setX",
        "pkg__setNP",
        "pkg__over__2",
        "pkg__nested__inner.2",
        "pkg__setN___elabb",
        "pkg__workerTKB___elabb",
        "_ada_main",
        "DW.ref.__gnat_personality_v0",
    ] {
        assert!(
            parse_bytes_with_budget(symbol.as_bytes(), DEFAULT_BUDGET).is_err(),
            "{symbol} should be rejected"
        );
    }
}

#[test]
fn rejects_malformed_names() {
    for symbol in [
        "",
        "ada__",
        "ada__Foo",
        "ada__9foo",
        "ada__foo_",
        "ada__foo__",
        "ada__foo___elabx",
        "ada__foo___elabb__2",
        "ada__foo-bar",
        "ada__foo bar",
    ] {
        assert!(
            parse_bytes_with_budget(symbol.as_bytes(), DEFAULT_BUDGET).is_err(),
            "{symbol:?} should be rejected"
        );
    }
}

#[test]
fn budgets_are_enforced() {
    let long = "a".repeat(MAX_INPUT_LEN + 1);
    assert!(matches!(
        parse_bytes_with_budget(long.as_bytes(), DEFAULT_BUDGET),
        Err(Failure::BudgetExceeded)
    ));

    // The segment list is bounded by the input budget alone.
    let many = alloc::format!("a{}", "__a".repeat(4096));
    assert_eq!(
        parse_bytes_with_budget(many.as_bytes(), DEFAULT_BUDGET)
            .expect("many segments parse")
            .to_string()
            .len(),
        8193
    );
}

#[test]
fn detection_is_precision_only() {
    for symbol in [
        "ada__foo",
        "ada__exceptions__triggered_by_abort",
        "system__foo",
        "gnat__foo",
        "_ada__foo",
        "__ada__foo",
        "ada__",
        "ada__workerTB",
        "ada__foo__2",
        "system__workerTB",
    ] {
        assert!(is_ada(symbol.as_bytes()), "{symbol} should be Ada");
    }
    for symbol in ["foo__bar", "pkg__hello", "a__b__c", "_ada_main"] {
        assert!(!is_ada(symbol.as_bytes()), "{symbol} should not be Ada");
    }
}
