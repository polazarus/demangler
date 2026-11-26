//! Pure-Rust gfortran external-name demangling.
//!
//! gfortran does not use a C++-style mangling. A Fortran name is lowercased and
//! decorated by the compiler: module entities get a `__<module>_MOD_` prefix, an
//! external procedure or common block gets one appended underscore
//! (`-funderscoring`), and the main program is the fixed name `MAIN__`. This
//! backend implements the forms whose reading is unambiguous and rejects the
//! rest instead of guessing.
//!
//! No external demangler is used and there is no runtime dependency.
//!
//! # Supported forms
//!
//! ```text
//! symbol   ::= module | helper | external | "MAIN__"
//! module   ::= "__" ident "_MOD_" ident          (plus one Mach-O underscore)
//! helper   ::= "__" ident "_MOD_" "__" helper "_" ident "_" type
//! helper   ::= "vtab" | "def_init" | "copy" | "final"
//! type     ::= [A-Z] [a-z0-9_]*
//! external ::= ident "_"
//! ident    ::= [a-z] ( [a-z0-9_] )*
//! ```
//!
//! - a module procedure, module variable or module-level `entry`
//!   (`__fprobe_MOD_add`);
//! - a module name that itself ends with `_` (`__my_mod__MOD_var`);
//! - a generated helper of a module-level derived type: its vtable, default
//!   initialization, copy and finalization procedures
//!   (`__fprobe_MOD___vtab_fprobe_Point`);
//! - an external procedure, block-data unit or named common block
//!   (`ext_sub_`, `probe_blk_`);
//! - a name that already ends with `_`, which the compiler turns into `__`
//!   (`extd__` -> `extd_`);
//! - the main program (`MAIN__` -> `main`);
//! - a Mach-O leading platform underscore on the module form
//!   (`___fprobe_MOD_add`);
//! - a trailing linker decoration (`.llvm.<hex>` or `@version`) is stripped
//!   before parsing, matching the other backends.
//!
//! # Canonical output
//!
//! No standard demangler exists for gfortran symbols (`c++filt` has no Fortran
//! mode), so the printed text is this crate's contract: the dotted spelling a
//! debugger shows for a Fortran entity.
//!
//! | Input | Output |
//! |---|---|
//! | `__fprobe_MOD_add` | `fprobe.add` |
//! | `__fprobe_child_MOD_do_child` | `fprobe_child.do_child` |
//! | `__my_mod__MOD_var_x_` | `my_mod_.var_x_` |
//! | `__fprobe_MOD___vtab_fprobe_Point` | `fprobe.Point'vtab` |
//! | `__fprobe_MOD___def_init_fprobe_Point` | `fprobe.Point'def_init` |
//! | `__fprobe_MOD___copy_fprobe_Point` | `fprobe.Point'copy` |
//! | `__fprobe_MOD___final_fprobe_Point` | `fprobe.Point'final` |
//! | `ext_sub_` | `ext_sub` |
//! | `extd__` | `extd_` |
//! | `MAIN__` | `main` |
//!
//! The spelling is a debugger name, not re-parsable Fortran source: `_MOD_`
//! becomes `.`, and the appended underscore of an external name is dropped.
//! Module and member names are printed verbatim, because the compiler does not
//! underscore them. A derived-type helper prints the declaring module, the type
//! name as gfortran records it (first letter upper-cased) and an apostrophe tag
//! naming the helper, so it cannot be mistaken for the type itself.
//!
//! # Detection
//!
//! [`crate::Scheme::detect`] claims only the distinctive module form
//! `__<module>_MOD_<member>` (and the Mach-O spelling). External names are not
//! claimed: `foo_` is an ordinary C name as far as a symbol table is concerned,
//! so claiming it would steal every trailing-underscore C symbol. External
//! names, `MAIN__` and every module form are available through
//! [`parse`](crate::fortran::parse).
//!
//! The module form is a shape prefilter: a module symbol whose member is a
//! compiler-generated helper is claimed as Fortran and then rejected by
//! [`parse`](crate::fortran::parse), so it never renders as a C name.
//!
//! # Budgets
//!
//! Untrusted input stays bounded by [`MAX_INPUT_LEN`](crate::fortran::MAX_INPUT_LEN),
//! [`MAX_DEPTH`](crate::fortran::MAX_DEPTH), [`MAX_WORK`](crate::fortran::MAX_WORK) and
//! [`MAX_OUTPUT_LEN`](crate::fortran::MAX_OUTPUT_LEN). The grammar is flat and
//! the AST borrows at most two slices of the input, so there is no node budget
//! and no recursion. Exceeding a budget is an error, never a partial result.
//!
//! The input and work limits are runtime-configurable.
//! [`parse`](crate::fortran::parse) keeps the compiled-in defaults, and a
//! [`crate::Demangler`] passes the configured budget, clamped to the hard
//! ceilings of 16 MiB input and 64M work units. The two dimensions are
//! independent: raising the input limit alone keeps the default work allowance,
//! so a caller that raises the input for a long symbol must raise
//! [`crate::Budget::work`] as well when the one-pass scan exceeds it. The
//! grammar has no depth or node dimension to configure.
//!
//! Printing is not a budget: the printer walks each form once, so the output is
//! shorter than the input and [`MAX_OUTPUT_LEN`](crate::fortran::MAX_OUTPUT_LEN)
//! is a fixed backstop sized for the hard input ceiling; no accepted parse can
//! exceed it. That backstop is sized from
//! [`MAX_INPUT_LEN`](crate::fortran::MAX_INPUT_LEN), so a caller that raises the
//! input budget past it accepts a name whose printed form no longer fits and
//! `Display` reports the overflow instead of truncating.
//!
//! # Gaps
//!
//! Unsupported forms are rejected, not guessed:
//!
//! - a derived-type helper whose type part is not `<module>_<Type>`, such as
//!   the one gfortran emits for a type imported from `iso_c_binding`
//!   (`__holder_MOD___vtab___iso_c_binding_C_ptr`);
//! - submodule symbols, whose module name contains a `.`
//!   (`__outer.inner_MOD___vtab_outer.inner_Deep_t`);
//! - a parameterized derived-type helper (`__holder_MOD___vtab_holder_Pdtkdt_2`)
//!   is read only while its instantiated name keeps the `[A-Z][a-z0-9_]*` shape;
//! - internal and host-associated procedures, which are local symbols with a
//!   `.` (`inner.0`, `master.0.with_entry_`);
//! - `bind(c)` names, which carry no underscoring and are indistinguishable
//!   from C;
//! - `-fno-underscoring` external names, for the same reason;
//! - the `-fsecond-underscore` form when it is ambiguous: `x_y__` is either
//!   `x_y_` with one appended underscore or `x_y` with a doubled one;
//! - uppercase names: gfortran lowercases Fortran names, so `Foo_` is a foreign
//!   symbol;
//! - Mach-O external names (`__ext_sub_`), whose extra platform underscore is
//!   not distinguishable from the module prefix in this slice;
//! - other compilers' manglings (Intel `m_mp_x_`, NAG, LLVM/flang `_QMmEx`),
//!   which are different schemes entirely.
//!
//! These classes are measured in `refs/fortran.md`.

