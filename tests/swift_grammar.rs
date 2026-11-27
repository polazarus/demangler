//! Swift productions that the vendored corpora do not exercise.
//!
//! `data/swift_grammar.tsv` lists global suffix operators and SIL function
//! types generated per grammar production and kept only where this crate and
//! `swift-demangle` print the same text. The table ships with the crate, so
//! these productions stay tested without `refs/`.
#![cfg(feature = "swift")]

use demangler::swift;

const TABLE: &str = include_str!("data/swift_grammar.tsv");

#[test]
fn rare_productions_print() {
    let mut rows = 0_usize;
    for line in TABLE.lines().filter(|line| !line.starts_with('#')) {
        let (symbol, expected) = line.split_once('\t').expect("tab-separated row");
        let ast =
            swift::parse(symbol.as_bytes()).unwrap_or_else(|error| panic!("{symbol}: {error}"));
        assert_eq!(ast.to_string(), expected, "{symbol}");
        rows += 1;
    }
    assert!(rows > 300, "only {rows} rows");
}
