//! Microsoft C++ corpus harness.
//!
//! Extracts the `?`-prefixed symbols from the vendored LLVM `llvm-undname` lit
//! tests and measures parser acceptance and demangled text. The lit tests are
//! reference material: when `refs/` is absent the tests skip instead of
//! failing.
//!
//! `msvc-demangler` is used only as an independent oracle, never by the crate.
//! The two implementations disagree on a few inputs (notably trailing
//! characters that LLVM tolerates and the oracle rejects, and one local-scope
//! form the oracle rejects) and on some output spellings, so the assertions pin
//! measured counts rather than exact parity.

use std::collections::BTreeMap;
use std::path::Path;
use std::string::String;
use std::vec::Vec;

use demangle::cpp::{self, Kind, NodeRef, ParseErrorKind};

const MSVC_DIR: &str = "refs/cpp/msvc";

/// Measured floor for accepted non-invalid lit symbols. Raise it when coverage
/// improves; never lower it.
const MIN_ACCEPTED: usize = 1_210;

/// Every symbol in `invalid-manglings.test` must be rejected.
const INVALID_TOTAL: usize = 80;

/// Real PE export dumps from the pinned PyTorch Windows wheel.
const REAL_BINARIES: [(&str, usize); 3] = [
    ("real-torch-2.5.1-cpu-win64-c10.txt", 831),
    ("real-torch-2.5.1-cpu-win64-fbgemm.txt", 941),
    ("real-torch-2.5.1-cpu-win64-torch_cpu.txt", 57_476),
];

#[test]
fn msvc_lit_corpus_baseline() {
    let Ok(symbols) = lit_symbols(MSVC_DIR) else {
        eprintln!("skipping MSVC corpus: {MSVC_DIR} not found");
        return;
    };
    let valid: Vec<&String> = symbols
        .iter()
        .filter(|(file, _)| file != "invalid-manglings.test")
        .map(|(_, symbol)| symbol)
        .collect();
    let total = valid.len();
    let mut accepted = 0_usize;
    let mut rejected = 0_usize;
    let mut oracle_accepted = 0_usize;
    let mut oracle_only = 0_usize;
    let mut reasons: BTreeMap<String, usize> = BTreeMap::new();
    let mut samples: Vec<String> = Vec::new();

    for symbol in &valid {
        let ours = cpp::parse_msvc(symbol);
        let oracle = msvc_demangler::parse(symbol).is_ok();
        if oracle {
            oracle_accepted += 1;
        }
        match ours {
            Ok(ast) => {
                accepted += 1;
                assert!(
                    !has_opaque(&ast.root()),
                    "{symbol} accepted with an Opaque node"
                );
                assert_eq!(ast.root().kind(), Kind::Encoding, "{symbol}");
            }
            Err(error) => {
                rejected += 1;
                if oracle {
                    oracle_only += 1;
                }
                *reasons.entry(format!("{:?}", error.kind())).or_default() += 1;
                if samples.len() < 8 {
                    samples.push(format!("{symbol}: {error}"));
                }
            }
        }
    }

    eprintln!(
        "msvc corpus: total={total} accepted={accepted} rejected={rejected} \
         oracle_accepted={oracle_accepted} oracle_only={oracle_only}"
    );
    eprintln!("msvc corpus rejections by kind: {reasons:?}");
    for sample in &samples {
        eprintln!("  sample: {sample}");
    }

    assert_eq!(total, 1_219, "lit symbol count changed");
    assert_eq!(accepted + rejected, total);
    assert_eq!(rejected, total - accepted);
    assert!(
        accepted >= MIN_ACCEPTED,
        "accepted {accepted} is below the measured floor {MIN_ACCEPTED}"
    );
    assert!(
        accepted > oracle_accepted,
        "in-tree parser should accept at least as much as the oracle"
    );
    assert!(
        oracle_only <= 5,
        "unexpectedly many symbols accepted only by the oracle: {oracle_only}"
    );
}

