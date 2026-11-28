use alloc::string::ToString;

use super::*;
use crate::kotlin_native::parse_bytes_with_budget;
use crate::kotlin_native::{DEFAULT_BUDGET, MAX_DEPTH, parse};

/// Parse with the compiled-in budget, the way the public entry point does.
fn parse_with(symbol: &[u8]) -> Result<Ast<'_>, ParseError> {
    parse_bytes(symbol, ParseLimits::from_effective(DEFAULT_BUDGET))
}

#[test]
fn parses_function_examples() {
    for (symbol, expected) in [
        (
            "kfun:com.example.foo#bar(kotlin.Int){}",
            "com.example.foo.bar(kotlin.Int)",
        ),
        (
            "kfun:com.example.Box#<init>(kotlin.Int){}",
            "com.example.Box.<init>(kotlin.Int)",
        ),
        ("kfun:com.example.Box#get(){}", "com.example.Box.get()"),
        ("kfun:com.example#topLevel(){}", "com.example.topLevel()"),
    ] {
        assert_eq!(
            parse_bytes_with_budget(symbol.as_bytes(), DEFAULT_BUDGET)
                .unwrap()
                .to_string(),
            expected,
            "{symbol}"
        );
    }
}

#[test]
fn parses_private_dot_separator() {
    assert_eq!(
        parse_bytes_with_budget(b"kfun:com.example#outer$local#internal", DEFAULT_BUDGET)
            .unwrap()
            .to_string(),
        "com.example.outer$local"
    );
    assert_eq!(
        parse_bytes_with_budget(
            b"kfun:com.example.useAll$lambda$1.invoke#internal",
            DEFAULT_BUDGET
        )
        .unwrap()
        .to_string(),
        "com.example.useAll$lambda$1.invoke"
    );
    assert_eq!(
        parse(
            b"kfun:com.example.useAll$lambda$1.$<bridge-UNU>invoke(kotlin.Int){}kotlin.Int#internal",
            crate::Budget::default(),
        )
        .unwrap()
        .to_string(),
        "com.example.useAll$lambda$1.$<bridge-UNU>invoke(kotlin.Int): kotlin.Int"
    );
}

#[test]
fn parses_extension_receiver() {
    let ast = parse_bytes_with_budget(
        b"kfun:com.example.kdemo#shout__at__kotlin.String(){}kotlin.String",
        DEFAULT_BUDGET,
    )
    .unwrap();
    assert_eq!(
        ast.to_string(),
        "com.example.kdemo.shout@kotlin.String(): kotlin.String"
    );
}

#[test]
fn parses_static_and_suspend_markers() {
    let ast = parse_bytes_with_budget(
        b"kfun:com.example.Singleton#<get-$instance>#static(){}com.example.Singleton",
        DEFAULT_BUDGET,
    )
    .unwrap();
    assert_eq!(
        ast.to_string(),
        "static com.example.Singleton.<get-$instance>(): com.example.Singleton"
    );
    let ast = parse(
        b"kfun:com.example#suspendWork#suspend(kotlin.String;kotlin.coroutines.Continuation<kotlin.String>){}kotlin.Any",
        crate::Budget::default(),
    )
    .unwrap();
    assert_eq!(
        ast.to_string(),
        "suspend com.example.suspendWork(kotlin.String, kotlin.coroutines.Continuation<kotlin.String>): kotlin.Any"
    );
}

#[test]
fn parses_type_parameters_and_bounds() {
    let ast = parse_bytes_with_budget(
        "kfun:com.example#maxOf(0:0;0:0){0§<kotlin.Comparable<0:0>>}0:0".as_bytes(),
        DEFAULT_BUDGET,
    )
    .unwrap();
    assert_eq!(
        ast.to_string(),
        "com.example.maxOf(T0:0, T0:0)<T0: kotlin.Comparable<T0:0>>: T0:0"
    );
}

