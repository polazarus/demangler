use alloc::format;
use alloc::string::ToString;

use super::node::{HARD_INPUT_LEN, HARD_WORK};
use super::*;
use crate::{Budget, Demangler, Scheme};

/// Parse `symbol` with `budget` resolved for [`Scheme::Delphi`].
fn parse_with(symbol: &[u8], budget: Budget) -> Result<Ast<'_>, ParseError> {
    parse_bytes_with_budget(symbol, budget.effective(Scheme::Delphi))
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

/// A routine symbol whose unit name is `unit_len` bytes long.
fn routine_with_unit_len(unit_len: usize) -> alloc::string::String {
    format!("{}_$$_NOARGS", "A".repeat(unit_len))
}

#[test]
fn the_compiled_in_budget_matches_the_policy_defaults() {
    let limits = ParseLimits::from_effective(DEFAULT_BUDGET);
    assert_eq!(limits.input, MAX_INPUT_LEN);
    assert_eq!(limits.work, MAX_WORK);
    assert_eq!(Scheme::Delphi.default_budget(), DEFAULT_BUDGET);

    // The policy table and this module agree on the ceilings, and the flat
    // grammar has no depth or node dimension to raise.
    let hard = Scheme::Delphi.hard_budget();
    assert_eq!(hard.input_bytes, Some(Limit::Bounded(HARD_INPUT_LEN)));
    assert_eq!(hard.parse_depth, None);
    assert_eq!(hard.work, Some(Limit::Bounded(HARD_WORK)));
    assert_eq!(hard.nodes, None);
}

#[test]
fn each_dimension_rejects_one_unit_below_what_the_symbol_needs() {
    for symbol in [
        "DELPHI_PROBE_$$_NOARGS",
        "DELPHI_PROBE$_$TPROBECLASS_$__$$_PLAIN",
        "VMT_$DELPHI_PROBE_$$_TPROBECLASS",
    ] {
        assert_dimension_boundary(symbol, "input", |value| {
            Budget::new().with_input_bytes(value)
        });
        assert_dimension_boundary(symbol, "work", |value| Budget::new().with_work(value));
    }
}

#[test]
fn raising_the_input_budget_accepts_a_longer_symbol() {
    // One byte past the default cap, in the unit name.
    let symbol = routine_with_unit_len(MAX_INPUT_LEN);
    assert!(matches!(
        parse_with(symbol.as_bytes(), Budget::new()),
        Err(ParseError::BudgetExceeded)
    ));
    let raised = Budget::new().with_input_bytes(2 * MAX_INPUT_LEN);
    let ast = parse_with(symbol.as_bytes(), raised).expect("raised input budget");
    // Printing drops the `_$$_` marker and appends the empty parameter list.
    assert_eq!(ast.to_string().len(), symbol.len() - 1);
}

/// The input and work dimensions are independent: raising the input alone
/// keeps the default work allowance, so a scan that exceeds it still fails.
#[test]
fn raising_the_input_budget_does_not_raise_the_work_budget() {
    // The parser charges one work unit per byte, so this symbol needs more
    // work than the default allowance. It is not printed: the fixed output
    // cap only covers a name accepted at the default input cap.
    let symbol = routine_with_unit_len(600_000);
    assert!(symbol.len() > MAX_WORK);
    let work = symbol.len();

    let input_only = Budget::new().with_input_bytes(16 * MAX_INPUT_LEN);
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
    let limits = ParseLimits::from_effective(budget.effective(Scheme::Delphi));
    assert_eq!(limits.input, HARD_INPUT_LEN);
    assert_eq!(limits.work, HARD_WORK);
    assert_eq!(
        budget.effective(Scheme::Delphi).input_bytes,
        Some(Limit::Bounded(HARD_INPUT_LEN))
    );

    // A name past the hard input cap stays rejected even when the caller
    // asks for more.
    let too_long = routine_with_unit_len(HARD_INPUT_LEN);
    assert!(matches!(
        parse_with(too_long.as_bytes(), budget),
        Err(ParseError::BudgetExceeded)
    ));
}

#[test]
fn a_zero_budget_rejects_through_every_entry_point() {
    let symbol = "DELPHI_PROBE_$$_NOARGS";
    assert!(Demangler::new().demangle(symbol).is_ok());
    for budget in [
        Budget::new().with_input_bytes(0),
        Budget::new().with_work(0),
    ] {
        let demangler = Demangler::with_budget(budget);
        // Detection retains the parsed tree, so this also covers the
        // re-parse a configured budget triggers.
        assert!(demangler.demangle(symbol).is_err(), "{budget:?}");
        assert!(
            demangler
                .demangle_with_scheme(Scheme::Delphi, symbol)
                .is_err(),
            "{budget:?}"
        );
        assert!(
            demangler
                .selection()
                .allow(Scheme::Delphi)
                .demangle(symbol)
                .is_err(),
            "{budget:?}"
        );
    }
}

/// Printing is not a budget: the output is linear in the input.
///
/// Printing reproduces the linker name, so the text is linear in the input.
/// The corpus gate exercises the same bound over every symbol.
#[test]
fn the_output_is_linear_in_the_input() {
    let symbol = routine_with_unit_len(MAX_INPUT_LEN - 10);
    assert_eq!(symbol.len(), MAX_INPUT_LEN);
    let rendered = parse(symbol.as_bytes())
        .expect("the default input cap parses")
        .to_string();
    assert!(rendered.len() <= symbol.len());
}

#[test]
fn the_default_budget_parses_the_corpus_like_parse() {
    let Ok(data) = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/refs/delphi/corpus/fpc-3.2.2-linux-x86_64.tsv"
    )) else {
        std::eprintln!("delphi corpus not present (published crate); skipping");
        return;
    };
    let mut compared = 0_usize;
    for line in data.lines() {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let symbol = line.split('\t').next().unwrap_or_default();
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
    assert!(compared > 0, "the default budget compared no symbols");
}
