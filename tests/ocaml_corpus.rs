//! OCaml corpus gate.
//!
//! Reads the corpora in `refs/ocaml/corpus/`, generated with
//! `refs/ocaml/build-corpus.py` from the fixture under `refs/ocaml/fixture/`
//! and from the OCaml 5.3.0 standard library archive. `refs/` is excluded from
//! the published crate, so the test skips gracefully when a corpus is absent.
//!
//! OCaml ships no demangler, so every expected text is the project display
//! contract from `refs/ocaml.md`. The rows are classified by provenance:
//!
//! - `exact`: the expected text is authored from the fixture sources, so it is
//!   an independent check of the contract;
//! - `contract`: the expected text is the documented transform applied to a
//!   standard library symbol, which guards detection, stamp stripping and
//!   escape decoding over the whole archive;
//! - `identity`: a runtime helper, printed as it stands; the row checks the
//!   classification and is never counted as reconstructed text;
//! - `unsupported`: the backend must reject the symbol instead of rendering it.
//!
//! The gate checks that every accepted row parses, prints the expected text and
//! re-parses stably, that every unsupported row is rejected and never claimed,
//! and that no accepted row is claimed by another scheme with different text.

#![cfg(feature = "ocaml")]

use std::fs;

use demangler::ocaml;
use demangler::{Demangler, Language, Scheme};

/// The corpora, in reading order.
const CORPORA: [&str; 5] = [
    "fixture.tsv",
    "runtime.tsv",
    "link.tsv",
    "fragments.tsv",
    "stdlib.tsv",
];

/// Exact measured counts. Raise them when coverage improves; never lower them.
const COUNTS: Counts = Counts {
    fixture_exact: 94,
    fixture_unsupported: 0,
    runtime_identity: 1045,
    link_exact: 11,
    link_unsupported: 5,
    fragment_exact: 12,
    stdlib_contract: 3619,
    stdlib_unsupported: 0,
};

#[test]
fn ocaml_corpus() {
    let mut counts = Counts::default();
    let mut failures: Vec<String> = Vec::new();
    let mut read = 0_usize;

    for name in CORPORA {
        let path = format!("{}/refs/ocaml/corpus/{name}", env!("CARGO_MANIFEST_DIR"));
        let Ok(data) = fs::read_to_string(&path) else {
            eprintln!("ocaml corpus {path} not present (published crate); skipping");
            continue;
        };
        read += 1;
        check_corpus(name, &data, &mut counts, &mut failures);
    }

    if read == 0 {
        return;
    }
    if read != CORPORA.len() {
        failures.push(format!(
            "only {read} of {} corpora are present",
            CORPORA.len()
        ));
    }

    eprintln!("ocaml corpus: {counts:?}, {} failures", failures.len());
    for failure in failures.iter().take(30) {
        eprintln!("{failure}");
    }
    assert!(
        failures.is_empty(),
        "{} ocaml corpus failures",
        failures.len()
    );
    assert_eq!(counts, COUNTS, "ocaml corpus counts changed");
}

/// Check every row of one corpus file.
fn check_corpus(name: &str, data: &str, counts: &mut Counts, failures: &mut Vec<String>) {
    for (index, line) in data.lines().enumerate() {
        let line_number = index + 1;
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let fields: Vec<&str> = line.split('\t').collect();
        if fields.len() != 5 {
            failures.push(format!(
                "{name}:{line_number}: expected 5 fields, got {}",
                fields.len()
            ));
            continue;
        }
        let symbol = fields[0];
        let note = fields[4];
        let Some(source) = Source::parse(fields[1]) else {
            failures.push(format!(
                "{name}:{line_number}: unknown source {:?}",
                fields[1]
            ));
            continue;
        };
        let Some(class) = Class::parse(fields[2]) else {
            failures.push(format!(
                "{name}:{line_number}: unknown class {:?}",
                fields[2]
            ));
            continue;
        };
        if note.is_empty() {
            failures.push(format!("{name}:{line_number}: missing note"));
        }
        if !source.allows(class) {
            failures.push(format!(
                "{name}:{line_number}: {source:?} rows cannot be {class:?}"
            ));
            continue;
        }
        counts.observe(source, class);
        let context = format!("{name}:{line_number}");
        check_entry(&context, symbol, class, fields[3], failures);
    }
}

