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
//! Untrusted input stays bounded by [`MAX_INPUT_LEN`](crate::objc::MAX_INPUT_LEN),
//! [`MAX_DEPTH`](crate::objc::MAX_DEPTH) and
//! [`MAX_WORK`](crate::objc::MAX_WORK). Exceeding a parse budget is an error,
//! never a partial result. [`MAX_OUTPUT_LEN`](crate::objc::MAX_OUTPUT_LEN) is
//! the structural output ceiling and is asserted by the tests.
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
//! Printing is not a budget: the printer walks the record once, so the output
//! is linear in the parsed name and
//! [`MAX_OUTPUT_LEN`](crate::objc::MAX_OUTPUT_LEN) is a fixed backstop sized
//! for the hard input ceiling. No accepted parse can exceed it.
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
    Ast, BlockDescriptor, Item, MAX_DEPTH, MAX_INPUT_LEN, MAX_OUTPUT_LEN, MAX_WORK, Metadata,
    MetadataKind, Method, MethodKind, ParseError,
};

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
/// [`MAX_DEPTH`], and [`ParseError::BudgetExceeded`] when the input or work
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
mod tests {
    use std::string::{String, ToString};
    use std::vec::Vec;

    use super::*;

    fn text(symbol: &str) -> String {
        parse(symbol)
            .unwrap_or_else(|error| panic!("{symbol}: {error}"))
            .to_string()
    }

    #[test]
    fn prints_apple_metadata() {
        for (symbol, expected) in [
            ("_OBJC_CLASS_$_NSString", "Objective-C class NSString"),
            (
                "_OBJC_METACLASS_$_NSString",
                "Objective-C metaclass NSString",
            ),
            (
                "__OBJC_$_CATEGORY_NSString_$_Drawing",
                "Objective-C category NSString (Drawing)",
            ),
            (
                "__OBJC_$_INSTANCE_METHODS_Foo",
                "Objective-C instance methods Foo",
            ),
            ("__OBJC_$_CLASS_METHODS_Foo", "Objective-C class methods Foo"),
            ("__OBJC_$_PROP_LIST_Foo", "Objective-C property list Foo"),
            (
                "__OBJC_$_INSTANCE_VARIABLES_Foo",
                "Objective-C instance variables Foo",
            ),
            ("__OBJC_PROTOCOL_$_NSCopying", "Objective-C protocol NSCopying"),
            (
                "__OBJC_PROTOCOL_REFERENCE_$_NSCopying",
                "Objective-C protocol ref NSCopying",
            ),
            (
                "__OBJC_LABEL_PROTOCOL_$_NSCopying",
                "Objective-C protocol label NSCopying",
            ),
            (
                "__OBJC_$_PROTOCOL_INSTANCE_METHODS_NSCopying",
                "Objective-C protocol instance methods NSCopying",
            ),
            (
                "__OBJC_$_PROTOCOL_CLASS_METHODS_NSCopying",
                "Objective-C protocol class methods NSCopying",
            ),
            (
                "__OBJC_$_PROTOCOL_INSTANCE_METHODS_OPT_NSCopying",
                "Objective-C protocol optional instance methods NSCopying",
            ),
            (
                "__OBJC_$_PROTOCOL_CLASS_METHODS_OPT_NSCopying",
                "Objective-C protocol optional class methods NSCopying",
            ),
            (
                "__OBJC_$_PROTOCOL_PROPERTIES_NSCopying",
                "Objective-C protocol property list NSCopying",
            ),
            (
                "__OBJC_$_PROTOCOL_REFS_NSCopying",
                "Objective-C protocol refs NSCopying",
            ),
            (
                "__OBJC_$_PROTOCOL_METHOD_TYPES_NSCopying",
                "Objective-C protocol method types NSCopying",
            ),
            (
                "__OBJC_$_CATEGORY_INSTANCE_METHODS_Foo_$_Bar",
                "Objective-C category instance methods Foo (Bar)",
            ),
            (
                "__OBJC_$_CATEGORY_CLASS_METHODS_Foo_$_Bar",
                "Objective-C category class methods Foo (Bar)",
            ),
            (
                "__OBJC_$_CATEGORY_PROP_LIST_Foo_$_Bar",
                "Objective-C category property list Foo (Bar)",
            ),
            (
                "__OBJC_CATEGORY_PROTOCOLS_$_Foo_$_Bar",
                "Objective-C category protocols Foo (Bar)",
            ),
            (
                "__OBJC_CLASS_PROTOCOLS_$_Foo",
                "Objective-C class protocols Foo",
            ),
            ("__OBJC_CLASS_RO_$_Foo", "Objective-C class data Foo"),
            (
                "__OBJC_METACLASS_RO_$_Foo",
                "Objective-C metaclass data Foo",
            ),
            (
                "_OBJC_IVAR_$_Foo._ivar",
                "Objective-C ivar Foo._ivar",
            ),
            (
                "_OBJC_METH_VAR_NAME_0",
                "Objective-C method name 0",
            ),
            (
                "_OBJC_METH_VAR_TYPE_12",
                "Objective-C method type 12",
            ),
        ] {
            assert_eq!(text(symbol), expected, "{symbol}");
        }
    }

