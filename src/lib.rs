//! # demangle
//!
//! A universal demangler for binary symbol names written in pure Rust.
//!
//! Detects the mangling scheme (language and/or convention) and demangles
//! symbol names accordingly.
//!
//! The core crate is `#![no_std]` and uses `alloc`. Demangled text is exposed
//! through [`core::fmt::Display`]; parsed trees and owned strings use `alloc`.
//! Backends are selected by Cargo features and are compiled in-tree.
//!
//! # Thread safety
//!
//! Swift demangling retains an `Rc`-owned node graph, so [`Demangled`] is
//! `!Send + !Sync`. Demangle again on the target thread instead of moving a
//! value across threads.
//!
//! # Supported Languages
//!
//! - **C**: plain C symbols (no mangling)
//! - **C++**: Itanium ABI (`_Z`), MSVC (`?`) and legacy GNU v2 — enable
//!   `cpp-itanium`, `cpp-msvc`, `cpp-gnu-v2` or the `cpp` umbrella
//! - **Rust**: rustc symbols (`_RN...`, `_ZN...`) — feature `rust`
//! - **Swift**: `$s`/`$S`/`_T0` manglings — feature `swift`
//! - **Go**: package/function/method/closure/generic/type names — feature
//!   `go`
//!
//! # Example
//!
//! ```rust
//! use demangle::{Demangler, Language, Scheme};
//!
//! let demangler = Demangler::new();
//! let demangled = demangler.demangle("main").unwrap();
//!
//! assert_eq!(demangled.scheme(), Scheme::C);
//! assert_eq!(demangled.language(), Language::C);
//! assert_eq!(demangled.to_string(), "main");
//! ```

#![no_std]

#[cfg(any(
    feature = "cpp-itanium",
    feature = "cpp-msvc",
    feature = "cpp-gnu-v2",
    feature = "rust",
    feature = "swift",
    feature = "go"
))]
extern crate alloc;

#[cfg(test)]
extern crate std;

/// C++ ABI parsers and inspectable syntax tree.
///
/// Enable all dialects with `cpp`, or select `cpp-itanium`, `cpp-msvc` and/or
/// `cpp-gnu-v2` for a lighter build.
#[cfg(any(
    feature = "cpp-itanium",
    feature = "cpp-msvc",
    feature = "cpp-gnu-v2"
))]
pub mod cpp;

/// Demangler orchestrator and symbol demangling entry point.
pub mod demangler;

/// Error type for demangling failures.
pub mod error;

/// Demangling results.
pub mod result;

/// Rust mangling parser and printer (pure Rust, `no_std` + `alloc`).
#[cfg(feature = "rust")]
pub mod rust;

/// Scheme detection and language identification.
pub mod scheme;

/// Swift mangling parser and printer (pure Rust, `no_std` + `alloc`).
#[cfg(feature = "swift")]
pub mod swift;

/// Go symbol name parser and printer (pure Rust, `no_std` + `alloc`).
#[cfg(feature = "go")]
pub mod go;

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
