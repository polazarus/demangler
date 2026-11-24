//! Bounded parser for Java JNI native-method names.
//!
//! The grammar is the JNI escaping rule from the Java Native Interface
//! specification, documented in `refs/jni.md`:
//!
//! ```text
//! name        ::= "Java_" class "_" method ( "__" descriptor )?
//! class       ::= encoded ( "_" encoded )*        (package and class parts)
//! encoded     ::= literal | "_1" | "_2" | "_3" | "_0" hex4
//! descriptor  ::= "(" field* ")" return
//! field       ::= "_3"* ( base | "L" class "_2" )
//! return      ::= field | "V"
//! ```
//!
//! A bare `_` is the only separator, so the class/method boundary is the last
//! one before the descriptor delimiter, and a `__` pair is the delimiter
//! itself. Escape sequences are consumed as units, so an escape that follows a
//! separator (`p_0d835_0dd38`) never looks like a delimiter.
//!
//! The grammar is matched on bytes through the shared [`Cursor`]: a symbol name
//! is a byte string, so the parser never requires the input to be UTF-8. Every
//! escape is decoded while parsing, so validation and rendering cannot disagree.

use alloc::borrow::Cow;
use alloc::vec::Vec;

use super::ParseError;
use super::node::{Ast, Descriptor, Element, MAX_DEPTH, MAX_INPUT_LEN, MAX_WORK, Primitive, Type};
use crate::parse::{Cursor, Fail, FailKind, Limits, State};

/// The prefix every JNI native-method name starts with.
const PREFIX: &[u8] = b"Java_";

/// Parse a JNI native-method name from raw bytes.
pub(crate) fn parse_bytes(symbol: &[u8]) -> Result<Ast<'_>, ParseError> {
    let body = symbol.strip_prefix(PREFIX).ok_or(ParseError::Invalid)?;

    let mut state = State::new(limits());
    let mut cursor = Cursor::new(symbol, 0, &mut state).map_err(map_fail)?;
    charge(&mut cursor, body.len())?;

    let body = split_body(body)?;
    let class = decode_name(body.class, '.')?;
    let method = decode_name(body.method, '.')?;
    let descriptor = match body.signature {
        Some(signature) => {
            charge(&mut cursor, signature.len())?;
            Some(parse_descriptor(signature)?)
        }
        None => None,
    };
    Ok(Ast {
        class,
        method,
        descriptor,
    })
}

/// The budgets the shared cursor enforces.
///
/// The parser has no node budget: one name holds one argument list, bounded by
/// the input length.
fn limits() -> Limits {
    Limits {
        input: MAX_INPUT_LEN as u32,
        depth: MAX_DEPTH as u16,
        work: MAX_WORK as u32,
    }
}