#[test]
fn msvc_invalid_manglings_are_rejected() {
    let Ok(symbols) = lit_symbols(MSVC_DIR) else {
        eprintln!("skipping MSVC corpus: {MSVC_DIR} not found");
        return;
    };
    let invalid: Vec<&String> = symbols
        .iter()
        .filter(|(file, _)| file == "invalid-manglings.test")
        .map(|(_, symbol)| symbol)
        .collect();
    assert_eq!(invalid.len(), INVALID_TOTAL, "invalid symbol count changed");
    let mut accepted = Vec::new();
    for symbol in invalid {
        if cpp::parse_msvc(symbol).is_ok() {
            accepted.push(symbol.clone());
        }
    }
    assert!(
        accepted.is_empty(),
        "invalid symbols accepted: {accepted:?}"
    );
}

/// Real stripped PE export tables exercise the parser on shipped C++ names.
///
/// These DLLs have no COFF symbol table, so only exported decorated names are
/// visible. That is the useful set for a runtime demangler.
#[test]
fn msvc_real_binary_exports_parse() {
    for (file, expected_total) in REAL_BINARIES {
        let path = Path::new(MSVC_DIR).join(file);
        let Ok(data) = std::fs::read_to_string(&path) else {
            eprintln!("skipping real MSVC corpus: {} not found", path.display());
            return;
        };
        let mut total = 0_usize;
        let mut failures = Vec::new();
        for symbol in data.lines().map(str::trim).filter(|line| !line.is_empty()) {
            total += 1;
            match cpp::parse_msvc(symbol) {
                Ok(ast) => {
                    assert_eq!(ast.root().kind(), Kind::Encoding, "{symbol}");
                    assert!(!has_opaque(&ast.root()), "{symbol} contains Opaque");
                }
                Err(error) => failures.push(format!("{symbol}: {error}")),
            }
        }
        assert_eq!(total, expected_total, "{file}: symbol count changed");
        assert!(
            failures.is_empty(),
            "{file}: {} symbols failed to parse: {:?}",
            failures.len(),
            failures.iter().take(5).collect::<Vec<_>>()
        );
        eprintln!("msvc real corpus {file}: {total}/{total} parsed");
    }
}

