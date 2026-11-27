use alloc::format;
use alloc::string::String;

use super::*;
use crate::{Budget, Scheme};

/// Parse `symbol` with `budget` resolved for `scheme`.
fn parse_with(symbol: &[u8], budget: Budget, scheme: Scheme) -> Result<Ast<'_>, fmt::Error> {
    parse_bytes_with_budget(symbol, budget.effective(scheme))
}

/// Whether `symbol` parses with one budget dimension lowered to `value`.
fn parses_with(
    symbol: &[u8],
    scheme: Scheme,
    dimension: impl Fn(usize) -> Budget,
    value: usize,
) -> bool {
    parse_with(symbol, dimension(value), scheme).is_ok()
}

/// Assert that `need` is the exact boundary of one dimension for `symbol`.
///
/// The dimension is the only override, so a failure one unit below the
/// boundary comes from that dimension and not from another limit.
fn assert_dimension_boundary(
    symbol: &str,
    scheme: Scheme,
    name: &str,
    dimension: impl Fn(usize) -> Budget,
) {
    let bytes = symbol.as_bytes();
    let ceiling = bytes.len() + 64;
    let need = (0..=ceiling)
        .find(|value| parses_with(bytes, scheme, &dimension, *value))
        .unwrap_or_else(|| panic!("{symbol}: no {name} budget parses"));
    assert!(need > 0, "{symbol}: {name} boundary is zero");
    assert!(
        parses_with(bytes, scheme, &dimension, need),
        "{symbol}: {name} budget {need} must accept"
    );
    assert!(
        !parses_with(bytes, scheme, &dimension, need - 1),
        "{symbol}: {name} budget {} must reject",
        need - 1
    );
}

#[test]
fn defaults_match_the_compiled_in_limits() {
    let limits = ParseLimits::from_effective(DEFAULT_BUDGET);
    assert_eq!(limits.input, MAX_INPUT_LEN);
    assert_eq!(limits.depth, MAX_DEPTH);
    assert_eq!(limits.work, WORK_BUDGET);
    assert_eq!(limits.nodes, MAX_NODES);
    assert_eq!(limits.expansion_visits, node::DEFAULT_EXPANSION_VISITS);
    assert_eq!(limits.expansion_bytes, node::DEFAULT_EXPANSION_BYTES);
    for scheme in [Scheme::SwiftStable, Scheme::SwiftLegacy] {
        assert_eq!(scheme.default_budget(), DEFAULT_BUDGET, "{scheme}");
    }
}

#[test]
fn each_dimension_rejects_one_unit_below_what_the_symbol_needs() {
    // One stable and one legacy symbol, so both grammars are covered.
    for (symbol, scheme) in [
        ("$s4main3fooyySiF", Scheme::SwiftStable),
        ("_TtC5sugar7MyClass", Scheme::SwiftLegacy),
    ] {
        assert_dimension_boundary(symbol, scheme, "input", |value| {
            Budget::new().with_input_bytes(value)
        });
        assert_dimension_boundary(symbol, scheme, "depth", |value| {
            Budget::new().with_parse_depth(value)
        });
        assert_dimension_boundary(symbol, scheme, "work", |value| {
            Budget::new().with_work(value)
        });
        assert_dimension_boundary(symbol, scheme, "nodes", |value| {
            Budget::new().with_nodes(value)
        });
    }
}

/// Assert that `estimate` is the exact boundary of one expansion gate.
///
/// The gate is the parser's own envelope, so the estimate is accepted and
/// one unit less is rejected. The dimension is the only override.
fn assert_expansion_boundary(
    symbol: &str,
    scheme: Scheme,
    name: &str,
    estimate: usize,
    dimension: impl Fn(usize) -> Budget,
) {
    assert!(
        parses_with(symbol.as_bytes(), scheme, &dimension, estimate),
        "{symbol}: {name} gate {estimate} must accept"
    );
    assert!(
        !parses_with(symbol.as_bytes(), scheme, &dimension, estimate - 1),
        "{symbol}: {name} gate {} must reject",
        estimate - 1
    );
}