    #[test]
    fn prints_gnustep_metadata() {
        for (symbol, expected) in [
            ("__objc_class_name_Foo", "Objective-C class Foo"),
            ("__objc_class_ref_Foo", "Objective-C class ref Foo"),
            (
                "__objc_ivar_offset_Foo._ivar",
                "Objective-C ivar offset Foo._ivar",
            ),
            (
                "__objc_ivar_offset_value_Foo._ivar",
                "Objective-C ivar offset value Foo._ivar",
            ),
            ("_OBJC_CLASS_Foo", "Objective-C class Foo"),
            ("_OBJC_METACLASS_Foo", "Objective-C metaclass Foo"),
            ("_OBJC_REF_CLASS_Foo", "Objective-C class ref Foo"),
            ("_OBJC_INIT_CLASS_Foo", "Objective-C class init Foo"),
            ("_OBJC_PROTOCOL_P", "Objective-C protocol P"),
            ("_OBJC_ClassName_Object", "Objective-C class name Object"),
            ("_OBJC_Class_Object", "Objective-C class Object"),
            ("_OBJC_MetaClass_Object", "Objective-C metaclass Object"),
            (
                "_OBJC_InstanceMethods_Object",
                "Objective-C instance methods Object",
            ),
            (
                "_OBJC_ClassMethods_Object",
                "Objective-C class methods Object",
            ),
            (
                "_OBJC_InstanceIvars_Object",
                "Objective-C instance variables Object",
            ),
            (
                "_OBJC_ClassIvars_Object",
                "Objective-C class variables Object",
            ),
            ("_OBJC_Module", "Objective-C module"),
            ("_OBJC_SYMBOLS", "Objective-C symbols"),
            ("_OBJC_SELECTOR_TABLE", "Objective-C selector table"),
            (
                "_OBJC_SELECTOR_REFERENCES_",
                "Objective-C selector references",
            ),
        ] {
            assert_eq!(text(symbol), expected, "{symbol}");
        }
    }

    #[test]
    fn prints_method_display_forms() {
        for symbol in [
            "-[NSObject init]",
            "+[Foo bar:baz:]",
            "-[Root(Cat) catMethod]",
            "-[Foo setX:]",
            "+[Foo _private]",
            "-[My.Module method]",
        ] {
            assert_eq!(text(symbol), symbol, "{symbol}");
        }
    }

    #[test]
    fn prints_block_descriptors() {
        assert_eq!(
            text("__block_descriptor_40_8_32r_e5_v8?0l"),
            "Objective-C block descriptor 40_8_32r_e5_v8?0l"
        );
        assert_eq!(
            text("___block_descriptor_40_e8_32r_e5_v8?0l"),
            "Objective-C block descriptor 40_e8_32r_e5_v8?0l"
        );
        // Clang's real block signatures contain a `\x01` separator.
        assert_eq!(
            text("___block_descriptor_40_e8_32r_e5_v8\u{1}?0l"),
            "Objective-C block descriptor 40_e8_32r_e5_v8\u{1}?0l"
        );
    }

    #[test]
    fn accepts_bare_and_single_underscore_spellings() {
        for (symbol, expected) in [
            ("_OBJC_CLASS_$_Foo", "Objective-C class Foo"),
            ("__OBJC_CLASS_$_Foo", "Objective-C class Foo"),
            ("OBJC_CLASS_$_Foo", "Objective-C class Foo"),
            ("_OBJC_$_INSTANCE_METHODS_Foo", "Objective-C instance methods Foo"),
            (
                "__OBJC_$_INSTANCE_METHODS_Foo",
                "Objective-C instance methods Foo",
            ),
        ] {
            assert_eq!(text(symbol), expected, "{symbol}");
        }
    }

