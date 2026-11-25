//! Rust demangling corpus gate.
//!
//! Reads the corpora vendored under `refs/rust/`, derived from
//! `rustc-demangle`'s own test modules at the pinned upstream commit, plus
//! toolchain-derived `libstd`/`librustc_driver` symbol lists from legacy
//! 1.75.0 and v0 1.100.0.
//! `refs/` is excluded from the published crate, so every test here skips
//! gracefully when the corpus is absent.
//!
//! Two independent checks run over the same symbols:
//!
//! 1. the in-tree parser and printer must reproduce the upstream expectation
//!    byte for byte in the file's mode (`Display` or `{:#}`), and
//! 2. whatever the in-tree parser accepts must also match the locked
//!    `rustc-demangle` 0.1.28 oracle in both modes.
//!
//! Check 1 is the gate; check 2 proves that no accepted symbol is silently
//! demangled differently from the reference.
#![cfg(feature = "rust")]

use std::collections::BTreeSet;
use std::fmt::Write as _;

/// Cases in the vendored `{:#}` corpus whose expectations the locked backend
/// cannot produce verbatim.
///
/// The vendored `rustc-demangle` is upstream `main`, which is ahead of the
/// `0.1.28` release in `Cargo.lock`: 0.1.28 prints `#[splat] ` where the
/// pinned corpus expects `#[rustc_splat] `. The in-tree printer follows the
/// pinned reference and prints `#[rustc_splat] `; [`check_oracle`] normalizes
/// the older spelling so the rest of the oracle output is still compared.
/// The corpus is never rewritten to match the older release.
const SPLAT_ORACLE_DIVERGENCE: [&str; 8] = [
    "_RMNvCsiksvdpJ6Jnj_14splat_mangling4mainINtB0_4TypeFwThmEEuE",
    "_RMs0_NvCsiksvdpJ6Jnj_14splat_mangling4mainINtB3_4TypeFwTThmEEEuE",
    "_RMs2_NvCsiksvdpJ6Jnj_14splat_mangling4mainINtB3_4TypeFwTmaEdEuE",
    "_RMs2_NvCsiksvdpJ6Jnj_14splat_mangling4mainINtB3_4TypeINtNtCsiksvdpJ6Jnj_5alloc5boxed3BoxFmwTFwuEuaEdEuEE",
    "_RMs2_NvCsiksvdpJ6Jnj_14splat_mangling4mainINtB3_4TypePFwTmaEEuE",
    "_RMs6_NvCsiksvdpJ6Jnj_14splat_mangling4mainINtB3_4TypeOFmawTdEEuE",
    "_RMs8_NvCsiksvdpJ6Jnj_14splat_mangling4mainINtB3_4TypeRFmwTafjEdEuE",
    "_RNvMNtCs5CcWRJzwAYz_4core6optionINtB2_6OptionFwTlEEuE6unwrapCsiksvdpJ6Jnj_14splat_mangling",
];

/// Measured stage gate: pair lines the in-tree parser reproduces exactly.
///
/// The corpus is fully implemented, including the `w` splat marker. Raise this
/// floor whenever coverage improves; never lower it.
const STAGE_GATE: usize = 115;

/// Number of pair lines across both corpus files.
const CORPUS_PAIRS: usize = 115;

/// Number of unique symbols across both corpus files.
const CORPUS_SYMBOLS: usize = 107;

/// Toolchain-derived corpora: `<file>` and symbol count.
///
/// These are real `libstd` and `librustc_driver` symbols from Rust 1.75.0 and
/// 1.100.0, covering legacy and v0 encodings. The test compares both printers
/// with the `rustc-demangle` oracle at run time, so the files contain only
/// inputs.
const TOOLCHAIN_CORPORA: [(&str, usize); 5] = [
    ("toolchain-1.75.0-legacy.txt", 1180),
    ("toolchain-1.75.0-rustc-legacy.txt", 381),
    ("toolchain-1.75.0-rustc-v0.txt", 2000),
    ("toolchain-1.100.0-v0.txt", 1397),
    ("toolchain-1.100.0-rustc-v0.txt", 2000),
];

