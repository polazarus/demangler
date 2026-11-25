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
pub(crate) const MAX_OUTPUT_BYTES: usize = 1 << 20;

/// Recursion-depth and node-visit ceilings for one print run.
///
/// This type carries policy only: the two dialect budgets named below are the
/// supported values, and a printer embeds one of them for its whole run.
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
    limits: PrintLimits,
    text: String,
    depth: u32,
    max_depth: u32,
    visits: usize,
}

impl PrintBuffer {
    /// Create an empty buffer bounded by `limits`.
    pub(crate) fn new(limits: PrintLimits) -> Self {
        Self {
            limits,
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
    pub(crate) fn enter(&mut self) -> fmt::Result {
        self.visits = self.visits.checked_add(1).ok_or(fmt::Error)?;
        if self.visits > self.limits.visits {
            return Err(fmt::Error);
        }
        self.depth = self.depth.checked_add(1).ok_or(fmt::Error)?;
        self.max_depth = self.max_depth.max(self.depth);
        if self.depth > self.limits.depth {
            self.depth -= 1;
            return Err(fmt::Error);
        }
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
    pub(crate) fn write(&mut self, text: &str) -> fmt::Result {
        if self
            .text
            .len()
            .checked_add(text.len())
            .is_none_or(|len| len > MAX_OUTPUT_BYTES)
        {
            return Err(fmt::Error);
        }
        self.text.push_str(text);
        Ok(())
    }

    /// Append one character, or fail at [`MAX_OUTPUT_BYTES`].
    pub(crate) fn write_char(&mut self, ch: char) -> fmt::Result {
        if self.text.len() >= MAX_OUTPUT_BYTES {
            return Err(fmt::Error);
        }
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
    #[cfg(feature = "cpp-itanium")]
    use super::{ITANIUM_PARSE_DEPTH, ITANIUM_PARSE_NODES, ITANIUM_PRINT_LIMITS};
    #[cfg(feature = "cpp-gnu-v2")]
    use super::{LEGACY_PARSE_DEPTH, LEGACY_PARSE_NODES, LEGACY_PRINT_LIMITS};
    use super::{MAX_OUTPUT_BYTES, PrintBuffer, PrintLimits};

    /// Limits that never bound the case under test.
    fn limits(depth: u32, visits: usize) -> PrintLimits {
        PrintLimits { depth, visits }
    }

    #[test]
    fn output_cap_is_shared_and_inclusive() {
        assert_eq!(MAX_OUTPUT_BYTES, 1 << 20);
        let mut buffer = PrintBuffer::new(limits(4, 4));
        let filler = "a".repeat(MAX_OUTPUT_BYTES - 1);
        buffer.write(&filler).unwrap();
        // The cap is a byte count, not a "one below" limit.
        buffer.write_char('b').unwrap();
        assert_eq!(buffer.as_str().len(), MAX_OUTPUT_BYTES);
        assert!(buffer.write_char('c').is_err());
        assert!(buffer.write("x").is_err());
        // A rejected write leaves the text untouched.
        assert_eq!(buffer.last_char(), Some('b'));
        assert_eq!(buffer.as_str().len(), MAX_OUTPUT_BYTES);
    }

    #[test]
    fn depth_cap_rejects_the_level_above_the_limit() {
        let mut buffer = PrintBuffer::new(limits(2, usize::MAX));
        buffer.enter().unwrap();
        buffer.enter().unwrap();
        assert!(buffer.enter().is_err());
        // The rejected level is not left open: two `leave` calls reach zero.
        buffer.leave();
        buffer.leave();
        buffer.enter().unwrap();
        buffer.leave();
    }

    #[test]
    fn visit_cap_counts_the_whole_run_not_the_open_levels() {
        let mut buffer = PrintBuffer::new(limits(u32::MAX, 2));
        buffer.enter().unwrap();
        buffer.leave();
        buffer.enter().unwrap();
        buffer.leave();
        // Visits accumulate across levels, so a second sibling visit fails.
        assert!(buffer.enter().is_err());
    }

    #[cfg(feature = "cpp-itanium")]
    #[test]
    fn itanium_dialect_ceilings_follow_the_parser_budgets() {
        assert_eq!(ITANIUM_PRINT_LIMITS.depth, ITANIUM_PARSE_DEPTH * 2);
        assert_eq!(
            ITANIUM_PRINT_LIMITS.visits,
            ITANIUM_PARSE_NODES as usize * 16
        );
    }

    #[cfg(feature = "cpp-gnu-v2")]
    #[test]
    fn legacy_dialect_ceilings_follow_the_parser_budgets() {
        assert_eq!(LEGACY_PRINT_LIMITS.depth, LEGACY_PARSE_DEPTH * 2);
        assert_eq!(LEGACY_PRINT_LIMITS.visits, LEGACY_PARSE_NODES as usize * 16);
    }
}
