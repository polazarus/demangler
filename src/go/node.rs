//! Go symbol name model, budgets and escape decoding.
//!
//! A Go linker name is not an encoding: it is the package path, a `.`
//! separator and the local name, with compiler-generated decorations appended.
//! The model therefore mirrors the name's own structure instead of inventing a
//! source-level syntax.
//!
//! Every parser allocation is proportional to the input: each suffix consumes
//! input, so the input budget ([`MAX_INPUT_LEN`] by default) bounds the suffix
//! list.

use alloc::borrow::Cow;
use alloc::vec::Vec;

use super::text::{decoded, text};
use crate::budget::{EffectiveBudget, Limit};
use crate::parse::{Escape, Limits};

/// Maximum accepted input length.
///
/// Go symbol names embed type strings, so they are longer than C or Rust
/// names, but real ones stay far below this cap. Bounding the input bounds
/// every parser allocation and the printed output, which is at most four times
/// the input (a byte that is not valid UTF-8 prints as `\xNN`).
///
/// This is the default a [`crate::Budget`] starts from; a [`crate::Demangler`]
/// may raise it up to `HARD_INPUT_LEN`.
pub const MAX_INPUT_LEN: usize = 64 * 1024;

/// Maximum nesting depth of `()`, `[]` and `{}` inside a receiver or generic
/// argument list.
///
/// This is the default a [`crate::Budget`] starts from; a [`crate::Demangler`]
/// may raise it up to `HARD_DEPTH`.
pub const MAX_DEPTH: usize = 32;

/// Maximum number of parser work units (bytes scanned).
///
/// A well-formed name costs a small constant number of scans per byte; the
/// budget rejects inputs that force repeated rescans. This is the default a
/// [`crate::Budget`] starts from; a [`crate::Demangler`] may raise it up to
/// `HARD_WORK`.
pub const MAX_WORK: usize = 8 * MAX_INPUT_LEN;

/// Hard input ceiling.
///
/// The parser only borrows the input, but the printed text grows with it (a
/// byte that is not valid UTF-8 prints as `\xNN`), so a raise stays bounded
/// whatever the caller asks for.
pub(crate) const HARD_INPUT_LEN: usize = 16 * 1024 * 1024;

/// Hard depth ceiling.
///
/// A balanced group is scanned iteratively, so the ceiling bounds the closer
/// stack rather than the machine stack.
pub(crate) const HARD_DEPTH: usize = 1024;

/// Hard work ceiling.
pub(crate) const HARD_WORK: usize = 64 * 1024 * 1024;

/// The input, depth and work limits one parse enforces.
///
/// Resolved from a [`crate::Budget`]: an absent dimension keeps its compiled-in
/// default and every value is clamped to this module's hard ceiling. A Go name
/// is one classified symbol with a bounded suffix list, so there is no node
/// dimension to resolve.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ParseLimits {
    /// Maximum accepted input length in bytes.
    pub(crate) input: usize,
    /// Maximum nesting depth of a balanced group.
    pub(crate) depth: usize,
    /// Maximum parser work units.
    pub(crate) work: usize,
}

impl ParseLimits {
    /// Resolve one scheme's budget for a parse of a Go symbol name.
    pub(crate) fn from_effective(budget: EffectiveBudget) -> Self {
        Self {
            input: clamp(budget.input_bytes, MAX_INPUT_LEN, HARD_INPUT_LEN),
            depth: clamp(budget.parse_depth, MAX_DEPTH, HARD_DEPTH),
            work: clamp(budget.work, MAX_WORK, HARD_WORK),
        }
    }

    /// Limits handed to the shared byte cursor.
    ///
    /// The input and work values are clamped to the hard ceilings, so both fit
    /// the cursor's `u32` fields, and the depth fits its `u16` field.
    pub(crate) const fn cursor(&self) -> Limits {
        Limits {
            input: self.input as u32,
            depth: self.depth as u16,
            work: self.work as u32,
        }
    }
}

/// One budget dimension after clamping to the compiled-in default and ceiling.
fn clamp(dimension: Option<Limit>, default: usize, ceiling: usize) -> usize {
    match dimension {
        Some(Limit::Bounded(value)) => value.min(ceiling),
        Some(Limit::Unbounded) | None => default.min(ceiling),
    }
}

