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

use crate::budget::{EffectiveBudget, Limit};
use crate::parse::Limits;

/// Maximum accepted input length.
///
/// This is the default a [`crate::Budget`] starts from; a [`crate::Demangler`]
/// may raise it up to a fixed 16 MiB hard ceiling.
pub const MAX_INPUT_LEN: usize = 64 * 1024;

/// Maximum number of parser work units (bytes scanned).
///
/// The parser makes one bounded left-to-right pass over the symbol, so this
/// budget is not reachable at the default input cap; the shared cursor enforces
/// it like every other backend. This is the default a [`crate::Budget`] starts
/// from; a [`crate::Demangler`] may raise it up to a fixed 64M-unit hard
/// ceiling.
pub const MAX_WORK: usize = 8 * MAX_INPUT_LEN;

/// Hard input ceiling.
///
/// The AST borrows the input, but the one-pass scan and the printed text grow
/// with it, so a raise stays bounded whatever the caller asks for.
pub const HARD_INPUT_LEN: usize = 16 * 1024 * 1024;

/// Hard work ceiling.
pub const HARD_WORK: usize = 64 * 1024 * 1024;

/// The input and work limits one parse enforces.
///
/// Resolved from a [`crate::Budget`]: an absent dimension keeps its compiled-in
/// default and every value is clamped to this module's hard ceiling. The
/// grammar is flat, so there is no depth or node dimension to resolve.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ParseLimits {
    /// Maximum accepted input length in bytes.
    pub(crate) input: usize,
    /// Maximum parser work units.
    pub(crate) work: usize,
}

impl ParseLimits {
    /// Resolve one scheme's budget for a parse of a gfortran external name.
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

/// A parsed gfortran external name.
#[derive(Debug, Clone, Copy)]
pub struct Ast<'a> {
    pub(crate) name: Name<'a>,
}

/// The symbol forms this backend recognises.
#[derive(Debug, Clone, Copy)]
pub enum Name<'a> {
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
pub enum HelperTag {
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