#[test]
fn msvc_shapes_are_inspectable() {
    let function = shape("?foo@@YAXXZ");
    assert_eq!(function.kind(), Kind::Function);
    assert_eq!(function.text(), Some("__cdecl"));
    assert_eq!(
        kinds(&function),
        vec![
            Kind::UnqualifiedName,
            Kind::BuiltinType,
            Kind::ParameterList
        ]
    );

    let method = shape("?method@Struct@@QEAAXXZ");
    assert_eq!(method.kind(), Kind::MemberFunction);
    assert_eq!(method.text(), Some("public: __cdecl"));
    assert_eq!(method.child(0).unwrap().kind(), Kind::NestedName);

    let variable = shape("?x@@3HA");
    assert_eq!(variable.kind(), Kind::Variable);
    assert_eq!(variable.text(), Some("global"));
    assert_eq!(
        kinds(&variable),
        vec![Kind::UnqualifiedName, Kind::BuiltinType]
    );

    let member_variable = shape("?var@Struct@@2HA");
    assert_eq!(member_variable.kind(), Kind::Variable);
    assert_eq!(member_variable.text(), Some("public-static"));

    let pointer = shape("?ptr@@3PEAHA");
    assert_eq!(pointer.child(1).unwrap().kind(), Kind::PointerType);

    let thunk = shape("?f@C@@WBA@EAAHXZ");
    assert_eq!(thunk.kind(), Kind::Thunk);
    assert_eq!(thunk.text(), Some("adjustor{16}"));
    let inner = thunk.child(0).unwrap();
    assert_eq!(inner.kind(), Kind::MemberFunction);
    assert!(inner.text().unwrap().contains("virtual"));

    let vftable = shape("??_7Base@@6B@");
    assert_eq!(vftable.kind(), Kind::Vftable);
    assert_eq!(vftable.text(), Some("const"));
    assert_eq!(vftable.child(0).unwrap().kind(), Kind::NestedName);

    let vbtable = shape("??_8Middle2@@7B@");
    assert_eq!(vbtable.kind(), Kind::Vbtable);
    assert_eq!(vbtable.text(), Some("const"));

    let type_descriptor = shape("??_R0?AUBase@@@8");
    assert_eq!(type_descriptor.kind(), Kind::Rtti);
    assert_eq!(type_descriptor.text(), Some("type-descriptor"));
    assert_eq!(type_descriptor.child(0).unwrap().kind(), Kind::TaggedType);

    let base_descriptor = shape("??_R1A@?0A@EA@Base@@8");
    assert_eq!(base_descriptor.kind(), Kind::Rtti);
    assert_eq!(
        base_descriptor.text(),
        Some("base-class-descriptor (0, -1, 0, 64)")
    );

    let object_locator = shape("??_R4Base@@6B@");
    assert_eq!(object_locator.kind(), Kind::Rtti);
    assert!(
        object_locator
            .text()
            .unwrap()
            .starts_with("complete-object-locator")
    );

    let string = shape("??_C@_02PCEFGMJL@hi?$AA@");
    assert_eq!(string.kind(), Kind::StringLiteral);
    assert_eq!(string.text(), Some("hi"));

    let vcall = shape("??_9Base@@$B7AA");
    assert_eq!(vcall.kind(), Kind::VCallThunk);
    assert!(vcall.text().unwrap().starts_with("vcall{8"));

    let guard = shape("??_B?1??getS@@YAAAUS@@XZ@51");
    assert_eq!(guard.kind(), Kind::GuardVariable);
    assert_eq!(guard.text(), Some("2"));
    assert_eq!(guard.child(0).unwrap().kind(), Kind::NestedName);

    let thread_guard = shape("?$TSS0@?1??f@@YAHXZ@4HA");
    assert_eq!(thread_guard.kind(), Kind::GuardVariable);
    assert_eq!(thread_guard.text(), Some("0"));
    assert_eq!(thread_guard.child(0).unwrap().kind(), Kind::LocalName);

    let initializer = shape("??__EFoo@@YAXXZ");
    assert_eq!(initializer.kind(), Kind::DynamicInitializer);

    let static_initializer = shape("??__E?i@C@@0HA@@YAXXZ");
    assert_eq!(static_initializer.kind(), Kind::DynamicInitializer);
    assert_eq!(static_initializer.child(0).unwrap().kind(), Kind::Variable);
    assert_eq!(
        static_initializer.child(0).unwrap().text(),
        Some("private-static")
    );

    let typeinfo_name = shape(".?AUStruct@@");
    assert_eq!(typeinfo_name.kind(), Kind::Rtti);
    assert_eq!(typeinfo_name.text(), Some("type-descriptor-name"));

    let md5 = shape("??@a6a285da2eea70dba6b578022be61d81@");
    assert_eq!(md5.kind(), Kind::Md5Name);
    assert_eq!(md5.text(), Some("a6a285da2eea70dba6b578022be61d81"));

    let template = shape("??$taker@HHU?$Single@U?$Pair@DF@@@@@@YAHHHU?$Single@U?$Pair@DF@@@@@Z");
    let name = template.child(0).unwrap();
    assert_eq!(name.kind(), Kind::Name);
    assert_eq!(name.child(1).unwrap().kind(), Kind::TemplateArgs);
    assert!(!has_opaque(&template));
}