/// A parsed Go symbol name plus the ABI marker stripped from it.
#[derive(Debug)]
pub struct Ast<'a> {
    pub(crate) name: Name<'a>,
    pub(crate) abi: Option<Abi>,
    pub(crate) version: Option<&'a [u8]>,
}

impl<'a> Ast<'a> {
    /// The classified symbol name.
    #[must_use]
    pub const fn name(&self) -> &Name<'a> {
        &self.name
    }

    /// The ABI marker removed from the symbol, if it had one.
    ///
    /// ABI markers (`.abi0`, `.abiinternal`, `.abiwrap`) are linker plumbing
    /// and are not part of the printed name.
    #[must_use]
    pub const fn abi(&self) -> Option<Abi> {
        self.abi
    }

    /// The object-file symbol version (`<N>`), if the name carried one.
    ///
    /// `go tool nm` appends `<1>` to static symbols read from a compiler
    /// object file or export archive (see `goobjName` in
    /// `cmd/internal/objfile`). The marker is kept in the printed name, as it
    /// distinguishes static symbols from their text counterparts.
    #[must_use]
    pub fn version(&self) -> Option<Cow<'a, str>> {
        self.version
            .map(|version| text(version, Escape::Identifier))
    }
}

/// The ABI marker a symbol carried.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Abi {
    /// `.abi0`: the stack-based ABI used by assembly.
    Abi0,
    /// `.abiinternal`: the linker-internal ABI wrapper.
    Internal,
    /// `.abiwrap`: the ABI wrapper symbol.
    Wrap,
}

impl Abi {
    /// The suffix as it appears in the symbol name.
    #[must_use]
    pub const fn suffix(self) -> &'static str {
        match self {
            Self::Abi0 => ".abi0",
            Self::Internal => ".abiinternal",
            Self::Wrap => ".abiwrap",
        }
    }
}

/// A classified Go symbol name.
#[derive(Debug)]
pub enum Name<'a> {
    /// A package-qualified or package-local symbol.
    Symbol(Symbol<'a>),
    /// A `type:` runtime type descriptor.
    TypeDescriptor(TypeDescriptor<'a>),
    /// A `go:` linker special symbol.
    Special(Special<'a>),
    /// A `go.shape.` generic shape type.
    Shape(&'a [u8]),
    /// A `$f64.`/`$i64.` compiler constant.
    Constant(Constant<'a>),
}

/// A package-qualified or package-local symbol.
#[derive(Debug)]
pub struct Symbol<'a> {
    pub(crate) package: Option<Package<'a>>,
    pub(crate) local: Local<'a>,
}

impl<'a> Symbol<'a> {
    /// The package path, when the symbol is package-qualified.
    #[must_use]
    pub const fn package(&self) -> Option<&Package<'a>> {
        self.package.as_ref()
    }

    /// The local part of the symbol.
    #[must_use]
    pub const fn local(&self) -> &Local<'a> {
        &self.local
    }
}

/// An escaped package path exactly as written in the symbol.
#[derive(Debug, Clone, Copy)]
pub struct Package<'a> {
    raw: &'a [u8],
}

impl<'a> Package<'a> {
    pub(crate) const fn new(raw: &'a [u8]) -> Self {
        Self { raw }
    }

    /// The package path as it appears in the symbol, still escaped.
    ///
    /// A path is ASCII, so this is the raw spelling; see [`Package::decoded`]
    /// for the readable form.
    #[must_use]
    pub fn raw(self) -> Cow<'a, str> {
        text(self.raw, Escape::Identifier)
    }

    /// The package path with `%XX` escapes decoded.
    ///
    /// Decoded bytes that are not valid UTF-8 are escaped as `\xNN`; Go
    /// import paths are valid UTF-8, so this only affects malformed input.
    #[must_use]
    pub fn decoded(self) -> Cow<'a, str> {
        decoded(self.raw)
    }
}

