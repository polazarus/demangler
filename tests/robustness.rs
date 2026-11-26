//! Cross-backend adversarial and detection-normalization tests.
//!
//! The fuzz tests are deterministic: a fixed-seed SplitMix64 generator
//! produces short token- and character-based inputs for every enabled parser,
//! and every seed is also truncated at each character boundary. They assert
//! only invariants that hold for untrusted input:
//!
//! - no parser or printer panics;
//! - a successful render stays inside the backend's declared output budget;
//! - Go output never grows past its input.
//!
//! The detection tests pin how ambiguous and decorated names are classified.
//! Every backend test is feature-gated so each subset builds and passes alone.

use std::fmt::Write as _;
use std::string::String;

use demangler::{Demangler, Scheme};

/// Every backend's declared print budget is at most 1 MiB. Rust's output
/// budget is 1,000,000 bytes plus the `{size limit reached}` marker, which
/// still fits under this ceiling.
const MAX_OUTPUT_BYTES: usize = 1 << 20;

/// A byte that is not valid UTF-8 prints as `\xNN`, four bytes per input byte.
const ESCAPED_OUTPUT_BYTES: usize = 4;

/// Iterations per backend. Short inputs keep the whole file fast in debug.
const ITERATIONS: usize = 3_000;

/// Maximum generated tokens after the prefix. Only token-based fuzzing uses
/// this; the detection fuzz is character-based.
#[cfg(any(
    feature = "cpp-itanium",
    feature = "cpp-msvc",
    feature = "cpp-gnu-v2",
    feature = "rust",
    feature = "swift",
    feature = "go",
    feature = "objc"
))]
const MAX_PARTS: usize = 16;
/// Maximum generated characters after the prefix.
const MAX_CHARS: usize = 48;

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

    /// A value in `0..bound`. `bound` must be non-zero.
    fn below(&mut self, bound: usize) -> usize {
        let bound = u64::try_from(bound).unwrap_or(u64::MAX);
        usize::try_from(self.next_u64() % bound).unwrap_or(0)
    }

    #[cfg(any(
        feature = "cpp-itanium",
        feature = "cpp-msvc",
        feature = "cpp-gnu-v2",
        feature = "rust",
        feature = "swift",
        feature = "go",
        feature = "objc"
    ))]
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
    let Ok(demangled_symbol) = demangler.demangle(symbol) else {
        return;
    };
    let mut out = String::new();
    let _ = write!(out, "{demangled_symbol}");
    assert!(
        out.len() <= MAX_OUTPUT_BYTES,
        "{symbol:?} rendered {} bytes",
        out.len()
    );
    if demangled_symbol.scheme() == Scheme::Go {
        // Bytes that are not valid UTF-8 print as `\xNN`, so the rendered name
        // can grow to four times the encoded spelling.
        assert!(
            out.len() <= ESCAPED_OUTPUT_BYTES * symbol.len() + 64,
            "{symbol:?} grew from {} to {} bytes",
            symbol.len(),
            out.len()
        );
    }
}

/// Parse one C++ input directly and check the printer stays bounded.
#[cfg(any(feature = "cpp-itanium", feature = "cpp-msvc", feature = "cpp-gnu-v2"))]
fn check_cpp(symbol: &str) {
    let Ok(ast) = demangler::cpp::parse(symbol) else {
        return;
    };
    let mut out = String::new();
    let _ = write!(out, "{ast}");
    assert!(
        out.len() <= MAX_OUTPUT_BYTES,
        "{symbol:?} rendered {} bytes",
        out.len()
    );
}

/// Parse one Rust input directly and check the printer stays bounded.
#[cfg(feature = "rust")]
fn check_rust(symbol: &str) {
    let Ok(ast) = demangler::rust::parse(symbol) else {
        return;
    };
    let mut out = String::new();
    let _ = write!(out, "{ast}");
    assert!(
        out.len() <= MAX_OUTPUT_BYTES,
        "{symbol:?} rendered {} bytes",
        out.len()
    );
}

/// Parse and print one Swift input directly.
#[cfg(feature = "swift")]
fn check_swift(symbol: &str) {
    let _ = demangler::swift::parse(symbol);
    let mut out = String::new();
    let _ = demangler::swift::print(symbol, &mut out);
    assert!(
        out.len() <= MAX_OUTPUT_BYTES,
        "{symbol:?} rendered {} bytes",
        out.len()
    );
}