/// One corpus file: its name and whether it holds `{:#}` expectations.
struct Corpus {
    file: &'static str,
    alternate: bool,
}

const CORPORA: [Corpus; 2] = [
    Corpus {
        file: "rustc-demangle-tests.txt",
        alternate: false,
    },
    Corpus {
        file: "rustc-demangle-tests-nohash.txt",
        alternate: true,
    },
];

#[test]
fn rust_corpus() {
    let mut total = 0;
    let mut pass = 0;
    let mut wrong: Vec<(String, String, String)> = Vec::new();
    let mut unsupported = 0;
    let mut oracle_mismatch: Vec<(String, String)> = Vec::new();
    let mut symbols = BTreeSet::new();

    for corpus in &CORPORA {
        let Some(data) = read_corpus(corpus.file) else {
            eprintln!("rust corpus not present (published crate); skipping");
            return;
        };
        for line in data.lines() {
            let Some((mangled, expected)) = line.split_once(" ---> ") else {
                continue;
            };
            // The expected side is compared verbatim: a leading space is part
            // of the demangled text (e.g. `$u20$` at the start of an element).
            // Only the mangled side may carry stray whitespace.
            let mangled = mangled.trim();
            let expected = expected.strip_suffix('\r').unwrap_or(expected);
            if mangled.is_empty() {
                continue;
            }
            total += 1;
            let _ = symbols.insert(mangled.to_owned());

            let rendered = match demangler::rust::parse(mangled) {
                Ok(ast) => {
                    let mut text = String::new();
                    if corpus.alternate {
                        let _ = write!(text, "{ast:#}");
                    } else {
                        let _ = write!(text, "{ast}");
                    }
                    text
                }
                Err(_) => {
                    unsupported += 1;
                    continue;
                }
            };

            if rendered == expected {
                pass += 1;
            } else {
                wrong.push((mangled.to_owned(), expected.to_owned(), rendered.clone()));
            }
            check_oracle(mangled, &rendered, corpus.alternate, &mut oracle_mismatch);
        }
    }

    eprintln!(
        "rust corpus: {pass}/{total} exact, {unsupported} unsupported, {} wrong",
        wrong.len()
    );
    for (mangled, expected, rendered) in wrong.iter().take(20) {
        eprintln!("{mangled}\n  want: {expected}\n  got : {rendered}");
    }
    for (mangled, detail) in oracle_mismatch.iter().take(20) {
        eprintln!("{mangled}\n  oracle mismatch: {detail}");
    }

    assert_eq!(total, CORPUS_PAIRS, "corpus pair lines changed");
    assert_eq!(symbols.len(), CORPUS_SYMBOLS, "corpus symbols changed");
    assert!(
        wrong.is_empty(),
        "{} symbols demangled to the wrong text",
        wrong.len()
    );
    assert!(
        oracle_mismatch.is_empty(),
        "{} accepted symbols disagree with the locked rustc-demangle",
        oracle_mismatch.len()
    );
    assert_eq!(
        unsupported, 0,
        "{unsupported} corpus symbols are still unsupported"
    );
    assert!(
        pass >= STAGE_GATE,
        "rust corpus regressed: {pass}/{total} exact (stage gate {STAGE_GATE})"
    );
}