    #[test]
    fn preserves_underscores_and_dollars() {
        assert_eq!(
            text("_OBJC_CLASS_$_My_Class$Sub"),
            "Objective-C class My_Class$Sub"
        );
        assert_eq!(
            text("__OBJC_$_CATEGORY_My_Class_$_My_Cat"),
            "Objective-C category My_Class (My_Cat)"
        );
        assert_eq!(
            text("__OBJC_$_PROP_LIST_My_Class_$_My_Cat"),
            "Objective-C property list My_Class (My_Cat)"
        );
    }

    #[test]
    fn accepts_gnustep_abi2_decorations() {
        // Clang's GNUstep ABI 2 output decorates `_OBJC_` by object format:
        // ELF emits `.`, COFF emits `$`, and the i686 COFF C decoration gives
        // `_$_`. All three render like the undecorated record.
        for (symbol, expected) in [
            ("_OBJC_CLASS_Foo", "Objective-C class Foo"),
            ("._OBJC_CLASS_Foo", "Objective-C class Foo"),
            ("$_OBJC_CLASS_Foo", "Objective-C class Foo"),
            ("_$_OBJC_CLASS_Foo", "Objective-C class Foo"),
            ("._OBJC_METACLASS_Foo", "Objective-C metaclass Foo"),
            ("._OBJC_REF_CLASS_Foo", "Objective-C class ref Foo"),
            ("$_OBJC_REF_CLASS_Foo", "Objective-C class ref Foo"),
            ("_$_OBJC_REF_CLASS_Foo", "Objective-C class ref Foo"),
            ("._OBJC_INIT_CLASS_Foo", "Objective-C class init Foo"),
            ("._OBJC_PROTOCOL_P", "Objective-C protocol P"),
            ("$_OBJC_PROTOCOL_P", "Objective-C protocol P"),
            ("_$_OBJC_PROTOCOL_P", "Objective-C protocol P"),
        ] {
            assert_eq!(text(symbol), expected, "{symbol}");
            assert!(is_objc(symbol), "{symbol}");
        }
        // The decoration only covers `_OBJC_`; local records, labels and the
        // Apple `__OBJC_` namespace are not decorated by any known target.
        for symbol in [
            ".objc_category_FooCat",
            ".L_OBJC_CLASS_Foo",
            "l_OBJC_LABEL_CLASS_$",
            ".__OBJC_$_INSTANCE_METHODS_Foo",
            "$_OBJC_NOPE_$_Foo",
        ] {
            assert!(parse(symbol).is_err(), "{symbol}");
            assert!(!is_objc(symbol), "{symbol}");
        }
        // A decorated known prefix with a malformed body stays claimed, like
        // the undecorated spelling, so `parse` reports the failure.
        assert!(parse("._OBJC_").is_err());
        assert!(is_objc("._OBJC_"));
    }

    #[test]
    fn exposes_structure() {
        let ast = parse("_OBJC_CLASS_$_Foo").unwrap();
        let metadata = ast.metadata().expect("metadata");
        assert_eq!(metadata.kind(), MetadataKind::Class);
        assert_eq!(metadata.subject().as_deref(), Some("Foo"));
        assert_eq!(metadata.category(), None);
        assert_eq!(metadata.member(), None);

        let ast = parse("__OBJC_$_CATEGORY_Foo_$_Bar").unwrap();
        let metadata = ast.metadata().expect("metadata");
        assert_eq!(metadata.kind(), MetadataKind::Category);
        assert_eq!(metadata.subject().as_deref(), Some("Foo"));
        assert_eq!(metadata.category().as_deref(), Some("Bar"));

        let ast = parse("_OBJC_IVAR_$_Foo._ivar").unwrap();
        let metadata = ast.metadata().expect("metadata");
        assert_eq!(metadata.kind(), MetadataKind::Ivar);
        assert_eq!(metadata.subject().as_deref(), Some("Foo"));
        assert_eq!(metadata.member().as_deref(), Some("_ivar"));

        let ast = parse("-[Foo bar:]").unwrap();
        let method = ast.method().expect("method");
        assert_eq!(method.kind(), MethodKind::Instance);
        assert_eq!(method.class(), "Foo");
        assert_eq!(method.category(), None);
        assert_eq!(method.selector(), "bar:");

        let ast = parse("__block_descriptor_40_8_32r_e5_v8?0l").unwrap();
        assert_eq!(
            ast.block_descriptor().expect("descriptor").payload(),
            "40_8_32r_e5_v8?0l"
        );
    }

