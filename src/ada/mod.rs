//! Pure-Rust Ada/GNAT external-name demangling.
//!
//! GNAT does not use a C++-style mangling for Ada declarations. It builds an
//! *external name* from the fully-qualified Ada name by replacing every `.`
//! with `__` and lowercasing the identifier, then appends compiler suffixes
//! for protected operations, entry barriers and bodies, task bodies,
//! elaboration procedures and runtime helpers. This backend implements the
//! user-level spelling and the suffixes whose reading is unambiguous.
//!
//! No external demangler is used and there is no runtime dependency.
//!
//! # Supported forms
//!
//! - qualified user names: lowercase identifiers joined by `__`, printed with
//!   `.` (`ada__exceptions__triggered_by_abort` ->
//!   `ada.exceptions.triggered_by_abort`);
//! - `_` inside an identifier (`ada__foo_bar` -> `ada.foo_bar`);
//! - protected-operation suffixes `N`/`P` on the last identifier
//!   (`pkg__prot__setN` and `pkg__prot__setP` -> `pkg.prot.set`);
//! - entry barrier and entry body suffixes `_B<digits>s`/`_E<digits>s`
//!   (`pkg__prot__wait_B4s` and `pkg__prot__wait_E3s` -> `pkg.prot.wait`);
//! - the task-body suffix `TKB` (`pkg__workerTKB` -> `pkg.worker`);
//! - elaboration body/spec suffixes `___elabb`/`___elabs`
//!   (`pkg___elabb` -> `pkg'Elab_Body`, `pkg___elabs` -> `pkg'Elab_Spec`);
//! - a Mach-O leading platform underscore on the high-confidence `ada__`
//!   family (`_ada__foo` and `__ada__foo` -> `ada.foo`);
//! - a trailing linker decoration (`.llvm.<hex>` or `@version`) is stripped
//!   before parsing, matching the other backends.
//!
//! # Canonical output
//!
//! There is no standard Ada demangler, so the printed text is this crate's
//! contract:
//!
//! | Input | Output |
//! |---|---|
//! | `ada__exceptions__triggered_by_abort` | `ada.exceptions.triggered_by_abort` |
//! | `pkg__child__nested` | `pkg.child.nested` |
//! | `pkg__prot__setN` | `pkg.prot.set` |
//! | `pkg__prot__wait_B4s` | `pkg.prot.wait` |
//! | `pkg__workerTKB` | `pkg.worker` |
//! | `pkg___elabb` | `pkg'Elab_Body` |
//! | `pkg___elabs` | `pkg'Elab_Spec` |
//!
//! The spelling is a debugger name, not re-parsable Ada source: `__` becomes
//! `.`, and the elaboration suffix becomes an Ada attribute-style apostrophe.
//! An operation, entry or task suffix names a generated subprogram of the
//! entity, so it prints nothing and the entity name remains: the barrier
//! `pkg__prot__wait_B4s` and the entry body `pkg__prot__wait_E3s` both print
//! `pkg.prot.wait`, which is also what `c++filt -s gnat` prints for them.
//!
//! # Detection
//!
//! [`crate::Scheme::detect`] claims only high-confidence GNAT prefixes:
//! `ada__`, `_ada__`, `__ada__`, `system__` and `gnat__`. This is a prefix
//! prefilter; [`parse`](crate::ada::parse) still rejects unsupported bodies (task, runtime and
//! other compiler suffixes), so a malformed high-confidence name becomes an
//! error rather than a guessed rendering. General `pkg__subp` names are valid
//! GNAT but indistinguishable from ordinary C names (`foo__bar`), so they are
//! available through [`parse`](crate::ada::parse) and are deliberately not auto-detected.
//!
//! # Budgets
//!
//! Untrusted input stays bounded by [`MAX_INPUT_LEN`](crate::ada::MAX_INPUT_LEN),
//! [`MAX_SEGMENTS`](crate::ada::MAX_SEGMENTS), [`MAX_WORK`](crate::ada::MAX_WORK)
//! and [`MAX_OUTPUT_LEN`](crate::ada::MAX_OUTPUT_LEN). Exceeding a budget is an error, never a
//! partial result. The grammar is flat, so there is no recursive depth to
//! bound; the segment limit is the node budget.
//!
//! The input and work limits are runtime-configurable.
//! [`parse`](crate::ada::parse) keeps the compiled-in defaults, and a
//! [`crate::Demangler`] passes the configured budget, clamped to the hard
//! ceilings of 16 MiB input and 64M work units. The two dimensions are
//! independent: raising the input limit alone keeps the default work allowance,
//! so a caller that raises the input for a long symbol must raise
//! [`crate::Budget::work`] as well when the scan exceeds it. The grammar has no
//! depth or node dimension to configure, and the segment limit is a fixed
//! backend extra rather than a budget.
//!
//! Printing is not a budget: the printer walks each segment once, so the output
//! is linear in the name and [`MAX_OUTPUT_LEN`](crate::ada::MAX_OUTPUT_LEN) is a
//! fixed backstop sized for the hard input ceiling. No accepted parse can
//! exceed it, so `Display` is total except for a failing sink.
//!
//! # Gaps
//!
//! Unsupported forms are rejected, not guessed:
//!
//! - task-type bodies and their runtime helpers (`_TB`, `_VIP`, `_Z`, `_T1`,
//!   `_TKE`, `_TKVIP`, `_TKZ`);
//! - package/child elaboration flags (`_E`) and protected-type descriptors
//!   (`_A`, `_F`, `_VDF`, `_VDI`, `_VFD`, `_VIP`);
//! - record initialization helpers (`_IP`), enum spec helpers (`_S`) and the
//!   protected-object elaboration helper (`_P<n>sIP`);
//! - package-level runtime helpers (`___master`, `___chain`);
//! - local-symbol `.N` suffixes and overload disambiguators (`__N`);
//! - foreign-convention, operator, and C++/other-language external names;
//! - names whose last identifier has an unknown uppercase compiler suffix
//!   (for example `enumS` or `setX`).
//!
//! An enum initialization helper such as `pkg__enumN` is accepted because it
//! uses the same trailing `N` as a protected operation.
//!
//! Every rejected class above is one that `c++filt -s gnat` cannot render
//! either: it echoes the symbol or prints a `<...>` placeholder. Classes the
//! oracle *can* render (the `_ada_` escape, overload disambiguators, `.N`
//! suffixes, the `X` suffix, operator and controlled-type helpers) stay
//! rejected because they are outside this slice; they are recorded in
//! `refs/ada.md` with their oracle output.
//!
//! These classes are counted in `refs/ada.md`.

