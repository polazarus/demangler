/// Whether an alternative may backtrack or has committed to a branch.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum FailMode {
    Backtrack,
    Cut,
}

/// A bounded parse failure.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct Fail {
    pub(crate) mode: FailMode,
    pub(crate) offset: u32,
    pub(crate) expected: &'static [u8],
}

impl Fail {
    pub(crate) const fn backtrack(offset: u32, expected: &'static [u8]) -> Self {
        Self {
            mode: FailMode::Backtrack,
            offset,
            expected,
        }
    }

    pub(crate) const fn cut(offset: u32, expected: &'static [u8]) -> Self {
        Self {
            mode: FailMode::Cut,
            offset,
            expected,
        }
    }
}
