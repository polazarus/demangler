use alloc::string::ToString;

use super::*;

#[test]
fn detects_go_names() {
    for name in [
        "fmt.Println",
        "main.main",
        "runtime.memmove.abi0",
        "runtime.(*mLockProfile).captureStack",
        "example.com/demangle/corpus/pkg%2ev1.Make",
        "type:.eq.main.Pair[string,int]",
        "type:.eqfunc96",
        "type:.hashfunc.go/types.importKey",
        "type:*",
        "go:itab.*os.File,io.Writer",
        "go:string.\"hello\"",
        "go.shape.int",
        "go.shape.struct { x int }",
        "cmp.Compare[go.shape.struct { x int }]",
        "$f64.3ff0000000000000",
        "internal/sync.(*HashTrieMap[go.shape.interface {},go.shape.interface {}]).Load",
        "internal/sync..dict.HashTrieMap[interface {},interface {}]",
        "gclocals·+ISnmBhJ1y+frD3sAmLgtg==",
        "runtime..stmp_31<1>",
        "fmt.(*pp).printArg.jump41<1>",
        "regexp.(*onePassInst).regexp/syntax.op",
        "runtime.(*sweepLocked).sweep.(*mheap).freeSpan.func3",
    ] {
        assert!(is_go(name), "{name}");
    }
}

#[test]
fn rejects_non_go_names() {
    for name in [
        "",
        "main",
        "memcpy",
        "_start",
        "12345",
        "_ZN3Foo3barEv",
        "_RNvC4test4main",
        "?foo@@YAXXZ",
        "$s4main3fooSi_yF",
        "$nope",
        "foo.bar baz",
        "foo.bar(baz)",
        "_ZN3fooE.llvm.9D1C9369",
        "foo.llvm.9D1C9369",
    ] {
        assert!(!is_go(name), "{name}");
    }
}

#[test]
fn detection_keeps_unicode_and_escapes_invalid_bytes() {
    // A valid Unicode identifier is a Go name, not a byte-error.
    assert!(is_go("pkg.Föö"));
    assert!(is_go_bytes("pkg.Föö".as_bytes()));
    assert!(is_go("runtime.memequal64·f"));
    // Bytes that are not UTF-8 do not make a Go-shaped name stop being Go.
    assert!(is_go_bytes(b"pkg.F\xffoo"));
    assert!(is_go_bytes(b"type:.eq.pkg.F\xffoo"));
    assert!(!is_go_bytes(b"main"));
    assert!(!is_go_bytes(b"foo.bar baz"));
}

#[test]
fn never_panics_on_arbitrary_input() {
    const CHARSET: &[u8] = b"abcXYZ019._()[]{},:*/%+-<>;= \"\\\xC2\xB7";
    let mut state = 0x1234_5678_u64;
    let mut next = || {
        state = state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        (state >> 33) as usize
    };
    for _ in 0..20_000 {
        let len = next() % 48;
        let mut bytes = alloc::vec::Vec::with_capacity(len);
        for _ in 0..len {
            bytes.push(CHARSET[next() % CHARSET.len()]);
        }
        if let Ok(text) = core::str::from_utf8(&bytes) {
            let _ = parse(text);
            let _ = is_go(text);
        }
    }
}

#[test]
fn never_panics_on_truncated_names() {
    for name in [
        "runtime.(*HashTrieMap[go.shape.struct { x int }]).Load.func1",
        "runtime.memequal64·f",
        "go:itab.*os.File,io.Writer",
        "type:.namedata.*func() int-",
        "gclocals·+ISnmBhJ1y+frD3sAmLgtg==",
        "example.com/x..stmp_31<1>",
        "fmt.(*pp).printArg.jump41<1>",
        "type:.eqfunc.go/types.importKey",
        "text/template.(*Template).text/template/parse.action.arginfo1<1>",
    ] {
        for end in 0..=name.len() {
            if let Some(prefix) = name.get(..end) {
                let _ = parse(prefix);
            }
        }
    }
}

#[test]
fn parse_is_display() {
    assert_eq!(parse("fmt.Println").unwrap().to_string(), "fmt.Println");
}

#[test]
fn errors_are_displayed() {
    assert_eq!(ParseError::Invalid.to_string(), "invalid syntax");
    assert_eq!(
        ParseError::RecursedTooDeep.to_string(),
        "recursion limit reached"
    );
    assert_eq!(ParseError::BudgetExceeded.to_string(), "size limit reached");
}

#[test]
fn ast_exposes_structure() {
    let ast = parse("runtime.memmove.abi0").unwrap();
    assert_eq!(ast.abi(), Some(Abi::Abi0));
    let Name::Symbol(symbol) = ast.name() else {
        panic!("not a symbol");
    };
    assert_eq!(symbol.package().unwrap().raw(), "runtime");
    assert!(matches!(symbol.local(), Local::Function(_)));
}

#[test]
fn local_variants_are_exposed() {
    assert!(matches!(
        parse("main..inittask").unwrap().name(),
        Name::Symbol(Symbol {
            local: Local::InitTask,
            ..
        })
    ));
    assert!(matches!(
        parse("go.shape.int").unwrap().name(),
        Name::Shape(b"int")
    ));
    assert!(matches!(
        parse("main..stmp_3").unwrap().name(),
        Name::Symbol(Symbol {
            local: Local::StaticTemp { index: b"3" },
            ..
        })
    ));
}
