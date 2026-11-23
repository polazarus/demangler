//! Ada/GNAT corpus gate.
//!
//! Reads `refs/ada/corpus/gnat-14.2-linux-x86_64.tsv`, generated from the
//! fixture under `refs/ada/fixture/` with GNAT 14.2.0 and classified against
//! the GNU Binutils 2.44 `c++filt -s gnat` oracle. `refs/` is excluded from the
//! published crate, so the test skips gracefully when the corpus is absent.
//!
//! The gate checks:
//!
//! - every `exact` and `oracle-gap` name parses and prints the expected text;
//! - every `unsupported` name is rejected rather than rendered;
//! - high-confidence names are detected as [`Scheme::Ada`] and demangled by
//!   [`Demangler`], while general `pkg__subp` names are not auto-claimed;
//! - `exact` expectations agree with the vendored `c++filt` output, and
//!   `oracle-gap` expectations are exactly the cases where `c++filt` echoes
//!   the input or prints a `<...>` placeholder.
//!
//! `c++filt` is never run by the test; only its recorded output is read.

#![cfg(feature = "ada")]

use std::fs;

use demangle::ada;
use demangle::{Demangler, Language, Scheme};

/// Curated corpus: symbol, source, class, expected, oracle.
const CORPUS: &str = "refs/ada/corpus/gnat-14.2-linux-x86_64.tsv";

/// Exact measured counts. Raise them when coverage improves; never lower them.
const COUNTS: Counts = Counts {
    generated_exact: 20,
    generated_unsupported: 22,
    generated_oracle_gap: 0,
    hand_exact: 11,
    hand_unsupported: 25,
    hand_oracle_gap: 2,
};

#[test]
fn ada_corpus() {
    let path = format!("{}/{CORPUS}", env!("CARGO_MANIFEST_DIR"));
    let Ok(data) = fs::read_to_string(&path) else {
        eprintln!("ada corpus {path} not present (published crate); skipping");
        return;
    };

    let mut counts = Counts::default();
    let mut failures: Vec<String> = Vec::new();

    for (index, line) in data.lines().enumerate() {
        let line_number = index + 1;
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let fields: Vec<&str> = line.split('\t').collect();
        if fields.len() != 5 {
            failures.push(format!(
                "line {line_number}: expected 5 fields, got {}",
                fields.len()
            ));
            continue;
        }
        let symbol = fields[0];
        let source = fields[1];
        let class = fields[2];
        let expected = fields[3];
        let oracle = fields[4];
        let Some(source) = Source::parse(source) else {
            failures.push(format!("line {line_number}: unknown source {source:?}"));
            continue;
        };
        let Some(class) = Class::parse(class) else {
            failures.push(format!("line {line_number}: unknown class {class:?}"));
            continue;
        };
        counts.observe(source, class);

        check_entry(symbol, class, expected, oracle, &mut failures);
    }

    eprintln!("ada corpus: {counts:?}, {} failures", failures.len());
    for failure in failures.iter().take(30) {
        eprintln!("{failure}");
    }
    assert!(
        failures.is_empty(),
        "{} ada corpus failures",
        failures.len()
    );
    assert_eq!(counts, COUNTS, "ada corpus counts changed");
}

/// Check one corpus row against the backend and the recorded oracle.
fn check_entry(
    symbol: &str,
    class: Class,
    expected: &str,
    oracle: &str,
    failures: &mut Vec<String>,
) {
    let parsed = ada::parse(symbol);
    match class {
        Class::Exact | Class::OracleGap => {
            if expected.is_empty() {
                failures.push(format!("{symbol}: missing expected text"));
                return;
            }
            match parsed {
                Ok(ast) => {
                    let rendered = ast.to_string();
                    if rendered != expected {
                        failures.push(format!("{symbol}: expected {expected:?}, got {rendered:?}"));
                    }
                    match ada::parse(symbol) {
                        Ok(again) if again.to_string() == rendered => {}
                        Ok(again) => failures.push(format!(
                            "{symbol}: unstable render {rendered:?} -> {:?}",
                            again.to_string()
                        )),
                        Err(_) => failures.push(format!("{symbol}: second parse failed")),
                    }
                }
                Err(_) => failures.push(format!("{symbol}: parse failed, expected {expected:?}")),
            }
            check_detection(symbol, expected, failures);
        }
        Class::Unsupported => {
            if parsed.is_ok() {
                failures.push(format!("{symbol}: unsupported name was accepted"));
            }
            if !expected.is_empty() {
                failures.push(format!("{symbol}: unsupported row has expected text"));
            }
            if has_detection_prefix(symbol) {
                if Scheme::detect(symbol) != Scheme::Ada {
                    failures.push(format!("{symbol}: high-confidence prefix not detected"));
                }
                if Demangler::new().demangle(symbol).is_ok() {
                    failures.push(format!("{symbol}: unsupported name was demangled"));
                }
            } else if Scheme::detect(symbol) == Scheme::Ada {
                failures.push(format!(
                    "{symbol}: general unsupported name was auto-detected"
                ));
            }
        }
    }

    match class {
        Class::Exact => {
            if oracle != expected {
                failures.push(format!(
                    "{symbol}: c++filt disagrees: {oracle:?} != {expected:?}"
                ));
            }
        }
        Class::OracleGap => {
            if oracle != symbol && !oracle.starts_with('<') {
                failures.push(format!("{symbol}: c++filt output {oracle:?} is not a gap"));
            }
        }
        Class::Unsupported => {}
    }
}

