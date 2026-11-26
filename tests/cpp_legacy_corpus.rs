//! Legacy C++ corpus harness.
//!
//! Reads the corpora under `refs/cpp/legacy/` and checks every pair: names that
//! are legacy C++ must be detected as such, parse into the shared AST with the
//! dialect the corpus names and print the pinned text; names that only look
//! legacy must fall through to plain text.
//!
//! The corpora are reference material: when `refs/` is absent the tests skip
//! instead of failing. They record the measured floor — raise it when coverage
//! improves, never lower it.
#![cfg(feature = "cpp-gnu-v2")]

use std::string::{String, ToString};
use std::vec::Vec;

use demangler::cpp::{self, Dialect};
use demangler::{Demangler, Language, Scheme};

const GNU_V2_CORPUS: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/refs/cpp/legacy/gnu-v2.txt");
const GNU_V2_LARGE_CORPUS: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/refs/cpp/legacy/gnu-v2-large.txt"
);
const ARM_CORPUS: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/refs/cpp/legacy/arm.txt");
const EDG_CORPUS: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/refs/cpp/legacy/edg.txt");
const HP_CORPUS: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/refs/cpp/legacy/hp.txt");

/// Measured number of pairs in each vendored corpus.
const GNU_V2_MIN_PAIRS: usize = 80;
const GNU_V2_LARGE_MIN_PAIRS: usize = 1_500;
const ARM_MIN_PAIRS: usize = 40;
const EDG_MIN_PAIRS: usize = 12;
const HP_MIN_PAIRS: usize = 99;

#[test]
fn gnu_v2_corpus() {
    let _ = check_corpus(GNU_V2_CORPUS, Dialect::GnuV2, GNU_V2_MIN_PAIRS);
}

#[test]
fn gnu_v2_large_corpus() {
    let _ = check_corpus(GNU_V2_LARGE_CORPUS, Dialect::GnuV2, GNU_V2_LARGE_MIN_PAIRS);
}

#[test]
fn arm_corpus() {
    let _ = check_corpus(ARM_CORPUS, Dialect::Arm, ARM_MIN_PAIRS);
}

#[test]
fn edg_corpus() {
    let _ = check_corpus(EDG_CORPUS, Dialect::Edg, EDG_MIN_PAIRS);
}

#[test]
fn hp_corpus() {
    if let Some((accepted, rejected)) = check_corpus(HP_CORPUS, Dialect::Hp, HP_MIN_PAIRS) {
        assert_eq!((accepted, rejected), (82, 17), "HP corpus split changed");
    }
}

/// The precise scheme of one legacy dialect.
const fn dialect_scheme(dialect: Dialect) -> Scheme {
    match dialect {
        Dialect::Itanium => Scheme::CppItanium,
        Dialect::Msvc => Scheme::CppMsvc,
        Dialect::GnuV2 => Scheme::CppGnuV2,
        Dialect::Arm => Scheme::CppArm,
        Dialect::Edg => Scheme::CppEdg,
        Dialect::Hp => Scheme::CppHp,
    }
}

/// Check every `<mangled><TAB><expected>` pair of one corpus.
///
/// A pair whose fields are equal is a name that must not be claimed as C++.
fn check_corpus(path: &str, dialect: Dialect, min_pairs: usize) -> Option<(usize, usize)> {
    let scheme = dialect_scheme(dialect);
    let Ok(source) = std::fs::read_to_string(path) else {
        eprintln!("skipping legacy corpus: {path} not found");
        return None;
    };
    let pairs = parse_corpus(&source);
    assert!(
        pairs.len() >= min_pairs,
        "{path} shrank: {} pairs, expected at least {min_pairs}",
        pairs.len()
    );

    let demangler = Demangler::new();
    let mut demangled = 0_usize;
    let mut pass_through = 0_usize;
    let mut failures: Vec<String> = Vec::new();
    for (mangled, expected) in &pairs {
        if mangled == expected {
            // A name that only looks legacy: no C++ dialect may claim it.
            assert_ne!(
                Scheme::detect(mangled).language(),
                Language::Cpp,
                "{mangled} must not be detected as C++"
            );
            let text = demangler.demangle(mangled).map_or_else(
                |_| String::from(mangled.as_str()),
                |parsed| parsed.to_string(),
            );
            if text == *expected {
                pass_through += 1;
            } else {
                failures.push(format!("{mangled}\n  want: {expected}\n  got:  {text}"));
            }
            continue;
        }

        let ast = match cpp::parse(mangled) {
            Ok(ast) => ast,
            Err(error) => {
                failures.push(format!("{mangled}\n  parse failed: {error}"));
                continue;
            }
        };
        assert_eq!(
            ast.dialect(),
            dialect,
            "{mangled} should parse as {dialect:?}"
        );
        assert_eq!(
            Scheme::detect(mangled),
            scheme,
            "{mangled} should be detected as {scheme:?}"
        );
        let text = match demangler.demangle(mangled) {
            Ok(parsed) => {
                assert_eq!(parsed.scheme(), scheme, "{mangled}");
                assert_eq!(parsed.language(), Language::Cpp, "{mangled}");
                parsed.to_string()
            }
            Err(error) => {
                failures.push(format!("{mangled}\n  demangle failed: {error}"));
                continue;
            }
        };
        if text == *expected {
            demangled += 1;
        } else {
            failures.push(format!("{mangled}\n  want: {expected}\n  got:  {text}"));
        }
    }

    assert!(
        failures.is_empty(),
        "{} corpus mismatches in {path}:\n{}",
        failures.len(),
        failures.join("\n")
    );
    assert_eq!(demangled + pass_through, pairs.len());
    Some((demangled, pass_through))
}

/// Split a corpus into `(mangled, expected)` pairs, dropping comments and
/// blank lines.
fn parse_corpus(source: &str) -> Vec<(String, String)> {
    let mut pairs = Vec::new();
    for line in source.lines() {
        let line = line.trim_end();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((mangled, expected)) = line.split_once('\t') else {
            panic!("corpus line without a tab separator: {line}");
        };
        pairs.push((String::from(mangled), String::from(expected)));
    }
    pairs
}