/// Parse one Go input directly and check its output stays bounded.
#[cfg(feature = "go")]
fn check_go(symbol: &str) {
    let Ok(ast) = demangler::go::parse(symbol) else {
        return;
    };
    let mut out = String::new();
    let _ = write!(out, "{ast}");
    assert!(
        out.len() <= ESCAPED_OUTPUT_BYTES * symbol.len() + 64,
        "{symbol:?} grew from {} to {} bytes",
        symbol.len(),
        out.len()
    );
}

/// Parse one Objective-C input directly and check the printer stays bounded.
#[cfg(feature = "objc")]
fn check_objc(symbol: &str) {
    let Ok(ast) = demangler::objc::parse(symbol) else {
        return;
    };
    let mut out = String::new();
    let _ = write!(out, "{ast}");
    assert!(
        out.len() <= MAX_OUTPUT_BYTES,
        "{symbol:?} rendered {} bytes",
        out.len()
    );
}

/// Valid symbols used as truncation seeds across all backends.
const SEEDS: &[&str] = &[
    "_ZN3Foo3barEv",
    "__ZN3Foo3barEv",
    "____Z1fPFivE",
    "_ZN4test4main17h1234567890abcdefE",
    "_RNvCs1234_4test4main",
    "_RC3foo",
    "__RC3foo",
    "$s4main3fooyySiF",
    "_T04main3fooSi_yF",
    "?x@@3HA",
    "?foo@@YAXXZ",
    "fmt.Println",
    "runtime.(*mLockProfile).captureStack",
    "type:.eq.main.Pair[string,int]",
    "go:itab.*os.File,io.Writer",
    "foo__Fiv",
    "_OBJC_CLASS_$_Foo",
    "__OBJC_$_CLASS_METHODS_Foo",
    "__objc_class_name_Foo",
    "-[Foo bar:]",
    "_$_3foo",
    "_3foo$bar",
    "a__b__c",
    "_Z3foov.llvm.1234",
    "main.llvm.1234",
    "_Z3foov@@GLIBCXX_3.4",
    "_D3foo3barFZv",
    "Java_com_example_Foo_bar___3Ljava_lang_String_2I",
    "ada__exceptions__triggered_by_abort",
    "__fprobe_MOD_add",
    "DELPHI_PROBE_$$_UNITPROC$LONGINT",
    "camlStdlib__List.map_569",
    "__imp__foo@4",
];

#[test]
fn truncations_never_panic() {
    let demangler = Demangler::new();
    for &seed in SEEDS {
        for (end, _) in seed.char_indices() {
            check_demangled(&demangler, &seed[..end]);
        }
        check_demangled(&demangler, seed);
    }
}

#[test]
fn fuzz_detection_never_panics() {
    // The letters the new byte-native backends need (D, JNI, Ada, Fortran,
    // Delphi, OCaml and COFF names) are part of the alphabet so random input
    // can reach them. Iterations and length are fixed, so the sweep stays
    // bounded.
    const ALPHABET: &[char] = &[
        'a', 'Z', '0', '_', '$', '?', '.', '@', ':', '/', '(', ')', '[', ']', '{', '}', '<', '>',
        ',', '*', '-', '+', '%', '·', '\t', '\n', ' ', 'A', 'C', 'D', 'E', 'F', 'I', 'J', 'L', 'M',
        'N', 'O', 'P', 'R', 'S', 'T', 'V', 'X', 'b', 'c', 'd', 'e', 'f', 'h', 'i', 'l', 'm', 'n',
        'o', 'p', 'r', 's', 't', 'u', 'v', 'x',
    ];
    let mut rng = Rng::new(0xD37E_C7ED_u64);
    let demangler = Demangler::new();
    for _ in 0..(ITERATIONS * 4) {
        let symbol = rng.chars("", ALPHABET, MAX_CHARS);
        let _ = Scheme::detect(&symbol);
        check_demangled(&demangler, &symbol);
    }
}

#[test]
fn fuzz_truncations_never_panic() {
    const ALPHABET: &[char] = &[
        'a', 'Z', '0', '_', '$', '?', '.', '@', ':', '/', '(', ')', '[', ']', '{', '}', '<', '>',
        ',', '*', '-', '+', '%', '·',
    ];
    let mut rng = Rng::new(0x7A11_C47E_u64);
    let demangler = Demangler::new();
    for _ in 0..ITERATIONS {
        let symbol = rng.chars("", ALPHABET, MAX_CHARS);
        for (end, _) in symbol.char_indices() {
            check_demangled(&demangler, &symbol[..end]);
        }
        check_demangled(&demangler, &symbol);
    }
}

