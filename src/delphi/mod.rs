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
//! Untrusted input stays bounded by the input and work budgets
//! (`MAX_INPUT_LEN` and
//! `MAX_WORK` by default). Exceeding a budget is
//! an error, never a partial result. The grammar is flat, so there is no
//! recursive depth to bound, and every scope and parameter consumes input, so
//! the input budget bounds both lists; no separate scope or parameter cap
//! exists.
//!
//! Both limits are runtime-configurable. [`parse`](crate::delphi::parse)
//! keeps the compiled-in defaults, and a [`crate::Demangler`] passes the
//! configured budget, clamped to the hard ceilings of 16 MiB input and 64M
//! work units. The two dimensions are independent: raising the input limit
//! alone keeps the default work allowance, so a caller that raises the input
//! for a long symbol must raise [`crate::Budget::work`] as well when the scan
//! exceeds it. The grammar has no depth or node dimension to configure.
//!
//! Printing is not a budget: the printer walks each name once and the output
//! is linear in the linker name, so `Display` is total except for a failing
//! sink.
//!
//! # Gaps
//!
//! Unsupported forms are rejected, not guessed:
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

use crate::budget::{EffectiveBudget, Limit};
pub use node::Ast;
use node::ParseLimits;
pub(crate) use node::{MAX_INPUT_LEN, MAX_WORK};
pub use parser::is_delphi;

impl fmt::Display for Ast<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.print(f)
    }
}

impl Ast<'_> {
    /// Render the parsed name into `out`, without a print cap
    pub(crate) fn print<W: fmt::Write + ?Sized>(&self, out: &mut W) -> fmt::Result {
        printer::print(&self.name, out)
    }
}

impl Ast<'_> {
    /// The scheme this tree was parsed from: always [`crate::Scheme::Delphi`].
    #[must_use]
    pub const fn scheme(&self) -> crate::Scheme {
        crate::Scheme::Delphi
    }
}

/// The budget the public entry point starts from.
///
/// [`parse`] uses it, so it keeps exactly the compiled-in limits; a
/// [`crate::Demangler`] resolves its configured [`crate::Budget`] for
/// [`Scheme::Delphi`](crate::Scheme::Delphi) instead.
pub(crate) const DEFAULT_BUDGET: EffectiveBudget = EffectiveBudget {
    input_bytes: Some(Limit::Bounded(MAX_INPUT_LEN)),
    parse_depth: None,
    work: Some(Limit::Bounded(MAX_WORK)),
    nodes: None,
    expansion_visits: None,
    expansion_bytes: None,
};

/// Parse a Free Pascal linker name into a reusable [`Ast`].
///
/// `budget` overrides the compiled-in limits per dimension, exactly as on a
/// [`Demangler`](crate::Demangler): [`Budget::default`](crate::Budget::default)
/// keeps them, and every value is clamped to
/// [`Scheme::hard_budget`](crate::Scheme::hard_budget).
///
/// # Errors
///
/// Returns [`ParseErrorKind::Invalid`](crate::ParseErrorKind::Invalid) when the symbol is not a supported Free
/// Pascal linker name (including the nested-routine, generic and
/// hash-substituted forms listed in the module docs), and
/// [`ParseErrorKind::Budget`](crate::ParseErrorKind::Budget) when the input, work, scope or parameter
/// budget is exceeded.
pub fn parse(symbol: &[u8], budget: crate::Budget) -> Result<Ast<'_>, crate::ParseError> {
    parse_bytes_with_budget(symbol, budget.effective(crate::Scheme::Delphi))
}

/// Parse a raw symbol byte string with one scheme's resolved budget.
///
/// The public [`parse`] wraps this with [`DEFAULT_BUDGET`]; a
/// [`crate::Demangler`] passes [`Demangler::effective_budget`] for
/// [`Scheme::Delphi`](crate::Scheme::Delphi), so a caller's overrides reach the
/// parser. The input and work values are clamped to this module's hard
/// ceilings, and the grammar has no depth or node dimension to resolve.
///
/// [`Demangler::effective_budget`]: crate::Demangler::effective_budget
pub(crate) fn parse_bytes_with_budget(
    symbol: &[u8],
    budget: EffectiveBudget,
) -> Result<Ast<'_>, crate::ParseError> {
    parser::parse_bytes(symbol, ParseLimits::from_effective(budget))
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod budget_tests;