/// Check one corpus row against the backend.
fn check_entry(
    context: &str,
    symbol: &str,
    class: Class,
    expected: &str,
    failures: &mut Vec<String>,
) {
    let parsed = ocaml::parse(symbol.as_bytes());
    match class {
        Class::Unsupported => {
            if parsed.is_ok() {
                failures.push(format!(
                    "{context}: {symbol}: unsupported name was accepted"
                ));
            }
            if !expected.is_empty() {
                failures.push(format!(
                    "{context}: {symbol}: unsupported row has expected text"
                ));
            }
            if ocaml::is_ocaml(symbol.as_bytes()) {
                failures.push(format!("{context}: {symbol}: claimed by is_ocaml"));
            }
            if Scheme::detect(symbol) == Scheme::OCaml {
                failures.push(format!("{context}: {symbol}: detected as OCaml"));
            }
            if Demangler::new()
                .demangle(symbol)
                .is_ok_and(|demangled| demangled.language() == Language::OCaml)
            {
                failures.push(format!("{context}: {symbol}: demangled as OCaml"));
            }
        }
        Class::Exact | Class::Contract | Class::Identity => {
            if expected.is_empty() {
                failures.push(format!("{context}: {symbol}: missing expected text"));
                return;
            }
            match parsed {
                Ok(ast) => {
                    let rendered = ast.to_string();
                    if rendered != expected {
                        failures.push(format!(
                            "{context}: {symbol}: expected {expected:?}, got {rendered:?}"
                        ));
                    }
                    match ocaml::parse(symbol.as_bytes()) {
                        Ok(again) if again.to_string() == rendered => {}
                        Ok(again) => failures.push(format!(
                            "{context}: {symbol}: unstable render {rendered:?} -> {:?}",
                            again.to_string()
                        )),
                        Err(_) => {
                            failures.push(format!("{context}: {symbol}: second parse failed"));
                        }
                    }
                }
                Err(error) => failures.push(format!(
                    "{context}: {symbol}: parse failed ({error:?}), expected {expected:?}"
                )),
            }
            if class == Class::Identity && expected != symbol {
                failures.push(format!(
                    "{context}: {symbol}: identity row expects the symbol itself"
                ));
            }
            check_detection(context, symbol, expected, failures);
        }
    }
}

/// Check scheme detection and end-to-end demangling for an accepted name.
fn check_detection(context: &str, symbol: &str, expected: &str, failures: &mut Vec<String>) {
    if !ocaml::is_ocaml(symbol.as_bytes()) {
        failures.push(format!("{context}: {symbol}: not claimed by is_ocaml"));
        return;
    }
    if Scheme::detect(symbol) != Scheme::OCaml {
        failures.push(format!("{context}: {symbol}: not detected as OCaml"));
        return;
    }
    if Scheme::detect_bytes(symbol.as_bytes()) != Scheme::OCaml {
        failures.push(format!(
            "{context}: {symbol}: not detected as OCaml from bytes"
        ));
    }
    match Demangler::new().demangle(symbol) {
        Ok(demangled) => {
            if demangled.scheme() != Scheme::OCaml || demangled.language() != Language::OCaml {
                failures.push(format!("{context}: {symbol}: wrong scheme or language"));
            } else if demangled.to_string() != expected {
                failures.push(format!(
                    "{context}: {symbol}: demangler printed {:?}, expected {expected:?}",
                    demangled.to_string()
                ));
            }
        }
        Err(_) => failures.push(format!("{context}: {symbol}: demangler failed")),
    }
    match Demangler::new().demangle_bytes(symbol.as_bytes()) {
        Ok(demangled) => {
            if demangled.to_string() != expected {
                failures.push(format!(
                    "{context}: {symbol}: byte demangler printed the wrong text"
                ));
            }
        }
        Err(_) => failures.push(format!("{context}: {symbol}: byte demangler failed")),
    }
}

