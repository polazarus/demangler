use alloc::string::{String, ToString};

use crate::result::DemangledResult;
use crate::scheme::Scheme;

/// Demangles symbol names using the appropriate demangling scheme.
///
/// Detects the scheme from the symbol name and delegates to the appropriate
/// backend crate. This is a **delegation-only** strategy for now: it keeps
/// speed and correctness for free while we later prioritize rewriting the
/// hot paths ourselves.
#[derive(Debug, Default)]
pub struct Demangler {
    // No state today; future backends may hold per-call scratch buffers.
}

impl Demangler {
    /// Create a new [`Demangler`].
    pub const fn new() -> Self {
        Self {}
    }

    /// Detect the scheme and demangle a symbol name.
    ///
    /// Returns a [`DemangledResult`] whose `success` flag indicates whether
    /// the backend could actually demangle it.
    pub fn demangle<'a>(&self, name: &'a str) -> DemangledResult<'a> {
        let scheme = Scheme::detect(name);
        match scheme {
            Scheme::Cpp => self.demangle_cpp(name, scheme),
            Scheme::Rust => self.demangle_rust(name, scheme),
            Scheme::Swift | Scheme::C | Scheme::Unknown | Scheme::Go => {
                // Swift uses a dedicated pure-Rust submodule (planned); the
                // other schemes are identity demanglings for now.
                DemangledResult::ok(name, String::from(name), scheme)
            }
        }
    }

    #[cfg(feature = "cpp")]
    fn demangle_cpp<'a>(&self, name: &'a str, scheme: Scheme) -> DemangledResult<'a> {
        let mut demangler = itanium_demangle::Demangler::new();
        if let Some(demangled) = demangler.demangle(name) {
            return DemangledResult::ok(name, demangled.to_string(), scheme);
        }
        if let Ok(demangled) = msvc_demangler::demangle(name, msvc_demangler::DemangleFlags::llvm())
        {
            return DemangledResult::ok(name, demangled, scheme);
        }
        DemangledResult::fail(name, scheme)
    }

    #[cfg(not(feature = "cpp"))]
    fn demangle_cpp<'a>(&self, name: &'a str, scheme: Scheme) -> DemangledResult<'a> {
        DemangledResult::fail(name, scheme)
    }

    #[cfg(feature = "rust")]
    fn demangle_rust<'a>(&self, name: &'a str, scheme: Scheme) -> DemangledResult<'a> {
        if let Ok(demangled) = rustc_demangle::try_demangle(name) {
            return DemangledResult::ok(name, demangled.to_string(), scheme);
        }
        DemangledResult::fail(name, scheme)
    }

    #[cfg(not(feature = "rust"))]
    fn demangle_rust<'a>(&self, name: &'a str, scheme: Scheme) -> DemangledResult<'a> {
        DemangledResult::fail(name, scheme)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_rust() {
        let d = Demangler::new();
        let result = d.demangle("_RN4main6hello17he3c1b56a8e1b2df5E");
        assert_eq!(result.scheme, Scheme::Rust);
    }

    #[test]
    fn detects_cpp_itanium() {
        let d = Demangler::new();
        let result = d.demangle("_ZN3Foo3barEv");
        assert_eq!(result.scheme, Scheme::Cpp);
    }

    #[test]
    fn detects_swift() {
        let d = Demangler::new();
        let result = d.demangle("$s4main3fooSi_yF");
        assert_eq!(result.scheme, Scheme::Swift);
    }

    #[test]
    fn detects_c() {
        let d = Demangler::new();
        let result = d.demangle("main");
        assert_eq!(result.scheme, Scheme::C);
    }

    #[test]
    fn detects_unknown() {
        let d = Demangler::new();
        let result = d.demangle("12345");
        assert_eq!(result.scheme, Scheme::Unknown);
    }

    #[cfg(feature = "cpp")]
    #[test]
    fn demangles_itanium() {
        let d = Demangler::new();
        let result = d.demangle("_ZN3Foo3barEv");
        assert!(result.success);
    }

    #[test]
    fn handles_empty_string() {
        let d = Demangler::new();
        let result = d.demangle("");
        assert_eq!(result.scheme, Scheme::Unknown);
    }
}
