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
//! Untrusted input stays bounded by the input and work budgets
//! (`MAX_INPUT_LEN` and
//! `MAX_WORK` by default). The grammar is flat
//! and the AST borrows at most two slices of the input, so there is no node
//! budget and no recursion. Exceeding a budget is an error, never a partial
//! result.
//!
//! Both limits are runtime-configurable. [`parse`](crate::fortran::parse)
//! keeps the compiled-in defaults, and a [`crate::Demangler`] passes the
//! configured budget, clamped to the hard ceilings of 16 MiB input and 64M
//! work units. The two dimensions are independent: raising the input limit
//! alone keeps the default work allowance, so a caller that raises the input
//! for a long symbol must raise [`crate::Budget::work`] as well when the
//! one-pass scan exceeds it. The grammar has no depth or node dimension to
//! configure.
//!
//! Printing is not a budget: the printer walks each form once and the output
//! is shorter than the input, so `Display` is total except for a failing sink.
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
pub use node::Ast;
use node::ParseLimits;
pub(crate) use node::{MAX_INPUT_LEN, MAX_WORK};
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
    /// Render the parsed name into `out`, without a print cap.
    pub(crate) fn print<W: fmt::Write + ?Sized>(&self, out: &mut W) -> fmt::Result {
        printer::print(&self.name, out)
    }
}

impl Ast<'_> {
    /// The scheme this tree was parsed from: always [`crate::Scheme::Fortran`].
    #[must_use]
    pub const fn scheme(&self) -> crate::Scheme {
        crate::Scheme::Fortran
    }
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
pub fn parse(symbol: &[u8]) -> Result<Ast<'_>, ParseError> {
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
mod tests;

#[cfg(test)]
mod budget_tests;
