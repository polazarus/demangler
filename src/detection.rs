//! Private detection result shared by scheme detection and dispatch.
//!
//! Public [`Scheme::detect`](crate::Scheme::detect) keeps its current result;
//! `Demangler` can additionally reuse the parsed legacy tree instead of
//! parsing the same symbol again.

use crate::scheme::{Scheme, is_plain_c_name, strip_decorations_bytes};

#[cfg(any(
    feature = "cpp-itanium",
    feature = "cpp-msvc",
    feature = "cpp-gnu-v2"
))]
use crate::scheme::scheme_for_cpp_dialect;

/// A tree parsed while detecting a scheme, when that parse is also the one the
/// backend would perform.
#[allow(unused_lifetimes)]
pub(crate) enum Parsed<'a> {
    #[cfg(any(
        feature = "cpp-itanium",
        feature = "cpp-msvc",
        feature = "cpp-gnu-v2"
    ))]
    Cpp(crate::cpp::Ast<'a>),
    #[cfg(feature = "swift")]
    Swift(crate::swift::Ast<'a>),
    #[cfg(feature = "d")]
    D(crate::d::Ast<'a>),
    #[cfg(feature = "ocaml")]
    OCaml(crate::ocaml::Ast<'a>),
    #[cfg(feature = "delphi")]
    Delphi(crate::delphi::Ast<'a>),
    #[cfg(feature = "jni")]
    Jni(crate::jni::Ast<'a>),
    /// Keeps the lifetime valid in builds with neither C++ nor Swift.
    #[doc(hidden)]
    #[allow(dead_code)]
    __Phantom(core::marker::PhantomData<&'a ()>),
}

/// The result of one detection pass.
pub(crate) struct Detection<'a> {
    /// The precise scheme.
    pub(crate) scheme: Scheme,
    /// Post-COFF, pre-version/ThinLTO dispatch spelling.
    pub(crate) symbol: &'a [u8],
    /// The stripped view detection parses and the C++ fallback uses.
    pub(crate) parse_symbol: &'a [u8],
    /// A legacy C++/Swift tree parsed during detection, when available.
    pub(crate) parsed: Option<Parsed<'a>>,
    /// The outermost COFF decoration, when the input was decorated.
    #[cfg(feature = "coff")]
    pub(crate) decoration: Option<crate::coff::Decoration<'a>>,
}

/// Detect the precise scheme and retain an AST when detection already parsed it.
pub(crate) fn detect(name: &[u8]) -> Detection<'_> {
    if name.is_empty() {
        return plain(Scheme::Unknown, name, name);
    }
    // COFF decorations are orthogonal and may contain `@`, so they must be
    // decoded before the GNU version-suffix stripper sees the name.
    #[cfg(feature = "coff")]
    if let Some(decoration) = crate::coff::parse_bytes(name) {
        let mut inner = detect(decoration.symbol());
        inner.decoration = Some(decoration);
        return inner;
    }
    #[cfg(feature = "go")]
    let decorated = name;
    let parse_symbol = strip_decorations_bytes(name);
    if parse_symbol.is_empty() {
        return plain(Scheme::Unknown, name, parse_symbol);
    }

    #[cfg(feature = "rust")]
    if crate::rust::is_v0_prefix_bytes(parse_symbol) {
        return plain(Scheme::RustV0, name, parse_symbol);
    }

    #[cfg(feature = "rust")]
    if crate::rust::is_legacy_hash_bytes(parse_symbol) {
        return plain(Scheme::RustLegacy, name, parse_symbol);
    }

    #[cfg(feature = "d")]
    if let Ok(ast) = crate::d::parse_bytes(parse_symbol) {
        return with_parsed(plain(Scheme::D, name, parse_symbol), Parsed::D(ast));
    }

    #[cfg(feature = "cpp-itanium")]
    if crate::cpp::is_alloc_token_bytes(parse_symbol)
        || crate::cpp::is_itanium_bytes(parse_symbol)
    {
        return plain(Scheme::CppItanium, name, parse_symbol);
    }

    #[cfg(all(feature = "rust", not(feature = "cpp-itanium")))]
    if is_rust_legacy_prefix_bytes(parse_symbol) {
        return plain(Scheme::RustLegacy, name, parse_symbol);
    }

    #[cfg(feature = "swift")]
    if let Some((scheme, parsed)) = swift_scheme_bytes(parse_symbol) {
        let detection = plain(scheme, name, parse_symbol);
        return match parsed {
            Some(parsed) => with_parsed(detection, parsed),
            None => detection,
        };
    }

    #[cfg(feature = "cpp-msvc")]
    if crate::cpp::is_msvc_bytes(parse_symbol) {
        return plain(Scheme::CppMsvc, name, parse_symbol);
    }

    #[cfg(feature = "objc")]
    if crate::objc::is_objc_bytes(parse_symbol) {
        return plain(Scheme::ObjC, name, parse_symbol);
    }

    #[cfg(feature = "objc")]
    if has_objc_namespace_bytes(parse_symbol) {
        return plain(Scheme::Unknown, name, parse_symbol);
    }

    #[cfg(feature = "kotlin-native")]
    if crate::kotlin_native::is_kotlin_native_bytes(parse_symbol) {
        return plain(Scheme::KotlinNative, name, parse_symbol);
    }

    #[cfg(feature = "delphi")]
    if let Ok(ast) = crate::delphi::parse_bytes(parse_symbol) {
        let mut detection = plain(Scheme::Delphi, name, parse_symbol);
        // Dispatch parses the decorated spelling; only reuse the tree when
        // detection parsed exactly those bytes.
        if name == parse_symbol {
            detection.parsed = Some(Parsed::Delphi(ast));
        }
        return detection;
    }

    #[cfg(feature = "ocaml")]
    if let Ok(ast) = crate::ocaml::parse_bytes(parse_symbol) {
        let mut detection = plain(Scheme::OCaml, name, parse_symbol);
        if name == parse_symbol {
            detection.parsed = Some(Parsed::OCaml(ast));
        }
        return detection;
    }

    #[cfg(feature = "go")]
    if crate::go::is_go_bytes(decorated) {
        return plain(Scheme::Go, name, parse_symbol);
    }

    #[cfg(feature = "jni")]
    if let Ok(ast) = crate::jni::parse_bytes(parse_symbol) {
        return with_parsed(plain(Scheme::Jni, name, parse_symbol), Parsed::Jni(ast));
    }

    #[cfg(feature = "ada")]
    if crate::ada::is_ada(parse_symbol) {
        return plain(Scheme::Ada, name, parse_symbol);
    }

    #[cfg(feature = "fortran")]
    if crate::fortran::is_fortran(parse_symbol) {
        return plain(Scheme::Fortran, name, parse_symbol);
    }

    #[cfg(feature = "cpp-gnu-v2")]
    if let Some(ast) = crate::cpp::legacy::parse_if_legacy(parse_symbol) {
        let scheme = scheme_for_cpp_dialect(ast.dialect());
        return with_parsed(plain(scheme, name, parse_symbol), Parsed::Cpp(ast));
    }

    if is_plain_c_name(parse_symbol) {
        return plain(Scheme::C, name, parse_symbol);
    }

    plain(Scheme::Unknown, name, parse_symbol)
}

fn plain<'a>(scheme: Scheme, symbol: &'a [u8], parse_symbol: &'a [u8]) -> Detection<'a> {
    Detection {
        scheme,
        symbol,
        parse_symbol,
        parsed: None,
        #[cfg(feature = "coff")]
        decoration: None,
    }
}

#[cfg(any(
    feature = "cpp-itanium",
    feature = "cpp-msvc",
    feature = "cpp-gnu-v2",
    feature = "swift",
    feature = "d",
    feature = "ocaml",
    feature = "delphi",
    feature = "jni"
))]
fn with_parsed<'a>(mut detection: Detection<'a>, parsed: Parsed<'a>) -> Detection<'a> {
    detection.parsed = Some(parsed);
    detection
}

#[cfg(feature = "swift")]
fn swift_scheme_bytes<'a>(name: &'a [u8]) -> Option<(Scheme, Option<Parsed<'a>>)> {
    let prefix = crate::swift::prefix_len_bytes(name)?;
    if prefix == 2 && name.starts_with(b"_T") {
        if let Some(ast) = crate::swift::parse_legacy_if(name) {
            return Some((Scheme::SwiftLegacy, Some(Parsed::Swift(ast))));
        }
        return crate::swift::legacy_claimed_bytes(name)
            .then_some((Scheme::SwiftLegacy, None));
    }
    Some((Scheme::SwiftStable, None))
}

