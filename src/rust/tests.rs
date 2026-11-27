use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use super::*;

#[test]
fn parse_errors_describe_themselves() {
    assert_eq!(Failure::Invalid.to_string(), "invalid syntax");
    assert_eq!(
        Failure::RecursedTooDeep.to_string(),
        "recursion limit reached"
    );
    assert_eq!(Failure::BudgetExceeded.to_string(), "size limit reached");
}
use crate::budget::Budget;
use crate::scheme::Scheme;

/// The scheme a symbol's mangling prefix announces.
fn scheme_of(symbol: &str) -> Scheme {
    if v0::prefix(symbol.as_bytes()).is_some() {
        Scheme::RustV0
    } else {
        Scheme::RustLegacy
    }
}

#[test]
fn the_compiled_in_budget_matches_the_policy_defaults() {
    let legacy = Budget::default().effective(Scheme::RustLegacy);
    assert_eq!(legacy.input_bytes, DEFAULT_BUDGET.input_bytes);
    assert_eq!(legacy.parse_depth, None);
    assert_eq!(legacy.work, DEFAULT_BUDGET.work);
    assert_eq!(legacy.nodes, DEFAULT_BUDGET.nodes);

    let v0 = Budget::default().effective(Scheme::RustV0);
    assert_eq!(v0, DEFAULT_BUDGET);
}

#[test]
fn absent_dimensions_keep_the_compiled_in_limits() {
    // An unset dimension must resolve to the constants the parser used
    // before the budget was configurable.
    for (scheme, v0) in [(Scheme::RustLegacy, false), (Scheme::RustV0, true)] {
        let limits = ParserLimits::from_effective(Budget::new().effective(scheme), v0);
        assert_eq!(limits.input, MAX_INPUT_LEN, "{scheme}");
        assert_eq!(limits.depth, MAX_DEPTH, "{scheme}");
        assert_eq!(limits.work, MAX_WORK, "{scheme}");
        assert_eq!(limits.nodes, MAX_NODES, "{scheme}");
    }
}

/// Every parser dimension rejects one unit below the value a symbol needs
/// and accepts at that value.
#[test]
fn lowering_each_dimension_rejects_at_the_boundary() {
    /// The hard cap of one dimension, which bounds the search.
    fn ceiling(dimension: Option<Limit>) -> usize {
        match dimension {
            Some(Limit::Bounded(value)) => value,
            _ => panic!("the scheme must bound this dimension"),
        }
    }

    fn assert_boundary(
        symbol: &str,
        name: &str,
        expected: Failure,
        ceiling: usize,
        budget: &dyn Fn(usize) -> Budget,
    ) {
        let scheme = scheme_of(symbol);
        let needed = (0..=ceiling)
            .find(|&value| {
                parse_bytes_with_budget(symbol.as_bytes(), budget(value).effective(scheme)).is_ok()
            })
            .unwrap_or_else(|| panic!("{symbol}: no {name} within {ceiling} parses"));
        assert!(needed > 0, "{symbol}: {name} is not bounded");
        assert_eq!(
            parse_bytes_with_budget(symbol.as_bytes(), budget(needed - 1).effective(scheme))
                .unwrap_err(),
            expected,
            "{symbol}: {name} accepted at {} (needs {needed})",
            needed - 1
        );
    }

    for symbol in [
        "_RC3foo",
        "_RNvCs1234_4test4main",
        "_ZN3fooE",
        "_ZN4test4main17h1234567890abcdefE",
    ] {
        let scheme = scheme_of(symbol);
        let hard = scheme.hard_budget();
        assert_boundary(
            symbol,
            "input",
            Failure::BudgetExceeded,
            ceiling(hard.input_bytes),
            &|value| Budget::new().with_input_bytes(value),
        );
        assert_boundary(
            symbol,
            "work",
            Failure::BudgetExceeded,
            ceiling(hard.work),
            &|value| Budget::new().with_work(value),
        );
        assert_boundary(
            symbol,
            "nodes",
            Failure::BudgetExceeded,
            ceiling(hard.nodes),
            &|value| Budget::new().with_nodes(value),
        );
        if scheme == Scheme::RustV0 {
            assert_boundary(
                symbol,
                "depth",
                Failure::RecursedTooDeep,
                ceiling(hard.parse_depth),
                &|value| Budget::new().with_parse_depth(value),
            );
        }
    }
}

