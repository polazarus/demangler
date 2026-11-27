//! Bounded parser for Ada/GNAT external names.
//!
//! GNAT encodes a library-unit or subprogram name by replacing every `.` with
//! `__` and lowercasing the Ada identifier. The parser accepts exactly that
//! user-level spelling plus the compiler suffixes this slice reads:
//! protected-operation `N`/`P`, entry `_B<digits>s`/`_E<digits>s`, task-object
//! `TKB`, and package elaboration `___elabb`/`___elabs`. Other
//! compiler-generated suffixes are rejected rather than guessed.
//!
//! The grammar is matched on bytes through the shared [`Cursor`]: a symbol name
//! is a byte string, so the parser never requires the whole input to be UTF-8.
//! Ada identifiers in external names are ASCII lowercase letters, digits and
//! `_`; anything else is rejected. That is also why a compiler suffix can be
//! recognised safely: an uppercase byte can never be part of a user identifier.

use alloc::vec::Vec;

use super::Failure;
use super::node::{Ast, MAX_INPUT_LEN, Name, ParseLimits, Suffix};
use crate::parse::{Cursor, Fail, FailKind, State};

/// Parse an Ada/GNAT external name from raw bytes within `limits`.
pub fn parse_bytes(symbol: &[u8], limits: ParseLimits) -> Result<Ast<'_>, Failure> {
    let symbol = crate::scheme::strip_decorations_bytes(symbol);
    if symbol.is_empty() {
        return Err(Failure::Invalid);
    }
    if symbol.len() > limits.input {
        return Err(Failure::BudgetExceeded);
    }

    let body = strip_platform_prefix(symbol);
    let (qualified, elab) = split_elab_suffix(body);
    if qualified.is_empty() {
        return Err(Failure::Invalid);
    }

    let mut state = State::new(limits.cursor());
    let base = u32::try_from(symbol.len() - qualified.len()).unwrap_or(u32::MAX);
    let mut cursor = Cursor::new(qualified, base, &mut state).map_err(map_fail)?;
    charge(&mut cursor, qualified.len())?;
    let (segments, generated) = parse_qualified(&mut cursor)?;
    // A name carries one suffix. An elaboration suffix attaches to a package or
    // subprogram name, not to a generated operation, entry or task subprogram;
    // rejecting the combination avoids inventing a name.
    if elab.is_some() && generated.is_some() {
        return Err(Failure::Invalid);
    }
    Ok(Ast {
        name: Name {
            segments,
            suffix: elab.or(generated),
        },
    })
}

/// Translate a shared-cursor failure into this backend's public error.
const fn map_fail(fail: Fail) -> Failure {
    match fail.kind {
        FailKind::Depth | FailKind::InputBudget | FailKind::Work => Failure::BudgetExceeded,
        _ => Failure::Invalid,
    }
}

