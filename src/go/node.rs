//! Go symbol name model, budgets and escape decoding.
//!
//! A Go linker name is not an encoding: it is the package path, a `.`
//! separator and the local name, with compiler-generated decorations appended.
//! The model therefore mirrors the name's own structure instead of inventing a
//! source-level syntax.
//!
//! Every parser allocation is proportional to the input: the suffix list is
//! capped by [`MAX_SUFFIXES`] and the input by [`MAX_INPUT_LEN`].

use alloc::borrow::Cow;
use alloc::string::String;
use alloc::vec::Vec;

/// Maximum accepted input length.
///
/// Go symbol names embed type strings, so they are longer than C or Rust
/// names, but real ones stay far below this cap. Bounding the input bounds
/// every parser allocation and the printed output, which is never longer than
/// the input.
pub const MAX_INPUT_LEN: usize = 64 * 1024;

/// Maximum nesting depth of `()`, `[]` and `{}` inside a receiver or generic
/// argument list.
pub const MAX_DEPTH: usize = 32;

/// Maximum number of parser work units (bytes scanned).
///
/// A well-formed name costs a small constant number of scans per byte; the
/// budget rejects inputs that force repeated rescans.
pub const MAX_WORK: usize = 8 * MAX_INPUT_LEN;

/// Maximum number of function suffix components (closures, wrappers, inlined
/// names) attached to one symbol.
pub const MAX_SUFFIXES: usize = 32;

/// A parsed Go symbol name plus the ABI marker stripped from it.
#[derive(Debug)]
pub struct Ast<'a> {
    pub(crate) name: Name<'a>,
    pub(crate) abi: Option<Abi>,
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
    Shape(&'a str),
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
    raw: &'a str,
}

impl<'a> Package<'a> {
    pub(crate) const fn new(raw: &'a str) -> Self {
        Self { raw }
    }

    /// The package path as it appears in the symbol, still escaped.
    #[must_use]
    pub const fn raw(self) -> &'a str {
        self.raw
    }

    /// The package path with `%XX` escapes decoded.
    ///
    /// Returns the raw path unchanged when an escape does not decode to valid
    /// UTF-8; Go import paths are valid UTF-8, so this only affects malformed
    /// input.
    #[must_use]
    pub fn decoded(self) -> Cow<'a, str> {
        unescape(self.raw)
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
        suffix: &'a str,
    },
    /// `..inittask`: the package initialisation task.
    InitTask,
    /// `..dict.<type>`: a generic dictionary.
    Dictionary {
        /// The type string the dictionary was built for.
        ty: &'a str,
    },
    /// `..interfaceSwitch.<n>`: an interface switch table.
    InterfaceSwitch {
        /// The table index.
        index: &'a str,
    },
    /// `..typeAssert.<n>`: a type assertion descriptor.
    TypeAssert {
        /// The descriptor index.
        index: &'a str,
    },
    /// `..gobytes.<n>`: read-only bytes backing a constant.
    GoBytes {
        /// The constant index.
        index: &'a str,
    },
    /// A compiler data symbol such as `gcbits.0100` or `gclocals.<base64>`.
    Data(DataName<'a>),
}

/// A compiler data symbol: an identifier and an opaque payload.
#[derive(Debug, Clone, Copy)]
pub struct DataName<'a> {
    /// The symbol's identifier, e.g. `gcbits`.
    pub name: &'a str,
    /// The payload after the separator, e.g. `0100` or `*`.
    pub payload: &'a str,
}

/// A function, method, closure or wrapper.
#[derive(Debug)]
pub struct Function<'a> {
    pub(crate) receiver: Option<Receiver<'a>>,
    pub(crate) name: &'a str,
    pub(crate) suffixes: Vec<Suffix<'a>>,
}

impl<'a> Function<'a> {
    /// The method receiver, for methods and method wrappers.
    #[must_use]
    pub const fn receiver(&self) -> Option<&Receiver<'a>> {
        self.receiver.as_ref()
    }

    /// The function name, including generic arguments when instantiated.
    #[must_use]
    pub const fn name(&self) -> &'a str {
        self.name
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
    pub ty: &'a str,
}