/// Check scheme detection for a supported name.
fn check_detection(symbol: &str, expected: &str, failures: &mut Vec<String>) {
    if has_detection_prefix(symbol) {
        if Scheme::detect(symbol) != Scheme::Ada {
            failures.push(format!("{symbol}: not detected as Ada"));
            return;
        }
        match Demangler::new().demangle(symbol) {
            Ok(demangled) => {
                if demangled.scheme() != Scheme::Ada || demangled.language() != Language::Ada {
                    failures.push(format!("{symbol}: wrong scheme or language"));
                } else if demangled.to_string() != expected {
                    failures.push(format!(
                        "{symbol}: demangler printed {:?}, expected {expected:?}",
                        demangled.to_string()
                    ));
                }
            }
            Err(_) => failures.push(format!("{symbol}: demangler failed")),
        }
    } else if Scheme::detect(symbol) == Scheme::Ada {
        failures.push(format!("{symbol}: general GNAT name was auto-detected"));
    }
}

/// True when a name carries one of the precision-only detection prefixes.
fn has_detection_prefix(symbol: &str) -> bool {
    symbol.starts_with("ada__")
        || symbol.starts_with("_ada__")
        || symbol.starts_with("__ada__")
        || symbol.starts_with("system__")
        || symbol.starts_with("gnat__")
}

#[test]
fn ada_detection_is_precision_only() {
    let demangler = Demangler::new();

    // Ordinary C names with `__` stay C and pass through unchanged.
    for symbol in ["foo__bar", "foo_bar", "foo", "main"] {
        assert_eq!(Scheme::detect(symbol), Scheme::C, "{symbol}");
        assert_eq!(demangler.demangle(symbol).unwrap().to_string(), symbol);
    }

    // Legacy GNU v2 C++ also uses `__`, but only when its parser consumes the
    // whole symbol. Ada must not take it over.
    assert_ne!(Scheme::detect("a__b__c"), Scheme::Ada);
    #[cfg(feature = "cpp-gnu-v2")]
    assert_eq!(Scheme::detect("a__b__c"), Scheme::Cpp);

    // High-confidence GNAT prefixes are claimed.
    for symbol in [
        "ada__foo",
        "ada__exceptions__triggered_by_abort",
        "system__foo",
        "gnat__foo",
        "_ada__foo",
        "__ada__foo",
    ] {
        assert_eq!(Scheme::detect(symbol), Scheme::Ada, "{symbol}");
    }

    // A high-confidence prefix is claimed even when the body is an unsupported
    // task/runtime suffix; the backend then rejects it instead of guessing.
    for symbol in [
        "ada__",
        "ada__workerTB",
        "ada__foo__2",
        "system__workerTB",
        "gnat__workerTB",
    ] {
        assert_eq!(Scheme::detect(symbol), Scheme::Ada, "{symbol}");
        assert!(demangler.demangle(symbol).is_err(), "{symbol}");
    }
}

/// Corpus provenance.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Source {
    /// Generated from `refs/ada/fixture/`.
    Generated,
    /// Hand-authored detection and suffix cases.
    Hand,
}

impl Source {
    fn parse(text: &str) -> Option<Self> {
        match text {
            "generated" => Some(Self::Generated),
            "hand" => Some(Self::Hand),
            _ => None,
        }
    }
}

/// Corpus classification.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Class {
    /// `ada::parse` prints the expected text and `c++filt` agrees.
    Exact,
    /// The backend must reject the name.
    Unsupported,
    /// The backend prints the expected text but `c++filt` echoes/placeholders.
    OracleGap,
}

impl Class {
    fn parse(text: &str) -> Option<Self> {
        match text {
            "exact" => Some(Self::Exact),
            "unsupported" => Some(Self::Unsupported),
            "oracle-gap" => Some(Self::OracleGap),
            _ => None,
        }
    }
}

/// Measured row counts, by source and classification.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Counts {
    generated_exact: usize,
    generated_unsupported: usize,
    generated_oracle_gap: usize,
    hand_exact: usize,
    hand_unsupported: usize,
    hand_oracle_gap: usize,
}

impl Counts {
    fn observe(&mut self, source: Source, class: Class) {
        let slot = match (source, class) {
            (Source::Generated, Class::Exact) => &mut self.generated_exact,
            (Source::Generated, Class::Unsupported) => &mut self.generated_unsupported,
            (Source::Generated, Class::OracleGap) => &mut self.generated_oracle_gap,
            (Source::Hand, Class::Exact) => &mut self.hand_exact,
            (Source::Hand, Class::Unsupported) => &mut self.hand_unsupported,
            (Source::Hand, Class::OracleGap) => &mut self.hand_oracle_gap,
        };
        *slot += 1;
    }
}