#[cfg(feature = "cpp-itanium")]
#[test]
fn fuzz_itanium_parser() {
    const PREFIXES: &[&str] = &["_Z", "__Z", "___Z", "____Z"];
    const TOKENS: &[&str] = &[
        "N", "E", "v", "i", "P", "K", "R", "O", "1", "2", "3", "4foo", "5", "T_", "S_", "I", "J",
        "C1", "D0", "F", "A5_", "M", "U", "L", "Z", "W", "B", "f", "d", "c", "l", "s", "x", "0",
        "9", "_", "St", "Sa", "Dn",
    ];
    const ALPHABET: &[char] = &[
        'N', 'E', 'v', 'i', 'P', 'K', 'R', 'O', 'T', 'S', 'I', 'J', 'C', 'D', 'F', 'A', 'M', 'U',
        'L', 'Z', 'W', 'B', '0', '1', '2', '3', '4', '5', '6', '7', '8', '9', 'a', 'b', 'c', 'd',
        'e', 'f', 'g', 'h', 'l', 'm', 'n', 'o', 'p', 's', 't', 'u', 'x', 'y', 'z', '_', '$', '.',
    ];
    let mut rng = Rng::new(0x17A7_A1A1_u64);
    for _ in 0..ITERATIONS {
        let prefix = PREFIXES[rng.below(PREFIXES.len())];
        check_cpp(&rng.tokens(prefix, TOKENS, MAX_PARTS));
        check_cpp(&rng.chars(prefix, ALPHABET, MAX_CHARS));
    }
}

#[cfg(feature = "cpp-msvc")]
#[test]
fn fuzz_msvc_parser() {
    const PREFIXES: &[&str] = &["?", ".?"];
    const TOKENS: &[&str] = &[
        "?",
        "@",
        "$",
        "A",
        "B",
        "C",
        "D",
        "E",
        "F",
        "G",
        "H",
        "I",
        "J",
        "K",
        "L",
        "M",
        "N",
        "O",
        "P",
        "Q",
        "R",
        "S",
        "T",
        "U",
        "V",
        "W",
        "X",
        "Y",
        "Z",
        "0",
        "1",
        "2",
        "3",
        "4",
        "5",
        "6",
        "7",
        "8",
        "9",
        "_",
        "?x@@3HA",
        "?foo@@YAXXZ",
    ];
    const ALPHABET: &[char] = &[
        '?', '@', '$', 'A', 'B', 'C', 'D', 'E', 'F', 'G', 'H', 'I', 'J', 'K', 'L', 'M', 'N', 'O',
        'P', 'Q', 'R', 'S', 'T', 'U', 'V', 'W', 'X', 'Y', 'Z', '0', '1', '2', '3', '4', '5', '6',
        '7', '8', '9', '_', '.', ':',
    ];
    let mut rng = Rng::new(0x35C0_55E5_u64);
    for _ in 0..ITERATIONS {
        let prefix = PREFIXES[rng.below(PREFIXES.len())];
        check_cpp(&rng.tokens(prefix, TOKENS, MAX_PARTS));
        check_cpp(&rng.chars(prefix, ALPHABET, MAX_CHARS));
    }
}

#[cfg(feature = "cpp-gnu-v2")]
#[test]
fn fuzz_legacy_parser() {
    const TOKENS: &[&str] = &[
        "__", "_", "$", "F", "v", "i", "P", "C", "c", "Q", "1", "2", "3", "4foo", "bar", "T0",
        "N0", "Z", "e", "pl", "op$pl", "_vt$", "__ti", "__thunk_", "Q2_", ".",
    ];
    const ALPHABET: &[char] = &[
        'a', 'b', 'c', 'd', 'e', 'f', 'g', 'i', 'l', 'm', 'n', 'o', 'p', 's', 't', 'v', 'x', 'Z',
        'F', 'P', 'C', 'Q', 'T', 'N', '0', '1', '2', '3', '4', '5', '6', '7', '8', '9', '_', '$',
        '.',
    ];
    let mut rng = Rng::new(0x1E6A_C7A1_u64);
    for _ in 0..ITERATIONS {
        check_cpp(&rng.tokens("", TOKENS, MAX_PARTS));
        check_cpp(&rng.chars("", ALPHABET, MAX_CHARS));
    }
}