#[test]
fn expansion_gates_accept_the_estimate_and_reject_one_unit_below() {
    // Both grammars, one shared-node symbol and one maximal `IndexSubset`,
    // whose text expands when it prints.
    let base = "$s13TangentVector16_Differentiation14DifferentiablePQzAaDQy_SdAFIegnnnr_TJSd";
    let long_subset = format!("{base}{}pSrSUSP", "S".repeat(64));
    for (symbol, scheme) in [
        ("$s4main3fooyySiF", Scheme::SwiftStable),
        ("_TtC5sugar7MyClass", Scheme::SwiftLegacy),
        ("$s4main1AVSDyACAC_G_ACt", Scheme::SwiftStable),
        (long_subset.as_str(), Scheme::SwiftStable),
    ] {
        let ast = parse_with(symbol.as_bytes(), Budget::new(), scheme)
            .unwrap_or_else(|error| panic!("{symbol}: {error:?}"));
        let root = ast.root();
        assert_expansion_boundary(symbol, scheme, "visits", root.print_visits(), |value| {
            Budget::new().with_expansion_visits(value)
        });
        assert_expansion_boundary(symbol, scheme, "bytes", root.print_bytes(), |value| {
            Budget::new().with_expansion_bytes(value)
        });
    }
}

#[test]
fn raising_the_input_budget_accepts_a_longer_symbol() {
    // One byte past the default cap, in the unmangled suffix.
    let base = "$s4main3fooyySiF.";
    let rest = base.len() - 2;
    let symbol = format!("{base}{}", "x".repeat(MAX_INPUT_LEN - rest + 1));
    assert!(parse_bytes(symbol.as_bytes()).is_err());
    let raised = Budget::new().with_input_bytes(2 * MAX_INPUT_LEN);
    assert!(parse_with(symbol.as_bytes(), raised, Scheme::SwiftStable).is_ok());

    // The legacy grammar reads the same dimension.
    let legacy = format!("_TtSi{}", "x".repeat(MAX_INPUT_LEN));
    assert!(parse_bytes(legacy.as_bytes()).is_err());
    assert!(parse_with(legacy.as_bytes(), raised, Scheme::SwiftLegacy).is_ok());
}

#[test]
fn raising_the_parse_depth_accepts_a_deeper_tree() {
    // `h` is an iterative shared-type wrapper and `PA__T` nests the legacy
    // parser's recursion: each adds one level, so both chains exercise the
    // depth dimension only.
    for (symbol, scheme) in [
        (
            format!("$sSi{}", "h".repeat(usize::from(MAX_DEPTH) + 1)),
            Scheme::SwiftStable,
        ),
        (
            format!("_TPA__T{}F3foog3barSi", "PA__T".repeat(100)),
            Scheme::SwiftLegacy,
        ),
    ] {
        assert!(
            parse_bytes(symbol.as_bytes()).is_err(),
            "{symbol}: the default depth must reject"
        );
        let raised = Budget::new().with_parse_depth(usize::MAX);
        assert!(
            parse_with(symbol.as_bytes(), raised, scheme).is_ok(),
            "{symbol}: the hard depth cap must accept"
        );
    }
}

#[test]
fn the_hard_depth_cap_fits_a_modest_stack() {
    // 512 KiB is the stack budget the crate's other recursive parser (D) is
    // tested with. The deepest legacy chain the print envelope accepts is
    // found under that stack, so raising the depth cap cannot recurse past
    // what the parser can safely build.
    let budget = Budget::new().with_parse_depth(usize::MAX);
    let deepest = std::thread::Builder::new()
        .stack_size(512 * 1024)
        .spawn(move || {
            (0..=usize::from(HARD_DEPTH)).rfind(|levels| {
                let symbol = format!("_TPA__T{}F3foog3barSi", "PA__T".repeat(*levels));
                parse_with(symbol.as_bytes(), budget, Scheme::SwiftLegacy).is_ok()
            })
        })
        .expect("spawn")
        .join()
        .expect("the depth cap must not overflow a 512 KiB stack");
    assert!(
        deepest.is_some_and(|levels| levels > usize::from(MAX_DEPTH)),
        "the raised cap must reach deeper than the default: {deepest:?}"
    );
}

