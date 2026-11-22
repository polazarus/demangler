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
    /// The returned [`Demangled`] implements [`core::fmt::Display`]. Swift
    /// retains the parsed node tree and prints it lazily; other backends
    /// borrow from `symbol` or store owned text.
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
            Scheme::Cpp => {
                // `_Z...` also prefixes legacy rustc symbols with a trailing
                // crate hash. Prefer Rust for those; `cpp_demangle` would
                // otherwise print them as C++ data names.
                if is_rust_legacy_hash(symbol)
                    && let Some(demangled) = demangle_rust(Scheme::Rust, symbol)
                {
                    return Ok(demangled);
                }
                demangle_cpp(scheme, symbol)
                    .or_else(|| demangle_rust(Scheme::Rust, symbol))
                    .ok_or(Error::InvalidSymbol(scheme))
            }
            Scheme::Swift => crate::swift::parse(symbol)
                .map(|parsed| Demangled::swift(scheme, parsed))
                .map_err(|_| Error::InvalidSymbol(scheme)),
            Scheme::Go => Err(Error::Unsupported(scheme)),
        }
    }
}

/// True for a legacy rustc symbol with the trailing 16-hex-digit crate hash.
///
/// The legacy prefix is shared with Itanium C++ names, so only the hash makes
/// the Rust encoding unambiguous.
fn is_rust_legacy_hash(symbol: &str) -> bool {
    let symbol = symbol.split(".llvm.").next().unwrap_or(symbol);
    let inner = symbol
        .strip_prefix("__ZN")
        .or_else(|| symbol.strip_prefix("_ZN"))
        .or_else(|| symbol.strip_prefix("ZN"));
    let Some(inner) = inner else {
        return false;
    };
    let Some(path) = inner.strip_suffix('E') else {
        return false;
    };
    let Some(hash) = path.get(path.len().saturating_sub(19)..) else {
        return false;
    };
    hash.strip_prefix("17h").is_some_and(|digits| {
        digits.len() == 16 && digits.bytes().all(|byte| byte.is_ascii_hexdigit())
    })
}

/// In-tree Rust backend (`no_std` + `alloc`).
#[allow(clippy::single_call_fn)]
fn demangle_rust<'a>(scheme: Scheme, symbol: &'a str) -> Option<Demangled<'a>> {
    crate::rust::parse(symbol)
        .ok()
        .map(|parsed| Demangled::rust(scheme, parsed))
}

/// C++ backends (Itanium ABI and MSVC). Requires `alloc`; opt-in.
#[cfg(feature = "cpp")]
#[allow(clippy::single_call_fn)]
fn demangle_cpp<'a>(scheme: Scheme, symbol: &'a str) -> Option<Demangled<'a>> {
    if symbol.len() > crate::cpp::MAX_INPUT_LEN {
        return None;
    }
    if let Ok(symbol) = cpp_demangle::Symbol::new(symbol.as_bytes())
        && let Ok(demangled) = symbol.demangle()
    {
        return Some(Demangled::owned(scheme, demangled));
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
    use std::string::{String, ToString};

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
        assert_eq!(demangled.to_string(), "Foo::bar()");
    }

    #[cfg(feature = "cpp")]
    #[test]
    fn cpp_input_is_bounded() {
        let demangler = Demangler::new();
        let mut symbol = String::from("_Z");
        symbol.push_str(&"1".repeat(crate::cpp::MAX_INPUT_LEN));
        assert!(demangler.demangle(&symbol).is_err());
    }

    #[cfg(feature = "cpp")]
    #[test]
    fn cpp_keeps_full_signatures() {
        let demangler = Demangler::new();
        for (symbol, expected) in [
            ("_Z1fPFivE", "f(int (*)())"),
            ("_Z1fIiEvT_", "void f<int>(int)"),
            ("_Z1fA5_i", "f(int [5])"),
            ("_Z1fM1AFivE", "f(int (A::*)())"),
            ("_ZSt4cout", "std::cout"),
        ] {
            assert_eq!(
                demangler.demangle(symbol).unwrap().to_string(),
                expected,
                "{symbol}"
            );
        }
    }

    #[test]
    fn rust_legacy_hash_is_not_parsed_as_cpp() {
        let demangler = Demangler::new();
        let demangled = demangler
            .demangle("_ZN4test4main17h1234567890abcdefE")
            .unwrap();
        assert_eq!(demangled.language(), crate::Language::Rust);
        assert_eq!(demangled.to_string(), "test::main::h1234567890abcdef");
    }

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
