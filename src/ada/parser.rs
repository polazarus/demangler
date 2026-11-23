//! Bounded parser for Ada/GNAT external names.
//!
//! GNAT encodes a library-unit or subprogram name by replacing every `.` with
//! `__` and lowercasing the Ada identifier. The parser accepts exactly that
//! user-level spelling plus the two documented suffixes in this slice:
//! protected-operation `N`/`P` and package elaboration `___elabb`/`___elabs`.
//! Compiler-generated task, record, enum and runtime suffixes are rejected
//! rather than guessed.
//!
//! The grammar is matched on bytes through the shared [`Cursor`]: a symbol name
//! is a byte string, so the parser never requires the whole input to be UTF-8.
//! Ada identifiers in external names are ASCII lowercase letters, digits and
//! `_`; anything else is rejected.

use alloc::vec::Vec;

use super::ParseError;
use super::node::{Ast, ElabSuffix, MAX_INPUT_LEN, MAX_SEGMENTS, MAX_WORK, Name};
use crate::parse::{Cursor, Fail, FailKind, Limits, State};

/// Parse an Ada/GNAT external name from raw bytes.
pub(crate) fn parse_bytes(symbol: &[u8]) -> Result<Ast<'_>, ParseError> {
    let symbol = crate::scheme::strip_decorations_bytes(symbol);
    if symbol.is_empty() {
        return Err(ParseError::Invalid);
    }
    if symbol.len() > MAX_INPUT_LEN {
        return Err(ParseError::BudgetExceeded);
    }

    let body = strip_platform_prefix(symbol);
    let (qualified, suffix) = split_elab_suffix(body);
    if qualified.is_empty() {
        return Err(ParseError::Invalid);
    }

    let mut state = State::new(limits());
    let base = u32::try_from(symbol.len() - qualified.len()).unwrap_or(u32::MAX);
    let mut cursor = Cursor::new(qualified, base, &mut state).map_err(map_fail)?;
    charge(&mut cursor, qualified.len())?;
    let (segments, protected) = parse_qualified(&mut cursor)?;
    // An elaboration suffix attaches to a package or subprogram name, not to a
    // protected operation. Rejecting the combination avoids inventing a name.
    if suffix.is_some() && protected {
        return Err(ParseError::Invalid);
    }
    Ok(Ast {
        name: Name { segments, suffix },
    })
}

/// The budgets the shared cursor enforces.
///
/// The grammar is flat, so the depth budget is never entered; the segment
/// limit is the node budget.
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

/// Charge `units` of work against the shared budget without moving the cursor.
///
/// The shared cursor is the only work meter the combinator layer exposes, so a
/// scan that works on a slice charges by advancing over it and rewinding.
fn charge(cursor: &mut Cursor<'_, '_>, units: usize) -> Result<(), ParseError> {
    let checkpoint = cursor.checkpoint();
    let _ = cursor.advance(units).map_err(map_fail)?;
    cursor.reset(checkpoint);
    Ok(())
}

/// Strip one or two leading platform underscores from a high-confidence Ada
/// prefix.
///
/// A Mach-O symbol table prefixes every name with `_`, so `ada__foo` appears as
/// `_ada__foo` and, when the original name already starts with `_`,
/// `__ada__foo`. Only the `ada__` family is accepted here; general GNAT names
/// are not auto-detected.
fn strip_platform_prefix(symbol: &[u8]) -> &[u8] {
    if symbol.starts_with(b"__ada__") {
        symbol.get(2..).unwrap_or(symbol)
    } else if symbol.starts_with(b"_ada__") {
        symbol.get(1..).unwrap_or(symbol)
    } else {
        symbol
    }
}

/// Split a trailing elaboration suffix from the qualified name.
fn split_elab_suffix(symbol: &[u8]) -> (&[u8], Option<ElabSuffix>) {
    symbol.strip_suffix(b"___elabb").map_or_else(
        || {
            symbol
                .strip_suffix(b"___elabs")
                .map_or((symbol, None), |body| (body, Some(ElabSuffix::Spec)))
        },
        |body| (body, Some(ElabSuffix::Body)),
    )
}

