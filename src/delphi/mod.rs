//! Pure-Rust Delphi/Object Pascal linker-name demangling.
//!
//! Delphi and Free Pascal do not mangle types in the C++ sense: the compiler
//! builds a linker name from the symbol's owning scope chain, the symbol name
//! and, for a routine, one token per parameter type. Free Pascal's scheme is
//! documented in `refs/delphi.md`; this backend implements the routine and
//! compiler-data subset of it in pure Rust, with no runtime dependency and no
//! external demangler.
//!
//! # Supported forms
//!
//! - unit-scope routines: `<unit>_$$_<name>[$<type>…][$$<result>]`
//!   (`delphi_probe_$$_unitproc$longint` -> `DELPHI_PROBE.UNITPROC(LONGINT)`);
//! - methods of classes and records, including nested types:
//!   `<unit>$_$<class>[_$_<nested>…]_$__$$_<name>…`
//!   (`DELPHI_PROBE$_$TPROBECLASS_$__$$_PLAIN` ->
//!   `DELPHI_PROBE.TPROBECLASS.PLAIN()`);
//! - operator and class-constructor names, printed as mangled:
//!   `DELPHI_PROBE_$$_$plus$TPROBEREC$TPROBEREC$$TPROBEREC` ->
//!   `DELPHI_PROBE.$plus(TPROBEREC, TPROBEREC): TPROBEREC`;
//! - open-array parameters: `array_of_X` prints as `array of X`, and
//!   `array_of_const` as `array of const`;
//! - namespaced units (`ns.esc`), whose dots are kept
//!   (`NS.ESC$_$TFOO_BAR_$__$$_METHODTWO$LONGINT` ->
//!   `NS.ESC.TFOO_BAR.METHODTWO(LONGINT)`);
//! - program-scope routines, whose unit part carries Free Pascal's `P$` marker
//!   (`P$DELPHI_PROBE_PROG_$$_PROGPROC$LONGINT` ->
//!   `P$DELPHI_PROBE_PROG.PROGPROC(LONGINT)`);
//! - compiler-generated data symbols, which reuse the same chain behind a fixed
//!   prefix that the printed text keeps: `U_$<chain>` for unit, class and
//!   thread variables (`U_$DELPHI_PROBE_$$_UNITCOUNTER` ->
//!   `U_$DELPHI_PROBE.UNITCOUNTER`), `VMT_$<chain>` for virtual method tables,
//!   `RTTI_$<chain>` for runtime type information and `INIT_$<chain>` for type
//!   initialization tables, each with the `$indirect` variant Free Pascal
//!   emits next to the plain table
//!   (`VMT_$DELPHI_PROBE_$$_TPROBECLASS$indirect` ->
//!   `VMT_$DELPHI_PROBE.TPROBECLASS$indirect`);
//! - the unit entries `INIT$_$<unit>` and `FINALIZE$_$<unit>`, which name the
//!   unit alone (`INIT$_$DELPHI_PROBE` -> `INIT$_$DELPHI_PROBE`), and the
//!   compiler-generated entry routines `<unit>_$$_init$`, `<unit>_$$_finalize$`
//!   and `<unit>_$$_finalize_implicit$`
//!   (`DELPHI_PROBE_$$_init$` -> `DELPHI_PROBE.init$()`);
//! - a trailing linker decoration (`.llvm.<hex>` or `@version`) is stripped
//!   before parsing, matching the other backends.
//!
//! A Mach-O platform underscore is not stripped: Free Pascal prepends `_` to
//! every symbol on Darwin, and the result (`_DELPHI_PROBE_$$_…`) is
//! indistinguishable from an identifier that starts with `_`, so the underscore
//! stays part of the printed unit name.
//!
//! `{$mode delphi}` does not change the linker-name scheme, so one grammar
//! covers both modes.
//!
//! # Canonical output
//!
//! Neither Free Pascal nor Delphi ships a demangler, so the printed text is
//! this crate's contract:
//!
//! | Input | Output |
//! |---|---|
//! | `DELPHI_PROBE_$$_NOARGS` | `DELPHI_PROBE.NOARGS()` |
//! | `DELPHI_PROBE_$$_UNITPROC$LONGINT` | `DELPHI_PROBE.UNITPROC(LONGINT)` |
//! | `DELPHI_PROBE_$$_UNITFUNC$ANSISTRING$$ANSISTRING` | `DELPHI_PROBE.UNITFUNC(ANSISTRING): ANSISTRING` |
//! | `DELPHI_PROBE$_$TPROBECLASS_$__$$_ADD$LONGINT$LONGINT$$LONGINT` | `DELPHI_PROBE.TPROBECLASS.ADD(LONGINT, LONGINT): LONGINT` |
//! | `DELPHI_PROBE$_$TPROBECLASS_$_TNESTED_$__$$_DEEP` | `DELPHI_PROBE.TPROBECLASS.TNESTED.DEEP()` |
//! | `DELPHI_PROBE_$$_UNITOPENARRAY$array_of_ANSISTRING` | `DELPHI_PROBE.UNITOPENARRAY(array of ANSISTRING)` |
//! | `U_$DELPHI_PROBE_$$_UNITCOUNTER` | `U_$DELPHI_PROBE.UNITCOUNTER` |
//! | `VMT_$DELPHI_PROBE_$$_TPROBECLASS$indirect` | `VMT_$DELPHI_PROBE.TPROBECLASS$indirect` |
//! | `RTTI_$DELPHI_PROBE_$$_TPROBEENUM_o2s` | `RTTI_$DELPHI_PROBE.TPROBEENUM_o2s` |
//! | `INIT$_$DELPHI_PROBE` | `INIT$_$DELPHI_PROBE` |
//! | `DELPHI_PROBE_$$_init$` | `DELPHI_PROBE.init$()` |
//!
//! The scope chain is joined with `.` and the signature is written as a
//! parameter list plus `: <result>`. A data symbol has no signature, and a unit
//! entry has no member at all, so neither is given a parameter list. Every name
//! and type token is printed as mangled, because Free Pascal uppercases
//! identifiers in linker names: the
//! declared spelling (`UnitProc`, `LongInt`, `TProbeClass`) is not recoverable,
//! and neither is a type alias (`Cardinal` mangles as `LONGWORD`). The unit and
//! routine separators (`_$$_`, `$_$`, `_$_`) are the compiler's, not the
//! source language's, so the text is a debugger name rather than re-parsable
//! Pascal.
//!
//! # Detection
//!
//! Free Pascal names have no unambiguous prefix, so [`crate::Scheme::detect`]
//! claims one only after [`parse`](crate::delphi::parse) accepts the whole
//! name. Ordinary C names never contain `$` and are never claimed, and a
//! malformed Free Pascal name is not claimed either.
//!
//! A namespaced unit puts a `.` in the linker name (`DELPHI_PROBE.NS_$$_…`).
//! Detection runs Delphi before Go, so these names are claimed end to end; a
//! Go linker name never contains `_$$_`.
//!
//! # Budgets
//!
//! Untrusted input stays bounded by [`MAX_INPUT_LEN`](crate::delphi::MAX_INPUT_LEN),
//! [`MAX_SCOPES`](crate::delphi::MAX_SCOPES),
//! [`MAX_PARAMS`](crate::delphi::MAX_PARAMS),
//! [`MAX_WORK`](crate::delphi::MAX_WORK) and
//! [`MAX_OUTPUT_LEN`](crate::delphi::MAX_OUTPUT_LEN). Exceeding a budget is
//! an error, never a partial result. The grammar is flat, so there is no
//! recursive depth to bound; the scope and parameter limits are the node
//! budget.
//!
//! # Gaps
//!
//! The first slice rejects, rather than guesses:
//!
//! - local routines nested in routines: Free Pascal joins the enclosing
//!   routines with `_`, which is also the identifier separator, so
//!   `outer_local_$$_inner` cannot be split without guessing
//!   (`DELPHI_PROBE$_$NESTEDOWNER_$$_LOCALPROC$LONGINT`);
//! - generic instantiations, whose scope or type token carries an undecodable
//!   `$<n>$CRC<hex>` marker (`DELPHI_PROBE_$$_USESGENERIC$TPROBEBOX$1$CRC9F312717`);
//! - the `$crc`/`$CRC` substitutions Free Pascal uses for over-long parameter
//!   lists and scope chains, which replace the tokens with a hash — including a
//!   `VMT_$`/`RTTI_$`/`INIT_$` table of a generic instantiation, whose type
//!   token carries the marker;
//! - the compiler data symbols that name a table of the whole unit rather than
//!   one of its types or variables (`DBG_$`, `DEBUGSTART_$`, `DEBUGEND_$`,
//!   `THREADVARLIST_$`) and the runtime tables (`FPC_*`, `INITFINAL`,
//!   `__fpc_*`);
//! - `file of X` parameters (`FILE$OF$X`), which cannot be told apart from a
//!   `FILE` parameter followed by an `OF` parameter;
//! - anonymous procedural-type tokens (`$_nestedprovar…_$`) and the `error`
//!   placeholder;
//! - the classic 32-bit Delphi form (`@Unit@Class@Method$QQRV…`). Its
//!   signature codes have no verifiable oracle here, and the shared detection
//!   path strips a leading `@`-suffix before this backend is asked, so the form
//!   is left to a later slice instead of being parsed without evidence.
//!
//! Same-named types from different units are indistinguishable in Free Pascal
//! linker names: `qc.usesboth(qa.tfoo, qb.tfoo)` mangles as
//! `QC_$$_USEBOTH$TFOO$TFOO`. The printed text repeats the token, which is all
//! the linker name carries.

