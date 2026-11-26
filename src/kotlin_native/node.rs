//! Kotlin/Native symbol model and budgets.
//!
//! Kotlin/Native is an LLVM backend with its own linker-symbol mangling. The
//! compiler's `KonanMangler` builds a declaration spelling from a qualified
//! path, a `#` member separator, a value-parameter list, a type-parameter list
//! and a return type; the LLVM backend then wraps it in a `k*:` prefix.
//!
//! The model mirrors that structure. Identifier text borrows the input when it
//! is valid UTF-8 and is owned only when it is not; lists (parameters, type
//! arguments, bounds) allocate.

use alloc::borrow::Cow;
use alloc::boxed::Box;
use alloc::vec::Vec;

use super::text::text;
use crate::budget::{EffectiveBudget, Limit};
use crate::parse::{Escape, Limits};

/// Maximum accepted input length.
///
/// This is the default a [`crate::Budget`] starts from; a [`crate::Demangler`]
/// may raise it up to `HARD_INPUT_LEN`.
pub const MAX_INPUT_LEN: usize = 64 * 1024;

/// Maximum nesting depth of type arguments and flexible types.
///
/// This is the default a [`crate::Budget`] starts from; a [`crate::Demangler`]
/// may raise it up to `HARD_DEPTH`.
pub const MAX_DEPTH: usize = 32;

/// Maximum number of parser work units (bytes scanned).
///
/// This is the default a [`crate::Budget`] starts from; a [`crate::Demangler`]
/// may raise it up to `HARD_WORK`.
pub const MAX_WORK: usize = 8 * MAX_INPUT_LEN;

/// Hard input ceiling.
///
/// The parser only borrows the input, but the type lists and the printed text
/// grow with it, so a raise stays bounded whatever the caller asks for.
pub(crate) const HARD_INPUT_LEN: usize = 16 * 1024 * 1024;

/// Hard depth ceiling.
///
/// Types are parsed recursively, so this bounds the parser's stack use as well
/// as the tree depth.
pub(crate) const HARD_DEPTH: usize = 1024;

/// Hard work ceiling.
pub(crate) const HARD_WORK: usize = 64 * 1024 * 1024;

/// The input, depth and work limits one parse enforces.
///
/// Resolved from a [`crate::Budget`]: an absent dimension keeps its compiled-in
/// default and every value is clamped to this module's hard ceiling. The parser
/// builds one symbol with its own bounded lists, so there is no node dimension
/// to resolve.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ParseLimits {
    /// Maximum accepted input length in bytes.
    pub(crate) input: usize,
    /// Maximum type nesting depth.
    pub(crate) depth: usize,
    /// Maximum parser work units.
    pub(crate) work: usize,
}

impl ParseLimits {
    /// Resolve one scheme's budget for a parse of a Kotlin/Native symbol.
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

/// A parsed Kotlin/Native symbol.
#[derive(Debug)]
pub struct Ast<'a> {
    pub(crate) symbol: Symbol<'a>,
}

impl<'a> Ast<'a> {
    /// The classified symbol.
    #[must_use]
    pub const fn symbol(&self) -> &Symbol<'a> {
        &self.symbol
    }
}