mod node;
mod parser;
mod printer;

use crate::error::BudgetKind;
use core::fmt;

use crate::budget::{EffectiveBudget, Limit};
use node::ParseLimits;
pub use node::{Ast, MAX_INPUT_LEN, MAX_OUTPUT_LEN, MAX_SEGMENTS, MAX_WORK};
pub use parser::is_ada;

/// Why an Ada/GNAT symbol could not be parsed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ParseError {
    /// The symbol does not match a supported GNAT external-name form.
    Invalid,
    /// The input, work or segment budget was exceeded.
    BudgetExceeded,
}

impl ParseError {
    /// The parser budget dimension this failure belongs to.
    #[must_use]
    pub const fn budget_kind(&self) -> Option<BudgetKind> {
        match self {
            Self::BudgetExceeded => Some(BudgetKind::Other),
            _ => None,
        }
    }
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Invalid => "invalid syntax",
            Self::BudgetExceeded => "size limit reached",
        })
    }
}

impl core::error::Error for ParseError {}

impl fmt::Display for Ast<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.print(f)
    }
}

impl Ast<'_> {
    /// Render the parsed name into `out`, without a print cap.
    pub(crate) fn print<W: fmt::Write + ?Sized>(&self, out: &mut W) -> fmt::Result {
        printer::print(&self.name, out)
    }
}

/// Parse an Ada/GNAT external name into a reusable [`Ast`].
///
/// General GNAT user names are accepted, including names that are not
/// auto-detected by [`crate::Scheme::detect`]. A Mach-O leading underscore on
/// the `ada__` family and a trailing linker decoration are stripped first.
///
/// # Errors
///
/// Returns [`ParseError::Invalid`] when the symbol is not a supported GNAT
/// external name (including task, protected-type, runtime and other
/// compiler-generated suffixes), and [`ParseError::BudgetExceeded`] when the
/// input, work or segment budget is exceeded.
pub fn parse(symbol: &str) -> Result<Ast<'_>, ParseError> {
    parse_bytes(symbol.as_bytes())
}

