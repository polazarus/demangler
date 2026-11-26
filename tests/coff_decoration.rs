//! PE/COFF decoration parser and `Demangler` integration tests.
//!
//! The hand-written tables pin the accepted and rejected spellings for both the
//! platform-agnostic entry points (`coff::parse`, used by the crate dispatcher)
//! and the target-aware entry points (`coff::parse_for`). The real corpus is
//! generated from COFF objects built by clang-19 and recorded in `refs/coff/`;
//! the corpus tests skip when that directory is absent, as it is from a
//! published crate tarball.

#![cfg(feature = "coff")]

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::string::String;
use std::vec::Vec;

use demangler::coff::{self, CallingConvention, DecorationKind, Fragment, Target};
use demangler::{Demangled, Demangler, Language, Scheme};

/// One row of the generated platform-agnostic corpus.
struct Row {
    symbol: String,
    decoration: Option<CallingConvention>,
    stack: Option<u16>,
    import: bool,
    inner: Option<String>,
    requires: String,
    scheme: ExpectedScheme,
    display: String,
}

/// One row of the generated target-aware corpus.
struct TargetRow {
    symbol: String,
    target: Target,
    kind: DecorationKind,
    convention: CallingConvention,
    stack: Option<u16>,
    import: bool,
    weak_alias: bool,
    fragment: Option<Fragment>,
    inner: String,
}

fn corpus_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("refs/coff")
}

fn parse_convention(value: &str) -> Option<CallingConvention> {
    match value {
        "none" => None,
        "cdecl" => Some(CallingConvention::Cdecl),
        "stdcall" => Some(CallingConvention::Stdcall),
        "fastcall" => Some(CallingConvention::Fastcall),
        "vectorcall" => Some(CallingConvention::Vectorcall),
        "unknown" => Some(CallingConvention::Unknown),
        other => panic!("unknown decoration {other}"),
    }
}

fn parse_kind(value: &str) -> DecorationKind {
    match value {
        "undecorated" => DecorationKind::Undecorated,
        "leadingunderscore" => DecorationKind::LeadingUnderscore,
        "conventionsuffix" => DecorationKind::ConventionSuffix,
        "import" => DecorationKind::Import,
        "weakalias" => DecorationKind::WeakAlias,
        "fragment" => DecorationKind::Fragment,
        other => panic!("unknown decoration kind {other}"),
    }
}

fn parse_fragment(value: &str) -> Option<Fragment> {
    if value == "-" {
        return None;
    }
    if value == "cold" {
        return Some(Fragment::Cold { index: None });
    }
    if let Some(index) = value.strip_prefix("cold.") {
        return Some(Fragment::Cold {
            index: Some(
                index
                    .parse()
                    .unwrap_or_else(|_| panic!("bad fragment {value}")),
            ),
        });
    }
    if let Some(index) = value.strip_prefix("part.") {
        return Some(Fragment::Part {
            index: index
                .parse()
                .unwrap_or_else(|_| panic!("bad fragment {value}")),
        });
    }
    panic!("unknown fragment {value}");
}

fn parse_target(value: &str) -> Target {
    match value {
        "x86" => Target::X86,
        "x64" => Target::X64,
        "arm64" => Target::Arm64,
        other => panic!("unknown target {other}"),
    }
}

/// The scheme a corpus row expects.
///
/// The vendored corpus records the coarse family in its `scheme` column; a
/// C++ row names its dialect through the `requires` column and a Rust row
/// records no encoding, so those rows are checked at the language level.
enum ExpectedScheme {
    /// The exact precise scheme.
    Exact(Scheme),
    /// The language group; the precise scheme may be any of its members.
    Language(Language),
}

impl ExpectedScheme {
    /// Assert that the demangled result reports this scheme.
    fn assert_reported(&self, demangled: &Demangled<'_>, symbol: &str) {
        match self {
            Self::Exact(scheme) => assert_eq!(demangled.scheme(), *scheme, "{symbol}"),
            Self::Language(language) => assert_eq!(demangled.language(), *language, "{symbol}"),
        }
    }

    /// Assert that the demangled result does not report this scheme.
    fn assert_not_reported(&self, demangled: &Demangled<'_>, symbol: &str) {
        match self {
            Self::Exact(scheme) => assert_ne!(demangled.scheme(), *scheme, "{symbol}"),
            Self::Language(language) => assert_ne!(demangled.language(), *language, "{symbol}"),
        }
    }
}

fn parse_scheme(value: &str, requires: &str) -> ExpectedScheme {
    match value {
        "C" => ExpectedScheme::Exact(Scheme::C),
        // The corpus records the coarse C++ family; the `requires` column is
        // the dialect the row was generated from.
        "Cpp" => match requires {
            "cpp-itanium" => ExpectedScheme::Exact(Scheme::CppItanium),
            "cpp-msvc" => ExpectedScheme::Exact(Scheme::CppMsvc),
            other => panic!("C++ corpus row without a single dialect: {other}"),
        },
        // A Rust row does not record its encoding, so it is checked at the
        // language level.
        "Rust" => ExpectedScheme::Language(Language::Rust),
        "Go" => ExpectedScheme::Exact(Scheme::Go),
        "Unknown" => ExpectedScheme::Exact(Scheme::Unknown),
        other => panic!("unknown scheme {other}"),
    }
}

