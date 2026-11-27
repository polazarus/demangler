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
//! # Rendering cost
//!
//! Backends that retain an AST render it again on every [`core::fmt::Display`]
//! call; rendered text is not cached by the crate. Call `to_string()` once, or
//! keep a caller-owned cache (for example an LRU), if a value is printed
//! repeatedly. C-like pass-through results do not re-render.
//!
//! # Thread safety
//!
//! C++, Rust and Swift retain `Rc`-owned node graphs, so [`Demangled`] is
//! `!Send + !Sync`. Demangle again on the target thread instead of moving a
//! value across threads.
//!
//! # Threat model
//!
//! The attack surface is untrusted symbol text or bytes passed to [`Demangler`]
//! and to the backend `parse` entry points, which take raw bytes. Parsers enforce the input,
//! depth, work and node limits they have and build acyclic immutable trees;
//! printers have no caps, because a retained tree already fits its parse-time
//! bounds. The bounded-work and printability
//! guarantees in this crate apply to those parser-produced ASTs.
//!
//! ASTs assembled by callers are outside the threat model. The AST and node
//! constructors are crate-private today, but a caller-built (or externally
//! mutated) tree is trusted: the crate does not defend it against cycles,
//! missing children, budget overruns or printer failure. A failing
//! [`core::fmt::Write`] sink is likewise a caller concern and remains part of
//! [`core::fmt::Display`] for every input.
//!
//! # Budgets
//!
//! Every parsing backend bounds untrusted input with compiled-in limits on the
//! dimensions it has: input bytes, nesting depth, work units and allocated
//! nodes. [`Budget`] carries caller
//! overrides, [`Demangler::effective_budget`] reports the clamped values a
//! backend would use for one [`Scheme`], and [`Scheme::default_budget`] /
//! [`Scheme::hard_budget`] expose the compiled-in defaults and the per-scheme
//! ceilings. Every dimension is a `usize`.
//!
//! `None` keeps the backend default, so a [`Demangler`] built with
//! [`Budget::default`] behaves exactly as before; a value is clamped to the
//! scheme's hard cap. Every backend enforces the resolved overrides. Printing
//! is not a budget. Shared-DAG printers (Rust, Swift and C++) are gated by a
//! parse-time envelope; linear printers visit each owned node once, so the
//! parser budgets already bound their output and no [`Budget`] field configures
//! a printer.
//!
//! [`Budget`] also carries two parse-time expansion gates,
//! `expansion_visits` and `expansion_bytes`, for the schemes whose spelling can
//! expand: the shared-DAG backends (Rust, Swift and C++) and D. They are
//! reported by [`Demangler::effective_budget`] with the same clamping, and the linear
//! backends report `None` for both. Every expanding backend enforces them at
//! parse time; they are AST/envelope gates, never printer caps. No backend
//! keeps a fixed limit outside these dimensions (see [`Budget`]).
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
//! return the same [`Error::InvalidSymbol`] a detected name would. A scheme is
//! exact: a C++ dialect, rustc encoding or Swift mangling the parser did not
//! produce is rejected instead of being retagged. This is the integration point
//! for names whose automatic detection is deliberately conservative: general
//! Ada external names (`pkg__subp`), gfortran external names (`foo_`),
//! namespaced Free Pascal units and OCaml external names.
//!
//! # Selecting schemes and languages
//!
//! [`Scheme`] names one exact grammar or encoding; [`Language`] groups the
//! schemes of one source language. Detection reports the precise scheme. When
//! a symbol is ambiguous, [`Demangler::selection`] builds an allocation-free,
//! ordered candidate list:
//!
//! ```rust
//! use demangler::{Demangler, Language, Scheme};
//!
//! let demangler = Demangler::new();
//! let demangled = demangler
//!     .selection()
//!     .allow(Language::C)
//!     .demangle("main")
//!     .unwrap();
//! assert_eq!(demangled.scheme(), Scheme::C);
//! assert_eq!(demangled.to_string(), "main");
//! ```
//!
//! An empty selection is exactly [`Demangler::demangle`], including its
//! detected C++ to Rust fallback. Every selected candidate is exact, and
//! [`Error::NoMatchingScheme`] reports that several candidates were tried and
//! all of them failed.
//!
//! # Platform decorations
//!
//! Two platform conventions are orthogonal to the language mangling and are
//! decoded only by an explicit entry point, never by detection:
//!
//! - **PE/COFF**: `coff::parse_for` decodes the decoration rules of a
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
//! never transcodes those bytes on its own. With the `encoding` feature,
//! `Demangled::to_string_with_encoding` decodes the escapes as UTF-8,
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

mod budget;

/// C++ ABI parsers and inspectable syntax tree.
///
/// Enable all dialects with `cpp`, or select `cpp-itanium`, `cpp-msvc` and/or
/// `cpp-gnu-v2` for a lighter build.
#[cfg(any(feature = "cpp-itanium", feature = "cpp-msvc", feature = "cpp-gnu-v2"))]
pub mod cpp;

mod demangler;

mod detection;
mod parse;

mod error;

#[cfg(feature = "encoding")]
mod encoding;

mod result;

/// Rust mangling parser and printer (pure Rust, `no_std` + `alloc`).
#[cfg(feature = "rust")]
pub mod rust;

mod scheme;

mod selection;

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

pub use budget::{Budget, EffectiveBudget, Limit};
pub use demangler::Demangler;
#[cfg(feature = "encoding")]
pub use encoding::{Encoding, EncodingError};
pub use error::{BudgetKind, Error, ParseError, ParseErrorKind};
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
    feature = "fortran",
    feature = "delphi",
    feature = "ocaml"
))]
pub use result::AstRef;
pub use result::Demangled;
pub use scheme::{Language, Scheme};
pub use selection::{Selection, Selector};

#[cfg(test)]
mod tests;
