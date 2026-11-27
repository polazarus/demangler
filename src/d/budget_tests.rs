use alloc::format;
use alloc::string::{String, ToString};

use super::node::{HARD_DEPTH, HARD_INPUT_LEN, HARD_NODES, HARD_WORK, ParseLimits};
use super::*;
use crate::budget::Limit;
use crate::{Budget, Demangler, Error, Scheme};

/// Parse `symbol` with `budget` resolved for [`Scheme::D`].
fn parse_with(symbol: &[u8], budget: Budget) -> Result<Ast<'_>, Failure> {
    parse_bytes_with_budget(symbol, budget.effective(Scheme::D))
}

/// Whether `symbol` parses with one budget dimension set to `value`.
fn parses_with(symbol: &[u8], dimension: impl Fn(usize) -> Budget, value: usize) -> bool {
    parse_with(symbol, dimension(value)).is_ok()
}

/// Assert that `need` is the exact boundary of one dimension for `symbol`.
///
/// The dimension is the only override, so a failure one unit below the
/// boundary comes from that dimension and not from another limit.
fn assert_dimension_boundary(
    symbol: &str,
    name: &str,
    ceiling: usize,
    dimension: impl Fn(usize) -> Budget,
) {
    let bytes = symbol.as_bytes();
    let need = (0..=ceiling)
        .find(|value| parses_with(bytes, &dimension, *value))
        .unwrap_or_else(|| panic!("{symbol}: no {name} budget parses"));
    assert!(need > 0, "{symbol}: the {name} boundary is zero");
    assert!(
        parses_with(bytes, &dimension, need),
        "{symbol}: {name} budget {need} must accept"
    );
    assert!(
        !parses_with(bytes, &dimension, need - 1),
        "{symbol}: {name} budget {} must reject",
        need - 1
    );
}

/// A symbol whose only printed component is `name_len` bytes long.
fn long_name(name_len: usize) -> String {
    format!("_D{name_len}{}Z", "a".repeat(name_len))
}

#[test]
fn the_compiled_in_budget_matches_the_policy_defaults() {
    let limits = ParseLimits::from_effective(DEFAULT_BUDGET);
    assert_eq!(limits.input, MAX_INPUT_LEN);
    assert_eq!(limits.depth, MAX_DEPTH);
    assert_eq!(limits.work, MAX_WORK);
    assert_eq!(limits.nodes, MAX_NODES);
    assert_eq!(Scheme::D.default_budget(), DEFAULT_BUDGET);

    // The policy table and this module agree on the ceilings, and every
    // hard cap still fits the shared cursor's narrower fields.
    let hard = Scheme::D.hard_budget();
    assert_eq!(hard.input_bytes, Some(Limit::Bounded(HARD_INPUT_LEN)));
    assert_eq!(hard.parse_depth, Some(Limit::Bounded(HARD_DEPTH)));
    assert_eq!(hard.work, Some(Limit::Bounded(HARD_WORK)));
    assert_eq!(hard.nodes, Some(Limit::Bounded(HARD_NODES)));

    let hard_limits = ParseLimits::from_effective(hard);
    let cursor = hard_limits.cursor();
    assert_eq!(usize::try_from(cursor.input).unwrap(), HARD_INPUT_LEN);
    assert_eq!(usize::from(cursor.depth), HARD_DEPTH);
    assert_eq!(usize::try_from(cursor.work).unwrap(), HARD_WORK);
}

#[test]
fn the_default_budget_matches_the_public_parse() {
    for symbol in [
        "_Dmain",
        "_D3foo3barFZv",
        "_D3foo3barFAiG5iHAyaiZi",
        "_D3app__T3BoxTiZQh3getMFZi",
        "_DThn16_3foo3barFZv",
        "_D4test3Foo6__initZ",
        "_D3foo3barFDFZvZ1aMQh",
        "_D",
        "_D3foo3bar",
    ] {
        let expected = parse_bytes_with_budget(symbol.as_bytes(), DEFAULT_BUDGET);
        let explicit = parse_with(symbol.as_bytes(), Budget::new());
        assert_eq!(
            expected.is_ok(),
            explicit.is_ok(),
            "{symbol}: the default budget changes acceptance"
        );
        if let (Ok(expected), Ok(explicit)) = (expected, explicit) {
            assert_eq!(expected.to_string(), explicit.to_string(), "{symbol}");
        }
    }
}

