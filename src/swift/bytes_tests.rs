use alloc::borrow::Cow;

use super::*;

/// Render a raw byte symbol through the byte entry point.
fn render_bytes(symbol: &[u8]) -> Option<alloc::string::String> {
    let mut out = alloc::string::String::new();
    print_bytes(symbol, &mut out).ok()?;
    Some(out)
}

#[test]
fn byte_entry_matches_text_entry() {
    // Stable, legacy and Unicode symbols must parse and print identically
    // through both entries.
    for symbol in [
        "$s4main3fooyySiF",
        "$s4main1AVSDyACAC_G_ACt",
        "$s4main3fooyySiF.ig",
        "$s4main5f\u{f6}\u{f6}",
        "$s1A1BA_",
        "_TtSi",
        "_TtGSaSS_",
        "_TtGSqGSaC5sugar7MyClass__",
        "_TtSiXYZ",
    ] {
        let mut from_text = alloc::string::String::new();
        if print(symbol, &mut from_text).is_err() {
            // Both entries must reject the same symbols.
            assert_eq!(render_bytes(symbol.as_bytes()), None, "{symbol}");
            continue;
        }
        assert_eq!(
            render_bytes(symbol.as_bytes()).as_deref(),
            Some(from_text.as_str()),
            "{symbol}"
        );
        assert_eq!(
            parse(symbol).is_ok(),
            parse_bytes(symbol.as_bytes()).is_ok(),
            "{symbol}"
        );
    }
}

#[test]
fn identifier_backslashes_are_reversible() {
    assert_eq!(render_bytes(b"$s4main3a\\b").as_deref(), Some("maina\\\\b"));
}

#[test]
fn invalid_identifier_bytes_are_escaped() {
    // Identifier lengths are byte counts, so a length can split a UTF-8
    // sequence; the bytes are kept and escaped instead of rejecting.
    assert_eq!(
        render_bytes(b"$s4main3f\xffo").as_deref(),
        Some("mainf\\xffo")
    );
    assert_eq!(
        render_bytes(b"_TtC5sugar3f\xffo").as_deref(),
        Some("sugar.f\\xffo")
    );
    assert_eq!(
        render_bytes(b"_TtC5sug\xffr3abc").as_deref(),
        Some("sug\\xffr.abc")
    );
}

#[test]
fn word_substitution_reuses_escaped_identifier() {
    // `0b0` is a word-substituted identifier: it must render exactly like
    // the identifier the word was read from.
    assert_eq!(
        render_bytes(b"$s4main3f\xffo0b0").as_deref(),
        Some("mainf\\xffof\\xffo")
    );
}

#[test]
fn valid_utf8_identifiers_are_borrowed() {
    // Valid UTF-8 without identifier-special bytes borrows from the input.
    assert_eq!(render_bytes(b"$s4main3abc").as_deref(), Some("mainabc"));
    let ast = parse("$s4main5f\u{f6}\u{f6}").expect("demangles");
    let mut stack = std::vec![ast.root()];
    let mut borrowed = 0_usize;
    while let Some(node) = stack.pop() {
        if let Some(text) = node.text_cow() {
            assert!(matches!(text, Cow::Borrowed(_)));
            borrowed += 1;
        }
        stack.extend(node.children().iter().cloned());
    }
    assert!(borrowed >= 2, "expected borrowed identifier text");
}

#[test]
fn lists_have_no_length_cap_of_their_own() {
    use alloc::format;
    use alloc::string::String;

    // C fixed-size arrays import as wide tuples; the node and input
    // budgets bound them, not a fixed element count.
    let tuple = format!("$sSi_{}tD", "Si".repeat(255));
    let ast = parse(&tuple).expect("a 256-element tuple parses");
    let mut out = String::new();
    ast.print(&mut out).unwrap();
    assert_eq!(out.matches("Swift.Int").count(), 256);
}

#[test]
fn generic_parameter_names_are_charged_not_elided() {
    use alloc::string::String;

    // 199 generic parameters at depth 1 print every name; the expansion
    // budget, not a printer cutoff, bounds a huge count.
    let mut out = String::new();
    print("_Ttu0_198_rW_2T_2TJ_", &mut out).expect("parses");
    assert!(!out.contains("..."), "{out}");
    assert!(out.contains(", RH1>"), "{out}");
    assert!(parse("_Ttu4222222222222222222222222_rW_2T_2TJ_").is_err());
}

