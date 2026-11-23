//! OCaml symbol model and budgets.
//!
//! An OCaml native linker name carries no type information: it is the
//! compilation-unit path, an optional member name and a numeric stamp that
//! makes the name unique. This module mirrors exactly that structure, plus the
//! compiler's static-data labels and the runtime-helper forms the compiler and
//! the runtime share the `caml` prefix with.

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
/// Printing drops the `caml` prefix, the stamp and a host-compiler fragment
/// suffix, and decodes `$XX` escapes, so the printed text is never longer than
/// the input; four times the input bounds the escaping fallback for
/// hand-written symbols.
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
    /// `caml<path>.<label>`: a compiler-emitted static-data label.
    Label {
        /// Compilation-unit path as mangled, `.`-separated.
        path: &'a [u8],
        /// Decimal label as emitted, without a name of its own.
        label: &'a [u8],
    },
    /// `caml_<name>`: a runtime helper, printed as it stands. A host-compiler
    /// fragment suffix (`.cold`, `.part.<N>`) is not part of the name and is
    /// dropped while parsing.
    Helper {
        /// The whole helper name, `caml_` included.
        symbol: &'a [u8],
    },
    /// A section marker of a runtime pseudo-unit: `caml_<unit>.<marker>` or the
    /// assembler's `caml_<unit>__<marker>` spelling.
    Marker {
        /// Pseudo-unit name without the `caml_` prefix (`startup`, `hot`, …).
        unit: &'a [u8],
        /// Section marker (`code_begin`, `frametable`, …).
        marker: &'a [u8],
        /// How the linker name joins the two components.
        join: MarkerJoin,
    },
}

/// How a runtime pseudo-unit's name and its section marker are joined.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MarkerJoin {
    /// `caml_<unit>.<marker>`: the compiler's spelling.
    Dot,
    /// `caml_<unit>__<marker>`: the assembler's doubled-underscore spelling.
    DoubleUnderscore,
}

impl MarkerJoin {
    /// The separator the linker name uses.
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Dot => ".",
            Self::DoubleUnderscore => "__",
        }
    }
}
