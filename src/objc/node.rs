//! Objective-C symbol model and budgets.
//!
//! Objective-C has no single C++-style mangling: the linker exposes runtime
//! metadata under a handful of documented prefixes, and developer tools also
//! show method spellings. The model mirrors those name families instead of
//! inventing a source-level syntax.
//!
//! The parser allocates only for text that is not valid UTF-8. The input,
//! work, depth and output budgets bound untrusted names.

pub(crate) use alloc::borrow::Cow;

use super::text::text;
use crate::parse::Escape;

/// Maximum accepted input length.
pub const MAX_INPUT_LEN: usize = 64 * 1024;

/// Maximum bracket nesting depth.
///
/// Metadata names contain no brackets, and a method display form has exactly
/// one pair. Anything deeper is rejected instead of being scanned unbounded.
pub const MAX_DEPTH: usize = 1;

/// Maximum number of parser work units (bytes scanned). Real inputs need at
/// most a small multiple of the input length; this is a hard ceiling.
pub const MAX_WORK: usize = 8 * MAX_INPUT_LEN;

/// Maximum printed output length.
///
/// Metadata output replaces a long prefix with a short description and adds
/// at most a few words, so it stays within a small constant of the input. A
/// method display form prints byte-for-byte. Text that is not valid UTF-8 is
/// escaped as `\xNN`, which costs four bytes per input byte, so the ceiling
/// allows for that expansion.
pub const MAX_OUTPUT_LEN: usize = 4 * MAX_INPUT_LEN + 64;

/// A parsed Objective-C symbol.
#[derive(Debug)]
pub struct Ast<'a> {
    pub(crate) item: Item<'a>,
}

impl<'a> Ast<'a> {
    /// The parsed symbol family.
    pub const fn item(&self) -> &Item<'a> {
        &self.item
    }

    /// The metadata record, when the symbol is runtime metadata.
    pub const fn metadata(&self) -> Option<&Metadata<'a>> {
        match &self.item {
            Item::Metadata(metadata) => Some(metadata),
            _ => None,
        }
    }

    /// The method display form, when the input was one.
    pub const fn method(&self) -> Option<&Method<'a>> {
        match &self.item {
            Item::Method(method) => Some(method),
            _ => None,
        }
    }

    /// The block descriptor, when the symbol is one.
    pub const fn block_descriptor(&self) -> Option<&BlockDescriptor<'a>> {
        match &self.item {
            Item::BlockDescriptor(descriptor) => Some(descriptor),
            _ => None,
        }
    }
}

/// The three name families this backend recognises.
#[derive(Debug)]
pub enum Item<'a> {
    /// Apple, Mach-O or GNUstep runtime metadata.
    Metadata(Metadata<'a>),
    /// A `-[Class selector:]` / `+[Class selector:]` display form.
    Method(Method<'a>),
    /// A `__block_descriptor_…` / `___block_descriptor_…` symbol.
    BlockDescriptor(BlockDescriptor<'a>),
}

/// One runtime metadata record.
///
/// `subject` is the class, protocol or index the record belongs to;
/// `category` is set for category-owned records; `member` is set for records
/// that name an ivar or other member. The fields are separate so the printer
/// never has to split an identifier.
#[derive(Debug)]
pub struct Metadata<'a> {
    pub(crate) kind: MetadataKind,
    pub(crate) subject: Option<&'a [u8]>,
    pub(crate) category: Option<&'a [u8]>,
    pub(crate) member: Option<&'a [u8]>,
}

impl<'a> Metadata<'a> {
    /// The metadata kind.
    pub const fn kind(&self) -> MetadataKind {
        self.kind
    }

    /// The class, protocol or index the record belongs to.
    ///
    /// Names are symbol bytes: valid UTF-8 is returned verbatim, and bytes
    /// that are not valid UTF-8 are escaped as `\xNN`.
    pub fn subject(&self) -> Option<Cow<'a, str>> {
        self.subject.map(|name| text(name, Escape::Identifier))
    }

    /// The category name for category-owned records.
    ///
    /// See [`Metadata::subject`] for the byte-to-text rule.
    pub fn category(&self) -> Option<Cow<'a, str>> {
        self.category.map(|name| text(name, Escape::Identifier))
    }

    /// The ivar or member name for member records.
    ///
    /// See [`Metadata::subject`] for the byte-to-text rule.
    pub fn member(&self) -> Option<Cow<'a, str>> {
        self.member.map(|name| text(name, Escape::Identifier))
    }
}

