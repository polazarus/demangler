//! Byte-level combinators shared by the byte-native backends.
//!
//! A combinator is a closure over [`Cursor`]. Failure carries a [`FailMode`]:
//! [`many0`] keeps going on `Backtrack` and stops on `Cut`, so a parser commits
//! only once it has consumed bytes that cannot be reinterpreted by a sibling
//! production.
//!
//! Grammars live in the backends; this module keeps only the primitives that a
//! backend needs and the failure vocabulary that `src/parse/tests.rs` pins.

#[cfg(feature = "rust")]
use super::FailMode;
use super::{Cursor, Fail, Parser, Span};

/// Match a literal byte tag and return its span.
pub fn tag<'a>(literal: &'static [u8]) -> impl Parser<'a, Output = Span> {
    move |input: &mut Cursor<'a, '_>| {
        let checkpoint = input.checkpoint();
        if !input.starts_with(literal) {
            // Input that ends inside the literal ran out rather than mismatched.
            if literal.starts_with(input.remaining()) {
                return Err(Fail::end_of_input(input.offset()));
            }
            return Err(Fail::backtrack(input.offset()));
        }
        let _ = input.advance(literal.len())?;
        Ok(input.span(checkpoint))
    }
}

/// Zero or more repetitions.
#[cfg(feature = "rust")]
pub fn many0<'a, O, P>(mut parser: P) -> impl Parser<'a, Output = alloc::vec::Vec<O>>
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

/// Parse a length-prefixed identifier using a decimal length.
pub fn decimal_length<'a>() -> impl Parser<'a, Output = usize> {
    move |input: &mut Cursor<'a, '_>| {
        let mut value = 0_usize;
        let mut any = false;
        while let Some(byte @ b'0'..=b'9') = input.peek() {
            value = value
                .checked_mul(10)
                .and_then(|v| v.checked_add(usize::from(byte - b'0')))
                .ok_or_else(|| Fail::cut(input.offset()))?;
            let _ = input.advance(1)?;
            any = true;
        }
        if !any {
            return Err(Fail::backtrack(input.offset()));
        }
        Ok(value)
    }
}