/// The runtime budget must not change corpus output.
///
/// Every symbol detection claims as Rust must demangle through the
/// [`demangler::Demangler`] path to exactly the text the crate's `rust::parse`
/// produces, both with the implicit default budget and with an explicit
/// [`demangler::Budget::default`].
#[test]
fn rust_corpus_output_is_unchanged_under_the_default_budget() {
    let default = demangler::Demangler::new();
    let explicit = demangler::Demangler::with_budget(demangler::Budget::default());
    let mut compared = 0;
    let mut symbols = 0;

    let corpora = CORPORA
        .iter()
        .map(|corpus| corpus.file)
        .chain(TOOLCHAIN_CORPORA.iter().map(|(file, _)| *file));
    for file in corpora {
        let Some(data) = read_corpus(file) else {
            eprintln!("rust corpus not present (published crate); skipping");
            return;
        };
        for line in data.lines() {
            // The pair corpora carry `mangled ---> expected` lines.
            let symbol = line.split(" ---> ").next().unwrap_or_default().trim();
            if symbol.is_empty() || symbol.starts_with('#') {
                continue;
            }
            let Ok(ast) = demangler::rust::parse(symbol) else {
                continue;
            };
            let scheme = match ast.encoding() {
                demangler::rust::Encoding::Legacy => demangler::Scheme::RustLegacy,
                demangler::rust::Encoding::V0 => demangler::Scheme::RustV0,
            };
            symbols += 1;
            for demangler in [&default, &explicit] {
                let Ok(demangled) = demangler.demangle(symbol) else {
                    // Detection is a prefilter and may claim another scheme.
                    continue;
                };
                if demangled.scheme() != scheme {
                    continue;
                }
                assert_eq!(demangled.to_string(), ast.to_string(), "{file}: {symbol}");
                compared += 1;
            }
        }
    }

    eprintln!("rust budget parity: {compared} comparisons over {symbols} symbols");
    assert!(
        compared > 1000,
        "only {compared} comparisons; the detected Rust corpus changed"
    );
}

/// Compare every symbol from the toolchain-derived corpora with the
/// locked `rustc-demangle` oracle in both display modes.
#[test]
fn rust_toolchain_corpus() {
    for (file, expected_total) in TOOLCHAIN_CORPORA {
        let Some(data) = read_corpus(file) else {
            eprintln!("rust toolchain corpus not present (published crate); skipping");
            return;
        };
        let mut total = 0;
        for symbol in data.lines().map(str::trim).filter(|line| !line.is_empty()) {
            total += 1;
            let ast = demangler::rust::parse(symbol)
                .unwrap_or_else(|error| panic!("{file}: {symbol}: {error:?}"));
            let oracle = rustc_demangle::try_demangle(symbol)
                .unwrap_or_else(|error| panic!("{file}: {symbol}: oracle rejected: {error:?}"));

            let display = ast.to_string();
            let alternate = format!("{ast:#}");
            assert_eq!(display, oracle.to_string(), "{file}: {symbol}");
            assert_eq!(
                alternate,
                format!("{oracle:#}"),
                "{file}: {symbol} (alternate)"
            );
        }
        assert_eq!(total, expected_total, "{file}: symbol count changed");
        eprintln!("rust toolchain corpus {file}: {total}/{expected_total} exact");
    }
}

/// Compare an accepted symbol with the locked `rustc-demangle` oracle.
fn check_oracle(
    mangled: &str,
    rendered: &str,
    alternate: bool,
    mismatches: &mut Vec<(String, String)>,
) {
    let Ok(oracle) = rustc_demangle::try_demangle(mangled) else {
        mismatches.push((mangled.to_owned(), "oracle rejected".to_owned()));
        return;
    };
    let expected = if alternate {
        format!("{oracle:#}")
    } else {
        format!("{oracle}")
    };
    // The locked release predates the pinned `#[rustc_splat]` spelling.
    let expected = if SPLAT_ORACLE_DIVERGENCE.contains(&mangled) {
        expected.replace("#[splat] ", "#[rustc_splat] ")
    } else {
        expected
    };
    if expected != rendered {
        mismatches.push((mangled.to_owned(), format!("oracle {expected:?}")));
    }
}

