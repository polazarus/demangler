//! Swift demangling corpus test.
//!
//! Reads the vendored official corpus (`refs/swift/manglings.txt`, pinned to an
//! upstream commit). `refs/` is excluded from the published crate, so the test
//! skips gracefully when the corpus is absent.

use std::fmt::Write as _;

#[test]
fn swift_corpus() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/refs/swift/manglings.txt");
    let Ok(data) = std::fs::read_to_string(path) else {
        eprintln!("swift corpus not present (published crate); skipping");
        return;
    };

    let demangler = demangle::Demangler::new();
    let mut total = 0_usize;
    let mut pass = 0_usize;
    let mut failures: Vec<(String, String, String)> = Vec::new();

    for line in data.lines() {
        let Some((mangled, expected)) = line.split_once(" ---> ") else {
            continue;
        };
        let mangled = mangled.trim();
        // Upstream expectations come from `swift-demangle -classify`, so they
        // may start with a `{T:<target>}`, `{C}` or `{N}` classification
        // prefix. Classification is not demangling: strip it before comparing.
        let expected = strip_classification(expected.trim());
        if !demangle::swift::is_swift(mangled) {
            continue;
        }
        total += 1;
        let mut rendered = String::new();
        match demangler.demangle(mangled) {
            Ok(demangled) => {
                let _ = write!(rendered, "{demangled}");
                if rendered == expected {
                    pass += 1;
                } else {
                    failures.push((mangled.to_owned(), expected.to_owned(), rendered));
                }
            }
            Err(error) => {
                // The corpus records known-unsupported symbols as identity
                // pairs; refusing them is the correct outcome.
                if expected == mangled {
                    pass += 1;
                } else {
                    failures.push((
                        mangled.to_owned(),
                        expected.to_owned(),
                        format!("<{error}>"),
                    ));
                }
            }
        }
    }

    let mut pass_mismatch: Vec<(String, String, String)> = Vec::new();
    for (mangled, expected, got) in &failures {
        if !got.starts_with('<') {
            pass_mismatch.push((mangled.clone(), expected.clone(), got.clone()));
        }
    }
    eprintln!(
        "near-misses (parsed but different): {}",
        pass_mismatch.len()
    );
    for (mangled, expected, got) in pass_mismatch.iter().take(25) {
        eprintln!("{mangled}\n  want: {expected}\n  got : {got}");
    }
    for (mangled, expected, got) in failures
        .iter()
        .filter(|(_, _, g)| g.starts_with('<'))
        .take(5)
    {
        eprintln!("{mangled}\n  want: {expected}\n  got : {got}");
    }
    eprintln!("swift corpus: {pass}/{total} ({} failures)", total - pass);
    assert!(total > 0, "corpus contained no supported symbols");
    assert!(
        pass >= 194,
        "swift corpus regressed: {pass}/{total} passed (floor 194)"
    );
}

/// Strip a leading `{...}` classification prefix produced by
/// `swift-demangle -classify`.
fn strip_classification(expected: &str) -> &str {
    let Some(rest) = expected.strip_prefix('{') else {
        return expected;
    };
    let Some(end) = rest.find('}') else {
        return expected;
    };
    let classification = &rest[..end];
    let is_classification = classification
        .split(',')
        .all(|part| part == "T" || part == "C" || part == "N" || part.starts_with("T:"));
    if is_classification {
        rest.get(end + 1..)
            .map_or(expected, |tail| tail.trim_start())
    } else {
        expected
    }
}
