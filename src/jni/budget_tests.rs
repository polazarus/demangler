use alloc::format;
use alloc::string::{String, ToString};

use super::node::{HARD_DEPTH, HARD_INPUT_LEN, HARD_WORK, ParseLimits};
use super::*;
use crate::budget::Limit;
use crate::{Budget, Demangler, Scheme};

/// Parse `symbol` with `budget` resolved for [`Scheme::Jni`].
fn parse_with(symbol: &[u8], budget: Budget) -> Result<Ast<'_>, crate::ParseError> {
    parse_bytes_with_budget(symbol, budget.effective(Scheme::Jni))
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

/// A long-form name whose single argument is an `dimensions`-deep array.
fn array_of(dimensions: usize) -> String {
    let mut symbol = String::from("Java_p_T_baz__");
    for _ in 0..dimensions {
        symbol.push_str("_3");
    }
    symbol.push('I');
    symbol
}

#[test]
fn the_compiled_in_budget_matches_the_policy_defaults() {
    let limits = ParseLimits::from_effective(DEFAULT_BUDGET);
    assert_eq!(limits.input, MAX_INPUT_LEN);
    assert_eq!(limits.depth, MAX_DEPTH);
    assert_eq!(limits.work, MAX_WORK);
    assert_eq!(Scheme::Jni.default_budget(), DEFAULT_BUDGET);

    // The policy table and this module agree on the ceilings. One name
    // holds one argument list, so there is no node dimension.
    let hard = Scheme::Jni.hard_budget();
    assert_eq!(hard.input_bytes, Some(Limit::Bounded(HARD_INPUT_LEN)));
    assert_eq!(hard.parse_depth, Some(Limit::Bounded(HARD_DEPTH)));
    assert_eq!(hard.work, Some(Limit::Bounded(HARD_WORK)));
    assert_eq!(hard.nodes, Some(Limit::Unbounded));
    assert_eq!(HARD_DEPTH, 1024);
}

#[test]
fn each_dimension_rejects_one_unit_below_what_the_symbol_needs() {
    for symbol in [
        "Java_com_example_Foo_plain",
        "Java_com_example_Foo_bar__",
        "Java_com_example_Foo_bar___3Ljava_lang_String_2I",
        "Java_p_T_baz_0d835_0dd38",
    ] {
        let ceiling = symbol.len() * 2 + 64;
        assert_dimension_boundary(symbol, "input", ceiling, |value| {
            Budget::new().with_input_bytes(value)
        });
        assert_dimension_boundary(symbol, "work", ceiling, |value| {
            Budget::new().with_work(value)
        });
    }

    // Every `_3` is one array dimension.
    assert_dimension_boundary(&array_of(3), "depth", 8, |value| {
        Budget::new().with_parse_depth(value)
    });
}

#[test]
fn raising_the_input_budget_accepts_a_longer_symbol() {
    let mut symbol = String::from("Java_p_T_");
    symbol.push_str(&"a".repeat(MAX_INPUT_LEN));
    assert!(
        matches!(parse_with(symbol.as_bytes(), Budget::new()), Err(e) if e.budget_kind().is_some())
    );
    let raised = Budget::new().with_input_bytes(2 * MAX_INPUT_LEN);
    let ast = parse_with(symbol.as_bytes(), raised).expect("raised input budget");
    assert_eq!(ast.to_string().len(), "p.T.".len() + MAX_INPUT_LEN);
}

#[test]
fn raising_the_depth_budget_accepts_a_deeper_array() {
    // One dimension past the default array depth.
    let symbol = array_of(MAX_DEPTH + 1);
    assert!(
        matches!(parse_with(symbol.as_bytes(), Budget::new()), Err(e) if e.budget_kind() == Some(crate::BudgetKind::ParseDepth))
    );
    let raised = Budget::new().with_parse_depth(MAX_DEPTH + 1);
    let ast = parse_with(symbol.as_bytes(), raised).expect("raised depth budget");
    assert_eq!(
        ast.descriptor().expect("a long-form name").args()[0].dimensions(),
        u16::try_from(MAX_DEPTH + 1).unwrap()
    );
}

