//! Ada/GNAT external-name model and budgets.
//!
//! GNAT does not encode a type grammar into a symbol. A symbol is the
//! fully-qualified Ada name with `.` replaced by `__`, optionally followed by a
//! compiler suffix: `N`/`P` on a protected operation, `_B<digits>s`/`_E<digits>s`
//! on an entry barrier or entry body, `TKB` on the body of a task object, or
//! `___elabb`/`___elabs` on a package elaboration procedure. The model mirrors
//! that structure: a list of identifier segments and one optional suffix.

use alloc::vec::Vec;

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
/// The parser only borrows the input, but the segment list and the printed text
/// both grow with it, so a raise stays bounded whatever the caller asks for.
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
    /// Resolve one scheme's budget for a parse of an Ada external name.
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
    pub(crate) const fn cursor(&self) -> Limits {
        Limits {
            input: self.input as u32,
            // The grammar is flat and the parser never enters a level, so the
            // cursor gets no depth allowance at all.
            depth: 0,
            work: self.work as u32,
        }
    }
}

/// One budget dimension after clamping to the compiled-in default and ceiling.
fn clamp(dimension: Option<Limit>, default: usize, ceiling: usize) -> usize {
    match dimension {
        Some(Limit::Bounded(value)) => value.min(ceiling),
        Some(Limit::Unbounded) | None => default.min(ceiling),
    }
}

/// A parsed Ada/GNAT external name.
#[derive(Debug)]
pub struct Ast<'a> {
    pub(crate) name: Name<'a>,
}

/// A qualified Ada name with an optional compiler suffix.
#[derive(Debug)]
pub(crate) struct Name<'a> {
    pub(crate) segments: Vec<&'a [u8]>,
    pub(crate) suffix: Option<Suffix>,
}

/// The compiler suffix appended to a GNAT external name.
///
/// At most one suffix applies to a name: the elaboration suffixes attach to a
/// package or subprogram, and the operation, entry and task suffixes attach to
/// the last identifier of a generated subprogram.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Suffix {
    /// `N` or `P`: a protected operation (or an enum initialization helper,
    /// which uses the same trailing `N`).
    ProtectedOp,
    /// `_B<digits>s`: the barrier procedure of an entry or entry family.
    EntryBarrier,
    /// `_E<digits>s`: the body procedure of an entry or entry family.
    EntryBody,
    /// `TKB`: the body procedure of a task object.
    TaskBody,
    /// `___elabb`: the package body or subprogram elaboration procedure.
    ElabBody,
    /// `___elabs`: the package spec elaboration procedure.
    ElabSpec,
}