#[cfg(all(feature = "rust", not(feature = "cpp-itanium")))]
fn is_rust_legacy_prefix_bytes(name: &[u8]) -> bool {
    name.len() > 3 && (name.starts_with(b"_ZN") || name.starts_with(b"__ZN"))
}

#[cfg(feature = "objc")]
fn has_objc_namespace_bytes(name: &[u8]) -> bool {
    name.starts_with(b"_OBJC_")
        || name.starts_with(b"__OBJC_")
        || name.starts_with(b"__objc_")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(feature = "cpp-gnu-v2")]
    #[test]
    fn legacy_cpp_detection_retains_the_tree() {
        let detection = detect(b"foo__Fiv");
        assert_eq!(detection.scheme, Scheme::CppGnuV2);
        assert!(detection.parsed.is_some());
        assert_eq!(detection.symbol, b"foo__Fiv");
        assert_eq!(detection.parse_symbol, b"foo__Fiv");
    }

    #[cfg(feature = "d")]
    #[test]
    fn d_detection_retains_the_tree() {
        let detection = detect(b"_D3foo3barFZv");
        assert_eq!(detection.scheme, Scheme::D);
        assert!(detection.parsed.is_some());
    }

    #[cfg(feature = "swift")]
    #[test]
    fn legacy_swift_detection_retains_the_tree() {
        let detection = detect(b"_TtSi");
        assert_eq!(detection.scheme, Scheme::SwiftLegacy);
        assert!(detection.parsed.is_some());
    }

    #[test]
    fn dispatch_view_keeps_stripped_suffixes() {
        let detection = detect(b"main.llvm.9D1C9369");
        assert_eq!(detection.scheme, Scheme::C);
        assert_eq!(detection.symbol, b"main.llvm.9D1C9369");
        assert_eq!(detection.parse_symbol, b"main");
    }

    #[cfg(feature = "coff")]
    #[cfg(feature = "cpp-gnu-v2")]
    #[test]
    fn coff_detection_retains_the_inner_tree_and_outer_decoration() {
        let detection = detect(b"__imp_foo__Fiv");
        assert_eq!(detection.scheme, Scheme::CppGnuV2);
        assert!(detection.parsed.is_some());
        assert_eq!(detection.symbol, b"foo__Fiv");
        assert_eq!(detection.parse_symbol, b"foo__Fiv");
        assert!(detection.decoration.is_some());
    }
}
