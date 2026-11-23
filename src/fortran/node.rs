//! gfortran external-name model and budgets.
//!
//! gfortran encodes no type grammar into a symbol. A module procedure or
//! module variable is `__<module>_MOD_<member>`; an external procedure or
//! named common block is the lowercased Fortran name plus the underscore that
//! `-funderscoring` appends; the main program is the fixed name `MAIN__`. The
//! model mirrors exactly those three forms, so printing needs no further
//! parsing and cannot disagree with validation.

/// Maximum accepted input length.
pub const MAX_INPUT_LEN: usize = 64 * 1024;

/// Maximum parser recursion depth.
///
/// The grammar is flat: a symbol is one module-qualified name, one external
/// name or the main program. The parser never recurses, so the depth budget is
/// never entered; it is set for uniformity with the shared cursor.
pub const MAX_DEPTH: usize = 1;

/// Maximum number of parser work units (bytes scanned).
///
/// The parser makes one bounded left-to-right pass over the symbol, so this
/// budget is not reachable; the shared cursor enforces it like every other
/// backend.
pub const MAX_WORK: usize = 8 * MAX_INPUT_LEN;

/// Maximum printed output length.
///
/// Printing drops the `__` module prefix, replaces the five-byte `_MOD_`
/// marker with one `.` and drops the trailing underscore of an external name,
/// so the output is always shorter than the input. The printer enforces the cap
/// anyway, which keeps `Display` total for anything `parse` accepts.
pub const MAX_OUTPUT_LEN: usize = MAX_INPUT_LEN;

/// A parsed gfortran external name.
#[derive(Debug, Clone, Copy)]
pub struct Ast<'a> {
    pub(crate) name: Name<'a>,
}

/// The three symbol forms this backend recognises.
#[derive(Debug, Clone, Copy)]
pub(crate) enum Name<'a> {
    /// `__<module>_MOD_<member>`: a module procedure, module variable or
    /// module-level `entry`.
    Module {
        /// The module name, without the `__` prefix and `_MOD_` marker.
        module: &'a [u8],
        /// The entity name inside the module.
        member: &'a [u8],
    },
    /// `<name>_`: an external procedure, block-data unit or named common block.
    External {
        /// The Fortran name, without the appended underscore.
        name: &'a [u8],
    },
    /// `MAIN__`: the main program.
    Main,
}
