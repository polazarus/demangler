//! D symbol model and budgets.
//!
//! The model mirrors the D ABI grammar: a mangled symbol is a qualified name
//! plus a type, and both names and non-basic types may be back references.
//! Identifier text borrows the input; lists and nested nodes allocate.

use alloc::boxed::Box;
use alloc::vec::Vec;

/// Maximum accepted input length.
pub const MAX_INPUT_LEN: usize = 64 * 1024;

/// Maximum nesting depth of types, qualified names and template arguments.
pub const MAX_DEPTH: usize = 64;

/// Maximum number of parser work units (bytes scanned).
///
/// Back references re-scan an earlier part of the symbol, so the work budget
/// is larger than the input.
pub const MAX_WORK: usize = 16 * MAX_INPUT_LEN;

/// Maximum number of parsed nodes.
pub const MAX_NODES: usize = 4 * MAX_INPUT_LEN;

/// Maximum printed output length.
///
/// This is four times the maximum input length. Back references can expand an
/// earlier name, so the ceiling is absolute rather than a per-input ratio; the
/// printer enforces it and reports a formatting error instead of truncating.
pub const MAX_OUTPUT_LEN: usize = 4 * MAX_INPUT_LEN;

/// The two top-level D symbol forms.
#[derive(Debug)]
pub(crate) enum Root<'a> {
    /// The runtime entry point `_Dmain`.
    Main,
    /// A qualified name followed by a type or `Z`.
    Symbol(Symbol<'a>),
}

/// A qualified name: dot-separated components, each optionally carrying a
/// function signature.
#[derive(Debug)]
pub(crate) struct Symbol<'a> {
    pub(crate) components: Vec<Component<'a>>,
}

/// One `SymbolFunctionName` component.
#[derive(Debug)]
pub(crate) struct Component<'a> {
    pub(crate) name: NameExpr<'a>,
    pub(crate) function: Option<FunctionSig<'a>>,
}

/// A function signature encoded directly after a name.
#[derive(Debug)]
pub(crate) struct FunctionSig<'a> {
    pub(crate) params: Vec<Parameter<'a>>,
    pub(crate) variadic: Variadic,
    pub(crate) modifiers: Vec<Modifier>,
    /// The signature was a back reference to a complete function type, whose
    /// return type is therefore already part of the encoded symbol and no
    /// trailing type follows it.
    pub(crate) ret_consumed: bool,
}

/// A symbol name: a plain identifier, a template instance or a compiler
/// special.
#[derive(Debug)]
pub(crate) enum NameExpr<'a> {
    /// A length-prefixed identifier.
    Plain(&'a [u8]),
    /// `__ctor`.
    This,
    /// `__dtor`.
    Destructor,
    /// `__postblitMFZ`.
    Postblit,
    /// `__initZ`.
    Initializer,
    /// `__vtblZ`.
    Vtable,
    /// `__ClassZ`.
    ClassInfo,
    /// `__InterfaceZ`.
    Interface,
    /// `__ModuleInfoZ`.
    ModuleInfo,
    /// `__T`/`__U` template instance.
    Template {
        name: Box<Self>,
        args: Vec<TemplateArg<'a>>,
    },
}

/// One template argument.
#[derive(Debug)]
pub(crate) enum TemplateArg<'a> {
    /// `T Type`.
    Type(Type<'a>),
    /// `V Type Value`.
    Value { value: Value<'a> },
    /// `S QualifiedName`.
    Symbol(Symbol<'a>),
    /// `X Number ExternallyMangledName`.
    External(&'a [u8]),
}

/// A parsed D type.
#[derive(Debug)]
pub(crate) enum Type<'a> {
    /// A primitive type.
    Primitive(Primitive),
    /// `shared(T)`.
    Shared(Box<Self>),
    /// `const(T)`.
    Const(Box<Self>),
    /// `immutable(T)`.
    Immutable(Box<Self>),
    /// `inout(T)`.
    Inout(Box<Self>),
    /// `__vector(T)`.
    Vector(Box<Self>),
    /// `typeof(null)`.
    TypeofNull,
    /// `typeof(*null)`.
    TypeofStarNull,
    /// `T[]`.
    DynamicArray(Box<Self>),
    /// `T[N]`.
    StaticArray {
        element: Box<Self>,
        length: &'a [u8],
    },
    /// `V[K]`.
    AssocArray { key: Box<Self>, value: Box<Self> },
    /// `T*`.
    Pointer(Box<Self>),
    /// A function type.
    Function(FunctionType<'a>),
    /// A delegate type.
    Delegate {
        function: FunctionType<'a>,
        modifiers: Vec<Modifier>,
    },
    /// `C`/`S`/`E`/`T`/`I` followed by a qualified name.
    Named(Symbol<'a>),
    /// `B Number Types` (binutils spelling).
    Tuple(Vec<Self>),
}

/// A function type with its return type.
#[derive(Debug)]
pub(crate) struct FunctionType<'a> {
    pub(crate) convention: Convention,
    pub(crate) attrs: Vec<Attribute>,
    pub(crate) params: Vec<Parameter<'a>>,
    pub(crate) variadic: Variadic,
    pub(crate) ret: Box<Type<'a>>,
}

/// A function parameter.
#[derive(Debug)]
pub(crate) struct Parameter<'a> {
    pub(crate) prefix: ParameterPrefix,
    pub(crate) kind: ParameterKind,
    pub(crate) ty: Type<'a>,
}

/// The `scope`/`return` markers in front of a parameter.
///
/// The mangler emits both markers in either order: `NkM` for a `return scope`
/// parameter and `MNk` for a `scope return` one. The order is preserved so the
/// printed text follows the encoded order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ParameterPrefix {
    /// No marker.
    Plain,
    /// `M`.
    Scope,
    /// `Nk`.
    Return,
    /// `MNk`.
    ScopeReturn,
    /// `NkM`.
    ReturnScope,
}

/// The storage class of a parameter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ParameterKind {
    /// No storage class.
    Plain,
    /// `in`.
    In,
    /// `in ref`.
    InRef,
    /// `out`.
    Out,
    /// `ref`.
    Ref,
    /// `lazy`.
    Lazy,
}