/// The local part of a symbol.
#[derive(Debug)]
pub enum Local<'a> {
    /// A function, method, closure or compiler-generated function wrapper.
    Function(Function<'a>),
    /// A `glob..funcN` package-scope closure.
    GlobalClosure {
        /// The component after `glob..`, e.g. `func1`.
        suffix: &'a [u8],
    },
    /// `..inittask`: the package initialisation task.
    InitTask,
    /// `..dict.<type>`: a generic dictionary.
    Dictionary {
        /// The type string the dictionary was built for.
        ty: &'a [u8],
    },
    /// `..interfaceSwitch.<n>`: an interface switch table.
    InterfaceSwitch {
        /// The table index.
        index: &'a [u8],
    },
    /// `..typeAssert.<n>`: a type assertion descriptor.
    TypeAssert {
        /// The descriptor index.
        index: &'a [u8],
    },
    /// `..gobytes.<n>`: read-only bytes backing a constant.
    GoBytes {
        /// The constant index.
        index: &'a [u8],
    },
    /// `..stmp_<n>`: a static temporary variable (`obj.StaticNamePrefix`).
    StaticTemp {
        /// The variable index.
        index: &'a [u8],
    },
    /// A compiler data symbol such as `gcbits.0100` or `gclocals.<base64>`.
    Data(DataName<'a>),
}

/// A compiler data symbol: an identifier and an opaque payload.
#[derive(Debug, Clone, Copy)]
pub struct DataName<'a> {
    /// The symbol's identifier, e.g. `gcbits`.
    pub name: &'a [u8],
    /// The payload after the separator, e.g. `0100` or `*`.
    pub payload: &'a [u8],
}

/// A function, method, closure or wrapper.
#[derive(Debug)]
pub struct Function<'a> {
    pub(crate) receiver: Option<Receiver<'a>>,
    pub(crate) name: &'a [u8],
    pub(crate) suffixes: Vec<Suffix<'a>>,
}

impl<'a> Function<'a> {
    /// The method receiver, for methods and method wrappers.
    #[must_use]
    pub const fn receiver(&self) -> Option<&Receiver<'a>> {
        self.receiver.as_ref()
    }

    /// The function name, including generic arguments when instantiated.
    ///
    /// `%XX` escapes are decoded; bytes that are not valid UTF-8 are escaped
    /// as `\xNN`.
    #[must_use]
    pub fn name(&self) -> Cow<'a, str> {
        decoded(self.name)
    }

    /// The compiler-generated components appended to the name.
    #[must_use]
    pub fn suffixes(&self) -> &[Suffix<'a>] {
        &self.suffixes
    }
}

/// A method receiver.
#[derive(Debug, Clone, Copy)]
pub struct Receiver<'a> {
    /// True for pointer receivers, written `(*T)`.
    pub pointer: bool,
    /// The receiver type string.
    pub ty: &'a [u8],
}

