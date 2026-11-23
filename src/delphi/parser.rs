//! Bounded parser for Free Pascal linker names.
//!
//! Free Pascal's mangling is defined by `make_mangledname` in the compiler's
//! `symdef.pas` (release 3.2.2), summarised in `refs/delphi.md`:
//!
//! ```text
//! symbol  := unit-part [ "$_$" scope ] "_$$_" routine signature
//! unit-part := identifier | "P$" identifier          (program marker)
//! scope   := ( identifier "_$_" )+                   (class/record scopes)
//! routine := identifier | "$" operator-name
//! signature := ( "$" type )* [ "$$" type ]
//! ```
//!
//! The grammar is matched on bytes through the shared [`Cursor`]: a linker name
//! is a byte string, so the parser never requires the whole input to be UTF-8.
//! Free Pascal uppercases identifiers in linker names, so type tokens start
//! with an uppercase letter (or `_`); the lowercase compiler tokens
//! (`array_of_`, `error`, the `$crc`/`$CRC` substitutions) are either handled
//! structurally or rejected.

use alloc::vec::Vec;

use super::ParseError;
use super::node::{Ast, MAX_INPUT_LEN, MAX_PARAMS, MAX_SCOPES, MAX_WORK, Name, Type};
use crate::parse::{Cursor, Fail, FailKind, Limits, State};

/// Free Pascal's open-array-of-`const` token.
const ARRAY_OF_CONST: &[u8] = b"array_of_const";

/// Free Pascal's open-array token prefix; the element type follows it.
const ARRAY_OF: &[u8] = b"array_of_";

/// Parse a Free Pascal linker name from raw bytes.
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
    let name = parse_name(&mut cursor)?;
    if !cursor.remaining().is_empty() {
        return Err(ParseError::Invalid);
    }
    Ok(Ast { name })
}

/// The budgets the shared cursor enforces.
///
/// The grammar is flat, so the depth budget is never entered; the scope and
/// parameter limits are the node budget.
fn limits() -> Limits {
    Limits {
        input: MAX_INPUT_LEN as u32,
        depth: 8,
        work: MAX_WORK as u32,
        nodes: 0,
    }
}

/// Translate a shared-cursor failure into this backend's public error.
fn map_fail(fail: Fail) -> ParseError {
    match fail.kind() {
        FailKind::Depth | FailKind::Budget => ParseError::BudgetExceeded,
        FailKind::Invalid => ParseError::Invalid,
    }
}

/// Advance the cursor by `len` bytes, translating budget failures.
fn advance(cursor: &mut Cursor<'_, '_>, len: usize) -> Result<(), ParseError> {
    let _ = cursor.advance(len).map_err(map_fail)?;
    Ok(())
}

/// Parse one complete linker name.
fn parse_name<'a>(cursor: &mut Cursor<'a, '_>) -> Result<Name<'a>, ParseError> {
    let program = cursor.starts_with(b"P$");
    if program {
        advance(cursor, 2)?;
    }
    let unit = parse_identifier(cursor, true)?;

    let mut scopes = Vec::new();
    if cursor.starts_with(b"$_$") {
        advance(cursor, 3)?;
        loop {
            if scopes.len() >= MAX_SCOPES {
                return Err(ParseError::BudgetExceeded);
            }
            scopes.push(parse_identifier(cursor, false)?);
            if !cursor.starts_with(b"_$_") {
                return Err(ParseError::Invalid);
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
        return Err(ParseError::Invalid);
    }
    advance(cursor, 4)?;

    let routine = parse_routine(cursor)?;
    let (params, ret) = parse_signature(cursor)?;
    Ok(Name {
        unit,
        program,
        scopes,
        routine,
        params,
        ret,
    })
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
            return Err(ParseError::Invalid);
        }
        if !namespaced || cursor.peek() != Some(b'.') {
            break;
        }
        advance(cursor, 1)?;
    }
    cursor
        .bytes_between(start, cursor.pos())
        .ok_or(ParseError::Invalid)
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
/// and class-constructor names (`$plus`, `$create`).
fn parse_routine<'a>(cursor: &mut Cursor<'a, '_>) -> Result<&'a [u8], ParseError> {
    let start = cursor.pos();
    if cursor.peek() == Some(b'$') {
        advance(cursor, 1)?;
        let name_start = cursor.pos();
        while matches!(cursor.peek(), Some(byte) if is_operator_byte(byte)) {
            advance(cursor, 1)?;
        }
        if cursor.pos() == name_start {
            return Err(ParseError::Invalid);
        }
    } else {
        return parse_identifier(cursor, false);
    }
    cursor
        .bytes_between(start, cursor.pos())
        .ok_or(ParseError::Invalid)
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
                if params.len() >= MAX_PARAMS {
                    return Err(ParseError::BudgetExceeded);
                }
                params.push(parse_type(cursor)?);
            }
            Some(_) => return Err(ParseError::Invalid),
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
        _ => return Err(ParseError::Invalid),
    }
    let name = parse_identifier(cursor, true)?;
    if name == b"FILE" || name == b"OF" {
        return Err(ParseError::Invalid);
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
/// never contain `$` and are therefore never claimed.
#[must_use]
pub fn is_delphi(symbol: &[u8]) -> bool {
    parse_bytes(symbol).is_ok()
}
