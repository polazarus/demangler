//! gfortran external-name model and budgets.
//!
//! gfortran encodes no type grammar into a symbol. A module procedure or
//! module variable is `__<module>_MOD_<member>`; an external procedure or
//! named common block is the lowercased Fortran name plus the underscore that
//! `-funderscoring` appends; the main program is the fixed name `MAIN__`. A
//! derived type in a module additionally gets four generated helpers whose
//! member is `__<helper>_<module>_<Type>`. The model mirrors exactly those
//! forms, so printing needs no further parsing and cannot disagree with
//! validation.

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
/// so the output is always shorter than the input. A derived-type helper drops
/// the duplicated `__<helper>_<module>_` prefix and appends at most
/// `'def_init`, which is still shorter than what it drops. The printer enforces
/// the cap anyway, which keeps `Display` total for anything `parse` accepts.
pub const MAX_OUTPUT_LEN: usize = MAX_INPUT_LEN;

/// A parsed gfortran external name.
#[derive(Debug, Clone, Copy)]
pub struct Ast<'a> {
    pub(crate) name: Name<'a>,
}

/// The symbol forms this backend recognises.
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
    /// `__<module>_MOD___<helper>_<module>_<Type>`: a generated helper of a
    /// module-level derived type.
    TypeHelper {
        /// The module that declares the type, which is also the module of the
        /// symbol.
        module: &'a [u8],
        /// Which generated helper this symbol is.
        tag: HelperTag,
        /// The type name as gfortran spells it in the helper: the first letter
        /// is upper case and the rest is lower case.
        type_name: &'a [u8],
    },
}

/// The generated helpers gfortran emits for a module-level derived type.
///
/// The member of the symbol is `__<tag>_<module>_<Type>`, where `<Type>` is the
/// type name with its first letter upper-cased. A derived type always gets
/// `vtab`, `def_init` and `copy`; it gets `final` only when it has a `final`
/// procedure binding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HelperTag {
    /// `__vtab_`: the type's vtable.
    Vtab,
    /// `__def_init_`: the default-initialization procedure.
    DefInit,
    /// `__copy_`: the copy procedure.
    Copy,
    /// `__final_`: the finalization wrapper.
    Final,
}

impl HelperTag {
    /// The marker text gfortran inserts between `__` and the type name.
    pub(crate) const fn marker(self) -> &'static [u8] {
        match self {
            Self::Vtab => b"vtab_",
            Self::DefInit => b"def_init_",
            Self::Copy => b"copy_",
            Self::Final => b"final_",
        }
    }

    /// The spelling this backend appends to the type name.
    pub(crate) const fn suffix(self) -> &'static str {
        match self {
            Self::Vtab => "'vtab",
            Self::DefInit => "'def_init",
            Self::Copy => "'copy",
            Self::Final => "'final",
        }
    }
}
