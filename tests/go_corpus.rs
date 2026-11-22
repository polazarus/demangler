//! Go demangling corpus gate.
//!
//! Reads the corpora under `refs/go/corpus/`, extracted by
//! `refs/go/extract-go-symbols.sh` from a Go 1.24.4 toolchain: the ELF symbol
//! table of a generated test binary, the symbol table of a generated compiler
//! object file, and the symbol tables of exported package archives for a
//! fixed set of standard library and generated packages. `refs/` is excluded
//! from the published crate, so the test skips gracefully when the corpus is
//! absent.
//!
//! There is no external Go demangler to use as an oracle, so the gate checks
//! the invariants the backend promises instead:
//!
//! - every corpus name that has the shape of a Go name is detected as
//!   [`Scheme::Go`](demangle::Scheme::Go);
//! - every detected name parses and prints;
//! - printed text contains no `·`, no `%XX` escapes (outside the opaque
//!   `go:string.` and `type:.namedata.` payloads) and no ABI marker;
//! - printing is idempotent: demangling the printed text reproduces it.
//!
//! A pinned set of exact expectations in the unit tests covers the text of
//! each name category; this file proves the backend accepts real names.
#![cfg(feature = "go")]

use std::fmt::Write as _;

use demangle::{Demangler, Scheme};

/// The generated test binary's symbol table (inlined and non-inlined builds).
const BINARY_CORPUS: &str = "refs/go/corpus/go1.24.4-binary.txt";
/// The generated compiler object file's symbol table.
const OBJECT_CORPUS: &str = "refs/go/corpus/go1.24.4-object.txt";
/// Exported package archives: standard library packages plus generated ones.
const ARCHIVE_CORPUS: &str = "refs/go/corpus/go1.24.4-archive.txt";

/// Measured floors. Raise them when coverage improves; never lower them.
const BINARY_GO_NAMES: usize = 3_300;
const OBJECT_GO_NAMES: usize = 75;
const ARCHIVE_GO_NAMES: usize = 12_000;

/// Categories the generated binary must keep covering.
const BINARY_REQUIRED: [&str; 11] = [
    "methods",
    "closures",
    "wrappers",
    "generics",
    "shapes",
    "type descriptors",
    "specials",
    "itabs",
    "constants",
    "escaped packages",
    "abi markers",
];

/// Categories the compiler object file must keep covering.
const OBJECT_REQUIRED: [&str; 6] = [
    "closures",
    "type descriptors",
    "specials",
    "strings",
    "namedata",
    "data symbols",
];

/// Categories the exported package archives must keep covering.
const ARCHIVE_REQUIRED: [&str; 18] = [
    "methods",
    "closures",
    "wrappers",
    "generics",
    "shapes",
    "type descriptors",
    "specials",
    "itabs",
    "strings",
    "namedata",
    "escaped packages",
    "data symbols",
    "static temps",
    "jump tables",
    "eqfunc",
    "hashfunc",
    "qualified methods",
    "object versions",
];

#[test]
fn go_binary_corpus() {
    check(BINARY_CORPUS, BINARY_GO_NAMES, &BINARY_REQUIRED);
}

#[test]
fn go_object_corpus() {
    check(OBJECT_CORPUS, OBJECT_GO_NAMES, &OBJECT_REQUIRED);
}

#[test]
fn go_archive_corpus() {
    check(ARCHIVE_CORPUS, ARCHIVE_GO_NAMES, &ARCHIVE_REQUIRED);
}

/// Run the invariants over one corpus file.
fn check(path: &str, floor: usize, required: &[&str]) {
    let path = format!("{}/{path}", env!("CARGO_MANIFEST_DIR"));
    let Ok(data) = std::fs::read_to_string(&path) else {
        eprintln!("go corpus {path} not present (published crate); skipping");
        return;
    };

    let demangler = Demangler::new();
    let mut total = 0_usize;
    let mut go_names = 0_usize;
    let mut failures: Vec<(String, String)> = Vec::new();
    let mut coverage = Coverage::default();

    for line in data.lines() {
        let name = line.trim_end();
        if name.is_empty() {
            continue;
        }
        total += 1;
        if !demangle::go::is_go_name(name) {
            continue;
        }
        go_names += 1;
        coverage.observe(name);

        if Scheme::detect(name) != Scheme::Go {
            failures.push((name.to_owned(), "not detected as Go".to_owned()));
            continue;
        }
        let Ok(demangled) = demangler.demangle(name) else {
            failures.push((name.to_owned(), "parse failed".to_owned()));
            continue;
        };
        let rendered = demangled.to_string();

        if rendered.contains('·') {
            failures.push((name.to_owned(), format!("middle dot left in {rendered}")));
            continue;
        }
        if !has_opaque_payload(name) && rendered.contains('%') {
            failures.push((name.to_owned(), format!("escape left in {rendered}")));
            continue;
        }
        if has_abi_marker(name) && has_abi_marker(&rendered) {
            failures.push((name.to_owned(), format!("ABI marker left in {rendered}")));
            continue;
        }
        if let Some(version) = object_version(name)
            && object_version(&rendered) != Some(version)
        {
            failures.push((
                name.to_owned(),
                format!("object version <{version}> not preserved in {rendered}"),
            ));
            continue;
        }
        match demangler.demangle(&rendered) {
            Ok(again) if again.to_string() == rendered => {}
            Ok(again) => failures.push((
                name.to_owned(),
                format!("not idempotent: {rendered} -> {again}"),
            )),
            Err(_) => failures.push((
                name.to_owned(),
                format!("printed form does not re-parse: {rendered}"),
            )),
        }
    }

    eprintln!(
        "{path}: {go_names}/{total} Go names, coverage {coverage:?}, {} failures",
        failures.len()
    );
    for (name, reason) in failures.iter().take(30) {
        eprintln!("{name}\n  {reason}");
    }
    assert!(total > 0, "{path}: corpus is empty");
    assert!(
        failures.is_empty(),
        "{path}: {} corpus failures",
        failures.len()
    );
    assert!(
        go_names >= floor,
        "{path}: regressed to {go_names} Go names (floor {floor})"
    );
    coverage.assert_covered(&path, required);
}