#[test]
fn a_payload_never_prints_undemangled_under_any_budget() {
    use alloc::string::ToString;

    const SYMBOL: &str =
        "_TTSf1cl35_TFF7specgen6callerFSiT_U_FTSiSi_T_Si___TF7specgen12take_closureFFTSiSi_T_T_";
    let full = crate::Demangler::new()
        .demangle(SYMBOL)
        .unwrap()
        .to_string();
    assert!(full.contains("closure #1 (Swift.Int, Swift.Int)"), "{full}");
    let mut rejected = 0;
    for limit in 0..400 {
        for budget in [
            crate::Budget::new().with_work(limit),
            crate::Budget::new().with_nodes(limit),
            crate::Budget::new().with_parse_depth(limit / 16),
        ] {
            match crate::Demangler::with_budget(budget).demangle(SYMBOL) {
                Ok(demangled) => assert_eq!(demangled.to_string(), full, "{budget:?}"),
                Err(_) => rejected += 1,
            }
        }
    }
    assert!(rejected > 0, "the sweep never reached the payload's budget");
}

#[test]
fn payload_bytes_stay_verbatim() {
    // Compiler payloads keep their bytes: an invalid byte is escaped, a
    // backslash is not rewritten.
    assert_eq!(
        render_bytes(b"$s4main3fooyySiF.\xff\\x").as_deref(),
        Some("main.foo(Swift.Int) -> () with unmangled suffix \".\\xff\\x\"")
    );
    assert_eq!(
        render_bytes(b"_TtSi\xff").as_deref(),
        Some("Swift.Int with unmangled suffix \"\\xff\"")
    );
}

#[test]
fn byte_truncations_never_panic() {
    for symbol in [
        &b"$s4main3fooyySiF"[..],
        b"$s4main3f\xffoyySiF",
        b"$s4main3fooyySiF.\xff",
        b"_TtC5sugar7MyClass",
        b"_TtC5sug\xffr3abc",
        b"_TtSi\xff",
    ] {
        for end in 0..=symbol.len() {
            let _ = parse_bytes(&symbol[..end]);
            let _ = render_bytes(&symbol[..end]);
        }
    }
}

#[test]
fn punycoded_identifier_separator_is_consumed() {
    // A Punycode payload that starts with a digit or `_` carries an extra
    // `_` between its length and the payload, and the length counts only
    // the payload. Both raw identifiers in the official corpus need it.
    assert_eq!(
        render_bytes(b"$s15raw_identifiers0018_3times_pgaIGJCFbhayyF").as_deref(),
        Some("raw_identifiers.`3\u{a0}times`() -> ()")
    );
    assert_eq!(
        render_bytes(b"$s15raw_identifiers10FontWeightO009_100_FpEpdyyFZ").as_deref(),
        Some("static raw_identifiers.FontWeight.`100`() -> ()")
    );
    // The declared payload length still bounds the identifier, so a
    // truncated payload is rejected instead of running into the tail.
    assert_eq!(
        render_bytes(b"$s15raw_identifiers0018_3times_pgaIGJCFb"),
        None
    );
}

#[test]
fn input_budget_is_enforced() {
    // The cap applies to the bytes after the prefix. A symbol that fills
    // the budget parses; one byte more is rejected.
    let base = "$s4main3fooyySiF.";
    let rest = base.len() - 2;
    let under = alloc::format!("{base}{}", "x".repeat(MAX_INPUT_LEN - rest));
    assert!(parse_bytes(under.as_bytes()).is_ok());
    let over = alloc::format!("{base}{}", "x".repeat(MAX_INPUT_LEN - rest + 1));
    assert!(parse_bytes(over.as_bytes()).is_err());
}

#[test]
fn nested_type_budget_is_preserved() {
    // The parser-side printability ceiling keeps rejecting trees the
    // recursive printer cannot safely descend into.
    let symbol = alloc::format!("$s{}Si", "Sg".repeat(1_000));
    assert!(parse_bytes(symbol.as_bytes()).is_err());
}
