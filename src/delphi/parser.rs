//! Bounded parser for Free Pascal linker names.
//!
//! Free Pascal's mangling is defined by `make_mangledname` in the compiler's
//! `symdef.pas` (release 3.2.2), summarised in `refs/delphi.md`:
//!
//! ```text
//! symbol      := [ data-prefix ] unit-part [ "$_$" scope ] [ "_$$_" name ]
//! data-prefix := "U_$" | "VMT_$" | "RTTI_$" | "INIT_$" | "INIT$_$" | "FINALIZE$_$"
//! unit-part   := identifier | "P$" identifier        (program marker)
//! scope       := ( identifier "_$_" )+               (class/record scopes)
//! name        := identifier | "$" operator-name      (routine or data symbol)
//! signature   := ( "$" type )* [ "$$" type ]
//! ```
//!
//! The grammar is matched on bytes through the shared [`Cursor`]: a linker name
//! is a byte string, so the parser never requires the whole input to be UTF-8.
//! Free Pascal uppercases identifiers in linker names, so type tokens start
//! with an uppercase letter (or `_`); the lowercase compiler tokens
//! (`array_of_`, `error`, the `$crc`/`$CRC` substitutions) are either handled
//! structurally or rejected.

use alloc::vec::Vec;

use super::node::{Ast, Kind, Name, ParseLimits, Type};
use crate::parse::{Cursor, State};
use crate::{BudgetKind, ParseError, ParseErrorKind};

/// Free Pascal's open-array-of-`const` token.
const ARRAY_OF_CONST: &[u8] = b"array_of_const";

/// Free Pascal's open-array token prefix; the element type follows it.
const ARRAY_OF: &[u8] = b"array_of_";

/// The markers Free Pascal puts in front of a compiler-generated data symbol.
///
/// Every marker ends with `$`, which no Pascal identifier and therefore no unit
/// name can contain, so the routine grammar can never start with these bytes
/// and the data grammar is tried first. `INIT$_$` is listed before `INIT_$`
/// because it is the longer match.
const DATA_PREFIXES: [(&[u8], Kind); 6] = [
    (b"INIT$_$", Kind::UnitInit),
    (b"FINALIZE$_$", Kind::UnitFinalize),
    (b"U_$", Kind::UnitVariable),
    (b"VMT_$", Kind::VirtualMethodTable),
    (b"RTTI_$", Kind::RuntimeTypeInfo),
    (b"INIT_$", Kind::TypeInit),
];

/// The `$indirect` variant of a `VMT_$`/`RTTI_$`/`INIT_$` table.
///
/// Free Pascal emits every table twice, once plain and once with this marker
/// (measured on the fixture). `$` is not an identifier byte, so the marker
/// cannot be part of the type name.
const INDIRECT: &[u8] = b"$indirect";

/// The compiler-generated unit entry routines.
///
/// A unit with an `initialization` section links `<unit>_$$_init$`, one with a
/// `finalization` section `<unit>_$$_finalize$`, and a unit whose managed
/// variables need an implicit finalizer `<unit>_$$_finalize_implicit$`
/// (measured with Free Pascal 3.2.2). The trailing `$` is the compiler's; no
/// Pascal identifier can contain it, so the three names are matched exactly
/// instead of being guessed from a prefix.
const ENTRY_ROUTINES: [&[u8]; 3] = [b"init$", b"finalize$", b"finalize_implicit$"];

/// Parse a Free Pascal linker name from raw bytes within `limits`.
pub fn parse_bytes(symbol: &[u8], limits: ParseLimits) -> Result<Ast<'_>, ParseError> {
    let symbol = crate::scheme::strip_decorations_bytes(symbol);
    if symbol.is_empty() {
        return Err(ParseError::new(ParseErrorKind::Invalid));
    }
    if symbol.len() > limits.input {
        return Err(ParseError::budget(BudgetKind::Input));
    }

    let mut state = State::new(limits.cursor());
    let mut cursor = Cursor::new(symbol, 0, &mut state)?;
    let name = parse_name(&mut cursor)?;
    if !cursor.remaining().is_empty() {
        return Err(cursor.invalid());
    }
    Ok(Ast { name })
}

/// Advance the cursor by `len` bytes, translating budget failures.
fn advance(cursor: &mut Cursor<'_, '_>, len: usize) -> Result<(), ParseError> {
    let _ = cursor.advance(len)?;
    Ok(())
}

