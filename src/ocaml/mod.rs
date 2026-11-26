//! Pure-Rust OCaml native linker-name demangling.
//!
//! OCaml's native compiler gives every compilation unit and every exported
//! value one linker symbol. The name is the `caml` prefix, the compilation-unit
//! path, the member name and a numeric stamp that keeps the symbol unique; the
//! unit path is joined with `.`, and every byte outside `[A-Za-z0-9_.]` is
//! written as a `$XX` escape. There is no type information in the name at all,
//! so nothing beyond the path, the member and the escape decoding can be
//! recovered. `refs/ocaml.md` records the measured grammar, the fixture and the
//! pinned corpus.
//!
//! # Supported forms
//!
//! - module records: `caml<path>` (`camlStdlib__List` -> `Stdlib__List`,
//!   `camlFoo$27` -> `Foo'`);
//! - values: `caml<path>.<member>_<stamp>`, with the stamp dropped
//!   (`camlStdlib__List.map_569` -> `Stdlib__List.map`);
//! - operator and other escaped members: `camlOps.$2b$2b_270` -> `Ops.++`,
//!   `camlCamlinternalFormat.k$27_3650` -> `CamlinternalFormat.k'`;
//! - packed units, whose pack name becomes another path segment
//!   (`camlOcaml_pack.Ocaml_pack_base.f_270` ->
//!   `Ocaml_pack.Ocaml_pack_base.f`);
//! - the compiler's section markers, the only members without a stamp
//!   (`camlProbe.entry`, `camlProbe.code_begin`, `camlProbe.frametable`, …);
//! - compiler-emitted static-data labels, whose member is a bare number and
//!   which print as it stands (`camlStdlib.11` -> `Stdlib.11`);
//! - runtime helpers: `caml_<name>` (`caml_alloc`, `caml_ml_open_descriptor_out`,
//!   `caml_program`), printed as they stand;
//! - the runtime's C-level pseudo-units, whose markers keep the `caml_` prefix
//!   (`caml_startup.code_begin` -> `startup.code_begin`, `caml_hot.frametable`,
//!   `caml_system.frametable`) and the assembler's doubled-underscore spelling of
//!   the same markers (`caml_system__code_begin` -> `system__code_begin`);
//! - a host C compiler's fragment suffix on a runtime helper: `.cold` (GCC's
//!   outlined cold half) and `.part.<N>` (partial inlining). The suffix is not
//!   part of the name, so `caml_ba_compare.cold` prints as `caml_ba_compare`;
//! - one optional leading platform underscore (Darwin, PE), which the printed
//!   text drops;
//! - a trailing ThinLTO decoration (`.llvm.<hex>`) is stripped before parsing,
//!   matching the other backends.
//!
//! # Canonical output
//!
//! OCaml ships no demangler, so the printed text is this crate's contract:
//!
//! | Input | Output |
//! |---|---|
//! | `camlProbe` | `Probe` |
//! | `camlProbe.unit_proc_271` | `Probe.unit_proc` |
//! | `camlStdlib__List.map_569` | `Stdlib__List.map` |
//! | `camlOps.$2b$2b_270` | `Ops.++` |
//! | `camlOcaml_pack.Ocaml_pack_base.f_270` | `Ocaml_pack.Ocaml_pack_base.f` |
//! | `camlProbe.entry` | `Probe.entry` |
//! | `camlStdlib.11` | `Stdlib.11` |
//! | `caml_alloc` | `caml_alloc` |
//! | `caml_startup.code_begin` | `startup.code_begin` |
//! | `caml_system__code_begin` | `system__code_begin` |
//! | `caml_ba_compare.cold` | `caml_ba_compare` |
//!
//! The path and the member are joined with `.`, the numeric stamp is dropped and
//! the `$XX` escapes are decoded. A static-data label has no name of its own, so
//! it keeps the number the compiler gave it. The unit path is printed as
//! mangled: the source spelling of a unit whose name contains `'` is recoverable
//! through the escape decoding, but a nested module's path is not in the symbol
//! at all — values of a submodule are emitted under the enclosing compilation
//! unit (`Probe.Nested.Deep.deep_fun` links as `camlProbe.deep_fun_295`).
//!
//! A runtime helper is printed as it stands: `caml_alloc` already is the C
//! identifier, so the backend only classifies it. The corpus records those rows
//! as `identity`, never as reconstructed text.
//!
//! # Detection
//!
//! [`crate::Scheme::detect`] claims a symbol only after
//! [`parse`](crate::ocaml::parse) accepts the whole name. `caml` alone is not a prefix: `camel_alloc`, `camlist.x` and a
//! `caml_` name with an empty or doubled segment are rejected, and so is a
//! `caml<path>.<member>` whose member carries neither a numeric stamp nor a
//! compiler marker.
//!
//! `Scheme::detect` runs this backend before Go, so dotted `caml...` names are
//! not stolen by the Go heuristic.
//!
//! # Budgets
//!
//! Untrusted input stays bounded by [`MAX_INPUT_LEN`](crate::ocaml::MAX_INPUT_LEN),
//! [`MAX_PATH_SEGMENTS`](crate::ocaml::MAX_PATH_SEGMENTS),
//! [`MAX_WORK`](crate::ocaml::MAX_WORK) and
//! [`MAX_OUTPUT_LEN`](crate::ocaml::MAX_OUTPUT_LEN). Exceeding a budget is an
//! error, never a partial result. The grammar is flat, so there is
//! no recursive depth to bound; the path-segment limit is the node budget.
//!
//! The input and work limits are runtime-configurable.
//! [`parse`](crate::ocaml::parse) keeps the compiled-in defaults, and a
//! [`crate::Demangler`] passes the configured budget, clamped to the hard
//! ceilings of 16 MiB input and 64M work units. The two dimensions are
//! independent: raising the input limit alone keeps the default work allowance,
//! so a caller that raises the input for a long symbol must raise
//! [`crate::Budget::work`] as well when the scan exceeds it. The grammar has no
//! depth or node dimension to configure, and the path-segment limit is a fixed
//! backend extra rather than a budget.
//!
//! Printing is not a budget: the printer walks each form once, so the output is
//! linear in the linker name and [`MAX_OUTPUT_LEN`](crate::ocaml::MAX_OUTPUT_LEN)
//! is a fixed backstop for a parser-built name. That backstop is sized from
//! [`MAX_INPUT_LEN`](crate::ocaml::MAX_INPUT_LEN), so a caller that raises the
//! input budget past it accepts a name whose printed form no longer fits and
//! `Display` reports the overflow instead of truncating.
//!
//! # Gaps
//!
//! Unsupported forms are rejected, not guessed:
//!
//! - C section suffixes on a *module* symbol (`camlFoo.f_1.cold`): the host
//!   compiler never sees OCaml-generated code, so the suffix was not measured
//!   there and only the runtime-helper form is peeled;
//! - a `.cold.<N>` LLVM index on a runtime helper, which the linked runtime was
//!   not observed to produce;
//! - bytecode global-table names: they are not linker symbols, and the bytecode
//!   compiler's spelling is not verified here;
//! - local labels (`.L…`) and anything else the assembler emits;
//! - the Mach-O/PE spelling of the path separator: only the `.` form the ELF
//!   fixture produces is verified, so a `camlFoo__bar_1` name is not claimed;
//! - OCaml type, arity and signature information, which the linker name does not
//!   carry.

