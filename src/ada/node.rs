//! Ada/GNAT external-name model and budgets.
//!
//! GNAT does not encode a type grammar into a symbol. A symbol is the
//! fully-qualified Ada name with `.` replaced by `__`, optionally followed by a
//! compiler suffix such as `N`/`P` (protected operations) or
//! `___elabb`/`___elabs` (package elaboration). The model mirrors that
//! structure: a list of identifier segments and one optional elaboration
//! suffix.

use alloc::vec::Vec;

/// Maximum accepted input length.
pub const MAX_INPUT_LEN: usize = 64 * 1024;

/// Maximum number of `__`-separated name segments.
///
/// This is also the parser's node budget: every segment is one allocation in
/// the owned segment list.
pub const MAX_SEGMENTS: usize = 256;

/// Maximum number of parser work units (bytes scanned).
pub const MAX_WORK: usize = 8 * MAX_INPUT_LEN;

/// Maximum printed output length.
///
/// Printing replaces every `__` separator with one `.` and an elaboration
/// suffix (`___elabb`/`___elabs`, eight bytes) with `'Elab_Body`/`'Elab_Spec`
/// (ten bytes), so the output is never longer than the input plus a small
/// fixed margin.
pub const MAX_OUTPUT_LEN: usize = MAX_INPUT_LEN + 8;

/// A parsed Ada/GNAT external name.
#[derive(Debug)]
pub struct Ast<'a> {
    pub(crate) name: Name<'a>,
}

/// A qualified Ada name with an optional elaboration suffix.
#[derive(Debug)]
pub(crate) struct Name<'a> {
    pub(crate) segments: Vec<&'a [u8]>,
    pub(crate) suffix: Option<ElabSuffix>,
}

/// The elaboration procedure suffix attached to a package or subprogram name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ElabSuffix {
    /// `___elabb`: the package body or subprogram elaboration procedure.
    Body,
    /// `___elabs`: the package spec elaboration procedure.
    Spec,
}