#[cfg(feature = "rust")]
#[test]
fn fuzz_rust_parser() {
    const LEGACY_PREFIXES: &[&str] = &["_ZN", "__ZN", "ZN"];
    const LEGACY_TOKENS: &[&str] = &[
        "E",
        "17h",
        "1234567890abcdef",
        "3foo",
        "4main",
        "$LT$",
        "$SP$",
        "..",
        "_$",
        "0",
        "9",
        "N",
        "5",
    ];
    const V0_PREFIXES: &[&str] = &["_R", "__R", "R"];
    const V0_TOKENS: &[&str] = &[
        "C", "N", "M", "X", "Y", "v", "t", "i", "p", "P", "R", "A", "B", "_", "0", "1", "2", "3",
        "4", "5", "6", "7", "8", "9", "a", "b", "c", "s", "h", "E", "I", "K", "L", "w", "S", "T",
    ];
    const ALPHABET: &[char] = &[
        '0', '1', '2', '3', '4', '5', '6', '7', '8', '9', 'A', 'B', 'C', 'D', 'E', 'F', 'G', 'H',
        'I', 'J', 'K', 'L', 'M', 'N', 'O', 'P', 'Q', 'R', 'S', 'T', 'U', 'V', 'W', 'X', 'Y', 'Z',
        'a', 'b', 'c', 'd', 'e', 'f', 'g', 'h', 'i', 'j', 'k', 'l', 'm', 'n', 'o', 'p', 'q', 'r',
        's', 't', 'u', 'v', 'w', 'x', 'y', 'z', '_', '$', '.',
    ];
    let mut rng = Rng::new(0x4E57_1E6A_u64);
    for _ in 0..ITERATIONS {
        let legacy = LEGACY_PREFIXES[rng.below(LEGACY_PREFIXES.len())];
        check_rust(&rng.tokens(legacy, LEGACY_TOKENS, MAX_PARTS));
        check_rust(&rng.chars(legacy, ALPHABET, MAX_CHARS));

        let v0 = V0_PREFIXES[rng.below(V0_PREFIXES.len())];
        check_rust(&rng.tokens(v0, V0_TOKENS, MAX_PARTS));
        check_rust(&rng.chars(v0, ALPHABET, MAX_CHARS));
    }
}

#[cfg(feature = "swift")]
#[test]
fn fuzz_swift_parser() {
    const PREFIXES: &[&str] = &["$s", "$S", "_T0"];
    const TOKENS: &[&str] = &[
        "4main", "3foo", "y", "F", "S", "i", "C", "V", "t", "T", "P", "R", "G", "_", "0", "1", "2",
        "3", "4", "5", "6", "7", "8", "9", "A", "B", "Q", "U", "W", "z", "l", "r", "n", "o", "p",
        "d", "m", "f", "g", "h", "s", "v", "x", "b", "e", "j", "k", "q", "u",
    ];
    const ALPHABET: &[char] = &[
        '0', '1', '2', '3', '4', '5', '6', '7', '8', '9', 'A', 'B', 'C', 'D', 'E', 'F', 'G', 'H',
        'I', 'J', 'K', 'L', 'M', 'N', 'O', 'P', 'Q', 'R', 'S', 'T', 'U', 'V', 'W', 'X', 'Y', 'Z',
        'a', 'b', 'c', 'd', 'e', 'f', 'g', 'h', 'i', 'j', 'k', 'l', 'm', 'n', 'o', 'p', 'q', 'r',
        's', 't', 'u', 'v', 'w', 'x', 'y', 'z', '_', '$', '.',
    ];
    let mut rng = Rng::new(0x005A_17F7_u64);
    for _ in 0..ITERATIONS {
        let prefix = PREFIXES[rng.below(PREFIXES.len())];
        check_swift(&rng.tokens(prefix, TOKENS, MAX_PARTS));
        check_swift(&rng.chars(prefix, ALPHABET, MAX_CHARS));
    }
}

