use crate::scheme::{Language, Scheme};

/// Result of demangling a symbol name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DemangledResult<'a> {
    /// The original mangled symbol.
    pub original: &'a str,

    /// The demangled symbol name, or the original if demangling failed.
    pub name: alloc::string::String,

    /// The detected mangling scheme.
    pub scheme: Scheme,

    /// Whether demangling succeeded.
    pub success: bool,
}

impl<'a> DemangledResult<'a> {
    /// Return the language associated with the detected scheme.
    pub const fn language(&self) -> Language {
        self.scheme.language()
    }

    /// Create a successful result for the given scheme.
    #[allow(clippy::missing_const_for_fn)]
    pub fn ok(original: &'a str, name: alloc::string::String, scheme: Scheme) -> Self {
        Self {
            original,
            name,
            scheme,
            success: true,
        }
    }

    /// Create a failed result, falling back to the original name.
    pub fn fail(original: &'a str, scheme: Scheme) -> Self {
        Self {
            original,
            name: alloc::string::String::from(original),
            scheme,
            success: false,
        }
    }
}