/// A symbol the default budget rejects for its length is accepted once the
/// caller raises the input budget within the hard cap.
#[test]
fn raising_the_input_budget_accepts_longer_symbols() {
    let mut symbol = String::from("_ZN");
    while symbol.len() < MAX_INPUT_LEN + 4 * 1024 {
        symbol.push_str("3foo");
    }
    symbol.push('E');
    assert_eq!(
        parse_bytes_with_budget(symbol.as_bytes(), DEFAULT_BUDGET).unwrap_err(),
        Failure::BudgetExceeded
    );

    let budget = Budget::new().with_input_bytes(2 * MAX_INPUT_LEN);
    let ast = parse_bytes_with_budget(symbol.as_bytes(), budget.effective(Scheme::RustLegacy))
        .expect("raised input budget");
    assert_eq!(ast.encoding(), Encoding::Legacy);
    assert_eq!(ast.root().children().len(), (symbol.len() - 3) / 4);
}

/// The depth hard cap admits a tree deeper than the reference limit, as
/// long as the fixed print envelope still covers it.
#[test]
fn raising_the_depth_budget_accepts_deeper_trees() {
    let symbol = format!("_R{}C0{}", "I".repeat(500), "E".repeat(500));
    assert_eq!(
        parse_bytes_with_budget(symbol.as_bytes(), DEFAULT_BUDGET).unwrap_err(),
        Failure::RecursedTooDeep
    );

    let budget = Budget::new().with_parse_depth(501);
    let ast = parse_bytes_with_budget(symbol.as_bytes(), budget.effective(Scheme::RustV0))
        .expect("raised depth budget");
    assert_ne!(ast.to_string(), "");
}

/// Work and nodes are separate dimensions: raising the input budget alone
/// still rejects on the node budget, and raising both accepts.
#[test]
fn raising_the_node_budget_accepts_wider_trees() {
    let symbol = format!("_RMC0T{}E", "p".repeat(70_000));
    assert_eq!(
        parse_bytes_with_budget(symbol.as_bytes(), DEFAULT_BUDGET).unwrap_err(),
        Failure::BudgetExceeded
    );

    let input_only = Budget::new().with_input_bytes(128 * 1024);
    assert_eq!(
        parse_bytes_with_budget(symbol.as_bytes(), input_only.effective(Scheme::RustV0))
            .unwrap_err(),
        Failure::BudgetExceeded
    );

    let raised = input_only.with_nodes(100_000);
    assert!(parse_bytes_with_budget(symbol.as_bytes(), raised.effective(Scheme::RustV0)).is_ok());
}

/// A long back-reference run costs work, not nodes, so it needs the input
/// and work budgets raised together.
#[test]
fn raising_the_work_budget_accepts_longer_backref_runs() {
    let mut symbol = String::from("_RMC0TTpE");
    for _ in 0..MAX_WORK {
        symbol.push_str("B3_");
    }
    symbol.push('E');
    assert_eq!(
        parse_bytes_with_budget(symbol.as_bytes(), DEFAULT_BUDGET).unwrap_err(),
        Failure::BudgetExceeded
    );

    let raised = Budget::new()
        .with_input_bytes(1024 * 1024)
        .with_work(1_000_000)
        .with_expansion_visits(usize::MAX)
        .with_expansion_bytes(usize::MAX);
    assert!(parse_bytes_with_budget(symbol.as_bytes(), raised.effective(Scheme::RustV0)).is_ok());
}

/// Raising a dimension past its hard cap is clamped, never unbounded.
#[test]
fn over_hard_budgets_are_clamped_to_the_hard_caps() {
    let budget = Budget::new()
        .with_input_bytes(usize::MAX)
        .with_parse_depth(usize::MAX)
        .with_work(usize::MAX)
        .with_nodes(usize::MAX);
    assert_eq!(
        budget.effective(Scheme::RustV0).input_bytes,
        Some(Limit::Bounded(1024 * 1024))
    );
    assert_eq!(
        budget.effective(Scheme::RustLegacy).input_bytes,
        Some(Limit::Bounded(16 * 1024 * 1024))
    );
    assert_eq!(
        budget.effective(Scheme::RustV0).parse_depth,
        Some(Limit::Bounded(512))
    );
    assert_eq!(
        budget.effective(Scheme::RustV0).nodes,
        Some(Limit::Bounded(1024 * 1024))
    );
}