mod node;
mod parser;
mod printer;

use crate::error::BudgetKind;
use core::fmt;

use crate::budget::{EffectiveBudget, Limit};
use crate::parse::LimitedWriter;
use node::ParseLimits;
pub use node::{Ast, MAX_DEPTH, MAX_INPUT_LEN, MAX_OUTPUT_LEN, MAX_WORK};
pub use parser::is_fortran;

/// Why a gfortran symbol could not be parsed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ParseError {
    /// The symbol does not match a supported gfortran form.
    Invalid,
    /// The input or work budget was exceeded.
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
    /// Render the parsed name into `out`, enforcing [`MAX_OUTPUT_LEN`].
    pub(crate) fn print<W: fmt::Write + ?Sized>(&self, out: &mut W) -> fmt::Result {
        let mut limited = LimitedWriter::new(out, MAX_OUTPUT_LEN);
        printer::print(&self.name, &mut limited)
    }
}

/// Parse a gfortran external name into a reusable [`Ast`].
///
/// Every supported form is accepted, including the external names and the main
/// program that [`crate::Scheme::detect`] deliberately does not claim. A
/// trailing linker decoration is stripped first.
///
/// # Errors
///
/// Returns [`ParseError::Invalid`] when the symbol is not a supported gfortran
/// form (see the [module docs](crate::fortran) for the rejected classes) and
/// [`ParseError::BudgetExceeded`] when the input or work budget is exceeded.
pub fn parse(symbol: &str) -> Result<Ast<'_>, ParseError> {
    parse_bytes(symbol.as_bytes())
}

