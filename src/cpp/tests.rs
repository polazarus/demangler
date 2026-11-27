use alloc::format;

use super::*;
use crate::{Budget, Demangler};
#[test]
fn corpus_print_envelope_holds() {
    let base = env!("CARGO_MANIFEST_DIR");
    if !std::path::Path::new(base).join("refs/cpp").is_dir() {
        std::eprintln!("refs/cpp is absent; skipping the C++ envelope corpus check");
        return;
    }
    let mut checked = 0_usize;

    // Itanium text corpus: { "mangled", "expected" } entries.
    if let Ok(data) =
        std::fs::read_to_string(format!("{base}/refs/cpp/itanium/DemangleTestCases.inc"))
    {
        for line in data.lines() {
            let Some(rest) = line.strip_prefix("{\"") else {
                continue;
            };
            let Some((symbol, _)) = rest.split_once("\",") else {
                continue;
            };
            checked += check_cpp_envelope(symbol);
        }
    }

    // Legacy corpora: {@code <mangled><TAB><expected>}.
    for name in [
        "gnu-v2.txt",
        "gnu-v2-large.txt",
        "arm.txt",
        "edg.txt",
        "hp.txt",
    ] {
        let path = format!("{base}/refs/cpp/legacy/{name}");
        let Ok(data) = std::fs::read_to_string(path) else {
            continue;
        };
        for line in data.lines() {
            if line.starts_with('#') || line.is_empty() {
                continue;
            }
            let symbol = line.split('\t').next().unwrap_or_default();
            checked += check_cpp_envelope(symbol);
        }
    }

    // MSVC real-symbol corpora and .test fixtures.
    let msvc = format!("{base}/refs/cpp/msvc");
    if let Ok(entries) = std::fs::read_dir(&msvc) {
        for entry in entries.flatten() {
            let path = entry.path();
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            let extension = path.extension().and_then(|extension| extension.to_str());
            if !(name.starts_with("real-") && extension == Some("txt") || extension == Some("test"))
            {
                continue;
            }
            let Ok(data) = std::fs::read_to_string(&path) else {
                continue;
            };
            for line in data.lines() {
                let symbol = line.trim();
                if symbol.starts_with('?') {
                    checked += check_cpp_envelope(symbol);
                }
            }
        }
    }

    assert!(checked > 0, "no C++ corpus symbols were checked");
}

fn check_cpp_envelope(symbol: &str) -> usize {
    let Ok(ast) = parse(symbol) else {
        return 0;
    };
    let mut out = alloc::string::String::new();
    let stats = match ast.dialect() {
        #[cfg(feature = "cpp-itanium")]
        Dialect::Itanium => printer::stats(&ast, &mut out),
        #[cfg(feature = "cpp-msvc")]
        Dialect::Msvc => msvc_printer::stats(&ast, &mut out).map_err(|_| fmt::Error),
        #[cfg(feature = "cpp-gnu-v2")]
        Dialect::GnuV2 | Dialect::Arm | Dialect::Edg | Dialect::Hp => legacy::stats(&ast, &mut out),
        #[allow(
            unreachable_patterns,
            reason = "reachable when a backend feature is disabled"
        )]
        _ => Err(fmt::Error),
    };
    let (visits, depth, bytes) = stats.expect("accepted C++ tree must render");
    let root = ast.root();
    let depth_factor = if matches!(ast.dialect(), Dialect::Msvc) {
        1
    } else {
        2
    };
    assert!(
        visits <= root.print_visits().saturating_mul(4),
        "{symbol}: {visits} visits > {}",
        root.print_visits().saturating_mul(4)
    );
    assert!(
        depth <= root.print_depth().saturating_mul(depth_factor),
        "{symbol}: depth {depth} > {}",
        root.print_depth().saturating_mul(depth_factor)
    );
    assert!(
        bytes <= root.print_bytes(),
        "{symbol}: {bytes} bytes > {}",
        root.print_bytes()
    );
    1
}

#[test]
fn recognises_prefixes() {
    assert_eq!(prefix_len("_Z1fv"), Some(2));
    assert_eq!(prefix_len("__Z1fv"), Some(3));
    assert_eq!(prefix_len("___Z1fv"), Some(4));
    assert_eq!(prefix_len("____Z1fv"), Some(5));
    assert_eq!(prefix_len("_Z"), Some(2));
    assert_eq!(prefix_len("main"), None);
    assert!(is_itanium("_ZN3Foo3barEv"));
    assert!(!is_itanium("$s4main3foo"));
}