/// True when the name carries an opaque payload that must be printed verbatim.
fn has_opaque_payload(name: &str) -> bool {
    name.starts_with("go:string.") || name.starts_with("type:.namedata.")
}

/// True when the name ends with one of the ABI markers.
fn has_abi_marker(name: &str) -> bool {
    [".abi0", ".abiinternal", ".abiwrap"]
        .iter()
        .any(|suffix| name.ends_with(suffix))
}

/// The object-file symbol version (`<N>`) appended by `go tool nm`, if any.
fn object_version(name: &str) -> Option<&str> {
    let rest = name.strip_suffix('>')?;
    let open = rest.rfind('<')?;
    let version = &rest[open + 1..];
    (!version.is_empty() && version.bytes().all(|byte| byte.is_ascii_digit())).then_some(version)
}

/// True when a suffix component is a `.jumpN` jump table.
fn has_jump_table(name: &str) -> bool {
    name.match_indices(".jump").any(|(start, _)| {
        name[start + ".jump".len()..]
            .bytes()
            .next()
            .is_some_and(|byte| byte.is_ascii_digit())
    })
}

/// True when a method name carries a package path qualifier (`(*T).pkg/path.m`).
fn has_qualified_method(name: &str) -> bool {
    let Some(receiver_end) = name.find(").") else {
        return false;
    };
    name[receiver_end + 2..]
        .split('.')
        .next()
        .is_some_and(|component| component.contains('/'))
}

/// Category coverage, so a corpus refresh cannot silently drop a category.
#[derive(Debug, Default)]
struct Coverage {
    methods: usize,
    closures: usize,
    wrappers: usize,
    generics: usize,
    shapes: usize,
    type_descriptors: usize,
    specials: usize,
    itabs: usize,
    strings: usize,
    namedata: usize,
    constants: usize,
    escaped_packages: usize,
    abi_markers: usize,
    data_symbols: usize,
    static_temps: usize,
    jump_tables: usize,
    eqfunc: usize,
    hashfunc: usize,
    qualified_methods: usize,
    object_versions: usize,
}

impl Coverage {
    fn observe(&mut self, name: &str) {
        if name.contains(").") {
            self.methods += 1;
        }
        if name.contains(".func") {
            self.closures += 1;
        }
        if name.contains(".gowrap")
            || name.contains(".deferwrap")
            || name.contains("-range")
            || name.ends_with("-fm")
        {
            self.wrappers += 1;
        }
        if name.contains('[') {
            self.generics += 1;
        }
        if name.contains("go.shape.") {
            self.shapes += 1;
        }
        if name.starts_with("type:") {
            self.type_descriptors += 1;
        }
        if name.starts_with("go:") {
            self.specials += 1;
        }
        if name.starts_with("go:itab.") {
            self.itabs += 1;
        }
        if name.starts_with("go:string.") {
            self.strings += 1;
        }
        if name.starts_with("type:.namedata.") {
            self.namedata += 1;
        }
        if name.starts_with('$') {
            self.constants += 1;
        }
        if name.contains('%') {
            self.escaped_packages += 1;
        }
        if has_abi_marker(name) {
            self.abi_markers += 1;
        }
        if name.contains(".gclocals") || name.contains(".gcbits") || name.contains(".gcargs") {
            self.data_symbols += 1;
        }
        if name.contains(".stmp_") {
            self.static_temps += 1;
        }
        if has_jump_table(name) {
            self.jump_tables += 1;
        }
        if name.starts_with("type:.eqfunc") {
            self.eqfunc += 1;
        }
        if name.starts_with("type:.hashfunc") {
            self.hashfunc += 1;
        }
        if has_qualified_method(name) {
            self.qualified_methods += 1;
        }
        if object_version(name).is_some() {
            self.object_versions += 1;
        }
    }

    fn assert_covered(&self, path: &str, required: &[&str]) {
        let counts = [
            ("methods", self.methods),
            ("closures", self.closures),
            ("wrappers", self.wrappers),
            ("generics", self.generics),
            ("shapes", self.shapes),
            ("type descriptors", self.type_descriptors),
            ("specials", self.specials),
            ("itabs", self.itabs),
            ("strings", self.strings),
            ("namedata", self.namedata),
            ("constants", self.constants),
            ("escaped packages", self.escaped_packages),
            ("abi markers", self.abi_markers),
            ("data symbols", self.data_symbols),
            ("static temps", self.static_temps),
            ("jump tables", self.jump_tables),
            ("eqfunc", self.eqfunc),
            ("hashfunc", self.hashfunc),
            ("qualified methods", self.qualified_methods),
            ("object versions", self.object_versions),
        ];
        let missing: Vec<&str> = counts
            .into_iter()
            .filter(|(name, count)| required.contains(name) && *count == 0)
            .map(|(name, _)| name)
            .collect();
        assert!(
            missing.is_empty(),
            "{path}: corpus lost coverage for {missing:?}"
        );
    }
}

/// Keep `std::fmt::Write` imported for the rendered-text diagnostics.
#[allow(dead_code)]
fn render(demangler: &Demangler, name: &str) -> String {
    let mut out = String::new();
    if let Ok(demangled) = demangler.demangle(name) {
        let _ = write!(out, "{demangled}");
    }
    out
}