#[test]
fn ocaml_detection_is_precision_only() {
    let demangler = Demangler::new();

    // The `caml` prefix alone is not a name: a lookalike, an empty or doubled
    // segment and a member without a stamp are all rejected.
    for symbol in [
        "main",
        "caml",
        "camel",
        "camel_alloc",
        "camlist",
        "camlist.x",
        "caml_",
        "caml__alloc",
        "caml_alloc_",
        "camlProbe.foo",
        "camlProbe.11x",
        "camlProbe.11.2",
        "caml_ba_compare.cold.1",
        "caml_ba_compare.part.",
        "camlOcaml_probe.f_1.cold",
    ] {
        assert!(!ocaml::is_ocaml(symbol.as_bytes()), "{symbol}");
        assert_ne!(Scheme::detect(symbol), Scheme::OCaml, "{symbol}");
    }

    // Other schemes keep their names.
    assert!(!ocaml::is_ocaml(b"_Z3foov"));
    assert!(!ocaml::is_ocaml(b"_RNvNtCs1234_7mycrate3foo"));
    assert!(!ocaml::is_ocaml(b"kfun:com.example.Foo#bar"));
    assert!(!ocaml::is_ocaml(b"Java_com_example_Foo_bar"));
    assert!(!ocaml::is_ocaml(b"DELPHI_PROBE_$$_UNITPROC$LONGINT"));

    // Module records and runtime helpers have no dot, so detection claims them
    // end to end.
    for symbol in [
        "camlProbe",
        "camlStdlib__List",
        "caml_alloc",
        "caml_program",
    ] {
        assert!(ocaml::is_ocaml(symbol.as_bytes()), "{symbol}");
        assert_eq!(Scheme::detect(symbol), Scheme::OCaml, "{symbol}");
        assert!(demangler.demangle(symbol).is_ok(), "{symbol}");
    }

    // Dotted members, static-data labels, doubled-underscore runtime markers and
    // host fragments are claimed by OCaml before Go and route end to end.
    for (symbol, expected) in [
        ("camlProbe.entry", "Probe.entry"),
        ("camlStdlib__List.map_569", "Stdlib__List.map"),
        ("camlStdlib.11", "Stdlib.11"),
        ("caml_system__code_begin", "system__code_begin"),
        ("caml_ba_compare.cold", "caml_ba_compare"),
        ("caml_ephe_clean.part.0", "caml_ephe_clean"),
    ] {
        assert!(ocaml::is_ocaml(symbol.as_bytes()), "{symbol}");
        assert_eq!(
            ocaml::parse(symbol.as_bytes()).unwrap().to_string(),
            expected,
            "{symbol}"
        );
        assert_eq!(Scheme::detect(symbol), Scheme::OCaml, "{symbol}");
        assert_eq!(
            demangler.demangle(symbol).unwrap().to_string(),
            expected,
            "{symbol}"
        );
    }
}

/// Corpus provenance.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Source {
    /// Generated from `refs/ocaml/fixture/` with OCaml 5.3.0.
    Fixture,
    /// Runtime helpers of the linked fixture program.
    Runtime,
    /// Runtime pseudo-unit markers and rejected runtime forms of the linked
    /// fixture program.
    Link,
    /// Host-compiler fragments of runtime helpers in the linked fixture program.
    Fragment,
    /// The installed OCaml 5.3.0 standard library archive.
    Stdlib,
}

impl Source {
    fn parse(text: &str) -> Option<Self> {
        match text {
            "fixture" => Some(Self::Fixture),
            "runtime" => Some(Self::Runtime),
            "link" => Some(Self::Link),
            "fragment" => Some(Self::Fragment),
            "stdlib" => Some(Self::Stdlib),
            _ => None,
        }
    }

    /// True when a corpus of this provenance may carry this classification.
    fn allows(self, class: Class) -> bool {
        match self {
            Self::Fixture => matches!(class, Class::Exact | Class::Unsupported),
            Self::Runtime => class == Class::Identity,
            Self::Link => matches!(class, Class::Exact | Class::Unsupported),
            Self::Fragment => class == Class::Exact,
            Self::Stdlib => matches!(class, Class::Contract | Class::Unsupported),
        }
    }
}

/// Corpus classification.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Class {
    /// `ocaml::parse` prints the expected contract text, authored from source.
    Exact,
    /// `ocaml::parse` prints the documented transform of a real symbol.
    Contract,
    /// `ocaml::parse` prints the runtime helper as it stands.
    Identity,
    /// The backend must reject the name.
    Unsupported,
}

impl Class {
    fn parse(text: &str) -> Option<Self> {
        match text {
            "exact" => Some(Self::Exact),
            "contract" => Some(Self::Contract),
            "identity" => Some(Self::Identity),
            "unsupported" => Some(Self::Unsupported),
            _ => None,
        }
    }
}

/// Measured row counts, by source and classification.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Counts {
    fixture_exact: usize,
    fixture_unsupported: usize,
    runtime_identity: usize,
    link_exact: usize,
    link_unsupported: usize,
    fragment_exact: usize,
    stdlib_contract: usize,
    stdlib_unsupported: usize,
}

impl Counts {
    fn observe(&mut self, source: Source, class: Class) {
        let slot = match (source, class) {
            (Source::Fixture, Class::Exact) => &mut self.fixture_exact,
            (Source::Fixture, Class::Unsupported) => &mut self.fixture_unsupported,
            (Source::Runtime, Class::Identity) => &mut self.runtime_identity,
            (Source::Link, Class::Exact) => &mut self.link_exact,
            (Source::Link, Class::Unsupported) => &mut self.link_unsupported,
            (Source::Fragment, Class::Exact) => &mut self.fragment_exact,
            (Source::Stdlib, Class::Contract) => &mut self.stdlib_contract,
            (Source::Stdlib, Class::Unsupported) => &mut self.stdlib_unsupported,
            // `Source::allows` rejects every other combination.
            (_, Class::Exact | Class::Contract | Class::Identity | Class::Unsupported) => {
                &mut self.stdlib_unsupported
            }
        };
        *slot += 1;
    }
}
