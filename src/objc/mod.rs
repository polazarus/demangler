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
//!   `_OBJC_INIT_CLASS_Foo` and `_OBJC_PROTOCOL_P` spellings;
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
//! | `-[Foo bar:baz:]` | `-[Foo bar:baz:]` |
//!
//! Class, category and protocol names are preserved verbatim. One Mach-O
//! leading underscore is stripped; both `_OBJC_` and `__OBJC_` are accepted.
//! Metadata prefixes are case-sensitive, and unknown prefixes are rejected.
//!
//! # Detection
//!
//! [`crate::Scheme::detect`] tries Objective-C after C++/Rust/Swift and
//! before Go and the legacy C++ dialects. It must run before Go because dotted
//! metadata such as `_OBJC_IVAR_$_Foo._ivar` looks like a package-qualified Go
//! name. Objective-C++ `_Z` names stay C++. `is_objc` claims known metadata
//! prefixes (so a malformed record is rejected by the parser) but leaves
//! generic `__objc_*` runtime helpers and unknown Apple metadata kinds to the
//! other schemes.
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
//! - Bare `OBJC_` and GNUstep 2.0 `_OBJC_REF_CLASS_*`/`_OBJC_INIT_CLASS_*`/
//!   `_OBJC_PROTOCOL_*` spellings are supported from the documented layout but
//!   are not present in the generated binaries, so they lack corpus coverage.
mod node;
mod parser;
mod printer;

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
    parser::parse(symbol)
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
    if name.len() > MAX_INPUT_LEN {
        return false;
    }
    if let Some(rest) = name.strip_prefix("__OBJC_") {
        return rest.is_empty() || has_known_apple_prefix(rest);
    }
    if let Some(rest) = name.strip_prefix("_OBJC_") {
        return rest.is_empty() || has_known_apple_prefix(rest);
    }
    if let Some(rest) = name.strip_prefix("OBJC_") {
        return (rest.is_empty() || has_known_apple_prefix(rest)) && parser::parse(name).is_ok();
    }
    if let Some(rest) = name.strip_prefix("__objc_") {
        if rest.starts_with("class_name_")
            || rest.starts_with("class_ref_")
            || rest.starts_with("ivar_offset_value_")
            || rest.starts_with("ivar_offset_")
        {
            return true;
        }
        return parser::parse(name).is_ok();
    }
    if name.starts_with('-')
        || name.starts_with('+')
        || name.starts_with("___block_descriptor_")
        || name.starts_with("__block_descriptor_")
    {
        return parser::parse(name).is_ok();
    }
    false
}

/// True when `name` is in the underscore-prefixed Objective-C metadata
/// namespace, whether or not the record itself is recognised.
///
/// [`crate::Scheme::detect`] uses this to keep an unknown metadata name out of
/// the legacy C++ dialects, which have no unambiguous prefix and would
/// otherwise guess at it.
pub(crate) fn has_objc_namespace(name: &str) -> bool {
    name.starts_with("_OBJC_") || name.starts_with("__OBJC_") || name.starts_with("__objc_")
}

/// True when the part after an Apple metadata prefix starts a known kind.
fn has_known_apple_prefix(rest: &str) -> bool {
    const PREFIXES: &[&str] = &[
        "CLASS_RO_$_",
        "METACLASS_RO_$_",
        "CLASS_PROTOCOLS_$_",
        "CATEGORY_PROTOCOLS_$_",
        "LABEL_PROTOCOL_$_",
        "PROTOCOL_REFERENCE_$_",
        "PROTOCOL_$_",
        "EHTYPE_$_",
        "CLASS_$_",
        "METACLASS_$_",
        "IVAR_$_",
        "$_",
        "REF_CLASS_",
        "INIT_CLASS_",
        "CLASS_",
        "METACLASS_",
        "PROTOCOL_",
        "ClassName_",
        "Class_",
        "MetaClass_",
        "InstanceMethods_",
        "ClassMethods_",
        "InstanceIvars_",
        "ClassIvars_",
        "METH_VAR_NAME_",
        "METH_VAR_TYPE_",
        "SELECTOR_REFERENCES_",
    ];
    PREFIXES.iter().any(|prefix| rest.starts_with(prefix))
        || matches!(rest, "Module" | "SYMBOLS" | "SELECTOR_TABLE")
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
    fn exposes_structure() {
        let ast = parse("_OBJC_CLASS_$_Foo").unwrap();
        let metadata = ast.metadata().expect("metadata");
        assert_eq!(metadata.kind(), MetadataKind::Class);
        assert_eq!(metadata.subject(), Some("Foo"));
        assert_eq!(metadata.category(), None);
        assert_eq!(metadata.member(), None);

        let ast = parse("__OBJC_$_CATEGORY_Foo_$_Bar").unwrap();
        let metadata = ast.metadata().expect("metadata");
        assert_eq!(metadata.kind(), MetadataKind::Category);
        assert_eq!(metadata.subject(), Some("Foo"));
        assert_eq!(metadata.category(), Some("Bar"));

        let ast = parse("_OBJC_IVAR_$_Foo._ivar").unwrap();
        let metadata = ast.metadata().expect("metadata");
        assert_eq!(metadata.kind(), MetadataKind::Ivar);
        assert_eq!(metadata.subject(), Some("Foo"));
        assert_eq!(metadata.member(), Some("_ivar"));

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
