//! Objective-C corpus gate.
//!
//! Reads the corpora under `refs/objc/corpus/`. The Apple and GNUstep files
//! are symbol names emitted by Clang 19.1.7 from generated Objective-C sources;
//! the GNU file is the metadata subset of Debian GCC 14's `libobjc.a`; the
//! methods file is the display names `llvm-nm` prints for the generated
//! objects. `rejected.txt` holds runtime helpers, local labels and truncated
//! prefixes that must not be rendered as Objective-C metadata.
//!
//! `refs/` is excluded from the published crate, so the test skips gracefully
//! when the corpus is absent.

#![cfg(feature = "objc")]

use demangle::{Demangler, Language, Scheme};

/// Metadata records emitted for the generated Apple/Mach-O objects.
const APPLE: &str = "refs/objc/corpus/apple-clang-19.txt";
/// Metadata records emitted for the generated GNUstep/GNU-runtime objects.
const GNUSTEP: &str = "refs/objc/corpus/gnustep-clang-19.txt";
/// Metadata records from Debian GCC 14's `libobjc.a`.
const LIBOBJC: &str = "refs/objc/corpus/gcc-libobjc-14.txt";
/// Real Apache-2.0 App Store app: Mattermost 2.46.0 iOS executable.
const MATTERMOST: &str = "refs/objc/corpus/mattermost-2.46.0-ios.txt";
/// Method display names printed by `llvm-nm` for the generated objects.
const METHODS: &str = "refs/objc/corpus/methods-clang-19.txt";
/// Names that must not be rendered as Objective-C metadata.
const REJECTED: &str = "refs/objc/corpus/rejected.txt";

/// Pinned corpus sizes. These are small, generated corpora, so the gate is
/// exact rather than a floor.
const APPLE_NAMES: usize = 88;
const GNUSTEP_NAMES: usize = 21;
const LIBOBJC_NAMES: usize = 51;
const MATTERMOST_NAMES: usize = 18_931;
const METHOD_NAMES: usize = 26;
const REJECTED_NAMES: usize = 17;

#[test]
fn apple_metadata_corpus() {
    check_metadata(APPLE, APPLE_NAMES);
}

#[test]
fn gnustep_metadata_corpus() {
    check_metadata(GNUSTEP, GNUSTEP_NAMES);
}

#[test]
fn gcc_libobjc_corpus() {
    check_metadata(LIBOBJC, LIBOBJC_NAMES);
}

#[test]
fn mattermost_real_app_corpus() {
    check_metadata(MATTERMOST, MATTERMOST_NAMES);
}

#[test]
fn method_display_corpus() {
    let Some(names) = read(METHODS) else {
        return;
    };
    assert_eq!(names.len(), METHOD_NAMES, "{METHODS}");
    let demangler = Demangler::new();
    for name in &names {
        assert_eq!(Scheme::detect(name), Scheme::ObjC, "{name}");
        let demangled = demangler
            .demangle(name)
            .unwrap_or_else(|error| panic!("{name}: {error}"));
        assert_eq!(demangled.scheme(), Scheme::ObjC, "{name}");
        assert_eq!(demangled.language(), Language::ObjectiveC, "{name}");
        assert_eq!(demangled.to_string(), *name, "{name}");
    }
}

#[test]
fn mattermost_spot_checks_are_exact() {
    let expected = [
        (
            "__OBJC_$_CLASS_METHODS_MSIDTelemetry(Internal|Cache)",
            "Objective-C class methods MSIDTelemetry (Internal|Cache)",
        ),
        (
            "_OBJC_IVAR_$_AnimationFrameQueue.timeDeltas_",
            "Objective-C ivar AnimationFrameQueue.timeDeltas_",
        ),
        (
            "__OBJC_$_CATEGORY_CLASS_METHODS_NSRunLoop_$_SRWebSocket",
            "Objective-C category class methods NSRunLoop (SRWebSocket)",
        ),
        ("_OBJC_EHTYPE_$_NSError", "Objective-C exception type NSError"),
        (
            "__OBJC_$_CLASS_PROP_LIST_MSIDTelemetry",
            "Objective-C class property list MSIDTelemetry",
        ),
    ];
    let demangler = Demangler::new();
    for (symbol, want) in expected {
        let got = demangler
            .demangle(symbol)
            .unwrap_or_else(|error| panic!("{symbol}: {error}"));
        assert_eq!(got.to_string(), want, "{symbol}");
    }
}