mod node;
mod parser;
mod printer;

use core::fmt;

use crate::budget::{EffectiveBudget, Limit};
use crate::parse::LimitedWriter;
use node::ParseLimits;
pub use node::{Ast, MAX_INPUT_LEN, MAX_OUTPUT_LEN, MAX_PATH_SEGMENTS, MAX_WORK};
pub use parser::is_ocaml;

/// Why an OCaml symbol could not be parsed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ParseError {
    /// The symbol does not match a supported `caml` form.
    Invalid,
    /// The input, work or path-segment budget was exceeded.
    BudgetExceeded,
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
    /// Render the parsed name into `out`, enforcing
    /// [`MAX_OUTPUT_LEN`](crate::ocaml::MAX_OUTPUT_LEN).
    pub(crate) fn print<W: fmt::Write + ?Sized>(&self, out: &mut W) -> fmt::Result {
        let mut limited = LimitedWriter::new(out, MAX_OUTPUT_LEN);
        printer::print(&self.form, &mut limited)
    }
}

/// Parse an OCaml linker name into a reusable [`Ast`].
///
/// # Errors
///
/// Returns [`ParseError::Invalid`] when the symbol is not a supported `caml`
/// form (including the C-section, bytecode and Mach-O-separator forms listed in
/// the module docs), and [`ParseError::BudgetExceeded`] when the input, work or
/// path-segment budget is exceeded.
pub fn parse(symbol: &str) -> Result<Ast<'_>, ParseError> {
    parse_bytes(symbol.as_bytes())
}