/// Zero at a migrated dimension rejects through the public demangler and
/// through a Selection sharing the same budget.
#[test]
fn zero_input_budget_rejects_through_demangler_and_selection() {
    use crate::Demangler;

    let demangler = Demangler::with_budget(Budget::new().with_input_bytes(0));
    assert!(demangler.demangle("_RC3foo").is_err());
    assert!(
        demangler
            .selection()
            .allow(Scheme::RustV0)
            .demangle("_RC3foo")
            .is_err()
    );
}

/// Raising a dimension through the demangler, not only through the
/// backend parse helper.
#[test]
fn demangler_raising_the_depth_budget_accepts_deeper_trees() {
    use crate::Demangler;

    let symbol = format!("_R{}C0{}", "I".repeat(500), "E".repeat(500));
    assert!(Demangler::new().demangle(&symbol).is_err());
    let raised = Demangler::with_budget(Budget::new().with_parse_depth(501));
    assert!(raised.demangle(&symbol).is_ok());
}

/// Rust legacy is iterative and has no depth dimension; a depth override
/// is not reported as enforced for it.
#[test]
fn legacy_reports_no_depth_dimension() {
    assert_eq!(
        Budget::new()
            .with_parse_depth(0)
            .effective(Scheme::RustLegacy)
            .parse_depth,
        None
    );
    assert_eq!(
        Scheme::RustV0.hard_budget().parse_depth,
        Some(Limit::Bounded(512))
    );
}

#[test]
fn detects_legacy_prefixes() {
    for (symbol, prefix) in [
        ("_ZN3fooE", "_ZN"),
        ("ZN3fooE", "ZN"),
        ("__ZN3fooE", "__ZN"),
    ] {
        let ast = parse_bytes_with_budget(symbol.as_bytes(), DEFAULT_BUDGET).unwrap();
        assert_eq!(ast.encoding(), Encoding::Legacy);
        assert_eq!(ast.prefix(), prefix);
        assert_eq!(ast.to_string(), "foo");
    }
}

#[test]
fn detects_v0_prefixes() {
    for (symbol, prefix) in [("_RC3foo", "_R"), ("RC3foo", "R"), ("__RC3foo", "__R")] {
        let ast = parse_bytes_with_budget(symbol.as_bytes(), DEFAULT_BUDGET).unwrap();
        assert_eq!(ast.encoding(), Encoding::V0);
        assert_eq!(ast.prefix(), prefix);
        assert_eq!(ast.to_string(), "foo");
    }
}

#[test]
fn rejects_short_prefixes() {
    for symbol in ["_R", "R", "__R", "_ZN", "ZN", "__ZN", ""] {
        assert_eq!(
            parse_bytes_with_budget(symbol.as_bytes(), DEFAULT_BUDGET).unwrap_err(),
            Failure::Invalid,
            "{symbol}"
        );
    }
}

#[test]
fn strips_thin_lto_suffix() {
    for symbol in [
        "_ZN3fooE.llvm.9D1C9369",
        "_ZN3fooE.llvm.9D1C9369@@16",
        "_RC3foo.llvm.9D1C9369",
    ] {
        let ast = parse_bytes_with_budget(symbol.as_bytes(), DEFAULT_BUDGET).unwrap();
        assert_eq!(ast.suffix(), "");
        assert_eq!(ast.to_string(), "foo");
    }
}

#[test]
fn keeps_vendor_suffix() {
    let ast = parse_bytes_with_budget(
        b"_RNvNtC4rand4fork23FORK_HANDLER_REGISTERED.0.0",
        DEFAULT_BUDGET,
    )
    .unwrap();
    assert_eq!(ast.suffix(), ".0.0");
    assert_eq!(ast.to_string(), "rand::fork::FORK_HANDLER_REGISTERED.0.0");
}