    #[test]
    fn rejects_malformed_names() {
        for symbol in [
            "",
            "-",
            "+",
            "-[Foo]",
            "-[Foo ]",
            "-[ Foo bar]",
            "-[Foo bar",
            "+Foo bar]",
            "-[Foo bar baz]",
            "-[Foo bar::]",
            "-[Foo :bar]",
            "-[Foo bar:baz]",
            "-[Foo(bar baz]",
            "-[Foo() bar]",
            "_OBJC_",
            "__OBJC_",
            "OBJC_",
            "__OBJC_$_",
            "_OBJC_CLASS_$_",
            "_OBJC_NOPE_$_Foo",
            "__OBJC_$_NOPE_Foo",
            "__OBJC_$_CATEGORY_Foo",
            "__OBJC_$_CATEGORY__$_Bar",
            "__OBJC_$_CATEGORY_Foo_$_",
            "__OBJC_$_PROP_LIST_Foo_$_",
            "_OBJC_IVAR_$_Foo",
            "_OBJC_IVAR_$_._ivar",
            "_OBJC_METH_VAR_NAME_",
            "_OBJC_METH_VAR_NAME_x",
            "__objc_",
            "__objc_exec_class",
            "__objc_class_name_",
            "__objc_ivar_offset_Foo",
            "__objc_category_name_Foo_Bar",
            "__block_descriptor_",
            "___block_descriptor_",
        ] {
            assert!(parse(symbol).is_err(), "{symbol}");
        }
    }

    #[test]
    fn is_objc_recognizes_namespaces() {
        for symbol in [
            "_OBJC_CLASS_$_Foo",
            "__OBJC_$_INSTANCE_METHODS_Foo",
            "__objc_class_name_Foo",
            "-[Foo bar:]",
            "__block_descriptor_40_8_32r_e5_v8?0l",
        ] {
            assert!(is_objc(symbol), "{symbol}");
        }
        // The Apple namespace is claimed only for known kinds; a malformed
        // record under a known prefix is claimed so `parse` can reject it.
        assert!(!is_objc("_OBJC_NOPE_$_Foo"));
        assert!(parse("_OBJC_NOPE_$_Foo").is_err());
        assert!(is_objc("_OBJC_"));
        assert!(parse("_OBJC_").is_err());
        assert!(is_objc("_OBJC_CLASS_$_"));
        assert!(parse("_OBJC_CLASS_$_").is_err());
        for symbol in [
            "main",
            "_Z3foov",
            "__objc_exec_class",
            "__objc_category_name_Foo_Bar",
            "-[Foo",
        ] {
            assert!(!is_objc(symbol), "{symbol}");
        }
    }

    #[test]
    fn decorated_names_are_matched_on_bytes() {
        // The decoration is matched on raw bytes, so a subject that is not
        // UTF-8 still demangles and prints escaped.
        let ast = parse_bytes(b"._OBJC_CLASS_F\xffo").expect("demangles");
        assert_eq!(ast.to_string(), "Objective-C class F\\xffo");
        assert!(is_objc_bytes(b"$_OBJC_REF_CLASS_F\xffo"));
        assert!(is_objc_bytes(b"_$_OBJC_PROTOCOL_F\xffo"));
        assert!(!is_objc_bytes(b".objc_class_F\xffo"));
    }

    #[test]
    fn input_budget_is_enforced() {
        let mut symbol = String::from("_OBJC_CLASS_$_");
        symbol.push_str(&"a".repeat(MAX_INPUT_LEN));
        assert_eq!(parse(&symbol).unwrap_err(), ParseError::BudgetExceeded);
    }

    #[test]
    fn depth_budget_is_enforced() {
        let symbol = "-[Foo bar[[]]]";
        assert_eq!(parse(symbol).unwrap_err(), ParseError::RecursedTooDeep);
    }

