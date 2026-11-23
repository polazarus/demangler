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
    parser::parse_bytes(symbol)
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
        return (rest.is_empty() || has_known_apple_prefix(rest)) && parser::parse_bytes(name).is_ok();
    }
    if let Some(rest) = name.strip_prefix(b"__objc_") {
        if rest.starts_with(b"class_name_")
            || rest.starts_with(b"class_ref_")
            || rest.starts_with(b"ivar_offset_value_")
            || rest.starts_with(b"ivar_offset_")
        {
            return true;
        }
        return parser::parse_bytes(name).is_ok();
    }
    if name.starts_with(b"-")
        || name.starts_with(b"+")
        || name.starts_with(b"___block_descriptor_")
        || name.starts_with(b"__block_descriptor_")
    {
        return parser::parse_bytes(name).is_ok();
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