#[test]
fn keeps_a_bare_thin_lto_marker() {
    // The shared decoration stripper only removes `.llvm.` when a non-empty
    // hex/`@` candidate follows, so a bare marker stays a vendor suffix
    // instead of vanishing from the printed name.
    let ast = parse_bytes_with_budget(b"_ZN3fooE.llvm.", DEFAULT_BUDGET).unwrap();
    assert_eq!(ast.suffix(), ".llvm.");
    assert_eq!(ast.to_string(), "foo.llvm.");
}

#[test]
fn rejects_unsymbol_like_suffix() {
    assert_eq!(
        parse_bytes_with_budget(b"_ZN3fooE.llvm moocow", DEFAULT_BUDGET).unwrap_err(),
        Failure::Invalid
    );
    assert_eq!(
        parse_bytes_with_budget(b"_ZN3fooEv", DEFAULT_BUDGET).unwrap_err(),
        Failure::Invalid
    );
    assert_eq!(
        parse_bytes_with_budget(b"_RC3foo bar", DEFAULT_BUDGET).unwrap_err(),
        Failure::Invalid
    );
}

#[test]
fn rejects_cpp_names() {
    // `_ZN3Foo3barEv` leaves a `v` tail that is not a vendor suffix, and
    // `_ZN4test` has no `E` terminator.
    for symbol in ["_ZN3Foo3barEv", "_ZN4test"] {
        assert_eq!(
            parse_bytes_with_budget(symbol.as_bytes(), DEFAULT_BUDGET).unwrap_err(),
            Failure::Invalid,
            "{symbol}"
        );
    }
    // `_ZN3fooE` is valid in both schemes; only a full parse decides.
    assert_eq!(
        parse_bytes_with_budget(b"_ZN3fooE", DEFAULT_BUDGET)
            .unwrap()
            .to_string(),
        "foo"
    );
}

#[test]
fn input_budget_is_enforced() {
    let mut symbol = String::from("_R");
    symbol.push_str(&"p".repeat(MAX_INPUT_LEN));
    assert_eq!(
        parse_bytes_with_budget(symbol.as_bytes(), DEFAULT_BUDGET).unwrap_err(),
        Failure::BudgetExceeded
    );
    // The byte entry point enforces the same input budget.
    assert_eq!(
        parse_bytes_with_budget(symbol.as_bytes(), DEFAULT_BUDGET).unwrap_err(),
        Failure::BudgetExceeded
    );
}

/// A long but valid symbol stays inside the work and node budgets: the
/// cursor charges one work unit per byte read, so a symbol that fits the
/// input budget must not trip the work budget first.
#[test]
fn long_symbols_stay_within_the_work_budget() {
    let mut symbol = String::from("_ZN");
    while symbol.len() < MAX_INPUT_LEN - 8 {
        symbol.push_str("3foo");
    }
    symbol.push('E');
    let ast = parse_bytes_with_budget(symbol.as_bytes(), DEFAULT_BUDGET).unwrap();
    assert_eq!(ast.encoding(), Encoding::Legacy);
    assert_eq!(ast.root().children().len(), (symbol.len() - 3) / 4);
    assert_ne!(ast.to_string(), "");
}

/// Arbitrary bytes, including invalid UTF-8, never panic the parsers and
/// never exceed the output budget.
#[test]
fn byte_fuzz_never_panics() {
    const PREFIXES: [&[u8]; 6] = [b"_R", b"_RN", b"_ZN", b"ZN", b"__R", b"R"];
    let mut state = 0x2545_F491_4F6C_DD1D_u64;
    let mut next = move || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    for iteration in 0..2_000 {
        let prefix = PREFIXES[usize::try_from(next() % 6).unwrap_or(0)];
        let mut symbol = Vec::from(prefix);
        let len = usize::try_from(next() % 24).unwrap_or(0);
        for _ in 0..len {
            symbol.push(u8::try_from(next() % 256).unwrap_or(0));
        }
        if let Ok(ast) = parse_bytes_with_budget(&symbol, DEFAULT_BUDGET) {
            let rendered = ast.to_string();
            assert!(
                rendered.len() <= 1_000_000,
                "iteration {iteration}: {} bytes of output",
                rendered.len()
            );
        }
    }
}

