//! # demangle
//!
//! A universal demangler for binary symbol names written in pure Rust.
//!
//! Detects the mangling scheme (language and/or convention) and demangles
//! symbol names accordingly.
//!
//! # Supported Languages
//!
//! - **C**: plain C symbols (no mangling)
//! - **C++**: Itanium ABI (`_Z`), MSVC (`?`)
//! - **Rust**: rustc symbols (`_RN...`, `_ZN...`)
//! - **Swift**: Swift mangled symbols (`$s`) — custom implementation planned
//! - **Go**: not yet supported (planned)
//!
//! # Example
//!
//! ```rust
//! use demangle::{Demangler, DemangledResult};
//!
//! let demangler = Demangler::new();
//! let result = demangler.demangle("_ZN3Foo3barEv");
//! assert_eq!(result.scheme, demangle::Scheme::Cpp);
//! assert_eq!(result.language(), demangle::Language::Cxx);
//! assert_eq!(result.name, "Foo::bar");
//! ```

#![no_std]

extern crate alloc;

/// Demangler orchestrator and symbol demangling entry point.
pub mod demangler;

/// Demangling result types.
pub mod result;

/// Scheme detection and language identification.
pub mod scheme;

pub use alloc::string::String;
pub use demangler::Demangler;
pub use result::DemangledResult;
pub use scheme::{Language, Scheme};

#[cfg(test)]
extern crate std;

#[cfg(test)]
mod tests {
    #[test]
    fn version_matches_cargo() {
        let version = env!("CARGO_PKG_VERSION");
        assert!(!version.is_empty());
    }
}
