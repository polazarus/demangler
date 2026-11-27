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
//! Untrusted input stays bounded by the input and work budgets
//! (`MAX_INPUT_LEN` and
//! `MAX_WORK` by default). Exceeding a budget is an
//! error, never a partial result. The grammar is flat, so there is no
//! recursive depth to bound, and every module path segment consumes input, so
//! the input budget bounds the path; no separate segment cap exists.
//!
//! Both limits are runtime-configurable. [`parse`](crate::ocaml::parse) keeps
//! the compiled-in defaults, and a [`crate::Demangler`] passes the configured
//! budget, clamped to the hard ceilings of 16 MiB input and 64M work units.
//! The two dimensions are independent: raising the input limit alone keeps the
//! default work allowance, so a caller that raises the input for a long symbol
//! must raise [`crate::Budget::work`] as well when the scan exceeds it. The
//! grammar has no depth or node dimension to configure.
//!
//! Printing is not a budget: the printer walks each form once and the output
//! is linear in the linker name, so `Display` is total except for a failing
//! sink.
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

use crate::error::BudgetKind;
use core::fmt;

use crate::budget::{EffectiveBudget, Limit};
pub use node::Ast;
use node::ParseLimits;
pub(crate) use node::{MAX_INPUT_LEN, MAX_WORK};
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
    /// Render the parsed name into `out`, without a print cap
    pub(crate) fn print<W: fmt::Write + ?Sized>(&self, out: &mut W) -> fmt::Result {
        printer::print(&self.form, out)
    }
}

impl Ast<'_> {
    /// The scheme this tree was parsed from: always [`crate::Scheme::OCaml`].
    #[must_use]
    pub const fn scheme(&self) -> crate::Scheme {
        crate::Scheme::OCaml
    }
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
    expansion_visits: None,
    expansion_bytes: None,
};

/// Parse an OCaml linker name into a reusable [`Ast`].
///
/// # Errors
///
/// Returns [`ParseError::Invalid`] when the symbol is not a supported `caml`
/// form (including the C-section, bytecode and Mach-O-separator forms listed in
/// the module docs), and [`ParseError::BudgetExceeded`] when the input, work or
/// path-segment budget is exceeded.
pub fn parse(symbol: &[u8]) -> Result<Ast<'_>, ParseError> {
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
mod tests;

#[cfg(test)]
mod budget_tests;
