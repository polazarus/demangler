//! OCaml symbol model and budgets.
//!
//! An OCaml native linker name carries no type information: it is the
//! compilation-unit path, an optional member name and a numeric stamp that
//! makes the name unique. This module mirrors exactly that structure, plus the
//! compiler's static-data labels and the runtime-helper forms the compiler and
//! the runtime share the `caml` prefix with.

use crate::budget::{EffectiveBudget, Limit};
use crate::parse::Limits;

/// Maximum accepted input length.
///
/// This is the default a [`crate::Budget`] starts from; a [`crate::Demangler`]
/// may raise it up to a fixed 16 MiB hard ceiling.
pub const MAX_INPUT_LEN: usize = 64 * 1024;

/// Maximum number of parser work units (bytes scanned).
///
/// This is the default a [`crate::Budget`] starts from; a [`crate::Demangler`]
/// may raise it up to a fixed 64M-unit hard ceiling.
pub const MAX_WORK: usize = 8 * MAX_INPUT_LEN;

/// Hard input ceiling.
///
/// The parser only borrows the input, but the one-pass scan and the printed
/// text grow with it, so a raise stays bounded whatever the caller asks for.
pub(crate) const HARD_INPUT_LEN: usize = 16 * 1024 * 1024;

/// Hard work ceiling.
pub(crate) const HARD_WORK: usize = 64 * 1024 * 1024;

/// The input and work limits one parse enforces.
///
/// Resolved from a [`crate::Budget`]: an absent dimension keeps its compiled-in
/// default and every value is clamped to this module's hard ceiling. The
/// grammar is flat, so there is no depth or node dimension to resolve.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ParseLimits {
    /// Maximum accepted input length in bytes.
    pub(crate) input: usize,
    /// Maximum parser work units.
    pub(crate) work: usize,
}

impl ParseLimits {
    /// Resolve one scheme's budget for a parse of an OCaml linker name.
    pub(crate) fn from_effective(budget: EffectiveBudget) -> Self {
        Self {
            input: clamp(budget.input_bytes, MAX_INPUT_LEN, HARD_INPUT_LEN),
            work: clamp(budget.work, MAX_WORK, HARD_WORK),
        }
    }

    /// Limits handed to the shared byte cursor.
    ///
    /// The input and work values are clamped to the hard ceilings, so both fit
    /// the cursor's `u32` fields.
    pub(crate) fn cursor(&self) -> Limits {
        // The grammar is flat and the parser never enters a level, so the
        // cursor gets no depth allowance at all.
        Limits::saturating(self.input, 0, self.work)
    }
}

/// One budget dimension after clamping to the compiled-in default and ceiling.
fn clamp(dimension: Option<Limit>, default: usize, ceiling: usize) -> usize {
    match dimension {
        Some(Limit::Bounded(value)) => value.min(ceiling),
        Some(Limit::Unbounded) | None => default.min(ceiling),
    }
}

/// A parsed OCaml linker name.
#[derive(Debug)]
pub struct Ast<'a> {
    pub(crate) form: Form<'a>,
}

/// Which `caml` form a symbol uses.
#[derive(Debug)]
pub(crate) enum Form<'a> {
    /// `caml<path>`: the module record of a compilation unit or pack.
    Module {
        /// Compilation-unit path as mangled, `.`-separated.
        path: &'a [u8],
    },
    /// `caml<path>.<member>`: a value, an operator or a compiler marker.
    Member {
        /// Compilation-unit path as mangled, `.`-separated.
        path: &'a [u8],
        /// Member as mangled, stamp removed and `$XX` escapes kept.
        member: &'a [u8],
    },
    /// `caml<path>.<label>`: a compiler-emitted static-data label.
    Label {
        /// Compilation-unit path as mangled, `.`-separated.
        path: &'a [u8],
        /// Decimal label as emitted, without a name of its own.
        label: &'a [u8],
    },
    /// `caml_<name>`: a runtime helper, printed as it stands. A host-compiler
    /// fragment suffix (`.cold`, `.part.<N>`) is not part of the name and is
    /// dropped while parsing.
    Helper {
        /// The whole helper name, `caml_` included.
        symbol: &'a [u8],
    },
    /// A section marker of a runtime pseudo-unit: `caml_<unit>.<marker>` or the
    /// assembler's `caml_<unit>__<marker>` spelling.
    Marker {
        /// Pseudo-unit name without the `caml_` prefix (`startup`, `hot`, …).
        unit: &'a [u8],
        /// Section marker (`code_begin`, `frametable`, …).
        marker: &'a [u8],
        /// How the linker name joins the two components.
        join: MarkerJoin,
    },
}

/// How a runtime pseudo-unit's name and its section marker are joined.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MarkerJoin {
    /// `caml_<unit>.<marker>`: the compiler's spelling.
    Dot,
    /// `caml_<unit>__<marker>`: the assembler's doubled-underscore spelling.
    DoubleUnderscore,
}

impl MarkerJoin {
    /// The separator the linker name uses.
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Dot => ".",
            Self::DoubleUnderscore => "__",
        }
    }
}
