//! Test-only bounded output sink used to assert printer output bounds.

use core::fmt;

/// A [`fmt::Write`] sink that refuses to grow the output past `limit` bytes.
///
/// Tests wrap a printer sink in it to check that an output estimate is an upper
/// bound. No production printer uses it.
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
