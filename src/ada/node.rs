//! Ada/GNAT external-name model and budgets.
//!
//! GNAT does not encode a type grammar into a symbol. A symbol is the
//! fully-qualified Ada name with `.` replaced by `__`, optionally followed by a
//! compiler suffix: `N`/`P` on a protected operation, `_B<digits>s`/`_E<digits>s`
//! on an entry barrier or entry body, `TKB` on the body of a task object, or
//! `___elabb`/`___elabs` on a package elaboration procedure. The model mirrors
//! that structure: a list of identifier segments and one optional suffix.

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
