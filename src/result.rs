use core::fmt;

use crate::scheme::{Language, Scheme};

/// The demangled text of a symbol.
///
/// Implements [`fmt::Display`] so it can be written to any sink. Rust, Swift,
/// C++, Go, Objective-C and Kotlin/Native retain their parsed representation
/// and render lazily.
///
/// # Thread safety
///
/// Rust, Swift and C++ retain `Rc`-owned node graphs, so `Demangled` is
/// `!Send + !Sync`. Demangle again on the target thread instead of moving a
/// value across threads.
#[derive(Debug)]
pub struct Demangled<'a> {
    scheme: Scheme,
    text: Text<'a>,
}

/// Backing text representation, chosen per backend.
#[derive(Debug)]
enum Text<'a> {
    /// The symbol is already readable (C, unknown, or pass-through).
    Plain(&'a str),
    /// Non-UTF-8 bytes passed through with escaped invalid bytes.
    Bytes(&'a [u8]),
    /// Parsed Rust tree, rendered lazily.
    #[cfg(feature = "rust")]
    Rust(crate::rust::Ast<'a>),
    /// A parsed C++ tree, rendered lazily.
    #[cfg(any(
        feature = "cpp-itanium",
        feature = "cpp-msvc",
        feature = "cpp-gnu-v2"
    ))]
    Cpp(crate::cpp::Ast<'a>),
    /// A parsed Swift tree, rendered lazily.
    #[cfg(feature = "swift")]
    Swift(crate::swift::Ast<'a>),
    /// A parsed Go tree, rendered lazily.
    #[cfg(feature = "go")]
    Go(crate::go::Ast<'a>),
    /// A parsed Objective-C symbol, rendered lazily.
    #[cfg(feature = "objc")]
    ObjC(crate::objc::Ast<'a>),
    /// A parsed Kotlin/Native symbol, rendered lazily.
    #[cfg(feature = "kotlin-native")]
    KotlinNative(crate::kotlin_native::Ast<'a>),
}

impl<'a> Demangled<'a> {
    /// Plain, non-mangled text.
    #[allow(clippy::single_call_fn)]
    pub(crate) const fn plain(scheme: Scheme, text: &'a str) -> Self {
        Self {
            scheme,
            text: Text::Plain(text),
        }
    }

    /// Raw non-UTF-8 bytes; rendered with invalid bytes escaped.
    #[allow(clippy::single_call_fn)]
    pub(crate) const fn bytes(scheme: Scheme, bytes: &'a [u8]) -> Self {
        Self {
            scheme,
            text: Text::Bytes(bytes),
        }
    }

    /// A parsed Rust tree.
    #[cfg(feature = "rust")]
    #[allow(clippy::single_call_fn)]
    pub(crate) const fn rust(scheme: Scheme, text: crate::rust::Ast<'a>) -> Self {
        Self {
            scheme,
            text: Text::Rust(text),
        }
    }

    /// A parsed C++ tree.
    #[cfg(any(
        feature = "cpp-itanium",
        feature = "cpp-msvc",
        feature = "cpp-gnu-v2"
    ))]
    #[allow(clippy::single_call_fn)]
    pub(crate) const fn cpp(scheme: Scheme, text: crate::cpp::Ast<'a>) -> Self {
        Self {
            scheme,
            text: Text::Cpp(text),
        }
    }

    /// A parsed Swift tree.
    #[cfg(feature = "swift")]
    #[allow(clippy::single_call_fn)]
    pub(crate) const fn swift(scheme: Scheme, parsed: crate::swift::Ast<'a>) -> Self {
        Self {
            scheme,
            text: Text::Swift(parsed),
        }
    }

    /// A parsed Go tree.
    #[cfg(feature = "go")]
    #[allow(clippy::single_call_fn)]
    pub(crate) const fn go(scheme: Scheme, parsed: crate::go::Ast<'a>) -> Self {
        Self {
            scheme,
            text: Text::Go(parsed),
        }
    }

    /// A parsed Objective-C symbol.
    #[cfg(feature = "objc")]
    #[allow(clippy::single_call_fn)]
    pub(crate) const fn objc(scheme: Scheme, parsed: crate::objc::Ast<'a>) -> Self {
        Self {
            scheme,
            text: Text::ObjC(parsed),
        }
    }

    /// A parsed Kotlin/Native symbol.
    #[cfg(feature = "kotlin-native")]
    #[allow(clippy::single_call_fn)]
    pub(crate) const fn kotlin_native(
        scheme: Scheme,
        parsed: crate::kotlin_native::Ast<'a>,
    ) -> Self {
        Self {
            scheme,
            text: Text::KotlinNative(parsed),
        }
    }

    /// The detected mangling scheme.
    pub const fn scheme(&self) -> Scheme {
        self.scheme
    }

    /// The language associated with the detected scheme.
    pub const fn language(&self) -> Language {
        self.scheme.language()
    }
}

/// Render raw bytes as printable ASCII, escaping everything else as `\xNN`.
fn write_escaped_bytes(f: &mut fmt::Formatter<'_>, bytes: &[u8]) -> fmt::Result {
    let mut start = 0;
    for (index, &byte) in bytes.iter().enumerate() {
        if matches!(byte, 0x20..=0x7e) {
            continue;
        }
        if start < index {
            f.write_str(core::str::from_utf8(&bytes[start..index]).map_err(|_| fmt::Error)?)?;
        }
        write!(f, "\\x{byte:02x}")?;
        start = index + 1;
    }
    if start < bytes.len() {
        f.write_str(core::str::from_utf8(&bytes[start..]).map_err(|_| fmt::Error)?)?;
    }
    Ok(())
}

impl fmt::Display for Demangled<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.text {
            Text::Plain(text) => f.write_str(text),
            Text::Bytes(bytes) => write_escaped_bytes(f, bytes),
            #[cfg(feature = "rust")]
            Text::Rust(text) => write!(f, "{text}"),
            #[cfg(any(
                feature = "cpp-itanium",
                feature = "cpp-msvc",
                feature = "cpp-gnu-v2"
            ))]
            Text::Cpp(text) => write!(f, "{text}"),
            #[cfg(feature = "swift")]
            Text::Swift(parsed) => parsed.print(f),
            #[cfg(feature = "go")]
            Text::Go(parsed) => parsed.print(f),
            #[cfg(feature = "objc")]
            Text::ObjC(parsed) => parsed.print(f),
            #[cfg(feature = "kotlin-native")]
            Text::KotlinNative(parsed) => parsed.print(f),
        }
    }
}
