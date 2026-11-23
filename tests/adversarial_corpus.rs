//! Adversarial symbol-name corpora.
//!
//! These are stress inputs, not expected-output pairs. Each line must
//! terminate, must not panic, and must not emit unbounded text. Recursive
//! bombs are intentionally oversized; a backend may reject them, but it must
//! not overflow the stack, loop forever, or allocate without a bound.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use demangle::Demangler;

const CORPORA: &[&str] = &[
    "cpp-itanium.txt",
    "cpp-msvc.txt",
    "cpp-legacy.txt",
    "rust.txt",
    "swift.txt",
    "go.txt",
    "objc.txt",
    "kotlin-native.txt",
    "d.txt",
    "jni.txt",
    "ada.txt",
    "coff.txt",
    "c-plain.txt",
];

/// A generous ceiling above every backend's declared print budget.
const MAX_ADVERSARIAL_OUTPUT: usize = 8 * 1024 * 1024;

#[test]
fn adversarial_corpora_terminate_and_stay_bounded() {
    let base = format!("{}/refs/adversarial", env!("CARGO_MANIFEST_DIR"));
    let mut found = false;
    for file in CORPORA {
        let path = format!("{base}/{file}");
        let Ok(data) = std::fs::read_to_string(&path) else {
            eprintln!("skipping adversarial corpus {path}: not present");
            continue;
        };
        found = true;
        for (index, line) in data.lines().enumerate() {
            if line.is_empty() {
                continue;
            }
            let name = format!("{file}:{}", index + 1);
            let bytes = line.as_bytes().to_vec();
            let (tx, rx) = mpsc::channel();
            let _ = thread::spawn(move || {
                let result = catch_unwind(AssertUnwindSafe(|| {
                    let demangler = Demangler::new();
                    demangler
                        .demangle_bytes(&bytes)
                        .map_or(0, |demangled| demangled.to_string().len())
                }));
                let _ = tx.send(result);
            });
            match rx.recv_timeout(Duration::from_secs(2)) {
                Ok(Ok(size)) => assert!(
                    size <= MAX_ADVERSARIAL_OUTPUT,
                    "{name}: rendered {size} bytes"
                ),
                Ok(Err(_)) => panic!("{name}: parser or printer panicked"),
                Err(_) => panic!("{name}: did not terminate within 2s"),
            }
        }
    }
    assert!(found, "no adversarial corpora found under {base}");
}