mod node;
mod parser;
mod printer;

use core::fmt;

use crate::parse::LimitedWriter;
pub use node::{Ast, MAX_INPUT_LEN, MAX_OUTPUT_LEN, MAX_PARAMS, MAX_SCOPES, MAX_WORK};
pub use parser::is_delphi;

/// Why a Delphi/Object Pascal symbol could not be parsed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ParseError {
    /// The symbol does not match a supported Free Pascal linker name.
    Invalid,
    /// The input, work, scope or parameter budget was exceeded.
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
    /// [`MAX_OUTPUT_LEN`](crate::delphi::MAX_OUTPUT_LEN).
    pub(crate) fn print<W: fmt::Write + ?Sized>(&self, out: &mut W) -> fmt::Result {
        let mut limited = LimitedWriter::new(out, MAX_OUTPUT_LEN);
        printer::print(&self.name, &mut limited)
    }
}

/// Parse a Free Pascal linker name into a reusable [`Ast`].
///
/// # Errors
///
/// Returns [`ParseError::Invalid`] when the symbol is not a supported Free
/// Pascal linker name (including the nested-routine, generic and
/// hash-substituted forms listed in the module docs), and
/// [`ParseError::BudgetExceeded`] when the input, work, scope or parameter
/// budget is exceeded.
pub fn parse(symbol: &str) -> Result<Ast<'_>, ParseError> {
    parse_bytes(symbol.as_bytes())
}

