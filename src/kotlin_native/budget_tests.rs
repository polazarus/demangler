use alloc::format;
use alloc::string::{String, ToString};

use super::node::{HARD_DEPTH, HARD_INPUT_LEN, HARD_WORK, ParseLimits};
use super::*;
use crate::budget::Limit;
use crate::{Budget, Demangler, Scheme};

/// Parse `symbol` with `budget` resolved for [`Scheme::KotlinNative`].
fn parse_with(symbol: &[u8], budget: Budget) -> Result<Ast<'_>, ParseError> {
    parse_bytes_with_budget(symbol, budget.effective(Scheme::KotlinNative))
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

#[test]
fn the_compiled_in_budget_matches_the_policy_defaults() {
    let limits = ParseLimits::from_effective(DEFAULT_BUDGET);
    assert_eq!(limits.input, MAX_INPUT_LEN);
    assert_eq!(limits.depth, MAX_DEPTH);
    assert_eq!(limits.work, MAX_WORK);
    assert_eq!(Scheme::KotlinNative.default_budget(), DEFAULT_BUDGET);

    // The policy table and this module agree on the ceilings. The parser
    // builds one symbol with its own bounded lists, so there is no node
    // dimension to raise.
    let hard = Scheme::KotlinNative.hard_budget();
    assert_eq!(hard.input_bytes, Some(Limit::Bounded(HARD_INPUT_LEN)));
    assert_eq!(hard.parse_depth, Some(Limit::Bounded(HARD_DEPTH)));
    assert_eq!(hard.work, Some(Limit::Bounded(HARD_WORK)));
    assert_eq!(hard.nodes, Some(Limit::Unbounded));
}

#[test]
fn each_dimension_rejects_one_unit_below_what_the_symbol_needs() {
    for symbol in [
        "kfun:com.example#main(){}",
        "ktype:kotlin.Array<kotlin.Int>",
        "kclass:com.example.Box#internal",
        "kfun:com.example.Box#get(){}kotlin.Int",
    ] {
        let ceiling = symbol.len() * 2 + 64;
        assert_dimension_boundary(symbol, "input", ceiling, |value| {
            Budget::new().with_input_bytes(value)
        });
        assert_dimension_boundary(symbol, "work", ceiling, |value| {
            Budget::new().with_work(value)
        });
    }

    // Every parsed type enters one depth level, so a one-argument type
    // needs two.
    assert_dimension_boundary("ktype:kotlin.Array<kotlin.Int>", "depth", 8, |value| {
        Budget::new().with_parse_depth(value)
    });
}

#[test]
fn raising_the_input_budget_accepts_a_longer_symbol() {
    let mut symbol = String::from("kclass:");
    symbol.push_str(&"a".repeat(MAX_INPUT_LEN));
    assert!(matches!(
        parse_with(symbol.as_bytes(), Budget::new()),
        Err(ParseError::BudgetExceeded)
    ));
    let raised = Budget::new().with_input_bytes(2 * MAX_INPUT_LEN);
    let ast = parse_with(symbol.as_bytes(), raised).expect("raised input budget");
    assert_eq!(ast.to_string().len(), MAX_INPUT_LEN);
}

#[test]
fn raising_the_depth_budget_accepts_a_deeper_type() {
    let mut symbol = String::from("ktype:");
    symbol.push_str(&"kotlin.Array<".repeat(MAX_DEPTH + 8));
    symbol.push('*');
    symbol.push_str(&">".repeat(MAX_DEPTH + 8));
    assert!(matches!(
        parse_with(symbol.as_bytes(), Budget::new()),
        Err(ParseError::RecursedTooDeep)
    ));
    let raised = Budget::new().with_parse_depth(MAX_DEPTH + 8);
    assert!(parse_with(symbol.as_bytes(), raised).is_ok());
}

/// The input and work dimensions are independent: raising the input alone
/// keeps the default work allowance, so a scan that exceeds it still fails.
#[test]
fn raising_the_input_budget_does_not_raise_the_work_budget() {
    // The parser charges one work unit per input byte before classifying.
    let mut symbol = String::from("kclass:");
    symbol.push_str(&"a".repeat(600_000));
    assert!(4 * symbol.len() > MAX_WORK);
    let work = 4 * symbol.len();

    let input_only = Budget::new().with_input_bytes(HARD_INPUT_LEN);
    assert!(matches!(
        parse_with(symbol.as_bytes(), input_only),
        Err(ParseError::BudgetExceeded)
    ));

    assert!(parse_with(symbol.as_bytes(), input_only.with_work(work)).is_ok());
}