/// Charge `units` of work against the shared budget without moving the cursor.
///
/// The shared cursor is the only work meter the combinator layer exposes, so a
/// scan that works on a slice charges by advancing over it and rewinding.
fn charge(cursor: &mut Cursor<'_, '_>, units: usize) -> Result<(), Failure> {
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
fn split_elab_suffix(symbol: &[u8]) -> (&[u8], Option<Suffix>) {
    symbol.strip_suffix(b"___elabb").map_or_else(
        || {
            symbol
                .strip_suffix(b"___elabs")
                .map_or((symbol, None), |body| (body, Some(Suffix::ElabSpec)))
        },
        |body| (body, Some(Suffix::ElabBody)),
    )
}

/// Parse the `__`-separated qualified name.
///
/// Returns the identifier segments and the generated-subprogram suffix
/// (`N`/`P`, `_B<digits>s`, `_E<digits>s` or `TKB`) stripped from the last
/// segment, if any.
fn parse_qualified<'a>(
    cursor: &mut Cursor<'a, '_>,
) -> Result<(Vec<&'a [u8]>, Option<Suffix>), Failure> {
    let mut segments = Vec::new();
    loop {
        let segment = parse_identifier(cursor)?;
        match cursor.peek() {
            None => {
                segments.push(segment);
                break;
            }
            Some(b'_') if cursor.peek_at(1) == Some(b'_') => {
                advance(cursor, 2)?;
                if cursor.peek().is_none() {
                    return Err(Failure::Invalid);
                }
                segments.push(segment);
            }
            Some(b'_') => {
                let Some((suffix, len)) = entry_suffix(cursor.remaining()) else {
                    return Err(Failure::Invalid);
                };
                advance(cursor, len)?;
                segments.push(segment);
                return Ok((segments, Some(suffix)));
            }
            Some(b'N' | b'P') => {
                advance(cursor, 1)?;
                if cursor.peek().is_some() {
                    return Err(Failure::Invalid);
                }
                segments.push(segment);
                return Ok((segments, Some(Suffix::ProtectedOp)));
            }
            Some(b'T') if cursor.remaining() == b"TKB" => {
                advance(cursor, 3)?;
                segments.push(segment);
                return Ok((segments, Some(Suffix::TaskBody)));
            }
            Some(_) => return Err(Failure::Invalid),
        }
    }
    if !cursor.remaining().is_empty() {
        return Err(Failure::Invalid);
    }
    Ok((segments, None))
}

/// Match an entry barrier or entry body suffix on the remaining bytes.
///
/// GNAT appends `_B<digits>s` to the barrier procedure and `_E<digits>s` to the
/// body procedure of an entry or entry family, where `<digits>` is the entry's
/// number. The number is not interpreted, so both procedures print the entry
/// name. The suffix must consume the rest of the symbol; `c++filt -s gnat`
/// accepts the same shape, including one with no digits at all
/// (`pkg__foo_Bs`), so the digits are optional here too.
fn entry_suffix(rest: &[u8]) -> Option<(Suffix, usize)> {
    let suffix = match rest {
        [b'_', b'B', ..] => Suffix::EntryBarrier,
        [b'_', b'E', ..] => Suffix::EntryBody,
        _ => return None,
    };
    let mut len = 2;
    while matches!(rest.get(len), Some(b'0'..=b'9')) {
        len += 1;
    }
    if rest.get(len) != Some(&b's') {
        return None;
    }
    len += 1;
    (len == rest.len()).then_some((suffix, len))
}

/// Parse one lowercase Ada identifier.
///
/// An identifier starts with a lowercase ASCII letter. Digits and `_` may
/// follow, but `_` must be followed by another identifier character, so a
/// trailing underscore and a `__` separator are both left for the caller.
fn parse_identifier<'a>(cursor: &mut Cursor<'a, '_>) -> Result<&'a [u8], Failure> {
    let start = cursor.pos();
    let Some(first) = cursor.peek() else {
        return Err(Failure::Invalid);
    };
    if !first.is_ascii_lowercase() {
        return Err(Failure::Invalid);
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
        .ok_or(Failure::Invalid)
}

/// Advance the cursor by `len` bytes, translating budget failures.
fn advance(cursor: &mut Cursor<'_, '_>, len: usize) -> Result<(), Failure> {
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
    is_ada_with(symbol, MAX_INPUT_LEN)
}

/// [`is_ada`] with one configured input ceiling.
///
/// Detection uses this so a raised input budget reaches a long Ada name; the
/// public [`is_ada`] keeps the compiled-in default.
#[must_use]
pub fn is_ada_with(symbol: &[u8], max_input: usize) -> bool {
    let symbol = crate::scheme::strip_decorations_bytes(symbol);
    if symbol.is_empty() || symbol.len() > max_input {
        return false;
    }
    let body = strip_platform_prefix(symbol);
    body.starts_with(b"ada__") || body.starts_with(b"system__") || body.starts_with(b"gnat__")
}
