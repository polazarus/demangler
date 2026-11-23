use super::{Cursor, Fail};

/// Result of one parser step.
pub(crate) type PResult<T> = Result<T, Fail>;

/// A parser over a bounded byte cursor.
pub(crate) trait Parser<'a> {
    type Output;

    fn parse<'s>(&mut self, input: &mut Cursor<'a, 's>) -> PResult<Self::Output>;
}

impl<'a, F, O> Parser<'a> for F
where
    F: for<'s> FnMut(&mut Cursor<'a, 's>) -> PResult<O>,
{
    type Output = O;

    fn parse<'s>(&mut self, input: &mut Cursor<'a, 's>) -> PResult<O> {
        self(input)
    }
}