#[test]
fn rejects_non_ascii_suffixes() {
    // A vendor suffix must stay symbol-like, which is an ASCII-only rule.
    assert_eq!(
        parse_bytes_with_budget(
            "_RNvNtC4rand4fork23FORK_HANDLER_REGISTERED.0é".as_bytes(),
            DEFAULT_BUDGET
        )
        .unwrap_err(),
        Failure::Invalid
    );
}

/// Names are byte strings: an identifier that is not UTF-8 parses and is
/// escaped by the printer instead of being rejected.
#[test]
fn accepts_invalid_utf8_identifiers() {
    let ast = parse_bytes_with_budget(b"_ZN2\xff\xfeE", DEFAULT_BUDGET).unwrap();
    assert_eq!(ast.to_string(), "\\xff\\xfe");
    let ast = parse_bytes_with_budget("_ZN5téstE".as_bytes(), DEFAULT_BUDGET).unwrap();
    assert_eq!(ast.to_string(), "tést");
    let ast = parse_bytes_with_budget(b"_RC2\xff\xfe", DEFAULT_BUDGET).unwrap();
    assert_eq!(ast.to_string(), "\\xff\\xfe");
}

/// The byte entry point is crate-internal and mirrors `parse`.
#[test]
fn corpus_print_envelope_holds() {
    let base = format!("{}/refs/rust/corpus", env!("CARGO_MANIFEST_DIR"));
    let Ok(entries) = std::fs::read_dir(base) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("txt") {
            continue;
        }
        let Ok(data) = std::fs::read_to_string(&path) else {
            continue;
        };
        for line in data.lines() {
            let symbol = line.split(" ---> ").next().unwrap_or_default();
            if symbol.is_empty() || symbol.starts_with('#') {
                continue;
            }
            let Ok(ast) = parse_bytes_with_budget(symbol.as_bytes(), DEFAULT_BUDGET) else {
                continue;
            };
            let mut out = String::new();
            let (visits, depth, bytes) = match printer::stats(&ast, &mut out) {
                Ok(stats) => stats,
                Err(error) => panic!("{symbol}: print failed after envelope: {error:?}"),
            };
            let root = ast.root();
            assert!(
                visits <= root.print_visits(),
                "{symbol}: {visits} visits > {}",
                root.print_visits()
            );
            assert!(
                depth <= root.print_depth().saturating_mul(2),
                "{symbol}: depth {depth} > {}",
                root.print_depth().saturating_mul(2)
            );
            assert!(
                bytes <= root.print_bytes(),
                "{symbol}: {bytes} bytes > {}",
                root.print_bytes()
            );
        }
    }
}

#[test]
fn parse_bytes_matches_parse() {
    for symbol in [
        "_ZN3fooE",
        "_ZN4test4main17h1234567890abcdefE",
        "_RC3foo",
        "_RNvCs1234_4test4main",
        "_ZN3fooE.llvm.9D1C9369",
        "_ZN3fooE.llvm moocow",
        "_RC3foo bar",
        "",
        "main",
    ] {
        let text = parse_bytes_with_budget(symbol.as_bytes(), DEFAULT_BUDGET);
        let bytes = parse_bytes_with_budget(symbol.as_bytes(), DEFAULT_BUDGET);
        match (text, bytes) {
            (Ok(text), Ok(bytes)) => {
                assert_eq!(text.to_string(), bytes.to_string(), "{symbol}");
                assert_eq!(text.prefix(), bytes.prefix(), "{symbol}");
                assert_eq!(text.suffix(), bytes.suffix(), "{symbol}");
                assert_eq!(text.encoding(), bytes.encoding(), "{symbol}");
            }
            (Err(text), Err(bytes)) => assert_eq!(text, bytes, "{symbol}"),
            _ => panic!("{symbol}: byte and text paths disagree"),
        }
    }
}

#[test]
fn ast_encoding_is_reported() {
    assert_eq!(
        parse_bytes_with_budget(b"_ZN3fooE", DEFAULT_BUDGET)
            .unwrap()
            .encoding(),
        Encoding::Legacy
    );
    assert_eq!(
        parse_bytes_with_budget(b"_RC3foo", DEFAULT_BUDGET)
            .unwrap()
            .encoding(),
        Encoding::V0
    );
}