/// The depth ceiling is the generic hard cap; the AST stores the count in
/// a `u16`, so every accepted depth is representable.
#[test]
fn the_depth_ceiling_is_the_generic_hard_cap() {
    let over_hard = Budget::new().with_parse_depth(usize::MAX);
    assert_eq!(
        over_hard.effective(Scheme::Jni).parse_depth,
        Some(Limit::Bounded(1024))
    );
    let limits = ParseLimits::from_effective(over_hard.effective(Scheme::Jni));
    assert_eq!(limits.depth, 1024);

    let deepest = array_of(1024);
    let ast = parse_with(deepest.as_bytes(), over_hard).expect("1024 dimensions fit a `u16`");
    assert_eq!(
        ast.descriptor().expect("a long-form name").args()[0].dimensions(),
        1024
    );

    let too_deep = array_of(1025);
    assert!(
        matches!(parse_with(too_deep.as_bytes(), over_hard), Err(e) if e.budget_kind() == Some(crate::BudgetKind::ParseDepth))
    );
}

/// The input and work dimensions are independent: raising the input alone
/// keeps the default work allowance, so a scan that exceeds it still fails.
#[test]
fn raising_the_input_budget_does_not_raise_the_work_budget() {
    // The parser charges one work unit per input byte before splitting.
    let mut symbol = String::from("Java_p_T_baz");
    symbol.push_str(&"a".repeat(600_000));
    assert!(4 * symbol.len() > MAX_WORK);
    let work = 4 * symbol.len();

    let input_only = Budget::new().with_input_bytes(HARD_INPUT_LEN);
    assert!(
        matches!(parse_with(symbol.as_bytes(), input_only), Err(e) if e.budget_kind().is_some())
    );

    assert!(parse_with(symbol.as_bytes(), input_only.with_work(work)).is_ok());
}

#[test]
fn clamping_keeps_the_hard_ceilings() {
    let budget = Budget::new()
        .with_input_bytes(usize::MAX)
        .with_work(usize::MAX);
    let limits = ParseLimits::from_effective(budget.effective(Scheme::Jni));
    assert_eq!(limits.input, HARD_INPUT_LEN);
    assert_eq!(limits.work, HARD_WORK);
    assert_eq!(
        budget.effective(Scheme::Jni).input_bytes,
        Some(Limit::Bounded(HARD_INPUT_LEN))
    );

    // A name past the hard input cap stays rejected even when the caller
    // asks for more.
    let mut too_long = String::from("Java_p_T_");
    too_long.push_str(&"a".repeat(HARD_INPUT_LEN));
    assert!(matches!(parse_with(too_long.as_bytes(), budget), Err(e) if e.budget_kind().is_some()));
}

#[test]
fn a_zero_budget_rejects_through_every_entry_point() {
    // A long-form name charges work and carries one array dimension, so
    // every dimension rejects it.
    let symbol = "Java_p_T_baz___3I";
    assert!(Demangler::new().demangle(symbol).is_ok());
    for budget in [
        Budget::new().with_input_bytes(0),
        Budget::new().with_work(0),
        Budget::new().with_parse_depth(0),
    ] {
        let demangler = Demangler::with_budget(budget);
        assert!(demangler.demangle(symbol).is_err(), "{budget:?}");
        assert!(
            demangler.demangle_with_scheme(Scheme::Jni, symbol).is_err(),
            "{budget:?}"
        );
        assert!(
            demangler
                .selection()
                .allow(Scheme::Jni)
                .demangle(symbol)
                .is_err(),
            "{budget:?}"
        );
    }

    // A short name carries no descriptor, so the depth dimension does not
    // apply to it.
    let demangler = Demangler::with_budget(Budget::new().with_parse_depth(0));
    assert!(demangler.demangle("Java_com_example_Foo_plain").is_ok());
}

/// Printing is not a budget: the output is linear in the input.
///
/// The printer walks each node once, and the widest expansion is a
/// primitive argument (`Z` becomes `boolean, `), so sixteen times the input
/// bounds the text.
#[test]
fn the_output_is_linear_in_the_input() {
    let mut symbol = String::from("Java_p_T_baz__");
    symbol.push_str(&"Z".repeat(MAX_INPUT_LEN));
    let raised = Budget::new().with_input_bytes(2 * MAX_INPUT_LEN);
    let rendered = parse_with(symbol.as_bytes(), raised)
        .expect("the raised input budget parses")
        .to_string();
    assert!(rendered.len() <= 16 * symbol.len());
}

#[test]
fn the_default_budget_parses_the_corpus_like_parse() {
    let corpora = [
        "/refs/jni/corpus/hand.txt",
        "/refs/jni/corpus/jdk21-javac-h.txt",
        "/refs/jni/corpus/jdk21-libraries.txt",
        "/refs/jni/corpus/rejected.txt",
    ];
    let mut compared = 0_usize;
    for corpus in corpora {
        let path = format!("{}{corpus}", std::env!("CARGO_MANIFEST_DIR"));
        let Ok(data) = std::fs::read_to_string(&path) else {
            std::eprintln!("jni corpus not present (published crate); skipping");
            return;
        };
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
