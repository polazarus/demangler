//! Legacy (`_ZN...E`) front-end.
//!
//! The legacy encoding is a flat, length-prefixed path with no types: every
//! element is a name segment, and the trailing element is usually the rustc
//! hash. Parsing is iterative (no recursion, no backrefs) and only checks the
//! length arithmetic; `$`-escapes are decoded while building the node text.
//!
//! Reference: `refs/rust/rustc-demangle/src/legacy.rs` (decoder) and
//! `refs/rust/rustc/compiler/rustc_symbol_mangling/src/legacy.rs` (encoder).

use alloc::borrow::Cow;
use alloc::string::String;
use alloc::vec::Vec;

use super::ParseError;
use super::node::{Builder, Kind, NodeData, NodeRef, Span};

/// Length of a legacy mangling prefix, if `symbol` has one.
pub(super) fn prefix_len(symbol: &str) -> Option<usize> {
    if symbol.starts_with("_ZN") {
        Some(3)
    } else if symbol.starts_with("ZN") {
        Some(2)
    } else if symbol.starts_with("__ZN") {
        Some(4)
    } else {
        None
    }
}

/// Parse a legacy symbol, returning the path node and the end offset.
pub(super) fn parse(symbol: &str) -> Result<(NodeRef<'_>, usize), ParseError> {
    let prefix_len = prefix_len(symbol).ok_or(ParseError::Invalid)?;
    let inner = symbol.get(prefix_len..).ok_or(ParseError::Invalid)?;
    // The reference rejects non-ASCII input, which also keeps every byte
    // offset a character boundary.
    if inner.bytes().any(|byte| byte & 0x80 != 0) {
        return Err(ParseError::Invalid);
    }

    let mut elements = Vec::new();
    let mut next = 0;
    loop {
        match inner.as_bytes().get(next) {
            Some(b'E') => {
                next += 1;
                break;
            }
            Some(byte) if byte.is_ascii_digit() => {}
            // The path must terminate at `E` inside the symbol.
            _ => return Err(ParseError::Invalid),
        }

        let mut len = 0_usize;
        while let Some(digit @ b'0'..=b'9') = inner.as_bytes().get(next) {
            len = len
                .checked_mul(10)
                .and_then(|len| len.checked_add(usize::from(*digit - b'0')))
                .ok_or(ParseError::Invalid)?;
            next += 1;
        }

        let start = next;
        let end = start.checked_add(len).ok_or(ParseError::Invalid)?;
        let raw = inner.get(start..end).ok_or(ParseError::Invalid)?;
        next = end;
        elements.push((start, raw));
    }

    let mut builder = Builder::new();
    let mut children = Vec::with_capacity(elements.len());
    let count = elements.len();
    for (index, (start, raw)) in elements.into_iter().enumerate() {
        // Only the last element can be the trailing rustc hash.
        let is_hash = index + 1 == count && is_rust_hash(raw);
        let span = Span {
            start: position(prefix_len, start),
            end: position(prefix_len, start + raw.len()),
        };
        children.push(builder.node(
            Kind::LegacyElement,
            Some(unescape(raw)),
            Some(span),
            NodeData::Hash(is_hash),
            Vec::new(),
        )?);
    }
    let span = Span {
        start: position(prefix_len, 0),
        end: position(prefix_len, next),
    };
    let root = builder.node(Kind::LegacyPath, None, Some(span), NodeData::None, children)?;
    Ok((root, prefix_len.saturating_add(next)))
}

/// `offset` shifted into original-symbol coordinates.
fn position(base: usize, offset: usize) -> u32 {
    u32::try_from(base.saturating_add(offset)).unwrap_or(u32::MAX)
}

/// A rustc hash is `h` followed by hex digits.
fn is_rust_hash(raw: &str) -> bool {
    raw.strip_prefix('h')
        .is_some_and(|digits| digits.bytes().all(|byte| byte.is_ascii_hexdigit()))
}

/// Decode the `$`-escapes of one path element.
///
/// Borrowed when the element needs no rewriting, owned when it does. An
/// unrecognised escape ends decoding, and the rest is kept verbatim — exactly
/// like the reference printer.
fn unescape(element: &str) -> Cow<'_, str> {
    // Fast path: only `..` and `$` are rewritten.
    if !element.contains('$') && !element.contains("..") {
        return Cow::Borrowed(element);
    }

    let mut out = String::with_capacity(element.len());
    let mut rest = element;
    // A leading `_$` keeps the escape but drops the `_`.
    if rest.starts_with("_$") {
        rest = rest.get(1..).unwrap_or(rest);
    }

    loop {
        if let Some(tail) = rest.strip_prefix('.') {
            if let Some(tail) = tail.strip_prefix('.') {
                out.push_str("::");
                rest = tail;
            } else {
                out.push('.');
                rest = tail;
            }
            continue;
        }

        let Some(tail) = rest.strip_prefix('$') else {
            match rest.find(['$', '.']) {
                Some(index) => {
                    out.push_str(rest.get(..index).unwrap_or(""));
                    rest = rest.get(index..).unwrap_or("");
                }
                None => break,
            }
            continue;
        };

        let Some(end) = tail.find('$') else {
            break;
        };
        let escape = tail.get(..end).unwrap_or("");
        let after = tail.get(end + 1..).unwrap_or("");
        let Some(unescaped) = unescape_token(escape) else {
            break;
        };
        match unescaped {
            Unescaped::Text(text) => out.push_str(text),
            Unescaped::Char(c) => out.push(c),
        }
        rest = after;
    }

    out.push_str(rest);
    Cow::Owned(out)
}

/// The decoded form of a `$...$` escape.
enum Unescaped<'a> {
    /// A fixed replacement such as `$LT$` → `<`.
    Text(&'a str),
    /// A `$u<hex>$` scalar value.
    Char(char),
}

/// Decode one escape, or `None` when it is unknown.
fn unescape_token(escape: &str) -> Option<Unescaped<'_>> {
    let text = match escape {
        "SP" => "@",
        "BP" => "*",
        "RF" => "&",
        "LT" => "<",
        "GT" => ">",
        "LP" => "(",
        "RP" => ")",
        "C" => ",",
        _ => {
            let digits = escape.strip_prefix('u')?;
            let all_lower_hex = !digits.is_empty()
                && digits
                    .bytes()
                    .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'));
            if !all_lower_hex {
                return None;
            }
            let value = u32::from_str_radix(digits, 16).ok()?;
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
        assert_eq!(element.text(), Some("foo"));

        let ast = parse("_ZN8foo..barE").unwrap();
        let element = ast.root().children().first().unwrap();
        assert_eq!(element.text(), Some("foo::bar"));
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
            "_ZN4t\u{e9}stE",
            "_ZN3f\u{e9}oE",
        ] {
            assert_eq!(parse(symbol).unwrap_err(), ParseError::Invalid, "{symbol}");
        }
    }

    #[test]
    fn unescape_returns_borrowed_when_unchanged() {
        assert!(matches!(unescape("plain"), Cow::Borrowed(_)));
        assert!(matches!(unescape("a.b"), Cow::Borrowed(_)));
        assert!(matches!(unescape("a..b"), Cow::Owned(_)));
        assert!(matches!(unescape("$LT$"), Cow::Owned(_)));
        assert!(matches!(unescape("_$LT$"), Cow::Owned(_)));
    }
}