/// The budget the public entry point starts from.
///
/// [`parse`] uses it, so it keeps exactly the compiled-in limits; a
/// [`crate::Demangler`] resolves its configured [`crate::Budget`] for
/// [`Scheme::Fortran`](crate::Scheme::Fortran) instead.
pub(crate) const DEFAULT_BUDGET: EffectiveBudget = EffectiveBudget {
    input_bytes: Some(Limit::Bounded(MAX_INPUT_LEN)),
    parse_depth: None,
    work: Some(Limit::Bounded(MAX_WORK)),
    nodes: None,
    expansion_visits: None,
    expansion_bytes: None,
};

/// Parse a gfortran external name from raw bytes.
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
/// [`Scheme::Fortran`](crate::Scheme::Fortran), so a caller's overrides reach
/// the parser. The input and work values are clamped to this module's hard
/// ceilings, and the grammar has no depth or node dimension to resolve.
///
/// [`Demangler::effective_budget`]: crate::Demangler::effective_budget
pub(crate) fn parse_bytes_with_budget(
    symbol: &[u8],
    budget: EffectiveBudget,
) -> Result<Ast<'_>, ParseError> {
    parser::parse_bytes(symbol, ParseLimits::from_effective(budget))
}

/// Budget-aware Fortran detection prefilter.
pub(crate) fn is_fortran_with_budget(symbol: &[u8], budget: EffectiveBudget) -> bool {
    parser::is_fortran_with(symbol, ParseLimits::from_effective(budget).input)
}

#[cfg(test)]
mod tests {
    use alloc::string::ToString;

    use super::*;

    #[test]
    fn parses_module_symbols() {
        for (symbol, expected) in [
            ("__fprobe_MOD_add", "fprobe.add"),
            ("__fprobe_MOD_counter", "fprobe.counter"),
            ("__fprobe_MOD_scale_factor", "fprobe.scale_factor"),
            ("__fprobe_child_MOD_do_child", "fprobe_child.do_child"),
            (
                "__fprobe_generic_MOD_apply_real",
                "fprobe_generic.apply_real",
            ),
            ("__fprobe_MOD_trailing_", "fprobe.trailing_"),
            ("__fprobe_MOD_with_entry2", "fprobe.with_entry2"),
            ("__my_mod__MOD_var_x_", "my_mod_.var_x_"),
            ("__m1_MOD_x2", "m1.x2"),
            ("__a_mod_MOD_x", "a_mod.x"),
            ("__mod_MOD_mod", "mod.mod"),
            ("__m_MOD_od_x", "m.od_x"),
        ] {
            assert_eq!(parse(symbol).unwrap().to_string(), expected, "{symbol}");
        }
    }

    #[test]
    fn parses_external_names() {
        for (symbol, expected) in [
            ("ext_sub_", "ext_sub"),
            ("my_ext_sub_", "my_ext_sub"),
            ("ext_with_digits_1_", "ext_with_digits_1"),
            ("probe_blk_", "probe_blk"),
            ("probe_block_", "probe_block"),
            // One appended underscore to a name that already ends with `_`.
            ("extd__", "extd_"),
            ("x1__", "x1_"),
        ] {
            assert_eq!(parse(symbol).unwrap().to_string(), expected, "{symbol}");
        }
    }