#[test]
fn each_dimension_rejects_one_unit_below_what_the_symbol_needs() {
    // `_Dmain` is recognised before the grammar runs, so it charges no
    // work, depth or nodes and has no non-zero boundary to test.
    for symbol in [
        "_D3foo3barFZv",
        "_D3foo3barFAiG5iHAyaiZi",
        "_D3app__T3BoxTiZQh3getMFZi",
    ] {
        let ceiling = symbol.len() + 8;
        assert_dimension_boundary(symbol, "input", ceiling, |value| {
            Budget::new().with_input_bytes(value)
        });
        assert_dimension_boundary(symbol, "work", 256, |value| Budget::new().with_work(value));
        assert_dimension_boundary(symbol, "nodes", 16, |value| Budget::new().with_nodes(value));
    }

    // A type, a qualified name and a template each enter the depth
    // dimension once.
    assert_dimension_boundary("_D3foo3barFZv", "depth", 8, |value| {
        Budget::new().with_parse_depth(value)
    });
    assert_dimension_boundary("_D3app__T3BoxTiZQh3getMFZi", "depth", 16, |value| {
        Budget::new().with_parse_depth(value)
    });
}

#[test]
fn raising_the_input_budget_accepts_a_longer_symbol() {
    let name_len = MAX_INPUT_LEN + 1024;
    let symbol = long_name(name_len);
    assert!(matches!(
        parse_with(symbol.as_bytes(), Budget::new()),
        Err(Failure::BudgetExceeded)
    ));

    let raised = Budget::new().with_input_bytes(HARD_INPUT_LEN);
    let ast = parse_with(symbol.as_bytes(), raised).expect("raised input budget");
    assert_eq!(ast.to_string().len(), name_len);
}

#[test]
fn raising_the_depth_budget_accepts_a_deeper_type() {
    let depth = MAX_DEPTH + 8;
    let symbol = format!("_D3foo3barF{}iZv", "P".repeat(depth));
    assert!(matches!(
        parse_with(symbol.as_bytes(), Budget::new()),
        Err(Failure::RecursedTooDeep)
    ));

    let raised = Budget::new().with_parse_depth(depth + 2);
    assert!(parse_with(symbol.as_bytes(), raised).is_ok());
}

#[test]
fn raising_the_node_budget_accepts_a_wider_signature() {
    // Each `i` parameter is one node, so the signature needs more nodes
    // than the default allowance while staying far below the hard input
    // ceiling.
    let params = MAX_NODES + 8;
    let mut symbol = String::from("_D3foo3barF");
    symbol.push_str(&"i".repeat(params));
    symbol.push_str("Zv");
    assert!(symbol.len() < HARD_INPUT_LEN);

    // The default input cap rejects it first, so raise the input and show
    // that the node allowance is an independent dimension.
    let input_only = Budget::new().with_input_bytes(HARD_INPUT_LEN);
    assert!(matches!(
        parse_with(symbol.as_bytes(), input_only),
        Err(Failure::BudgetExceeded)
    ));

    let nodes = params + 8;
    assert!(parse_with(symbol.as_bytes(), input_only.with_nodes(nodes)).is_ok());
}

/// The input and work dimensions are independent: raising the input alone
/// keeps the default work allowance, so a scan that exceeds it still fails.
#[test]
fn raising_the_input_budget_does_not_raise_the_work_budget() {
    // Every byte of the identifier is scanned and charged once, so this
    // symbol needs more work than the default allowance.
    let symbol = long_name(MAX_WORK + 1024);
    assert!(symbol.len() < HARD_INPUT_LEN);

    let input_only = Budget::new().with_input_bytes(HARD_INPUT_LEN);
    assert!(matches!(
        parse_with(symbol.as_bytes(), input_only),
        Err(Failure::BudgetExceeded)
    ));

    let work = symbol.len() + 64;
    assert!(parse_with(symbol.as_bytes(), input_only.with_work(work)).is_ok());
}

#[test]
fn clamping_keeps_the_hard_ceilings() {
    let budget = Budget::new()
        .with_input_bytes(usize::MAX)
        .with_parse_depth(usize::MAX)
        .with_work(usize::MAX)
        .with_nodes(usize::MAX);
    let effective = budget.effective(Scheme::D);
    assert_eq!(effective.input_bytes, Some(Limit::Bounded(HARD_INPUT_LEN)));
    assert_eq!(effective.parse_depth, Some(Limit::Bounded(HARD_DEPTH)));
    assert_eq!(effective.work, Some(Limit::Bounded(HARD_WORK)));
    assert_eq!(effective.nodes, Some(Limit::Bounded(HARD_NODES)));

    let limits = ParseLimits::from_effective(effective);
    assert_eq!(limits.input, HARD_INPUT_LEN);
    assert_eq!(limits.depth, HARD_DEPTH);
    assert_eq!(limits.work, HARD_WORK);
    assert_eq!(limits.nodes, HARD_NODES);

    // A symbol past the hard input cap stays rejected even when the caller
    // asks for more.
    let too_long = long_name(HARD_INPUT_LEN);
    assert!(matches!(
        parse_with(too_long.as_bytes(), budget),
        Err(Failure::BudgetExceeded)
    ));
}