/// The budget the public entry point starts from.
///
/// [`parse`] uses it, so it keeps exactly the compiled-in limits; a
/// [`crate::Demangler`] resolves its configured [`crate::Budget`] for
/// [`Scheme::OCaml`](crate::Scheme::OCaml) instead.
pub(crate) const DEFAULT_BUDGET: EffectiveBudget = EffectiveBudget {
    input_bytes: Some(Limit::Bounded(MAX_INPUT_LEN)),
    parse_depth: None,
    work: Some(Limit::Bounded(MAX_WORK)),
    nodes: None,
};

/// Parse an OCaml linker name from raw bytes.
///
/// This is the entry point the demangler dispatch uses once symbol bytes, not
/// text, are the input. See [`parse`](crate::ocaml::parse) for the accepted
/// forms; the only difference is that the name does not have to be UTF-8.
pub(crate) fn parse_bytes(symbol: &[u8]) -> Result<Ast<'_>, ParseError> {
    parse_bytes_with_budget(symbol, DEFAULT_BUDGET)
}

/// Parse a raw symbol byte string with one scheme's resolved budget.
///
/// The public [`parse`] wraps this with [`DEFAULT_BUDGET`]; a
/// [`crate::Demangler`] passes [`Demangler::effective_budget`] for
/// [`Scheme::OCaml`](crate::Scheme::OCaml), so a caller's overrides reach the
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

#[cfg(test)]
mod tests {
    use alloc::format;
    use alloc::string::{String, ToString};

    use super::*;

    #[test]
    fn parses_module_records() {
        for (symbol, expected) in [
            ("camlProbe", "Probe"),
            ("camlStdlib__List", "Stdlib__List"),
            ("camlCamlinternalFormatBasics", "CamlinternalFormatBasics"),
            ("camlFoo$27", "Foo'"),
            (
                "camlOcaml_pack.Ocaml_pack_base",
                "Ocaml_pack.Ocaml_pack_base",
            ),
            ("_camlProbe", "Probe"),
        ] {
            assert_eq!(render(symbol), expected, "{symbol}");
        }
    }

    #[test]
    fn strips_the_stamp_from_values() {
        for (symbol, expected) in [
            ("camlProbe.unit_proc_271", "Probe.unit_proc"),
            ("camlStdlib__List.map_569", "Stdlib__List.map"),
            ("camlProbe.foo__bar_270", "Probe.foo__bar"),
            ("camlProbe.upper_Case_Fun_274", "Probe.upper_Case_Fun"),
            ("camlProbe.fun_350", "Probe.fun"),
            (
                "camlOcaml_pack.Ocaml_pack_base.f_270",
                "Ocaml_pack.Ocaml_pack_base.f",
            ),
            ("camlProbe.foo_2_271", "Probe.foo_2"),
        ] {
            assert_eq!(render(symbol), expected, "{symbol}");
        }
    }