fn parse_stack(value: &str) -> Option<u16> {
    if value == "-" {
        None
    } else {
        Some(
            value
                .parse()
                .unwrap_or_else(|_| panic!("bad stack {value}")),
        )
    }
}

fn read_corpus() -> Option<Vec<Row>> {
    let path = corpus_dir().join("symbols.tsv");
    let Ok(text) = fs::read_to_string(&path) else {
        eprintln!("skipping COFF corpus: {} is absent", path.display());
        return None;
    };
    let mut rows = Vec::new();
    for (index, line) in text.lines().enumerate() {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let fields: Vec<&str> = line.split('\t').collect();
        assert_eq!(fields.len(), 9, "line {}", index + 1);
        let decoration = parse_convention(fields[1]);
        match decoration {
            None => assert_eq!(fields[2], "-", "line {}", index + 1),
            Some(convention) => assert_eq!(
                parse_convention(fields[2]),
                Some(convention),
                "line {}",
                index + 1
            ),
        }
        let requires = String::from(fields[6]);
        rows.push(Row {
            symbol: String::from(fields[0]),
            decoration,
            stack: parse_stack(fields[3]),
            import: fields[4] == "yes",
            inner: (fields[5] != "-").then(|| String::from(fields[5])),
            scheme: parse_scheme(fields[7], &requires),
            requires,
            display: String::from(fields[8]),
        });
    }
    Some(rows)
}

fn read_target_corpus() -> Option<Vec<TargetRow>> {
    let path = corpus_dir().join("targets.tsv");
    let Ok(text) = fs::read_to_string(&path) else {
        eprintln!("skipping COFF target corpus: {} is absent", path.display());
        return None;
    };
    let mut rows = Vec::new();
    for (index, line) in text.lines().enumerate() {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let fields: Vec<&str> = line.split('\t').collect();
        assert_eq!(fields.len(), 9, "line {}", index + 1);
        rows.push(TargetRow {
            symbol: String::from(fields[0]),
            target: parse_target(fields[1]),
            kind: parse_kind(fields[2]),
            convention: parse_convention(fields[3])
                .unwrap_or_else(|| panic!("line {}: no convention", index + 1)),
            stack: parse_stack(fields[4]),
            import: fields[5] == "yes",
            weak_alias: fields[6] == "yes",
            fragment: parse_fragment(fields[7]),
            inner: String::from(fields[8]),
        });
    }
    Some(rows)
}

fn feature_enabled(requires: &str) -> bool {
    match requires {
        "-" => true,
        "cpp-itanium" => cfg!(feature = "cpp-itanium"),
        "cpp-msvc" => cfg!(feature = "cpp-msvc"),
        "rust" => cfg!(feature = "rust"),
        // Go claims dotted names, so a fragment of a bare x86 C name is Go
        // text that happens to be unchanged.
        "go" => cfg!(feature = "go"),
        other => panic!("unknown feature requirement {other}"),
    }
}

fn check_decoration(row: &Row) {
    let parsed = coff::parse(&row.symbol);
    match row.decoration {
        None => {
            assert!(parsed.is_none(), "{}", row.symbol);
            assert!(!coff::is_coff(row.symbol.as_bytes()), "{}", row.symbol);
        }
        Some(convention) => {
            let decoration = parsed.unwrap_or_else(|| panic!("{}", row.symbol));
            assert_eq!(
                decoration.symbol(),
                row.inner.as_deref().unwrap_or("").as_bytes(),
                "{}",
                row.symbol
            );
            assert_eq!(decoration.convention(), convention, "{}", row.symbol);
            assert_eq!(decoration.stack_bytes(), row.stack, "{}", row.symbol);
            assert_eq!(decoration.import(), row.import, "{}", row.symbol);
            assert!(!decoration.weak_alias(), "{}", row.symbol);
            assert!(coff::is_coff(row.symbol.as_bytes()), "{}", row.symbol);
        }
    }
}

fn check_target_decoration(row: &TargetRow) {
    let parsed = coff::parse_bytes_for(row.symbol.as_bytes(), row.target)
        .unwrap_or_else(|| panic!("{} {:?}", row.symbol, row.target));
    assert_eq!(parsed.symbol(), row.inner.as_bytes(), "{}", row.symbol);
    assert_eq!(parsed.kind(), row.kind, "{}", row.symbol);
    assert_eq!(parsed.convention(), row.convention, "{}", row.symbol);
    assert_eq!(parsed.stack_bytes(), row.stack, "{}", row.symbol);
    assert_eq!(parsed.import(), row.import, "{}", row.symbol);
    assert_eq!(parsed.weak_alias(), row.weak_alias, "{}", row.symbol);
    assert_eq!(parsed.fragment(), row.fragment, "{}", row.symbol);
}