/// Parse the `__`-separated qualified name.
///
/// Returns the identifier segments and whether a protected-operation `N`/`P`
/// suffix was stripped from the last segment.
fn parse_qualified<'a>(cursor: &mut Cursor<'a, '_>) -> Result<(Vec<&'a [u8]>, bool), ParseError> {
    let mut segments = Vec::new();
    let mut protected = false;
    loop {
        if segments.len() >= MAX_SEGMENTS {
            return Err(ParseError::BudgetExceeded);
        }
        let segment = parse_identifier(cursor)?;
        match cursor.peek() {
            None => {
                segments.push(segment);
                break;
            }
            Some(b'_') => {
                if cursor.peek_at(1) != Some(b'_') {
                    return Err(ParseError::Invalid);
                }
                advance(cursor, 2)?;
                if cursor.peek().is_none() {
                    return Err(ParseError::Invalid);
                }
                segments.push(segment);
            }
            Some(b'N' | b'P') => {
                advance(cursor, 1)?;
                if cursor.peek().is_some() {
                    return Err(ParseError::Invalid);
                }
                segments.push(segment);
                protected = true;
                break;
            }
            Some(_) => return Err(ParseError::Invalid),
        }
    }
    if !cursor.remaining().is_empty() {
        return Err(ParseError::Invalid);
    }
    Ok((segments, protected))
}

/// Parse one lowercase Ada identifier.
///
/// An identifier starts with a lowercase ASCII letter. Digits and `_` may
/// follow, but `_` must be followed by another identifier character, so a
/// trailing underscore and a `__` separator are both left for the caller.
fn parse_identifier<'a>(cursor: &mut Cursor<'a, '_>) -> Result<&'a [u8], ParseError> {
    let start = cursor.pos();
    let Some(first) = cursor.peek() else {
        return Err(ParseError::Invalid);
    };
    if !first.is_ascii_lowercase() {
        return Err(ParseError::Invalid);
    }
    advance(cursor, 1)?;
    loop {
        match cursor.peek() {
            Some(byte) if byte.is_ascii_lowercase() || byte.is_ascii_digit() => {
                advance(cursor, 1)?;
            }
            Some(b'_') => {
                let Some(next) = cursor.peek_at(1) else {
                    break;
                };
                if next.is_ascii_lowercase() || next.is_ascii_digit() {
                    advance(cursor, 1)?;
                } else {
                    break;
                }
            }
            _ => break,
        }
    }
    cursor
        .bytes_between(start, cursor.pos())
        .ok_or(ParseError::Invalid)
}

/// Advance the cursor by `len` bytes, translating budget failures.
fn advance(cursor: &mut Cursor<'_, '_>, len: usize) -> Result<(), ParseError> {
    let _ = cursor.advance(len).map_err(map_fail)?;
    Ok(())
}

/// True when a symbol has a high-confidence GNAT prefix.
///
/// Only the `ada__`, `_ada__`, `__ada__`, `system__` and `gnat__` prefixes are
/// considered. This is a prefix prefilter: [`crate::ada::parse`] still rejects
/// unsupported bodies such as task and runtime suffixes. General `pkg__subp`
/// names are valid Ada but indistinguishable from plain C, so they are not
/// auto-detected and remain available through [`crate::ada::parse`].
#[must_use]
pub fn is_ada(symbol: &[u8]) -> bool {
    let symbol = crate::scheme::strip_decorations_bytes(symbol);
    if symbol.is_empty() || symbol.len() > MAX_INPUT_LEN {
        return false;
    }
    let body = strip_platform_prefix(symbol);
    body.starts_with(b"ada__") || body.starts_with(b"system__") || body.starts_with(b"gnat__")
}
