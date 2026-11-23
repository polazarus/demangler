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
//! - runtime helpers: `caml_<name>` (`caml_alloc`, `caml_ml_open_descriptor_out`,
//!   `caml_program`), printed as they stand;
//! - the runtime's C-level pseudo-units, whose markers keep the `caml_` prefix
//!   (`caml_startup.code_begin` -> `startup.code_begin`, `caml_hot.frametable`,
//!   `caml_system.frametable`);
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
//! | `caml_alloc` | `caml_alloc` |
//! | `caml_startup.code_begin` | `startup.code_begin` |
//!
//! The path and the member are joined with `.`, the numeric stamp is dropped and
//! the `$XX` escapes are decoded. The unit path is printed as mangled: the
//! source spelling of a unit whose name contains `'` is recoverable through the
//! escape decoding, but a nested module's path is not in the symbol at all —
//! values of a submodule are emitted under the enclosing compilation unit
//! (`Probe.Nested.Deep.deep_fun` links as `camlProbe.deep_fun_295`).
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
//! # Gaps
//!
//! The first slice rejects, rather than guesses:
//!
//! - compiler-generated static-data labels, whose member is a bare number
//!   (`camlStdlib.11`, `camlBc.1`);
//! - the runtime's double-underscore markers (`caml_system__code_begin`,
//!   `caml_system__code_end`), which are not `caml<path>.<member>` names;
//! - C section suffixes the host compiler adds to runtime functions
//!   (`caml_ba_compare.cold`, `caml_foo.part.0`), which are not OCaml names;
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

use crate::parse::LimitedWriter;
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
/// form (including the static-data, double-underscore and C-section forms
/// listed in the module docs), and [`ParseError::BudgetExceeded`] when the
/// input, work or path-segment budget is exceeded.
pub fn parse(symbol: &str) -> Result<Ast<'_>, ParseError> {
    parse_bytes(symbol.as_bytes())
}

/// Parse an OCaml linker name from raw bytes.
///
/// This is the entry point the demangler dispatch uses once symbol bytes, not
/// text, are the input. See [`parse`](crate::ocaml::parse) for the accepted
/// forms; the only difference is that the name does not have to be UTF-8.
pub(crate) fn parse_bytes(symbol: &[u8]) -> Result<Ast<'_>, ParseError> {
    parser::parse_bytes(symbol)
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
            // Static-data labels are numbers, not names.
            "camlStdlib.11",
            "camlBc.1",
            // The runtime's double-underscore markers.
            "caml_system__code_begin",
            "caml_system__code_end",
            // Host-compiler section suffixes on runtime functions.
            "caml_ba_compare.cold",
            "caml_fatal_uncaught_exception.cold",
            "caml_foo.part.0",
            // A pseudo-unit that the runtime does not have, and a non-marker.
            "caml_other.entry",
            "caml_hot.entry_1",
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
            "camlProbe.11",
            "caml_ba_compare.cold",
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
