//! GNU v2 legacy C++ corpus harness.
//!
//! Reads `refs/cpp/legacy/gnu-v2.txt` and checks every pair: names that are
//! legacy C++ must be detected as such, parse into the shared AST with the
//! [`Dialect::GnuV2`] dialect and print the pinned text; names that only look
//! legacy must fall through to plain text.
//!
//! The corpus is reference material: when `refs/` is absent the test skips
//! instead of failing. It records the measured floor — raise it when coverage
//! improves, never lower it.

use std::string::{String, ToString};
use std::vec::Vec;

use demangle::cpp::{self, Dialect};
use demangle::{Demangler, Scheme};

const CORPUS: &str = "refs/cpp/legacy/gnu-v2.txt";

/// Measured number of demangled pairs in the vendored corpus.
const MIN_PAIRS: usize = 80;

#[test]
fn gnu_v2_corpus() {
    let Ok(source) = std::fs::read_to_string(CORPUS) else {
        eprintln!("skipping GNU v2 corpus: {CORPUS} not found");
        return;
    };
    let pairs = parse_corpus(&source);
    assert!(
        pairs.len() >= MIN_PAIRS,
        "corpus shrank: {} pairs, expected at least {MIN_PAIRS}",
        pairs.len()
    );

    let demangler = Demangler::new();
    let mut demangled = 0_usize;
    let mut pass_through = 0_usize;
    let mut failures: Vec<String> = Vec::new();
    for (mangled, expected) in &pairs {
        if mangled == expected {
            // A name that only looks legacy: detection must not claim it.
            assert_ne!(
                Scheme::detect(mangled),
                Scheme::Cpp,
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
            Dialect::GnuV2,
            "{mangled} should parse as GNU v2"
        );
        assert_eq!(
            Scheme::detect(mangled),
            Scheme::Cpp,
            "{mangled} should be detected as C++"
        );
        let text = match demangler.demangle(mangled) {
            Ok(parsed) => {
                assert_eq!(parsed.scheme(), Scheme::Cpp, "{mangled}");
                assert_eq!(parsed.language(), demangle::Language::Cxx, "{mangled}");
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
        "{} corpus mismatches:\n{}",
        failures.len(),
        failures.join("\n")
    );
    assert_eq!(demangled + pass_through, pairs.len());
}

/// Split the corpus into `(mangled, expected)` pairs, dropping comments and
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