    #[test]
    fn output_budget_is_respected() {
        for symbol in [
            "_OBJC_CLASS_$_Foo",
            "__OBJC_$_PROTOCOL_INSTANCE_METHODS_OPT_Foo",
            "__OBJC_$_CATEGORY_Foo_$_Bar",
            "__objc_ivar_offset_value_Foo._ivar",
            "-[Foo bar:baz:]",
            "___block_descriptor_40_e8_32r_e5_v8?0l",
        ] {
            let rendered = text(symbol);
            assert!(
                rendered.len() <= symbol.len() + 64,
                "{symbol}: {} > {}",
                rendered.len(),
                symbol.len() + 64
            );
            assert!(rendered.len() <= MAX_OUTPUT_LEN);
        }
    }

    #[test]
    fn errors_are_displayed() {
        assert_eq!(ParseError::Invalid.to_string(), "invalid syntax");
        assert_eq!(
            ParseError::RecursedTooDeep.to_string(),
            "recursion limit reached"
        );
        assert_eq!(ParseError::BudgetExceeded.to_string(), "size limit reached");
    }

    #[test]
    fn never_panics_on_truncated_names() {
        for symbol in [
            "_OBJC_CLASS_$_My_Class$Sub",
            "__OBJC_$_CATEGORY_Foo_$_Bar",
            "__OBJC_$_PROTOCOL_INSTANCE_METHODS_OPT_NSCopying",
            "__objc_ivar_offset_value_Foo._ivar",
            "-[Root(Cat) bar:baz:]",
            "___block_descriptor_40_e8_32r_e5_v8?0l",
        ] {
            for (end, _) in symbol.char_indices() {
                let _ = parse(&symbol[..end]);
                let _ = is_objc(&symbol[..end]);
            }
            let _ = parse(symbol);
        }
    }

    #[test]
    fn never_panics_on_arbitrary_input() {
        const CHARSET: &[u8] = b"abcXYZ019._$()[]:? -+@";
        let mut state = 0x0B7C_1234_u64;
        let mut next = || {
            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            (state >> 33) as usize
        };
        for _ in 0..10_000 {
            let len = next() % 48;
            let mut bytes = Vec::with_capacity(len);
            for _ in 0..len {
                bytes.push(CHARSET[next() % CHARSET.len()]);
            }
            if let Ok(text) = core::str::from_utf8(&bytes) {
                let _ = parse(text);
                let _ = is_objc(text);
            }
        }
    }
}

#[cfg(test)]
mod budget_tests {
    use alloc::format;
    use alloc::string::{String, ToString};

    use super::node::{HARD_INPUT_LEN, HARD_WORK, ParseLimits};
    use super::*;
    use crate::budget::Limit;
    use crate::{Budget, Demangler, Scheme};

