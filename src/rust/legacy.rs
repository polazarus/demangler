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

use super::ParseError;
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
) -> Result<(NodeRef<'_>, usize), ParseError> {
    let mut state = State::new(limits.cursor());
    let mut input = Cursor::new(symbol, 0, &mut state).map_err(|_| ParseError::Invalid)?;
    let _ = input.advance(prefix_len).map_err(|_| ParseError::Invalid)?;

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
fn invalid(_fail: Fail) -> ParseError {
    ParseError::Invalid
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
mod tests {
    use alloc::borrow::Cow;
    use alloc::format;
    use alloc::string::ToString;
    use alloc::vec::Vec;

    use super::*;
    use crate::rust::{Encoding, Kind, NodeData, parse};

    /// Assert both renderings, and cross-check them with the reference decoder.
    fn check(symbol: &str, display: &str, alternate: &str) {
        let ast = parse(symbol).unwrap();
        assert_eq!(ast.encoding(), Encoding::Legacy);
        assert_eq!(ast.to_string(), display, "{symbol}");
        assert_eq!(format!("{ast:#}"), alternate, "{symbol}");
        check_oracle(symbol, display, alternate);
    }

    /// Compare with `rustc-demangle`, the executable oracle.
    fn check_oracle(symbol: &str, display: &str, alternate: &str) {
        let oracle = rustc_demangle::try_demangle(symbol).unwrap();
        assert_eq!(format!("{oracle}"), display, "{symbol}");
        assert_eq!(format!("{oracle:#}"), alternate, "{symbol}");
    }

    /// Demangle raw symbol bytes, which the public `parse` cannot express.
    fn check_bytes(symbol: &[u8], display: &str) {
        let ast = crate::rust::parse_bytes(symbol).unwrap();
        assert_eq!(ast.to_string(), display, "{symbol:?}");
    }

    #[test]
    fn plain_paths() {
        check("_ZN4testE", "test", "test");
        check("_ZN3foo3barE", "foo::bar", "foo::bar");
        check("ZN3fooE", "foo", "foo");
        check("__ZN3fooE", "foo", "foo");
        // An empty path and an empty element both print nothing.
        check("_ZNE", "", "");
        check("_ZN0E", "", "");
    }

    #[test]
    fn hash_is_omitted_in_alternate_mode() {
        check(
            "_ZN3foo17h05af221e174051e9E",
            "foo::h05af221e174051e9",
            "foo",
        );
        // Only a trailing `h<hex>` element is a hash, so `h05` counts too.
        check("_ZN3foo3h05E", "foo::h05", "foo");
        // A non-hex tail is an ordinary element.
        check("_ZN3foo4h05zE", "foo::h05z", "foo::h05z");
    }

    #[test]
    fn escapes_are_decoded() {
        check("_ZN13_$LT$test$GT$E", "<test>", "<test>");
        check("_ZN8foo..barE", "foo::bar", "foo::bar");
        check("_ZN7foo.barE", "foo.bar", "foo.bar");
        check("_ZN12test$SP$testE", "test@test", "test@test");
        check("_ZN12test$BP$testE", "test*test", "test*test");
        check("_ZN12test$RF$testE", "test&test", "test&test");
        check("_ZN8$LP$$RP$E", "()", "()");
        check("_ZN8a$C$b.cdE", "a,b.cd", "a,b.cd");
        check("_ZN9$u20$testE", " test", " test");
        check("_ZN10$u7b$$u7d$E", "{}", "{}");
        check(
            "_ZN28_$u7b$$u7b$closure$u7d$$u7d$E",
            "{{closure}}",
            "{{closure}}",
        );
    }

    #[test]
    fn unknown_escapes_stop_decoding() {
        // An unknown escape ends decoding and the rest is kept verbatim.
        check("_ZN7a$zz$bcE", "a$zz$bc", "a$zz$bc");
        // Control characters are not decoded.
        check("_ZN7a$u0$bcE", "a$u0$bc", "a$u0$bc");
        // Uppercase hex is not an escape.
        check("_ZN7a$uFF$bE", "a$uFF$b", "a$uFF$b");
        // An unterminated escape is kept verbatim.
        check("_ZN4a$bcE", "a$bc", "a$bc");
    }

    #[test]
    fn elements_borrow_unless_rewritten() {
        let ast = parse("_ZN3fooE").unwrap();
        let element = ast.root().children().first().unwrap();
        assert_eq!(element.kind(), Kind::LegacyElement);
        assert_eq!(element.text(), Some(&b"foo"[..]));

        let ast = parse("_ZN8foo..barE").unwrap();
        let element = ast.root().children().first().unwrap();
        assert_eq!(element.text(), Some(&b"foo::bar"[..]));
    }

    #[test]
    fn hash_flag_is_recorded() {
        let ast = parse("_ZN3foo17h05af221e174051e9E").unwrap();
        let children = ast.root().children();
        assert_eq!(children.len(), 2);
        assert_eq!(children[0].data(), &NodeData::Hash(false));
        assert_eq!(children[1].data(), &NodeData::Hash(true));
    }

    #[test]
    fn spans_cover_the_element_bytes() {
        let symbol = "_ZN3foo3barE";
        let ast = parse(symbol).unwrap();
        let first = ast.root().children().first().unwrap();
        let span = first.span().unwrap();
        assert_eq!(
            symbol.get(span.start as usize..span.end as usize),
            Some("foo")
        );
    }

    #[test]
    fn malformed_symbols_are_rejected() {
        for symbol in [
            "_ZN",
            "_ZN3foo",
            "_ZNfooE",
            "_ZN3fooEv",
            "_ZN99999999999999999999fooE",
        ] {
            assert_eq!(parse(symbol).unwrap_err(), ParseError::Invalid, "{symbol}");
        }
    }

    /// A length is a byte count, so an element may hold bytes that are not
    /// UTF-8; the printer escapes them instead of the parser rejecting them.
    #[test]
    fn invalid_utf8_elements_are_escaped() {
        check_bytes(b"_ZN4F\xffoo3barE", "F\\xffoo::bar");
        // A length that cuts a UTF-8 sequence in half is legal at this layer.
        check_bytes(b"_ZN1\xc3E", "\\xc3");
        // Valid UTF-8 keeps printing verbatim.
        check_bytes("_ZN5féstE".as_bytes(), "fést");
    }

    /// Truncating a valid symbol never panics and never escapes the budgets.
    #[test]
    fn truncations_are_rejected() {
        let symbol = b"_ZN4test4main17h1234567890abcdefE";
        for end in 0..symbol.len() {
            let _ = crate::rust::parse_bytes(&symbol[..end]);
        }
        // Dropping the terminator leaves the path unterminated.
        assert_eq!(
            crate::rust::parse_bytes(&symbol[..symbol.len() - 1]).unwrap_err(),
            ParseError::Invalid
        );
    }

    /// A length that runs past the end of the symbol is rejected at the
    /// length prefix, not accepted as a short element.
    #[test]
    fn length_prefixes_must_fit() {
        for symbol in [&b"_ZN9fooE"[..], b"_ZN4fooE", b"_ZN2E"] {
            assert_eq!(
                crate::rust::parse_bytes(symbol).unwrap_err(),
                ParseError::Invalid,
                "{symbol:?}"
            );
        }
        // The same bytes with a matching length parse.
        assert!(crate::rust::parse_bytes(b"_ZN3fooE").is_ok());
    }

    #[test]
    fn unescape_returns_borrowed_when_unchanged() {
        assert!(matches!(unescape(b"plain"), Cow::Borrowed(_)));
        assert!(matches!(unescape(b"a.b"), Cow::Borrowed(_)));
        assert!(matches!(unescape(b"a..b"), Cow::Owned(_)));
        assert!(matches!(unescape(b"$LT$"), Cow::Owned(_)));
        assert!(matches!(unescape(b"_$LT$"), Cow::Owned(_)));
    }

    /// Escapes decode to bytes, and non-UTF-8 bytes pass through decoding.
    #[test]
    fn unescape_keeps_raw_bytes() {
        assert_eq!(unescape(b"a\xffb").as_ref(), &b"a\xffb"[..]);
        assert_eq!(unescape(b"a\xff..b").as_ref(), &b"a\xff::b"[..]);
        assert_eq!(unescape(b"$u7b$\xff").as_ref(), &b"{\xff"[..]);
        assert_eq!(unescape(b"a$zz$\xff").as_ref(), &b"a$zz$\xff"[..]);
        let decoded: Vec<u8> = unescape(b"$u7b$$u7d$").into_owned();
        assert_eq!(decoded, b"{}");
    }
}
