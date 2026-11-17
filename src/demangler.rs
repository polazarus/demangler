use crate::error::Error;
use crate::result::Demangled;
use crate::scheme::Scheme;

/// Demangles symbol names using the appropriate demangling scheme.
///
/// Detects the scheme from the symbol name and delegates to the appropriate
/// backend. Delegation keeps the crate fast and correct today; hot paths can
/// be rewritten in-tree later without changing the public API.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Demangler;

impl Demangler {
    /// Create a new [`Demangler`].
    pub const fn new() -> Self {
        Self
    }

    /// Detect the scheme and demangle a symbol name.
    ///
    /// The returned [`Demangled`] borrows from `symbol` for non-allocating
    /// backends and implements [`core::fmt::Display`]; write it wherever the
    /// caller wants.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Unsupported`] when no backend implements the detected
    /// scheme (Swift and Go today) and [`Error::InvalidSymbol`] when the
    /// backend cannot demangle the input.
    pub fn demangle<'a>(&self, symbol: &'a str) -> Result<Demangled<'a>, Error> {
        let scheme = Scheme::detect(symbol);
        match scheme {
            Scheme::C | Scheme::Unknown => Ok(Demangled::plain(scheme, symbol)),
            Scheme::Rust => demangle_rust(scheme, symbol).ok_or(Error::InvalidSymbol(scheme)),
            // `_Z...` also prefixes legacy rustc symbols; fall back to the
            // Rust backend when the C++ backends decline.
            Scheme::Cpp => demangle_cpp(scheme, symbol)
                .or_else(|| demangle_rust(Scheme::Rust, symbol))
                .ok_or(Error::InvalidSymbol(scheme)),
            Scheme::Swift => {
                // Validate eagerly so callers get `InvalidSymbol` instead of a
                // formatting failure; `Display` re-runs the (deterministic)
                // printer, which is allocation-free.
                if crate::swift::print(symbol, &mut NullSink).is_ok() {
                    Ok(Demangled::swift(scheme, symbol))
                } else {
                    Err(Error::InvalidSymbol(scheme))
                }
            }
            Scheme::Go => Err(Error::Unsupported(scheme)),
        }
    }
}

/// A sink that accepts and discards all output.
struct NullSink;

impl core::fmt::Write for NullSink {
    fn write_str(&mut self, _s: &str) -> core::fmt::Result {
        Ok(())
    }
}

/// Rust backend (`rustc-demangle`, `no_std`, no `alloc`).
#[cfg(feature = "rust")]
#[allow(clippy::single_call_fn)]
fn demangle_rust<'a>(scheme: Scheme, symbol: &'a str) -> Option<Demangled<'a>> {
    rustc_demangle::try_demangle(symbol)
        .ok()
        .map(|demangled| Demangled::rust(scheme, demangled))
}

/// Rust backend stub when the feature is disabled.
#[cfg(not(feature = "rust"))]
#[allow(clippy::single_call_fn)]
const fn demangle_rust<'a>(_scheme: Scheme, _symbol: &'a str) -> Option<Demangled<'a>> {
    None
}

/// C++ backends (Itanium ABI and MSVC). Requires `alloc`/`std`; opt-in.
#[cfg(feature = "cpp")]
#[allow(clippy::single_call_fn)]
fn demangle_cpp<'a>(scheme: Scheme, symbol: &'a str) -> Option<Demangled<'a>> {
    let mut itanium = itanium_demangle::Demangler::new();
    if let Some(demangled) = itanium.demangle(symbol) {
        return Some(Demangled::owned(
            scheme,
            alloc::string::String::from(demangled),
        ));
    }

    msvc_demangler::demangle(symbol, msvc_demangler::DemangleFlags::llvm())
        .ok()
        .map(|demangled| Demangled::owned(scheme, demangled))
}

/// C++ backend stub when the feature is disabled.
#[cfg(not(feature = "cpp"))]
#[allow(clippy::single_call_fn)]
const fn demangle_cpp<'a>(_scheme: Scheme, _symbol: &'a str) -> Option<Demangled<'a>> {
    None
}

#[cfg(test)]
mod tests {
    use std::string::ToString;

    use super::*;

    #[test]
    fn detects_rust() {
        assert_eq!(
            Scheme::detect("_RN4main6hello17he3c1b56a8e1b2df5E"),
            Scheme::Rust
        );
    }

    #[test]
    fn detects_cpp_itanium() {
        assert_eq!(Scheme::detect("_ZN3Foo3barEv"), Scheme::Cpp);
    }

    #[test]
    fn detects_swift() {
        assert_eq!(Scheme::detect("$s4main3fooSi_yF"), Scheme::Swift);
    }

    #[test]
    fn detects_c() {
        assert_eq!(Scheme::detect("main"), Scheme::C);
    }

    #[test]
    fn detects_unknown() {
        assert_eq!(Scheme::detect("12345"), Scheme::Unknown);
        assert_eq!(Scheme::detect(""), Scheme::Unknown);
    }

    #[test]
    fn plain_names_pass_through() {
        let demangler = Demangler::new();
        let demangled = demangler.demangle("main").unwrap();
        assert_eq!(demangled.scheme(), Scheme::C);
        assert_eq!(demangled.to_string(), "main");
    }

    #[cfg(feature = "cpp")]
    #[test]
    fn demangles_cpp_itanium() {
        let demangler = Demangler::new();
        let demangled = demangler.demangle("_ZN3Foo3barEv").unwrap();
        assert_eq!(demangled.scheme(), Scheme::Cpp);
        assert_eq!(demangled.to_string(), "Foo::bar");
    }

    #[cfg(feature = "rust")]
    #[test]
    fn demangles_rust() {
        let demangler = Demangler::new();
        let demangled = demangler
            .demangle("_RNvCs1234_4test4main")
            .or_else(|_| demangler.demangle("_ZN4test4main17h1234567890abcdefE"))
            .unwrap();
        assert_eq!(demangled.language(), crate::Language::Rust);
    }

    #[test]
    fn demangles_swift() {
        let demangler = Demangler::new();
        let demangled = demangler.demangle("$s4main3fooyySiF").unwrap();
        assert_eq!(demangled.scheme(), Scheme::Swift);
        assert_eq!(demangled.language(), crate::Language::Swift);
        assert_eq!(demangled.to_string(), "main.foo(Swift.Int) -> ()");
    }

    #[test]
    fn rejects_swift_symbolic_references() {
        let demangler = Demangler::new();
        assert!(matches!(
            demangler.demangle("$s4main3foo\u{1}0123"),
            Err(Error::InvalidSymbol(Scheme::Swift))
        ));
    }

    #[test]
    fn swift_unsupported_prefix_is_not_detected() {
        // Legacy `_Tt` manglings are not part of the stable prefixes yet.
        let demangler = Demangler::new();
        assert_ne!(
            demangler.demangle("_TtBf32_").map(|d| d.scheme()),
            Ok(Scheme::Swift)
        );
    }
}
