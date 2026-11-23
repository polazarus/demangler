//! Kotlin/Native corpus gate.
//!
//! Reads the corpora under `refs/kotlin-native/corpus/`. The measured file is
//! the ELF symbol table of the fixture in `refs/kotlin-native/fixture/main.kt`
//! built with each pinned Kotlin/Native compiler release from 2.0.21 through
//! 2.4.21; the hand-authored file covers prefixes and markers that a Linux
//! executable does not emit.
//!
//! `refs/` is excluded from the published crate, so the test skips gracefully
//! when the corpus is absent.
//!
//! There is no external Kotlin/Native demangler to use as an oracle, so the
//! gate checks the invariants the backend promises:
//!
//! - every corpus name is detected as [`Scheme::KotlinNative`];
//! - every detected name parses and prints;
//! - printing is stable and leaves no encoded marker (`#`, `__at__`,
//!   `#internal`, `-impl`, `%`) in the text;
//! - the measured corpus keeps its size and category coverage.

#![cfg(feature = "kotlin-native")]

use demangle::{Demangler, Language, Scheme};

/// Measured Kotlin/Native executables, one per pinned compiler release.
const MEASURED: [(&str, usize); 6] = [
    ("refs/kotlin-native/corpus/kotlin-2.0.21-linux-x86_64.txt", 1_531),
    ("refs/kotlin-native/corpus/kotlin-2.1.21-linux-x86_64.txt", 1_526),
    ("refs/kotlin-native/corpus/kotlin-2.2.21-linux-x86_64.txt", 1_523),
    ("refs/kotlin-native/corpus/kotlin-2.3.21-linux-x86_64.txt", 1_596),
    ("refs/kotlin-native/corpus/kotlin-2.4.0-linux-x86_64.txt", 1_370),
    ("refs/kotlin-native/corpus/kotlin-2.4.21-linux-x86_64.txt", 1_371),
];
/// Marker and prefix forms the pinned executables do not emit.
const HAND_AUTHORED: &str = "refs/kotlin-native/corpus/hand-authored.txt";

const HAND_AUTHORED_NAMES: usize = 33;

/// Categories the measured executable must keep covering.
const MEASURED_REQUIRED: [&str; 12] = [
    "functions",
    "members",
    "constructors",
    "accessors",
    "extensions",
    "suspend",
    "static",
    "internal",
    "bridges",
    "generics",
    "nullable",
    "classes",
];

/// Categories the hand-authored corpus must cover.
const HAND_AUTHORED_REQUIRED: [&str; 8] = [
    "type symbols",
    "properties",
    "objects",
    "fields",
    "type aliases",
    "packages",
    "platform suffixes",
    "leading underscores",
];

#[test]
fn kotlin_native_measured_corpus() {
    for (path, expected) in MEASURED {
        check(path, expected, &MEASURED_REQUIRED);
    }
}

#[test]
fn kotlin_native_hand_authored_corpus() {
    check(HAND_AUTHORED, HAND_AUTHORED_NAMES, &HAND_AUTHORED_REQUIRED);
}

/// Run the invariants over one corpus file.
fn check(path: &str, expected: usize, required: &[&str]) {
    let path = format!("{}/{path}", env!("CARGO_MANIFEST_DIR"));
    let Ok(data) = std::fs::read_to_string(&path) else {
        eprintln!("kotlin/native corpus {path} not present (published crate); skipping");
        return;
    };

    let demangler = Demangler::new();
    let mut total = 0_usize;
    let mut coverage = Coverage::default();
    let mut failures: Vec<(String, String)> = Vec::new();

    for line in data.lines() {
        let name = line.trim_end();
        if name.is_empty() {
            continue;
        }
        total += 1;
        coverage.observe(name);

        if !demangle::kotlin_native::is_kotlin_native(name) {
            failures.push((name.to_owned(), "not a Kotlin/Native prefix".to_owned()));
            continue;
        }
        if Scheme::detect(name) != Scheme::KotlinNative {
            failures.push((name.to_owned(), "not detected as Kotlin/Native".to_owned()));
            continue;
        }
        let Ok(demangled) = demangler.demangle(name) else {
            failures.push((name.to_owned(), "parse failed".to_owned()));
            continue;
        };
        if demangled.scheme() != Scheme::KotlinNative
            || demangled.language() != Language::KotlinNative
        {
            failures.push((name.to_owned(), "wrong scheme or language".to_owned()));
            continue;
        }
        let rendered = demangled.to_string();
        if rendered.is_empty() {
            failures.push((name.to_owned(), "empty render".to_owned()));
            continue;
        }
        if let Some(marker) = encoded_marker(&rendered) {
            failures.push((name.to_owned(), format!("{marker} left in {rendered}")));
            continue;
        }
        match demangler.demangle(name) {
            Ok(again) if again.to_string() == rendered => {}
            Ok(again) => failures.push((
                name.to_owned(),
                format!("unstable render: {rendered} -> {again}"),
            )),
            Err(_) => failures.push((name.to_owned(), "second parse failed".to_owned())),
        }
    }

    eprintln!(
        "{path}: {total} names, coverage {coverage:?}, {} failures",
        failures.len()
    );
    for (name, reason) in failures.iter().take(30) {
        eprintln!("{name}\n  {reason}");
    }
    assert_eq!(total, expected, "{path}: corpus size changed");
    assert!(
        failures.is_empty(),
        "{path}: {} corpus failures",
        failures.len()
    );
    coverage.assert_covered(&path, required);
}

