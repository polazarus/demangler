//! Bounded parser for gfortran external names.
//!
//! The grammar is the gfortran mangling (GCC `gcc/fortran/mangle.cc`), restricted
//! to the forms whose reading is unambiguous:
//!
//! ```text
//! symbol   ::= module | helper | external | "MAIN__"
//! module   ::= "__" ident "_MOD_" ident          (plus one Mach-O underscore)
//! helper   ::= "__" ident "_MOD_" "__" helper "_" ident "_" type
//! helper   ::= "vtab" | "def_init" | "copy" | "final"
//! type     ::= [A-Z] [a-z0-9_]*
//! external ::= ident "_"
//! ident    ::= [a-z] ( [a-z0-9_] )*
//! ```
//!
//! The grammar is matched on bytes through the shared [`Cursor`]: a symbol name
//! is a byte string, so the parser never requires the input to be UTF-8.
//! gfortran lowercases every Fortran name, so an identifier that contains an
//! uppercase byte, or that starts with `_`, cannot be a user entity; such names
//! are rejected rather than guessed. The one exception is the type name of a
//! generated derived-type helper, which gfortran writes with its first letter
//! upper-cased.

use super::ParseError;
use super::node::{Ast, HelperTag, MAX_INPUT_LEN, Name, ParseLimits};
use crate::parse::{Cursor, Fail, FailKind, State, find};

/// The marker gfortran inserts between a module name and its member.
const MODULE_MARKER: &[u8] = b"_MOD_";

/// The fixed symbol gfortran emits for a main program.
const MAIN_PROGRAM: &[u8] = b"MAIN__";

/// Parse a gfortran external name from raw bytes within `limits`.
pub fn parse_bytes(symbol: &[u8], limits: ParseLimits) -> Result<Ast<'_>, ParseError> {
    let symbol = crate::scheme::strip_decorations_bytes(symbol);
    if symbol.is_empty() {
        return Err(ParseError::Invalid);
    }
    if symbol.len() > limits.input {
        return Err(ParseError::BudgetExceeded);
    }

    let mut state = State::new(limits.cursor());
    let mut cursor = Cursor::new(symbol, 0, &mut state).map_err(map_fail)?;
    let name = classify(symbol)?;
    // The parser makes one bounded pass over the symbol; charge it as work.
    let _ = cursor.advance(symbol.len()).map_err(map_fail)?;
    Ok(Ast { name })
}

/// Translate a shared-cursor failure into this backend's public error.
const fn map_fail(fail: Fail) -> ParseError {
    match fail.kind {
        FailKind::InputBudget | FailKind::Work => ParseError::BudgetExceeded,
        _ => ParseError::Invalid,
    }
}

/// Classify one decoration-stripped symbol.
fn classify(symbol: &[u8]) -> Result<Name<'_>, ParseError> {
    if symbol == MAIN_PROGRAM {
        return Ok(Name::Main);
    }
    if symbol.starts_with(b"__") {
        // A `__` prefix is the module form. There is no external reading of it
        // (a Fortran name starts with a letter), so a malformed module symbol is
        // an error instead of falling through to the external rule.
        let (module, member) = module_shape(symbol).ok_or(ParseError::Invalid)?;
        // A user entity is a lowercase identifier. The generated derived-type
        // helpers start with `__` and carry the type name in gfortran's
        // capitalized spelling, so they are matched before the identifier rule.
        if let Some((tag, type_name)) = helper_shape(module, member) {
            return Ok(Name::TypeHelper {
                module,
                tag,
                type_name,
            });
        }
        if !is_identifier(member) {
            return Err(ParseError::Invalid);
        }
        return Ok(Name::Module { module, member });
    }
    external_name(symbol)
}

/// Split a generated derived-type helper member into its tag and type name.
///
/// gfortran names a helper `__<tag>_<module>_<Type>`, where `<module>` is the
/// lower-cased declaring module and `<Type>` is the type name with its first
/// letter upper-cased (`type :: point` becomes `Point`). Requiring the type
/// part to repeat the symbol's own module is what makes the reading
/// unambiguous: a helper for a type imported from another module (for example
/// `iso_c_binding`'s `c_ptr`) carries that module instead and is rejected.
///
/// Returns `None` for any member that is not a well-formed helper, including a
/// helper whose type part is not a `[A-Z][a-z0-9_]*` name.
fn helper_shape<'a>(module: &[u8], member: &'a [u8]) -> Option<(HelperTag, &'a [u8])> {
    let body = member.strip_prefix(b"__")?;
    let (tag, rest) = [
        HelperTag::Vtab,
        HelperTag::DefInit,
        HelperTag::Copy,
        HelperTag::Final,
    ]
    .into_iter()
    .find_map(|tag| Some((tag, body.strip_prefix(tag.marker())?)))?;
    let type_name = rest.strip_prefix(module)?.strip_prefix(b"_")?;
    if !is_type_name(type_name) {
        return None;
    }
    Some((tag, type_name))
}

