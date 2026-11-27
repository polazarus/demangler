use alloc::format;
use alloc::string::{String, ToString};

use super::node::{HARD_INPUT_LEN, HARD_WORK, ParseLimits};
use super::*;
use crate::budget::Limit;
use crate::{Budget, Demangler, Scheme};

/// Parse `symbol` with `budget` resolved for [`Scheme::ObjC`].
fn parse_with(symbol: &[u8], budget: Budget) -> Result<Ast<'_>, ParseError> {
    parse_bytes_with_budget(symbol, budget.effective(Scheme::ObjC))
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
    assert_eq!(
        Scheme::ObjC.default_budget().effective(Scheme::ObjC),
        DEFAULT_BUDGET
    );

    // The policy table and this module agree on the ceilings, and an
    // Objective-C symbol is one record, so there is no node dimension.
    let hard = Scheme::ObjC.hard_budget();
    assert_eq!(hard.input_bytes, Some(Limit::Bounded(HARD_INPUT_LEN)));
    assert_eq!(hard.parse_depth, Some(Limit::Bounded(MAX_DEPTH)));
    assert_eq!(hard.work, Some(Limit::Bounded(HARD_WORK)));
    assert_eq!(hard.nodes, Some(Limit::Unbounded));
}

#[test]
fn each_dimension_rejects_one_unit_below_what_the_symbol_needs() {
    for symbol in [
        "_OBJC_CLASS_$_Foo",
        "__OBJC_$_CATEGORY_Foo_$_Bar",
        "__objc_ivar_offset_value_Foo._ivar",
        "___block_descriptor_40_e8_32r_e5_v8?0l",
    ] {
        let ceiling = symbol.len() * 2 + 64;
        assert_dimension_boundary(symbol, "input", ceiling, |value| {
            Budget::new().with_input_bytes(value)
        });
        assert_dimension_boundary(symbol, "work", ceiling, |value| {
            Budget::new().with_work(value)
        });
    }

    // A method display form carries exactly one bracket level.
    assert_dimension_boundary("-[Foo bar]", "depth", 4, |value| {
        Budget::new().with_parse_depth(value)
    });
}

#[test]
fn raising_the_input_budget_accepts_a_longer_symbol() {
    let mut symbol = String::from("_OBJC_CLASS_$_");
    symbol.push_str(&"a".repeat(MAX_INPUT_LEN));
    assert!(matches!(
        parse_with(symbol.as_bytes(), Budget::new()),
        Err(ParseError::BudgetExceeded)
    ));
    let raised = Budget::new().with_input_bytes(2 * MAX_INPUT_LEN);
    let ast = parse_with(symbol.as_bytes(), raised).expect("raised input budget");
    assert_eq!(
        ast.to_string().len(),
        "Objective-C class ".len() + MAX_INPUT_LEN
    );
}

/// The bracket depth is the grammar's own limit, so a depth override can
/// only tighten it: asking for more is clamped to the one level the
/// grammar has.
#[test]
fn the_depth_budget_cannot_be_raised_past_the_grammar() {
    let budget = Budget::new().with_parse_depth(usize::MAX);
    assert_eq!(
        budget.effective(Scheme::ObjC).parse_depth,
        Some(Limit::Bounded(MAX_DEPTH))
    );
    assert!(parse_with(b"-[Foo bar]", budget).is_ok());
    assert!(matches!(
        parse_with(b"-[Foo bar[[]]]", budget),
        Err(ParseError::RecursedTooDeep)
    ));
}

/// The input and work dimensions are independent: raising the input alone
/// keeps the default work allowance, so a scan that exceeds it still fails.
#[test]
fn raising_the_input_budget_does_not_raise_the_work_budget() {
    // The parser charges one work unit per input byte for the balance scan
    // and one per name byte for validation.
    let mut symbol = String::from("_OBJC_CLASS_$_");
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
        .with_work(usize::MAX);
    let limits = ParseLimits::from_effective(budget.effective(Scheme::ObjC));
    assert_eq!(limits.input, HARD_INPUT_LEN);
    assert_eq!(limits.work, HARD_WORK);
    assert_eq!(
        budget.effective(Scheme::ObjC).input_bytes,
        Some(Limit::Bounded(HARD_INPUT_LEN))
    );

    // A name past the hard input cap stays rejected even when the caller
    // asks for more.
    let mut too_long = String::from("_OBJC_CLASS_$_");
    too_long.push_str(&"a".repeat(HARD_INPUT_LEN));
    assert!(matches!(
        parse_with(too_long.as_bytes(), budget),
        Err(ParseError::BudgetExceeded)
    ));
}

#[test]
fn a_zero_budget_rejects_through_every_entry_point() {
    let symbol = "_OBJC_CLASS_$_Foo";
    assert!(Demangler::new().demangle(symbol).is_ok());
    for budget in [
        Budget::new().with_input_bytes(0),
        Budget::new().with_work(0),
    ] {
        let demangler = Demangler::with_budget(budget);
        assert!(demangler.demangle(symbol).is_err(), "{budget:?}");
        assert!(
            demangler
                .demangle_with_scheme(Scheme::ObjC, symbol)
                .is_err(),
            "{budget:?}"
        );
        assert!(
            demangler
                .selection()
                .allow(Scheme::ObjC)
                .demangle(symbol)
                .is_err(),
            "{budget:?}"
        );
    }

    // The depth dimension only rejects a name that carries a bracket.
    let method = "-[Foo bar]";
    let demangler = Demangler::with_budget(Budget::new().with_parse_depth(0));
    assert!(demangler.demangle(method).is_err());
    assert!(demangler.demangle(symbol).is_ok());
}

/// Printing is not a budget: the output is linear in the input.
///
/// The printer walks the record once, and the widest expansion is `\xNN`
/// for a byte that is not valid UTF-8, so four times the input plus the
/// description bounds the text. A name accepted under a raised input
/// budget is covered too.
#[test]
fn the_output_is_linear_in_the_input() {
    let mut symbol = b"_OBJC_CLASS_$_".to_vec();
    symbol.resize(MAX_INPUT_LEN, 0xff);
    let raised = Budget::new().with_input_bytes(2 * MAX_INPUT_LEN);
    let rendered = parse_with(&symbol, raised)
        .expect("the raised input budget parses")
        .to_string();
    assert!(rendered.len() <= 4 * symbol.len() + 64);
}

#[test]
fn the_default_budget_parses_the_corpus_like_parse() {
    let corpora = [
        "/refs/objc/corpus/apple-clang-19.txt",
        "/refs/objc/corpus/gcc-libobjc-14.txt",
        "/refs/objc/corpus/gnustep-abi2-clang-19.txt",
        "/refs/objc/corpus/gnustep-abi2-coff-clang-19.txt",
        "/refs/objc/corpus/gnustep-clang-19.txt",
        "/refs/objc/corpus/mattermost-2.46.0-ios.txt",
        "/refs/objc/corpus/methods-clang-19.txt",
    ];
    let mut compared = 0_usize;
    for corpus in corpora {
        let path = format!("{}{corpus}", std::env!("CARGO_MANIFEST_DIR"));
        let Ok(data) = std::fs::read_to_string(&path) else {
            std::eprintln!("objc corpus not present (published crate); skipping");
            return;
        };
        for line in data.lines() {
            let symbol = line.split('\t').next().unwrap_or_default();
            if symbol.is_empty() || symbol.starts_with('#') {
                continue;
            }
            let expected = parse(symbol);
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
