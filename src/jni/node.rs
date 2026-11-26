//! Java JNI native-method name model and budgets.
//!
//! A JNI symbol name is `Java_` followed by the mangled binary name of the
//! class, `_`, the mangled method name and, for an overloaded method, `__` and
//! the mangled method descriptor. The mangling escapes every character that is
//! not an ASCII alphanumeric (`_1` for `_`, `_2` for `;`, `_3` for `[` and
//! `_0xxxx` for a UTF-16 code unit) and writes `/` and `.` as a bare `_`.
//!
//! The model therefore stores the decoded class path, the decoded method name
//! and, for the long form, the parsed descriptor. Decoding happens once, while
//! parsing, so printing is total and cannot disagree with validation.
//!
//! Text borrows the input when the name has no escapes and is owned otherwise.

use alloc::borrow::Cow;
use alloc::vec::Vec;
use core::fmt;

use crate::budget::{EffectiveBudget, Limit};
use crate::parse::Limits;

/// Maximum accepted input length.
///
/// A JNI name is at most a few hundred bytes; the cap bounds every parser and
/// printer allocation. This is the default a [`crate::Budget`] starts from; a
/// [`crate::Demangler`] may raise it up to `HARD_INPUT_LEN`.
pub const MAX_INPUT_LEN: usize = 64 * 1024;

/// Maximum number of `[` array dimensions in one descriptor type.
///
/// This is the default a [`crate::Budget`] starts from; a [`crate::Demangler`]
/// may raise it up to `HARD_DEPTH`, the widest array a [`Type`] can store.
pub const MAX_DEPTH: usize = 32;

/// Maximum number of parser work units (bytes scanned).
///
/// A well-formed name is scanned a small constant number of times per byte; the
/// budget rejects inputs that force repeated rescans. This is the default a
/// [`crate::Budget`] starts from; a [`crate::Demangler`] may raise it up to
/// `HARD_WORK`.
pub const MAX_WORK: usize = 8 * MAX_INPUT_LEN;

/// Hard input ceiling.
///
/// The parser decodes escapes into owned text and the printed form grows with
/// it, so a raise stays bounded whatever the caller asks for.
pub(crate) const HARD_INPUT_LEN: usize = 16 * 1024 * 1024;

/// Hard depth ceiling.
///
/// [`Type::dimensions`] stores the array-dimension count in a `u16`, so the
/// generic 1024 parser ceiling fits comfortably; deeper input remains bounded
/// by the cursor and the AST representation.
pub(crate) const HARD_DEPTH: usize = 1024;

/// Hard work ceiling.
pub(crate) const HARD_WORK: usize = 64 * 1024 * 1024;

/// The input, depth and work limits one parse enforces.
///
/// Resolved from a [`crate::Budget`]: an absent dimension keeps its compiled-in
/// default and every value is clamped to this module's hard ceiling. One name
/// holds one argument list bounded by the input length, so there is no node
/// dimension to resolve.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ParseLimits {
    /// Maximum accepted input length in bytes.
    pub(crate) input: usize,
    /// Maximum array-dimension depth.
    pub(crate) depth: usize,
    /// Maximum parser work units.
    pub(crate) work: usize,
}

impl ParseLimits {
    /// Resolve one scheme's budget for a parse of a JNI native-method name.
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

/// A parsed JNI native-method name.
#[derive(Debug)]
pub struct Ast<'a> {
    pub(crate) class: Cow<'a, str>,
    pub(crate) method: Cow<'a, str>,
    pub(crate) descriptor: Option<Descriptor<'a>>,
}

impl Ast<'_> {
    /// The class/package path in Java-like spelling, e.g.
    /// `com.example.Foo$Inner`.
    ///
    /// The binary-name separator is recovered from the unescaped `_` that the
    /// mangling uses for both `.` and `/`; the last component is always the
    /// method, so every earlier component belongs to the class.
    #[must_use]
    pub fn class(&self) -> &str {
        &self.class
    }

    /// The method name in Java-like spelling, e.g. `under_score`.
    #[must_use]
    pub fn method(&self) -> &str {
        &self.method
    }

    /// The parsed descriptor of a long-form name.
    ///
    /// A short name carries no signature: the method is not overloaded, so
    /// there is nothing to disambiguate.
    #[must_use]
    pub const fn descriptor(&self) -> Option<&Descriptor<'_>> {
        self.descriptor.as_ref()
    }
}

/// The parsed argument list of a long-form name.
///
/// The long form encodes only the argument types: it exists to disambiguate
/// overloads, and Java forbids return-type-only overloading, so the return type
/// carries no information and is not part of the symbol.
#[derive(Debug)]
pub struct Descriptor<'a> {
    pub(crate) args: Vec<Type<'a>>,
}

impl<'a> Descriptor<'a> {
    /// The argument types, in declaration order.
    #[must_use]
    pub fn args(&self) -> &[Type<'a>] {
        &self.args
    }
}

/// A descriptor field type: an element type plus its array dimensions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Type<'a> {
    pub(crate) dimensions: u16,
    pub(crate) element: Element<'a>,
}

impl<'a> Type<'a> {
    /// The number of leading `[` dimensions.
    #[must_use]
    pub const fn dimensions(&self) -> u16 {
        self.dimensions
    }

    /// The element type the dimensions apply to.
    #[must_use]
    pub const fn element(&self) -> &Element<'a> {
        &self.element
    }
}

impl fmt::Display for Type<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.element, f)?;
        for _ in 0..self.dimensions {
            f.write_str("[]")?;
        }
        Ok(())
    }
}

/// The element type of a descriptor field type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Element<'a> {
    /// A primitive base type (`B`, `C`, `D`, `F`, `I`, `J`, `S`, `Z`).
    Primitive(Primitive),
    /// An object type (`L...;`), spelled as its dotted binary name.
    Object(Cow<'a, str>),
}

impl fmt::Display for Element<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Primitive(primitive) => f.write_str(primitive.java_name()),
            Self::Object(name) => f.write_str(name),
        }
    }
}

/// A primitive JNI descriptor base type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Primitive {
    /// `B`: `byte`.
    Byte,
    /// `C`: `char`.
    Char,
    /// `D`: `double`.
    Double,
    /// `F`: `float`.
    Float,
    /// `I`: `int`.
    Int,
    /// `J`: `long`.
    Long,
    /// `S`: `short`.
    Short,
    /// `Z`: `boolean`.
    Boolean,
}

impl Primitive {
    /// The Java source spelling of the type.
    #[must_use]
    pub const fn java_name(self) -> &'static str {
        match self {
            Self::Byte => "byte",
            Self::Char => "char",
            Self::Double => "double",
            Self::Float => "float",
            Self::Int => "int",
            Self::Long => "long",
            Self::Short => "short",
            Self::Boolean => "boolean",
        }
    }

    /// Classify a descriptor base-type byte.
    pub(crate) const fn from_descriptor(byte: u8) -> Option<Self> {
        match byte {
            b'B' => Some(Self::Byte),
            b'C' => Some(Self::Char),
            b'D' => Some(Self::Double),
            b'F' => Some(Self::Float),
            b'I' => Some(Self::Int),
            b'J' => Some(Self::Long),
            b'S' => Some(Self::Short),
            b'Z' => Some(Self::Boolean),
            _ => None,
        }
    }
}