/// A runtime metadata kind.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MetadataKind {
    /// A class object (`_OBJC_CLASS_$_Foo`, `_OBJC_CLASS_Foo`, …).
    Class,
    /// A metaclass object.
    Metaclass,
    /// A class reference (`_OBJC_REF_CLASS_Foo`).
    ClassRef,
    /// A class initialiser (`_OBJC_INIT_CLASS_Foo`).
    ClassInit,
    /// A class name string (`_OBJC_ClassName_Object`).
    ClassName,
    /// A category (`__OBJC_$_CATEGORY_Foo_$_Bar`).
    Category,
    /// An instance-method list.
    InstanceMethods,
    /// A class-method list.
    ClassMethods,
    /// A property list.
    PropertyList,
    /// A class property list.
    ClassPropertyList,
    /// An Objective-C exception type (`_OBJC_EHTYPE_$_Foo`).
    ExceptionType,
    /// An instance-ivar list.
    InstanceVariables,
    /// A class-ivar list.
    ClassVariables,
    /// A protocol.
    Protocol,
    /// A protocol reference.
    ProtocolRef,
    /// A protocol label.
    ProtocolLabel,
    /// A protocol's required instance methods.
    ProtocolInstanceMethods,
    /// A protocol's required class methods.
    ProtocolClassMethods,
    /// A protocol's optional instance methods.
    ProtocolOptionalInstanceMethods,
    /// A protocol's optional class methods.
    ProtocolOptionalClassMethods,
    /// A protocol's property list.
    ProtocolPropertyList,
    /// A protocol's parent protocol references.
    ProtocolRefs,
    /// A protocol's method type strings.
    ProtocolMethodTypes,
    /// A category's instance-method list.
    CategoryInstanceMethods,
    /// A category's class-method list.
    CategoryClassMethods,
    /// A category's property list.
    CategoryPropertyList,
    /// A category's adopted protocols.
    CategoryProtocols,
    /// A class's adopted protocols.
    ClassProtocols,
    /// A class's read-only metadata.
    ClassData,
    /// A metaclass's read-only metadata.
    MetaclassData,
    /// One instance variable (`_OBJC_IVAR_$_Foo._ivar`).
    Ivar,
    /// A GNUstep ivar offset (`__objc_ivar_offset_Foo._ivar`).
    IvarOffset,
    /// A GNUstep ivar offset value.
    IvarOffsetValue,
    /// A method name string (`_OBJC_METH_VAR_NAME_0`).
    MethodName,
    /// A method type string (`_OBJC_METH_VAR_TYPE_0`).
    MethodType,
    /// The runtime module (`_OBJC_Module`).
    Module,
    /// The runtime symbol list (`_OBJC_SYMBOLS`).
    Symbols,
    /// The selector table (`_OBJC_SELECTOR_TABLE`).
    SelectorTable,
    /// Selector references (`_OBJC_SELECTOR_REFERENCES_`).
    SelectorReferences,
}