    /// Parse `symbol` with `budget` resolved for [`Scheme::ObjC`].
    fn parse_with(symbol: &[u8], budget: Budget) -> Result<Ast<'_>, ParseError> {
        parse_bytes_with_budget(symbol, budget.effective(Scheme::ObjC))
    }

    /// Whether `symbol` parses with one budget dimension set to `value`.
    fn parses_with(symbol: &[u8], dimension: impl Fn(u32) -> Budget, value: u32) -> bool {
        parse_with(symbol, dimension(value)).is_ok()
    }

    /// Assert that `need` is the exact boundary of one dimension for `symbol`.
    ///
    /// The dimension is the only override, so a failure one unit below the
    /// boundary comes from that dimension and not from another limit.
    fn assert_dimension_boundary(
        symbol: &str,
        name: &str,
        ceiling: u32,
        dimension: impl Fn(u32) -> Budget,
    ) {
        let bytes = symbol.as_bytes();
        let need = (0..=ceiling)
            .find(|value| parses_with(bytes, &dimension, *value))
            .unwrap_or_else(|| panic!("{symbol}: no {name} budget parses"));
        assert!(need > 0, "{symbol}: the {name} boundary is zero");
        assert!(
            parses_with(bytes, &dimension, need),
            "{symbol}: {name} budget {need} must accept"
        );
        assert!(
            !parses_with(bytes, &dimension, need - 1),
            "{symbol}: {name} budget {} must reject",
            need - 1
        );
    }

    #[test]
    fn the_compiled_in_budget_matches_the_policy_defaults() {
        let limits = ParseLimits::from_effective(DEFAULT_BUDGET);
        assert_eq!(limits.input, MAX_INPUT_LEN);
        assert_eq!(limits.depth, MAX_DEPTH);
        assert_eq!(limits.work, MAX_WORK);
        assert_eq!(
            Scheme::ObjC.default_budget().effective(Scheme::ObjC),
            DEFAULT_BUDGET
        );

        // The policy table and this module agree on the ceilings, and an
        // Objective-C symbol is one record, so there is no node dimension.
        let hard = Scheme::ObjC.hard_budget();
        assert_eq!(hard.input_bytes, Some(Limit::Bounded(HARD_INPUT_LEN)));
        assert_eq!(hard.parse_depth, Some(Limit::Bounded(MAX_DEPTH)));
        assert_eq!(hard.work, Some(Limit::Bounded(HARD_WORK)));
        assert_eq!(hard.nodes, Some(Limit::Unbounded));
    }

    #[test]
    fn each_dimension_rejects_one_unit_below_what_the_symbol_needs() {
        for symbol in [
            "_OBJC_CLASS_$_Foo",
            "__OBJC_$_CATEGORY_Foo_$_Bar",
            "__objc_ivar_offset_value_Foo._ivar",
            "___block_descriptor_40_e8_32r_e5_v8?0l",
        ] {
            let ceiling = u32::try_from(symbol.len()).unwrap_or(u32::MAX) * 2 + 64;
            assert_dimension_boundary(symbol, "input", ceiling, |value| {
                Budget::new().with_input_bytes(value as usize)
            });
            assert_dimension_boundary(symbol, "work", ceiling, |value| {
                Budget::new().with_work(value)
            });
        }

        // A method display form carries exactly one bracket level.
        assert_dimension_boundary("-[Foo bar]", "depth", 4, |value| {
            Budget::new().with_parse_depth(value)
        });
    }

    #[test]
    fn raising_the_input_budget_accepts_a_longer_symbol() {
        let mut symbol = String::from("_OBJC_CLASS_$_");
        symbol.push_str(&"a".repeat(MAX_INPUT_LEN));
        assert!(matches!(
            parse_with(symbol.as_bytes(), Budget::new()),
            Err(ParseError::BudgetExceeded)
        ));
        let raised = Budget::new().with_input_bytes(2 * MAX_INPUT_LEN);
        let ast = parse_with(symbol.as_bytes(), raised).expect("raised input budget");
        assert_eq!(
            ast.to_string().len(),
            "Objective-C class ".len() + MAX_INPUT_LEN
        );
    }

    /// The bracket depth is the grammar's own limit, so a depth override can
    /// only tighten it: asking for more is clamped to the one level the
    /// grammar has.
    #[test]
    fn the_depth_budget_cannot_be_raised_past_the_grammar() {
        let budget = Budget::new().with_parse_depth(u32::MAX);
        assert_eq!(
            budget.effective(Scheme::ObjC).parse_depth,
            Some(Limit::Bounded(MAX_DEPTH))
        );
        assert!(parse_with(b"-[Foo bar]", budget).is_ok());
        assert!(matches!(
            parse_with(b"-[Foo bar[[]]]", budget),
            Err(ParseError::RecursedTooDeep)
        ));
    }

    /// The input and work dimensions are independent: raising the input alone
    /// keeps the default work allowance, so a scan that exceeds it still fails.
    #[test]
    fn raising_the_input_budget_does_not_raise_the_work_budget() {
        // The parser charges one work unit per input byte for the balance scan
        // and one per name byte for validation.
        let mut symbol = String::from("_OBJC_CLASS_$_");
        symbol.push_str(&"a".repeat(600_000));
        assert!(4 * symbol.len() > MAX_WORK);
        let work = u32::try_from(4 * symbol.len()).expect("work fits a u32");

        let input_only = Budget::new().with_input_bytes(HARD_INPUT_LEN);
        assert!(matches!(
            parse_with(symbol.as_bytes(), input_only),
            Err(ParseError::BudgetExceeded)
        ));

        assert!(parse_with(symbol.as_bytes(), input_only.with_work(work)).is_ok());
    }

    #[test]
    fn clamping_keeps_the_hard_ceilings() {
        let budget = Budget::new()
            .with_input_bytes(usize::MAX)
            .with_work(u32::MAX);
        let limits = ParseLimits::from_effective(budget.effective(Scheme::ObjC));
        assert_eq!(limits.input, HARD_INPUT_LEN);
        assert_eq!(limits.work, HARD_WORK);
        assert_eq!(
            budget.effective(Scheme::ObjC).input_bytes,
            Some(Limit::Bounded(HARD_INPUT_LEN))
        );

        // A name past the hard input cap stays rejected even when the caller
        // asks for more.
        let mut too_long = String::from("_OBJC_CLASS_$_");
        too_long.push_str(&"a".repeat(HARD_INPUT_LEN));
        assert!(matches!(
            parse_with(too_long.as_bytes(), budget),
            Err(ParseError::BudgetExceeded)
        ));
    }

    #[test]
    fn a_zero_budget_rejects_through_every_entry_point() {
        let symbol = "_OBJC_CLASS_$_Foo";
        assert!(Demangler::new().demangle(symbol).is_ok());
        for budget in [
            Budget::new().with_input_bytes(0),
            Budget::new().with_work(0),
        ] {
            let demangler = Demangler::with_budget(budget);
            assert!(demangler.demangle(symbol).is_err(), "{budget:?}");
            assert!(
                demangler
                    .demangle_with_scheme(Scheme::ObjC, symbol)
                    .is_err(),
                "{budget:?}"
            );
            assert!(
                demangler
                    .selection()
                    .allow(Scheme::ObjC)
                    .demangle(symbol)
                    .is_err(),
                "{budget:?}"
            );
        }

        // The depth dimension only rejects a name that carries a bracket.
        let method = "-[Foo bar]";
        let demangler = Demangler::with_budget(Budget::new().with_parse_depth(0));
        assert!(demangler.demangle(method).is_err());
        assert!(demangler.demangle(symbol).is_ok());
    }

    /// The fixed output cap is a backstop, not a print budget.
    ///
    /// The printer walks the record once, and the widest expansion is `\xNN`
    /// for a byte that is not valid UTF-8, so four times the input plus the
    /// description bounds the text. The cap is sized from the hard input
    /// ceiling, so a name accepted under a raised input budget still fits.
    #[test]
    fn the_output_cap_is_a_backstop_the_printer_never_reaches() {
        let mut symbol = b"_OBJC_CLASS_$_".to_vec();
        symbol.resize(MAX_INPUT_LEN, 0xff);
        let raised = Budget::new().with_input_bytes(2 * MAX_INPUT_LEN);
        let rendered = parse_with(&symbol, raised)
            .expect("the raised input budget parses")
            .to_string();
        assert!(rendered.len() <= MAX_OUTPUT_LEN);
        assert!(rendered.len() <= 4 * symbol.len() + 64);
    }

    #[test]
    fn the_default_budget_parses_the_corpus_like_parse() {
        let corpora = [
            "/refs/objc/corpus/apple-clang-19.txt",
            "/refs/objc/corpus/gcc-libobjc-14.txt",
            "/refs/objc/corpus/gnustep-abi2-clang-19.txt",
            "/refs/objc/corpus/gnustep-abi2-coff-clang-19.txt",
            "/refs/objc/corpus/gnustep-clang-19.txt",
            "/refs/objc/corpus/mattermost-2.46.0-ios.txt",
            "/refs/objc/corpus/methods-clang-19.txt",
        ];
        let mut compared = 0_usize;
        for corpus in corpora {
            let path = format!("{}{corpus}", std::env!("CARGO_MANIFEST_DIR"));
            let Ok(data) = std::fs::read_to_string(&path) else {
                std::eprintln!("objc corpus not present (published crate); skipping");
                return;
            };
            for line in data.lines() {
                let symbol = line.split('\t').next().unwrap_or_default();
                if symbol.is_empty() || symbol.starts_with('#') {
                    continue;
                }
                let expected = parse(symbol);
                let explicit = parse_with(symbol.as_bytes(), Budget::new());
                assert_eq!(
                    expected.is_ok(),
                    explicit.is_ok(),
                    "{symbol}: the default budget changes acceptance"
                );
                let (Ok(expected), Ok(explicit)) = (expected, explicit) else {
                    continue;
                };
                assert_eq!(expected.to_string(), explicit.to_string(), "{symbol}");
                compared += 1;
            }
        }
        assert!(compared > 0, "the default budget compared no symbols");
    }
}