#[test]
fn decodes_supported_decorations() {
    for (symbol, inner, convention, stack, import) in [
        ("_foo@8", "foo", CallingConvention::Stdcall, Some(8), false),
        ("_foo@0", "foo", CallingConvention::Stdcall, Some(0), false),
        (
            "_foo@65535",
            "foo",
            CallingConvention::Stdcall,
            Some(u16::MAX),
            false,
        ),
        ("@foo@8", "foo", CallingConvention::Fastcall, Some(8), false),
        ("__imp_foo", "foo", CallingConvention::Cdecl, None, true),
        ("__imp__foo", "foo", CallingConvention::Cdecl, None, true),
        (
            "__imp__foo@8",
            "foo",
            CallingConvention::Stdcall,
            Some(8),
            true,
        ),
        (
            "__imp_@foo@8",
            "foo",
            CallingConvention::Fastcall,
            Some(8),
            true,
        ),
        (
            "__imp_?foo@@YAXXZ",
            "?foo@@YAXXZ",
            CallingConvention::Cdecl,
            None,
            true,
        ),
        (
            "__imp__Z3foov",
            "_Z3foov",
            CallingConvention::Cdecl,
            None,
            true,
        ),
        (
            "__imp__RNvCs1234_4test4main",
            "_RNvCs1234_4test4main",
            CallingConvention::Cdecl,
            None,
            true,
        ),
        (
            "__imp___Z12imported_cppi",
            "_Z12imported_cppi",
            CallingConvention::Cdecl,
            None,
            true,
        ),
        ("__imp__Zone", "Zone", CallingConvention::Cdecl, None, true),
    ] {
        let parsed = coff::parse(symbol).unwrap_or_else(|| panic!("{symbol}"));
        assert_eq!(parsed.symbol(), inner.as_bytes(), "{symbol}");
        assert_eq!(parsed.convention(), convention, "{symbol}");
        assert_eq!(parsed.stack_bytes(), stack, "{symbol}");
        assert_eq!(parsed.import(), import, "{symbol}");
        assert!(!parsed.weak_alias(), "{symbol}");
        assert!(coff::is_coff(symbol.as_bytes()), "{symbol}");
    }

    const EIGHT: &str = concat!(
        "__imp_", "__imp_", "__imp_", "__imp_", "__imp_", "__imp_", "__imp_", "__imp_", "foo",
    );
    let nested = coff::parse(EIGHT).unwrap();
    assert_eq!(nested.symbol(), b"foo");
    assert!(nested.import());
}

#[test]
fn rejects_malformed_decorations() {
    for symbol in [
        "",
        "_foo",
        "foo@8",
        "@foo",
        "_foo@",
        "_foo@x",
        "_foo@8x",
        "_foo@@8",
        "_foo@8@",
        "_foo@8@9",
        "_foo@65536",
        "_foo@18446744073709551616",
        "@foo@",
        "@@foo@8",
        "@foo@8@9",
        "__imp_",
        "__imp__",
        "__imp__foo@",
        "__imp__foo@8x",
        "__imp__Z3foov@@16",
        "__imp__foo@@GLIBC_2.2.5",
        "__imp__ZN4test4main17h1234567890abcdefE@@16",
        "coff_vectorcall@@8",
        "__imp_coff_vectorcall@@8",
        "main@@GLIBC_2.2.5",
        "main@VERS_1.0",
        "_ZN4test4main17h1234567890abcdefE@@16",
        "__Z3foov.llvm.9D1C9369@@16",
        ".weak._weak_cdecl.default",
        ".weak._weak_stdcall@8.default",
    ] {
        assert!(coff::parse(symbol).is_none(), "{symbol}");
        assert!(!coff::is_coff(symbol.as_bytes()), "{symbol}");
    }

    // Nested import prefixes have no count cap: each consumes input.
    const NINE: &str = concat!(
        "__imp_", "__imp_", "__imp_", "__imp_", "__imp_", "__imp_", "__imp_", "__imp_", "__imp_",
        "foo",
    );
    assert_eq!(coff::parse(NINE).map(|d| d.symbol()), Some(&b"foo"[..]));
}

