//! D corpus gate.
//!
//! The corpus is generated from the D ABI spec, LDC 1.40 and the Debian
//! libphobos 2.110.0 shared library; expected text comes from
//! `c++filt -s dlang`. Every exact row must round-trip through
//! [`demangle::d::parse`], every negative/unsupported row must be rejected, and
//! no oracle gap may be echoed as a successful demangling.
//!
//! The test skips when `refs/d/` is absent, as it is in the published crate.

#![cfg(feature = "d")]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use demangle::d;

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
    assert!(rows.len() >= 100, "hand corpus is unexpectedly small");
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

    let mut counts = app;
    for (category, count) in phobos {
        *counts.entry(category).or_insert(0) += count;
    }

    let exact = app_rows + phobos_rows;
    assert!(exact >= 500, "generated corpus has only {exact} exact rows");

    // The plan's category set. A category may be absent only if the corpus
    // genuinely contains no such symbol; these are the categories the pinned
    // libphobos/LDC corpus is expected to exercise.
    for category in [
        "array",
        "class",
        "constant",
        "delegate",
        "function",
        "local",
        "qualified",
        "runtime-special",
        "template",
        "typeinfo",
    ] {
        assert!(
            counts.get(category).copied().unwrap_or(0) > 0,
            "generated corpus has no `{category}` rows"
        );
    }
}

#[test]
fn negative_and_unsupported_forms_are_rejected() {
    let Some(root) = corpus_root() else {
        eprintln!("refs/d is absent; skipping D rejection corpus");
        return;
    };

    let mut failures = Vec::new();
    for file in ["negative.tsv", "unsupported.tsv"] {
        for (symbol, reason) in read_rejections(&root.join(file)) {
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
fn oracle_gaps_are_not_echoed() {
    let Some(root) = corpus_root() else {
        eprintln!("refs/d is absent; skipping D oracle-gap corpus");
        return;
    };

    let rows = read_rejections(&root.join("oracle-gap.tsv"));
    assert!(!rows.is_empty(), "oracle-gap corpus is empty");

    let mut echoed = Vec::new();
    for (symbol, reason) in rows {
        if let Ok(ast) = d::parse(&symbol) {
            let actual = ast.to_string();
            if actual == symbol {
                echoed.push(format!("{symbol}: echoed ({reason})"));
            }
        }
    }

    assert!(
        echoed.is_empty(),
        "{} oracle gaps were echoed as exact; first failures:\n{}",
        echoed.len(),
        echoed
            .iter()
            .take(8)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    );
}
