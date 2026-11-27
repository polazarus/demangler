use alloc::string::ToString;

use crate::kotlin_native::parse;

fn text(symbol: &str) -> alloc::string::String {
    parse(symbol.as_bytes()).unwrap().to_string()
}

#[test]
fn prints_plan_examples() {
    assert_eq!(
        text("kfun:com.example.foo#bar(kotlin.Int){}"),
        "com.example.foo.bar(kotlin.Int)"
    );
    assert_eq!(
        text("kfun:com.example.Box#<init>(kotlin.Int){}"),
        "com.example.Box.<init>(kotlin.Int)"
    );
    assert_eq!(
        text("kfun:com.example.Box#get(){}"),
        "com.example.Box.get()"
    );
    assert_eq!(
        text("kfun:com.example#topLevel(){}"),
        "com.example.topLevel()"
    );
    assert_eq!(
        text("ktype:com.example.Box<kotlin.Int>"),
        "com.example.Box<kotlin.Int>"
    );
}

#[test]
fn prints_every_symbol_family() {
    for (symbol, expected) in [
        ("kclass:com.example.Box", "com.example.Box"),
        ("kproperty:com.example.Box#value", "com.example.Box.value"),
        ("kobj:com.example.Singleton", "com.example.Singleton"),
        ("kfield:com.example.Box.value", "com.example.Box.value"),
        ("ktypealias:com.example.IntList", "com.example.IntList"),
        ("kpackage:com.example", "com.example"),
        ("knamespace:com.example", "com.example"),
        (
            "kvar:com.example.Singleton.$instance",
            "com.example.Singleton.$instance",
        ),
    ] {
        assert_eq!(text(symbol), expected, "{symbol}");
    }
}

#[test]
fn prints_flexible_and_error_types() {
    assert_eq!(
        text("ktype:[kotlin.String~kotlin.String?]"),
        "kotlin.String..kotlin.String?"
    );
    assert_eq!(text("ktype:<dynamic>"), "dynamic");
    assert_eq!(text("ktype:<ERROR CLASS>"), "<ERROR CLASS>");
}

#[test]
fn prints_multiple_bounds() {
    assert_eq!(
        text("kfun:com.example#foo(0:0){0§<kotlin.Any&kotlin.Comparable<0:0>>}0:0"),
        "com.example.foo(T0:0)<T0: kotlin.Any & kotlin.Comparable<T0:0>>: T0:0"
    );
}