#[test]
fn decodes_vectorcall_for_declared_targets() {
    for (symbol, target, inner, stack) in [
        ("coff_vectorcall@@8", Target::X86, "coff_vectorcall", 8),
        (
            "coff_vectorcall_none@@0",
            Target::X86,
            "coff_vectorcall_none",
            0,
        ),
        (
            "coff_vectorcall_three@@12",
            Target::X86,
            "coff_vectorcall_three",
            12,
        ),
        ("coff_vectorcall@@16", Target::X64, "coff_vectorcall", 16),
        (
            "coff_vectorcall_one@@8",
            Target::X64,
            "coff_vectorcall_one",
            8,
        ),
        (
            "coff_vectorcall_three@@24",
            Target::X64,
            "coff_vectorcall_three",
            24,
        ),
    ] {
        let parsed = coff::parse_for(symbol, target).unwrap_or_else(|| panic!("{symbol}"));
        assert_eq!(parsed.symbol(), inner.as_bytes(), "{symbol}");
        assert_eq!(parsed.kind(), DecorationKind::ConventionSuffix, "{symbol}");
        assert_eq!(
            parsed.convention(),
            CallingConvention::Vectorcall,
            "{symbol}"
        );
        assert_eq!(parsed.stack_bytes(), Some(stack), "{symbol}");
        assert!(!parsed.import(), "{symbol}");
        assert!(!parsed.weak_alias(), "{symbol}");
    }

    // An import pointer composes with the vectorcall suffix.
    let imported = coff::parse_for("__imp_imported_vectorcall@@16", Target::X64).unwrap();
    assert_eq!(imported.symbol(), b"imported_vectorcall");
    assert_eq!(imported.kind(), DecorationKind::Import);
    assert_eq!(imported.convention(), CallingConvention::Vectorcall);
    assert_eq!(imported.stack_bytes(), Some(16));
    assert!(imported.import());
}

#[test]
fn decodes_arm64_and_x64_undecorated_names() {
    for (symbol, target) in [
        ("arm_plain", Target::Arm64),
        ("arm_stdcall", Target::Arm64),
        ("arm_fastcall", Target::Arm64),
        ("call_arm", Target::Arm64),
        ("?cpp@@YAXXZ", Target::X64),
        ("_Z3foov", Target::X64),
    ] {
        let parsed = coff::parse_for(symbol, target).unwrap_or_else(|| panic!("{symbol}"));
        assert_eq!(parsed.symbol(), symbol.as_bytes(), "{symbol}");
        assert_eq!(parsed.kind(), DecorationKind::Undecorated, "{symbol}");
        assert_eq!(parsed.convention(), CallingConvention::Unknown, "{symbol}");
        assert_eq!(parsed.stack_bytes(), None, "{symbol}");
        assert!(!parsed.import(), "{symbol}");
    }

    let imported = coff::parse_for("__imp_imported_arm", Target::Arm64).unwrap();
    assert_eq!(imported.symbol(), b"imported_arm");
    assert_eq!(imported.kind(), DecorationKind::Import);
    assert_eq!(imported.convention(), CallingConvention::Cdecl);
    assert!(imported.import());
}

#[test]
fn decodes_leading_underscore_with_the_explicit_helper() {
    let parsed = coff::parse_leading_underscore("_coff_cdecl").unwrap();
    assert_eq!(parsed.symbol(), b"coff_cdecl");
    assert_eq!(parsed.kind(), DecorationKind::LeadingUnderscore);
    assert_eq!(parsed.convention(), CallingConvention::Cdecl);
    assert_eq!(parsed.stack_bytes(), None);
    assert!(!parsed.import());

    // The x86 target rules decode the same form, and keep the Itanium
    // underscore of the MinGW spelling.
    let x86 = coff::parse_for("_coff_cdecl", Target::X86).unwrap();
    assert_eq!(x86.symbol(), b"coff_cdecl");
    assert_eq!(x86.kind(), DecorationKind::LeadingUnderscore);
    let itanium = coff::parse_for("__Z17call_imported_cppi", Target::X86).unwrap();
    assert_eq!(itanium.symbol(), b"_Z17call_imported_cppi");

    for symbol in ["", "foo", "_", "_foo@8", "_foo@", "_foo@bar", "__Z3foov@8"] {
        assert!(coff::parse_leading_underscore(symbol).is_none(), "{symbol}");
    }
}

#[test]
fn decodes_comdat_weak_aliases() {
    for (symbol, target, inner, convention, stack) in [
        (
            ".weak._weak_cdecl.default",
            Target::X86,
            "weak_cdecl",
            CallingConvention::Cdecl,
            None,
        ),
        (
            ".weak._weak_stdcall@8.default",
            Target::X86,
            "weak_stdcall",
            CallingConvention::Stdcall,
            Some(8),
        ),
        (
            ".weak.@weak_fastcall@8.default",
            Target::X86,
            "weak_fastcall",
            CallingConvention::Fastcall,
            Some(8),
        ),
        (
            ".weak.weak_stdcall.default",
            Target::X64,
            "weak_stdcall",
            CallingConvention::Unknown,
            None,
        ),
        (
            ".weak.weak_fastcall.default",
            Target::Arm64,
            "weak_fastcall",
            CallingConvention::Unknown,
            None,
        ),
    ] {
        let parsed = coff::parse_for(symbol, target).unwrap_or_else(|| panic!("{symbol}"));
        assert_eq!(parsed.symbol(), inner.as_bytes(), "{symbol}");
        assert_eq!(parsed.kind(), DecorationKind::WeakAlias, "{symbol}");
        assert_eq!(parsed.convention(), convention, "{symbol}");
        assert_eq!(parsed.stack_bytes(), stack, "{symbol}");
        assert!(parsed.weak_alias(), "{symbol}");
        assert!(!parsed.import(), "{symbol}");
    }

    // The irregular associative spelling clang emits for some C++ weak
    // COMDATs is rejected rather than guessed.
    for symbol in [
        ".weak.",
        ".weak.foo",
        ".weak..default",
        ".weak.?weak_cpp@@YAHH@Z.default.??1WeakClass@@UEAA@XZ",
    ] {
        assert!(coff::parse_for(symbol, Target::X64).is_none(), "{symbol}");
    }
}

