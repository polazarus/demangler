use alloc::format;
use alloc::string::ToString;

use super::node::{HARD_INPUT_LEN, HARD_WORK};
use super::*;
use crate::{Budget, Demangler, Scheme};

/// Parse `symbol` with `budget` resolved for [`Scheme::Fortran`].
fn parse_with(symbol: &[u8], budget: Budget) -> Result<Ast<'_>, Failure> {
    parse_bytes_with_budget(symbol, budget.effective(Scheme::Fortran))
}

/// Whether `symbol` parses with one budget dimension set to `value`.
fn parses_with(symbol: &[u8], dimension: impl Fn(usize) -> Budget, value: usize) -> bool {
    parse_with(symbol, dimension(value)).is_ok()
}

/// Assert that `need` is the exact boundary of one dimension for `symbol`.
///
/// The dimension is the only override, so a failure one unit below the
/// boundary comes from that dimension and not from another limit.
fn assert_dimension_boundary(symbol: &str, name: &str, dimension: impl Fn(usize) -> Budget) {
    let bytes = symbol.as_bytes();
    let ceiling = bytes.len() + 64;
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
    assert_eq!(limits.work, MAX_WORK);
    assert_eq!(Scheme::Fortran.default_budget(), DEFAULT_BUDGET);

    // The policy table and this module agree on the ceilings, and the flat
    // grammar has no depth or node dimension to raise.
    let hard = Scheme::Fortran.hard_budget();
    assert_eq!(hard.input_bytes, Some(Limit::Bounded(HARD_INPUT_LEN)));
    assert_eq!(hard.parse_depth, None);
    assert_eq!(hard.work, Some(Limit::Bounded(HARD_WORK)));
    assert_eq!(hard.nodes, None);
}

#[test]
fn each_dimension_rejects_one_unit_below_what_the_symbol_needs() {
    for symbol in ["__fprobe_MOD_add", "ext_sub_", "MAIN__"] {
        assert_dimension_boundary(symbol, "input", |value| {
            Budget::new().with_input_bytes(value)
        });
        assert_dimension_boundary(symbol, "work", |value| Budget::new().with_work(value));
    }
}

#[test]
fn raising_the_input_budget_accepts_a_longer_symbol() {
    // One byte past the default cap: the trailing underscore of an external
    // name.
    let symbol = format!("{}_", "a".repeat(MAX_INPUT_LEN));
    assert!(matches!(
        parse_with(symbol.as_bytes(), Budget::new()),
        Err(Failure::BudgetExceeded)
    ));
    let raised = Budget::new().with_input_bytes(2 * MAX_INPUT_LEN);
    let ast = parse_with(symbol.as_bytes(), raised).expect("raised input budget");
    assert_eq!(ast.to_string().len(), MAX_INPUT_LEN);
}

/// The input and work dimensions are independent: raising the input alone
/// keeps the default work allowance, so a scan that exceeds it still fails.
#[test]
fn raising_the_input_budget_does_not_raise_the_work_budget() {
    // The parser charges one work unit per byte, so this symbol needs more
    // work than the default allowance. It is not printed: the fixed output
    // cap only covers a name accepted at the default input cap.
    let symbol = format!("{}_", "a".repeat(600_000));
    assert!(symbol.len() > MAX_WORK);
    let work = symbol.len();

    let input_only = Budget::new().with_input_bytes(16 * MAX_INPUT_LEN);
    assert!(matches!(
        parse_with(symbol.as_bytes(), input_only),
        Err(Failure::BudgetExceeded)
    ));

    assert!(parse_with(symbol.as_bytes(), input_only.with_work(work)).is_ok());
}

#[test]
fn clamping_keeps_the_hard_ceilings() {
    let budget = Budget::new()
        .with_input_bytes(usize::MAX)
        .with_work(usize::MAX);
    let limits = ParseLimits::from_effective(budget.effective(Scheme::Fortran));
    assert_eq!(limits.input, HARD_INPUT_LEN);
    assert_eq!(limits.work, HARD_WORK);
    assert_eq!(
        budget.effective(Scheme::Fortran).input_bytes,
        Some(Limit::Bounded(HARD_INPUT_LEN))
    );

    // A name past the hard input cap stays rejected even when the caller
    // asks for more.
    let too_long = format!("{}_", "a".repeat(HARD_INPUT_LEN));
    assert!(matches!(
        parse_with(too_long.as_bytes(), budget),
        Err(Failure::BudgetExceeded)
    ));
}

#[test]
fn a_zero_budget_rejects_through_every_entry_point() {
    let symbol = "__fprobe_MOD_add";
    assert!(Demangler::new().demangle(symbol).is_ok());
    for budget in [
        Budget::new().with_input_bytes(0),
        Budget::new().with_work(0),
    ] {
        let demangler = Demangler::with_budget(budget);
        assert!(demangler.demangle(symbol).is_err(), "{budget:?}");
        assert!(
            demangler
                .demangle_with_scheme(Scheme::Fortran, symbol)
                .is_err(),
            "{budget:?}"
        );
        assert!(
            demangler
                .selection()
                .allow(Scheme::Fortran)
                .demangle(symbol)
                .is_err(),
            "{budget:?}"
        );
    }
}

/// Printing is not a budget: the output is linear in the input.
///
/// Printing drops the `__` prefix and the `_MOD_` marker, so the text is
/// shorter than the input. The corpus gate exercises the same bound over
/// every symbol.
#[test]
fn the_output_is_linear_in_the_input() {
    let symbol = format!("{}_", "a".repeat(MAX_INPUT_LEN - 1));
    let rendered = parse_bytes_with_budget(symbol.as_bytes(), DEFAULT_BUDGET)
        .expect("the default input cap parses")
        .to_string();
    assert!(rendered.len() < symbol.len());
}

#[test]
fn the_default_budget_parses_the_corpus_like_parse() {
    let Ok(data) = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/refs/fortran/corpus/gfortran-14.2-linux-x86_64.tsv"
    )) else {
        std::eprintln!("fortran corpus not present (published crate); skipping");
        return;
    };
    let mut compared = 0_usize;
    for line in data.lines() {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let symbol = line.split('\t').next().unwrap_or_default();
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
    assert!(compared > 0, "the default budget compared no symbols");
}