#[cfg(feature = "go")]
#[test]
fn fuzz_go_parser() {
    const PREFIXES: &[&str] = &["", "fmt.", "main.", "type:", "go:", "go.shape.", "$f64."];
    const TOKENS: &[&str] = &[
        "type:",
        "go:",
        "go.shape.",
        "fmt.",
        "main.",
        "runtime.",
        "·",
        ".abi0",
        ".func1",
        "(*T)",
        "[",
        "]",
        "(",
        ")",
        "{",
        "}",
        "%2e",
        "/",
        "*",
        "int",
        "string",
        ",",
        " ",
        "<",
        ">",
        "$f64.",
        "gcbits.",
        "gclocals.",
        "..dict.",
        "..inittask",
    ];
    const ALPHABET: &[char] = &[
        'a', 'b', 'c', 'd', 'e', 'f', 'g', 'i', 'l', 'm', 'n', 'o', 'p', 'r', 's', 't', 'v', 'x',
        'A', 'F', 'P', 'T', '0', '1', '2', '3', '4', '5', '6', '7', '8', '9', '_', '.', '·', '(',
        ')', '[', ']', '{', '}', '<', '>', ',', '*', '/', '%', ':', '$', ' ', '-',
    ];
    let mut rng = Rng::new(0x60_1EAF_u64);
    let demangler = Demangler::new();
    for _ in 0..ITERATIONS {
        let prefix = PREFIXES[rng.below(PREFIXES.len())];
        let token = rng.tokens(prefix, TOKENS, MAX_PARTS);
        check_go(&token);
        check_demangled(&demangler, &token);

        let chars = rng.chars(prefix, ALPHABET, MAX_CHARS);
        check_go(&chars);
        check_demangled(&demangler, &chars);
    }
}

#[cfg(feature = "objc")]
#[test]
fn fuzz_objc_parser() {
    const PREFIXES: &[&str] = &[
        "_OBJC_",
        "__OBJC_",
        "OBJC_",
        "__objc_",
        "__block_descriptor_",
        "-[",
        "+[",
    ];
    const TOKENS: &[&str] = &[
        "CLASS_$_",
        "METACLASS_$_",
        "IVAR_$_",
        "$_",
        "CLASS_METHODS_",
        "INSTANCE_METHODS_",
        "PROP_LIST_",
        "CLASS_PROP_LIST_",
        "EHTYPE_$_",
        "CATEGORY_",
        "class_name_",
        "ivar_offset_",
        "Foo",
        "Bar",
        "_x",
        "(",
        ")",
        "|",
        "]",
        "[",
        " ",
        ":",
        "\u{1}",
    ];
    const ALPHABET: &[char] = &[
        'A', 'B', 'C', 'D', 'E', 'F', 'G', 'H', 'I', 'J', 'K', 'L', 'M', 'N', 'O', 'P', 'Q', 'R',
        'S', 'T', 'U', 'V', 'W', 'X', 'Y', 'Z', 'a', 'b', 'c', 'd', 'e', 'f', 'g', 'h', 'i', 'j',
        'k', 'l', 'm', 'n', 'o', 'p', 'q', 'r', 's', 't', 'u', 'v', 'w', 'x', 'y', 'z', '0', '1',
        '2', '3', '4', '5', '6', '7', '8', '9', '_', '$', '.', '@', '|', '(', ')', '[', ']', ' ',
        ':', '-', '\u{1}', '\u{1b}',
    ];
    let mut rng = Rng::new(0x0B1E_C7A1_u64);
    for _ in 0..ITERATIONS {
        let prefix = PREFIXES[rng.below(PREFIXES.len())];
        check_objc(&rng.tokens(prefix, TOKENS, MAX_PARTS));
        check_objc(&rng.chars(prefix, ALPHABET, MAX_CHARS));
    }
}

#[cfg(feature = "objc")]
#[test]
fn objc_budgets_are_enforced() {
    let mut input = String::from("_OBJC_CLASS_$_");
    input.push_str(&"A".repeat(demangler::objc::MAX_INPUT_LEN));
    assert!(demangler::objc::parse(&input).is_err());

    let block = "__block_descriptor_40_\u{1b}[2J";
    assert!(demangler::objc::parse(block).is_err());
}

#[cfg(feature = "cpp-itanium")]
#[test]
fn cpp_budgets_are_enforced() {
    // `cpp::MAX_INPUT_LEN` is crate-private and is 64 KiB.
    let mut input = String::from("_Z");
    input.push_str(&"1".repeat(64 * 1024));
    assert!(demangler::cpp::parse(&input).is_err());

    let mut deep = String::from("_Z1f");
    deep.push_str(&"P".repeat(1_000));
    deep.push('i');
    assert!(demangler::cpp::parse(&deep).is_err());
}

#[cfg(feature = "rust")]
#[test]
fn rust_budgets_are_enforced() {
    let mut input = String::from("_R");
    input.push_str(&"p".repeat(demangler::rust::MAX_INPUT_LEN));
    assert!(demangler::rust::parse(&input).is_err());

    let mut deep = String::from("_RIC3foo");
    deep.push_str(&"P".repeat(demangler::rust::MAX_DEPTH as usize + 1));
    deep.push_str("iE");
    assert!(demangler::rust::parse(&deep).is_err());
}