    #[test]
    fn decodes_operator_escapes() {
        for (symbol, expected) in [
            ("camlOps.$2b$2b_270", "Ops.++"),
            ("camlOps.$3d$3d$21_274", "Ops.==!"),
            ("camlOps.$40$40_278", "Ops.@@"),
            ("camlOps.$7e$2d_348", "Ops.~-"),
            ("camlOps..$25$28$29_289", "Ops..%()"),
            // A functor is a module, so its member starts with an uppercase
            // letter; the path only grows when the rest is not a member.
            ("camlOcaml_probe.F_310", "Ocaml_probe.F"),
            (
                "camlOcaml_pack.Ocaml_inner.Ocaml_pack_inner.deep_fun_271",
                "Ocaml_pack.Ocaml_inner.Ocaml_pack_inner.deep_fun",
            ),
            ("camlCamlinternalFormat.k$27_3650", "CamlinternalFormat.k'"),
            ("camlFoo$27.get_271", "Foo'.get"),
        ] {
            assert_eq!(render(symbol), expected, "{symbol}");
        }
    }

    #[test]
    fn escapes_backslash_reversibly() {
        // `$5c` is a decoded backslash, not literal payload text. It must use
        // the reversible identifier escape so the encoding layer can decode it.
        assert_eq!(render("camlProbe.a$5cb_1"), "Probe.a\\\\b");
        assert_eq!(
            render("camlProbe.a$5c$78$34$31_1"),
            "Probe.a\\\\x41"
        );
    }

    #[test]
    fn keeps_a_hand_written_dot_escape() {
        // The compiler never escapes `.`; a `$2e` stays verbatim so the printed
        // path keeps exactly one separator per segment.
        assert_eq!(render("camlProbe.a$2eb_1"), "Probe.a$2eb");
        assert_eq!(render("camlProbe.a$00b_1"), "Probe.a$00b");
    }

    #[test]
    fn prints_section_markers() {
        for (symbol, expected) in [
            ("camlProbe.entry", "Probe.entry"),
            ("camlProbe.code_begin", "Probe.code_begin"),
            ("camlProbe.code_end", "Probe.code_end"),
            ("camlProbe.data_begin", "Probe.data_begin"),
            ("camlProbe.data_end", "Probe.data_end"),
            ("camlProbe.frametable", "Probe.frametable"),
            ("camlProbe.gc_roots", "Probe.gc_roots"),
            (
                "camlOcaml_pack.Ocaml_pack_base.entry",
                "Ocaml_pack.Ocaml_pack_base.entry",
            ),
        ] {
            assert_eq!(render(symbol), expected, "{symbol}");
        }
    }

    #[test]
    fn prints_runtime_helpers_as_they_stand() {
        for symbol in [
            "caml_alloc",
            "caml_alloc1",
            "caml_ml_open_descriptor_out",
            "caml_program",
            "caml_startup",
            "caml_main",
            "caml_MD5Init",
            "caml_ba_get_N",
        ] {
            assert_eq!(render(symbol), symbol, "{symbol}");
            assert_eq!(render(&format!("_{symbol}")), symbol, "_{symbol}");
        }
    }

    #[test]
    fn prints_runtime_pseudo_unit_markers() {
        for (symbol, expected) in [
            ("caml_startup.code_begin", "startup.code_begin"),
            ("caml_startup.frametable", "startup.frametable"),
            ("caml_hot.code_begin", "hot.code_begin"),
            ("caml_hot.code_end", "hot.code_end"),
            ("caml_system.frametable", "system.frametable"),
            // The assembler's doubled-underscore spelling keeps both underscores,
            // so it stays distinguishable from the dotted marker.
            ("caml_system__code_begin", "system__code_begin"),
            ("caml_system__code_end", "system__code_end"),
        ] {
            assert_eq!(render(symbol), expected, "{symbol}");
        }
    }

    #[test]
    fn prints_static_data_labels() {
        for (symbol, expected) in [
            ("camlStdlib.11", "Stdlib.11"),
            ("camlBc.1", "Bc.1"),
            ("camlOcaml_probe.3", "Ocaml_probe.3"),
            ("camlCamlinternalFormat.116", "CamlinternalFormat.116"),
            (
                "camlOcaml_pack.Ocaml_pack_base.2",
                "Ocaml_pack.Ocaml_pack_base.2",
            ),
            ("camlOcaml_prime$27.4", "Ocaml_prime'.4"),
        ] {
            assert_eq!(render(symbol), expected, "{symbol}");
        }
    }

