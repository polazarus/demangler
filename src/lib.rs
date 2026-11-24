//! # demangler
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
//! Rust and Swift retain `Rc`-owned node graphs, so [`Demangled`] is
//! `!Send + !Sync`. Demangle again on the target thread instead of moving a
//! value across threads.
//!
//! # Supported Languages
//!
//! - **C**: plain C symbols (no mangling)
//! - **C++**: Itanium ABI (`_Z`), MSVC (`?`) and legacy GNU v2/ARM/EDG/HP aCC
//!   — enable `cpp-itanium`, `cpp-msvc`, `cpp-gnu-v2` or the `cpp` umbrella
//! - **Rust**: rustc symbols (`_RN...`, `_ZN...`) — feature `rust`
//! - **Swift**: `$s`/`$S`/`_T0` manglings — feature `swift`
//! - **Go**: package/function/method/closure/generic/type names — feature
//!   `go`
//! - **Objective-C**: Apple/Mach-O and GNUstep runtime metadata, plus method
//!   display forms — feature `objc`
//! - **Kotlin/Native**: `kfun:`/`ktype:`/`kclass:` and related linker symbols —
//!   feature `kotlin-native`
//! - **D**: `_D` ABI names — feature `d`
//! - **Java**: JNI native-method names (`Java_...`) — feature `jni`
//! - **Ada**: GNAT external names (`pkg__subp`) — feature `ada`
//! - **Fortran**: gfortran external and module names — feature `fortran`
//! - **Delphi/Object Pascal**: Free Pascal and Delphi linker names — feature
//!   `delphi`
//! - **OCaml**: native `caml...` linker names — feature `ocaml`
//! - **PE/COFF**: target-aware decoration metadata (`_name@N`, `@name@N`,
//!   `name@@N`, `__imp_`, weak aliases, cold/partial fragments) — feature
//!   `coff`
//!
//! # Example
//!
//! ```rust
//! use demangler::{Demangler, Language, Scheme};
//!
//! let demangler = Demangler::new();
//! let demangled = demangler.demangle("main").unwrap();
//!
//! assert_eq!(demangled.scheme(), Scheme::C);
//! assert_eq!(demangled.language(), Language::C);
//! assert_eq!(demangled.to_string(), "main");
//! ```
//!
//! # Explicit schemes
//!
//! [`Demangler::demangle_with_scheme`] (`&str`) and
//! [`Demangler::demangle_bytes_with_scheme`] (`&[u8]`) bypass detection when
//! the caller already knows the scheme. They validate the selected backend and
//! return the same [`Error::InvalidSymbol`] a detected name would. This is the
//! integration point for names whose automatic detection is deliberately
//! conservative: general Ada external names (`pkg__subp`), gfortran external
//! names (`foo_`), namespaced Free Pascal units and OCaml external names.
//!
//! # Platform decorations
//!
//! Two platform conventions are orthogonal to the language mangling and are
//! decoded only by an explicit entry point, never by detection:
//!
//! - **PE/COFF**: `coff::parse_bytes_for` decodes the decoration rules of a
//!   declared `coff::Target` (`_name@N`, `name@@N`, `__imp_`, `.weak.`
//!   wrappers and `.cold`/`.cold.N`/`.part.N` fragments) and reports it
//!   through `Demangled::coff_decoration`.
//! - **Mach-O**: [`Demangler::demangle_macho`] and
//!   [`Demangler::demangle_macho_bytes`] strip the platform leading underscore
//!   of a C symbol (`_main` -> `main`) when no backend claims the name as
//!   written and the remainder is a plain C identifier.
//!
//! # Character encodings
//!
//! Backends render identifier bytes with reversible escapes: a literal
//! backslash becomes `\\` and an invalid byte becomes `\xNN`. The demangler
//! never transcodes those bytes on its own. `Encoding` and
//! `Demangler::demangle_to_string_with_encoding` decode the escapes as UTF-8,
//! Latin-1, Windows-1252, MacRoman or EBCDIC code page 037 when the caller
//! knows the object file's encoding. Shift-JIS, EUC-JP, other EBCDIC pages and
//! other legacy code pages are not supported, and no UTF-16 mode exists: a
//! symbol table carries NUL-terminated bytes, so there is no byte-order mark to
//! read. Convert such a name to UTF-8, or decode it with an external codec,
//! before demangling.

#![no_std]

#[cfg(any(
    feature = "cpp-itanium",
    feature = "cpp-msvc",
    feature = "cpp-gnu-v2",
    feature = "rust",
    feature = "swift",
    feature = "go",
    feature = "objc",
    feature = "kotlin-native",
    feature = "d",
    feature = "jni",
    feature = "ada",
    feature = "delphi",
    feature = "encoding",
    all(test, any(feature = "fortran", feature = "ocaml"))
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

mod parse;

/// Error type for demangling failures.
pub mod error;

/// Explicit decoding of reversible identifier escapes.
#[cfg(feature = "encoding")]
pub mod encoding;

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

/// Objective-C metadata and method display parser and printer.
#[cfg(feature = "objc")]
pub mod objc;

/// Kotlin/Native linker symbol parser and printer (pure Rust, `no_std` + `alloc`).
#[cfg(feature = "kotlin-native")]
pub mod kotlin_native;

/// D ABI parser and printer (pure Rust, `no_std` + `alloc`).
#[cfg(feature = "d")]
pub mod d;

/// Java JNI native-method name parser and printer (pure Rust, `no_std` + `alloc`).
#[cfg(feature = "jni")]
pub mod jni;

/// Ada/GNAT external-name parser and printer (pure Rust, `no_std` + `alloc`).
#[cfg(feature = "ada")]
pub mod ada;

/// PE/COFF symbol decoration parser (`no_std`, no allocator).
#[cfg(feature = "coff")]
pub mod coff;

/// Fortran external-name parser and printer (pure Rust, `no_std` + `alloc`).
#[cfg(feature = "fortran")]
pub mod fortran;

/// Delphi/Object Pascal external-name parser and printer (pure Rust, `no_std` + `alloc`).
#[cfg(feature = "delphi")]
pub mod delphi;

/// OCaml external-name parser and printer (pure Rust, `no_std` + `alloc`).
#[cfg(feature = "ocaml")]
pub mod ocaml;

pub use demangler::Demangler;
#[cfg(feature = "encoding")]
pub use encoding::{Encoding, EncodingError};
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
