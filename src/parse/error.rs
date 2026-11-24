/// Whether an alternative may backtrack or has committed to a branch.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum FailMode {
    Backtrack,
    Cut,
}

/// The failure axis a [`Fail`] reports.
///
/// A failure carries its kind as data: the constructors below name the bound
/// they tripped, so a backend maps the kind to its own public error variant
/// and never matches on a diagnostic label. `expected` stays a message for
/// humans and tests.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum FailKind {
    /// The grammar did not match.
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
}

/// A bounded parse failure.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct Fail {
    pub(crate) mode: FailMode,
    pub(crate) kind: FailKind,
    pub(crate) offset: u32,
    pub(crate) expected: &'static [u8],
}

impl Fail {
    const fn new(mode: FailMode, kind: FailKind, offset: u32, expected: &'static [u8]) -> Self {
        Self {
            mode,
            kind,
            offset,
            expected,
        }
    }

    /// A grammar failure that a sibling production may retry.
    pub(crate) const fn backtrack(offset: u32, expected: &'static [u8]) -> Self {
        Self::new(FailMode::Backtrack, FailKind::Invalid, offset, expected)
    }

    /// A grammar failure that commits the branch.
    pub(crate) const fn cut(offset: u32, expected: &'static [u8]) -> Self {
        Self::new(FailMode::Cut, FailKind::Invalid, offset, expected)
    }

    /// The input ended inside a production.
    pub(crate) const fn end_of_input(offset: u32) -> Self {
        Self::new(FailMode::Backtrack, FailKind::EndOfInput, offset, b"input")
    }

    /// The input-length budget was exhausted.
    pub(crate) const fn input_budget(offset: u32) -> Self {
        Self::new(FailMode::Cut, FailKind::InputBudget, offset, b"input budget")
    }

    /// A byte length overflowed the addressable input.
    pub(crate) const fn length_overflow(offset: u32) -> Self {
        Self::new(
            FailMode::Cut,
            FailKind::LengthOverflow,
            offset,
            b"byte length overflow",
        )
    }

    /// The nesting depth budget was exhausted.
    pub(crate) const fn depth_budget(offset: u32) -> Self {
        Self::new(FailMode::Cut, FailKind::Depth, offset, b"depth budget")
    }

    /// The work budget was exhausted.
    pub(crate) const fn work_budget(offset: u32) -> Self {
        Self::new(FailMode::Cut, FailKind::Work, offset, b"work budget")
    }

    /// The same failure, committed to the current branch.
    pub(crate) const fn committed(self) -> Self {
        Self::new(FailMode::Cut, self.kind, self.offset, self.expected)
    }
}
