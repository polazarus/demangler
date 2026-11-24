use core::fmt;

use super::{Fail, PResult, Span};

/// A checkpoint restores position and depth; consumed work is never refunded.
#[derive(Clone, Copy)]
pub(crate) struct Checkpoint {
    pos: usize,
    depth: u16,
}

/// Parser budgets shared by the byte cursor.
///
/// Node allocation is owned by the AST-building backend (D, Rust, Swift and
/// the C++ parsers keep their own counters); the shared cursor bounds the
/// input, nesting and work axes.
pub(crate) struct Limits {
    pub(crate) input: u32,
    pub(crate) depth: u16,
    pub(crate) work: u32,
}

/// Mutable work/depth accounting for one parse.
pub(crate) struct State {
    pub(crate) limits: Limits,
    work: u32,
    depth: u16,
}

impl State {
    pub(crate) const fn new(limits: Limits) -> Self {
        Self {
            limits,
            work: 0,
            depth: 0,
        }
    }

    fn charge(&mut self, units: u32, offset: u32) -> PResult<()> {
        self.work = self.work.saturating_add(units);
        if self.work > self.limits.work {
            return Err(Fail::work_budget(offset));
        }
        Ok(())
    }
}

/// A bounded byte cursor over one input slice.
pub(crate) struct Cursor<'a, 's> {
    bytes: &'a [u8],
    pos: usize,
    base: u32,
    state: &'s mut State,
}

impl<'a, 's> Cursor<'a, 's> {
    pub(crate) fn new(bytes: &'a [u8], base: u32, state: &'s mut State) -> PResult<Self> {
        if bytes.len() > state.limits.input as usize {
            return Err(Fail::input_budget(0));
        }
        state.charge(0, 0)?;
        Ok(Self {
            bytes,
            pos: 0,
            base,
            state,
        })
    }

    pub(crate) const fn pos(&self) -> usize {
        self.pos
    }

    /// Total input length, independent of the cursor position.
    pub(crate) fn input_len(&self) -> usize {
        self.bytes.len()
    }

    pub(crate) fn remaining(&self) -> &'a [u8] {
        self.bytes.get(self.pos..).unwrap_or(&[])
    }

    pub(crate) fn bytes_between(&self, start: usize, end: usize) -> Option<&'a [u8]> {
        self.bytes.get(start..end)
    }

    /// The byte at the absolute position `pos`.
    ///
    /// A back-reference re-scan resolves a target behind the cursor, so it
    /// reads at an absolute position rather than through [`Self::peek`].
    pub(crate) fn byte_at(&self, pos: usize) -> Option<u8> {
        self.bytes.get(pos).copied()
    }

    pub(crate) fn peek(&self) -> Option<u8> {
        self.byte_at(self.pos)
    }

    pub(crate) fn peek_at(&self, offset: usize) -> Option<u8> {
        self.byte_at(self.pos.saturating_add(offset))
    }

    pub(crate) fn starts_with(&self, tag: &[u8]) -> bool {
        self.starts_with_at(self.pos, tag)
    }

    /// True when the input at the absolute position `pos` starts with `tag`.
    ///
    /// A back-reference re-scan inspects bytes behind the cursor, so it reads at
    /// an absolute position rather than through [`Self::starts_with`].
    pub(crate) fn starts_with_at(&self, pos: usize, tag: &[u8]) -> bool {
        self.bytes
            .get(pos..)
            .is_some_and(|rest| rest.starts_with(tag))
    }

    /// Charge `units` of work without moving the cursor.
    ///
    /// A lookahead scan that reads a slice without consuming it meters its
    /// bytes here instead of by advancing over them.
    pub(crate) fn charge(&mut self, units: usize) -> PResult<()> {
        let units = u32::try_from(units).unwrap_or(u32::MAX);
        self.state.charge(units, self.offset())
    }

    /// Move to the absolute position `pos` without charging work.
    ///
    /// Back references re-scan an earlier part of the input, so a parser has to
    /// jump backwards; the scan that follows is what the work budget meters.
    /// A forward jump only skips a length the caller has already read and
    /// metered itself.
    pub(crate) fn seek(&mut self, pos: usize) -> PResult<()> {
        if pos > self.bytes.len() {
            return Err(Fail::backtrack(self.offset()));
        }
        self.pos = pos;
        Ok(())
    }

    pub(crate) fn checkpoint(&self) -> Checkpoint {
        Checkpoint {
            pos: self.pos,
            depth: self.state.depth,
        }
    }

    pub(crate) fn reset(&mut self, checkpoint: Checkpoint) {
        self.pos = checkpoint.pos;
        self.state.depth = checkpoint.depth;
    }

    pub(crate) fn advance(&mut self, len: usize) -> PResult<&'a [u8]> {
        let end = self
            .pos
            .checked_add(len)
            .ok_or_else(|| Fail::length_overflow(self.offset()))?;
        let bytes = self
            .bytes
            .get(self.pos..end)
            .ok_or_else(|| Fail::end_of_input(self.offset()))?;
        self.state
            .charge(u32::try_from(len).unwrap_or(u32::MAX), self.offset())?;
        self.pos = end;
        Ok(bytes)
    }

    pub(crate) fn enter(&mut self) -> PResult<()> {
        self.state.charge(1, self.offset())?;
        self.state.depth = self.state.depth.saturating_add(1);
        if self.state.depth > self.state.limits.depth {
            return Err(Fail::depth_budget(self.offset()));
        }
        Ok(())
    }

    pub(crate) fn leave(&mut self) {
        self.state.depth = self.state.depth.saturating_sub(1);
    }

    pub(crate) fn span(&self, from: Checkpoint) -> Span {
        Span::new(
            self.base
                .saturating_add(u32::try_from(from.pos).unwrap_or(u32::MAX)),
            self.base
                .saturating_add(u32::try_from(self.pos).unwrap_or(u32::MAX)),
        )
    }

    pub(crate) fn offset(&self) -> u32 {
        self.base
            .saturating_add(u32::try_from(self.pos).unwrap_or(u32::MAX))
    }
}

impl fmt::Debug for Cursor<'_, '_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Cursor")
            .field("pos", &self.pos)
            .field("remaining", &self.remaining().len())
            .finish()
    }
}