#[test]
fn a_zero_budget_rejects_through_every_entry_point() {
    // A qualified name with a signature charges work, enters depth and
    // builds nodes, so every dimension rejects this symbol.
    let symbol = "_D3foo3barFZv";
    assert!(Demangler::new().demangle(symbol).is_ok());
    for budget in [
        Budget::new().with_input_bytes(0),
        Budget::new().with_parse_depth(0),
        Budget::new().with_work(0),
        Budget::new().with_nodes(0),
    ] {
        let demangler = Demangler::with_budget(budget);
        // Detection may have classified with its default fallback; the
        // configured budget rejects the re-parse rather than
        // misclassifying the symbol as plain C.
        assert!(
            matches!(demangler.demangle(symbol).unwrap_err(), Error::Parse { scheme: Scheme::D, error: err_detail } if err_detail.budget_kind().is_some())
        );
        assert!(matches!(demangler
                .demangle_with_scheme(Scheme::D, symbol)
                .unwrap_err(), Error::Parse { scheme: Scheme::D, error: err_detail } if err_detail.budget_kind().is_some()));
        assert!(matches!(demangler
                .selection()
                .allow(Scheme::D)
                .demangle(symbol)
                .unwrap_err(), Error::Parse { scheme: Scheme::D, error: err_detail } if err_detail.budget_kind().is_some()));
    }
}

/// Printing is not a budget: the printer walks each owned node once and has
/// no output cap, so a short symbol whose spelling expands (an `immutable`
/// modifier prints ten bytes) still renders completely.
#[test]
fn the_printer_has_no_output_cap() {
    // Twenty thousand `immutable int` parameters print past four times
    // the default input cap.
    let mut symbol = String::from("_D3foo3barF");
    symbol.push_str(&"yi".repeat(20_000));
    symbol.push_str("Zv");
    assert!(symbol.len() < MAX_INPUT_LEN);

    let ast = parse_bytes_with_budget(symbol.as_bytes(), DEFAULT_BUDGET)
        .expect("the parser accepts the signature");
    let mut out = String::new();
    ast.print(&mut out).expect("the printer has no cap");
    assert!(out.len() > 4 * MAX_INPUT_LEN);
    assert!(Demangler::new().demangle(&symbol).is_ok());
}

/// A symbol accepted under a raised input budget prints completely.
#[test]
fn a_raised_input_budget_prints_completely() {
    let name_len = MAX_INPUT_LEN + 1024;
    let symbol = long_name(name_len);
    let raised = Budget::new().with_input_bytes(HARD_INPUT_LEN);
    let rendered = parse_with(symbol.as_bytes(), raised)
        .expect("the raised budget parses")
        .to_string();
    assert_eq!(rendered.len(), name_len);
}

#[test]
fn the_default_budget_parses_the_corpus_like_parse() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("refs/d");
    if !root.is_dir() {
        std::eprintln!("refs/d is absent; skipping the D budget corpus parity check");
        return;
    }

    let mut compared = 0_usize;
    for corpus in ["hand.tsv", "ldc-1.40-app.tsv", "libphobos-2.110.0.tsv"] {
        let path = root.join(corpus);
        let data = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()));
        for line in data.lines() {
            let symbol = line.split('\t').next().unwrap_or_default();
            if symbol.is_empty() || symbol.starts_with('#') {
                continue;
            }
            let expected = parse_bytes_with_budget(symbol.as_bytes(), DEFAULT_BUDGET);
            let explicit = parse_with(symbol.as_bytes(), Budget::new());
            assert_eq!(
                expected.is_ok(),
                explicit.is_ok(),
                "{symbol}: the default budget changes acceptance"
            );
            let (Ok(expected), Ok(explicit)) = (expected, explicit) else {
                continue;
            };
            assert_eq!(expected.to_string(), explicit.to_string(), "{symbol}");
            compared += 1;
        }
    }
    assert!(compared > 0, "the default budget compared no symbols");
}
