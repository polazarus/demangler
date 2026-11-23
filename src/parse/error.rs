/// Whether an alternative may backtrack or has committed to a branch.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum FailMode {
    Backtrack,
    Cut,
}

/// The budget axis a [`Fail`] reports.
///
/// The shared cursor labels a failure with the byte string of the budget it
/// exhausted. This classifier is the single owner of that label matching;
/// backends only decide which of their public error variants a kind maps to.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum FailKind {
    /// The grammar did not match, or a non-budget bound tripped.
    Invalid,
    /// The nesting depth budget was exhausted.
    Depth,
    /// The input, work or node budget was exhausted.
    Budget,
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

    /// Classify this failure by the budget it reports.
    pub(crate) fn kind(self) -> FailKind {
        match self.expected {
            b"depth budget" => FailKind::Depth,
            b"input budget" | b"work budget" | b"node budget" => FailKind::Budget,
            _ => FailKind::Invalid,
        }
    }
}
