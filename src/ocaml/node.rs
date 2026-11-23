//! OCaml symbol model and budgets.
//!
//! An OCaml native linker name carries no type information: it is the
//! compilation-unit path, an optional member name and a numeric stamp that
//! makes the name unique. This module mirrors exactly that structure, plus the
//! runtime-helper form the compiler and the runtime share the `caml` prefix
//! with.

/// Maximum accepted input length.
pub const MAX_INPUT_LEN: usize = 64 * 1024;

/// Maximum number of module-path segments in one name.
///
/// A pack (`-pack`) nests one segment per packed unit, so this is also the
/// parser's node budget for module paths.
pub const MAX_PATH_SEGMENTS: usize = 64;

/// Maximum number of parser work units (bytes scanned).
pub const MAX_WORK: usize = 8 * MAX_INPUT_LEN;

/// Maximum printed output length.
///
/// Printing drops the `caml` prefix and the stamp and decodes `$XX` escapes, so
/// the printed text is never longer than the input; four times the input bounds
/// the escaping fallback for hand-written symbols.
pub const MAX_OUTPUT_LEN: usize = 4 * MAX_INPUT_LEN;

/// A parsed OCaml linker name.
#[derive(Debug)]
pub struct Ast<'a> {
    pub(crate) form: Form<'a>,
}

/// Which `caml` form a symbol uses.
#[derive(Debug)]
pub(crate) enum Form<'a> {
    /// `caml<path>`: the module record of a compilation unit or pack.
    Module {
        /// Compilation-unit path as mangled, `.`-separated.
        path: &'a [u8],
    },
    /// `caml<path>.<member>`: a value, an operator or a compiler marker.
    Member {
        /// Compilation-unit path as mangled, `.`-separated.
        path: &'a [u8],
        /// Member as mangled, stamp removed and `$XX` escapes kept.
        member: &'a [u8],
    },
    /// `caml_<name>`: a runtime helper, printed as it stands.
    Helper {
        /// The whole helper name, `caml_` included.
        symbol: &'a [u8],
    },
    /// `caml_<unit>.<marker>`: a marker of a runtime pseudo-unit.
    PseudoUnit {
        /// Pseudo-unit name without the `caml_` prefix (`startup`, `hot`, …).
        unit: &'a [u8],
        /// Section marker (`code_begin`, `frametable`, …).
        marker: &'a [u8],
    },
}