#[test]
fn rejects_non_itanium() {
    let error = parse("main").unwrap_err();
    assert_eq!(error.kind(), ParseErrorKind::NotItanium);
    assert_eq!(error.position(), 0);
}

#[cfg(feature = "cpp-itanium")]
#[test]
fn root_is_an_encoding() {
    let ast = parse("_Z1A").unwrap();
    assert_eq!(ast.root().kind(), Kind::Encoding);
    assert_eq!(ast.root().child_count(), 1);
    assert_eq!(ast.root().child(0).unwrap().kind(), Kind::Data);
}

#[cfg(feature = "cpp-itanium")]
#[test]
fn lambda_flag_tracks_the_graph() {
    assert!(parse("_ZNK10inline_varMUlvE_clEv").unwrap().has_lambda());
    assert!(!parse("_Z3foov").unwrap().has_lambda());
}

#[cfg(feature = "cpp-itanium")]
#[test]
fn ast_clone_shares_the_root() {
    let ast = parse("_Z1fv").unwrap();
    let cloned = ast.clone();
    assert!(Rc::ptr_eq(&ast.root(), &cloned.root()));
}

#[cfg(feature = "cpp-itanium")]
#[test]
fn ast_debug_is_bounded() {
    let ast = parse("_Z1fPPPPi").unwrap();
    let rendered = alloc::format!("{ast:?}");
    assert!(rendered.contains("Ast"));
    assert!(rendered.contains("Encoding"));
    assert!(!rendered.contains("BuiltinType"));
}

/// Collect the text of every node carrying `kind`, bounded by the node
/// budget so a shared DAG cannot loop.
#[cfg(any(feature = "cpp-itanium", feature = "cpp-msvc", feature = "cpp-gnu-v2"))]
fn texts_of(ast: &Ast<'_>, kind: Kind) -> alloc::vec::Vec<alloc::string::String> {
    let mut out = alloc::vec::Vec::new();
    let mut stack = alloc::vec![ast.root()];
    let mut visits = 0_usize;
    while let Some(node) = stack.pop() {
        visits += 1;
        assert!(visits < 1 << 16, "node walk is bounded");
        if node.kind() == kind
            && let Some(text) = node.text()
        {
            out.push(alloc::string::String::from(text));
        }
        stack.extend(node.children().iter().cloned());
    }
    out
}

#[cfg(feature = "cpp-itanium")]
#[test]
fn byte_path_accepts_invalid_utf8_in_source_names() {
    // The source name is length-prefixed by bytes, so the `\xff` byte is
    // part of the identifier even though it is not UTF-8.
    let ast = parse_bytes(b"_ZN4F\xffoo3barEv").unwrap();
    let texts = texts_of(&ast, Kind::Identifier);
    assert!(texts.iter().any(|text| text == "F\\xffoo"), "{texts:?}");
    assert!(texts.iter().any(|text| text == "bar"), "{texts:?}");

    // Spans address the supplied bytes.
    let mut stack = alloc::vec![ast.root()];
    let mut found = None;
    while let Some(node) = stack.pop() {
        if node.text() == Some("F\\xffoo") {
            found = node.span();
            break;
        }
        stack.extend(node.children().iter().cloned());
    }
    assert_eq!(found, Some(3..8));
}

#[cfg(feature = "cpp-gnu-v2")]
#[test]
fn byte_path_accepts_invalid_utf8_in_legacy_names() {
    let ast = parse_bytes(b"f\xffoo__Fiv").unwrap();
    assert_eq!(ast.dialect(), Dialect::GnuV2);
    let texts = texts_of(&ast, Kind::Identifier);
    assert!(texts.iter().any(|text| text == "f\\xffoo"), "{texts:?}");
}

#[cfg(feature = "cpp-msvc")]
#[test]
fn byte_path_accepts_invalid_utf8_in_msvc_names() {
    let ast = parse_bytes(b"?F\xffoo@@YAXXZ").unwrap();
    assert_eq!(ast.dialect(), Dialect::Msvc);
    let texts = texts_of(&ast, Kind::Identifier);
    assert!(texts.iter().any(|text| text == "F\\xffoo"), "{texts:?}");
}