#[test]
fn rejects_malformed_target_decorations() {
    for (symbol, target) in [
        ("", Target::X86),
        ("", Target::X64),
        ("", Target::Arm64),
        ("__imp_", Target::X86),
        ("_foo@", Target::X86),
        ("_foo@x", Target::X86),
        ("_foo@65536", Target::X86),
        ("_foo@18446744073709551616", Target::X86),
        ("@foo@8@9", Target::X86),
        ("_foo@@8", Target::X86),
        ("@@foo@8", Target::X86),
        ("foo@@", Target::X86),
        ("foo@@x", Target::X64),
        ("_foo@@8", Target::X64),
        ("_foo@8", Target::X64),
        ("@foo@8", Target::X64),
    ] {
        assert!(
            coff::parse_for(symbol, target).is_none(),
            "{symbol} {target:?}"
        );
    }
}

#[test]
fn decodes_cold_and_part_fragments() {
    // The fragment suffix is appended after every other decoration, so it is
    // the outermost marker and the base name keeps its own rules.
    for (symbol, target, inner, convention, stack, fragment) in [
        (
            "_coff_stdcall@8.cold.1",
            Target::X86,
            "coff_stdcall",
            CallingConvention::Stdcall,
            Some(8),
            Fragment::Cold { index: Some(1) },
        ),
        (
            "_cold_plain.cold.1",
            Target::X86,
            "cold_plain",
            CallingConvention::Cdecl,
            None,
            Fragment::Cold { index: Some(1) },
        ),
        (
            "__Z11cpp_stdcallii.cold.1",
            Target::X86,
            "_Z11cpp_stdcallii",
            CallingConvention::Cdecl,
            None,
            Fragment::Cold { index: Some(1) },
        ),
        (
            "cold_plain.cold.1",
            Target::X64,
            "cold_plain",
            CallingConvention::Unknown,
            None,
            Fragment::Cold { index: Some(1) },
        ),
        (
            "?cold_cpp@@YAHH@Z.cold.1",
            Target::X64,
            "?cold_cpp@@YAHH@Z",
            CallingConvention::Unknown,
            None,
            Fragment::Cold { index: Some(1) },
        ),
        (
            "cold_plain.cold",
            Target::Arm64,
            "cold_plain",
            CallingConvention::Unknown,
            None,
            Fragment::Cold { index: None },
        ),
        (
            "cold_plain.part.0",
            Target::Arm64,
            "cold_plain",
            CallingConvention::Unknown,
            None,
            Fragment::Part { index: 0 },
        ),
    ] {
        let parsed = coff::parse_for(symbol, target).unwrap_or_else(|| panic!("{symbol}"));
        assert_eq!(parsed.symbol(), inner.as_bytes(), "{symbol}");
        assert_eq!(parsed.kind(), DecorationKind::Fragment, "{symbol}");
        assert_eq!(parsed.convention(), convention, "{symbol}");
        assert_eq!(parsed.stack_bytes(), stack, "{symbol}");
        assert_eq!(parsed.fragment(), Some(fragment), "{symbol}");
        assert!(!parsed.import(), "{symbol}");
        assert!(!parsed.weak_alias(), "{symbol}");
    }
}

#[test]
fn rejects_malformed_fragments() {
    for (symbol, target) in [
        // An empty base is a section name, not a fragment of a symbol.
        (".cold", Target::X86),
        (".cold.1", Target::X86),
        (".part.0", Target::X86),
        // An empty, non-decimal or oversized index is not a fragment.
        ("cold_plain.cold.", Target::X86),
        ("cold_plain.cold.x", Target::X86),
        ("cold_plain.cold.65536", Target::X86),
        ("cold_plain.part", Target::X86),
        ("cold_plain.part.", Target::X86),
        ("cold_plain.part.65536", Target::X86),
        // Only one fragment suffix is decoded.
        ("cold_plain.cold.1.cold.2", Target::X86),
        ("cold_plain.part.0.cold.1", Target::X86),
        // The irregular COMDAT association spelling stays rejected.
        (
            ".weak.?weak_cpp@@YAHH@Z.default.??1WeakClass@@UEAA@XZ.cold.1",
            Target::X64,
        ),
    ] {
        assert!(
            coff::parse_for(symbol, target).is_none(),
            "{symbol} {target:?}"
        );
    }
}