/// The budget the public entry point starts from.
///
/// [`parse`] uses it, so it keeps exactly the compiled-in limits; a
/// [`crate::Demangler`] resolves its configured [`crate::Budget`] for
/// [`Scheme::Ada`](crate::Scheme::Ada) instead.
pub(crate) const DEFAULT_BUDGET: EffectiveBudget = EffectiveBudget {
    input_bytes: Some(Limit::Bounded(MAX_INPUT_LEN)),
    parse_depth: None,
    work: Some(Limit::Bounded(MAX_WORK)),
    nodes: None,
    expansion_visits: None,
    expansion_bytes: None,
};

/// Parse an Ada/GNAT external name from raw bytes.
///
/// This is the entry point the demangler dispatch uses once symbol bytes, not
/// text, are the input. See [`parse`] for the accepted forms; the only
/// difference is that the name does not have to be UTF-8.
pub(crate) fn parse_bytes(symbol: &[u8]) -> Result<Ast<'_>, ParseError> {
    parse_bytes_with_budget(symbol, DEFAULT_BUDGET)
}

/// Parse a raw symbol byte string with one scheme's resolved budget.
///
/// The public [`parse`] wraps this with [`DEFAULT_BUDGET`]; a
/// [`crate::Demangler`] passes [`Demangler::effective_budget`] for
/// [`Scheme::Ada`](crate::Scheme::Ada), so a caller's overrides reach the
/// parser. The input and work values are clamped to this module's hard
/// ceilings, and the grammar has no depth or node dimension to resolve.
///
/// [`Demangler::effective_budget`]: crate::Demangler::effective_budget
pub(crate) fn parse_bytes_with_budget(
    symbol: &[u8],
    budget: EffectiveBudget,
) -> Result<Ast<'_>, ParseError> {
    parser::parse_bytes(symbol, ParseLimits::from_effective(budget))
}

/// Budget-aware Ada detection prefilter.
pub(crate) fn is_ada_with_budget(symbol: &[u8], budget: EffectiveBudget) -> bool {
    parser::is_ada_with(symbol, ParseLimits::from_effective(budget).input)
}

#[cfg(test)]
mod tests {
    use alloc::string::{String, ToString};

    use super::*;

    #[test]
    fn parses_qualified_names() {
        for (symbol, expected) in [
            (
                "ada__exceptions__triggered_by_abort",
                "ada.exceptions.triggered_by_abort",
            ),
            ("pkg__child__nested", "pkg.child.nested"),
            ("ada__foo_bar", "ada.foo_bar"),
            ("pkg__x1_y2", "pkg.x1_y2"),
        ] {
            assert_eq!(parse(symbol).unwrap().to_string(), expected, "{symbol}");
        }
    }

    #[test]
    fn strips_protected_suffixes() {
        for (symbol, expected) in [
            ("pkg__prot__setN", "pkg.prot.set"),
            ("pkg__prot__setP", "pkg.prot.set"),
            ("pkg__prot__getN", "pkg.prot.get"),
            ("pkg__prot__getP", "pkg.prot.get"),
            ("pkg__enumN", "pkg.enum"),
            ("pkg__set2N", "pkg.set2"),
        ] {
            assert_eq!(parse(symbol).unwrap().to_string(), expected, "{symbol}");
        }
    }

    #[test]
    fn strips_entry_suffixes() {
        for (symbol, expected) in [
            ("pkg__prot__wait_B4s", "pkg.prot.wait"),
            ("pkg__prot__wait_E3s", "pkg.prot.wait"),
            ("pkg__prot__fam_B9s", "pkg.prot.fam"),
            ("pkg__prot__fam_E8s", "pkg.prot.fam"),
            ("pkg__prot__wait_Bs", "pkg.prot.wait"),
            ("pkg__prot__wait_B04s", "pkg.prot.wait"),
            ("ada__prot__wait_B4s", "ada.prot.wait"),
        ] {
            assert_eq!(parse(symbol).unwrap().to_string(), expected, "{symbol}");
        }
    }

    #[test]
    fn strips_task_body_suffix() {
        for (symbol, expected) in [
            ("pkg__workerTKB", "pkg.worker"),
            ("ada_probe__singletonTKB", "ada_probe.singleton"),
        ] {
            assert_eq!(parse(symbol).unwrap().to_string(), expected, "{symbol}");
        }
    }

    #[test]
    fn prints_elaboration_suffixes() {
        for (symbol, expected) in [
            ("pkg___elabb", "pkg'Elab_Body"),
            ("pkg___elabs", "pkg'Elab_Spec"),
            ("pkg__child___elabb", "pkg.child'Elab_Body"),
            ("ada__foo___elabs", "ada.foo'Elab_Spec"),
        ] {
            assert_eq!(parse(symbol).unwrap().to_string(), expected, "{symbol}");
        }
    }

