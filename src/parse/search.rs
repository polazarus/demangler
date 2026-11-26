//! Byte-substring search shared by the byte-native backends.
//!
//! A symbol is a byte string, so a backend that needs a marker position asks
//! these helpers instead of growing its own `windows` loop. An empty needle
//! has no useful offset and is reported as absent; that also keeps a
//! zero-width `slice::windows` call, which panics, out of every caller.

/// Byte offset of the first occurrence of `needle` in `haystack`.
///
/// Returns `None` for an empty needle or one longer than `haystack`.
pub fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || needle.len() > haystack.len() {
        return None;
    }
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

/// Byte offset of the last occurrence of `needle` in `haystack`.
///
/// Returns `None` for an empty needle or one longer than `haystack`.
pub fn rfind(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || needle.len() > haystack.len() {
        return None;
    }
    haystack
        .windows(needle.len())
        .rposition(|window| window == needle)
}