    #[test]
    fn peels_host_compiler_fragments() {
        for (symbol, expected) in [
            ("caml_ba_compare.cold", "caml_ba_compare"),
            (
                "caml_fatal_uncaught_exception.cold",
                "caml_fatal_uncaught_exception",
            ),
            ("caml_ephe_clean.part.0", "caml_ephe_clean"),
            ("caml_sys_check_path.part.1", "caml_sys_check_path"),
            ("_caml_ba_hash.cold", "caml_ba_hash"),
        ] {
            assert_eq!(render(symbol), expected, "{symbol}");
        }
    }

    #[test]
    fn strips_linker_decorations() {
        assert_eq!(render("camlProbe.entry.llvm.1234"), "Probe.entry");
        assert_eq!(render("camlProbe.unit_proc_271.llvm.9F"), "Probe.unit_proc");
    }

    #[test]
    fn rejects_unsupported_forms() {
        for symbol in [
            // Fragments are only peeled from a runtime helper.
            "camlProbe.f_1.cold",
            "camlProbe.entry.part.0",
            // A fragment suffix must be well formed.
            "caml_ba_compare.cold.1",
            "caml_ba_compare.part.",
            "caml_ba_compare.part.x",
            "caml_ba_compare.coldx",
            // A pseudo-unit that the runtime does not have, and a non-marker.
            "caml_other.entry",
            "caml_hot.entry_1",
            "caml_other__code_begin",
            "caml_hot__entry_1",
        ] {
            assert!(parse(symbol).is_err(), "{symbol} should be rejected");
        }
    }

    #[test]
    fn rejects_malformed_names() {
        for symbol in [
            "",
            "caml",
            "caml_",
            "caml__alloc",
            "caml_alloc_",
            "caml_alloc__x",
            "camlProbe.",
            "camlProbe.entry_1_",
            "camlProbe._271",
            "camlOps..foo_1",
            "camlProbe.foo.1",
            "camlProbe.foo",
            "camlProbe.-foo_1",
            "camlProbe.foo_1x",
            "camlProbe.1x",
            "camlProbe.1.2",
            "camlProbe.11_",
            "camlprobe.foo_1",
            "caml1Probe.foo_1",
            "camlProbe.foo$2_1",
            "camlProbe.foo$2B_1",
            "camlProbe.foo$ZZ_1",
            "camlProbe.foo bar_1",
            "camlProbe.foo/bar_1",
            "camelProbe.foo_1",
            "camlist.x",
            "_Z3foov",
            "_RNvNtCs1234_7mycrate3foo",
        ] {
            assert!(parse(symbol).is_err(), "{symbol:?} should be rejected");
        }
    }

    #[test]
    fn budgets_are_enforced() {
        let long = format!("caml{}", "A".repeat(MAX_INPUT_LEN));
        assert!(matches!(parse(&long), Err(ParseError::BudgetExceeded)));

        let mut deep = String::from("camlA");
        for _ in 0..MAX_PATH_SEGMENTS {
            deep.push_str(".A");
        }
        assert!(matches!(parse(&deep), Err(ParseError::BudgetExceeded)));
    }

    #[test]
    fn rejects_non_identifier_bytes() {
        // The compiler escapes every byte outside `[A-Za-z0-9_.]`, so a raw
        // non-ASCII byte is not part of a name.
        for symbol in [
            b"camlProbe.fo\xffo_1".as_slice(),
            b"camlPro\xc3\xa9be.entry".as_slice(),
            b"caml_alloc\x00".as_slice(),
        ] {
            assert!(parse_bytes(symbol).is_err(), "{symbol:?}");
        }
        assert_eq!(
            parse_bytes(b"camlProbe.unit_proc_271").unwrap().to_string(),
            "Probe.unit_proc"
        );
    }

