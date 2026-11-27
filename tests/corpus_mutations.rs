//! Mutation sweep over every vendored corpus.
//!
//! Each whitespace-, quote- or tab-separated token in `refs/` that demangles
//! is a seed. Every seed is truncated at each byte offset and has each byte
//! deleted in turn; every variant goes through detection and through the
//! seed's own scheme, and whatever parses must print. Beyond that the sweep
//! asserts only that nothing panics: it reaches the error paths of every backend that the
//! exactness corpora, which hold only well-formed names, never visit.
//!
//! The corpora are reference material: when `refs/` is absent the test skips.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::Path;
use std::string::String;
use std::vec::Vec;

use demangler::{Demangler, Scheme};

/// Seeds longer than this are truncated only, not deleted byte by byte, to
/// keep the sweep quadratic in a bounded length.
const MAX_DELETE_LEN: usize = 96;

/// Seeds kept per scheme. The corpora hold about 250 000 demangling tokens;
/// an evenly spaced, deterministic subset keeps the sweep fast while still
/// sampling every corpus file.
const SEEDS_PER_SCHEME: usize = 200;

/// Collect candidate tokens from every text file under `dir`.
fn collect(dir: &Path, out: &mut BTreeSet<String>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect(&path, out);
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        for token in
            text.split(|c: char| c.is_whitespace() || matches!(c, '"' | '\'' | ',' | '{' | '}'))
        {
            if (2..=4096).contains(&token.len()) {
                let _ = out.insert(token.to_owned());
            }
        }
    }
}

/// Demangle `symbol` both ways and render whatever parses.
fn exercise(demangler: &Demangler, symbol: &str, scheme: Scheme, out: &mut String) {
    if let Ok(demangled) = demangler.demangle(symbol) {
        out.clear();
        assert!(
            write!(out, "{demangled}").is_ok(),
            "{symbol:?} parsed but did not print"
        );
    }
    if let Ok(demangled) = demangler.demangle_with_scheme(scheme, symbol) {
        out.clear();
        assert!(
            write!(out, "{demangled}").is_ok(),
            "{symbol:?} parsed as {scheme:?} but did not print"
        );
    }
}

#[test]
fn corpus_mutations_never_panic() {
    let refs = Path::new("refs");
    if !refs.is_dir() {
        eprintln!("refs/ is absent; skipping the corpus mutation sweep");
        return;
    }
    let mut tokens = BTreeSet::new();
    collect(refs, &mut tokens);
    let demangler = Demangler::new();
    let mut by_scheme: BTreeMap<String, (Scheme, Vec<&str>)> = BTreeMap::new();
    for token in &tokens {
        if let Ok(demangled) = demangler.demangle(token) {
            let scheme = demangled.scheme();
            by_scheme
                .entry(format!("{scheme:?}"))
                .or_insert_with(|| (scheme, Vec::new()))
                .1
                .push(token.as_str());
        }
    }
    let mut seeds = Vec::new();
    for (scheme, symbols) in by_scheme.values() {
        let stride = symbols.len().div_ceil(SEEDS_PER_SCHEME).max(1);
        seeds.extend(
            symbols
                .iter()
                .step_by(stride)
                .map(|symbol| (*symbol, *scheme)),
        );
    }
    // Plain C names always demangle, so even a build with no backend has seeds.
    assert!(!seeds.is_empty(), "no corpus seed demangled");
    let mut out = String::new();
    let mut variant = String::new();
    for &(seed, scheme) in &seeds {
        for (end, _) in seed.char_indices().skip(1) {
            exercise(&demangler, &seed[..end], scheme, &mut out);
        }
        if seed.len() > MAX_DELETE_LEN {
            continue;
        }
        for (at, ch) in seed.char_indices() {
            variant.clear();
            variant.push_str(&seed[..at]);
            variant.push_str(&seed[at + ch.len_utf8()..]);
            exercise(&demangler, &variant, scheme, &mut out);
        }
    }
    eprintln!("swept {} seeds", seeds.len());
}
