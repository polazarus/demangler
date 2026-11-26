//! Bounded output buffer shared by the C++ printers.
//!
//! The Itanium printer ([`super::printer`]) and the legacy printer
//! ([`super::legacy`]) write their text through the same buffer: a symbol
//! prints at most [`MAX_OUTPUT_BYTES`] bytes, and the print run is bounded by a
//! recursion depth and a node-visit budget, because the node graph may be a
//! DAG and a shared node is re-visited on every reference.
//!
//! The depth and visit ceilings are derived from each parser's own AST budgets:
//! both printers get twice their parser's depth budget and sixteen times its
//! node budget. The absolute numbers therefore differ only because the two
//! parsers have different budgets; the factors are shared policy, named and
//! tested here rather than left as unexplained constants. The boundary probes
//! in `tests/cpp_nesting_limits.rs` show the parser cap binds first on the
//! adversarial deep-pointer shape.

use alloc::string::String;
use core::fmt;

#[cfg(feature = "cpp-gnu-v2")]
use super::legacy::{MAX_DEPTH as LEGACY_PARSE_DEPTH, MAX_NODES as LEGACY_PARSE_NODES};
#[cfg(feature = "cpp-itanium")]
use super::parser::{MAX_DEPTH as ITANIUM_PARSE_DEPTH, MAX_NODES as ITANIUM_PARSE_NODES};

/// Maximum bytes written for one symbol, shared by both C++ printers.
#[allow(dead_code)]
pub(crate) const MAX_OUTPUT_BYTES: usize = 1 << 20;

/// Recursion-depth and node-visit ceilings for one print run.
///
/// This type carries policy only: the two dialect budgets named below are the
/// supported values, and a printer embeds one of them for its whole run.
#[allow(dead_code)]
pub(crate) struct PrintLimits {
    /// Maximum number of nested recursion levels.
    pub(crate) depth: u32,
    /// Maximum node visits for one symbol.
    pub(crate) visits: usize,
}

/// Itanium print ceilings: twice the parser-depth budget and sixteen times the
/// parser's node budget.
///
/// The 2x depth margin covers the printer's left/right declarator walk, which
/// can descend further than the parser's production stack. Visits are larger
/// than nodes because substitutions can revisit a shared DAG node.
#[cfg(feature = "cpp-itanium")]
pub(crate) const ITANIUM_PRINT_LIMITS: PrintLimits = PrintLimits {
    depth: ITANIUM_PARSE_DEPTH * 2,
    visits: ITANIUM_PARSE_NODES as usize * 16,
};

/// Legacy (GNU v2, ARM, EDG, HP aCC) print ceilings: twice the parser-depth
/// budget and sixteen times the parser's node budget.
///
/// Same factors as Itanium. The smaller absolute numbers come from the legacy
/// parser's smaller AST budgets, not from a different printer policy.
#[cfg(feature = "cpp-gnu-v2")]
pub(crate) const LEGACY_PRINT_LIMITS: PrintLimits = PrintLimits {
    depth: LEGACY_PARSE_DEPTH * 2,
    visits: LEGACY_PARSE_NODES as usize * 16,
};

/// Output text with the depth, visit and output-byte accounting of one print
/// run.
pub(crate) struct PrintBuffer {
    text: String,
    depth: u32,
    max_depth: u32,
    visits: usize,
}

impl PrintBuffer {
    /// Create an empty buffer.
    ///
    /// `PrintLimits` is retained only for the call sites that still name a
    /// dialect; the buffer itself enforces no print cap.
    pub(crate) fn new(_limits: PrintLimits) -> Self {
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

    /// Enter one recursion level, charging one node visit.
    ///
    /// A rejected level leaves the depth where it was, so a caller that
    /// propagates the error never has to unwind it.
        #[allow(clippy::unnecessary_wraps)]
    pub(crate) fn enter(&mut self) -> fmt::Result {
        self.visits = self.visits.saturating_add(1);
        self.depth = self.depth.saturating_add(1);
        self.max_depth = self.max_depth.max(self.depth);
        Ok(())
    }

    /// Leave one recursion level opened by [`Self::enter`].
    pub(crate) fn leave(&mut self) {
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
    pub(crate) fn bytes(&self) -> usize {
        self.text.len()
    }

    /// Collected output text.
    #[cfg(test)]
    pub(crate) fn text(&self) -> &str {
        &self.text
    }

    /// Append `text`, or fail when that would exceed [`MAX_OUTPUT_BYTES`].
        #[allow(clippy::unnecessary_wraps)]
    pub(crate) fn write(&mut self, text: &str) -> fmt::Result {
        self.text.push_str(text);
        Ok(())
    }

    /// Append one character, or fail at [`MAX_OUTPUT_BYTES`].
        #[allow(clippy::unnecessary_wraps)]
    pub(crate) fn write_char(&mut self, ch: char) -> fmt::Result {
        self.text.push(ch);
        Ok(())
    }

    /// Number of bytes written so far.
    ///
    /// The Itanium printer rewinds speculative output, so it is the only
    /// caller of this pair.
    #[cfg(feature = "cpp-itanium")]
    pub(crate) fn position(&self) -> usize {
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
mod tests {
    use super::PrintBuffer;
    use super::PrintLimits;

    /// Limits are ignored; the buffer has no print cap.
    fn limits() -> PrintLimits {
        PrintLimits { depth: 2, visits: 2 }
    }

    #[test]
    fn the_buffer_has_no_output_depth_or_visit_cap() {
        let mut buffer = PrintBuffer::new(limits());
        let filler = "a".repeat(2 * 1024 * 1024);
        buffer.write(&filler).unwrap();
        assert_eq!(buffer.as_str().len(), filler.len());
        for _ in 0..10 {
            buffer.enter().unwrap();
        }
        for _ in 0..10 {
            buffer.leave();
        }
        buffer.write_char('c').unwrap();
        assert_eq!(buffer.last_char(), Some('c'));
    }
}