#[test]
fn clamping_keeps_the_hard_ceilings() {
    let limits = ParseLimits::from_effective(
        Budget::new()
            .with_input_bytes(usize::MAX)
            .with_parse_depth(usize::MAX)
            .with_work(usize::MAX)
            .with_nodes(usize::MAX)
            .with_expansion_visits(usize::MAX)
            .with_expansion_bytes(usize::MAX)
            .effective(Scheme::SwiftStable),
    );
    assert_eq!(limits.input, HARD_INPUT_LEN);
    assert_eq!(limits.depth, HARD_DEPTH);
    assert_eq!(limits.work, HARD_WORK);
    assert_eq!(limits.nodes, HARD_NODES);
    assert_eq!(limits.expansion_visits, node::HARD_EXPANSION_VISITS);
    assert_eq!(limits.expansion_bytes, node::HARD_EXPANSION_BYTES);

    // A chain past the hard depth stays rejected even when the caller asks
    // for more.
    let deeper = format!("$sSi{}", "h".repeat(usize::from(HARD_DEPTH) + 1));
    let unbounded = Budget::new().with_parse_depth(usize::MAX);
    assert!(parse_with(deeper.as_bytes(), unbounded, Scheme::SwiftStable).is_err());
}

#[test]
fn the_ast_printability_ceiling_is_not_a_budget() {
    // Every parser and expansion dimension raised to its hard ceiling. A
    // chain of `n` shared wrappers nests `n + 3` levels in the tree, so the
    // fixed ceiling admits one wrapper less than the depth cap allows and
    // rejects the next one at parse time.
    let all_raised = Budget::new()
        .with_input_bytes(usize::MAX)
        .with_parse_depth(usize::MAX)
        .with_work(usize::MAX)
        .with_nodes(usize::MAX)
        .with_expansion_visits(usize::MAX)
        .with_expansion_bytes(usize::MAX);
    let scheme = Scheme::SwiftStable;
    let ceiling = usize::from(MAX_PRINT_DEPTH) - 3;
    let accepted = format!("$sSi{}", "h".repeat(ceiling));
    assert!(parse_with(accepted.as_bytes(), all_raised, scheme).is_ok());
    let rejected = format!("$sSi{}", "h".repeat(ceiling + 1));
    assert!(parse_with(rejected.as_bytes(), all_raised, scheme).is_err());
}

#[test]
fn the_default_budget_parses_the_corpus_like_parse() {
    let base = env!("CARGO_MANIFEST_DIR");
    if !std::path::Path::new(base).join("refs/swift").is_dir() {
        std::eprintln!("refs/swift is absent; skipping the Swift budget corpus parity check");
        return;
    }
    let mut compared = 0_usize;
    for file in [
        "refs/swift/manglings.txt",
        "refs/swift/toolchain-6.4.2-manglings.txt",
    ] {
        let Ok(data) = std::fs::read_to_string(format!("{base}/{file}")) else {
            continue;
        };
        for line in data.lines() {
            let symbol = line.split(" ---> ").next().unwrap_or_default().trim();
            if symbol.is_empty() || symbol.starts_with('#') {
                continue;
            }
            let expected = parse(symbol);
            let explicit = parse_bytes_with_budget(symbol.as_bytes(), DEFAULT_BUDGET);
            assert_eq!(
                expected.is_ok(),
                explicit.is_ok(),
                "{file}: {symbol}: the default budget changes acceptance"
            );
            let Ok(expected) = expected else {
                continue;
            };
            let mut want = String::new();
            let mut got = String::new();
            expected.print(&mut want).unwrap();
            explicit.unwrap().print(&mut got).unwrap();
            assert_eq!(want, got, "{file}: {symbol}");
            compared += 1;
        }
    }
    assert!(compared > 0, "the default budget compared no symbols");
}