#[cfg(feature = "swift")]
#[test]
fn swift_budgets_are_enforced() {
    let deep = format!("$s{}Si", "Sg".repeat(1_000));
    assert!(demangler::swift::parse(&deep).is_err());
}

#[cfg(feature = "go")]
#[test]
fn go_budgets_are_enforced() {
    let mut input = String::from("fmt.");
    input.push_str(&"a".repeat(demangler::go::MAX_INPUT_LEN));
    assert!(demangler::go::parse(&input).is_err());

    let mut deep = String::from("pkg.F");
    deep.push_str(&"[".repeat(demangler::go::MAX_DEPTH + 1));
    deep.push_str(&"]".repeat(demangler::go::MAX_DEPTH + 1));
    assert!(demangler::go::parse(&deep).is_err());
}

#[cfg(feature = "cpp-itanium")]
#[test]
fn itanium_leading_underscores_normalize() {
    let demangler = Demangler::new();
    for (symbol, expected) in [
        ("_Z3foov", "foo()"),
        ("__Z3foov", "foo()"),
        ("___Z3foov", "foo()"),
        ("____Z3foov", "foo()"),
    ] {
        assert_eq!(Scheme::detect(symbol), Scheme::CppItanium, "{symbol}");
        let demangled_symbol = demangler
            .demangle(symbol)
            .unwrap_or_else(|error| panic!("{symbol}: {error}"));
        assert_eq!(demangled_symbol.scheme(), Scheme::CppItanium, "{symbol}");
        assert_eq!(demangled_symbol.to_string(), expected, "{symbol}");
    }
}

#[cfg(feature = "rust")]
#[test]
fn rust_v0_prefixes_normalize() {
    let demangler = Demangler::new();
    for (symbol, expected) in [
        ("_RC3foo", "foo"),
        ("__RC3foo", "foo"),
        ("_RNvCs1234_4test4main", "test[3c1c0]::main"),
        ("__RNvCs1234_4test4main", "test[3c1c0]::main"),
        ("_RNvCs1234_4test4main.llvm.9D1C9369", "test[3c1c0]::main"),
    ] {
        assert_eq!(Scheme::detect(symbol), Scheme::RustV0, "{symbol}");
        let demangled_symbol = demangler
            .demangle(symbol)
            .unwrap_or_else(|error| panic!("{symbol}: {error}"));
        assert_eq!(demangled_symbol.scheme(), Scheme::RustV0, "{symbol}");
        assert_eq!(demangled_symbol.to_string(), expected, "{symbol}");
    }
    // Without a leading underscore `R...` is indistinguishable from a C
    // identifier, so detection leaves it alone.
    assert_eq!(Scheme::detect("RC3foo"), Scheme::C);
    assert_eq!(Scheme::detect("RNvCs1234_4test4main"), Scheme::C);
}

#[cfg(all(feature = "rust", feature = "cpp-itanium"))]
#[test]
fn rust_legacy_hash_wins_over_cpp() {
    let demangler = Demangler::new();
    for symbol in [
        "_ZN4test4main17h1234567890abcdefE",
        "__ZN4test4main17h1234567890abcdefE",
        "ZN4test4main17h1234567890abcdefE",
        "_ZN4test4main17h1234567890abcdefE.llvm.9D1C9369",
        "_ZN4test4main17h1234567890abcdefE@@16",
    ] {
        assert_eq!(Scheme::detect(symbol), Scheme::RustLegacy, "{symbol}");
        let demangled_symbol = demangler
            .demangle(symbol)
            .unwrap_or_else(|error| panic!("{symbol}: {error}"));
        assert_eq!(demangled_symbol.scheme(), Scheme::RustLegacy, "{symbol}");
        assert_eq!(
            demangled_symbol.to_string(),
            "test::main::h1234567890abcdef",
            "{symbol}"
        );
    }
    // `_ZN` without the rustc crate hash is ambiguous; Itanium C++ keeps the
    // prefix.
    for symbol in ["_ZN3fooE", "__ZN3fooE", "_ZN3Foo3barEv", "__ZN3Foo3barEv"] {
        assert_eq!(Scheme::detect(symbol), Scheme::CppItanium, "{symbol}");
        let demangled_symbol = demangler
            .demangle(symbol)
            .unwrap_or_else(|error| panic!("{symbol}: {error}"));
        assert_eq!(demangled_symbol.scheme(), Scheme::CppItanium, "{symbol}");
    }
}

