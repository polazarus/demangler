use alloc::string::{String, ToString};

use super::*;
use crate::go::{DEFAULT_BUDGET, MAX_DEPTH, MAX_INPUT_LEN, parse};

/// Parse with the compiled-in budget, the way the public entry point does.
fn parse_with(symbol: &[u8]) -> Result<Ast<'_>, ParseError> {
    parse_bytes(symbol, ParseLimits::from_effective(DEFAULT_BUDGET))
}

fn text(name: &str) -> String {
    parse(name).unwrap().to_string()
}

#[test]
fn splits_package_and_function() {
    assert_eq!(text("fmt.Println"), "fmt.Println");
    assert_eq!(
        text("runtime.(*mLockProfile).captureStack"),
        "runtime.(*mLockProfile).captureStack"
    );
    assert_eq!(
        text("example.com/demangle/corpus/pkg%2ev1.Make"),
        "example.com/demangle/corpus/pkg.v1.Make"
    );
}

#[test]
fn parses_closures_and_wrappers() {
    assert_eq!(text("main.main.func1.1"), "main.main.func1.1");
    assert_eq!(
        text("fmt.(*pp).handleMethods.deferwrap1"),
        "fmt.(*pp).handleMethods.deferwrap1"
    );
    assert_eq!(
        text("runtime.gcBgMarkStartWorkers.gowrap1"),
        "runtime.gcBgMarkStartWorkers.gowrap1"
    );
    assert_eq!(
        text("sync/atomic.(*Uint64).Load-fm"),
        "sync/atomic.(*Uint64).Load-fm"
    );
    assert_eq!(
        text("internal/godebug.update-range1"),
        "internal/godebug.update-range1"
    );
}

#[test]
fn parses_inlined_components() {
    assert_eq!(
        text("example.com/x/lib.init.0.Counter.func1"),
        "example.com/x/lib.init.0.Counter.func1"
    );
    assert_eq!(
        text("runtime.(*sweepLocked).sweep.(*mheap).freeSpan.func3"),
        "runtime.(*sweepLocked).sweep.(*mheap).freeSpan.func3"
    );
    assert_eq!(text("runtime.memequal64·f"), "runtime.memequal64.f");
}

#[test]
fn parses_generics_and_shapes() {
    assert_eq!(
        text("cmp.Compare[go.shape.int]"),
        "cmp.Compare[go.shape.int]"
    );
    assert_eq!(
        text("internal/sync.(*HashTrieMap[go.shape.interface {},go.shape.interface {}]).Load"),
        "internal/sync.(*HashTrieMap[go.shape.interface {},go.shape.interface {}]).Load"
    );
    assert_eq!(text("cmp..dict.Compare[int]"), "cmp..dict.Compare[int]");
    assert_eq!(text("go.shape.int"), "go.shape.int");
}

#[test]
fn parses_compiler_symbols() {
    assert_eq!(text("main..inittask"), "main..inittask");
    assert_eq!(
        text("errors..interfaceSwitch.0"),
        "errors..interfaceSwitch.0"
    );
    assert_eq!(
        text("runtime.gclocals·+ISnmBhJ1y+frD3sAmLgtg=="),
        "runtime.gclocals.+ISnmBhJ1y+frD3sAmLgtg=="
    );
    assert_eq!(text("runtime.gcbits."), "runtime.gcbits.");
    assert_eq!(text("runtime.gcbits.*"), "runtime.gcbits.*");
    assert_eq!(
        text("example.com/x.(*Err).Error.arginfo1<1>"),
        "example.com/x.(*Err).Error.arginfo1<1>"
    );
}

#[test]
fn parses_type_descriptors() {
    let parsed = parse("type:.eq.main.Pair[string,int]").unwrap();
    let Name::TypeDescriptor(descriptor) = parsed.name() else {
        panic!("not a type descriptor");
    };
    assert_eq!(descriptor.kind(), TypeKind::Equal);
    assert_eq!(descriptor.payload(), "main.Pair[string,int]");

    let parsed = parse("type:*").unwrap();
    let Name::TypeDescriptor(descriptor) = parsed.name() else {
        panic!("not a type descriptor");
    };
    assert_eq!(descriptor.kind(), TypeKind::Pointer);
    assert_eq!(descriptor.payload(), "");

    let parsed = parse("type:.importpath.example.com/x.").unwrap();
    let Name::TypeDescriptor(descriptor) = parsed.name() else {
        panic!("not a type descriptor");
    };
    assert_eq!(descriptor.kind(), TypeKind::ImportPath);
    assert_eq!(descriptor.payload(), "example.com/x.");

    assert_eq!(
        text("type:.namedata.*func() int-"),
        "type:.namedata.*func() int-"
    );
}