/// Parse one complete linker name.
fn parse_name<'a>(cursor: &mut Cursor<'a, '_>) -> Result<Name<'a>, ParseError> {
    for (prefix, kind) in DATA_PREFIXES {
        if !cursor.starts_with(prefix) {
            continue;
        }
        // A data prefix ends with `$`, which no unit name can contain, so the
        // routine grammar can never start with the same bytes. Falling back
        // keeps a unit whose name matches a prefix parseable: `U_$$_PROC` is
        // the routine `PROC` of `unit U`, not a `U_$` symbol.
        let checkpoint = cursor.checkpoint();
        advance(cursor, prefix.len())?;
        match parse_data_name(cursor, kind) {
            Ok(name) => return Ok(name),
            Err(error) if error.budget_kind().is_none() => cursor.reset(checkpoint),
            Err(error) => return Err(error),
        }
    }

    let (program, unit) = parse_unit_part(cursor)?;
    let scopes = parse_scopes(cursor)?;
    let member = parse_routine(cursor)?;
    let (params, ret) = parse_signature(cursor)?;
    Ok(Name {
        kind: Kind::Routine,
        unit,
        program,
        scopes,
        member: Some(member),
        params,
        ret,
        indirect: false,
    })
}

/// Parse one compiler-generated data symbol after its prefix.
///
/// The two unit entry symbols name the unit alone; every other data symbol
/// carries the same scope chain and name a routine does, without a signature.
fn parse_data_name<'a>(cursor: &mut Cursor<'a, '_>, kind: Kind) -> Result<Name<'a>, ParseError> {
    let (program, unit) = parse_unit_part(cursor)?;
    let entry = matches!(kind, Kind::UnitInit | Kind::UnitFinalize);
    let (scopes, member, indirect) = if entry {
        (Vec::new(), None, false)
    } else {
        let scopes = parse_scopes(cursor)?;
        let member = parse_identifier(cursor, false)?;
        // Only a table is emitted twice; a variable has no `$indirect` form.
        let indirect = kind.is_table() && parse_indirect(cursor)?;
        (scopes, Some(member), indirect)
    };
    Ok(Name {
        kind,
        unit,
        program,
        scopes,
        member,
        params: Vec::new(),
        ret: None,
        indirect,
    })
}

/// Parse the optional `P$` program marker and the unit name.
fn parse_unit_part<'a>(cursor: &mut Cursor<'a, '_>) -> Result<(bool, &'a [u8]), ParseError> {
    let program = cursor.starts_with(b"P$");
    if program {
        advance(cursor, 2)?;
    }
    let unit = parse_identifier(cursor, true)?;
    Ok((program, unit))
}

/// Parse the optional `$_$`-separated class/record scope chain.
///
/// Consumes the `_$$_` separator that ends the chain.
fn parse_scopes<'a>(cursor: &mut Cursor<'a, '_>) -> Result<Vec<&'a [u8]>, ParseError> {
    let mut scopes = Vec::new();
    if cursor.starts_with(b"$_$") {
        advance(cursor, 3)?;
        loop {
            scopes.push(parse_identifier(cursor, false)?);
            if !cursor.starts_with(b"_$_") {
                return Err(cursor.invalid());
            }
            advance(cursor, 3)?;
            // A class scope always ends with `_$_`, so the signature separator
            // `_$$_` only follows the last component.
            if cursor.starts_with(b"_$$_") {
                break;
            }
        }
    }
    if !cursor.starts_with(b"_$$_") {
        return Err(cursor.invalid());
    }
    advance(cursor, 4)?;
    Ok(scopes)
}

/// Parse the optional `$indirect` marker of a table symbol.
fn parse_indirect(cursor: &mut Cursor<'_, '_>) -> Result<bool, ParseError> {
    if !cursor.starts_with(INDIRECT) {
        return Ok(false);
    }
    advance(cursor, INDIRECT.len())?;
    Ok(true)
}

/// Parse an identifier, optionally with `.`-separated namespace segments.
///
/// Unit names may be namespaced (`ns.esc`), so the unit allows dots; class and
/// routine names never contain one. Dots are structural, so a leading, trailing
/// or doubled dot is rejected.
fn parse_identifier<'a>(
    cursor: &mut Cursor<'a, '_>,
    namespaced: bool,
) -> Result<&'a [u8], ParseError> {
    let start = cursor.pos();
    loop {
        let segment = cursor.pos();
        while matches!(cursor.peek(), Some(byte) if is_identifier_byte(byte))
            && !at_separator(cursor)
        {
            advance(cursor, 1)?;
        }
        if cursor.pos() == segment {
            return Err(cursor.invalid());
        }
        if !namespaced || cursor.peek() != Some(b'.') {
            break;
        }
        advance(cursor, 1)?;
    }
    cursor
        .bytes_between(start, cursor.pos())
        .ok_or_else(|| cursor.invalid())
}

/// True when the cursor is at a Free Pascal scope separator.
///
/// `_` is an identifier byte, so the identifier scan stops before a separator
/// instead of swallowing its leading underscore. A class name may itself end
/// with `_`: `TFOO_` mangles as `TFOO__$__$$_…`, and the scan stops before the
/// `_$_` that follows it.
fn at_separator(cursor: &Cursor<'_, '_>) -> bool {
    cursor.starts_with(b"_$$_") || cursor.starts_with(b"_$_")
}