#[cfg(feature = "cpp-itanium")]
#[test]
fn byte_path_keeps_valid_text_unchanged() {
    for ast in [
        parse_bytes(b"_ZN3Foo3barEv").unwrap(),
        parse("_ZN3Foo3barEv").unwrap(),
    ] {
        let texts = texts_of(&ast, Kind::Identifier);
        assert!(texts.iter().any(|text| text == "Foo"), "{texts:?}");
        assert!(texts.iter().any(|text| text == "bar"), "{texts:?}");
    }
}

/// The C++ symbols the budget tests parse, with the scheme each reports.
fn budget_symbols() -> alloc::vec::Vec<(&'static str, Scheme)> {
    alloc::vec![
        #[cfg(feature = "cpp-itanium")]
        ("_Z1fPi", Scheme::CppItanium),
        #[cfg(feature = "cpp-msvc")]
        ("?bar@Foo@@QEAAXXZ", Scheme::CppMsvc),
        #[cfg(feature = "cpp-gnu-v2")]
        ("foo__Fiv", Scheme::CppGnuV2),
        #[cfg(feature = "cpp-gnu-v2")]
        ("__ct__3fooFv", Scheme::CppArm),
    ]
}

/// One dimension's ceiling from the policy table.
fn ceiling(dimension: Option<Limit>) -> usize {
    match dimension {
        Some(Limit::Bounded(value)) => value,
        _ => panic!("the scheme must bound this dimension"),
    }
}

/// A dimension left unset keeps the parser's compiled-in limit, so the
/// public entry points are exactly the pre-budget behaviour.
#[test]
fn the_default_budget_keeps_the_compiled_in_limits() {
    #[cfg(feature = "cpp-itanium")]
    {
        let limits = ParseLimits::itanium(DEFAULT_BUDGET);
        assert_eq!(limits.input(), MAX_INPUT_LEN);
        assert_eq!(limits.depth(), parser::MAX_DEPTH);
        assert_eq!(limits.work(), DEFAULT_WORK);
        assert_eq!(limits.nodes(), parser::MAX_NODES);
        assert_eq!(
            limits,
            ParseLimits::itanium(Budget::default().effective(Scheme::CppItanium))
        );
    }
    #[cfg(feature = "cpp-msvc")]
    {
        let limits = ParseLimits::msvc(DEFAULT_BUDGET);
        assert_eq!(limits.input(), MAX_INPUT_LEN);
        assert_eq!(limits.depth(), msvc::MAX_DEPTH);
        assert_eq!(limits.work(), DEFAULT_WORK);
        assert_eq!(limits.nodes(), msvc::MAX_NODES);
        assert_eq!(
            limits,
            ParseLimits::msvc(Budget::default().effective(Scheme::CppMsvc))
        );
    }
    #[cfg(feature = "cpp-gnu-v2")]
    {
        let limits = ParseLimits::legacy(DEFAULT_BUDGET);
        assert_eq!(limits.input(), MAX_INPUT_LEN);
        assert_eq!(limits.depth(), legacy::MAX_DEPTH);
        assert_eq!(limits.work(), DEFAULT_WORK);
        assert_eq!(limits.nodes(), legacy::MAX_NODES);
        for scheme in [
            Scheme::CppGnuV2,
            Scheme::CppArm,
            Scheme::CppEdg,
            Scheme::CppHp,
        ] {
            assert_eq!(
                limits,
                ParseLimits::legacy(Budget::default().effective(scheme)),
                "{scheme}"
            );
        }
    }
}

/// The public entry point parses exactly what the default budget parses.
#[test]
fn public_parse_matches_the_default_budget() {
    for (symbol, scheme) in budget_symbols() {
        let public = parse(symbol).unwrap_or_else(|error| panic!("{symbol}: {error}"));
        let budgeted = parse_bytes_with_budget(symbol.as_bytes(), Budget::new().effective(scheme))
            .unwrap_or_else(|error| panic!("{symbol}: {error}"));
        assert_eq!(public.dialect(), budgeted.dialect(), "{symbol}");
        assert_eq!(format!("{public}"), format!("{budgeted}"), "{symbol}");
        let effective = Budget::new().effective(scheme);
        assert_eq!(
            public.print_envelope_ok(effective),
            budgeted.print_envelope_ok(effective),
            "{symbol}"
        );
    }
}

