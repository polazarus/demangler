//! Delphi/Object Pascal corpus gate.
//!
//! Reads `refs/delphi/corpus/fpc-3.2.2-linux-x86_64.tsv`, generated from the
//! fixture under `refs/delphi/fixture/` with Free Pascal 3.2.2. `refs/` is
//! excluded from the published crate, so the test skips gracefully when the
//! corpus is absent.
//!
//! Neither Free Pascal nor Delphi ships a demangler, so the `exact` rows are
//! the project display contract from `refs/delphi.md`, not an oracle's output.
//! The gate checks:
//!
//! - every `exact` name parses, prints the expected text and re-parses stably;
//! - every `unsupported` name is rejected rather than rendered;
//! - `exact` names are detected as [`Scheme::Delphi`] and demangled by
//!   [`Demangler`] with the same text, while `unsupported` names are never
//!   claimed.

#![cfg(feature = "delphi")]

use std::fs;

use demangle::delphi;
use demangle::{Demangler, Language, Scheme};

/// Curated corpus: symbol, source, class, expected, note.
const CORPUS: &str = "refs/delphi/corpus/fpc-3.2.2-linux-x86_64.tsv";

/// Exact measured counts. Raise them when coverage improves; never lower them.
const COUNTS: Counts = Counts {
    generated_exact: 33,
    generated_unsupported: 26,
    hand_exact: 0,
    hand_unsupported: 15,
};

#[test]
fn delphi_corpus() {
    let path = format!("{}/{CORPUS}", env!("CARGO_MANIFEST_DIR"));
    let Ok(data) = fs::read_to_string(&path) else {
        eprintln!("delphi corpus {path} not present (published crate); skipping");
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
        let note = fields[4];
        let Some(source) = Source::parse(source) else {
            failures.push(format!("line {line_number}: unknown source {source:?}"));
            continue;
        };
        let Some(class) = Class::parse(class) else {
            failures.push(format!("line {line_number}: unknown class {class:?}"));
            continue;
        };
        if note.is_empty() {
            failures.push(format!("line {line_number}: missing note"));
        }
        counts.observe(source, class);
        check_entry(symbol, class, expected, &mut failures);
    }

    eprintln!("delphi corpus: {counts:?}, {} failures", failures.len());
    for failure in failures.iter().take(30) {
        eprintln!("{failure}");
    }
    assert!(
        failures.is_empty(),
        "{} delphi corpus failures",
        failures.len()
    );
    assert_eq!(counts, COUNTS, "delphi corpus counts changed");
}

/// Check one corpus row against the backend.
fn check_entry(symbol: &str, class: Class, expected: &str, failures: &mut Vec<String>) {
    let parsed = delphi::parse(symbol);
    match class {
        Class::Exact => {
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
                    match delphi::parse(symbol) {
                        Ok(again) if again.to_string() == rendered => {}
                        Ok(again) => failures.push(format!(
                            "{symbol}: unstable render {rendered:?} -> {:?}",
                            again.to_string()
                        )),
                        Err(_) => failures.push(format!("{symbol}: second parse failed")),
                    }
                }
                Err(error) => failures.push(format!(
                    "{symbol}: parse failed ({error:?}), expected {expected:?}"
                )),
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
            if delphi::is_delphi(symbol.as_bytes()) {
                failures.push(format!(
                    "{symbol}: unsupported name is claimed by is_delphi"
                ));
            }
            if Scheme::detect(symbol) == Scheme::Delphi {
                failures.push(format!("{symbol}: unsupported name was detected as Delphi"));
            }
            if Demangler::new().demangle(symbol).is_ok_and(|demangled| {
                demangled.scheme() == Scheme::Delphi || demangled.language() == Language::Delphi
            }) {
                failures.push(format!(
                    "{symbol}: unsupported name was demangled as Delphi"
                ));
            }
        }
    }
}