/// Translate a shared-cursor failure into this backend's public error.
fn map_fail(fail: Fail) -> ParseError {
    match fail.kind {
        FailKind::InputBudget | FailKind::Work => ParseError::BudgetExceeded,
        _ => ParseError::Invalid,
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

/// One element of an encoded name.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Token {
    /// A byte the mangling leaves unescaped.
    Literal(u8),
    /// A bare `_`: a binary-name separator, or `/` inside a descriptor name.
    Separator,
    /// `_1` or `_0xxxx`: an escaped character.
    Escaped(char),
    /// `_2`: `;`, the end of an object type.
    Semicolon,
    /// `_3`: `[`, one array dimension.
    Bracket,
    /// A truncated, malformed or unpaired-surrogate escape.
    Invalid,
}

/// Classify the element at the start of `bytes`, returning it with the number
/// of bytes it occupies.
fn token(bytes: &[u8]) -> (Token, usize) {
    let Some(&first) = bytes.first() else {
        return (Token::Invalid, 0);
    };
    if first != b'_' {
        return (Token::Literal(first), 1);
    }
    match bytes.get(1) {
        // A trailing `_` is a separator: `Java_foo_bar__` ends with an empty
        // signature, so the second `_` of the delimiter is the last byte.
        None => (Token::Separator, 1),
        Some(b'1') => (Token::Escaped('_'), 2),
        Some(b'2') => (Token::Semicolon, 2),
        Some(b'3') => (Token::Bracket, 2),
        Some(b'0') => escaped_scalar(bytes).map_or((Token::Invalid, 1), |(scalar, length)| {
            (Token::Escaped(scalar), length)
        }),
        // A separator is never followed by a digit: the escape set covers `_0`
        // to `_3`, and no Java identifier starts with a digit, so `_4` to `_9`
        // are malformed escapes rather than separator-plus-name.
        Some(byte) if byte.is_ascii_digit() => (Token::Invalid, 1),
        Some(_) => (Token::Separator, 1),
    }
}

/// Decode the `_0xxxx` escape at the start of `bytes`, pairing surrogates.
///
/// A high surrogate must be followed by a low-surrogate escape; a lone low
/// surrogate is rejected. The returned length covers both halves of a pair.
fn escaped_scalar(bytes: &[u8]) -> Option<(char, usize)> {
    let high = hex4(bytes.get(2..6))?;
    if (0xD800..=0xDBFF).contains(&high) {
        let low = hex4(bytes.get(8..12))?;
        if bytes.get(6) != Some(&b'_') || bytes.get(7) != Some(&b'0') {
            return None;
        }
        if !(0xDC00..=0xDFFF).contains(&low) {
            return None;
        }
        let scalar = 0x1_0000 + ((high - 0xD800) << 10) + (low - 0xDC00);
        return char::from_u32(scalar).map(|scalar| (scalar, 12));
    }
    char::from_u32(high).map(|scalar| (scalar, 6))
}

/// Read exactly four hexadecimal digits, in either case.
fn hex4(bytes: Option<&[u8]>) -> Option<u32> {
    let bytes = bytes?;
    let mut value = 0;
    for &byte in bytes {
        value = value * 16 + char::from(byte).to_digit(16)?;
    }
    Some(value)
}

/// The three parts of an encoded body.
struct Body<'a> {
    /// The encoded class/package part, before the method separator.
    class: &'a [u8],
    /// The encoded method name.
    method: &'a [u8],
    /// The encoded argument list of a long-form name.
    signature: Option<&'a [u8]>,
}

/// Split the encoded body into the class part, the method and the signature.
///
/// The delimiter is the first pair of adjacent bare separators; escapes are
/// consumed as units, so `p_0d835_0dd38bc__` splits after the method rather
/// than inside it.
fn split_body(body: &[u8]) -> Result<Body<'_>, ParseError> {
    let mut position = 0;
    let mut previous_separator = None;
    let mut last_separator = None;
    let mut delimiter = None;
    let mut separator = false;
    while position < body.len() {
        let (token, length) = token(&body[position..]);
        match token {
            Token::Separator => {
                if separator {
                    delimiter = Some(position - 1);
                    break;
                }
                separator = true;
                previous_separator = last_separator;
                last_separator = Some(position);
            }
            Token::Literal(byte) if byte.is_ascii_alphanumeric() => separator = false,
            Token::Escaped(_) => separator = false,
            _ => return Err(ParseError::Invalid),
        }
        position += length;
    }

    let end = delimiter.unwrap_or(body.len());
    // The delimiter's first `_` is the last separator seen, so the method
    // separator is the one before it.
    let separator = match delimiter {
        Some(_) => previous_separator,
        None => last_separator,
    }
    .ok_or(ParseError::Invalid)?;
    let class = &body[..separator];
    let method = &body[separator + 1..end];
    if class.is_empty() || method.is_empty() {
        return Err(ParseError::Invalid);
    }
    Ok(Body {
        class,
        method,
        signature: delimiter.map(|at| &body[at + 2..]),
    })
}

