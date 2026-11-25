//! Private detection result shared by scheme detection and dispatch.
//!
//! Public [`Scheme::detect`](crate::Scheme::detect) keeps its current result;
//! `Demangler` can additionally reuse the parsed legacy tree instead of
//! parsing the same symbol again.

use crate::scheme::{
    Scheme, is_plain_c_name, scheme_for_cpp_dialect, strip_decorations_bytes,
};

/// A tree parsed while detecting a scheme, when that parse is also the one the
/// backend would perform.
#[allow(dead_code)] // Filled in by the dispatch handoff commit.
pub(crate) enum Parsed<'a> {
    #[cfg(any(
        feature = "cpp-itanium",
        feature = "cpp-msvc",
        feature = "cpp-gnu-v2"
    ))]
    Cpp(crate::cpp::Ast<'a>),
    #[cfg(feature = "swift")]
    Swift(crate::swift::Ast<'a>),
}

/// The result of one detection pass.
#[allow(dead_code)] // Fields are consumed by the dispatch handoff commit.
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
    if crate::d::is_d(parse_symbol) {
        return plain(Scheme::D, name, parse_symbol);
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
    if crate::delphi::is_delphi(parse_symbol) {
        return plain(Scheme::Delphi, name, parse_symbol);
    }

    #[cfg(feature = "ocaml")]
    if crate::ocaml::is_ocaml(parse_symbol) {
        return plain(Scheme::OCaml, name, parse_symbol);
    }

    #[cfg(feature = "go")]
    if crate::go::is_go_bytes(decorated) {
        return plain(Scheme::Go, name, parse_symbol);
    }

    #[cfg(feature = "jni")]
    if crate::jni::is_jni(parse_symbol) {
        return plain(Scheme::Jni, name, parse_symbol);
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