#[test]
fn msvc_dispatch_and_rejection() {
    assert!(cpp::is_msvc("?x@@3HA"));
    assert!(cpp::is_msvc(".?AUStruct@@"));
    assert!(!cpp::is_msvc("_Z1fv"));

    let ast = cpp::parse("?foo@@YAXXZ").unwrap();
    assert_eq!(ast.root().child(0).unwrap().kind(), Kind::Function);
    let itanium = cpp::parse("_Z1fv").unwrap();
    assert_eq!(itanium.root().child(0).unwrap().kind(), Kind::Function);

    let error = cpp::parse_msvc("_Z1fv").unwrap_err();
    assert_eq!(error.kind(), ParseErrorKind::NotMsvc);

    for symbol in [
        "?A@@",
        "?x@@3",
        "??_R0",
        "??_C@_0A@01234567@a",
        "?foo@@YAHH",
    ] {
        assert!(cpp::parse_msvc(symbol).is_err(), "{symbol} should fail");
    }
}

#[test]
fn msvc_budgets_are_enforced() {
    let mut long = String::from("?");
    long.push_str(&"A".repeat(65 * 1024));
    assert_eq!(
        cpp::parse_msvc(&long).unwrap_err().kind(),
        ParseErrorKind::TooLong
    );

    let mut deep = String::from("?x@@3");
    deep.push_str(&"PEA".repeat(300));
    deep.push_str("HA");
    assert_eq!(
        cpp::parse_msvc(&deep).unwrap_err().kind(),
        ParseErrorKind::TooDeep
    );

    let mut wide = String::from("?f@@YAX");
    wide.push_str(&"H".repeat(20_000));
    wide.push_str("@Z");
    assert_eq!(
        cpp::parse_msvc(&wide).unwrap_err().kind(),
        ParseErrorKind::TooManyNodes
    );
}

fn shape(symbol: &str) -> NodeRef<'_> {
    let ast = cpp::parse_msvc(symbol).unwrap_or_else(|error| panic!("{symbol}: {error}"));
    ast.root().child(0).unwrap()
}

fn kinds(node: &NodeRef<'_>) -> Vec<Kind> {
    node.children().iter().map(|child| child.kind()).collect()
}

fn has_opaque(node: &NodeRef<'_>) -> bool {
    node.kind() == Kind::Opaque || node.children().iter().any(has_opaque)
}

/// Read every `?`/`.?` symbol line from the vendored lit tests.
fn lit_symbols(dir: &str) -> Result<Vec<(String, String)>, std::io::Error> {
    let mut files = Vec::new();
    for entry in std::fs::read_dir(Path::new(dir))? {
        let path = entry?.path();
        if path.extension().and_then(|extension| extension.to_str()) == Some("test") {
            files.push(path);
        }
    }
    files.sort();
    let mut symbols = Vec::new();
    for path in files {
        let name = path
            .file_name()
            .map_or_else(String::new, |name| name.to_string_lossy().into_owned());
        let source = std::fs::read_to_string(&path)?;
        for line in source.lines() {
            let line = line.trim_end();
            if line.starts_with('?') || line.starts_with(".?") {
                symbols.push((name.clone(), line.to_string()));
            }
        }
    }
    Ok(symbols)
}

// -- text corpus ------------------------------------------------------------

/// Measured floor for byte-identical text against the `msvc-demangler` oracle.
/// Raise it when coverage improves; never lower it.
const MIN_EXACT: usize = 971;

/// Measured ceiling for oracle disagreements. The printer follows the vendored
/// `llvm-undname` tests (`refs/cpp/msvc`); `msvc-demangler` 0.11 targets an
/// older LLVM, so a bounded number of symbols differ. See
/// [`msvc_text_corpus_baseline`] for the divergence classes.
const MAX_WRONG: usize = 122;

/// Measured ceiling for symbols the printer cannot render from the AST.
const MAX_ERROR: usize = 31;

