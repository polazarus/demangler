//! Bounded parser for gfortran external names.
//!
//! The grammar is the gfortran mangling (GCC `gcc/fortran/mangle.cc`), restricted
//! to the forms whose reading is unambiguous:
//!
//! ```text
//! symbol   ::= module | external | "MAIN__"
//! module   ::= "__" ident "_MOD_" ident          (plus one Mach-O underscore)
//! external ::= ident "_"
//! ident    ::= [a-z] ( [a-z0-9_] )*
//! ```
//!
//! The grammar is matched on bytes through the shared [`Cursor`]: a symbol name
//! is a byte string, so the parser never requires the input to be UTF-8.
//! gfortran lowercases every Fortran name, so an identifier that contains an
//! uppercase byte, or that starts with `_`, cannot be a user entity; such names
//! are rejected rather than guessed.

use super::ParseError;
use super::node::{Ast, MAX_DEPTH, MAX_INPUT_LEN, MAX_WORK, Name};
use crate::parse::{Cursor, Fail, Limits, State};

/// The marker gfortran inserts between a module name and its member.
const MODULE_MARKER: &[u8] = b"_MOD_";

/// The fixed symbol gfortran emits for a main program.
const MAIN_PROGRAM: &[u8] = b"MAIN__";

/// Parse a gfortran external name from raw bytes.
pub(crate) fn parse_bytes(symbol: &[u8]) -> Result<Ast<'_>, ParseError> {
    let symbol = crate::scheme::strip_decorations_bytes(symbol);
    if symbol.is_empty() {
        return Err(ParseError::Invalid);
    }
    if symbol.len() > MAX_INPUT_LEN {
        return Err(ParseError::BudgetExceeded);
    }

    let mut state = State::new(limits());
    let mut cursor = Cursor::new(symbol, 0, &mut state).map_err(map_fail)?;
    let name = classify(symbol)?;
    // The parser makes one bounded pass over the symbol; charge it as work.
    let _ = cursor.advance(symbol.len()).map_err(map_fail)?;
    Ok(Ast { name })
}

/// The budgets the shared cursor enforces.
///
/// The parser allocates nothing and has a flat grammar, so there is no node
/// budget: the AST borrows at most two slices of the input.
fn limits() -> Limits {
    Limits {
        input: MAX_INPUT_LEN as u32,
        depth: MAX_DEPTH as u16,
        work: MAX_WORK as u32,
        nodes: 0,
    }
}

/// Translate a shared-cursor failure into this backend's public error.
fn map_fail(fail: Fail) -> ParseError {
    if fail.expected == b"input budget" || fail.expected == b"work budget" {
        ParseError::BudgetExceeded
    } else {
        ParseError::Invalid
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
        // A user entity is a lowercase identifier. gfortran's generated type
        // helpers (`__vtab_`, `__def_init_`, `__copy_`, `__final_`) start with
        // an underscore and are rejected here.
        if !is_identifier(member) {
            return Err(ParseError::Invalid);
        }
        return Ok(Name::Module { module, member });
    }
    external_name(symbol)
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

/// Find `needle` in `haystack`.
fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

/// True when a symbol has the distinctive gfortran module form.
///
/// Only `__<module>_MOD_<member>` (and its Mach-O spelling
/// `___<module>_MOD_<member>`) is claimed. External names are not claimed: an
/// ordinary C name such as `foo_` is indistinguishable from a gfortran
/// external name, so those stay C and remain available through
/// [`crate::fortran::parse`]. This is a shape prefilter: a module symbol whose
/// member is a generated helper is claimed here and then rejected by
/// [`crate::fortran::parse`], so it never renders as C.
#[must_use]
pub fn is_fortran(symbol: &[u8]) -> bool {
    let symbol = crate::scheme::strip_decorations_bytes(symbol);
    if symbol.is_empty() || symbol.len() > MAX_INPUT_LEN {
        return false;
    }
    module_shape(symbol).is_some()
}