#[test]
fn fragments_are_not_decoded_without_a_declared_target() {
    // A fragment is language-neutral: the platform-agnostic path must leave it
    // to the language backend, which may still demangle the whole symbol.
    for symbol in [
        "cold_plain.cold.1",
        "_cold_plain.cold.1",
        "_coff_stdcall@8.cold.1",
        "?cold_cpp@@YAHH@Z.cold.1",
        "__Z11cpp_stdcallii.cold.1",
        "cold_plain.part.0",
    ] {
        assert!(coff::parse(symbol).is_none(), "{symbol}");
        assert!(!coff::is_coff(symbol.as_bytes()), "{symbol}");
        let demangled = Demangler::new()
            .demangle(symbol)
            .unwrap_or_else(|error| panic!("{symbol}: {error}"));
        assert!(demangled.coff_decoration().is_none(), "{symbol}");
    }

    // The Itanium backend keeps the clone suffix in its own spelling.
    #[cfg(feature = "cpp-itanium")]
    assert_eq!(
        Demangler::new()
            .demangle("_Z11cpp_stdcallii.cold.1")
            .unwrap()
            .to_string(),
        "cpp_stdcall(int, int) (.cold.1)"
    );
}

#[test]
fn thiscall_has_no_free_standing_decoration() {
    // MSVC encodes `thiscall` inside the `?` mangling; there is no C
    // decoration for it, so no spelling is decoded as thiscall. The `@N` form
    // stays `stdcall`.
    let decorated = coff::parse_for("_coff_thiscall@4", Target::X86).unwrap();
    assert_eq!(decorated.convention(), CallingConvention::Stdcall);
    assert_eq!(decorated.stack_bytes(), Some(4));
    let mangled = coff::parse_for("?coff_thiscall@Class@@QAEXXZ", Target::X86).unwrap();
    assert_eq!(mangled.kind(), DecorationKind::Undecorated);
    assert_eq!(mangled.convention(), CallingConvention::Unknown);
}

#[test]
fn demangler_applies_c_decorations_without_annotations() {
    let demangler = Demangler::new();
    for (symbol, text, convention, stack, import) in [
        (
            "_coff_stdcall@8",
            "coff_stdcall",
            CallingConvention::Stdcall,
            Some(8),
            false,
        ),
        (
            "@coff_fastcall@8",
            "coff_fastcall",
            CallingConvention::Fastcall,
            Some(8),
            false,
        ),
        (
            "__imp__imported_stdcall@8",
            "imported_stdcall",
            CallingConvention::Stdcall,
            Some(8),
            true,
        ),
        (
            "__imp_@imported_fastcall@8",
            "imported_fastcall",
            CallingConvention::Fastcall,
            Some(8),
            true,
        ),
        (
            "__imp__imported_cdecl",
            "imported_cdecl",
            CallingConvention::Cdecl,
            None,
            true,
        ),
        ("__imp__Zone", "Zone", CallingConvention::Cdecl, None, true),
    ] {
        let demangled = demangler
            .demangle(symbol)
            .unwrap_or_else(|error| panic!("{symbol}: {error}"));
        assert_eq!(demangled.scheme(), Scheme::C, "{symbol}");
        assert_eq!(demangled.to_string(), text, "{symbol}");
        let decoration = demangled
            .coff_decoration()
            .unwrap_or_else(|| panic!("{symbol}"));
        assert_eq!(decoration.convention(), convention, "{symbol}");
        assert_eq!(decoration.stack_bytes(), stack, "{symbol}");
        assert_eq!(decoration.import(), import, "{symbol}");
        // Decoration metadata is not appended to the display text.
        assert!(!demangled.to_string().contains('@'), "{symbol}");
        assert!(!demangled.to_string().contains("__imp_"), "{symbol}");
    }
}