#[test]
fn parses_variance_nullable_star_vararg() {
    let ast = parse(
        b"kfun:com.example#projections(kotlin.collections.List<out|kotlin.Number>;kotlin.collections.MutableList<in|kotlin.Number>){}kotlin.Int",
        crate::Budget::default(),
    )
    .unwrap();
    assert_eq!(
        ast.to_string(),
        "com.example.projections(kotlin.collections.List<out kotlin.Number>, kotlin.collections.MutableList<in kotlin.Number>): kotlin.Int"
    );
    assert_eq!(
        parse_bytes_with_budget(
            b"kfun:com.example#stars(kotlin.collections.List<*>){}kotlin.Int",
            DEFAULT_BUDGET
        )
        .unwrap()
        .to_string(),
        "com.example.stars(kotlin.collections.List<*>): kotlin.Int"
    );
    assert_eq!(
        parse_bytes_with_budget(
            b"kfun:com.example#varargs(kotlin.IntArray...){}kotlin.Int",
            DEFAULT_BUDGET
        )
        .unwrap()
        .to_string(),
        "com.example.varargs(vararg kotlin.IntArray): kotlin.Int"
    );
    assert_eq!(
        parse_bytes_with_budget(
            b"kfun:com.example#nullable(kotlin.String?){}kotlin.String?",
            DEFAULT_BUDGET
        )
        .unwrap()
        .to_string(),
        "com.example.nullable(kotlin.String?): kotlin.String?"
    );
    // A class name starting with `in` is not a bare `in` variance marker.
    assert_eq!(
        parse_bytes_with_budget(b"ktype:kotlin.Array<input>", DEFAULT_BUDGET)
            .unwrap()
            .to_string(),
        "kotlin.Array<input>"
    );
}

#[test]
fn parses_named_prefixes() {
    for (symbol, expected) in [
        (
            "ktype:com.example.Box<kotlin.Int>",
            "com.example.Box<kotlin.Int>",
        ),
        ("kclass:com.example.Box", "com.example.Box"),
        ("kclass:com.example.Box#internal", "com.example.Box"),
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
        assert_eq!(
            parse_bytes_with_budget(symbol.as_bytes(), DEFAULT_BUDGET)
                .unwrap()
                .to_string(),
            expected,
            "{symbol}"
        );
    }
}

#[test]
fn accepts_platform_underscores() {
    for symbol in [
        "_kfun:com.example#main(){}",
        "__kfun:com.example#main(){}",
        "_kclass:com.example.Box",
    ] {
        assert!(is_kotlin_native(symbol.as_bytes()), "{symbol}");
        assert!(
            parse_bytes_with_budget(symbol.as_bytes(), DEFAULT_BUDGET).is_ok(),
            "{symbol}"
        );
    }
}

#[test]
fn accepts_platform_suffix() {
    let ast = parse_bytes_with_budget(b"kfun:com.example#foo(){}%objc", DEFAULT_BUDGET).unwrap();
    assert_eq!(ast.to_string(), "com.example.foo() %objc");
}

#[test]
fn rejects_unknown_markers() {
    for symbol in [
        "kfun:com.example#foo#nope(){}",
        "kfun:com.example#foo(kotlin.Int){}#nope",
        "kfun:com.example#foo#staticx(){}",
        "kfun:com.example",
        "kfun:com.example.foo",
        "kfun:com.example.foo#static(){}",
    ] {
        assert!(
            parse_bytes_with_budget(symbol.as_bytes(), DEFAULT_BUDGET).is_err(),
            "{symbol}"
        );
    }
}

#[test]
fn rejects_malformed_bodies() {
    for symbol in [
        "kfun:",
        "kfun:#foo(){}",
        "kfun:com.example#",
        "kfun:com.example#foo(",
        "kfun:com.example#foo(kotlin.Int",
        "kfun:com.example#foo(){}kotlin.Int<",
        "kfun:com.example#foo(kotlin.Int;",
        "kfun:com.example#foo(kotlin.Int;){}",
        "kfun:com.example#foo{0§<kotlin.Int>",
        "kfun:com.example#foo{0§<kotlin.Int}",
        "kfun:com.example#foo(){}kotlin.Array<",
        "kfun:com.example#foo(){}kotlin.Array<>",
        "kfun:com.example#foo(){}kotlin.Array<out|>",
        "kfun:com.example#foo(){}kotlin.Array<out>",
        "kfun:com.example#foo(){}kotlin.Array<in>",
        "ktype:",
        "kclass:",
        "kclass:com.example.Box#",
        "kproperty:com.example#a#b",
        "kfield:com.example.",
        "kpackage:.com",
    ] {
        assert!(
            parse_bytes_with_budget(symbol.as_bytes(), DEFAULT_BUDGET).is_err(),
            "{symbol}"
        );
    }
}

