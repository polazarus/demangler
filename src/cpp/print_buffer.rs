//! Output buffer shared by the C++ printers.
//!
//! The Itanium printer ([`super::printer`]) and the legacy printer
//! ([`super::legacy`]) write their text through the same buffer. Printing is
//! not a budget: the buffer enforces no depth, visit or output cap, because a
//! retained AST already fits the parse-time print envelope
//! ([`super::Ast::printable`]) resolved from the caller's
//! [`crate::Budget`]. The buffer only counts visits and depth so tests can
//! check the envelope is an upper bound.

use alloc::string::String;
use core::fmt;

/// Output text with the depth and visit accounting of one print run.
pub struct PrintBuffer {
    text: String,
    depth: u32,
    max_depth: u32,
    visits: usize,
}

impl PrintBuffer {
    /// Create an empty buffer.
    pub(crate) const fn new() -> Self {
        Self {
            text: String::new(),
            depth: 0,
            max_depth: 0,
            visits: 0,
        }
    }

    /// The text written so far.
    pub(crate) fn as_str(&self) -> &str {
        &self.text
    }

    /// Enter one recursion level, counting one node visit.
    #[expect(
        clippy::unnecessary_wraps,
        reason = "both printers propagate these with `?` like `fmt::Write`"
    )]
    pub(crate) fn enter(&mut self) -> fmt::Result {
        self.visits = self.visits.saturating_add(1);
        self.depth = self.depth.saturating_add(1);
        self.max_depth = self.max_depth.max(self.depth);
        Ok(())
    }

    /// Leave one recursion level opened by [`Self::enter`].
    pub(crate) const fn leave(&mut self) {
        self.depth -= 1;
    }

    /// Node visits charged by this print run.
    #[cfg(test)]
    pub(crate) const fn visits(&self) -> usize {
        self.visits
    }

    /// Maximum recursion depth reached.
    #[cfg(test)]
    pub(crate) const fn max_depth(&self) -> u32 {
        self.max_depth
    }

    /// Bytes collected in the buffer.
    #[cfg(test)]
    pub(crate) const fn bytes(&self) -> usize {
        self.text.len()
    }

    /// Collected output text.
    #[cfg(all(test, feature = "cpp-itanium"))]
    pub(crate) fn text(&self) -> &str {
        &self.text
    }

    /// Append `text`.
    #[expect(
        clippy::unnecessary_wraps,
        reason = "both printers propagate these with `?` like `fmt::Write`"
    )]
    pub(crate) fn write(&mut self, text: &str) -> fmt::Result {
        self.text.push_str(text);
        Ok(())
    }

    /// Append one character.
    #[expect(
        clippy::unnecessary_wraps,
        reason = "both printers propagate these with `?` like `fmt::Write`"
    )]
    pub(crate) fn write_char(&mut self, ch: char) -> fmt::Result {
        self.text.push(ch);
        Ok(())
    }

    /// Number of bytes written so far.
    ///
    /// The Itanium printer rewinds speculative output, so it is the only
    /// caller of this pair.
    #[cfg(feature = "cpp-itanium")]
    pub(crate) const fn position(&self) -> usize {
        self.text.len()
    }

    /// Drop everything after the first `position` bytes.
    #[cfg(feature = "cpp-itanium")]
    pub(crate) fn truncate(&mut self, position: usize) {
        self.text.truncate(position);
    }

    /// The last character written.
    pub(crate) fn last_char(&self) -> Option<char> {
        self.text.chars().next_back()
    }
}

#[cfg(test)]
mod tests;
