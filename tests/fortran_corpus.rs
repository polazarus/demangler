//! gfortran corpus gate.
//!
//! Reads `refs/fortran/corpus/gfortran-14.2-linux-x86_64.tsv`, generated from
//! the fixture under `refs/fortran/fixture/` with GNU Fortran 14.2.0 and
//! classified against the display contract documented in `refs/fortran.md`.
//! `refs/` is excluded from the published crate, so the test skips gracefully
//! when the corpus is absent.
//!
//! `c++filt` has no Fortran mode, so there is no demangler oracle: the expected
//! text is the project contract (dotted module/member spelling, external name
//! without its appended underscore, `MAIN__` as `main`, and a generated
//! derived-type helper as `<module>.<Type>'<helper>`).
//!
//! The gate checks:
//!
//! - every `exact` name parses and prints the expected text, and renders the
//!   same text again from a second parse;
//! - every `unsupported` name is rejected rather than rendered;
//! - module symbols are detected as [`Scheme::Fortran`] and demangled by
//!   [`Demangler`], while external names are not auto-claimed;
//! - ordinary C names with a trailing underscore stay C.

#![cfg(feature = "fortran")]

use std::fs;

use demangler::{Demangler, Language, Scheme, fortran};

/// Curated corpus: symbol, source, class, expected.
const CORPUS: &str = "refs/fortran/corpus/gfortran-14.2-linux-x86_64.tsv";

/// Exact measured counts. Raise them when coverage improves; never lower them.
const COUNTS: Counts = Counts {
    default_exact: 36,
    default_unsupported: 6,
    second_underscore_unsupported: 8,
    no_underscoring_unsupported: 7,
    hand_exact: 19,
    hand_unsupported: 22,
};

#[test]
fn fortran_corpus() {
    let path = format!("{}/{CORPUS}", env!("CARGO_MANIFEST_DIR"));
    let Ok(data) = fs::read_to_string(&path) else {
        eprintln!("fortran corpus {path} not present (published crate); skipping");
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
        if fields.len() != 4 {
            failures.push(format!(
                "line {line_number}: expected 4 fields, got {}",
                fields.len()
            ));
            continue;
        }
        let symbol = fields[0];
        let source = fields[1];
        let class = fields[2];
        let expected = fields[3];
        let Some(source) = Source::parse(source) else {
            failures.push(format!("line {line_number}: unknown source {source:?}"));
            continue;
        };
        let Some(class) = Class::parse(class) else {
            failures.push(format!("line {line_number}: unknown class {class:?}"));
            continue;
        };
        let Some(slot) = counts.slot(source, class) else {
            failures.push(format!(
                "line {line_number}: {source:?} names cannot be {class:?}"
            ));
            continue;
        };
        *slot += 1;

        check_entry(symbol, class, expected, &mut failures);
    }

    eprintln!("fortran corpus: {counts:?}, {} failures", failures.len());
    for failure in failures.iter().take(30) {
        eprintln!("{failure}");
    }
    assert!(
        failures.is_empty(),
        "{} fortran corpus failures",
        failures.len()
    );
    assert_eq!(counts, COUNTS, "fortran corpus counts changed");
}