/// A compiler-generated component appended to a function name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Suffix<'a> {
    /// `.funcN`: a function literal.
    Closure(&'a [u8]),
    /// `.gowrapN`: a `go` statement wrapper.
    GoWrap(&'a [u8]),
    /// `.deferwrapN`: a `defer` statement wrapper.
    DeferWrap(&'a [u8]),
    /// `-rangeN`: a range-over-func wrapper.
    Range(&'a [u8]),
    /// `-fm`: a method value wrapper.
    MethodValue,
    /// `·f`: a function value symbol.
    FuncValue,
    /// `.N`: a nested closure index.
    Nested(&'a [u8]),
    /// `.name`: an inlined function name.
    Inlined(&'a [u8]),
    /// `.jumpN`: a jump table for the `N`th SSA block (`makeJumpTableSym`).
    JumpTable(&'a [u8]),
    /// `.arginfo1<1>`, `.stkobj` and other compiler-internal suffixes.
    Internal(&'a [u8]),
}

/// A `type:` runtime type descriptor.
#[derive(Debug)]
pub struct TypeDescriptor<'a> {
    pub(crate) kind: TypeKind,
    pub(crate) payload: &'a [u8],
}

impl<'a> TypeDescriptor<'a> {
    /// The descriptor kind, i.e. the tag between `type:` and the payload.
    #[must_use]
    pub const fn kind(&self) -> TypeKind {
        self.kind
    }

    /// The type string or name after the kind tag.
    ///
    /// `%XX` escapes are decoded; bytes that are not valid UTF-8 are escaped
    /// as `\xNN`.
    #[must_use]
    pub fn payload(&self) -> Cow<'a, str> {
        decoded(self.payload)
    }
}

/// The kind of a `type:` descriptor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TypeKind {
    /// No tag: a plain type descriptor such as `type:int`.
    Plain,
    /// `*`: a pointer type descriptor, or the `type:*` group symbol.
    Pointer,
    /// `.eq.`: an equality function.
    Equal,
    /// `.eqfunc`: an equality closure (`type:.eqfunc<N>` or
    /// `type:.eqfunc.<type>`).
    EqualFunc,
    /// `.hash.`: a hash function.
    Hash,
    /// `.hashfunc`: a hash closure (`type:.hashfunc<N>` or
    /// `type:.hashfunc.<type>`).
    HashFunc,
    /// `.gcprog.`: a GC program.
    GcProg,
    /// `.namedata.`: a `reflect.name` blob.
    NameData,
    /// `.importpath.`: a package path blob.
    ImportPath,
    /// `noalg.`: a type without an algorithm.
    NoAlg,
}

impl TypeKind {
    /// The tag as it appears after `type:`.
    #[must_use]
    pub const fn prefix(self) -> &'static str {
        match self {
            Self::Plain => "",
            Self::Pointer => "*",
            Self::Equal => ".eq.",
            Self::EqualFunc => ".eqfunc",
            Self::Hash => ".hash.",
            Self::HashFunc => ".hashfunc",
            Self::GcProg => ".gcprog.",
            Self::NameData => ".namedata.",
            Self::ImportPath => ".importpath.",
            Self::NoAlg => "noalg.",
        }
    }

    /// True when the payload is opaque and must not be unescaped.
    ///
    /// `reflect.name` blobs carry a struct tag, which may contain `%` and
    /// arbitrary bytes; decoding them would corrupt the tag.
    #[must_use]
    pub const fn is_opaque(self) -> bool {
        matches!(self, Self::NameData)
    }
}

/// A `go:` linker special symbol.
#[derive(Debug)]
pub enum Special<'a> {
    /// `go:itab.<concrete>,<interface>`: an interface table.
    Itab {
        /// The concrete type string.
        concrete: &'a [u8],
        /// The interface type string.
        iface: &'a [u8],
    },
    /// `go:string.<payload>`: string data named by its quoted contents.
    String {
        /// The quoted or `.gostring.` payload, opaque.
        payload: &'a [u8],
    },
    /// Any other `go:` symbol: `go:buildid`, `go:info.<type>`, `go:func.*`, …
    Other {
        /// The kind between `go:` and the first `.`.
        kind: &'a [u8],
        /// The rest of the name, empty when there is no `.`.
        payload: &'a [u8],
    },
}

/// A `$f32.`/`$f64.`/`$i32.`/`$i64.`/`$i128.` compiler constant.
#[derive(Debug)]
pub struct Constant<'a> {
    pub(crate) class: ConstantClass,
    pub(crate) value: &'a [u8],
}

impl<'a> Constant<'a> {
    /// The constant's class.
    #[must_use]
    pub const fn class(&self) -> ConstantClass {
        self.class
    }

    /// The hexadecimal bits, plus any architecture suffix.
    #[must_use]
    pub fn value(&self) -> Cow<'a, str> {
        text(self.value, Escape::Identifier)
    }
}

/// The class of a compiler constant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ConstantClass {
    /// `$f32.`: a 32-bit float.
    F32,
    /// `$f64.`: a 64-bit float.
    F64,
    /// `$i32.`: a 32-bit integer.
    I32,
    /// `$i64.`: a 64-bit integer.
    I64,
    /// `$i128.`: a 128-bit integer.
    I128,
}

impl ConstantClass {
    /// The prefix as it appears after `$`.
    #[must_use]
    pub const fn prefix(self) -> &'static str {
        match self {
            Self::F32 => "f32.",
            Self::F64 => "f64.",
            Self::I32 => "i32.",
            Self::I64 => "i64.",
            Self::I128 => "i128.",
        }
    }
}

#[cfg(test)]
mod tests {
    use alloc::string::ToString;

    use super::*;

    #[test]
    fn abi_suffixes_round_trip() {
        for abi in [Abi::Abi0, Abi::Internal, Abi::Wrap] {
            assert!(abi.suffix().starts_with('.'));
        }
    }

    #[test]
    fn package_decodes() {
        let pkg = Package::new(b"example.com/x%2ey");
        assert_eq!(pkg.raw(), "example.com/x%2ey");
        assert_eq!(pkg.decoded().to_string(), "example.com/x.y");
    }

    #[test]
    fn package_escapes_invalid_bytes() {
        let pkg = Package::new(b"example.com/x%ffy");
        assert_eq!(pkg.raw(), "example.com/x%ffy");
        assert_eq!(pkg.decoded(), "example.com/x\\xffy");
    }
}