/// Every parser dimension rejects one unit below the value a symbol needs
/// and accepts at that value.
#[test]
fn lowering_each_dimension_rejects_at_the_boundary() {
    fn assert_boundary(
        symbol: &str,
        scheme: Scheme,
        name: &str,
        expected: ParseErrorKind,
        limit: usize,
        budget: &dyn Fn(usize) -> Budget,
    ) {
        let needed = (0..=limit)
            .find(|&value| {
                parse_bytes_with_budget(symbol.as_bytes(), budget(value).effective(scheme)).is_ok()
            })
            .unwrap_or_else(|| panic!("{symbol}: no {name} within {limit} parses"));
        assert!(needed > 0, "{symbol}: {name} is not bounded");
        let error =
            parse_bytes_with_budget(symbol.as_bytes(), budget(needed - 1).effective(scheme))
                .unwrap_err();
        assert_eq!(
            error.kind(),
            expected,
            "{symbol}: {name} accepted at {} (needs {needed})",
            needed - 1
        );
    }

    for (symbol, scheme) in budget_symbols() {
        let hard = scheme.hard_budget();
        assert_boundary(
            symbol,
            scheme,
            "input",
            ParseErrorKind::TooLong,
            ceiling(hard.input_bytes),
            &|value| Budget::new().with_input_bytes(value),
        );
        assert_boundary(
            symbol,
            scheme,
            "depth",
            ParseErrorKind::TooDeep,
            ceiling(hard.parse_depth),
            &|value| Budget::new().with_parse_depth(value),
        );
        assert_boundary(
            symbol,
            scheme,
            "work",
            ParseErrorKind::TooMuchWork,
            ceiling(hard.work),
            &|value| Budget::new().with_work(value),
        );
        assert_boundary(
            symbol,
            scheme,
            "nodes",
            ParseErrorKind::TooManyNodes,
            ceiling(hard.nodes),
            &|value| Budget::new().with_nodes(value),
        );
    }
}

/// A symbol the default budget rejects for its length is accepted once the
/// caller raises the input budget within the hard cap.
#[test]
#[cfg(feature = "cpp-itanium")]
fn raising_the_input_budget_accepts_longer_symbols() {
    // One long source name: few nodes, so only the input budget binds.
    let symbol = long_identifier_symbol();
    assert_eq!(parse(&symbol).unwrap_err().kind(), ParseErrorKind::TooLong);

    let budget = Budget::new().with_input_bytes(2 * MAX_INPUT_LEN);
    let ast = parse_bytes_with_budget(symbol.as_bytes(), budget.effective(Scheme::CppItanium))
        .expect("raised input budget");
    assert_eq!(ast.dialect(), Dialect::Itanium);
}

/// An Itanium symbol whose only large dimension is one source name.
#[cfg(feature = "cpp-itanium")]
fn long_identifier_symbol() -> alloc::string::String {
    let length = MAX_INPUT_LEN + 4 * 1024;
    let mut symbol = alloc::string::String::from("_ZN");
    symbol.push_str(&alloc::string::ToString::to_string(&length));
    symbol.push_str(&"a".repeat(length));
    symbol.push_str("3barEv");
    symbol
}

/// Raising the node budget accepts a wider tree; raising the input budget
/// alone does not, because the two dimensions are independent.
#[test]
#[cfg(feature = "cpp-itanium")]
fn raising_the_node_budget_accepts_wider_trees() {
    let symbol = wide_parameter_symbol();
    assert_eq!(
        parse(&symbol).unwrap_err().kind(),
        ParseErrorKind::TooManyNodes
    );

    let raised = Budget::new().with_nodes(0x8000);
    let ast = parse_bytes_with_budget(symbol.as_bytes(), raised.effective(Scheme::CppItanium))
        .expect("raised node budget");
    assert_eq!(ast.dialect(), Dialect::Itanium);
}

/// An Itanium function with more parameters than the default node budget
/// allows.
#[cfg(feature = "cpp-itanium")]
fn wide_parameter_symbol() -> alloc::string::String {
    let mut symbol = alloc::string::String::from("_Z1f");
    symbol.push_str(&"i".repeat(10_000));
    symbol
}