#[cfg(feature = "cpp-itanium")]
#[test]
fn cpp_suffixes_do_not_change_detection() {
    let demangler = Demangler::new();
    for symbol in [
        "_Z3foov.llvm.9D1C9369",
        "_Z3foov@@GLIBCXX_3.4",
        "__Z3foov.llvm.9D1C9369@@16",
    ] {
        assert_eq!(Scheme::detect(symbol), Scheme::CppItanium, "{symbol}");
        let demangled_symbol = demangler
            .demangle(symbol)
            .unwrap_or_else(|error| panic!("{symbol}: {error}"));
        assert_eq!(demangled_symbol.scheme(), Scheme::CppItanium, "{symbol}");
        assert_eq!(demangled_symbol.to_string(), "foo()", "{symbol}");
    }
}

#[test]
fn c_suffixes_are_detected_as_c() {
    for symbol in ["main.llvm.9D1C9369", "main@@GLIBC_2.2.5", "main@VERS_1.0"] {
        assert_eq!(Scheme::detect(symbol), Scheme::C, "{symbol}");
        let demangled = Demangler::new()
            .demangle(symbol)
            .unwrap_or_else(|error| panic!("{symbol}: {error}"));
        assert_eq!(demangled.scheme(), Scheme::C, "{symbol}");
        // Plain C is a pass-through: the decoration stays in the text.
        assert_eq!(demangled.to_string(), symbol, "{symbol}");
    }
}

#[cfg(feature = "go")]
#[test]
fn dotted_go_names_are_not_c() {
    let demangler = Demangler::new();
    for symbol in [
        "fmt.Println",
        "main.main",
        "example.com/pkg.Make",
        "foo.bar",
    ] {
        assert_eq!(Scheme::detect(symbol), Scheme::Go, "{symbol}");
        let demangled_symbol = demangler
            .demangle(symbol)
            .unwrap_or_else(|error| panic!("{symbol}: {error}"));
        assert_eq!(demangled_symbol.scheme(), Scheme::Go, "{symbol}");
    }
    // A ThinLTO suffix is a C decoration, not a Go package separator.
    assert_eq!(Scheme::detect("foo.llvm.9D1C9369"), Scheme::C);
    // `is_go` deliberately rejects `.llvm.`-decorated names, so a Go-like
    // name with one stays out of Go instead of failing in the parser.
    assert_ne!(Scheme::detect("fmt.Println.llvm.9D1C9369"), Scheme::Go);
}

#[cfg(feature = "cpp-msvc")]
#[test]
fn msvc_and_digital_mars_names_are_separated_by_the_parser() {
    let demangler = Demangler::new();
    let msvc = demangler
        .demangle("?x@@3HA")
        .unwrap_or_else(|error| panic!("?x@@3HA: {error}"));
    assert_eq!(msvc.scheme(), Scheme::CppMsvc);
    assert_eq!(msvc.to_string(), "int x");

    // Digital Mars uses `?` too, but its `$`-separated grammar is not MSVC.
    // Detection is a prefix test; the backend must reject it instead of
    // producing text.
    let digital_mars = "?foo$q$C";
    assert_eq!(Scheme::detect(digital_mars), Scheme::CppMsvc);
    assert!(demangler.demangle(digital_mars).is_err());

    // The `.?` RTTI descriptor form is Microsoft as well.
    assert_eq!(Scheme::detect(".?AVType@@"), Scheme::CppMsvc);
}

#[cfg(not(feature = "cpp-itanium"))]
#[test]
fn itanium_is_not_detected_without_its_feature() {
    for symbol in ["_Z3foov", "__Z3foov", "___Z3foov", "____Z3foov"] {
        assert_ne!(Scheme::detect(symbol), Scheme::CppItanium, "{symbol}");
    }
}

#[cfg(not(feature = "cpp-msvc"))]
#[test]
fn msvc_is_not_detected_without_its_feature() {
    for symbol in ["?x@@3HA", "?foo@@YAXXZ", ".?AVType@@"] {
        assert_ne!(Scheme::detect(symbol), Scheme::CppMsvc, "{symbol}");
    }
}

