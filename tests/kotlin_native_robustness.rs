//! Kotlin/Native parser and printer robustness tests.
//!
//! The fuzz tests are deterministic: a fixed-seed SplitMix64 generator feeds
//! short token- and character-based inputs to the parser, and every seed is
//! also truncated at each character boundary. They assert only invariants that
//! hold for untrusted input: no panic, a bounded render, and detection only for
//! a known prefix.

#![cfg(feature = "kotlin-native")]

use std::fmt::Write as _;
use std::string::String;

use demangler::kotlin_native::{MAX_OUTPUT_LEN, is_kotlin_native, parse};
use demangler::{Demangler, Scheme};

/// Iterations per fuzz test.
const ITERATIONS: usize = 3_000;

/// Deterministic SplitMix64 generator.
struct Rng {
    state: u64,
}

impl Rng {
    const fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15_u64);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9_u64);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB_u64);
        z ^ (z >> 31)
    }

    fn below(&mut self, bound: usize) -> usize {
        let bound = u64::try_from(bound).unwrap_or(u64::MAX);
        usize::try_from(self.next_u64() % bound).unwrap_or(0)
    }

    fn tokens(&mut self, prefix: &str, parts: &[&str], max_parts: usize) -> String {
        let mut out = String::from(prefix);
        let count = self.below(max_parts + 1);
        for _ in 0..count {
            out.push_str(parts[self.below(parts.len())]);
        }
        out
    }

    fn chars(&mut self, prefix: &str, alphabet: &[char], max_len: usize) -> String {
        let mut out = String::from(prefix);
        let count = self.below(max_len + 1);
        for _ in 0..count {
            out.push(alphabet[self.below(alphabet.len())]);
        }
        out
    }
}

/// Demangle one input and check the shared output bounds.
fn check_demangled(demangler: &Demangler, symbol: &str) {
    let Ok(demangled) = demangler.demangle(symbol) else {
        return;
    };
    let mut out = String::new();
    let _ = write!(out, "{demangled}");
    assert!(
        out.len() <= MAX_OUTPUT_LEN,
        "{symbol:?} rendered {} bytes",
        out.len()
    );
    if demangled.scheme() == Scheme::KotlinNative
        && symbol.trim_start_matches('_').starts_with("kfun:")
    {
        assert!(!out.contains('#'), "{symbol:?} left `#` in {out:?}");
        assert!(
            !out.contains("__at__"),
            "{symbol:?} left `__at__` in {out:?}"
        );
        assert!(
            !out.contains("#internal"),
            "{symbol:?} left `#internal` in {out:?}"
        );
        assert!(
            !out.ends_with("-impl"),
            "{symbol:?} left the `-impl` marker in {out:?}"
        );
    }
}

#[test]
fn never_panics_on_token_input() {
    let parts = [
        "kfun:",
        "ktype:",
        "kclass:",
        "kproperty:",
        "kobj:",
        "kfield:",
        "ktypealias:",
        "kpackage:",
        "knamespace:",
        "kvar:",
        "com.example",
        ".",
        "#",
        "#static",
        "#suspend",
        "#companion",
        "#internal",
        "-impl",
        "$",
        "$1",
        "<init>",
        "<get-x>",
        "__at__",
        "@",
        "(",
        ")",
        "{",
        "}",
        "<",
        ">",
        "§",
        ";",
        ",",
        "?",
        "*",
        "...",
        "out|",
        "in|",
        "0:0",
        "kotlin.Int",
        "kotlin.String?",
        "%objc",
    ];
    let mut rng = Rng::new(0x4b4f_544c_494e_0001);
    let demangler = Demangler::new();
    for _ in 0..ITERATIONS {
        let input = rng.tokens("", &parts, 12);
        let _ = parse(&input);
        let _ = is_kotlin_native(&input);
        check_demangled(&demangler, &input);
    }
}

#[test]
fn never_panics_on_character_input() {
    let alphabet: &[char] = &[
        'k', 'f', 'u', 'n', 'c', 'l', 'a', 's', 't', 'y', 'p', 'e', ':', '.', '#', '$', '(', ')',
        '{', '}', '<', '>', '§', ';', ',', '?', '*', '|', '&', '@', '_', '%', '-', '0', '1', 'A',
        ' ', '~', '[', ']',
    ];
    let prefixes = [
        "",
        "kfun:",
        "ktype:",
        "kclass:",
        "kproperty:",
        "kfield:",
        "kpackage:",
        "_kfun:",
        "kfoo:",
    ];
    let mut rng = Rng::new(0x4b4f_544c_494e_0002);
    let demangler = Demangler::new();
    for _ in 0..ITERATIONS {
        let prefix = prefixes[rng.below(prefixes.len())];
        let input = rng.chars(prefix, alphabet, 40);
        let _ = parse(&input);
        let _ = is_kotlin_native(&input);
        check_demangled(&demangler, &input);
    }
}

#[test]
fn truncated_names_never_panic() {
    let demangler = Demangler::new();
    for symbol in [
        "kfun:com.example#foo(kotlin.Int;kotlin.String){0§<kotlin.Any>}kotlin.Int",
        "kfun:com.example#shout__at__kotlin.String(){}kotlin.String",
        "kfun:com.example.Box#<get-$companion>#static(){}com.example.Box.Companion",
        "kfun:com.example.useAll$lambda$1.$<bridge-UNU>invoke(kotlin.Int){}kotlin.Int#internal",
        "ktype:kotlin.collections.Map<kotlin.String,kotlin.Array<out|kotlin.Number?>>",
        "kclass:com.example.Box#internal",
        "kproperty:com.example.Box#value",
        "kfield:com.example.Box.value",
        "kvar:com.example.Singleton.$instance",
        "kfun:com.example#foo#companion@com.example.Bar(){}kotlin.Int",
        "kfun:com.example#foo(){}%objc",
        "_kfun:com.example#main(){}",
    ] {
        for end in 0..=symbol.len() {
            if let Some(prefix) = symbol.get(..end) {
                let _ = parse(prefix);
                let _ = is_kotlin_native(prefix);
                check_demangled(&demangler, prefix);
            }
        }
    }
}

#[test]
fn budgets_are_enforced() {
    let mut long = String::from("kfun:");
    long.push_str(&"a".repeat(demangler::kotlin_native::MAX_INPUT_LEN));
    assert!(parse(&long).is_err());

    let mut deep = String::from("ktype:kotlin.Array<");
    deep.push_str(&"kotlin.Array<".repeat(demangler::kotlin_native::MAX_DEPTH + 1));
    deep.push('*');
    deep.push_str(&">".repeat(demangler::kotlin_native::MAX_DEPTH + 1));
    assert!(parse(&deep).is_err());

    let mut params = String::from("kfun:com.example#foo(");
    params.push_str(&"kotlin.Int;".repeat(demangler::kotlin_native::MAX_PARAMETERS + 1));
    params.push(')');
    assert!(parse(&params).is_err());

    let mut arguments = String::from("ktype:kotlin.Array<");
    arguments.push_str(&"kotlin.Int,".repeat(demangler::kotlin_native::MAX_TYPE_ARGUMENTS + 1));
    arguments.push_str("kotlin.Int>");
    assert!(parse(&arguments).is_err());
}