/// Compare the printer with `msvc-demangler` over the whole lit corpus.
///
/// Four classes are reported:
///
/// - `exact`: byte-identical to the oracle;
/// - `wrong`: both produced text, but they differ;
/// - `error`: the in-tree parser or printer failed;
/// - `unverified`: the oracle rejected the symbol, so it supplies no reference.
///
/// The disagreements are not a claim that the printer is wrong: where the
/// oracle and the vendored `llvm-undname` tests disagree, the printer follows
/// the lit tests. The measured classes are:
///
/// - nested template ids close with `>>` (21) and function types print
///   `__cdecl(void)` without a space (26); the oracle emits the older LLVM
///   spelling `> >` and `__cdecl (`;
/// - `$1?<symbol>` non-type arguments print as `&<symbol text>` (13) instead
///   of a synthesized member-pointer type;
/// - operator spellings follow the lit tests: `vector deleting dtor`,
///   `scalar deleting dtor`, `copy ctor closure`, ... (6);
/// - every `{for ...}` vftable base is listed (5) and vftables/vbtables do not
///   carry the oracle's stray `{for `'}}` suffix (1);
/// - thunk adjustments print as `` `adjustor{N}' `` (2), dynamic initializers
///   wrap the initialized entity (2), templated conversion operators keep
///   their target type (4) and the RTTI type descriptor uses a space (1);
/// - the rest are oracle defects: duplicated `const`, truncated
///   rvalue-reference arrays, mis-decoded integer template arguments, and a
///   `ctor<double>` spelling the lit tests do not use (33).
///
/// The `error` class is dominated by productions whose encoding the parser
/// does not retain, so no correct text exists: the decoded member-pointer
/// offsets of `$F`/`$G`/`$H`/`$I`/`$J` template arguments (25) and the
/// discarded signature of a dynamic initializer for a static data member (2).
#[test]
fn msvc_text_corpus_baseline() {
    let Ok(symbols) = lit_symbols(MSVC_DIR) else {
        eprintln!("skipping MSVC corpus: {MSVC_DIR} not found");
        return;
    };
    let valid: Vec<&String> = symbols
        .iter()
        .filter(|(file, _)| file != "invalid-manglings.test")
        .map(|(_, symbol)| symbol)
        .collect();
    let total = valid.len();
    let mut exact = 0_usize;
    let mut wrong = 0_usize;
    let mut error = 0_usize;
    let mut unverified = 0_usize;
    let mut mismatches: Vec<String> = Vec::new();
    let mut failures: BTreeMap<String, usize> = BTreeMap::new();

    for symbol in &valid {
        let ast = match cpp::parse_msvc(symbol) {
            Ok(ast) => ast,
            Err(parse_error) => {
                error += 1;
                *failures
                    .entry(format!("parse {parse_error:?}"))
                    .or_default() += 1;
                continue;
            }
        };
        let mut rendered = String::new();
        if let Err(print_error) = cpp::print_msvc(&ast, &mut rendered) {
            error += 1;
            *failures
                .entry(format!("print {print_error:?}"))
                .or_default() += 1;
            continue;
        }
        let Ok(expected) = msvc_demangler::demangle(symbol, msvc_demangler::DemangleFlags::llvm())
        else {
            unverified += 1;
            continue;
        };
        if rendered == expected {
            exact += 1;
        } else {
            wrong += 1;
            if mismatches.len() < 16 {
                mismatches.push(format!(
                    "{symbol}\n    oracle: {expected}\n    ours:   {rendered}"
                ));
            }
        }
    }

    eprintln!(
        "msvc text corpus: total={total} exact={exact} wrong={wrong} error={error} \
         unverified={unverified}"
    );
    eprintln!("msvc text failures: {failures:?}");
    for mismatch in &mismatches {
        eprintln!("  mismatch: {mismatch}");
    }

    assert_eq!(total, 1_219, "lit symbol count changed");
    assert_eq!(exact + wrong + error + unverified, total);
    assert!(
        exact >= MIN_EXACT,
        "exact {exact} is below the measured floor {MIN_EXACT}"
    );
    assert!(
        wrong <= MAX_WRONG,
        "wrong {wrong} is above the measured ceiling {MAX_WRONG}"
    );
    assert!(
        error <= MAX_ERROR,
        "error {error} is above the measured ceiling {MAX_ERROR}"
    );
}