    #[test]
    fn strips_platform_underscores() {
        for symbol in ["_ada__foo", "__ada__foo"] {
            assert_eq!(parse(symbol).unwrap().to_string(), "ada.foo", "{symbol}");
        }
    }

    #[test]
    fn strips_linker_decorations() {
        assert_eq!(parse("ada__foo.llvm.1234").unwrap().to_string(), "ada.foo");
        assert_eq!(parse("ada__foo@@GNAT_1").unwrap().to_string(), "ada.foo");
    }

    #[test]
    fn rejects_unsupported_suffixes() {
        for symbol in [
            "pkg__workerTB",
            "pkg__workerVIP",
            "pkg__workerZ",
            "pkg__workerTKE",
            "pkg__workerTKVIP",
            "pkg__workerTKZ",
            "pkg__workerT1",
            "pkg_E",
            "pkg__child_E",
            "pkg__recIP",
            "pkg__protVIP",
            "pkg__protA",
            "pkg__prot__wait_B4",
            "pkg__prot__wait_Bx4s",
            "pkg__prot__wait_B4S",
            "pkg__prot__wait_B4s__2",
            "pkg__prot__waitN_B4s",
            "pkg__prot__wait_B4s.2",
            "pkg___master",
            "pkg___chain",
            "pkg__objP2sIP",
            "pkg__enumS",
            "pkg__setX",
            "pkg__setNP",
            "pkg__over__2",
            "pkg__nested__inner.2",
            "pkg__setN___elabb",
            "pkg__workerTKB___elabb",
            "_ada_main",
            "DW.ref.__gnat_personality_v0",
        ] {
            assert!(parse(symbol).is_err(), "{symbol} should be rejected");
        }
    }

    #[test]
    fn rejects_malformed_names() {
        for symbol in [
            "",
            "ada__",
            "ada__Foo",
            "ada__9foo",
            "ada__foo_",
            "ada__foo__",
            "ada__foo___elabx",
            "ada__foo___elabb__2",
            "ada__foo-bar",
            "ada__foo bar",
        ] {
            assert!(parse(symbol).is_err(), "{symbol:?} should be rejected");
        }
    }

    #[test]
    fn budgets_are_enforced() {
        let long = "a".repeat(MAX_INPUT_LEN + 1);
        assert!(matches!(parse(&long), Err(ParseError::BudgetExceeded)));

        let mut many = String::from("a");
        for _ in 0..MAX_SEGMENTS {
            many.push_str("__a");
        }
        assert!(matches!(parse(&many), Err(ParseError::BudgetExceeded)));
    }

    #[test]
    fn detection_is_precision_only() {
        for symbol in [
            "ada__foo",
            "ada__exceptions__triggered_by_abort",
            "system__foo",
            "gnat__foo",
            "_ada__foo",
            "__ada__foo",
            "ada__",
            "ada__workerTB",
            "ada__foo__2",
            "system__workerTB",
        ] {
            assert!(is_ada(symbol.as_bytes()), "{symbol} should be Ada");
        }
        for symbol in ["foo__bar", "pkg__hello", "a__b__c", "_ada_main"] {
            assert!(!is_ada(symbol.as_bytes()), "{symbol} should not be Ada");
        }
    }
}

#[cfg(test)]
mod budget_tests {
    use alloc::string::ToString;

    use super::node::{HARD_INPUT_LEN, HARD_WORK};
    use super::*;
    use crate::{Budget, Demangler, Scheme};

    /// Parse `symbol` with `budget` resolved for [`Scheme::Ada`].
    fn parse_with(symbol: &[u8], budget: Budget) -> Result<Ast<'_>, ParseError> {
        parse_bytes_with_budget(symbol, budget.effective(Scheme::Ada))
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
        let ceiling = bytes.len() * 2 + 64;
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
        assert_eq!(
            Scheme::Ada.default_budget().effective(Scheme::Ada),
            DEFAULT_BUDGET
        );