#[test]
fn parses_equality_and_hash_closures() {
    for (name, kind, payload) in [
        ("type:.eqfunc96", TypeKind::EqualFunc, "96"),
        (
            "type:.eqfunc.go/types.importKey",
            TypeKind::EqualFunc,
            ".go/types.importKey",
        ),
        (
            "type:.hashfunc.go/types.importKey",
            TypeKind::HashFunc,
            ".go/types.importKey",
        ),
    ] {
        let parsed = parse(name).unwrap();
        let Name::TypeDescriptor(descriptor) = parsed.name() else {
            panic!("not a type descriptor: {name}");
        };
        assert_eq!(descriptor.kind(), kind);
        assert_eq!(descriptor.payload(), payload);
        assert_eq!(parsed.to_string(), name);
    }
}

#[test]
fn parses_static_temps_and_jump_tables() {
    let parsed = parse("example.com/x..stmp_31<1>").unwrap();
    assert_eq!(parsed.version().as_deref(), Some("1"));
    let Name::Symbol(symbol) = parsed.name() else {
        panic!("not a symbol");
    };
    let Local::StaticTemp { index } = symbol.local() else {
        panic!("not a static temp");
    };
    assert_eq!(index, b"31");
    assert_eq!(parsed.to_string(), "example.com/x..stmp_31<1>");

    let parsed = parse("fmt.(*pp).printArg.jump41<1>").unwrap();
    let Name::Symbol(symbol) = parsed.name() else {
        panic!("not a symbol");
    };
    let Local::Function(function) = symbol.local() else {
        panic!("not a function");
    };
    assert!(matches!(function.suffixes(), [Suffix::JumpTable(b"41")]));
    assert_eq!(parsed.to_string(), "fmt.(*pp).printArg.jump41<1>");
}

#[test]
fn parses_package_qualified_methods() {
    let name = "text/template.(*Template).text/template/parse.action.arginfo1<1>";
    assert_eq!(text(name), name);

    let parsed = parse("regexp.(*onePassInst).regexp/syntax.op").unwrap();
    let Name::Symbol(symbol) = parsed.name() else {
        panic!("not a symbol");
    };
    let Local::Function(function) = symbol.local() else {
        panic!("not a function");
    };
    assert_eq!(function.name(), "regexp/syntax.op");
    assert_eq!(function.receiver().unwrap().ty, b"onePassInst");
}

#[test]
fn splits_object_file_versions() {
    assert_eq!(parse("fmt.Println").unwrap().version(), None);
    let parsed = parse("fmt.Println<1>").unwrap();
    assert_eq!(parsed.version().as_deref(), Some("1"));
    assert_eq!(parsed.to_string(), "fmt.Println<1>");
    assert!(parse("fmt.Println<x>").is_err());
    assert!(parse("fmt.Println<>").is_err());
}

#[test]
fn parses_specials() {
    let parsed = parse("go:itab.*os.File,io.Writer").unwrap();
    let Name::Special(Special::Itab { concrete, iface }) = parsed.name() else {
        panic!("not an itab");
    };
    assert_eq!(concrete, b"*os.File");
    assert_eq!(iface, b"io.Writer");

    let parsed = parse("go:string.\"100%\"").unwrap();
    let Name::Special(Special::String { payload }) = parsed.name() else {
        panic!("not a string symbol");
    };
    assert_eq!(payload, b"\"100%\"");

    let parsed = parse("go:buildid").unwrap();
    let Name::Special(Special::Other { kind, payload }) = parsed.name() else {
        panic!("not a special");
    };
    assert_eq!(kind, b"buildid");
    assert_eq!(payload, b"");
}

#[test]
fn strips_abi_markers() {
    assert_eq!(
        parse("runtime.memmove.abi0").unwrap().abi(),
        Some(Abi::Abi0)
    );
    assert_eq!(
        parse("runtime.memmove.abiinternal").unwrap().abi(),
        Some(Abi::Internal)
    );
    assert_eq!(
        parse("runtime.memmove.abiwrap").unwrap().abi(),
        Some(Abi::Wrap)
    );
    assert_eq!(parse("runtime.memmove").unwrap().abi(), None);
    assert_eq!(text("runtime.memmove.abi0"), "runtime.memmove");
}

#[test]
fn parses_constants() {
    let parsed = parse("$f64.3ff0000000000000").unwrap();
    let Name::Constant(constant) = parsed.name() else {
        panic!("not a constant");
    };
    assert_eq!(constant.class(), ConstantClass::F64);
    assert_eq!(constant.value(), "3ff0000000000000");
    assert_eq!(text("$i64.0000000000000001"), "$i64.0000000000000001");
}

#[test]
fn rejects_malformed_names() {
    for name in [
        "",
        "type:",
        "type:.unknown.x",
        "go:",
        "go:itab.no-comma",
        "go:string.",
        "go.shape.",
        "$nope",
        "fmt.",
        "fmt.Println(",
        "fmt.Println[",
        "fmt.Println[go.shape.int",
        "fmt.Println-unknown",
        "main..nope",
        "main.(*T).",
        "main.()",
        "main.123",
    ] {
        assert!(parse(name).is_err(), "{name}");
    }
}