    #[test]
    fn detection_requires_a_full_parse() {
        for symbol in [
            "camlProbe",
            "camlStdlib__List.map_569",
            "camlProbe.entry",
            "caml_alloc",
            "caml_program",
            "caml_startup.code_begin",
        ] {
            assert!(is_ocaml(symbol.as_bytes()), "{symbol} should be OCaml");
        }
        for symbol in [
            "main",
            "caml",
            "camel",
            "camel_alloc",
            "camlist.x",
            "caml_",
            "caml__alloc",
            "camlProbe.foo",
            "camlProbe.foo.1",
            "caml_ba_compare.cold.1",
            "_Z3foov",
            "_RNvNtCs1234_7mycrate3foo",
            "kfun:com.example.Foo#bar",
            "Java_com_example_Foo_bar",
        ] {
            assert!(!is_ocaml(symbol.as_bytes()), "{symbol} should not be OCaml");
        }
    }

    /// Render a symbol, panicking with the symbol on a parse error.
    fn render(symbol: &str) -> String {
        parse(symbol)
            .unwrap_or_else(|error| panic!("{symbol}: {error:?}"))
            .to_string()
    }
}

#[cfg(test)]
mod budget_tests {
    use alloc::format;
    use alloc::string::ToString;

    use super::node::{HARD_INPUT_LEN, HARD_WORK};
    use super::*;
    use crate::{Budget, Demangler, Scheme};

