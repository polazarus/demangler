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
/// The parser only borrows the input, but the scope and parameter lists and
/// the printed text grow with it, so a raise stays bounded whatever the caller
/// asks for.
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
    /// Resolve one scheme's budget for a parse of a Free Pascal linker name.
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
