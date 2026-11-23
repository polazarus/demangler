use super::{
    Checkpoint, Cursor, Fail, FailMode, Ident, Parser, Span,
};

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
pub(crate) fn take<'a>(len: usize) -> impl Parser<'a, Output = &'a [u8]> {
    move |input: &mut Cursor<'a, '_>| input.advance(len)
}

/// Take bytes while `predicate` holds.
pub(crate) fn take_while<'a>(
    predicate: fn(u8) -> bool,
) -> impl Parser<'a, Output = &'a [u8]> {
    move |input: &mut Cursor<'a, '_>| {
        let start = input.pos();
        while input.peek().is_some_and(predicate) {
            let _ = input.advance(1)?;
        }
        input
            .remaining()
            .get(..0)
            .and_then(|_| input_bytes(input, start, input.pos()))
            .ok_or_else(|| Fail::backtrack(input.offset(), b"take_while"))
    }
}

fn input_bytes<'a>(
    input: &Cursor<'a, '_>,
    start: usize,
    end: usize,
) -> Option<&'a [u8]> {
    // The cursor's `remaining` starts at `pos`; reconstruct through its
    // public byte slice only in this prototype helper.
    input.bytes_between(start, end)
}

/// Try `first`, then `second` on a backtracking failure.
pub(crate) fn alt<'a, O, A, B>(
    mut first: A,
    mut second: B,
) -> impl Parser<'a, Output = O>
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

/// Optional parser.
pub(crate) fn opt<'a, O, P>(mut parser: P) -> impl Parser<'a, Output = Option<O>>
where
    P: Parser<'a, Output = O>,
{
    move |input: &mut Cursor<'a, '_>| {
        let checkpoint = input.checkpoint();
        match parser.parse(input) {
            Ok(value) => Ok(Some(value)),
            Err(fail) if fail.mode == FailMode::Backtrack => {
                input.reset(checkpoint);
                Ok(None)
            }
            Err(fail) => Err(fail),
        }
    }
}

/// One or more repetitions.
#[cfg(any(
    feature = "cpp-itanium",
    feature = "cpp-msvc",
    feature = "cpp-gnu-v2",
    feature = "rust",
    feature = "swift",
    feature = "go",
    feature = "objc",
    feature = "kotlin-native"
))]
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

/// Parse `open`, then `parser`.
pub(crate) fn preceded<'a, O, P, Q>(
    mut open: P,
    mut parser: Q,
) -> impl Parser<'a, Output = O>
where
    P: Parser<'a>,
    Q: Parser<'a, Output = O>,
{
    move |input: &mut Cursor<'a, '_>| {
        let _ = open.parse(input)?;
        parser.parse(input)
    }
}

/// Parse `parser`, then `close`.
pub(crate) fn delimited<'a, O, P, Q, R>(
    mut open: P,
    mut parser: Q,
    mut close: R,
) -> impl Parser<'a, Output = O>
where
    P: Parser<'a>,
    Q: Parser<'a, Output = O>,
    R: Parser<'a>,
{
    move |input: &mut Cursor<'a, '_>| {
        let _ = open.parse(input)?;
        let value = parser.parse(input)?;
        let _ = close.parse(input)?;
        Ok(value)
    }
}

/// Look ahead without consuming input.
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
pub(crate) fn ident<'a>() -> impl Parser<'a, Output = Ident<'a>> {
    move |input: &mut Cursor<'a, '_>| {
        let len = decimal_length().parse(input)?;
        let bytes_start = input.checkpoint();
        let bytes = input.advance(len)?;
        Ok(Ident::new(bytes, input.span(bytes_start)))
    }
}
