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
//! Untrusted input stays bounded by the input and work budgets
//! (`MAX_INPUT_LEN` and
//! `MAX_WORK` by default). Exceeding a budget is an
//! error, never a partial result. The grammar is flat, so there is no
//! recursive depth to bound, and every segment consumes at least one input
//! byte and a `__` separator, so the input budget bounds the segment list; no
//! separate segment cap exists.
//!
//! Both limits are runtime-configurable. [`parse`](crate::ada::parse) keeps
//! the compiled-in defaults, and a [`crate::Demangler`] passes the configured
//! budget, clamped to the hard ceilings of 16 MiB input and 64M work units.
//! The two dimensions are independent: raising the input limit alone keeps the
//! default work allowance, so a caller that raises the input for a long symbol
//! must raise [`crate::Budget::work`] as well when the scan exceeds it. The
//! grammar has no depth or node dimension to configure.
//!
//! Printing is not a budget: the printer walks each segment once and the
//! output is never longer than the input plus eight bytes, so `Display` is
//! total except for a failing sink.
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
pub use node::Ast;
use node::ParseLimits;
pub(crate) use node::{MAX_INPUT_LEN, MAX_WORK};
pub use parser::is_ada;

/// Why an Ada/GNAT symbol could not be parsed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub(crate) enum Failure {
    /// The symbol does not match a supported GNAT external-name form.
    Invalid,
    /// The input, work or segment budget was exceeded.
    BudgetExceeded,
}

impl Failure {
    /// The parser budget dimension this failure belongs to.
    #[must_use]
    pub(crate) const fn budget_kind(self) -> Option<BudgetKind> {
        match self {
            Self::BudgetExceeded => Some(BudgetKind::Other),
            _ => None,
        }
    }
}

impl fmt::Display for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Invalid => "invalid syntax",
            Self::BudgetExceeded => "size limit reached",
        })
    }
}

impl core::error::Error for Failure {}

impl From<Failure> for crate::ParseError {
    fn from(failure: Failure) -> Self {
        Self::new(failure.budget_kind().map_or(
            crate::ParseErrorKind::Invalid,
            crate::ParseErrorKind::Budget,
        ))
    }
}

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
    /// The scheme this tree was parsed from: always [`crate::Scheme::Ada`].
    #[must_use]
    pub const fn scheme(&self) -> crate::Scheme {
        crate::Scheme::Ada
    }
}

/// The budget the public entry point starts from.
///
/// [`parse`] uses it, so it keeps exactly the compiled-in limits; a
/// [`crate::Demangler`] resolves its configured [`crate::Budget`] for
/// [`Scheme::Ada`](crate::Scheme::Ada) instead.
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "the unit tests parse with the default budget")
)]
pub(crate) const DEFAULT_BUDGET: EffectiveBudget = EffectiveBudget {
    input_bytes: Some(Limit::Bounded(MAX_INPUT_LEN)),
    parse_depth: None,
    work: Some(Limit::Bounded(MAX_WORK)),
    nodes: None,
    expansion_visits: None,
    expansion_bytes: None,
};

/// Parse an Ada/GNAT external name into a reusable [`Ast`].
///
/// General GNAT user names are accepted, including names that are not
/// auto-detected by [`crate::Scheme::detect`]. A Mach-O leading underscore on
/// the `ada__` family and a trailing linker decoration are stripped first.
///
/// `budget` overrides the compiled-in limits per dimension, exactly as on a
/// [`Demangler`](crate::Demangler): [`Budget::default`](crate::Budget::default)
/// keeps them, and every value is clamped to
/// [`Scheme::hard_budget`](crate::Scheme::hard_budget).
///
/// # Errors
///
/// Returns [`ParseErrorKind::Invalid`](crate::ParseErrorKind::Invalid) when the symbol is not a supported GNAT
/// external name (including task, protected-type, runtime and other
/// compiler-generated suffixes), and [`ParseErrorKind::Budget`](crate::ParseErrorKind::Budget) when the
/// input, work or segment budget is exceeded.
pub fn parse(symbol: &[u8], budget: crate::Budget) -> Result<Ast<'_>, crate::ParseError> {
    parse_bytes_with_budget(symbol, budget.effective(crate::Scheme::Ada))
        .map_err(crate::ParseError::from)
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
) -> Result<Ast<'_>, Failure> {
    parser::parse_bytes(symbol, ParseLimits::from_effective(budget))
}

/// Budget-aware Ada detection prefilter.
pub(crate) fn is_ada_with_budget(symbol: &[u8], budget: EffectiveBudget) -> bool {
    parser::is_ada_with(symbol, ParseLimits::from_effective(budget).input)
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod budget_tests;