    #[test]
    fn parses_main_program() {
        assert_eq!(parse("MAIN__").unwrap().to_string(), "main");
    }

    #[test]
    fn strips_platform_underscore_on_module_form() {
        assert_eq!(
            parse("___fprobe_MOD_add").unwrap().to_string(),
            "fprobe.add"
        );
        assert_eq!(parse("___m_MOD_x").unwrap().to_string(), "m.x");
    }

    #[test]
    fn strips_linker_decorations() {
        assert_eq!(parse("__m_MOD_x.llvm.1234").unwrap().to_string(), "m.x");
        assert_eq!(parse("__m_MOD_x@@GFORTRAN_1").unwrap().to_string(), "m.x");
        assert_eq!(
            parse("ext_sub_@@GFORTRAN_1").unwrap().to_string(),
            "ext_sub"
        );
    }

    #[test]
    fn parses_derived_type_helpers() {
        for (symbol, expected) in [
            ("__fprobe_MOD___vtab_fprobe_Point", "fprobe.Point'vtab"),
            (
                "__fprobe_MOD___def_init_fprobe_Point",
                "fprobe.Point'def_init",
            ),
            ("__fprobe_MOD___copy_fprobe_Point", "fprobe.Point'copy"),
            ("__fprobe_MOD___final_fprobe_Point", "fprobe.Point'final"),
            ("__m_MOD___vtab_m_My_type", "m.My_type'vtab"),
            ("__m_MOD___copy_m_Camelcase", "m.Camelcase'copy"),
            (
                "__m_MOD___def_init_m_T_with_underscore_",
                "m.T_with_underscore_'def_init",
            ),
            ("___m_MOD___vtab_m_T", "m.T'vtab"),
            // A parameterized derived-type instantiation keeps the shape.
            (
                "__holder_MOD___vtab_holder_Pdtkdt_2",
                "holder.Pdtkdt_2'vtab",
            ),
        ] {
            assert_eq!(parse(symbol).unwrap().to_string(), expected, "{symbol}");
        }
    }

    #[test]
    fn rejects_unreadable_derived_type_helpers() {
        for symbol in [
            // The helper repeats a module that is not the symbol's own.
            "__holder_MOD___vtab___iso_c_binding_C_ptr",
            "__m_MOD___vtab_other_T",
            // The type part is missing or not a gfortran type spelling.
            "__m_MOD___vtab_m",
            "__m_MOD___vtab_m_",
            "__m_MOD___vtab_m_point",
            "__m_MOD___vtab_m_Point_X",
            "__m_MOD___vtab_m_Point.ext",
            "__m_MOD___vtabs_m_Point",
            // Submodule symbols carry a `.` in the module name.
            "__outer.inner_MOD___vtab_outer.inner_Deep_t",
        ] {
            assert!(parse(symbol).is_err(), "{symbol:?} should be rejected");
        }
    }

    #[test]
    fn rejects_ambiguous_external_names() {
        for symbol in [
            // `x_y__` is `x_y_` (default) or `x_y` (`-fsecond-underscore`).
            "ext_underscored__",
            "ext_underscored___",
            "my_ext_sub__",
            "probe_block__",
            "a_b__",
        ] {
            assert!(parse(symbol).is_err(), "{symbol} should be rejected");
        }
    }

    #[test]
    fn rejects_foreign_and_unsupported_forms() {
        for symbol in [
            // No underscoring: indistinguishable from C.
            "ext_sub",
            "main",
            "probe_blk",
            // `bind(c)` names carry no decoration.
            "c_proc_binding",
            "ext_c",
            // Uppercase cannot come from a gfortran Fortran name.
            "Foo_",
            "MAIN___",
            "MAIN",
            // Local symbols of internal and host-associated procedures.
            "inner.0",
            "master.0.with_entry_",
            // Mach-O external names are not claimed in this slice.
            "__ext_sub_",
            // Malformed module forms.
            "__m_MOD_",
            "__m_MOD_X",
            "__m_OD_x",
            "__MOD_x",
            "m_MOD_x",
            "_m_MOD_x",
            "___MOD_x",
            "__m_MOD_x.y",
            "__m__MOD_",
            // An uppercase marker inside the member cannot be a Fortran name.
            "__m_MOD_x_MOD_y",
        ] {
            assert!(parse(symbol).is_err(), "{symbol:?} should be rejected");
        }
    }