/// A Kotlin/Native linker symbol family.
#[derive(Debug)]
pub enum Symbol<'a> {
    /// A `kfun:` function, constructor, accessor or local function.
    Function(Function<'a>),
    /// A `ktype:` type symbol.
    Type(Type<'a>),
    /// A `kclass:` class symbol.
    Class(&'a [u8]),
    /// A `kproperty:` property symbol.
    Property(&'a [u8]),
    /// A `kobj:` object symbol.
    Object(&'a [u8]),
    /// A `kfield:` field symbol.
    Field(&'a [u8]),
    /// A `ktypealias:` type-alias symbol.
    TypeAlias(&'a [u8]),
    /// A `kpackage:` package symbol.
    Package(&'a [u8]),
    /// A `knamespace:` namespace symbol.
    Namespace(&'a [u8]),
    /// A `kvar:` global variable storage symbol.
    Variable(&'a [u8]),
}

/// A parsed `kfun:` function.
#[derive(Debug)]
pub struct Function<'a> {
    pub(crate) path: &'a [u8],
    pub(crate) name: &'a [u8],
    pub(crate) receiver: Option<Type<'a>>,
    pub(crate) context_params: Vec<Type<'a>>,
    pub(crate) params: Vec<Type<'a>>,
    pub(crate) type_params: Vec<TypeParameter<'a>>,
    pub(crate) ret: Option<Type<'a>>,
    pub(crate) is_static: bool,
    pub(crate) is_suspend: bool,
    pub(crate) is_internal: bool,
    pub(crate) is_impl: bool,
    pub(crate) companion: Option<&'a [u8]>,
    pub(crate) platform: Option<&'a [u8]>,
    pub(crate) signature: bool,
}

impl<'a> Function<'a> {
    /// The container's qualified path, before the `#` member separator.
    ///
    /// Names are symbol bytes: valid UTF-8 is returned verbatim, and bytes
    /// that are not valid UTF-8 are escaped as `\xNN`.
    #[must_use]
    pub fn path(&self) -> Cow<'a, str> {
        text(self.path, Escape::Identifier)
    }

    /// The function or accessor name.
    ///
    /// See [`Function::path`] for the byte-to-text rule.
    #[must_use]
    pub fn name(&self) -> Cow<'a, str> {
        text(self.name, Escape::Identifier)
    }

    /// The extension receiver, when the function is an extension.
    #[must_use]
    pub const fn receiver(&self) -> Option<&Type<'a>> {
        self.receiver.as_ref()
    }

    /// Context parameters, which precede the extension receiver in the encoding.
    #[must_use]
    pub fn context_params(&self) -> &[Type<'a>] {
        &self.context_params
    }

    /// Regular value parameters.
    #[must_use]
    pub fn params(&self) -> &[Type<'a>] {
        &self.params
    }

    /// Type parameters declared by the function.
    #[must_use]
    pub fn type_params(&self) -> &[TypeParameter<'a>] {
        &self.type_params
    }

    /// The encoded return type, when it was not `Unit`.
    #[must_use]
    pub const fn return_type(&self) -> Option<&Type<'a>> {
        self.ret.as_ref()
    }

    /// True for `#static` members.
    #[must_use]
    pub const fn is_static(&self) -> bool {
        self.is_static
    }

    /// True for `#suspend` functions.
    #[must_use]
    pub const fn is_suspend(&self) -> bool {
        self.is_suspend
    }

    /// True for the `#internal` linkage marker.
    #[must_use]
    pub const fn is_internal(&self) -> bool {
        self.is_internal
    }

    /// True for the `-impl` virtual-trampoline marker.
    #[must_use]
    pub const fn is_impl(&self) -> bool {
        self.is_impl
    }

    /// The `#companion` extension class, when present.
    ///
    /// See [`Function::path`] for the byte-to-text rule.
    #[must_use]
    pub fn companion(&self) -> Option<Cow<'a, str>> {
        self.companion.map(|name| text(name, Escape::Identifier))
    }

    /// The `%` platform-specific suffix, when present.
    ///
    /// The suffix is compiler data, so it is printed as written; bytes that
    /// are not valid UTF-8 are escaped as `\xNN`.
    #[must_use]
    pub fn platform(&self) -> Option<Cow<'a, str>> {
        self.platform.map(|suffix| text(suffix, Escape::Verbatim))
    }

    /// True when the symbol carries a value-parameter, type-parameter or
    /// return-type signature. Local functions with only `#internal` do not.
    #[must_use]
    pub const fn has_signature(&self) -> bool {
        self.signature
    }
}

/// A Kotlin type as encoded by the mangler.
#[derive(Debug)]
pub enum Type<'a> {
    /// A class or type-alias reference with optional type arguments.
    Class {
        /// The qualified type name.
        name: &'a [u8],
        /// Type arguments, empty when the reference is not parameterised.
        arguments: Vec<TypeArgument<'a>>,
        /// True when the type is marked `?`.
        nullable: bool,
    },
    /// A reference to a type parameter as `container:index`.
    TypeParameter {
        /// Index of the type-parameter container in the signature.
        container: u32,
        /// Index of the parameter within that container.
        index: u32,
        /// True when the type is marked `?`.
        nullable: bool,
    },
    /// The `<dynamic>` type.
    Dynamic,
    /// The `<ERROR CLASS>` placeholder emitted for unresolved types.
    Error,
    /// A flexible type encoded as `[lower~upper]`.
    Flexible {
        /// The lower bound.
        lower: Box<Self>,
        /// The upper bound.
        upper: Box<Self>,
    },
    /// A `...` vararg marker attached to a value-parameter type.
    Vararg(Box<Self>),
}

/// One type argument.
#[derive(Debug)]
pub enum TypeArgument<'a> {
    /// A `*` star projection.
    Star,
    /// A type with optional declaration-site variance.
    Type {
        /// The encoded variance.
        variance: Variance,
        /// The argument type.
        ty: Type<'a>,
    },
}

/// Declaration-site variance on a type argument.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Variance {
    /// No variance marker.
    Invariant,
    /// `out|`: covariant.
    Out,
    /// `in|`: contravariant.
    In,
}

impl Variance {
    /// The Kotlin keyword for the variance, or an empty string when invariant.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Invariant => "",
            Self::Out => "out",
            Self::In => "in",
        }
    }
}

/// A type parameter declaration: an index and its upper bounds.
#[derive(Debug)]
pub struct TypeParameter<'a> {
    pub(crate) index: u32,
    pub(crate) bounds: Vec<Type<'a>>,
}

impl<'a> TypeParameter<'a> {
    /// The parameter's index within the function's type-parameter list.
    #[must_use]
    pub const fn index(&self) -> u32 {
        self.index
    }

    /// Upper bounds separated by `&` in the encoding.
    #[must_use]
    pub fn bounds(&self) -> &[Type<'a>] {
        &self.bounds
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec::Vec;

    use super::*;

    #[test]
    fn reports_linkage_flags() {
        let function = Function {
            path: b"pkg.Foo",
            name: b"bar",
            receiver: None,
            context_params: Vec::new(),
            params: Vec::new(),
            type_params: Vec::new(),
            ret: None,
            is_static: false,
            is_suspend: false,
            is_internal: true,
            is_impl: true,
            companion: None,
            platform: None,
            signature: false,
        };
        assert!(function.is_internal());
        assert!(function.is_impl());
        assert!(!function.is_static());
        assert!(!function.is_suspend());
    }

    #[test]
    fn variance_keywords_are_complete() {
        assert_eq!(Variance::Invariant.as_str(), "");
        assert_eq!(Variance::Out.as_str(), "out");
        assert_eq!(Variance::In.as_str(), "in");
    }
}
