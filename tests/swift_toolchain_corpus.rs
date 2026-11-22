//! Swift symbols extracted from the official Swift 6.4.2 Linux toolchain.
//!
//! `refs/` is excluded from the published crate, so this test skips when the
//! corpus is absent. Expectations were produced with the toolchain's own
//! `swift-demangle` (see `refs/swift.md`).
#![cfg(feature = "swift")]

use std::fmt::Write as _;

const FLOOR: usize = 23_975;

#[test]
fn swift_toolchain_corpus() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/refs/swift/toolchain-6.4.2-manglings.txt"
    );
    let Ok(data) = std::fs::read_to_string(path) else {
        eprintln!("swift toolchain corpus not present (published crate); skipping");
        return;
    };

    let demangler = demangle::Demangler::new();
    let mut total = 0_usize;
    let mut pass = 0_usize;
    let mut wrong = 0_usize;
    let mut errors = 0_usize;
    let mut examples: Vec<(String, String, String)> = Vec::new();

    for line in data.lines() {
        let Some((mangled, expected)) = line.split_once(" ---> ") else {
            continue;
        };
        total += 1;
        let mut rendered = String::new();
        match demangler.demangle(mangled) {
            Ok(demangled) => {
                let _ = write!(rendered, "{demangled}");
                if rendered == expected {
                    pass += 1;
                } else {
                    wrong += 1;
                    if examples.len() < 10 {
                        examples.push((mangled.to_owned(), expected.to_owned(), rendered));
                    }
                }
            }
            Err(_) => {
                errors += 1;
                if examples.len() < 10 {
                    examples.push((
                        mangled.to_owned(),
                        expected.to_owned(),
                        "<error>".to_owned(),
                    ));
                }
            }
        }
    }

    for (mangled, expected, got) in &examples {
        eprintln!("{mangled}\n  want: {expected}\n  got : {got}");
    }
    eprintln!("swift toolchain corpus: {pass}/{total} exact, {wrong} wrong, {errors} errors");

    assert_eq!(
        total, 23_975,
        "toolchain corpus changed without updating the pin"
    );
    assert!(
        pass >= FLOOR,
        "toolchain corpus regressed: {pass} < {FLOOR}"
    );
    assert_eq!(wrong, 0, "silent wrong-output regressions: {wrong}");
}