#[test]
fn decorated_metadata_is_normalized() {
    let demangler = Demangler::new();
    for (symbol, want) in [
        (
            "__objc_ivar_offset_Foo._x@@GNUstep2.0",
            "Objective-C ivar offset Foo._x",
        ),
        (
            "_OBJC_IVAR_$_Foo._ivar@VERS_1",
            "Objective-C ivar Foo._ivar",
        ),
    ] {
        assert_eq!(demangler.demangle(symbol).unwrap().to_string(), want);
    }
}

#[test]
fn block_control_bytes_are_rejected() {
    for symbol in [
        "__block_descriptor_40_\u{1b}[2J",
        "__block_descriptor_40_\u{7}",
        "__block_descriptor_40_\u{7f}",
    ] {
        assert!(demangle::objc::parse(symbol).is_err(), "{symbol:?}");
    }
    assert!(demangle::objc::parse("__block_descriptor_40_\u{1}").is_ok());
}

#[test]
fn rejected_names_are_not_objc_metadata() {
    let Some(names) = read(REJECTED) else {
        return;
    };
    assert_eq!(names.len(), REJECTED_NAMES, "{REJECTED}");
    let demangler = Demangler::new();
    for name in &names {
        if let Ok(demangled) = demangler.demangle(name) {
            assert_ne!(
                demangled.scheme(),
                Scheme::ObjC,
                "{name} was rendered as {}",
                demangled
            );
        }
    }
}

/// Check one metadata corpus file.
fn check_metadata(path: &str, expected: usize) {
    let Some(names) = read(path) else {
        return;
    };
    assert_eq!(names.len(), expected, "{path}");

    let demangler = Demangler::new();
    let mut failures: Vec<String> = Vec::new();
    for name in &names {
        if !demangle::objc::is_objc(name) {
            failures.push(format!("{name}: is_objc returned false"));
            continue;
        }
        if Scheme::detect(name) != Scheme::ObjC {
            failures.push(format!("{name}: not detected as Objective-C"));
            continue;
        }
        let Ok(demangled) = demangler.demangle(name) else {
            failures.push(format!("{name}: parse failed"));
            continue;
        };
        if demangled.scheme() != Scheme::ObjC {
            failures.push(format!("{name}: wrong scheme"));
            continue;
        }
        if demangled.language() != Language::ObjectiveC {
            failures.push(format!("{name}: wrong language"));
            continue;
        }
        let rendered = demangled.to_string();
        if !rendered.starts_with("Objective-C ") {
            failures.push(format!("{name}: unexpected output {rendered}"));
            continue;
        }
        if rendered.len() > name.len() + 64 {
            failures.push(format!(
                "{name}: output grew from {} to {} bytes",
                name.len(),
                rendered.len()
            ));
        }
    }

    for failure in failures.iter().take(30) {
        eprintln!("{failure}");
    }
    assert!(failures.is_empty(), "{path}: {} failures", failures.len());
}

/// Read a corpus file, or return `None` when `refs/` is absent.
fn read(path: &str) -> Option<Vec<String>> {
    let path = format!("{}/{path}", env!("CARGO_MANIFEST_DIR"));
    let Ok(data) = std::fs::read_to_string(&path) else {
        eprintln!("objc corpus {path} not present (published crate); skipping");
        return None;
    };
    let names: Vec<String> = data
        .lines()
        .map(str::trim_end)
        .filter(|line| !line.is_empty())
        .map(str::to_owned)
        .collect();
    assert!(!names.is_empty(), "{path}: corpus is empty");
    Some(names)
}