#[test]
fn input_budget_is_enforced() {
    let mut name = String::from("fmt.");
    name.push_str(&"a".repeat(MAX_INPUT_LEN));
    assert_eq!(parse(&name).unwrap_err(), ParseError::BudgetExceeded);
}

#[test]
fn depth_budget_is_enforced() {
    let mut name = String::from("pkg.F");
    name.push_str(&"[".repeat(MAX_DEPTH + 1));
    name.push_str(&"]".repeat(MAX_DEPTH + 1));
    assert_eq!(parse(&name).unwrap_err(), ParseError::RecursedTooDeep);
}

#[test]
fn suffix_lists_are_bounded_by_the_input_budget() {
    let name = alloc::format!("pkg.F{}", ".func1".repeat(1024));
    assert!(parse(&name).is_ok());
}

#[test]
fn rejects_unbalanced_groups() {
    assert!(parse("pkg.F[)]").is_err());
    assert!(parse("pkg.F[go.shape.struct { x int })").is_err());
}

#[test]
fn middle_dot_separators_are_accepted() {
    assert_eq!(text("runtime·memmove"), "runtime.memmove");
}

#[test]
fn package_paths_are_decoded() {
    let parsed = parse("gopkg.in/yaml%2ev3.F[go.shape.int]").unwrap();
    let Name::Symbol(symbol) = parsed.name() else {
        panic!("not a symbol");
    };
    let package = symbol.package().unwrap();
    assert_eq!(package.raw(), "gopkg.in/yaml%2ev3");
    assert_eq!(package.decoded().to_string(), "gopkg.in/yaml.v3");
    assert_eq!(parsed.to_string(), "gopkg.in/yaml.v3.F[go.shape.int]");
}

#[test]
fn byte_path_matches_the_text_path() {
    for name in [
        "fmt.Println",
        "runtime.(*mLockProfile).captureStack",
        "example.com/demangle/corpus/pkg%2ev1.Make",
        "type:.eq.main.Pair[string,int]",
        "go:itab.*os.File,io.Writer",
        "go:string.\"100%\"",
        "runtime.gclocals·+ISnmBhJ1y+frD3sAmLgtg==",
        "text/template.(*Template).text/template/parse.action.arginfo1<1>",
        "$f64.3ff0000000000000",
    ] {
        assert_eq!(
            parse_with(name.as_bytes()).unwrap().to_string(),
            parse(name).unwrap().to_string(),
            "{name}"
        );
    }
}

#[test]
fn byte_path_replays_the_corpus() {
    for path in [
        "refs/go/corpus/go1.24.4-binary.txt",
        "refs/go/corpus/go1.24.4-object.txt",
        "refs/go/corpus/go1.24.4-archive.txt",
    ] {
        let path = std::format!("{}/{path}", std::env!("CARGO_MANIFEST_DIR"));
        let Ok(data) = std::fs::read_to_string(&path) else {
            continue;
        };
        for line in data.lines() {
            let name = line.trim_end();
            if name.is_empty() {
                continue;
            }
            let Ok(text) = parse(name) else {
                continue;
            };
            let bytes = parse_with(name.as_bytes()).unwrap_or_else(|error| {
                panic!("{name}: byte path failed: {error}");
            });
            assert_eq!(bytes.to_string(), text.to_string(), "{name}");
        }
    }
}

#[test]
fn invalid_utf8_text_is_escaped() {
    let ast = parse_with(b"type:.eq.main.F\xffoo").unwrap();
    assert_eq!(ast.to_string(), "type:.eq.main.F\\xffoo");
    // A decoded `%XX` escape that is not UTF-8 is escaped, not re-encoded.
    let ast = parse_with(b"type:.eq.example.com/x%2ff%ff.T").unwrap();
    assert_eq!(ast.to_string(), "type:.eq.example.com/x/f\\xff.T");
    // Opaque payloads keep their own quoting and backslashes.
    let ast = parse_with(b"go:string.\"a\\b\"").unwrap();
    assert_eq!(ast.to_string(), "go:string.\"a\\b\"");
}

#[test]
fn valid_unicode_text_stays_verbatim() {
    let ast = parse_with("runtime.memequal64·f".as_bytes()).unwrap();
    assert_eq!(ast.to_string(), "runtime.memequal64.f");
    let ast = parse_with("type:.eq.pkg.Föö".as_bytes()).unwrap();
    assert_eq!(ast.to_string(), "type:.eq.pkg.Föö");
}

#[test]
fn suffix_lists_are_reported() {
    let parsed = parse("pkg.F.func1.func2.func3").unwrap();
    let Name::Symbol(symbol) = parsed.name() else {
        panic!("not a symbol");
    };
    let Local::Function(function) = symbol.local() else {
        panic!("not a function");
    };
    assert_eq!(function.suffixes().len(), 3);
    assert!(matches!(function.suffixes()[0], Suffix::Closure(b"1")));
}