/// A compiler-generated component appended to a function name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Suffix<'a> {
    /// `.funcN`: a function literal.
    Closure(&'a str),
    /// `.gowrapN`: a `go` statement wrapper.
    GoWrap(&'a str),
    /// `.deferwrapN`: a `defer` statement wrapper.
    DeferWrap(&'a str),
    /// `-rangeN`: a range-over-func wrapper.
    Range(&'a str),
    /// `-fm`: a method value wrapper.
    MethodValue,
    /// `·f`: a function value symbol.
    FuncValue,
    /// `.N`: a nested closure index.
    Nested(&'a str),
    /// `.name`: an inlined function name.
    Inlined(&'a str),
    /// `.arginfo1<1>`, `.stkobj` and other compiler-internal suffixes.
    Internal(&'a str),
}

/// A `type:` runtime type descriptor.
#[derive(Debug)]
pub struct TypeDescriptor<'a> {
    pub(crate) kind: TypeKind,
    pub(crate) payload: &'a str,
}

impl<'a> TypeDescriptor<'a> {
    /// The descriptor kind, i.e. the tag between `type:` and the payload.
    #[must_use]
    pub const fn kind(&self) -> TypeKind {
        self.kind
    }

    /// The type string or name after the kind tag, still escaped.
    #[must_use]
    pub const fn payload(&self) -> &'a str {
        self.payload
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
    /// `.hash.`: a hash function.
    Hash,
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
            Self::Hash => ".hash.",
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
        concrete: &'a str,
        /// The interface type string.
        iface: &'a str,
    },
    /// `go:string.<payload>`: string data named by its quoted contents.
    String {
        /// The quoted or `.gostring.` payload, opaque.
        payload: &'a str,
    },
    /// Any other `go:` symbol: `go:buildid`, `go:info.<type>`, `go:func.*`, …
    Other {
        /// The kind between `go:` and the first `.`.
        kind: &'a str,
        /// The rest of the name, empty when there is no `.`.
        payload: &'a str,
    },
}

/// A `$f32.`/`$f64.`/`$i32.`/`$i64.`/`$i128.` compiler constant.
#[derive(Debug)]
pub struct Constant<'a> {
    pub(crate) class: ConstantClass,
    pub(crate) value: &'a str,
}

impl<'a> Constant<'a> {
    /// The constant's class.
    #[must_use]
    pub const fn class(&self) -> ConstantClass {
        self.class
    }

    /// The hexadecimal bits, plus any architecture suffix.
    #[must_use]
    pub const fn value(&self) -> &'a str {
        self.value
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

/// Decode `%XX` escapes, as produced by Go's `objabi.PathToPrefix`.
///
/// A `%` that is not followed by two hex digits is kept verbatim, and so is
/// the whole input when the decoded bytes are not valid UTF-8.
pub(crate) fn unescape(raw: &str) -> Cow<'_, str> {
    if !raw.as_bytes().contains(&b'%') {
        return Cow::Borrowed(raw);
    }

    let bytes = raw.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && let (Some(high), Some(low)) = (
                bytes.get(i + 1).and_then(|byte| hex_value(*byte)),
                bytes.get(i + 2).and_then(|byte| hex_value(*byte)),
            )
        {
            decoded.push((high << 4) | low);
            i += 3;
            continue;
        }
        decoded.push(bytes[i]);
        i += 1;
    }

    String::from_utf8(decoded).map_or(Cow::Borrowed(raw), Cow::Owned)
}

/// The value of one hexadecimal digit.
fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use alloc::string::ToString;

    use super::*;

    #[test]
    fn decodes_path_escapes() {
        assert_eq!(unescape("pkg%2ev1"), "pkg.v1");
        assert_eq!(unescape("a%2fb"), "a/b");
        assert_eq!(unescape("%2e%2e"), "..");
        assert_eq!(unescape("%25"), "%");
    }

    #[test]
    fn keeps_malformed_escapes() {
        assert_eq!(unescape("100%"), "100%");
        assert_eq!(unescape("%zz"), "%zz");
        assert_eq!(unescape("%2"), "%2");
        assert_eq!(unescape("a%2"), "a%2");
    }

    #[test]
    fn borrows_when_unchanged() {
        assert!(matches!(unescape("fmt.Println"), Cow::Borrowed(_)));
        assert!(matches!(unescape("%2e"), Cow::Owned(_)));
    }

    #[test]
    fn abi_suffixes_round_trip() {
        for abi in [Abi::Abi0, Abi::Internal, Abi::Wrap] {
            assert!(abi.suffix().starts_with('.'));
        }
    }

    #[test]
    fn package_decodes() {
        let pkg = Package::new("example.com/x%2ey");
        assert_eq!(pkg.raw(), "example.com/x%2ey");
        assert_eq!(pkg.decoded().to_string(), "example.com/x.y");
    }
}
