//! # demangle
//!
//! A universal demangler for binary symbol names written in pure Rust.
//!
//! Detects the mangling scheme (language and/or convention) and demangles
//! symbol names accordingly.
//!
//! The core crate is `#![no_std]` and does not require `alloc`: demangled
//! text is exposed through [`core::fmt::Display`], so callers decide whether
//! and where to allocate. Backends that require `alloc`/`std` (the C++
//! backends) are opt-in via the `cpp` feature and are earmarked for a rewrite.
//!
//! # Supported Languages
//!
//! - **C**: plain C symbols (no mangling)
//! - **C++**: Itanium ABI (`_Z`), MSVC (`?`) — feature `cpp`
//! - **Rust**: rustc symbols (`_RN...`, `_ZN...`) — feature `rust`
//! - **Swift**: Swift mangled symbols (`$s`) — dedicated pure-Rust submodule (planned)
//! - **Go**: not yet supported (planned)
//!
//! # Example
//!
//! ```rust
//! use demangle::{Demangler, Language, Scheme};
//!
//! let demangler = Demangler::new();
//! let demangled = demangler.demangle("_ZN3Foo3barEv").unwrap();
//!
//! assert_eq!(demangled.scheme(), Scheme::Cpp);
//! assert_eq!(demangled.language(), Language::Cxx);
//! assert_eq!(demangled.to_string(), "Foo::bar");
//! ```

#![no_std]

#[cfg(feature = "cpp")]
extern crate alloc;

#[cfg(test)]
extern crate std;

/// Demangler orchestrator and symbol demangling entry point.
pub mod demangler;

/// Error type for demangling failures.
pub mod error;

/// Demangling results.
pub mod result;

/// Scheme detection and language identification.
pub mod scheme;

pub use demangler::Demangler;
pub use error::Error;
pub use result::Demangled;
pub use scheme::{Language, Scheme};

#[cfg(test)]
mod tests {
    #[test]
    fn version_matches_cargo() {
        let version = env!("CARGO_PKG_VERSION");
        assert!(!version.is_empty());
    }
}
