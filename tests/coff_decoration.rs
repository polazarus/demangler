//! PE/COFF decoration parser and `Demangler` integration tests.
//!
//! The hand-written table pins the accepted and rejected spellings. The real
//! corpus is generated from 32-bit COFF objects built by clang-19 and recorded
//! in `refs/coff/`; the corpus tests skip when that directory is absent, as it
//! is from a published crate tarball.

#![cfg(feature = "coff")]

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::string::String;
use std::vec::Vec;

use demangle::coff::{self, CallingConvention};
use demangle::{Demangler, Scheme};

/// One row of the generated corpus.
struct Row {
    symbol: String,
    decoration: Option<CallingConvention>,
    stack: Option<u16>,
    import: bool,
    inner: Option<String>,
    requires: String,
    scheme: Scheme,
    display: String,
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
        other => panic!("unknown decoration {other}"),
    }
}

fn parse_scheme(value: &str) -> Scheme {
    match value {
        "C" => Scheme::C,
        "Cpp" => Scheme::Cpp,
        "Rust" => Scheme::Rust,
        "Unknown" => Scheme::Unknown,
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
        rows.push(Row {
            symbol: String::from(fields[0]),
            decoration,
            stack: parse_stack(fields[3]),
            import: fields[4] == "yes",
            inner: (fields[5] != "-").then(|| String::from(fields[5])),
            requires: String::from(fields[6]),
            scheme: parse_scheme(fields[7]),
            display: String::from(fields[8]),
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
            assert!(coff::is_coff(row.symbol.as_bytes()), "{}", row.symbol);
        }
    }
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
    ] {
        assert!(coff::parse(symbol).is_none(), "{symbol}");
        assert!(!coff::is_coff(symbol.as_bytes()), "{symbol}");
    }

    const NINE: &str = concat!(
        "__imp_", "__imp_", "__imp_", "__imp_", "__imp_", "__imp_", "__imp_", "__imp_", "__imp_",
        "foo",
    );
    assert!(coff::parse(NINE).is_none(), "{NINE}");
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
            assert_eq!(demangled.scheme(), Scheme::Cpp, "{symbol}");
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
        assert_eq!(demangled.scheme(), Scheme::Cpp);
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
        assert_eq!(v0.scheme(), Scheme::Rust);
        assert_eq!(v0.to_string(), "test[3c1c0]::main");
        assert_eq!(
            v0.coff_decoration().unwrap().symbol(),
            b"_RNvCs1234_4test4main"
        );

        let legacy = demangler
            .demangle("__imp__ZN4test4main17h1234567890abcdefE")
            .unwrap();
        assert_eq!(legacy.scheme(), Scheme::Rust);
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
        assert_eq!(rust.scheme(), Scheme::Rust);
        assert_eq!(rust.to_string(), "test::main::h1234567890abcdef");
    }

    #[cfg(feature = "cpp-itanium")]
    {
        for symbol in ["__Z3foov.llvm.9D1C9369@@16", "_Z3foov@@GLIBCXX_3.4"] {
            let demangled = demangler
                .demangle(symbol)
                .unwrap_or_else(|error| panic!("{symbol}: {error}"));
            assert_eq!(demangled.scheme(), Scheme::Cpp, "{symbol}");
            assert_eq!(demangled.to_string(), "foo()", "{symbol}");
        }
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
            assert_eq!(demangled.scheme(), row.scheme, "{}", row.symbol);
            assert_eq!(demangled.to_string(), row.display, "{}", row.symbol);
        } else {
            // Without the language backend the undecorated name is a plain
            // pass-through, but the decoration is still reported.
            let expected = row.inner.as_deref().unwrap_or(&row.symbol);
            assert_eq!(demangled.to_string(), expected, "{}", row.symbol);
            assert_ne!(demangled.scheme(), row.scheme, "{}", row.symbol);
        }
    }
}

#[test]
fn corpus_covers_every_external_object_symbol() {
    let Some(rows) = read_corpus() else {
        return;
    };
    let recorded: BTreeSet<&str> = rows.iter().map(|row| row.symbol.as_str()).collect();
    let directory = corpus_dir();
    let Ok(entries) = fs::read_dir(&directory) else {
        return;
    };
    let mut found = Vec::new();
    for entry in entries {
        let path = entry.unwrap().path();
        let is_objdump = path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.ends_with(".objdump.txt"));
        if !is_objdump {
            continue;
        }
        let text = fs::read_to_string(&path).unwrap();
        for line in text.lines() {
            // External symbols use storage class 2; sections, files and the
            // `@feat.00` marker do not.
            if !line.contains("(scl   2)") {
                continue;
            }
            let Some(symbol) = line.split_whitespace().last() else {
                continue;
            };
            if symbol.starts_with('.') || symbol == "@feat.00" {
                continue;
            }
            found.push(String::from(symbol));
        }
    }
    assert!(!found.is_empty(), "no object symbols found");
    for symbol in found {
        assert!(recorded.contains(symbol.as_str()), "missing {symbol}");
    }
}