    #[test]
    fn rejects_malformed_names() {
        for symbol in ["", "_", "__", "___", "MOD", "1x_"] {
            assert!(parse(symbol).is_err(), "{symbol:?} should be rejected");
        }
    }

    #[test]
    fn rejects_invalid_utf8() {
        // A module shape is claimed by detection, then rejected on parse.
        assert!(parse_bytes(b"__m_MOD_x\x80").is_err());
        assert!(parse_bytes(b"ext\x80_").is_err());
        assert!(!is_fortran(b"ext\x80_"));
    }

    #[test]
    fn budgets_are_enforced() {
        let long = "a".repeat(MAX_INPUT_LEN + 1);
        assert!(matches!(parse(&long), Err(ParseError::BudgetExceeded)));
        let long_module = alloc::format!("__m_MOD_{long}");
        assert!(matches!(
            parse(&long_module),
            Err(ParseError::BudgetExceeded)
        ));
        assert!(!is_fortran(long_module.as_bytes()));
        // The longest accepted name still parses.
        let limit = alloc::format!("{}_", "a".repeat(MAX_INPUT_LEN - 1));
        assert_eq!(parse(&limit).unwrap().to_string().len(), MAX_INPUT_LEN - 1);
    }

    #[test]
    fn detection_is_precision_only() {
        for symbol in [
            "__fprobe_MOD_add",
            "__fprobe_MOD___vtab_fprobe_Point",
            "___fprobe_MOD_add",
            "__my_mod__MOD_var_x_",
            "__m_MOD_x.llvm.1234",
        ] {
            assert!(is_fortran(symbol.as_bytes()), "{symbol} should be Fortran");
        }
        for symbol in [
            "foo_",
            "foo__",
            "foo",
            "main",
            "MAIN__",
            "ext_sub_",
            "m_MOD_x",
            "_m_MOD_x",
            "__foo",
            "__m_OD_x",
            "c_proc_binding",
        ] {
            assert!(
                !is_fortran(symbol.as_bytes()),
                "{symbol} should not be Fortran"
            );
        }
    }
}

#[cfg(test)]
mod budget_tests {
    use alloc::format;
    use alloc::string::ToString;

    use super::node::{HARD_INPUT_LEN, HARD_WORK};
    use super::*;
    use crate::{Budget, Demangler, Scheme};

    /// Parse `symbol` with `budget` resolved for [`Scheme::Fortran`].
    fn parse_with(symbol: &[u8], budget: Budget) -> Result<Ast<'_>, ParseError> {
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
        assert_eq!(
            Scheme::Fortran.default_budget().effective(Scheme::Fortran),
            DEFAULT_BUDGET
        );

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
            Err(ParseError::BudgetExceeded)
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
            Err(ParseError::BudgetExceeded)
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
            Err(ParseError::BudgetExceeded)
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

    /// The fixed output cap is a backstop, not a print budget.
    ///
    /// Printing drops the `__` prefix and the `_MOD_` marker, so the text is
    /// shorter than the input. The longest name the default input cap accepts
    /// therefore still fits, and the corpus gate exercises the same bound over
    /// every symbol.
    #[test]
    fn the_output_cap_is_a_backstop_the_printer_never_reaches() {
        let symbol = format!("{}_", "a".repeat(MAX_INPUT_LEN - 1));
        let rendered = parse(&symbol)
            .expect("the default input cap parses")
            .to_string();
        assert!(rendered.len() <= MAX_OUTPUT_LEN);
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
