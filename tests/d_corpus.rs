//! D corpus gate.
//!
//! The corpus is generated from the D ABI spec, LDC 1.40 and the Debian
//! libphobos 2.110.0 shared library; expected text comes from
//! `c++filt -s dlang`. Every exact row must round-trip through
//! [`demangler::d::parse`], every negative/unsupported row must be rejected, and
//! no oracle gap may be echoed as a successful demangling.
//!
//! The test skips when `refs/d/` is absent, as it is in the published crate.

#![cfg(feature = "d")]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use demangler::d;

/// Pinned hand-corpus size; raise it when coverage improves, never lower it.
const HAND_ROWS: usize = 173;

/// Pinned generated-corpus sizes (LDC probe object and libphobos).
const LDC_ROWS: usize = 36;
const PHOBOS_ROWS: usize = 10_718;
const GENERATED_ROWS: usize = LDC_ROWS + PHOBOS_ROWS;

/// Pinned rejection and oracle-gap corpus sizes.
const NEGATIVE_ROWS: usize = 24;
const UNSUPPORTED_ROWS: usize = 2;
const ORACLE_GAP_ROWS: usize = 0;

/// Pinned category counts over the generated rows, measured from the corpus.
///
/// Categories are multi-label, so the counts sum to more than the row count.
/// The measured breakdown is recorded in `refs/d.md`.
const GENERATED_CATEGORIES: &[(&str, usize)] = &[
    ("array", 4_206),
    ("class", 710),
    ("constant", 3_190),
    ("delegate", 192),
    ("function", 9_147),
    ("local", 111),
    ("qualified", 10_345),
    ("runtime-special", 1_520),
    ("template", 7_083),
    ("typeinfo", 432),
];

/// One `symbol<TAB>expected<TAB>categories` row.
struct ExactRow {
    symbol: String,
    expected: String,
    categories: Vec<String>,
}

fn corpus_root() -> Option<PathBuf> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("refs/d");
    root.is_dir().then_some(root)
}

fn read_rows(path: &Path) -> Vec<ExactRow> {
    let text = std::fs::read_to_string(path)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()));
    text.lines()
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(|line| {
            let mut fields = line.splitn(3, '\t');
            let symbol = fields.next().unwrap_or_default();
            let expected = fields.next().unwrap_or_default();
            let categories = fields.next().unwrap_or_default();
            assert!(
                !symbol.is_empty() && !expected.is_empty(),
                "malformed corpus row in {}: {line}",
                path.display()
            );
            ExactRow {
                symbol: symbol.to_owned(),
                expected: expected.to_owned(),
                categories: categories
                    .split(',')
                    .filter(|category| !category.is_empty())
                    .map(str::to_owned)
                    .collect(),
            }
        })
        .collect()
}

fn read_rejections(path: &Path) -> Vec<(String, String)> {
    let text = std::fs::read_to_string(path)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()));
    text.lines()
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(|line| {
            let mut fields = line.splitn(2, '\t');
            let symbol = fields.next().unwrap_or_default();
            let reason = fields.next().unwrap_or_default();
            assert!(
                !symbol.is_empty() && !reason.is_empty(),
                "malformed rejection row in {}: {line}",
                path.display()
            );
            (symbol.to_owned(), reason.to_owned())
        })
        .collect()
}

/// Check every exact row and return the row count and category counts.
fn check_exact(path: &Path) -> (usize, BTreeMap<String, usize>) {
    let rows = read_rows(path);
    let mut counts = BTreeMap::new();
    let mut failures = Vec::new();

    for row in &rows {
        match d::parse(&row.symbol) {
            Ok(ast) => {
                let actual = ast.to_string();
                if actual != row.expected {
                    failures.push(format!(
                        "{}: expected {:?}, got {:?}",
                        row.symbol, row.expected, actual
                    ));
                }
            }
            Err(error) => failures.push(format!("{}: rejected: {error}", row.symbol)),
        }
        for category in &row.categories {
            *counts.entry(category.clone()).or_insert(0) += 1;
        }
    }

    assert!(
        failures.is_empty(),
        "{} has {} wrong rows; first failures:\n{}",
        path.display(),
        failures.len(),
        failures
            .iter()
            .take(8)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    );
    (rows.len(), counts)
}