        // The policy table and this module agree on the ceilings, and the flat
        // grammar has no depth or node dimension to raise.
        let hard = Scheme::Ada.hard_budget();
        assert_eq!(hard.input_bytes, Some(Limit::Bounded(HARD_INPUT_LEN)));
        assert_eq!(hard.parse_depth, None);
        assert_eq!(hard.work, Some(Limit::Bounded(HARD_WORK)));
        assert_eq!(hard.nodes, None);
    }

    #[test]
    fn each_dimension_rejects_one_unit_below_what_the_symbol_needs() {
        for symbol in ["ada__foo", "pkg__prot__wait_B4s", "pkg___elabb"] {
            assert_dimension_boundary(symbol, "input", |value| {
                Budget::new().with_input_bytes(value)
            });
            assert_dimension_boundary(symbol, "work", |value| Budget::new().with_work(value));
        }
    }

    #[test]
    fn raising_the_input_budget_accepts_a_longer_symbol() {
        // One byte past the default cap, in a single identifier.
        let symbol = "a".repeat(MAX_INPUT_LEN + 1);
        assert!(matches!(
            parse_with(symbol.as_bytes(), Budget::new()),
            Err(ParseError::BudgetExceeded)
        ));
        let raised = Budget::new().with_input_bytes(2 * MAX_INPUT_LEN);
        let ast = parse_with(symbol.as_bytes(), raised).expect("raised input budget");
        assert_eq!(ast.to_string().len(), symbol.len());
    }

    /// The input and work dimensions are independent: raising the input alone
    /// keeps the default work allowance, so a scan that exceeds it still fails.
    #[test]
    fn raising_the_input_budget_does_not_raise_the_work_budget() {
        // The lookahead charge plus the scan cost two work units per byte, so
        // this symbol needs more work than the default allowance. It is not
        // printed: the fixed output cap only covers a name accepted at the
        // default input cap.
        let symbol = "a".repeat(300_000);
        let work = 2 * symbol.len();
        assert!(work > MAX_WORK);

        let input_only = Budget::new().with_input_bytes(8 * MAX_INPUT_LEN);
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
        let limits = ParseLimits::from_effective(budget.effective(Scheme::Ada));
        assert_eq!(limits.input, HARD_INPUT_LEN);
        assert_eq!(limits.work, HARD_WORK);
        assert_eq!(
            budget.effective(Scheme::Ada).input_bytes,
            Some(Limit::Bounded(HARD_INPUT_LEN))
        );

        // A name past the hard input cap stays rejected even when the caller
        // asks for more.
        let too_long = "a".repeat(HARD_INPUT_LEN + 1);
        assert!(matches!(
            parse_with(too_long.as_bytes(), budget),
            Err(ParseError::BudgetExceeded)
        ));
    }

    #[test]
    fn a_zero_budget_rejects_through_every_entry_point() {
        let symbol = "ada__foo";
        assert!(Demangler::new().demangle(symbol).is_ok());
        for budget in [
            Budget::new().with_input_bytes(0),
            Budget::new().with_work(0),
        ] {
            let demangler = Demangler::with_budget(budget);
            assert!(demangler.demangle(symbol).is_err(), "{budget:?}");
            assert!(
                demangler.demangle_with_scheme(Scheme::Ada, symbol).is_err(),
                "{budget:?}"
            );
            assert!(
                demangler
                    .selection()
                    .allow(Scheme::Ada)
                    .demangle(symbol)
                    .is_err(),
                "{budget:?}"
            );
        }
    }

    /// The fixed output cap is a backstop, not a print budget.
    ///
    /// Printing replaces `__` with `.` and an elaboration suffix with a ten-byte
    /// attribute, so the text is never longer than the input plus a small
    /// margin. The longest name the default input cap accepts therefore still
    /// fits, and the corpus gate exercises the same bound over every symbol.
    #[test]
    fn the_output_cap_is_a_backstop_the_printer_never_reaches() {
        let symbol = "a".repeat(MAX_INPUT_LEN);
        let rendered = parse(&symbol)
            .expect("the default input cap parses")
            .to_string();
        assert!(rendered.len() <= MAX_OUTPUT_LEN);
        assert!(rendered.len() <= symbol.len() + 8);
    }

    #[test]
    fn the_default_budget_parses_the_corpus_like_parse() {
        let Ok(data) = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/refs/ada/corpus/gnat-14.2-linux-x86_64.tsv"
        )) else {
            std::eprintln!("ada corpus not present (published crate); skipping");
            return;
        };
        let mut compared = 0_usize;
        for line in data.lines() {
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let symbol = line.split('\t').next().unwrap_or_default();
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
        assert!(compared > 0, "the default budget compared no symbols");
    }
}