/// Check one corpus row against the backend.
fn check_entry(symbol: &str, class: Class, expected: &str, failures: &mut Vec<String>) {
    let parsed = fortran::parse(symbol.as_bytes(), demangler::Budget::default());
    let detected = Scheme::detect(symbol) == Scheme::Fortran;
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
                    match fortran::parse(symbol.as_bytes(), demangler::Budget::default()) {
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
            if is_module_symbol(symbol) {
                if !detected {
                    failures.push(format!("{symbol}: module symbol not detected as Fortran"));
                    return;
                }
                match Demangler::new().demangle(symbol) {
                    Ok(demangled) => {
                        if demangled.scheme() != Scheme::Fortran
                            || demangled.language() != Language::Fortran
                        {
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
            } else if detected {
                failures.push(format!("{symbol}: external name was auto-detected"));
            }
        }
        Class::Unsupported => {
            if parsed.is_ok() {
                failures.push(format!("{symbol}: unsupported name was accepted"));
            }
            if !expected.is_empty() {
                failures.push(format!("{symbol}: unsupported row has expected text"));
            }
            if detected && Demangler::new().demangle(symbol).is_ok() {
                failures.push(format!("{symbol}: unsupported name was demangled"));
            }
        }
    }
}

/// True when the row is a `__<module>_MOD_<member>` symbol.
fn is_module_symbol(symbol: &str) -> bool {
    symbol.starts_with("__") && symbol.contains("_MOD_")
}

#[test]
fn fortran_detection_is_precision_only() {
    let demangler = Demangler::new();

    // Ordinary C names with a trailing underscore stay C and pass through.
    for symbol in ["foo_", "foo", "my_sub_", "main", "foo__bar"] {
        assert_eq!(Scheme::detect(symbol), Scheme::C, "{symbol}");
        assert_eq!(demangler.demangle(symbol).unwrap().to_string(), symbol);
    }

    // Legacy GNU v2 C++ also uses `__`, but only when its parser consumes the
    // whole symbol. Fortran must not take it over.
    assert_ne!(Scheme::detect("a__b__c"), Scheme::Fortran);
    #[cfg(feature = "cpp-gnu-v2")]
    assert_eq!(Scheme::detect("a__b__c"), Scheme::CppGnuV2);

    // The module form is claimed, including the generated derived-type helpers.
    for symbol in [
        "__fprobe_MOD_add",
        "__fprobe_child_MOD_do_child",
        "___fprobe_MOD_add",
        "__m_MOD_x.llvm.1234",
        "__fprobe_MOD___vtab_fprobe_Point",
        "__fprobe_MOD___def_init_fprobe_Point",
    ] {
        assert_eq!(Scheme::detect(symbol), Scheme::Fortran, "{symbol}");
        assert!(demangler.demangle(symbol).is_ok(), "{symbol}");
    }

    // A module symbol whose member this slice does not read is claimed as
    // Fortran and then rejected, never rendered as C.
    for symbol in [
        "__holder_MOD___vtab___iso_c_binding_C_ptr",
        "__m_MOD_X",
        "__m_MOD_x_MOD_y",
    ] {
        assert_eq!(Scheme::detect(symbol), Scheme::Fortran, "{symbol}");
        assert!(demangler.demangle(symbol).is_err(), "{symbol}");
    }

    // External names and the main program carry no distinctive marker.
    for symbol in ["ext_sub_", "MAIN__", "x_y__", "__ext_sub_", "m_MOD_x"] {
        assert_ne!(Scheme::detect(symbol), Scheme::Fortran, "{symbol}");
    }
}

/// Corpus provenance.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Source {
    /// Compiled with the default `-funderscoring`.
    Default,
    /// Compiled with `-fsecond-underscore`.
    SecondUnderscore,
    /// Compiled with `-fno-underscoring`.
    NoUnderscoring,
    /// Hand-authored detection and rejection cases.
    Hand,
}

impl Source {
    fn parse(text: &str) -> Option<Self> {
        match text {
            "default" => Some(Self::Default),
            "second-underscore" => Some(Self::SecondUnderscore),
            "no-underscoring" => Some(Self::NoUnderscoring),
            "hand" => Some(Self::Hand),
            _ => None,
        }
    }
}

/// Corpus classification.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Class {
    /// `fortran::parse` prints the expected text.
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
    default_exact: usize,
    default_unsupported: usize,
    second_underscore_unsupported: usize,
    no_underscoring_unsupported: usize,
    hand_exact: usize,
    hand_unsupported: usize,
}

impl Counts {
    /// The counter for one (source, class) pair.
    ///
    /// `-fsecond-underscore` and `-fno-underscoring` names are never exact:
    /// both modes produce external names that cannot be read back.
    fn slot(&mut self, source: Source, class: Class) -> Option<&mut usize> {
        Some(match (source, class) {
            (Source::Default, Class::Exact) => &mut self.default_exact,
            (Source::Default, Class::Unsupported) => &mut self.default_unsupported,
            (Source::SecondUnderscore, Class::Unsupported) => {
                &mut self.second_underscore_unsupported
            }
            (Source::NoUnderscoring, Class::Unsupported) => &mut self.no_underscoring_unsupported,
            (Source::Hand, Class::Exact) => &mut self.hand_exact,
            (Source::Hand, Class::Unsupported) => &mut self.hand_unsupported,
            (Source::SecondUnderscore | Source::NoUnderscoring, Class::Exact) => return None,
        })
    }
}