#[test]
fn hand_corpus_is_exact() {
    let Some(root) = corpus_root() else {
        eprintln!("refs/d is absent; skipping D hand corpus");
        return;
    };
    let rows = read_rows(&root.join("hand.tsv"));
    assert_eq!(rows.len(), HAND_ROWS, "hand corpus row count changed");
    let _ = check_exact(&root.join("hand.tsv"));
}

#[test]
fn generated_corpus_is_exact() {
    let Some(root) = corpus_root() else {
        eprintln!("refs/d is absent; skipping D generated corpus");
        return;
    };

    let (app_rows, app) = check_exact(&root.join("ldc-1.40-app.tsv"));
    let (phobos_rows, phobos) = check_exact(&root.join("libphobos-2.110.0.tsv"));

    assert_eq!(app_rows, LDC_ROWS, "LDC corpus row count changed");
    assert_eq!(
        phobos_rows, PHOBOS_ROWS,
        "libphobos corpus row count changed"
    );
    assert_eq!(
        app_rows + phobos_rows,
        GENERATED_ROWS,
        "generated corpus row count changed"
    );

    let mut counts = app;
    for (category, count) in phobos {
        *counts.entry(category).or_insert(0) += count;
    }

    // Pin the measured per-category coverage so a corpus regression cannot
    // hide behind a total that still clears a loose floor.
    let mut failures = Vec::new();
    for (category, expected) in GENERATED_CATEGORIES {
        let actual = counts.remove(*category).unwrap_or(0);
        if actual != *expected {
            failures.push(format!(
                "{category}: expected {expected} rows, got {actual}"
            ));
        }
    }
    for (category, actual) in &counts {
        failures.push(format!(
            "{category}: unexpected category with {actual} rows"
        ));
    }
    assert!(
        failures.is_empty(),
        "generated corpus category counts changed; first failures:\n{}",
        failures.join("\n")
    );
}

#[test]
fn negative_and_unsupported_forms_are_rejected() {
    let Some(root) = corpus_root() else {
        eprintln!("refs/d is absent; skipping D rejection corpus");
        return;
    };

    let mut failures = Vec::new();
    for (file, expected) in [
        ("negative.tsv", NEGATIVE_ROWS),
        ("unsupported.tsv", UNSUPPORTED_ROWS),
    ] {
        let rows = read_rejections(&root.join(file));
        assert_eq!(rows.len(), expected, "{file} row count changed");
        for (symbol, reason) in rows {
            if d::parse(&symbol).is_ok() {
                failures.push(format!("{symbol}: accepted ({reason})"));
            }
        }
    }

    assert!(
        failures.is_empty(),
        "{} rejected forms were accepted; first failures:\n{}",
        failures.len(),
        failures
            .iter()
            .take(8)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    );
}

#[test]
fn oracle_gaps_are_rejected() {
    let Some(root) = corpus_root() else {
        eprintln!("refs/d is absent; skipping D oracle-gap corpus");
        return;
    };

    let rows = read_rejections(&root.join("oracle-gap.tsv"));
    assert_eq!(
        rows.len(),
        ORACLE_GAP_ROWS,
        "oracle-gap corpus row count changed"
    );

    let mut accepted = Vec::new();
    for (symbol, _reason) in rows {
        if d::parse(&symbol).is_ok() {
            accepted.push(symbol);
        }
    }

    assert!(
        accepted.is_empty(),
        "{} oracle gaps unexpectedly parsed; first failures:\n{}",
        accepted.len(),
        accepted.iter().take(8).cloned().collect::<Vec<_>>().join("\n")
    );
}