#[test]
fn demangler_recurses_into_imported_mangled_names() {
    let demangler = Demangler::new();

    #[cfg(feature = "cpp-itanium")]
    {
        for symbol in [
            "__imp__Z3foov",
            "__imp___imp__Z3foov",
            "__imp__ZN3Foo3barEv",
        ] {
            let demangled = demangler
                .demangle(symbol)
                .unwrap_or_else(|error| panic!("{symbol}: {error}"));
            assert_eq!(demangled.scheme(), Scheme::CppItanium, "{symbol}");
            assert!(demangled.coff_decoration().is_some(), "{symbol}");
        }
        assert_eq!(
            demangler.demangle("__imp__Z3foov").unwrap().to_string(),
            "foo()"
        );
        assert_eq!(
            demangler
                .demangle("__imp___imp__Z3foov")
                .unwrap()
                .to_string(),
            "foo()"
        );
        assert_eq!(
            demangler
                .demangle("__imp__ZN3Foo3barEv")
                .unwrap()
                .to_string(),
            "Foo::bar()"
        );
    }
    #[cfg(not(feature = "cpp-itanium"))]
    {
        let demangled = demangler.demangle("__imp__Z3foov").unwrap();
        assert_eq!(demangled.to_string(), "_Z3foov");
        assert_eq!(demangled.coff_decoration().unwrap().symbol(), b"_Z3foov");
    }

    #[cfg(feature = "cpp-msvc")]
    {
        let demangled = demangler.demangle("__imp_?imported_cpp@@YAHH@Z").unwrap();
        assert_eq!(demangled.scheme(), Scheme::CppMsvc);
        assert_eq!(demangled.to_string(), "int __cdecl imported_cpp(int)");
        assert_eq!(
            demangled.coff_decoration().unwrap().symbol(),
            b"?imported_cpp@@YAHH@Z"
        );
    }
    #[cfg(not(feature = "cpp-msvc"))]
    {
        let demangled = demangler.demangle("__imp_?imported_cpp@@YAHH@Z").unwrap();
        assert_eq!(demangled.to_string(), "?imported_cpp@@YAHH@Z");
        assert!(demangled.coff_decoration().is_some());
    }

    #[cfg(feature = "rust")]
    {
        let v0 = demangler.demangle("__imp__RNvCs1234_4test4main").unwrap();
        assert_eq!(v0.scheme(), Scheme::RustV0);
        assert_eq!(v0.to_string(), "test[3c1c0]::main");
        assert_eq!(
            v0.coff_decoration().unwrap().symbol(),
            b"_RNvCs1234_4test4main"
        );

        let legacy = demangler
            .demangle("__imp__ZN4test4main17h1234567890abcdefE")
            .unwrap();
        assert_eq!(legacy.scheme(), Scheme::RustLegacy);
        assert_eq!(legacy.to_string(), "test::main::h1234567890abcdef");
    }
    #[cfg(not(feature = "rust"))]
    {
        let demangled = demangler.demangle("__imp__RNvCs1234_4test4main").unwrap();
        assert_eq!(demangled.to_string(), "_RNvCs1234_4test4main");
        assert!(demangled.coff_decoration().is_some());
    }
}

#[test]
fn raw_byte_symbols_keep_the_identifier_bytes() {
    let parsed = coff::parse_bytes(b"_f\xff@8").unwrap();
    assert_eq!(parsed.symbol(), b"f\xff");
    assert_eq!(parsed.convention(), CallingConvention::Stdcall);
    assert_eq!(parsed.stack_bytes(), Some(8));

    let demangled = Demangler::new().demangle_bytes(b"_f\xff@8").unwrap();
    assert_eq!(demangled.scheme(), Scheme::Unknown);
    assert_eq!(demangled.to_string(), "f\\xff");
    let decoration = demangled.coff_decoration().unwrap();
    assert_eq!(decoration.symbol(), b"f\xff");
    assert_eq!(decoration.convention(), CallingConvention::Stdcall);
}

#[test]
fn gnu_versions_and_thin_lto_suffixes_are_not_coff() {
    let demangler = Demangler::new();
    for symbol in [
        "_ZN4test4main17h1234567890abcdefE@@16",
        "__Z3foov.llvm.9D1C9369@@16",
        "_Z3foov@@GLIBCXX_3.4",
        "main@@GLIBC_2.2.5",
        "main.llvm.9D1C9369",
    ] {
        assert!(coff::parse(symbol).is_none(), "{symbol}");
        assert!(!coff::is_coff(symbol.as_bytes()), "{symbol}");
        let demangled = demangler
            .demangle(symbol)
            .unwrap_or_else(|error| panic!("{symbol}: {error}"));
        assert!(demangled.coff_decoration().is_none(), "{symbol}");
    }

    let main = demangler.demangle("main@@GLIBC_2.2.5").unwrap();
    assert_eq!(main.scheme(), Scheme::C);
    assert_eq!(main.to_string(), "main@@GLIBC_2.2.5");

    #[cfg(feature = "rust")]
    {
        let rust = demangler
            .demangle("_ZN4test4main17h1234567890abcdefE@@16")
            .unwrap();
        assert_eq!(rust.scheme(), Scheme::RustLegacy);
        assert_eq!(rust.to_string(), "test::main::h1234567890abcdef");
    }

    #[cfg(feature = "cpp-itanium")]
    {
        for symbol in ["__Z3foov.llvm.9D1C9369@@16", "_Z3foov@@GLIBCXX_3.4"] {
            let demangled = demangler
                .demangle(symbol)
                .unwrap_or_else(|error| panic!("{symbol}: {error}"));
            assert_eq!(demangled.scheme(), Scheme::CppItanium, "{symbol}");
            assert_eq!(demangled.to_string(), "foo()", "{symbol}");
        }
    }
}