#[test]
fn rejects_unknown_prefixes() {
    for symbol in ["kfoo:bar", "kintf:com.example", "kifacetable:x", "kclass"] {
        assert!(!is_kotlin_native(symbol.as_bytes()), "{symbol}");
    }
}

#[test]
fn input_is_bounded() {
    let mut symbol = alloc::string::String::from("kfun:");
    symbol.push_str(&"a".repeat(crate::kotlin_native::MAX_INPUT_LEN));
    assert!(
        matches!(parse_bytes_with_budget(symbol.as_bytes(), DEFAULT_BUDGET), Err(e) if e.budget_kind().is_some())
    );
}

#[test]
fn depth_is_bounded() {
    let mut symbol = alloc::string::String::from("ktype:kotlin.Array<");
    symbol.push_str(&"kotlin.Array<".repeat(MAX_DEPTH + 1));
    symbol.push('*');
    symbol.push_str(&">".repeat(MAX_DEPTH + 1));
    assert!(matches!(
        parse_bytes_with_budget(symbol.as_bytes(), DEFAULT_BUDGET),
        Err(e) if e.budget_kind() == Some(crate::BudgetKind::ParseDepth)
    ));
}

#[test]
fn byte_path_matches_the_text_path() {
    for symbol in [
        "kfun:com.example.foo#bar(kotlin.Int){}",
        "kfun:com.example#shout__at__kotlin.String(){}kotlin.String",
        "ktype:kotlin.collections.Map<kotlin.String,kotlin.Array<out|kotlin.Number?>>",
        "kclass:com.example.Box#internal",
        "kproperty:com.example.Box#value",
        "kfun:com.example#foo(){}%objc",
        "_kfun:com.example#main(){}",
    ] {
        assert_eq!(
            parse_with(symbol.as_bytes()).unwrap().to_string(),
            parse_bytes_with_budget(symbol.as_bytes(), DEFAULT_BUDGET)
                .unwrap()
                .to_string(),
            "{symbol}"
        );
    }
}

#[test]
fn byte_path_replays_the_corpus() {
    for path in [
        "refs/kotlin-native/corpus/kotlin-2.4.21-linux-x86_64.txt",
        "refs/kotlin-native/corpus/hand-authored.txt",
    ] {
        let path = std::format!("{}/{path}", std::env!("CARGO_MANIFEST_DIR"));
        let Ok(data) = std::fs::read_to_string(&path) else {
            continue;
        };
        for line in data.lines() {
            let symbol = line.trim_end();
            if symbol.is_empty() {
                continue;
            }
            let Ok(text) = parse_bytes_with_budget(symbol.as_bytes(), DEFAULT_BUDGET) else {
                continue;
            };
            let bytes = parse_with(symbol.as_bytes()).unwrap_or_else(|error| {
                panic!("{symbol}: byte path failed: {error}");
            });
            assert_eq!(bytes.to_string(), text.to_string(), "{symbol}");
        }
    }
}

#[test]
fn invalid_utf8_text_is_escaped() {
    assert_eq!(
        parse_with(b"kclass:com.example.F\xffoo")
            .unwrap()
            .to_string(),
        "com.example.F\\xffoo"
    );
    assert_eq!(
        parse_with(b"kfun:com.example#f\xffoo(){}")
            .unwrap()
            .to_string(),
        "com.example.f\\xffoo()"
    );
}

#[test]
fn valid_unicode_text_stays_verbatim() {
    assert_eq!(
        parse_with("kclass:com.example.Föö".as_bytes())
            .unwrap()
            .to_string(),
        "com.example.Föö"
    );
}

#[test]
fn truncated_names_never_panic() {
    for symbol in [
        "kfun:com.example#foo(kotlin.Int;kotlin.String){0§<kotlin.Any>}kotlin.Int",
        "kfun:com.example#shout__at__kotlin.String(){}kotlin.String",
        "ktype:kotlin.collections.Map<kotlin.String,kotlin.Array<out|kotlin.Number?>>",
        "kclass:com.example.Box#internal",
        "kproperty:com.example.Box#value",
    ] {
        for end in 0..=symbol.len() {
            if let Some(prefix) = symbol.get(..end) {
                let _ = parse_bytes_with_budget(prefix.as_bytes(), DEFAULT_BUDGET);
                let _ = is_kotlin_native(prefix.as_bytes());
            }
        }
    }
}
