//! Byte-level combinators shared by the byte-native backends.
//!
//! A combinator is a closure over [`Cursor`]. Failure carries a [`FailMode`]:
//! [`alt`] retries on `Backtrack` and stops on `Cut`, so a parser commits only
//! once it has consumed bytes that cannot be reinterpreted by a sibling
//! production.
//!
//! Grammars live in the backends; this module keeps only the primitives that
//! a backend needs and the failure vocabulary that `src/parse/tests.rs` pins.

use super::{Checkpoint, Cursor, Fail, FailMode, Ident, Parser, Span};

/// Match a literal byte tag and return its span.
pub(crate) fn tag<'a>(literal: &'static [u8]) -> impl Parser<'a, Output = Span> {
    move |input: &mut Cursor<'a, '_>| {
        let checkpoint = input.checkpoint();
        if !input.starts_with(literal) {
            return Err(Fail::backtrack(input.offset(), literal));
        }
        let _ = input.advance(literal.len())?;
        Ok(input.span(checkpoint))
    }
}

/// Take exactly `len` bytes.
// The combinators below are the shared vocabulary that `src/parse/tests.rs`
// exercises to pin backtracking, cut, EOF and lookahead semantics. No backend
// calls them directly yet, so the library build needs this narrow allowance;
// the tests keep the semantics from rotting.
#[allow(dead_code)]
pub(crate) fn take<'a>(len: usize) -> impl Parser<'a, Output = &'a [u8]> {
    move |input: &mut Cursor<'a, '_>| input.advance(len)
}

/// Try `first`, then `second` on a backtracking failure.
#[allow(dead_code)]
pub(crate) fn alt<'a, O, A, B>(mut first: A, mut second: B) -> impl Parser<'a, Output = O>
where
    A: Parser<'a, Output = O>,
    B: Parser<'a, Output = O>,
{
    move |input: &mut Cursor<'a, '_>| {
        let checkpoint: Checkpoint = input.checkpoint();
        match first.parse(input) {
            Ok(value) => Ok(value),
            Err(fail) if fail.mode == FailMode::Backtrack => {
                input.reset(checkpoint);
                second.parse(input)
            }
            Err(fail) => Err(fail),
        }
    }
}

/// One or more repetitions.
#[cfg(feature = "rust")]
pub(crate) fn many0<'a, O, P>(mut parser: P) -> impl Parser<'a, Output = alloc::vec::Vec<O>>
where
    P: Parser<'a, Output = O>,
{
    move |input: &mut Cursor<'a, '_>| {
        let mut values = alloc::vec::Vec::new();
        loop {
            let checkpoint = input.checkpoint();
            match parser.parse(input) {
                Ok(value) => values.push(value),
                Err(fail) if fail.mode == FailMode::Backtrack => {
                    input.reset(checkpoint);
                    break;
                }
                Err(fail) => return Err(fail),
            }
        }
        Ok(values)
    }
}

/// Look ahead without consuming input.
#[allow(dead_code)]
pub(crate) fn peek<'a, O, P>(mut parser: P) -> impl Parser<'a, Output = O>
where
    P: Parser<'a, Output = O>,
{
    move |input: &mut Cursor<'a, '_>| {
        let checkpoint = input.checkpoint();
        let value = parser.parse(input);
        input.reset(checkpoint);
        value
    }
}

/// Match only at end of input.
#[allow(dead_code)]
pub(crate) fn eof<'a>() -> impl Parser<'a, Output = ()> {
    |input: &mut Cursor<'a, '_>| {
        if input.peek().is_none() {
            Ok(())
        } else {
            Err(Fail::backtrack(input.offset(), b"eof"))
        }
    }
}

/// Convert a backtracking failure into a committed failure.
#[allow(dead_code)]
pub(crate) fn cut<'a, O, P>(mut parser: P) -> impl Parser<'a, Output = O>
where
    P: Parser<'a, Output = O>,
{
    move |input: &mut Cursor<'a, '_>| match parser.parse(input) {
        Ok(value) => Ok(value),
        Err(fail) => Err(Fail::cut(fail.offset, fail.expected)),
    }
}

/// Parse a length-prefixed identifier using a decimal length.
pub(crate) fn decimal_length<'a>() -> impl Parser<'a, Output = usize> {
    move |input: &mut Cursor<'a, '_>| {
        let mut value = 0_usize;
        let mut any = false;
        while let Some(byte @ b'0'..=b'9') = input.peek() {
            value = value
                .checked_mul(10)
                .and_then(|v| v.checked_add(usize::from(byte - b'0')))
                .ok_or_else(|| Fail::cut(input.offset(), b"decimal overflow"))?;
            let _ = input.advance(1)?;
            any = true;
        }
        if !any {
            return Err(Fail::backtrack(input.offset(), b"decimal"));
        }
        Ok(value)
    }
}

/// Parse `len` bytes as an `Ident`.
#[allow(dead_code)]
pub(crate) fn ident<'a>() -> impl Parser<'a, Output = Ident<'a>> {
    move |input: &mut Cursor<'a, '_>| {
        let len = decimal_length().parse(input)?;
        let bytes_start = input.checkpoint();
        let bytes = input.advance(len)?;
        Ok(Ident::new(bytes, input.span(bytes_start)))
    }
}
