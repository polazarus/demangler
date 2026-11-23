/// Whether an alternative may backtrack or has committed to a branch.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum FailMode {
    Backtrack,
    Cut,
}

/// The failure axis a [`Fail`] reports.
///
/// The shared cursor labels a failure with the byte string of the bound it
/// tripped. This classifier is the single owner of that label matching;
/// backends only decide which of their public error variants a kind maps to.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum FailKind {
    /// The grammar did not match, or a non-budget bound tripped.
    Invalid,
    /// The input ended while a production still needed bytes.
    EndOfInput,
    /// The input-length budget was exhausted.
    InputBudget,
    /// A byte length overflowed the addressable input.
    LengthOverflow,
    /// The nesting depth budget was exhausted.
    Depth,
    /// The work budget was exhausted.
    Work,
    /// The node budget was exhausted.
    Node,
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
            b"input" => FailKind::EndOfInput,
            b"input budget" => FailKind::InputBudget,
            b"byte length overflow" => FailKind::LengthOverflow,
            b"depth budget" => FailKind::Depth,
            b"work budget" => FailKind::Work,
            b"node budget" => FailKind::Node,
            _ => FailKind::Invalid,
        }
    }
}
