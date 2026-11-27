use alloc::string::ToString;

use crate::go::node::{ConstantClass, TypeKind};
use crate::go::parse;

fn text(name: &str) -> alloc::string::String {
    parse(name).unwrap().to_string()
}

#[test]
fn prints_packages_and_receivers() {
    assert_eq!(text("fmt.Println"), "fmt.Println");
    assert_eq!(text("fmt.(*pp).doPrintln"), "fmt.(*pp).doPrintln");
    assert_eq!(text("fmt.(pp).doPrintln"), "fmt.(pp).doPrintln");
}

#[test]
fn prints_escapes_decoded() {
    assert_eq!(
        text("example.com/x/pkg%2ev1.Make"),
        "example.com/x/pkg.v1.Make"
    );
    assert_eq!(
        text("go:itab.*example.com/x/pkg%2ev1.T,error"),
        "go:itab.*example.com/x/pkg.v1.T,error"
    );
}

#[test]
fn prints_opaque_payloads_verbatim() {
    assert_eq!(text("go:string.\"100%\""), "go:string.\"100%\"");
    assert_eq!(
        text("type:.namedata.F.json:\"%2e\""),
        "type:.namedata.F.json:\"%2e\""
    );
}

#[test]
fn prints_abi_markers_stripped() {
    assert_eq!(text("runtime.memmove.abi0"), "runtime.memmove");
    assert_eq!(text("runtime.memmove.abiinternal"), "runtime.memmove");
    assert_eq!(text("runtime.memmove.abiwrap"), "runtime.memmove");
}

#[test]
fn prints_every_name_kind() {
    for name in [
        "fmt.Println",
        "type:.eq.main.T",
        "type:*[]struct { key string; elem func() int }",
        "go:itab.*os.File,io.Writer",
        "go:buildid",
        "go:string.\"hello\"",
        "go.shape.struct { x int }",
        "$f64.3ff0000000000000",
        "main..inittask",
        "cmp..dict.Compare[int]",
        "errors..interfaceSwitch.0",
        "errors..typeAssert.1",
        "runtime..gobytes.2",
        "runtime..stmp_7<1>",
        "fmt.(*pp).printArg.jump41<1>",
        "type:.eqfunc96",
        "type:.hashfunc.go/types.importKey",
        "text/template.(*Template).text/template/parse.action.arginfo1<1>",
        "runtime.gcbits.*",
    ] {
        assert_eq!(text(name), name, "{name}");
    }
    // `·` is normalised to `.`, like the linker's ELF symbol table does.
    assert_eq!(
        text("runtime.gclocals·+ISnmBhJ1y+frD3sAmLgtg=="),
        "runtime.gclocals.+ISnmBhJ1y+frD3sAmLgtg=="
    );
}

#[test]
fn prints_constants() {
    for name in [
        "$f32.3f800000",
        "$f64.3ff0000000000000",
        "$i32.00000001",
        "$i64.0000000000000001",
        "$i128.00000000000000000000000000000001",
    ] {
        assert_eq!(text(name), name);
    }
}

#[test]
fn constant_classes_are_reported() {
    assert_eq!(ConstantClass::F32.prefix(), "f32.");
    assert_eq!(ConstantClass::F64.prefix(), "f64.");
    assert_eq!(ConstantClass::I32.prefix(), "i32.");
    assert_eq!(ConstantClass::I64.prefix(), "i64.");
    assert_eq!(ConstantClass::I128.prefix(), "i128.");
    assert!(TypeKind::NameData.is_opaque());
}