/// Parse a Free Pascal linker name from raw bytes.
///
/// This is the entry point the demangler dispatch uses once symbol bytes, not
/// text, are the input. See [`parse`](crate::delphi::parse) for the accepted forms; the only
/// difference is that the name does not have to be UTF-8.
pub(crate) fn parse_bytes(symbol: &[u8]) -> Result<Ast<'_>, ParseError> {
    parser::parse_bytes(symbol)
}

#[cfg(test)]
mod tests {
    use alloc::string::{String, ToString};

    use super::*;

    #[test]
    fn parses_unit_scope_routines() {
        for (symbol, expected) in [
            ("DELPHI_PROBE_$$_NOARGS", "DELPHI_PROBE.NOARGS()"),
            (
                "DELPHI_PROBE_$$_UNITPROC$LONGINT",
                "DELPHI_PROBE.UNITPROC(LONGINT)",
            ),
            (
                "DELPHI_PROBE_$$_UNITFUNC$ANSISTRING$$ANSISTRING",
                "DELPHI_PROBE.UNITFUNC(ANSISTRING): ANSISTRING",
            ),
            (
                "DELPHI_PROBE_$$_MANYPARAMS$LONGINT$INT64$ANSISTRING$POINTER$DOUBLE$BOOLEAN$CHAR",
                "DELPHI_PROBE.MANYPARAMS(LONGINT, INT64, ANSISTRING, POINTER, DOUBLE, BOOLEAN, CHAR)",
            ),
            (
                "DELPHI_PROBE_$$_$plus$TPROBEREC$TPROBEREC$$TPROBEREC",
                "DELPHI_PROBE.$plus(TPROBEREC, TPROBEREC): TPROBEREC",
            ),
        ] {
            assert_eq!(
                parse(symbol)
                    .unwrap_or_else(|error| panic!("{symbol}: {error:?}"))
                    .to_string(),
                expected,
                "{symbol}"
            );
        }
    }