#[test]
fn target_rules_do_not_change_global_dispatch() {
    // The ambiguous forms the target-aware entry points decode stay
    // undecorated in the crate dispatcher, so ELF versions and pass-through
    // text are unchanged.
    let demangler = Demangler::new();
    for (symbol, text) in [
        ("_coff_cdecl", "_coff_cdecl"),
        ("coff_vectorcall@@8", "coff_vectorcall@@8"),
        ("coff_vectorcall@@16", "coff_vectorcall@@16"),
        (
            "__imp_imported_vectorcall@@8",
            "__imp_imported_vectorcall@@8",
        ),
        (".weak._weak_cdecl.default", ".weak._weak_cdecl.default"),
        (
            ".weak._weak_stdcall@8.default",
            ".weak._weak_stdcall@8.default",
        ),
    ] {
        assert!(coff::parse(symbol).is_none(), "{symbol}");
        let demangled = demangler
            .demangle(symbol)
            .unwrap_or_else(|error| panic!("{symbol}: {error}"));
        assert!(demangled.coff_decoration().is_none(), "{symbol}");
        assert_eq!(demangled.to_string(), text, "{symbol}");
        // The target-aware entry points decode the same bytes.
        let target = if symbol.starts_with(".weak.weak_") {
            Target::X64
        } else if symbol.starts_with(".weak.") {
            Target::X86
        } else if symbol.contains("@@16") {
            Target::X64
        } else {
            Target::X86
        };
        assert!(
            coff::parse_bytes_for(symbol.as_bytes(), target).is_some(),
            "{symbol}"
        );
    }
}

#[test]
fn real_corpus_metadata() {
    let Some(rows) = read_corpus() else {
        return;
    };
    for row in &rows {
        check_decoration(row);
    }
}

#[test]
fn real_corpus_target_metadata() {
    let Some(rows) = read_target_corpus() else {
        return;
    };
    for row in &rows {
        check_target_decoration(row);
    }
}

#[test]
fn real_corpus_integration() {
    let Some(rows) = read_corpus() else {
        return;
    };
    let demangler = Demangler::new();
    for row in &rows {
        let demangled = demangler
            .demangle(&row.symbol)
            .unwrap_or_else(|error| panic!("{}: {error}", row.symbol));
        match row.decoration {
            None => assert!(demangled.coff_decoration().is_none(), "{}", row.symbol),
            Some(convention) => {
                let decoration = demangled
                    .coff_decoration()
                    .unwrap_or_else(|| panic!("{}", row.symbol));
                assert_eq!(decoration.convention(), convention, "{}", row.symbol);
                assert_eq!(decoration.stack_bytes(), row.stack, "{}", row.symbol);
                assert_eq!(decoration.import(), row.import, "{}", row.symbol);
            }
        }

        if feature_enabled(&row.requires) {
            row.scheme.assert_reported(&demangled, &row.symbol);
            assert_eq!(demangled.to_string(), row.display, "{}", row.symbol);
        } else {
            // Without the language backend the undecorated name is a plain
            // pass-through, but the decoration is still reported.
            let expected = row.inner.as_deref().unwrap_or(&row.symbol);
            assert_eq!(demangled.to_string(), expected, "{}", row.symbol);
            row.scheme.assert_not_reported(&demangled, &row.symbol);
        }
    }
}

/// External symbols recorded in the vendored symbol-table transcripts.
///
/// `objdump -t` marks external symbols with storage class 2 and weak externals
/// with 105; the ARM64 object is recorded with `llvm-nm` because GNU objdump
/// does not read ARM64 COFF.
fn recorded_symbols(directory: &Path) -> Vec<String> {
    let mut found = Vec::new();
    let Ok(entries) = fs::read_dir(directory) else {
        return found;
    };
    for entry in entries {
        let path = entry.unwrap().path();
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default();
        if name.ends_with(".objdump.txt") {
            let text = fs::read_to_string(&path).unwrap();
            for line in text.lines() {
                if !(line.contains("(scl   2)") || line.contains("(scl 105)")) {
                    continue;
                }
                let Some(symbol) = line.split_whitespace().last() else {
                    continue;
                };
                if symbol == "@feat.00" {
                    continue;
                }
                found.push(String::from(symbol));
            }
        } else if name.ends_with(".llvm-nm.txt") {
            let text = fs::read_to_string(&path).unwrap();
            for line in text.lines() {
                let Some(symbol) = line.split_whitespace().last() else {
                    continue;
                };
                if symbol == "@feat.00" {
                    continue;
                }
                found.push(String::from(symbol));
            }
        }
    }
    found
}

#[test]
fn corpus_covers_every_external_object_symbol() {
    let Some(rows) = read_corpus() else {
        return;
    };
    let Some(target_rows) = read_target_corpus() else {
        return;
    };
    let recorded: BTreeSet<&str> = rows.iter().map(|row| row.symbol.as_str()).collect();
    let targeted: BTreeSet<&str> = target_rows.iter().map(|row| row.symbol.as_str()).collect();
    let found = recorded_symbols(&corpus_dir());
    assert!(!found.is_empty(), "no object symbols found");
    for symbol in found {
        assert!(recorded.contains(symbol.as_str()), "missing {symbol}");
        assert!(
            targeted.contains(symbol.as_str()),
            "missing target {symbol}"
        );
    }
}