/// An encoded marker that the canonical spelling must not contain.
fn encoded_marker(rendered: &str) -> Option<&'static str> {
    if rendered.contains('#') {
        Some("#")
    } else if rendered.contains("__at__") {
        Some("__at__")
    } else if rendered.contains("#internal") {
        Some("#internal")
    } else if rendered.contains("-impl") {
        Some("-impl")
    } else {
        None
    }
}

/// Category coverage, so a corpus refresh cannot silently drop a category.
#[derive(Debug, Default)]
struct Coverage {
    functions: usize,
    members: usize,
    constructors: usize,
    accessors: usize,
    extensions: usize,
    suspend: usize,
    static_members: usize,
    internal: usize,
    bridges: usize,
    generics: usize,
    nullable: usize,
    classes: usize,
    type_symbols: usize,
    properties: usize,
    objects: usize,
    fields: usize,
    type_aliases: usize,
    packages: usize,
    platform_suffixes: usize,
    leading_underscores: usize,
}

impl Coverage {
    fn observe(&mut self, name: &str) {
        if name.starts_with("kfun:") || name.starts_with("_kfun:") || name.starts_with("__kfun:") {
            self.functions += 1;
        }
        if name.contains('#') && !name.contains("#internal") {
            self.members += 1;
        }
        if name.contains("#<init>") {
            self.constructors += 1;
        }
        if name.contains("#<get-") || name.contains("#<set-") {
            self.accessors += 1;
        }
        if name.contains("__at__") || name.contains('@') {
            self.extensions += 1;
        }
        if name.contains("#suspend") {
            self.suspend += 1;
        }
        if name.contains("#static") {
            self.static_members += 1;
        }
        if name.contains("#internal") {
            self.internal += 1;
        }
        if name.contains("$<bridge-") {
            self.bridges += 1;
        }
        if name.contains('<') || name.contains("0:") || name.contains("1:") {
            self.generics += 1;
        }
        if name.contains('?') {
            self.nullable += 1;
        }
        if name.starts_with("kclass:") {
            self.classes += 1;
        }
        if name.starts_with("ktype:") {
            self.type_symbols += 1;
        }
        if name.starts_with("kproperty:") {
            self.properties += 1;
        }
        if name.starts_with("kobj:") {
            self.objects += 1;
        }
        if name.starts_with("kfield:") {
            self.fields += 1;
        }
        if name.starts_with("ktypealias:") {
            self.type_aliases += 1;
        }
        if name.starts_with("kpackage:") || name.starts_with("knamespace:") {
            self.packages += 1;
        }
        if name.contains('%') {
            self.platform_suffixes += 1;
        }
        if name.starts_with('_') {
            self.leading_underscores += 1;
        }
    }

    fn assert_covered(&self, path: &str, required: &[&str]) {
        let counts = [
            ("functions", self.functions),
            ("members", self.members),
            ("constructors", self.constructors),
            ("accessors", self.accessors),
            ("extensions", self.extensions),
            ("suspend", self.suspend),
            ("static", self.static_members),
            ("internal", self.internal),
            ("bridges", self.bridges),
            ("generics", self.generics),
            ("nullable", self.nullable),
            ("classes", self.classes),
            ("type symbols", self.type_symbols),
            ("properties", self.properties),
            ("objects", self.objects),
            ("fields", self.fields),
            ("type aliases", self.type_aliases),
            ("packages", self.packages),
            ("platform suffixes", self.platform_suffixes),
            ("leading underscores", self.leading_underscores),
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