    #[test]
    fn parses_methods_and_nested_types() {
        for (symbol, expected) in [
            (
                "DELPHI_PROBE$_$TPROBECLASS_$__$$_PLAIN",
                "DELPHI_PROBE.TPROBECLASS.PLAIN()",
            ),
            (
                "DELPHI_PROBE$_$TPROBECLASS_$__$$_ADD$LONGINT$LONGINT$$LONGINT",
                "DELPHI_PROBE.TPROBECLASS.ADD(LONGINT, LONGINT): LONGINT",
            ),
            (
                "DELPHI_PROBE$_$TPROBECLASS_$__$$_CREATE$$TPROBECLASS",
                "DELPHI_PROBE.TPROBECLASS.CREATE(): TPROBECLASS",
            ),
            (
                "DELPHI_PROBE$_$TPROBECLASS_$_TNESTED_$__$$_DEEP",
                "DELPHI_PROBE.TPROBECLASS.TNESTED.DEEP()",
            ),
            (
                "DELPHI_PROBE$_$TPROBEREC_$__$$_MOVEBY$LONGINT$LONGINT",
                "DELPHI_PROBE.TPROBEREC.MOVEBY(LONGINT, LONGINT)",
            ),
        ] {
            assert_eq!(
                parse(symbol)
                    .unwrap_or_else(|error| panic!("{symbol}: {error:?}"))
                    .to_string(),
                expected,
                "{symbol}"
            );
        }
    }

    #[test]
    fn prints_open_arrays_and_namespaced_units() {
        for (symbol, expected) in [
            (
                "DELPHI_PROBE_$$_UNITOPENARRAY$array_of_ANSISTRING",
                "DELPHI_PROBE.UNITOPENARRAY(array of ANSISTRING)",
            ),
            (
                "DELPHI_PROBE$_$TPROBECLASS_$__$$_OPENARRAYCONST$array_of_const",
                "DELPHI_PROBE.TPROBECLASS.OPENARRAYCONST(array of const)",
            ),
            (
                "NS.ESC$_$TFOO_BAR_$__$$_METHODTWO$LONGINT",
                "NS.ESC.TFOO_BAR.METHODTWO(LONGINT)",
            ),
            (
                "P$DELPHI_PROBE_PROG_$$_PROGPROC$LONGINT",
                "P$DELPHI_PROBE_PROG.PROGPROC(LONGINT)",
            ),
        ] {
            assert_eq!(
                parse(symbol)
                    .unwrap_or_else(|error| panic!("{symbol}: {error:?}"))
                    .to_string(),
                expected,
                "{symbol}"
            );
        }
    }

    #[test]
    fn strips_linker_decorations() {
        assert_eq!(
            parse("DELPHI_PROBE_$$_NOARGS.llvm.1234")
                .unwrap()
                .to_string(),
            "DELPHI_PROBE.NOARGS()"
        );
        assert_eq!(
            parse("DELPHI_PROBE_$$_NOARGS@@FPC_3.2")
                .unwrap()
                .to_string(),
            "DELPHI_PROBE.NOARGS()"
        );
    }