#[test]
fn clamping_keeps_the_hard_ceilings() {
    let budget = Budget::new()
        .with_input_bytes(usize::MAX)
        .with_parse_depth(usize::MAX)
        .with_work(usize::MAX);
    let limits = ParseLimits::from_effective(budget.effective(Scheme::KotlinNative));
    assert_eq!(limits.input, HARD_INPUT_LEN);
    assert_eq!(limits.depth, HARD_DEPTH);
    assert_eq!(limits.work, HARD_WORK);
    assert_eq!(
        budget.effective(Scheme::KotlinNative).input_bytes,
        Some(Limit::Bounded(HARD_INPUT_LEN))
    );
    assert_eq!(
        budget.effective(Scheme::KotlinNative).parse_depth,
        Some(Limit::Bounded(HARD_DEPTH))
    );

    // A name past the hard input cap stays rejected even when the caller
    // asks for more.
    let mut too_long = String::from("kclass:");
    too_long.push_str(&"a".repeat(HARD_INPUT_LEN));
    assert!(matches!(
        parse_with(too_long.as_bytes(), budget),
        Err(ParseError::BudgetExceeded)
    ));
}

#[test]
fn a_zero_budget_rejects_through_every_entry_point() {
    // The parameter type enters a depth level, and the parser charges work
    // per input byte, so every dimension rejects this symbol.
    let symbol = "kfun:com.example#foo(kotlin.Int){}";
    assert!(Demangler::new().demangle(symbol).is_ok());
    for budget in [
        Budget::new().with_input_bytes(0),
        Budget::new().with_work(0),
    ] {
        let demangler = Demangler::with_budget(budget);
        assert!(demangler.demangle(symbol).is_err(), "{budget:?}");
        assert!(
            demangler
                .demangle_with_scheme(Scheme::KotlinNative, symbol)
                .is_err(),
            "{budget:?}"
        );
        assert!(
            demangler
                .selection()
                .allow(Scheme::KotlinNative)
                .demangle(symbol)
                .is_err(),
            "{budget:?}"
        );
    }

    // The depth dimension only rejects a symbol that declares a type.
    let demangler = Demangler::with_budget(Budget::new().with_parse_depth(0));
    assert!(demangler.demangle(symbol).is_err());
    assert!(demangler.demangle("kclass:com.example.Box").is_ok());
}

/// Printing is not a budget: the output is linear in the input.
///
/// The printer walks each node once, and the widest expansion is `\xNN`
/// for a byte that is not valid UTF-8, so four times the input bounds the
/// text.
#[test]
fn the_output_is_linear_in_the_input() {
    let mut symbol = b"kclass:".to_vec();
    symbol.resize(MAX_INPUT_LEN, 0xff);
    let raised = Budget::new().with_input_bytes(2 * MAX_INPUT_LEN);
    let rendered = parse_with(&symbol, raised)
        .expect("the raised input budget parses")
        .to_string();
    assert!(rendered.len() <= 4 * symbol.len());
}

#[test]
fn the_default_budget_parses_the_corpus_like_parse() {
    let corpora = [
        "/refs/kotlin-native/corpus/hand-authored.txt",
        "/refs/kotlin-native/corpus/kotlin-2.0.21-linux-x86_64.txt",
        "/refs/kotlin-native/corpus/kotlin-2.1.21-linux-x86_64.txt",
        "/refs/kotlin-native/corpus/kotlin-2.2.21-linux-x86_64.txt",
        "/refs/kotlin-native/corpus/kotlin-2.3.21-linux-x86_64.txt",
        "/refs/kotlin-native/corpus/kotlin-2.4.0-linux-x86_64.txt",
        "/refs/kotlin-native/corpus/kotlin-2.4.21-linux-x86_64.txt",
    ];
    let mut compared = 0_usize;
    for corpus in corpora {
        let path = format!("{}{corpus}", std::env!("CARGO_MANIFEST_DIR"));
        let Ok(data) = std::fs::read_to_string(&path) else {
            std::eprintln!("kotlin-native corpus not present (published crate); skipping");
            return;
        };
        for line in data.lines() {
            let symbol = line.split('\t').next().unwrap_or_default().trim_end();
            if symbol.is_empty() || symbol.starts_with('#') {
                continue;
            }
            let expected = parse(symbol.as_bytes());
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
