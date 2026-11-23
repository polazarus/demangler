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
//! symbol   ::= module | external | "MAIN__"
//! module   ::= "__" ident "_MOD_" ident          (plus one Mach-O underscore)
//! external ::= ident "_"
//! ident    ::= [a-z] ( [a-z0-9_] )*
//! ```
//!
//! - a module procedure, module variable or module-level `entry`
//!   (`__fprobe_MOD_add`);
//! - a module name that itself ends with `_` (`__my_mod__MOD_var`);
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
//! | `ext_sub_` | `ext_sub` |
//! | `extd__` | `extd_` |
//! | `MAIN__` | `main` |
//!
//! The spelling is a debugger name, not re-parsable Fortran source: `_MOD_`
//! becomes `.`, and the appended underscore of an external name is dropped.
//! Module and member names are printed verbatim, because the compiler does not
//! underscore them.
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
//! # Gaps
//!
//! The first slice rejects, rather than guesses:
//!
//! - generated type helpers (`__vtab_`, `__def_init_`, `__copy_`, `__final_`),
//!   whose member starts with an underscore;
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

use core::fmt;

use crate::parse::LimitedWriter;
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

/// Parse a gfortran external name from raw bytes.
///
/// This is the entry point the demangler dispatch uses once symbol bytes, not
/// text, are the input. See [`parse`] for the accepted forms; the only
/// difference is that the name does not have to be UTF-8.
pub(crate) fn parse_bytes(symbol: &[u8]) -> Result<Ast<'_>, ParseError> {
    parser::parse_bytes(symbol)
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
    fn rejects_generated_helpers() {
        for symbol in [
            "__fprobe_MOD___vtab_fprobe_Point",
            "__fprobe_MOD___def_init_fprobe_Point",
            "__fprobe_MOD___copy_fprobe_Point",
            "__fprobe_MOD___final_fprobe_Point",
        ] {
            assert!(parse(symbol).is_err(), "{symbol} should be rejected");
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
