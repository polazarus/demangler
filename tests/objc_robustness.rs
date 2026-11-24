//! Objective-C parser and printer robustness tests.
//!
//! The fuzz tests are deterministic: a fixed-seed SplitMix64 generator feeds
//! short token- and character-based inputs to the parser, and every seed is
//! also truncated at each character boundary. They assert only invariants that
//! hold for untrusted input: no panic, a bounded render, and no successful
//! Objective-C parse for a name outside the recognised families.

#![cfg(feature = "objc")]

use std::fmt::Write as _;

use demangler::{Demangler, Scheme};

/// Every successful Objective-C render is bounded by the input plus the
/// canonical prefix overhead.
const MAX_OVERHEAD: usize = 64;

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

/// Check one input through the parser and the demangler.
fn check(symbol: &str) {
    let demangler = Demangler::new();
    if let Ok(ast) = demangler::objc::parse(symbol) {
        let mut out = String::new();
        let _ = write!(out, "{ast}");
        assert!(
            out.len() <= symbol.len() + MAX_OVERHEAD,
            "{symbol:?} rendered {} bytes",
            out.len()
        );
    }
    if let Ok(demangled) = demangler.demangle(symbol)
        && demangled.scheme() == Scheme::ObjC
    {
        let mut out = String::new();
        let _ = write!(out, "{demangled}");
        assert!(
            out.len() <= symbol.len() + MAX_OVERHEAD,
            "{symbol:?} rendered {} bytes",
            out.len()
        );
    }
}

const SEEDS: &[&str] = &[
    "_OBJC_CLASS_$_Foo",
    "__OBJC_$_INSTANCE_METHODS_Foo",
    "__OBJC_$_CATEGORY_Foo_$_Bar",
    "__OBJC_PROTOCOL_$_NSCopying",
    "_OBJC_IVAR_$_Foo._ivar",
    "__objc_class_name_Foo",
    "__objc_ivar_offset_Foo._ivar",
    "-[Foo bar:baz:]",
    "+[Foo(Cat) catMethod]",
    "___block_descriptor_40_e8_32r_e5_v8?0l",
    "_Z3foov",
];

#[test]
fn truncations_never_panic() {
    for &seed in SEEDS {
        for (end, _) in seed.char_indices() {
            check(&seed[..end]);
        }
        check(seed);
    }
}

#[test]
fn fuzz_parser_never_panics() {
    const PREFIXES: &[&str] = &[
        "",
        "_OBJC_",
        "__OBJC_",
        "OBJC_",
        "__objc_",
        "-[",
        "+[",
        "__block_descriptor_",
        "___block_descriptor_",
    ];
    const TOKENS: &[&str] = &[
        "CLASS_$_",
        "METACLASS_$_",
        "IVAR_$_",
        "$_",
        "CLASS_",
        "METACLASS_",
        "PROTOCOL_",
        "CATEGORY_",
        "PROP_LIST_",
        "PROTOCOL_REFS_",
        "class_name_",
        "ivar_offset_",
        "_$_",
        "Foo",
        "Bar",
        "._x",
        "0",
        "12",
        "bar:",
        "baz:",
        " ",
        "[",
        "]",
        "(",
        ")",
    ];
    const ALPHABET: &[char] = &[
        'a', 'b', 'c', 'f', 'o', 'A', 'B', 'C', 'F', 'O', '0', '1', '2', '9', '_', '$', '.', '-',
        '+', ':', '?', ' ', '[', ']', '(', ')',
    ];
    let mut rng = Rng::new(0x0B7C_0B7C_u64);
    for _ in 0..ITERATIONS {
        let prefix = PREFIXES[rng.below(PREFIXES.len())];
        check(&rng.tokens(prefix, TOKENS, 12));
        check(&rng.chars(prefix, ALPHABET, 48));
    }
}

#[test]
fn budgets_are_enforced() {
    let mut input = String::from("_OBJC_CLASS_$_");
    input.push_str(&"a".repeat(demangler::objc::MAX_INPUT_LEN));
    assert!(demangler::objc::parse(&input).is_err());

    let deep = "-[Foo bar[[]]]";
    assert!(demangler::objc::parse(deep).is_err());
}

#[test]
fn unknown_prefixes_are_not_objc() {
    for name in [
        "main",
        "_Z3foov",
        "__objc_exec_class",
        "__objc_load",
        "_OBJC_NOPE_$_Foo",
        "_OBJC_CLASS_$_",
        "-[Foo",
    ] {
        // The Apple namespace may be detected, but no unknown name may render
        // as Objective-C text.
        if let Ok(demangled) = Demangler::new().demangle(name) {
            assert_ne!(demangled.scheme(), Scheme::ObjC, "{name}");
        }
    }
}