/// Decode an encoded name, mapping a bare `_` to `separator`.
fn decode_name(bytes: &[u8], separator: char) -> Result<Cow<'_, str>, ParseError> {
    if !bytes.contains(&b'_') {
        // Without escapes every byte is a literal alphanumeric, so the encoded
        // bytes are already the spelling.
        if !bytes.iter().all(u8::is_ascii_alphanumeric) {
            return Err(ParseError::Invalid);
        }
        return Ok(Cow::Borrowed(core::str::from_utf8(bytes).unwrap_or_default()));
    }
    let mut text = alloc::string::String::with_capacity(bytes.len());
    let mut position = 0;
    while position < bytes.len() {
        let (token, length) = token(&bytes[position..]);
        match token {
            Token::Literal(byte) if byte.is_ascii_alphanumeric() => text.push(char::from(byte)),
            Token::Separator => text.push(separator),
            Token::Escaped(scalar) => text.push(scalar),
            // `;` and `[` cannot appear in an unqualified name, and an invalid
            // escape is rejected rather than guessed.
            _ => return Err(ParseError::Invalid),
        }
        position += length;
    }
    Ok(Cow::Owned(text))
}

/// Parse the mangled argument list of a long-form name.
///
/// The mangled form is the argument types with the descriptor's parentheses
/// dropped, so `([Ljava/lang/String;I)` becomes `_3Ljava_lang_String_2I`. An
/// empty signature is the no-argument overload.
fn parse_descriptor(signature: &[u8]) -> Result<Descriptor<'_>, ParseError> {
    let mut cursor = Signature {
        bytes: signature,
        position: 0,
    };
    let mut args = Vec::new();
    while cursor.position < signature.len() {
        args.push(cursor.field_type()?);
    }
    Ok(Descriptor { args })
}

/// A position in a mangled descriptor.
struct Signature<'a> {
    bytes: &'a [u8],
    position: usize,
}

impl<'a> Signature<'a> {
    /// The token at the current position, without consuming it.
    fn peek(&self) -> Option<(Token, usize)> {
        if self.position >= self.bytes.len() {
            return None;
        }
        Some(token(&self.bytes[self.position..]))
    }

    /// Parse one field type: `_3`* followed by a base or object type.
    fn field_type(&mut self) -> Result<Type<'a>, ParseError> {
        let mut dimensions = 0_u8;
        while self.peek() == Some((Token::Bracket, 2)) {
            dimensions = dimensions.saturating_add(1);
            if usize::from(dimensions) > MAX_DEPTH {
                return Err(ParseError::RecursedTooDeep);
            }
            self.position += 2;
        }
        let element = match self.peek() {
            Some((Token::Literal(b'L'), 1)) => {
                self.position += 1;
                Element::Object(self.object_name()?)
            }
            Some((Token::Literal(byte), 1)) => {
                let primitive = Primitive::from_descriptor(byte).ok_or(ParseError::Invalid)?;
                self.position += 1;
                Element::Primitive(primitive)
            }
            _ => return Err(ParseError::Invalid),
        };
        Ok(Type {
            dimensions,
            element,
        })
    }

    /// Parse an object type's binary name up to and including its `_2`.
    fn object_name(&mut self) -> Result<Cow<'a, str>, ParseError> {
        let start = self.position;
        let end = loop {
            match self.peek() {
                Some((Token::Semicolon, 2)) => break self.position,
                Some((Token::Literal(byte), 1)) if byte.is_ascii_alphanumeric() => {
                    self.position += 1;
                }
                Some((Token::Separator, 1)) => self.position += 1,
                Some((Token::Escaped(_), length)) => self.position += length,
                _ => return Err(ParseError::Invalid),
            }
        };
        self.position = end + 2;
        let name = decode_name(&self.bytes[start..end], '.')?;
        // A binary name has no empty component: `L_2` or `La__b_2` are
        // malformed rather than an empty package or class.
        if name.is_empty() || name.split('.').any(str::is_empty) {
            return Err(ParseError::Invalid);
        }
        Ok(name)
    }
}