#[test]
fn rust_corpus_errors_are_rejected() {
    let Some(data) = read_corpus("rustc-demangle-tests-errors.txt") else {
        eprintln!("rust corpus not present (published crate); skipping");
        return;
    };

    let mut total = 0;
    let mut failures: Vec<String> = Vec::new();
    for line in data.lines() {
        let symbol = line.trim();
        if symbol.is_empty() {
            continue;
        }
        total += 1;
        if let Ok(ast) = demangler::rust::parse(symbol) {
            failures.push(format!("{symbol:?} parsed as {ast:?}"));
        }
        if rustc_demangle::try_demangle(symbol).is_ok() {
            failures.push(format!("{symbol:?} accepted by the oracle"));
        }
    }

    eprintln!(
        "rust error corpus: {total} inputs, {} accepted",
        failures.len()
    );
    for failure in &failures {
        eprintln!("{failure}");
    }
    assert_eq!(total, 4, "error corpus changed");
    assert!(failures.is_empty(), "malformed symbols were accepted");
}

/// Every proper prefix of every corpus symbol must be handled without a panic
/// or a budget escape. Some prefixes are valid symbols (dropping a `.llvm.`
/// suffix, for example), so acceptance is counted, not asserted.
#[test]
fn rust_corpus_truncations_are_rejected() {
    let mut checked = 0;
    let mut accepted = 0;
    for corpus in &CORPORA {
        let Some(data) = read_corpus(corpus.file) else {
            eprintln!("rust corpus not present (published crate); skipping");
            return;
        };
        for line in data.lines() {
            let Some((mangled, _)) = line.split_once(" ---> ") else {
                continue;
            };
            let mangled = mangled.trim();
            for end in 0..mangled.len() {
                let Some(prefix) = mangled.get(..end) else {
                    continue;
                };
                checked += 1;
                if demangler::rust::parse(prefix).is_ok() {
                    accepted += 1;
                }
            }
        }
    }
    eprintln!("rust truncations: {checked} prefixes, {accepted} still valid");
    assert!(checked > 0, "corpus contained no symbols");
}

/// The vendored `early-recursion-limit` fixture is a negative corpus: the
/// reference rejects every symbol with `RecursedTooDeep`, and so must we.
#[test]
fn rust_recursion_limit_is_rejected() {
    let path = format!(
        "{}/refs/rust/rustc-demangle/src/v0-large-test-symbols/early-recursion-limit",
        env!("CARGO_MANIFEST_DIR")
    );
    let Ok(data) = std::fs::read_to_string(path) else {
        eprintln!("rust recursion fixture not present (published crate); skipping");
        return;
    };

    let mut total = 0;
    let mut recursed = 0;
    let mut rejected_earlier: Vec<String> = Vec::new();
    let mut accepted: Vec<String> = Vec::new();
    for line in data.lines() {
        let symbol = line.trim();
        if symbol.is_empty() || symbol.starts_with('#') {
            continue;
        }
        total += 1;
        match demangler::rust::parse(symbol) {
            Err(demangler::rust::ParseError::RecursedTooDeep) => recursed += 1,
            Err(error) => rejected_earlier.push(format!(
                "{:?} rejected with {error:?}",
                &symbol[..symbol.len().min(40)]
            )),
            Ok(_) => accepted.push(format!("{:?}", &symbol[..symbol.len().min(40)])),
        }
    }
    eprintln!(
        "rust recursion fixture: {total} symbols, {recursed} recursed, \
         {} rejected earlier, {} accepted",
        rejected_earlier.len(),
        accepted.len()
    );
    for detail in rejected_earlier.iter().chain(&accepted) {
        eprintln!("{detail}");
    }
    assert_eq!(total, 4, "recursion fixture changed");
    assert!(
        accepted.is_empty(),
        "recursion fixture symbols were accepted"
    );
    assert!(recursed > 0, "the depth limit was never exercised");
}

/// Read one corpus file, or `None` when `refs/` is absent.
fn read_corpus(file: &str) -> Option<String> {
    let path = format!("{}/refs/rust/corpus/{file}", env!("CARGO_MANIFEST_DIR"));
    std::fs::read_to_string(path).ok()
}