/// True for the type spelling gfortran uses in a generated helper: the first
/// letter upper-cased, the rest lower-cased letters, digits and underscores.
fn is_type_name(bytes: &[u8]) -> bool {
    let Some((first, rest)) = bytes.split_first() else {
        return false;
    };
    first.is_ascii_uppercase()
        && rest
            .iter()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || *byte == b'_')
}

/// Split the module form into its module name and member.
///
/// Returns `None` unless the symbol has the `__` prefix, a lowercase module
/// identifier and a non-empty member. This is the shape test [`is_fortran`]
/// uses; [`classify`] additionally requires the member to be an identifier.
fn module_shape(symbol: &[u8]) -> Option<(&[u8], &[u8])> {
    let body = symbol.strip_prefix(b"__")?;
    if let Some(parts) = split_module(body) {
        return Some(parts);
    }
    // A Mach-O symbol table prefixes every name with `_`, so a gfortran module
    // symbol appears as `___<module>_MOD_<member>`.
    let body = symbol.strip_prefix(b"___")?;
    split_module(body)
}

/// Split `<module>_MOD_<member>` and validate the module identifier.
fn split_module(body: &[u8]) -> Option<(&[u8], &[u8])> {
    let index = find(body, MODULE_MARKER)?;
    let (module, rest) = body.split_at(index);
    let member = rest.get(MODULE_MARKER.len()..)?;
    if !is_identifier(module) || member.is_empty() {
        return None;
    }
    Some((module, member))
}

/// Split an external name: `<name>` plus the underscore `-funderscoring`
/// appends.
///
/// With the default `-funderscoring`, a Fortran name `n` becomes `n_`; with
/// `-fsecond-underscore` a name that already contains `_` becomes `n__`. Both
/// modes therefore produce a trailing `__`, and `x_y__` is ambiguous between
/// `x_y_` (one appended underscore) and `x_y` (second-underscore mode). Such a
/// name is rejected. `x__` is not ambiguous: second-underscore mode only doubles
/// an underscore that is already there, so `x__` is `x_` in the default mode.
fn external_name(symbol: &[u8]) -> Result<Name<'_>, ParseError> {
    let base = symbol.strip_suffix(b"_").ok_or(ParseError::Invalid)?;
    if let Some(without_second) = base.strip_suffix(b"_")
        && without_second.contains(&b'_')
    {
        return Err(ParseError::Invalid);
    }
    if !is_identifier(base) {
        return Err(ParseError::Invalid);
    }
    Ok(Name::External { name: base })
}

/// True for a Fortran identifier as gfortran writes it: a lowercase letter
/// followed by lowercase letters, digits and underscores.
///
/// gfortran lowercases every Fortran name, so an uppercase byte can only come
/// from a `bind(c)` name (which drops the mangling), from a foreign compiler or
/// from a C symbol.
fn is_identifier(bytes: &[u8]) -> bool {
    let Some((first, rest)) = bytes.split_first() else {
        return false;
    };
    first.is_ascii_lowercase()
        && rest
            .iter()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || *byte == b'_')
}

/// True when a symbol has the distinctive gfortran module form.
///
/// Only `__<module>_MOD_<member>` (and its Mach-O spelling
/// `___<module>_MOD_<member>`) is claimed. External names are not claimed: an
/// ordinary C name such as `foo_` is indistinguishable from a gfortran
/// external name, so those stay C and remain available through
/// [`crate::fortran::parse`]. This is a shape prefilter: a module symbol whose
/// member is a form this slice does not read is claimed here and then rejected
/// by [`crate::fortran::parse`], so it never renders as C.
#[must_use]
pub fn is_fortran(symbol: &[u8]) -> bool {
    is_fortran_with(symbol, MAX_INPUT_LEN)
}

/// [`is_fortran`] with one configured input ceiling.
///
/// Detection uses this so a raised input budget reaches a long Fortran name;
/// the public [`is_fortran`] keeps the compiled-in default.
#[must_use]
pub fn is_fortran_with(symbol: &[u8], max_input: usize) -> bool {
    let symbol = crate::scheme::strip_decorations_bytes(symbol);
    if symbol.is_empty() || symbol.len() > max_input {
        return false;
    }
    module_shape(symbol).is_some()
}
