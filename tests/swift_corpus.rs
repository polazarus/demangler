//! Swift demangling corpus tests.
//!
//! Reads the vendored official corpus (`refs/swift/manglings.txt`, pinned to an
//! upstream commit). `refs/` is excluded from the published crate, so the tests
//! skip gracefully when the corpus is absent.
//!
//! Two gates share the corpus: one over every recognised prefix, and one over
//! the pre-stable `_T` manglings, which the dedicated legacy parser renders.
#![cfg(feature = "swift")]

use std::fmt::Write as _;

/// One `mangled ---> expected` corpus pair.
struct Pair {
    mangled: String,
    expected: String,
}

/// Accumulates exact matches and failures for one gate.
#[derive(Default)]
struct Tally {
    total: usize,
    exact: usize,
    /// Symbols that demangled to text that differs from the expectation.
    wrong: Vec<(String, String, String)>,
    /// Symbols the demangler refused; correct only when the corpus records the
    /// symbol as known-unsupported (an identity pair).
    errors: Vec<(String, String)>,
}

impl Tally {
    fn record(&mut self, demangler: &demangle::Demangler, pair: &Pair) {
        self.total += 1;
        let mut rendered = String::new();
        match demangler.demangle(&pair.mangled) {
            Ok(demangled) => {
                let _ = write!(rendered, "{demangled}");
                if rendered == pair.expected {
                    self.exact += 1;
                } else {
                    self.wrong
                        .push((pair.mangled.clone(), pair.expected.clone(), rendered));
                }
            }
            Err(_) => {
                if pair.expected == pair.mangled {
                    self.exact += 1;
                } else {
                    self.errors
                        .push((pair.mangled.clone(), pair.expected.clone()));
                }
            }
        }
    }

    /// Report the tally and assert that nothing was printed wrongly.
    fn finish(&self, label: &str, floor: usize) {
        for (mangled, expected, got) in self.wrong.iter().take(25) {
            eprintln!("{mangled}\n  want: {expected}\n  got : {got}");
        }
        for (mangled, expected) in self.errors.iter().take(10) {
            eprintln!("{mangled}\n  want: {expected}\n  got : <error>");
        }
        eprintln!(
            "{label}: {}/{} exact, {} wrong, {} errors",
            self.exact,
            self.total,
            self.wrong.len(),
            self.errors.len()
        );
        assert!(self.total > 0, "{label} contained no symbols");
        assert!(
            self.wrong.is_empty(),
            "silent wrong-output regressions: {} (must be zero)",
            self.wrong.len()
        );
        assert!(
            self.exact >= floor,
            "{label} regressed: {}/{} passed (floor {floor})",
            self.exact,
            self.total
        );
    }
}

/// Load the corpus, or `None` when it is not present.
fn corpus() -> Option<Vec<Pair>> {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/refs/swift/manglings.txt");
    let data = std::fs::read_to_string(path).ok()?;
    let mut pairs = Vec::new();
    for line in data.lines() {
        let Some((mangled, expected)) = line.split_once(" ---> ") else {
            continue;
        };
        pairs.push(Pair {
            mangled: mangled.trim().to_owned(),
            // Upstream expectations come from `swift-demangle -classify`, so
            // they may start with a `{T:<target>}`, `{C}` or `{N}`
            // classification prefix. Classification is not demangling: strip
            // it before comparing.
            expected: strip_classification(expected.trim()).to_owned(),
        });
    }
    Some(pairs)
}

#[test]
fn swift_corpus() {
    let Some(pairs) = corpus() else {
        eprintln!("swift corpus not present (published crate); skipping");
        return;
    };
    let demangler = demangle::Demangler::new();
    let mut tally = Tally::default();
    for pair in &pairs {
        if !demangle::swift::is_swift(&pair.mangled) {
            continue;
        }
        tally.record(&demangler, pair);
    }
    // Two raw-identifier (`_$s...00...`) symbols are not implemented yet; the
    // rest of the corpus matches exactly.
    tally.finish("swift corpus", 522);
}

#[test]
fn swift_legacy_corpus() {
    let Some(pairs) = corpus() else {
        eprintln!("swift corpus not present (published crate); skipping");
        return;
    };
    let demangler = demangle::Demangler::new();
    let mut tally = Tally::default();
    for pair in &pairs {
        // `_T0` is the Swift 4.0 postfix grammar; every other `_T` name is a
        // pre-stable mangling handled by the legacy parser.
        let mangled = &pair.mangled;
        if !mangled.starts_with("_T") || mangled.starts_with("_T0") {
            continue;
        }
        tally.record(&demangler, pair);
    }
    tally.finish("swift legacy corpus", 248);
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
