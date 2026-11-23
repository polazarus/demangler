//! Delphi/Object Pascal linker-name model and budgets.
//!
//! Free Pascal does not encode a type grammar. It builds a linker name from the
//! symbol's owning scope chain, the symbol name and, for a routine, one token
//! per parameter type, and this module mirrors exactly that structure: a unit, a
//! list of class/record scopes, a member name and a signature. Parameter types
//! are kept as the tokens Free Pascal emitted; only the open-array form is
//! decoded, because `array_of_` is a structural prefix rather than a type name.
//!
//! Compiler-generated data symbols reuse the same chain behind a fixed prefix
//! (`U_$`, `VMT_$`, `RTTI_$`, `INIT_$`, `INIT$_$`, `FINALIZE$_$`), so they are
//! modelled by [`Kind`] rather than by a second grammar.

use alloc::vec::Vec;

/// Maximum accepted input length.
pub const MAX_INPUT_LEN: usize = 64 * 1024;

/// Maximum number of class/record scope components in one name.
///
/// This is also the parser's node budget for scopes: every component is one
/// allocation in the owned scope list.
pub const MAX_SCOPES: usize = 64;

/// Maximum number of parameter types in one signature.
pub const MAX_PARAMS: usize = 256;

/// Maximum number of parser work units (bytes scanned).
pub const MAX_WORK: usize = 8 * MAX_INPUT_LEN;

/// Maximum printed output length.
///
/// Printing mostly reproduces the mangled text: `array_of_X` prints shorter as
/// `array of X`. A byte that is not valid UTF-8 costs four bytes as `\xNN`, so
/// four times the input bounds every printed name.
pub const MAX_OUTPUT_LEN: usize = 4 * MAX_INPUT_LEN;

/// A parsed Free Pascal linker name.
#[derive(Debug)]
pub struct Ast<'a> {
    pub(crate) name: Name<'a>,
}

/// Which compiler symbol class a linker name belongs to.
///
/// Free Pascal puts a fixed marker in front of the `<unit>[$_$scope]_$$_<name>`
/// chain for compiler-generated data symbols. The marker is part of the linker
/// name, so the printer writes it verbatim; only a routine carries a signature.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    /// A routine: `<unit>[$_$scope]_$$_<name>` plus its signature.
    Routine,
    /// `U_$<chain>`: a unit, class or thread variable.
    UnitVariable,
    /// `VMT_$<chain>`: a class's virtual method table.
    VirtualMethodTable,
    /// `RTTI_$<chain>`: the runtime type information of a type.
    RuntimeTypeInfo,
    /// `INIT_$<chain>`: a type's initialization table.
    TypeInit,
    /// `INIT$_$<unit>`: a unit's initialization entry.
    UnitInit,
    /// `FINALIZE$_$<unit>`: a unit's finalization entry.
    UnitFinalize,
}

impl Kind {
    /// True for the three table symbols Free Pascal emits in a `$indirect`
    /// variant next to the plain one.
    pub(crate) const fn is_table(self) -> bool {
        matches!(
            self,
            Self::VirtualMethodTable | Self::RuntimeTypeInfo | Self::TypeInit
        )
    }
}

/// One symbol name with its owning scope chain and signature.
#[derive(Debug)]
pub(crate) struct Name<'a> {
    /// Compiler symbol class; decides the prefix the printer writes.
    pub(crate) kind: Kind,
    /// Unit name as mangled, including any namespace dots.
    pub(crate) unit: &'a [u8],
    /// True when the unit part is Free Pascal's `P$<program>` marker.
    pub(crate) program: bool,
    /// Enclosing class/record names, outermost first.
    pub(crate) scopes: Vec<&'a [u8]>,
    /// Symbol name, including the leading `$` of an operator name. Absent for
    /// the unit entry symbols `INIT$_$<unit>` and `FINALIZE$_$<unit>`, which
    /// name the unit alone.
    pub(crate) member: Option<&'a [u8]>,
    /// Parameter types in declaration order; empty for a data symbol.
    pub(crate) params: Vec<Type<'a>>,
    /// Declared result type, absent for a procedure.
    pub(crate) ret: Option<Type<'a>>,
    /// True for the `$indirect` variant of a `VMT_$`/`RTTI_$`/`INIT_$` table.
    pub(crate) indirect: bool,
}

/// One parameter or result type token.
#[derive(Debug)]
pub(crate) enum Type<'a> {
    /// A named type, printed as mangled (Free Pascal uppercases identifiers).
    Named(&'a [u8]),
    /// An open array (`array_of_X`), printed as `array of X`.
    ArrayOf(&'a [u8]),
    /// An open array of `const` (`array_of_const`).
    ArrayOfConst,
}
