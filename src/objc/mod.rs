//! Pure-Rust Objective-C symbol demangling.
//!
//! Objective-C has no single C++-style mangling. Linker symbol tables expose
//! runtime metadata (classes, metaclasses, categories, protocols, method and
//! property lists, ivars and block descriptors), and developer tools show
//! method spellings such as `-[NSObject init]`. This module recognises those
//! name families and renders a stable, documented spelling for them.
//!
//! No external demangler is used and there is no runtime dependency.
//!
//! # Supported forms
//!
//! - Apple/Mach-O metadata with one or two leading underscores, or none:
//!   `_OBJC_CLASS_$_Foo`, `__OBJC_$_INSTANCE_METHODS_Foo`,
//!   `__OBJC_$_CATEGORY_Foo_$_Bar`, `__OBJC_PROTOCOL_$_P`,
//!   `__OBJC_PROTOCOL_REFERENCE_$_P`, `_OBJC_IVAR_$_Foo._ivar`,
//!   `_OBJC_METH_VAR_NAME_0` and the related read-only, label, property and
//!   protocol records;
//! - GNUstep/GNU runtime metadata: `__objc_class_name_Foo`,
//!   `__objc_class_ref_Foo`, `__objc_ivar_offset_Foo._ivar`, plus the
//!   GNUstep 2.0 `_OBJC_CLASS_Foo`, `_OBJC_REF_CLASS_Foo`,
//!   `_OBJC_INIT_CLASS_Foo` and `_OBJC_PROTOCOL_P` spellings and the
//!   object-format decoration the GNUstep ABI 2 compilers put in front of them
//!   (`._OBJC_CLASS_Foo` on ELF, `$_OBJC_CLASS_Foo` and `_$_OBJC_CLASS_Foo` on
//!   COFF);
//! - method display forms `-[Class selector:]` and `+[Class selector:]`,
//!   including `-[Class(Category) selector]`;
//! - block descriptors `__block_descriptor_…` and
//!   `___block_descriptor_…`.
//!
//! # Canonical output
//!
//! There is no standard Objective-C demangler, so the printed text is this
//! crate's contract:
//!
//! | Input | Output |
//! |---|---|
//! | `_OBJC_CLASS_$_NSString` | `Objective-C class NSString` |
//! | `_OBJC_METACLASS_$_NSString` | `Objective-C metaclass NSString` |
//! | `__OBJC_$_CATEGORY_NSString_$_Drawing` | `Objective-C category NSString (Drawing)` |
//! | `__OBJC_$_INSTANCE_METHODS_Foo` | `Objective-C instance methods Foo` |
//! | `__OBJC_$_CLASS_METHODS_Foo` | `Objective-C class methods Foo` |
//! | `__OBJC_$_PROP_LIST_Foo` | `Objective-C property list Foo` |
//! | `__OBJC_$_INSTANCE_VARIABLES_Foo` | `Objective-C instance variables Foo` |
//! | `__OBJC_PROTOCOL_$_NSCopying` | `Objective-C protocol NSCopying` |
//! | `__objc_class_name_Foo` | `Objective-C class Foo` |
//! | `._OBJC_REF_CLASS_Foo` | `Objective-C class ref Foo` |
//! | `_$_OBJC_PROTOCOL_P` | `Objective-C protocol P` |
//! | `-[Foo bar:baz:]` | `-[Foo bar:baz:]` |
//!
//! Class, category and protocol names are preserved verbatim. One Mach-O
//! leading underscore is stripped, both `_OBJC_` and `__OBJC_` are accepted,
//! and the GNUstep ABI 2 object-format decoration is accepted in front of
//! `_OBJC_`. Metadata prefixes are case-sensitive, and unknown prefixes are
//! rejected.
//!
//! # Raw bytes
//!
//! A symbol name is a byte string, so the parser matches the grammar on bytes
//! and never requires the whole name to be UTF-8. Class, category, protocol,
//! selector and payload text that is valid UTF-8 prints verbatim (so Unicode
//! names survive); bytes that are not valid UTF-8 print escaped as `\xNN`.
//! A backslash is printed as `\\` so the spelling stays reversible.
//!
//! # Detection
//!
//! [`crate::Scheme::detect`] tries Objective-C after C++/Rust/Swift and
//! before Go and the legacy C++ dialects. It must run before Go because dotted
//! metadata such as `_OBJC_IVAR_$_Foo._ivar` looks like a package-qualified Go
//! name. Objective-C++ `_Z` names stay C++. `is_objc` claims known metadata
//! prefixes (so a malformed record is rejected by the parser) but leaves
//! generic `__objc_*` runtime helpers and unknown Apple metadata kinds to the
//! other schemes. A GNUstep ABI 2 decoration (`.`, `$` or `_$`) is only
//! claimed when `_OBJC_` follows it, so local `.objc_*` records and
//! `l_OBJC_*` labels stay unclaimed.
//!
//! # Budgets
//!
//! Untrusted input stays bounded by `MAX_INPUT_LEN`,
//! `MAX_DEPTH` and
//! `MAX_WORK`. Exceeding a parse budget is an error,
//! never a partial result.
//!
//! The input, depth and work limits are runtime-configurable.
//! [`parse`](crate::objc::parse) keeps the compiled-in defaults, and a
//! [`crate::Demangler`] passes the configured budget, clamped to the hard
//! ceilings of 16 MiB input, one bracket level and 64M work units. The bracket
//! level is the grammar's own limit, so a depth override can only tighten it.
//! Input and work are independent: raising the input limit alone keeps the
//! default work allowance, so a caller that raises the input for a long symbol
//! must raise [`crate::Budget::work`] as well when the scan exceeds it. An
//! Objective-C symbol is one record, so there is no node dimension to
//! configure.
//!
//! Printing is not a budget: the printer walks the record once and the output
//! is linear in the parsed name, so `Display` is total except for a failing
//! sink.
//!
//! # Gaps
//!
//! - `__objc_category_name_*` cannot be split unambiguously because class and
//!   category names may both contain `_`; it is rejected rather than guessed.
//! - Local `.objc_*` symbols, `l_OBJC_LABEL_*` labels, block helper functions
//!   and generic `__objc_*` runtime functions are not metadata records.
//! - Method selectors are display input, not linker symbols.
//! - Apple's `PROTOCOL_*_OPT_*` prefix is inherently ambiguous for a protocol
//!   literally named `OPT_*`; it is interpreted as the optional-method form.
//! - The undecorated `_OBJC_REF_CLASS_*`/`_OBJC_INIT_CLASS_*`/
//!   `_OBJC_PROTOCOL_*` spellings and bare `OBJC_*` are documented-layout
//!   compatibility aliases: no compiler target in the pinned snapshot emits
//!   them (see `refs/objc.md`), so they have no corpus rows of their own.
mod node;
mod parser;
mod printer;
mod text;