/// Check scheme detection and end-to-end demangling for a supported name.
fn check_detection(symbol: &str, expected: &str, failures: &mut Vec<String>) {
    if !delphi::is_delphi(symbol.as_bytes()) {
        failures.push(format!("{symbol}: not claimed by is_delphi"));
        return;
    }
    if let Some(other) = dotted_unit_claim(symbol) {
        // A namespaced unit (`DELPHI_PROBE.NS_$$_…`) is dotted, so the Go
        // heuristic in `Scheme::detect` claims it before this backend is asked.
        // Go then rejects the name, so the symbol is left undemangled rather
        // than mis-rendered; `refs/delphi.md` records the required ordering
        // change. The print contract above is still checked.
        if other != Scheme::Go {
            failures.push(format!("{symbol}: detected as {other:?}, not Delphi"));
        }
        return;
    }
    if Scheme::detect(symbol) != Scheme::Delphi {
        failures.push(format!("{symbol}: not detected as Delphi"));
        return;
    }
    if Scheme::detect_bytes(symbol.as_bytes()) != Scheme::Delphi {
        failures.push(format!("{symbol}: not detected as Delphi from bytes"));
    }
    match Demangler::new().demangle(symbol) {
        Ok(demangled) => {
            if demangled.scheme() != Scheme::Delphi || demangled.language() != Language::Delphi {
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
    match Demangler::new().demangle_bytes(symbol.as_bytes()) {
        Ok(demangled) => {
            if demangled.to_string() != expected {
                failures.push(format!("{symbol}: byte demangler printed the wrong text"));
            }
        }
        Err(_) => failures.push(format!("{symbol}: byte demangler failed")),
    }
}

#[test]
fn delphi_detection_is_precision_only() {
    let demangler = Demangler::new();

    // Ordinary C names are never claimed: Free Pascal names always contain a
    // `$` separator, and a C identifier cannot.
    for symbol in ["main", "foo_bar", "foo__bar", "a__b__c", "_start"] {
        assert!(!delphi::is_delphi(symbol.as_bytes()), "{symbol}");
        assert_ne!(Scheme::detect(symbol), Scheme::Delphi, "{symbol}");
    }

    // Other schemes keep their names.
    assert!(!delphi::is_delphi(b"_Z3foov"));
    assert!(!delphi::is_delphi(b"_RNvNtCs1234_7mycrate3foo"));
    assert!(!delphi::is_delphi(b"kfun:com.example.Foo#bar"));
    assert!(!delphi::is_delphi(b"Java_com_example_Foo_bar"));

    // Free Pascal names are claimed and demangled end to end.
    for symbol in [
        "DELPHI_PROBE_$$_UNITPROC$LONGINT",
        "DELPHI_PROBE$_$TPROBECLASS_$__$$_ADD$LONGINT$LONGINT$$LONGINT",
        "P$DELPHI_PROBE_PROG_$$_PROGPROC$LONGINT",
    ] {
        assert!(delphi::is_delphi(symbol.as_bytes()), "{symbol}");
        assert_eq!(Scheme::detect(symbol), Scheme::Delphi, "{symbol}");
        assert!(demangler.demangle(symbol).is_ok(), "{symbol}");
    }

    // A malformed Free Pascal name is not claimed at all: this backend has no
    // prefix that could identify a symbol before the grammar is validated.
    for symbol in [
        "DELPHI_PROBE_$$_",
        "DELPHI_PROBE_$$_NOARGS$",
        "DELPHI_PROBE$_$TPROBECLASS_$$_PLAIN",
        "DELPHI_PROBE_$$_USESGENERIC$TPROBEBOX$1$CRC9F312717",
        "@System@@LStrClr$QQRPV",
    ] {
        assert!(!delphi::is_delphi(symbol.as_bytes()), "{symbol}");
        assert_ne!(Scheme::detect(symbol), Scheme::Delphi, "{symbol}");
    }
}

/// Report the scheme a dotted Free Pascal name is claimed by, if any.
///
/// Only a namespaced unit (`ns.esc`) can put a `.` in a Free Pascal linker
/// name. That dot makes the Go heuristic claim the name before the Delphi
/// check runs.
fn dotted_unit_claim(symbol: &str) -> Option<Scheme> {
    if !symbol.contains('.') {
        return None;
    }
    match Scheme::detect(symbol) {
        Scheme::Delphi => None,
        other => Some(other),
    }
}

/// Corpus provenance.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Source {
    /// Generated from `refs/delphi/fixture/` with Free Pascal 3.2.2.
    Generated,
    /// Hand-authored boundary and detection cases.
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
    /// `delphi::parse` prints the expected contract text.
    Exact,
    /// The backend must reject the name.
    Unsupported,
}

impl Class {
    fn parse(text: &str) -> Option<Self> {
        match text {
            "exact" => Some(Self::Exact),
            "unsupported" => Some(Self::Unsupported),
            _ => None,
        }
    }
}

/// Measured row counts, by source and classification.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Counts {
    generated_exact: usize,
    generated_unsupported: usize,
    hand_exact: usize,
    hand_unsupported: usize,
}

impl Counts {
    fn observe(&mut self, source: Source, class: Class) {
        let slot = match (source, class) {
            (Source::Generated, Class::Exact) => &mut self.generated_exact,
            (Source::Generated, Class::Unsupported) => &mut self.generated_unsupported,
            (Source::Hand, Class::Exact) => &mut self.hand_exact,
            (Source::Hand, Class::Unsupported) => &mut self.hand_unsupported,
        };
        *slot += 1;
    }
}