/// The calling convention of a function type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Convention {
    /// `F`, the D convention.
    D,
    /// `U`, the C convention.
    C,
    /// `W`, the Windows convention.
    Windows,
    /// `V`, the Pascal convention.
    Pascal,
    /// `R`, the C++ convention.
    Cpp,
    /// `Y`, the Objective-C convention.
    ObjectiveC,
}

/// A function attribute.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Attribute {
    /// `Na`.
    Pure,
    /// `Nb`.
    Nothrow,
    /// `Nc`.
    Ref,
    /// `Nd`.
    Property,
    /// `Ne`.
    Trusted,
    /// `Nf`.
    Safe,
    /// `Ni`.
    Nogc,
    /// `Nj`.
    Return,
    /// `Nl`.
    Scope,
    /// `Nm`.
    Live,
}

/// The trailing variadic marker of a parameter list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Variadic {
    /// `Z`.
    None,
    /// `X`, `T t...`.
    Typesafe,
    /// `Y`, `T t, ...`.
    DStyle,
}

/// A type modifier printed as a suffix.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Modifier {
    /// `x`.
    Const,
    /// `y`.
    Immutable,
    /// `O`.
    Shared,
    /// `Ng`.
    Inout,
}

/// A primitive D type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Primitive {
    /// `void`.
    Void,
    /// `byte`.
    Byte,
    /// `ubyte`.
    Ubyte,
    /// `short`.
    Short,
    /// `ushort`.
    Ushort,
    /// `int`.
    Int,
    /// `uint`.
    Uint,
    /// `long`.
    Long,
    /// `ulong`.
    Ulong,
    /// `cent`.
    Cent,
    /// `ucent`.
    Ucent,
    /// `float`.
    Float,
    /// `double`.
    Double,
    /// `real`.
    Real,
    /// `ifloat`.
    Ifloat,
    /// `idouble`.
    Idouble,
    /// `ireal`.
    Ireal,
    /// `cfloat`.
    Cfloat,
    /// `cdouble`.
    Cdouble,
    /// `creal`.
    Creal,
    /// `bool`.
    Bool,
    /// `char`.
    Char,
    /// `wchar`.
    Wchar,
    /// `dchar`.
    Dchar,
}

/// A template value.
#[derive(Debug)]
pub(crate) enum Value<'a> {
    /// `null`.
    Null,
    /// An integral, character or boolean value.
    Integer {
        negative: bool,
        kind: IntegerKind<'a>,
    },
    /// A floating-point literal.
    Real(HexFloat<'a>),
    /// A complex literal.
    Complex { re: HexFloat<'a>, im: HexFloat<'a> },
    /// A string literal, with the raw hex payload retained.
    String { width: StringWidth, hex: &'a [u8] },
    /// An array literal.
    Array(Vec<Self>),
    /// An associative-array literal.
    AssocArray(Vec<(Self, Self)>),
    /// A struct literal.
    Struct {
        name: Option<Type<'a>>,
        fields: Vec<Self>,
    },
    /// A function symbol.
    Symbol(Symbol<'a>),
}

/// The kind of an integral value.
#[derive(Debug)]
pub(crate) enum IntegerKind<'a> {
    /// A character literal.
    Char(CharWidth, u64),
    /// A boolean literal.
    Bool(bool),
    /// A numeric literal with its type suffix.
    Number {
        digits: &'a [u8],
        suffix: IntegerSuffix,
    },
}

/// The width of a character literal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CharWidth {
    /// `a`, UTF-8.
    Utf8,
    /// `u`, UTF-16.
    Utf16,
    /// `w`, UTF-32.
    Utf32,
}

/// The width of a string literal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StringWidth {
    /// `a`, UTF-8.
    Utf8,
    /// `w`, UTF-16.
    Utf16,
    /// `d`, UTF-32.
    Utf32,
}

/// The suffix of a numeric literal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum IntegerSuffix {
    /// No suffix.
    None,
    /// `u`.
    U,
    /// `L`.
    L,
    /// `uL`.
    UL,
}

/// A hexadecimal floating-point literal.
#[derive(Debug)]
pub(crate) enum HexFloat<'a> {
    /// `NAN`.
    Nan,
    /// `INF`.
    Inf,
    /// `NINF`.
    NegInf,
    /// A finite hexadecimal literal.
    Number {
        negative: bool,
        first: u8,
        rest: &'a [u8],
        exponent_negative: bool,
        exponent: &'a [u8],
    },
}
