//! Legacy (`_ZN...E`) front-end.
//!
//! The legacy encoding is a flat, length-prefixed path with no types: every
//! element is a name segment, and the trailing element is usually the rustc
//! hash. Parsing is iterative (no recursion, no backrefs) and only checks the
//! length arithmetic; `$`-escapes are decoded while building the node text.
//!
//! The symbol is read as bytes through [`crate::parse::Cursor`]: lengths count
//! bytes, so an element may hold any byte sequence, including one that is not
//! UTF-8. Escaping is the printer's job.
//!
//! Reference: `refs/rust/rustc-demangle/src/legacy.rs` (decoder) and
//! `refs/rust/rustc/compiler/rustc_symbol_mangling/src/legacy.rs` (encoder).

use alloc::borrow::Cow;
use alloc::vec::Vec;

use crate::parse::{
    Cursor, Fail, PResult, Parser, Span as ParseSpan, State, decimal_length, many0, tag,
};

use super::Failure;
use super::node::{Builder, Kind, NodeData, NodeRef, ParserLimits, Span};

/// The legacy mangling prefixes, longest first, with their text spelling.
const PREFIXES: [(&[u8], &str); 3] = [(b"__ZN", "__ZN"), (b"_ZN", "_ZN"), (b"ZN", "ZN")];

/// The legacy mangling prefix of `symbol`: its length and spelling.
pub(super) fn prefix(symbol: &[u8]) -> Option<(usize, &'static str)> {
    PREFIXES
        .iter()
        .find(|(prefix, _)| symbol.starts_with(prefix))
        .map(|&(prefix, text)| (prefix.len(), text))
}

/// Parse a legacy symbol, returning the path node and the end offset.
pub(super) fn parse(
    symbol: &[u8],
    prefix_len: usize,
    limits: ParserLimits,
) -> Result<(NodeRef<'_>, usize), Failure> {
    let mut state = State::new(limits.cursor());
    let mut input = Cursor::new(symbol, 0, &mut state).map_err(|_| Failure::Invalid)?;
    let _ = input.advance(prefix_len).map_err(|_| Failure::Invalid)?;

    // Elements are length-prefixed and the path ends at `E`; an element parser
    // that backtracks at `E` (or at any other byte) ends the list, and the
    // terminator check below rejects a missing `E`.
    let elements = many0(element).parse(&mut input).map_err(invalid)?;
    let _ = tag(b"E").parse(&mut input).map_err(invalid)?;
    let end = input.pos();

    let mut builder = Builder::new(limits);
    let mut children = Vec::with_capacity(elements.len());
    let count = elements.len();
    for (index, (span, raw)) in elements.into_iter().enumerate() {
        // Only the last element can be the trailing rustc hash.
        let is_hash = index + 1 == count && is_rust_hash(raw);
        children.push(builder.node(
            Kind::LegacyElement,
            Some(unescape(raw)),
            Some(span),
            NodeData::Hash(is_hash),
            Vec::new(),
        )?);
    }
    let span = Span {
        start: position(prefix_len),
        end: position(end),
    };
    let root = builder.node(Kind::LegacyPath, None, Some(span), NodeData::None, children)?;
    Ok((root, end))
}

/// One length-prefixed path element: its span and its raw bytes.
fn element<'a>(input: &mut Cursor<'a, '_>) -> PResult<(Span, &'a [u8])> {
    let len = decimal_length().parse(input)?;
    // The span covers the element bytes, not the length prefix.
    let start = input.checkpoint();
    let bytes = input.advance(len)?;
    let span: ParseSpan = input.span(start);
    Ok((
        Span {
            start: span.start,
            end: span.end,
        },
        bytes,
    ))
}

/// Map a cursor failure to the public error.
///
/// The cursor only reports byte-level failures here: the input length is
/// checked before parsing and the work, node and depth budgets stay with
/// [`Builder`] and the v0 parser's depth counter.
const fn invalid(_fail: Fail) -> Failure {
    Failure::Invalid
}

/// `offset` as a span bound, saturating at `u32::MAX`.
fn position(offset: usize) -> u32 {
    u32::try_from(offset).unwrap_or(u32::MAX)
}

/// A rustc hash is `h` followed by hex digits.
fn is_rust_hash(raw: &[u8]) -> bool {
    raw.strip_prefix(b"h")
        .is_some_and(|digits| digits.iter().all(u8::is_ascii_hexdigit))
}

/// Decode the `$`-escapes of one path element.
///
/// Borrowed when the element needs no rewriting, owned when it does. An
/// unrecognised escape ends decoding, and the rest is kept verbatim — exactly
/// like the reference printer. Bytes that are not part of an escape are copied
/// unchanged, so a non-UTF-8 element survives decoding.
fn unescape(element: &[u8]) -> Cow<'_, [u8]> {
    // Fast path: only `..` and `$` are rewritten.
    if !element.contains(&b'$') && !element.windows(2).any(|pair| pair == b"..") {
        return Cow::Borrowed(element);
    }

    let mut out = Vec::with_capacity(element.len());
    let mut rest = element;
    // A leading `_$` keeps the escape but drops the `_`.
    if rest.starts_with(b"_$") {
        rest = &rest[1..];
    }

    loop {
        if let Some(tail) = rest.strip_prefix(b".") {
            if let Some(tail) = tail.strip_prefix(b".") {
                out.extend_from_slice(b"::");
                rest = tail;
            } else {
                out.push(b'.');
                rest = tail;
            }
            continue;
        }

        let Some(tail) = rest.strip_prefix(b"$") else {
            match rest.iter().position(|&byte| byte == b'$' || byte == b'.') {
                Some(index) => {
                    out.extend_from_slice(&rest[..index]);
                    rest = &rest[index..];
                }
                None => break,
            }
            continue;
        };

        let Some(end) = tail.iter().position(|&byte| byte == b'$') else {
            break;
        };
        let escape = &tail[..end];
        let after = &tail[end + 1..];
        let Some(unescaped) = unescape_token(escape) else {
            break;
        };
        match unescaped {
            Unescaped::Text(text) => out.extend_from_slice(text),
            Unescaped::Char(c) => {
                let mut buf = [0_u8; 4];
                out.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
            }
        }
        rest = after;
    }

    out.extend_from_slice(rest);
    Cow::Owned(out)
}

/// The decoded form of a `$...$` escape.
enum Unescaped<'a> {
    /// A fixed replacement such as `$LT$` → `<`.
    Text(&'a [u8]),
    /// A `$u<hex>$` scalar value.
    Char(char),
}

/// Decode one escape, or `None` when it is unknown.
fn unescape_token(escape: &[u8]) -> Option<Unescaped<'_>> {
    let text: &[u8] = match escape {
        b"SP" => b"@",
        b"BP" => b"*",
        b"RF" => b"&",
        b"LT" => b"<",
        b"GT" => b">",
        b"LP" => b"(",
        b"RP" => b")",
        b"C" => b",",
        _ => {
            let digits = escape.strip_prefix(b"u")?;
            if digits.is_empty() {
                return None;
            }
            let value = digits.iter().try_fold(0_u32, |value, &byte| {
                let digit = match byte {
                    b'0'..=b'9' => u32::from(byte - b'0'),
                    b'a'..=b'f' => u32::from(byte - b'a') + 10,
                    _ => return None,
                };
                value.checked_mul(16)?.checked_add(digit)
            })?;
            let c = char::from_u32(value)?;
            // Control characters stay escaped.
            if c.is_control() {
                return None;
            }
            return Some(Unescaped::Char(c));
        }
    };
    Some(Unescaped::Text(text))
}

#[cfg(test)]
mod tests;