/// Parse a routine name, including the `$` prefix Free Pascal adds to operator
/// and class-constructor names (`$plus`, `$create`) and the trailing `$` of a
/// compiler-generated unit entry.
fn parse_routine<'a>(cursor: &mut Cursor<'a, '_>) -> Result<&'a [u8], ParseError> {
    for entry in ENTRY_ROUTINES {
        if cursor.starts_with(entry) {
            advance(cursor, entry.len())?;
            return Ok(entry);
        }
    }
    let start = cursor.pos();
    if cursor.peek() == Some(b'$') {
        advance(cursor, 1)?;
        let name_start = cursor.pos();
        while matches!(cursor.peek(), Some(byte) if is_operator_byte(byte)) {
            advance(cursor, 1)?;
        }
        if cursor.pos() == name_start {
            return Err(cursor.invalid());
        }
    } else {
        return parse_identifier(cursor, false);
    }
    cursor
        .bytes_between(start, cursor.pos())
        .ok_or_else(|| cursor.invalid())
}

/// Parse the `$`-separated parameter list and the optional `$$` result type.
fn parse_signature<'a>(
    cursor: &mut Cursor<'a, '_>,
) -> Result<(Vec<Type<'a>>, Option<Type<'a>>), ParseError> {
    let mut params = Vec::new();
    let mut ret = None;
    loop {
        match cursor.peek() {
            None => break,
            Some(b'$') => {
                if cursor.starts_with(b"$$") {
                    advance(cursor, 2)?;
                    ret = Some(parse_type(cursor)?);
                    break;
                }
                advance(cursor, 1)?;
                params.push(parse_type(cursor)?);
            }
            Some(_) => return Err(cursor.invalid()),
        }
    }
    Ok((params, ret))
}

/// Parse one parameter or result type token.
fn parse_type<'a>(cursor: &mut Cursor<'a, '_>) -> Result<Type<'a>, ParseError> {
    if cursor.starts_with(ARRAY_OF_CONST) && ends_token(cursor, ARRAY_OF_CONST.len()) {
        advance(cursor, ARRAY_OF_CONST.len())?;
        return Ok(Type::ArrayOfConst);
    }
    if cursor.starts_with(ARRAY_OF) {
        advance(cursor, ARRAY_OF.len())?;
        return Ok(Type::ArrayOf(parse_type_name(cursor)?));
    }
    Ok(Type::Named(parse_type_name(cursor)?))
}

/// True when the byte `offset` bytes ahead is absent or starts the next token.
fn ends_token(cursor: &Cursor<'_, '_>, offset: usize) -> bool {
    matches!(cursor.peek_at(offset), None | Some(b'$'))
}

/// Parse a named type token: an identifier, optionally with `.`-separated
/// nested-type segments.
///
/// Free Pascal uppercases identifiers in linker names, so a token that starts
/// with a lowercase letter is a compiler marker rather than a type name: the
/// `$crc` parameter substitution, the `error` placeholder or the
/// `$_nestedprovar` spelling. They are rejected instead of guessed.
///
/// A `file of X` parameter mangles as `$FILE$OF$X`, which is byte-identical to
/// three parameters named `FILE`, `OF` and `X`. `file` and `of` are reserved
/// words, so those two tokens cannot be type names; rejecting them keeps the
/// ambiguous form out instead of printing a guess.
fn parse_type_name<'a>(cursor: &mut Cursor<'a, '_>) -> Result<&'a [u8], ParseError> {
    match cursor.peek() {
        Some(byte) if byte.is_ascii_uppercase() || byte == b'_' => {}
        _ => return Err(cursor.invalid()),
    }
    let name = parse_identifier(cursor, true)?;
    if name == b"FILE" || name == b"OF" {
        return Err(cursor.invalid());
    }
    Ok(name)
}

/// True for an identifier byte: ASCII letters, digits and `_`.
const fn is_identifier_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

/// True for the body of a `$`-prefixed operator name.
///
/// Free Pascal's operator and class-constructor names are lowercase
/// (`$plus`, `$minus`, `$create`, `$destroy`).
const fn is_operator_byte(byte: u8) -> bool {
    byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_'
}

/// True when a symbol is a fully valid Free Pascal linker name.
///
/// Free Pascal names have no unambiguous prefix, so detection is a full parse:
/// a name is claimed only when the whole grammar is accepted. Ordinary C names
/// never contain `$` and are therefore never claimed. Detection runs with the
/// compiled-in budget; a [`crate::Demangler`] applies its configured budget to
/// the dispatch parse.
#[must_use]
pub fn is_delphi(symbol: &[u8]) -> bool {
    super::parse_bytes_with_budget(symbol, super::DEFAULT_BUDGET).is_ok()
}
