//! # demangle
//!
//! A universal demangler for binary symbol names written in pure Rust.
//!
//! Detects the mangling scheme (language and/or convention) and demangles
//! symbol names accordingly.
//!
//! The core crate is `#![no_std]` and uses `alloc`. Demangled text is exposed
//! through [`core::fmt::Display`]; parsed trees and owned strings use `alloc`.
//! C++, Rust and Swift are in-tree; there are no runtime backends.
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
//! - **C++**: Itanium ABI (`_Z`), MSVC (`?`) — in-tree, always available
//! - **Rust**: rustc symbols (`_RN...`, `_ZN...`) — in-tree, always available
//! - **Swift**: `$s`/`$S`/`_T0` manglings — pure Rust, `no_std` + `alloc`
//! - **Go**: not yet supported (planned)
//!
//! # Example
//!
//! ```rust
//! use demangle::{Demangler, Language, Scheme};
//!
//! let demangler = Demangler::new();
//! let demangled = demangler.demangle("_RNvC4test4main").unwrap();
//!
//! assert_eq!(demangled.scheme(), Scheme::Rust);
//! assert_eq!(demangled.language(), Language::Rust);
//! assert_eq!(demangled.to_string(), "test::main");
//! ```

#![no_std]

extern crate alloc;

#[cfg(test)]
extern crate std;

/// Itanium C++ ABI parser and inspectable syntax tree.
pub mod cpp;

/// Demangler orchestrator and symbol demangling entry point.
pub mod demangler;

/// Error type for demangling failures.
pub mod error;

/// Demangling results.
pub mod result;

/// Rust mangling parser and printer (pure Rust, `no_std` + `alloc`).
pub mod rust;

/// Scheme detection and language identification.
pub mod scheme;

/// Swift mangling parser and printer (pure Rust, `no_std` + `alloc`).
pub mod swift;

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