#[cfg(not(feature = "rust"))]
#[test]
fn rust_is_not_detected_without_its_feature() {
    for symbol in [
        "_RNvCs1234_4test4main",
        "__RNvCs1234_4test4main",
        "_RC3foo",
        "_ZN4test4main17h1234567890abcdefE",
    ] {
        assert!(
            !matches!(Scheme::detect(symbol), Scheme::RustV0 | Scheme::RustLegacy),
            "{symbol}"
        );
    }
}

#[cfg(not(feature = "swift"))]
#[test]
fn swift_is_not_detected_without_its_feature() {
    for symbol in ["$s4main3fooyySiF", "$S4main3fooyySiF", "_T04main3fooSi_yF"] {
        assert!(
            !matches!(
                Scheme::detect(symbol),
                Scheme::SwiftStable | Scheme::SwiftLegacy
            ),
            "{symbol}"
        );
    }
}

#[cfg(not(feature = "go"))]
#[test]
fn go_is_not_detected_without_its_feature() {
    for symbol in ["fmt.Println", "main.main", "go:itab.*os.File,io.Writer"] {
        assert_ne!(Scheme::detect(symbol), Scheme::Go, "{symbol}");
    }
}

#[cfg(not(feature = "cpp-gnu-v2"))]
#[test]
fn legacy_is_not_detected_without_its_feature() {
    for symbol in ["foo__Fiv", "_$_3foo", "_3foo$bar", "a__b__c"] {
        assert_ne!(
            Scheme::detect(symbol).language(),
            demangler::Language::Cpp,
            "{symbol}"
        );
    }
}

#[cfg(not(feature = "d"))]
#[test]
fn d_is_not_detected_without_its_feature() {
    for symbol in ["_D3foo3barFZv", "__D3foo3barFZv", "_Dmain"] {
        assert_ne!(Scheme::detect(symbol), Scheme::D, "{symbol}");
    }
}

#[cfg(not(feature = "jni"))]
#[test]
fn jni_is_not_detected_without_its_feature() {
    for symbol in [
        "Java_com_example_Foo_plain",
        "Java_com_example_Foo_bar__I",
        "Java_p_T_baz__Lq_1w_x_2_3_3I",
    ] {
        assert_ne!(Scheme::detect(symbol), Scheme::Jni, "{symbol}");
    }
}

#[cfg(not(feature = "ada"))]
#[test]
fn ada_is_not_detected_without_its_feature() {
    for symbol in [
        "ada__exceptions__triggered_by_abort",
        "_ada__foo",
        "system__standard_library",
    ] {
        assert_ne!(Scheme::detect(symbol), Scheme::Ada, "{symbol}");
    }
}

#[cfg(not(feature = "fortran"))]
#[test]
fn fortran_is_not_detected_without_its_feature() {
    for symbol in ["__fprobe_MOD_add", "___fprobe_MOD_add", "MAIN__"] {
        assert_ne!(Scheme::detect(symbol), Scheme::Fortran, "{symbol}");
    }
}

#[cfg(not(feature = "delphi"))]
#[test]
fn delphi_is_not_detected_without_its_feature() {
    for symbol in [
        "DELPHI_PROBE_$$_UNITPROC$LONGINT",
        "P$DELPHI_PROBE_PROG_$$_PROGPROC$LONGINT",
        "DELPHI_PROBE$_$TPROBECLASS_$__$$_PLAIN",
    ] {
        assert_ne!(Scheme::detect(symbol), Scheme::Delphi, "{symbol}");
    }
}

#[cfg(not(feature = "ocaml"))]
#[test]
fn ocaml_is_not_detected_without_its_feature() {
    for symbol in [
        "camlStdlib__List.map_569",
        "camlOps.$2b$2b_270",
        "caml_alloc",
    ] {
        assert_ne!(Scheme::detect(symbol), Scheme::OCaml, "{symbol}");
    }
}

#[cfg(not(feature = "coff"))]
#[test]
fn coff_decorations_are_not_decoded_without_their_feature() {
    // Without the decoration parser the `@N` suffix is not a GNU version, so
    // the decorated name is neither peeled nor claimed: it stays unknown text.
    for symbol in ["_foo@8", "__imp__foo@4"] {
        assert_eq!(Scheme::detect(symbol), Scheme::Unknown, "{symbol}");
        let demangled = Demangler::new()
            .demangle(symbol)
            .unwrap_or_else(|error| panic!("{symbol}: {error}"));
        assert_eq!(demangled.scheme(), Scheme::Unknown, "{symbol}");
        assert_eq!(demangled.to_string(), symbol, "{symbol}");
    }
}