use core::fmt;

use crate::budget::{EffectiveBudget, Limit};
use node::ParseLimits;

pub use node::{
    Ast, BlockDescriptor, Item, Metadata, MetadataKind, Method, MethodKind, ParseError,
};
pub(crate) use node::{MAX_DEPTH, MAX_INPUT_LEN, MAX_WORK};

impl fmt::Display for Ast<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        printer::print(&self.item, f)
    }
}

impl Ast<'_> {
    /// Render the parsed symbol into `out`.
    pub(crate) fn print<W: fmt::Write + ?Sized>(&self, out: &mut W) -> fmt::Result {
        printer::print(&self.item, out)
    }
}

impl Ast<'_> {
    /// The scheme this tree was parsed from: always [`crate::Scheme::ObjC`].
    #[must_use]
    pub const fn scheme(&self) -> crate::Scheme {
        crate::Scheme::ObjC
    }
}

/// Parse an Objective-C symbol name into a reusable [`Ast`].
///
/// `symbol` is expected to be decoration-free; [`crate::Demangler`] strips a
/// ThinLTO `.llvm.<hex>` suffix and a GNU symbol-version `@` suffix before
/// calling this parser.
///
/// # Errors
///
/// Returns [`ParseError::Invalid`] when the name matches no recognised
/// Objective-C metadata, method-display or block-descriptor family,
/// [`ParseError::RecursedTooDeep`] when brackets nest deeper than
/// `MAX_DEPTH`, and [`ParseError::BudgetExceeded`] when the input or work
/// budget is exceeded.
pub fn parse(symbol: &str) -> Result<Ast<'_>, ParseError> {
    parse_bytes(symbol.as_bytes())
}

/// Parse an Objective-C symbol name from raw bytes.
///
/// This is the entry point the other backends and the demangler dispatch to
/// once symbol bytes, not text, are the input. See [`parse`] for the accepted
/// forms; the only difference is that the name does not have to be UTF-8.
pub(crate) fn parse_bytes(symbol: &[u8]) -> Result<Ast<'_>, ParseError> {
    parse_bytes_with_budget(symbol, DEFAULT_BUDGET)
}

/// The budget the public entry point starts from.
///
/// [`parse`] uses it, so it keeps exactly the compiled-in limits; a
/// [`crate::Demangler`] resolves its configured [`crate::Budget`] for
/// [`Scheme::ObjC`](crate::Scheme::ObjC) instead.
pub(crate) const DEFAULT_BUDGET: EffectiveBudget = EffectiveBudget {
    input_bytes: Some(Limit::Bounded(MAX_INPUT_LEN)),
    parse_depth: Some(Limit::Bounded(MAX_DEPTH)),
    work: Some(Limit::Bounded(MAX_WORK)),
    nodes: Some(Limit::Unbounded),
    expansion_visits: None,
    expansion_bytes: None,
};

