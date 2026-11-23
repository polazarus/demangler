//! Delphi/Object Pascal linker-name model and budgets.
//!
//! Free Pascal does not encode a type grammar. It builds a linker name from the
//! symbol's owning scope chain, the routine name and one token per parameter
//! type, and this module mirrors exactly that structure: a unit, a list of
//! class/record scopes, a routine name and a signature. Parameter types are
//! kept as the tokens Free Pascal emitted; only the open-array form is decoded,
//! because `array_of_` is a structural prefix rather than a type name.

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

/// One routine name with its owning scope chain and signature.
#[derive(Debug)]
pub(crate) struct Name<'a> {
    /// Unit name as mangled, including any namespace dots.
    pub(crate) unit: &'a [u8],
    /// True when the unit part is Free Pascal's `P$<program>` marker.
    pub(crate) program: bool,
    /// Enclosing class/record names, outermost first.
    pub(crate) scopes: Vec<&'a [u8]>,
    /// Routine name, including the leading `$` of an operator name.
    pub(crate) routine: &'a [u8],
    /// Parameter types in declaration order.
    pub(crate) params: Vec<Type<'a>>,
    /// Declared result type, absent for a procedure.
    pub(crate) ret: Option<Type<'a>>,
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