    /// Parse `symbol` with `budget` resolved for [`Scheme::OCaml`].
    fn parse_with(symbol: &[u8], budget: Budget) -> Result<Ast<'_>, ParseError> {
        parse_bytes_with_budget(symbol, budget.effective(Scheme::OCaml))
    }

    /// Whether `symbol` parses with one budget dimension set to `value`.
    fn parses_with(symbol: &[u8], dimension: impl Fn(u32) -> Budget, value: u32) -> bool {
        parse_with(symbol, dimension(value)).is_ok()
    }

    /// Assert that `need` is the exact boundary of one dimension for `symbol`.
    ///
    /// The dimension is the only override, so a failure one unit below the
    /// boundary comes from that dimension and not from another limit.
    fn assert_dimension_boundary(symbol: &str, name: &str, dimension: impl Fn(u32) -> Budget) {
        let bytes = symbol.as_bytes();
        let ceiling = u32::try_from(bytes.len()).unwrap_or(u32::MAX) + 64;
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

    /// A module record whose single path segment is `segment_len` bytes long.
    fn module_with_segment_len(segment_len: usize) -> alloc::string::String {
        format!("caml{}", "A".repeat(segment_len))
    }

    #[test]
    fn the_compiled_in_budget_matches_the_policy_defaults() {
        let limits = ParseLimits::from_effective(DEFAULT_BUDGET);
        assert_eq!(limits.input, MAX_INPUT_LEN);
        assert_eq!(limits.work, MAX_WORK);
        assert_eq!(
            Scheme::OCaml.default_budget().effective(Scheme::OCaml),
            DEFAULT_BUDGET
        );

        // The policy table and this module agree on the ceilings, and the flat
        // grammar has no depth or node dimension to raise.
        let hard = Scheme::OCaml.hard_budget();
        assert_eq!(hard.input_bytes, Some(Limit::Bounded(HARD_INPUT_LEN)));
        assert_eq!(hard.parse_depth, None);
        assert_eq!(hard.work, Some(Limit::Bounded(HARD_WORK)));
        assert_eq!(hard.nodes, None);
    }

    #[test]
    fn each_dimension_rejects_one_unit_below_what_the_symbol_needs() {
        for symbol in [
            "camlProbe.unit_proc_271",
            "camlStdlib__List.map_569",
            "caml_startup",
        ] {
            assert_dimension_boundary(symbol, "input", |value| {
                Budget::new().with_input_bytes(value as usize)
            });
            assert_dimension_boundary(symbol, "work", |value| Budget::new().with_work(value));
        }
    }

    #[test]
    fn raising_the_input_budget_accepts_a_longer_symbol() {
        // Four bytes past the default cap, in the module path.
        let symbol = module_with_segment_len(MAX_INPUT_LEN);
        assert!(matches!(
            parse_with(symbol.as_bytes(), Budget::new()),
            Err(ParseError::BudgetExceeded)
        ));
        let raised = Budget::new().with_input_bytes(2 * MAX_INPUT_LEN);
        let ast = parse_with(symbol.as_bytes(), raised).expect("raised input budget");
        // Printing drops the `caml` prefix.
        assert_eq!(ast.to_string().len(), symbol.len() - 4);
    }

    /// The input and work dimensions are independent: raising the input alone
    /// keeps the default work allowance, so a scan that exceeds it still fails.
    #[test]
    fn raising_the_input_budget_does_not_raise_the_work_budget() {
        // The parser charges one work unit per byte, so this symbol needs more
        // work than the default allowance. It is not printed: the fixed output
        // cap only covers a name accepted at the default input cap.
        let symbol = module_with_segment_len(600_000);
        assert!(symbol.len() > MAX_WORK);
        let work = u32::try_from(symbol.len()).expect("work fits a u32");

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
            .with_work(u32::MAX);
        let limits = ParseLimits::from_effective(budget.effective(Scheme::OCaml));
        assert_eq!(limits.input, HARD_INPUT_LEN);
        assert_eq!(limits.work, HARD_WORK);
        assert_eq!(
            budget.effective(Scheme::OCaml).input_bytes,
            Some(Limit::Bounded(HARD_INPUT_LEN))
        );

        // A name past the hard input cap stays rejected even when the caller
        // asks for more.
        let too_long = module_with_segment_len(HARD_INPUT_LEN);
        assert!(matches!(
            parse_with(too_long.as_bytes(), budget),
            Err(ParseError::BudgetExceeded)
        ));
    }

    #[test]
    fn a_zero_budget_rejects_through_every_entry_point() {
        let symbol = "camlProbe.unit_proc_271";
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
                    .demangle_with_scheme(Scheme::OCaml, symbol)
                    .is_err(),
                "{budget:?}"
            );
            assert!(
                demangler
                    .selection()
                    .allow(Scheme::OCaml)
                    .demangle(symbol)
                    .is_err(),
                "{budget:?}"
            );
        }
    }

    /// The fixed output cap is a backstop, not a print budget.
    ///
    /// Printing drops the `caml` prefix, the stamp and a host-compiler fragment
    /// and decodes `$XX` escapes, so the text is never longer than the input.
    /// The longest name the default input cap accepts still fits the cap, and
    /// the corpus gate exercises the same bound over every symbol.
    #[test]
    fn the_output_cap_is_a_backstop_the_printer_never_reaches() {
        let symbol = module_with_segment_len(MAX_INPUT_LEN - 4);
        assert_eq!(symbol.len(), MAX_INPUT_LEN);
        let rendered = parse(&symbol)
            .expect("the default input cap parses")
            .to_string();
        assert!(rendered.len() <= MAX_OUTPUT_LEN);
        assert!(rendered.len() < symbol.len());
    }

    #[test]
    fn the_default_budget_parses_the_corpus_like_parse() {
        let mut compared = 0_usize;
        let mut present = 0_usize;
        for name in [
            "fixture.tsv",
            "fragments.tsv",
            "link.tsv",
            "runtime.tsv",
            "stdlib.tsv",
        ] {
            let Ok(data) = std::fs::read_to_string(format!(
                "{}/refs/ocaml/corpus/{name}",
                env!("CARGO_MANIFEST_DIR")
            )) else {
                continue;
            };
            present += 1;
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
                    "{name}: {symbol}: the default budget changes acceptance"
                );
                let (Ok(expected), Ok(explicit)) = (expected, explicit) else {
                    continue;
                };
                assert_eq!(
                    expected.to_string(),
                    explicit.to_string(),
                    "{name}: {symbol}"
                );
                compared += 1;
            }
        }
        if present == 0 {
            std::eprintln!("ocaml corpus not present (published crate); skipping");
            return;
        }
        assert!(compared > 0, "the default budget compared no symbols");
    }
}