impl MetadataKind {
    /// The canonical description used by the printer.
    pub const fn description(self) -> &'static str {
        match self {
            Self::Class => "class",
            Self::Metaclass => "metaclass",
            Self::ClassRef => "class ref",
            Self::ClassInit => "class init",
            Self::ClassName => "class name",
            Self::Category => "category",
            Self::InstanceMethods => "instance methods",
            Self::ClassMethods => "class methods",
            Self::PropertyList => "property list",
            Self::ClassPropertyList => "class property list",
            Self::ExceptionType => "exception type",
            Self::InstanceVariables => "instance variables",
            Self::ClassVariables => "class variables",
            Self::Protocol => "protocol",
            Self::ProtocolRef => "protocol ref",
            Self::ProtocolLabel => "protocol label",
            Self::ProtocolInstanceMethods => "protocol instance methods",
            Self::ProtocolClassMethods => "protocol class methods",
            Self::ProtocolOptionalInstanceMethods => "protocol optional instance methods",
            Self::ProtocolOptionalClassMethods => "protocol optional class methods",
            Self::ProtocolPropertyList => "protocol property list",
            Self::ProtocolRefs => "protocol refs",
            Self::ProtocolMethodTypes => "protocol method types",
            Self::CategoryInstanceMethods => "category instance methods",
            Self::CategoryClassMethods => "category class methods",
            Self::CategoryPropertyList => "category property list",
            Self::CategoryProtocols => "category protocols",
            Self::ClassProtocols => "class protocols",
            Self::ClassData => "class data",
            Self::MetaclassData => "metaclass data",
            Self::Ivar => "ivar",
            Self::IvarOffset => "ivar offset",
            Self::IvarOffsetValue => "ivar offset value",
            Self::MethodName => "method name",
            Self::MethodType => "method type",
            Self::Module => "module",
            Self::Symbols => "symbols",
            Self::SelectorTable => "selector table",
            Self::SelectorReferences => "selector references",
        }
    }
}

/// A method display form.
#[derive(Debug)]
pub struct Method<'a> {
    pub(crate) kind: MethodKind,
    pub(crate) class: &'a [u8],
    pub(crate) category: Option<&'a [u8]>,
    pub(crate) selector: &'a [u8],
}

impl<'a> Method<'a> {
    /// Whether this is an instance (`-`) or class (`+`) method.
    pub const fn kind(&self) -> MethodKind {
        self.kind
    }

    /// The class name.
    ///
    /// See [`Metadata::subject`] for the byte-to-text rule.
    pub fn class(&self) -> Cow<'a, str> {
        text(self.class, Escape::Identifier)
    }

    /// The category name for a category method display form.
    ///
    /// See [`Metadata::subject`] for the byte-to-text rule.
    pub fn category(&self) -> Option<Cow<'a, str>> {
        self.category.map(|name| text(name, Escape::Identifier))
    }

    /// The selector, with its colons preserved.
    ///
    /// See [`Metadata::subject`] for the byte-to-text rule.
    pub fn selector(&self) -> Cow<'a, str> {
        text(self.selector, Escape::Identifier)
    }
}

/// The receiver kind of a method display form.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MethodKind {
    /// `-[Class selector:]`.
    Instance,
    /// `+[Class selector:]`.
    Class,
}

impl MethodKind {
    /// The sigil printed before the bracket.
    pub const fn sign(self) -> char {
        match self {
            Self::Instance => '-',
            Self::Class => '+',
        }
    }
}

/// A block descriptor symbol.
#[derive(Debug)]
pub struct BlockDescriptor<'a> {
    pub(crate) payload: &'a [u8],
}

impl<'a> BlockDescriptor<'a> {
    /// The descriptor payload after the runtime prefix.
    ///
    /// The payload is compiler data, so it is printed as written; bytes that
    /// are not valid UTF-8 are escaped as `\xNN`.
    pub fn payload(&self) -> Cow<'a, str> {
        text(self.payload, Escape::Verbatim)
    }
}

/// Why a symbol could not be parsed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ParseError {
    /// The symbol does not match a known Objective-C name family.
    Invalid,
    /// The symbol nests brackets deeper than [`MAX_DEPTH`].
    RecursedTooDeep,
    /// The symbol exceeds the input or work budget.
    BudgetExceeded,
}

impl core::fmt::Display for ParseError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(match self {
            Self::Invalid => "invalid syntax",
            Self::RecursedTooDeep => "recursion limit reached",
            Self::BudgetExceeded => "size limit reached",
        })
    }
}

impl core::error::Error for ParseError {}
