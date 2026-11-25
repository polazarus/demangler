//! Bounded output sink shared by every backend printer.

use core::fmt;

/// A [`fmt::Write`] sink that refuses to grow the output past `limit` bytes.
///
/// Each backend passes its own absolute output ceiling, so one writer enforces
/// `MAX_OUTPUT_LEN` for the byte-native backends and the Swift printer's output
/// cap. A write that would cross the limit fails without reaching `inner`, so a
/// caller never observes a partial over-budget result.
pub(crate) struct LimitedWriter<'a, W: fmt::Write + ?Sized> {
    inner: &'a mut W,
    written: usize,
    limit: usize,
}

impl<'a, W: fmt::Write + ?Sized> LimitedWriter<'a, W> {
    /// Bytes written through this guard.
    #[cfg(test)]
    pub(crate) const fn written(&self) -> usize {
        self.written
    }

    /// Wrap `inner`, rejecting any write that would exceed `limit` bytes.
    pub(crate) const fn new(inner: &'a mut W, limit: usize) -> Self {
        Self {
            inner,
            written: 0,
            limit,
        }
    }
}

impl<W: fmt::Write + ?Sized> fmt::Write for LimitedWriter<'_, W> {
    fn write_str(&mut self, text: &str) -> fmt::Result {
        let total = self.written.checked_add(text.len()).ok_or(fmt::Error)?;
        if total > self.limit {
            return Err(fmt::Error);
        }
        self.written = total;
        self.inner.write_str(text)
    }
}

#[cfg(test)]
impl<'a, W: fmt::Write + ?Sized> LimitedWriter<'a, W> {
    /// Wrap `inner` with a pre-charged counter, so the overflow guard is
    /// reachable without a multi-exabyte write.
    pub(crate) const fn with_written(inner: &'a mut W, limit: usize, written: usize) -> Self {
        Self {
            inner,
            written,
            limit,
        }
    }
}
