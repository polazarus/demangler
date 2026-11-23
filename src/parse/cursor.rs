use core::fmt;

use super::{Fail, PResult, Span};

/// A checkpoint restores position and depth; consumed work is never refunded.
#[derive(Clone, Copy)]
pub(crate) struct Checkpoint {
    pos: usize,
    depth: u16,
}

/// Five-axis parser budget.
pub(crate) struct Limits {
    pub(crate) input: u32,
    pub(crate) depth: u16,
    pub(crate) work: u32,
    pub(crate) nodes: u32,
}

/// Mutable work/depth/node accounting for one parse.
pub(crate) struct State {
    pub(crate) limits: Limits,
    work: u32,
    depth: u16,
    nodes: u32,
}

impl State {
    pub(crate) const fn new(limits: Limits) -> Self {
        Self {
            limits,
            work: 0,
            depth: 0,
            nodes: 0,
        }
    }

    fn charge(&mut self, units: u32) -> PResult<()> {
        self.work = self.work.saturating_add(units);
        if self.work > self.limits.work {
            return Err(Fail::cut(self.work, b"work budget"));
        }
        Ok(())
    }

    // AST-building backends keep their own node counters; the shared hook is
    // retained so a `State`-driven backend can charge the same axis.
    #[allow(dead_code)]
    pub(crate) fn node(&mut self) -> PResult<()> {
        self.nodes = self.nodes.saturating_add(1);
        if self.nodes > self.limits.nodes {
            return Err(Fail::cut(self.nodes, b"node budget"));
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
            return Err(Fail::cut(0, b"input budget"));
        }
        state.charge(0)?;
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

    pub(crate) fn remaining(&self) -> &'a [u8] {
        self.bytes.get(self.pos..).unwrap_or(&[])
    }

    pub(crate) fn bytes_between(&self, start: usize, end: usize) -> Option<&'a [u8]> {
        self.bytes.get(start..end)
    }

    pub(crate) fn peek(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }

    pub(crate) fn peek_at(&self, offset: usize) -> Option<u8> {
        self.bytes.get(self.pos.saturating_add(offset)).copied()
    }

    pub(crate) fn starts_with(&self, tag: &[u8]) -> bool {
        self.bytes
            .get(self.pos..)
            .is_some_and(|rest| rest.starts_with(tag))
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
            .ok_or_else(|| Fail::cut(self.offset(), b"byte length overflow"))?;
        let bytes = self
            .bytes
            .get(self.pos..end)
            .ok_or_else(|| Fail::backtrack(self.offset(), b"input"))?;
        self.state.charge(u32::try_from(len).unwrap_or(u32::MAX))?;
        self.pos = end;
        Ok(bytes)
    }

    pub(crate) fn enter(&mut self) -> PResult<()> {
        self.state.charge(1)?;
        self.state.depth = self.state.depth.saturating_add(1);
        if self.state.depth > self.state.limits.depth {
            return Err(Fail::cut(self.offset(), b"depth budget"));
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