/// Over-hard values are clamped, visibly and in the parser limits.
#[test]
fn over_hard_budgets_are_clamped_to_the_hard_caps() {
    let budget = Budget::new()
        .with_input_bytes(usize::MAX)
        .with_parse_depth(usize::MAX)
        .with_work(usize::MAX)
        .with_nodes(usize::MAX);
    let itanium = budget.effective(Scheme::CppItanium);
    assert_eq!(itanium.input_bytes, Some(Limit::Bounded(16 * 1024 * 1024)));
    assert_eq!(itanium.parse_depth, Some(Limit::Bounded(256)));
    assert_eq!(itanium.work, Some(Limit::Bounded(64 * 1024 * 1024)));
    assert_eq!(itanium.nodes, Some(Limit::Bounded(1024 * 1024)));
    let legacy = budget.effective(Scheme::CppGnuV2);
    assert_eq!(legacy.parse_depth, Some(Limit::Bounded(64)));

    #[cfg(feature = "cpp-itanium")]
    {
        let limits = ParseLimits::itanium(itanium);
        assert_eq!(limits.input(), 16 * 1024 * 1024);
        // The depth ceiling is the compiled-in depth budget; the demangler
        // applies the separate structural printability gate.
        assert_eq!(limits.depth(), parser::MAX_DEPTH);
        assert_eq!(limits.work(), 64 * 1024 * 1024);
        assert_eq!(limits.nodes(), 1024 * 1024);
    }
    #[cfg(feature = "cpp-gnu-v2")]
    {
        let limits = ParseLimits::legacy(legacy);
        assert_eq!(limits.depth(), legacy::MAX_DEPTH);
        assert_eq!(limits.nodes(), 1024 * 1024);
    }
}

/// The print envelope stays fixed policy: raising the node budget parses a
/// wider tree, but the demangler still rejects one the printers cannot
/// render.
#[test]
#[cfg(feature = "cpp-itanium")]
fn the_print_envelope_is_not_a_budget() {
    let symbol = wide_parameter_symbol();
    let budget = Budget::new().with_nodes(0x8000).with_expansion_visits(1);
    let effective = budget.effective(Scheme::CppItanium);
    let ast = parse_bytes_with_budget(symbol.as_bytes(), effective).expect("raised node budget");
    assert!(!ast.print_envelope_ok(effective));
    assert!(
        Demangler::with_budget(budget)
            .demangle_with_scheme(Scheme::CppItanium, &symbol)
            .is_err()
    );
}

/// Zero at a migrated dimension rejects through the forced-scheme and
/// `Selection` paths.
#[test]
fn a_zero_input_budget_rejects_through_demangler_and_selection() {
    let demangler = Demangler::with_budget(Budget::new().with_input_bytes(0));
    for (symbol, scheme) in budget_symbols() {
        assert!(
            demangler.demangle_with_scheme(scheme, symbol).is_err(),
            "{symbol}"
        );
        assert!(
            demangler
                .selection()
                .allow(scheme)
                .demangle(symbol)
                .is_err(),
            "{symbol}"
        );
    }
}

/// A configured budget reaches the parser through the detected path too.
///
/// Detection parses a legacy name and retains
/// the tree, so the demangler re-parses it under the configured budget
/// instead of reusing a tree that may exceed it.
#[test]
fn a_configured_budget_reaches_the_detected_path() {
    for (symbol, scheme) in budget_symbols() {
        assert!(Demangler::new().demangle(symbol).is_ok(), "{symbol}");
        let tightened = Demangler::with_budget(Budget::new().with_parse_depth(0));
        assert!(tightened.demangle(symbol).is_err(), "{symbol} {scheme}");
    }
}

/// Raising the input budget through the demangler, not only through the
/// backend parse helper.
#[test]
#[cfg(feature = "cpp-itanium")]
fn demangler_raising_the_input_budget_accepts_longer_symbols() {
    let symbol = long_identifier_symbol();
    assert!(Demangler::new().demangle(&symbol).is_err());
    let raised = Demangler::with_budget(Budget::new().with_input_bytes(2 * MAX_INPUT_LEN));
    assert!(raised.demangle(&symbol).is_ok());
}