    #[test]
    fn parses_compiler_data_symbols() {
        for (symbol, expected) in [
            (
                "U_$DELPHI_PROBE_$$_UNITCOUNTER",
                "U_$DELPHI_PROBE.UNITCOUNTER",
            ),
            ("U_$P$DELPHI_PROBE_PROG_$$_G", "U_$P$DELPHI_PROBE_PROG.G"),
            (
                "VMT_$DELPHI_PROBE_$$_TPROBECLASS",
                "VMT_$DELPHI_PROBE.TPROBECLASS",
            ),
            (
                "VMT_$DELPHI_PROBE_$$_TPROBECLASS$indirect",
                "VMT_$DELPHI_PROBE.TPROBECLASS$indirect",
            ),
            (
                "VMT_$DELPHI_PROBE$_$TPROBECLASS_$__$$_TNESTED",
                "VMT_$DELPHI_PROBE.TPROBECLASS.TNESTED",
            ),
            (
                "RTTI_$DELPHI_PROBE_$$_TPROBEENUM_o2s",
                "RTTI_$DELPHI_PROBE.TPROBEENUM_o2s",
            ),
            (
                "RTTI_$DELPHI_PROBE.NS_$$_TNAMESPACED$indirect",
                "RTTI_$DELPHI_PROBE.NS.TNAMESPACED$indirect",
            ),
            (
                "INIT_$DELPHI_PROBE_$$_TPROBECLASS",
                "INIT_$DELPHI_PROBE.TPROBECLASS",
            ),
            ("INIT$_$DELPHI_PROBE", "INIT$_$DELPHI_PROBE"),
            ("FINALIZE$_$DELPHI_PROBE.NS", "FINALIZE$_$DELPHI_PROBE.NS"),
            ("DELPHI_PROBE_$$_init$", "DELPHI_PROBE.init$()"),
            (
                "DELPHI_PROBE_$$_finalize_implicit$",
                "DELPHI_PROBE.finalize_implicit$()",
            ),
            // A unit whose name matches a data prefix keeps its routines: no
            // unit name can contain `$`, so the routine grammar wins when the
            // data grammar does not match.
            ("U_$$_PROC$LONGINT", "U.PROC(LONGINT)"),
            ("INIT_$$_PROC", "INIT.PROC()"),
            ("RTTI_$$_FOO$LONGINT", "RTTI.FOO(LONGINT)"),
        ] {
            assert_eq!(
                parse(symbol)
                    .unwrap_or_else(|error| panic!("{symbol}: {error:?}"))
                    .to_string(),
                expected,
                "{symbol}"
            );
        }
    }

    #[test]
    fn rejects_unsupported_forms() {
        for symbol in [
            // Nested routines: the enclosing chain is `_`-joined and ambiguous.
            "DELPHI_PROBE$_$NESTEDOWNER_$$_LOCALPROC$LONGINT",
            "DELPHI_PROBE$_$OVERLOADED$LONGINT_$$_LOCALINOVERLOAD",
            "DELPHI_PROBE$_$TPROBECLASS_$_WITHLOCAL_$$_LOCALOFMETHOD$LONGINT",
            // Generic instantiations carry an undecodable `$<n>$CRC<hex>`.
            "DELPHI_PROBE$_$TPROBEBOX$1$CRC9F312717_$__$$_PUT$LONGINT",
            "DELPHI_PROBE_$$_USESGENERIC$TPROBEBOX$1$CRC9F312717",
            "VMT_$DELPHI_PROBE_$$_TPROBEBOX$1$CRC9F312717",
            "RTTI_$DELPHI_PROBE_$$_TPROBEBOX$1$CRC9F312717$indirect",
            "INIT_$DELPHI_PROBE_$$_TPROBEBOX$1$CRC9F312717",
            // Hash substitutions.
            "DELPHI_PROBE_$$_MANYPARAMS$crc8415F53F",
            "DELPHI_PROBE$_$$CRCF62B7D7C_$$_INNEROFLONGOUTER",
            // Unit-wide tables and runtime data, which name no type or variable.
            "DBG_$DELPHI_PROBE_$$_TPROBECLASS",
            "DEBUGSTART_$DELPHI_PROBE",
            "DEBUGEND_$DELPHI_PROBE.NS",
            "THREADVARLIST_$DELPHI_PROBE",
            "FPC_THREADVARTABLES",
            "INITFINAL",
            "__fpc_ident",
            // File types and the error placeholder.
            "DELPHI_PROBE_$$_P$FILE$OF$BYTE",
            "DELPHI_PROBE_$$_P$FILE",
            "DELPHI_PROBE_$$_P$error",
            // The classic Delphi form has no supported signature decoding.
            "@System@@LStrClr$QQRPV",
            "@System@TObject@Create",
        ] {
            assert!(parse(symbol).is_err(), "{symbol} should be rejected");
        }
    }

