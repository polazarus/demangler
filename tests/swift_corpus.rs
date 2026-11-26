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

/// Corpus symbols where the project intentionally follows the Swift mangling
/// grammar instead of the reference demangler's implementation limits.
///
/// Each entry is the mangled name, the project's documented result (`None`
/// for a rejection) and why it differs. An entry counts toward the floor only
/// when the demangler produces exactly the documented result, so any other
/// change to these symbols still fails the gate.
const DIVERGENCES: &[(&str, Option<&str>, &str)] = &[
    (
        "_$S3BBBBf0602365061_",
        Some("BBBBuiltin.FPIEEE602365061"),
        "the grammar puts no bound on a builtin width; Apple's 4096 \
         `maxTypeSize` is an implementation limit",
    ),
    (
        "_$S3BBBBi0602365061_",
        Some("BBBBuiltin.Int602365061"),
        "the grammar puts no bound on a builtin width; Apple's 4096 \
         `maxTypeSize` is an implementation limit",
    ),
    (
        "_Ttu4222222222222222222222222_rW_2T_2TJ_",
        None,
        "every counted generic parameter is printed and charged to the \
         expansion budget; Apple elides names past 128 instead",
    ),
];

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
    /// Symbols matching their documented [`DIVERGENCES`] entry.
    divergent: usize,
    /// Symbols that demangled to text that differs from the expectation.
    wrong: Vec<(String, String, String)>,
    /// Symbols the demangler refused; correct only when the corpus records the
    /// symbol as known-unsupported (an identity pair).
    errors: Vec<(String, String)>,
}

impl Tally {
    fn record(&mut self, demangler: &demangler::Demangler, pair: &Pair) {
        self.total += 1;
        if let Some((_, documented, why)) = DIVERGENCES
            .iter()
            .find(|(mangled, ..)| *mangled == pair.mangled)
        {
            let got = demangler
                .demangle(&pair.mangled)
                .ok()
                .map(|demangled| demangled.to_string());
            assert_eq!(
                got.as_deref(),
                *documented,
                "{}: documented divergence changed ({why})",
                pair.mangled
            );
            self.divergent += 1;
            return;
        }
        let mut rendered = String::new();
        match demangler.demangle(&pair.mangled) {
            Ok(demangled_symbol) => {
                let _ = write!(rendered, "{demangled_symbol}");
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
            "{label}: {}/{} exact, {} documented divergences, {} wrong, {} errors",
            self.exact,
            self.total,
            self.divergent,
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
            self.exact + self.divergent >= floor,
            "{label} regressed: {} exact + {} documented divergences of {} (floor {floor})",
            self.exact,
            self.divergent,
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
    let demangler = demangler::Demangler::new();
    let mut tally = Tally::default();
    for pair in &pairs {
        if !demangler::swift::is_swift(&pair.mangled) {
            continue;
        }
        tally.record(&demangler, pair);
    }
    // Every symbol in the corpus matches exactly or its documented divergence,
    // including the raw identifier (`00...`) Punycode spellings.
    tally.finish("swift corpus", 524);
}

#[test]
fn swift_legacy_corpus() {
    let Some(pairs) = corpus() else {
        eprintln!("swift corpus not present (published crate); skipping");
        return;
    };
    let demangler = demangler::Demangler::new();
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

/// The runtime budget must not change corpus output.
///
/// Every symbol the public `swift::parse` accepts must demangle through the
/// [`demangler::Demangler`] path to exactly the same text, both with the
/// implicit default budget and with an explicit
/// [`demangler::Budget::default`].
#[test]
fn swift_corpus_output_is_unchanged_under_the_default_budget() {
    let Some(pairs) = corpus() else {
        eprintln!("swift corpus not present (published crate); skipping");
        return;
    };
    let default = demangler::Demangler::new();
    let explicit = demangler::Demangler::with_budget(demangler::Budget::default());
    let mut compared = 0_usize;
    let mut symbols = 0_usize;
    for pair in &pairs {
        let Ok(ast) = demangler::swift::parse(&pair.mangled) else {
            continue;
        };
        let scheme = match ast.encoding() {
            demangler::swift::Encoding::Stable => demangler::Scheme::SwiftStable,
            demangler::swift::Encoding::Legacy => demangler::Scheme::SwiftLegacy,
        };
        let mut expected = String::new();
        demangler::swift::print(&pair.mangled, &mut expected).expect("the parsed symbol prints");
        symbols += 1;
        for demangler in [&default, &explicit] {
            let Ok(demangled) = demangler.demangle(&pair.mangled) else {
                // Detection is a prefilter and may claim another scheme.
                continue;
            };
            if demangled.scheme() != scheme {
                continue;
            }
            assert_eq!(demangled.to_string(), expected, "{}", pair.mangled);
            compared += 1;
        }
    }
    eprintln!("swift budget parity: {compared} comparisons over {symbols} symbols");
    assert!(compared > 0, "the default budget compared no symbols");
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
