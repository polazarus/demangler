/// A byte range in the original, undecorated symbol.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub struct Span {
    pub(crate) start: u32,
    pub(crate) end: u32,
}

impl Span {
    pub(crate) const fn new(start: u32, end: u32) -> Self {
        Self { start, end }
    }
}