/// Parse an Objective-C symbol name with one scheme's resolved budget.
///
/// [`parse_bytes`] wraps this with [`DEFAULT_BUDGET`]; a [`crate::Demangler`]
/// passes [`Demangler::effective_budget`](crate::Demangler::effective_budget)
/// for [`Scheme::ObjC`](crate::Scheme::ObjC), so a caller's overrides reach the
/// parser. The input, depth and work values are clamped to this module's hard
/// ceilings, and the grammar has no node dimension to resolve.
pub(crate) fn parse_bytes_with_budget(
    symbol: &[u8],
    budget: EffectiveBudget,
) -> Result<Ast<'_>, ParseError> {
    parser::parse_bytes(symbol, ParseLimits::from_effective(budget))
}

/// True when `name` is a recognised Objective-C symbol.
///
/// This is the prefilter used by [`crate::Scheme::detect`]. Apple/Mach-O and
/// GNUstep metadata namespaces are claimed only when the record after the
/// prefix is a known kind, so an unknown `_OBJC_*` name is left to the other
/// backends instead of being guessed. A known prefix with a malformed body is
/// claimed and then rejected by [`parse`]. Generic `__objc_*` runtime helpers
/// and method-display or block-descriptor shapes are only claimed when they
/// parse. `name` is expected to be decoration-free, like [`parse`]'s input.
#[must_use]
pub fn is_objc(name: &str) -> bool {
    is_objc_bytes(name.as_bytes())
}

/// True when `name` is a recognised Objective-C symbol, as raw bytes.
///
/// Detection is a byte-prefix test, so a name is claimed regardless of whether
/// the rest of it decodes as UTF-8.
pub(crate) fn is_objc_bytes(name: &[u8]) -> bool {
    if name.len() > MAX_INPUT_LEN {
        return false;
    }
    // GNUstep ABI 2 metadata is decorated by the object format (`.`, `$` or
    // `_$` in front of `_OBJC_`), so the decoration is not part of the match.
    let name = parser::strip_abi2_decoration(name);
    if let Some(rest) = name.strip_prefix(b"__OBJC_") {
        return rest.is_empty() || has_known_apple_prefix(rest);
    }
    if let Some(rest) = name.strip_prefix(b"_OBJC_") {
        return rest.is_empty() || has_known_apple_prefix(rest);
    }
    if let Some(rest) = name.strip_prefix(b"OBJC_") {
        return (rest.is_empty() || has_known_apple_prefix(rest)) && parse_bytes(name).is_ok();
    }
    if let Some(rest) = name.strip_prefix(b"__objc_") {
        if rest.starts_with(b"class_name_")
            || rest.starts_with(b"class_ref_")
            || rest.starts_with(b"ivar_offset_value_")
            || rest.starts_with(b"ivar_offset_")
        {
            return true;
        }
        return parse_bytes(name).is_ok();
    }
    if name.starts_with(b"-")
        || name.starts_with(b"+")
        || name.starts_with(b"___block_descriptor_")
        || name.starts_with(b"__block_descriptor_")
    {
        return parse_bytes(name).is_ok();
    }
    false
}

/// True when the part after an Apple metadata prefix starts a known kind.
fn has_known_apple_prefix(rest: &[u8]) -> bool {
    const PREFIXES: &[&[u8]] = &[
        b"CLASS_RO_$_",
        b"METACLASS_RO_$_",
        b"CLASS_PROTOCOLS_$_",
        b"CATEGORY_PROTOCOLS_$_",
        b"LABEL_PROTOCOL_$_",
        b"PROTOCOL_REFERENCE_$_",
        b"PROTOCOL_$_",
        b"EHTYPE_$_",
        b"CLASS_$_",
        b"METACLASS_$_",
        b"IVAR_$_",
        b"$_",
        b"REF_CLASS_",
        b"INIT_CLASS_",
        b"CLASS_",
        b"METACLASS_",
        b"PROTOCOL_",
        b"ClassName_",
        b"Class_",
        b"MetaClass_",
        b"InstanceMethods_",
        b"ClassMethods_",
        b"InstanceIvars_",
        b"ClassIvars_",
        b"METH_VAR_NAME_",
        b"METH_VAR_TYPE_",
        b"SELECTOR_REFERENCES_",
    ];
    PREFIXES.iter().any(|prefix| rest.starts_with(prefix))
        || matches!(rest, b"Module" | b"SYMBOLS" | b"SELECTOR_TABLE")
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod budget_tests;