    #[test]
    fn rejects_malformed_names() {
        for symbol in [
            "",
            "DELPHI_PROBE",
            "DELPHI_PROBE_$$_",
            "DELPHI_PROBE_$$_$",
            "DELPHI_PROBE_$$_NOARGS$",
            "DELPHI_PROBE_$$_NOARGS$LONGINT$",
            "DELPHI_PROBE_$$_NOARGS$$",
            "DELPHI_PROBE_$$_NOARGS$$$$LONGINT",
            "DELPHI_PROBE_$$_NOARGS$array_of_",
            "DELPHI_PROBE_$$_NOARGS$array_of_constX",
            "DELPHI_PROBE_$$_NOARGS$lowercase",
            "DELPHI_PROBE$_$TPROBECLASS_$$_PLAIN",
            "DELPHI_PROBE$_$_$$_PLAIN",
            "DELPHI_PROBE$_$TPROBECLASS_$__$$_",
            "P$_$$_PLAIN",
            "DELPHI_PROBE..X_$$_PLAIN",
            "DELPHI_PROBE_$$_NO-ARGS",
            "DELPHI_PROBE_$$_NO ARGS",
            // Compiler data symbols must carry the full chain.
            "U_$",
            "U_$DELPHI_PROBE",
            "U_$DELPHI_PROBE_$$_",
            "U_$DELPHI_PROBE_$$_X$indirect",
            "VMT_$DELPHI_PROBE_$$_TPROBECLASS$indirectX",
            "RTTI_$",
            "INIT_$DELPHI_PROBE_$$_TPROBECLASS$",
            // Unit entries name the unit alone.
            "INIT$_$",
            "INIT$_$DELPHI_PROBE_$$_X",
            "FINALIZE$_$DELPHI_PROBE$",
            // The entry routine names are matched exactly.
            "DELPHI_PROBE_$$_init$x",
        ] {
            assert!(parse(symbol).is_err(), "{symbol:?} should be rejected");
        }
    }

    #[test]
    fn budgets_are_enforced() {
        let long = "A".repeat(MAX_INPUT_LEN + 1);
        assert!(matches!(parse(&long), Err(ParseError::BudgetExceeded)));

        let mut many = String::from("UNIT_$$_PROC");
        for _ in 0..=MAX_PARAMS {
            many.push_str("$LONGINT");
        }
        assert!(matches!(parse(&many), Err(ParseError::BudgetExceeded)));

        let mut scopes = String::from("UNIT$_$");
        for _ in 0..=MAX_SCOPES {
            scopes.push_str("TCLASS_$_");
        }
        scopes.push_str("_$$_PROC");
        assert!(matches!(parse(&scopes), Err(ParseError::BudgetExceeded)));
    }

    #[test]
    fn rejects_non_identifier_bytes() {
        // Free Pascal identifiers are ASCII, so a non-ASCII byte is not a name.
        for symbol in [
            b"DELPHI_PROBE_$$_NO\xffARGS".as_slice(),
            b"DELPHI_PROBE_$$_\xc3\xa9".as_slice(),
            b"DELPHI_PROBE$_$TCLASS_$__$$_M\x80".as_slice(),
        ] {
            assert!(parse_bytes(symbol).is_err(), "{symbol:?}");
        }
        assert_eq!(
            parse_bytes(b"DELPHI_PROBE_$$_NOARGS").unwrap().to_string(),
            "DELPHI_PROBE.NOARGS()"
        );
    }

    #[test]
    fn detection_requires_a_full_parse() {
        for symbol in [
            "DELPHI_PROBE_$$_NOARGS",
            "DELPHI_PROBE$_$TPROBECLASS_$__$$_PLAIN",
            "NS.ESC_$$_FOO_BAR$LONGINT",
            "P$DELPHI_PROBE_PROG_$$_PROGPROC$LONGINT",
        ] {
            assert!(is_delphi(symbol.as_bytes()), "{symbol} should be Delphi");
        }
        for symbol in [
            "main",
            "foo_bar",
            "foo__bar",
            "_Z3foov",
            "_RNvNtCs1234_7mycrate3foo",
            "DELPHI_PROBE_$$_NOARGS$",
            "DELPHI_PROBE_$$_USESGENERIC$TPROBEBOX$1$CRC9F312717",
            "@System@@LStrClr$QQRPV",
        ] {
            assert!(
                !is_delphi(symbol.as_bytes()),
                "{symbol} should not be Delphi"
            );
        }
    }
}
