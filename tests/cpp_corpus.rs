//! Itanium C++ corpus harness.
//!
//! Reads the vendored libc++abi `DemangleTestCases.inc` corpus and measures how
//! much of it the phase-0 AST parser accepts. The corpus is reference material:
//! when `refs/` is absent the test skips instead of failing.
//!
//! The harness reports four classes:
//!
//! - `pass`: parsed into an AST with no [`Kind::Opaque`] nodes;
//! - `wrong`: parsed, but the AST contains an `Opaque` node (lossy coverage);
//! - `error`: rejected by the parser;
//! - `total`: every pair in the corpus.
//!
//! There is no AST printer yet, so this harness measures parser acceptance and
//! AST completeness, not byte-identical demangled output. It makes no claim of
//! full 29,933-pair parity.

use std::collections::BTreeMap;
use std::string::String;
use std::vec::Vec;

use demangle::cpp::{self, Kind, NodeRef};

const CORPUS: &str = "refs/cpp/itanium/DemangleTestCases.inc";

/// Measured floor for phase-0 coverage. Raise it when coverage improves; never
/// lower it.
const MIN_PASS: usize = 25_500;

#[test]
fn itanium_corpus_baseline() {
    let Ok(source) = std::fs::read_to_string(CORPUS) else {
        eprintln!("skipping Itanium corpus: {CORPUS} not found");
        return;
    };
    let pairs = parse_corpus(&source).expect("corpus should be well formed");
    let total = pairs.len();
    let mut pass = 0_usize;
    let mut wrong = 0_usize;
    let mut error = 0_usize;
    let mut reasons: BTreeMap<String, usize> = BTreeMap::new();
    let mut samples: Vec<String> = Vec::new();

    for (mangled, _expected) in &pairs {
        match cpp::parse(mangled) {
            Ok(ast) => {
                if has_opaque(&ast.root()) {
                    wrong += 1;
                } else {
                    pass += 1;
                }
            }
            Err(parse_error) => {
                error += 1;
                *reasons
                    .entry(format!("{:?}", parse_error.kind()))
                    .or_default() += 1;
                if samples.len() < 8 {
                    samples.push(format!("{mangled}: {parse_error}"));
                }
            }
        }
    }

    eprintln!("cpp corpus: total={total} pass={pass} wrong={wrong} error={error}");
    eprintln!("cpp corpus errors by kind: {reasons:?}");
    for sample in &samples {
        eprintln!("  sample: {sample}");
    }

    assert_eq!(total, 29_933, "corpus pair count changed");
    assert_eq!(pass + wrong + error, total);
    assert_eq!(wrong, 0, "phase 0 does not emit Opaque nodes");
    assert!(
        pass >= MIN_PASS,
        "pass count {pass} is below the measured floor {MIN_PASS}"
    );
}

#[test]
fn core_subset_is_inspectable() {
    // Representative symbols from the supported phase-0 grammar. Every one must
    // parse without Opaque nodes and expose the expected root shape.
    let symbols = [
        "_Z1A",
        "_Z1Av",
        "_Z1A1B1C",
        "_ZN3Foo3barEv",
        "_ZNK1A1fEv",
        "_ZNR1X1fEv",
        "_Z1fPKc",
        "_Z1fA5_i",
        "_Z1fPFivE",
        "_Z1fM1AFivE",
        "_Z3fooIiEvT_",
        "_ZSt4cout",
        "_Z1f1AS_S_",
        "_Z1fDn",
        "_Z1fDB3_",
        "_Z1fDF16_",
        "_Z1fDp1A",
        "_Z1fu3foo",
    ];
    for symbol in symbols {
        let ast = cpp::parse(symbol).unwrap_or_else(|error| panic!("{symbol}: {error}"));
        assert_eq!(ast.root().kind(), Kind::Encoding, "{symbol}");
        assert!(!has_opaque(&ast.root()), "{symbol} contains Opaque");
    }
}

/// True when the graph contains an opaque (unmodelled) production.
fn has_opaque(node: &NodeRef<'_>) -> bool {
    node.kind() == Kind::Opaque || node.children().iter().any(has_opaque)
}

/// Parse the C++ array literal in `DemangleTestCases.inc`.
///
/// The file is C++ source: pairs may span lines, adjacent string literals are
/// concatenated, and `//` comments may appear between tokens.
fn parse_corpus(source: &str) -> Result<Vec<(String, String)>, String> {
    let bytes = source.as_bytes();
    let mut pos = 0_usize;
    let mut pairs = Vec::new();
    loop {
        skip_trivia(bytes, &mut pos)?;
        if pos >= bytes.len() {
            break;
        }
        expect(bytes, &mut pos, b'{')?;
        let mangled = parse_string_field(bytes, &mut pos)?;
        skip_trivia(bytes, &mut pos)?;
        expect(bytes, &mut pos, b',')?;
        let expected = parse_string_field(bytes, &mut pos)?;
        skip_trivia(bytes, &mut pos)?;
        expect(bytes, &mut pos, b'}')?;
        skip_trivia(bytes, &mut pos)?;
        if bytes.get(pos) == Some(&b',') {
            pos += 1;
        }
        pairs.push((mangled, expected));
    }
    Ok(pairs)
}

/// One or more adjacent C string literals.
fn parse_string_field(bytes: &[u8], pos: &mut usize) -> Result<String, String> {
    let mut out = Vec::new();
    let mut any = false;
    loop {
        skip_trivia(bytes, pos)?;
        if bytes.get(*pos) != Some(&b'"') {
            break;
        }
        parse_string(bytes, pos, &mut out)?;
        any = true;
    }
    if !any {
        return Err(format!("expected a string at byte {}", *pos));
    }
    Ok(String::from_utf8_lossy(&out).into_owned())
}

/// Decode one C string literal into `out`.
fn parse_string(bytes: &[u8], pos: &mut usize, out: &mut Vec<u8>) -> Result<(), String> {
    expect(bytes, pos, b'"')?;
    while let Some(&byte) = bytes.get(*pos) {
        match byte {
            b'"' => {
                *pos += 1;
                return Ok(());
            }
            b'\\' => {
                *pos += 1;
                let escape = *bytes
                    .get(*pos)
                    .ok_or_else(|| format!("unterminated escape at byte {}", *pos))?;
                *pos += 1;
                match escape {
                    b'n' => out.push(b'\n'),
                    b't' => out.push(b'\t'),
                    b'r' => out.push(b'\r'),
                    b'0'..=b'7' => {
                        let mut value = u32::from(escape - b'0');
                        let mut digits = 1;
                        while digits < 3 {
                            let Some(&next @ b'0'..=b'7') = bytes.get(*pos) else {
                                break;
                            };
                            value = value * 8 + u32::from(next - b'0');
                            *pos += 1;
                            digits += 1;
                        }
                        out.push(value as u8);
                    }
                    b'x' => {
                        let mut value = 0_u32;
                        let mut digits = 0;
                        while digits < 2 {
                            let Some(&next) = bytes.get(*pos) else {
                                break;
                            };
                            let Some(digit) = (next as char).to_digit(16) else {
                                break;
                            };
                            value = value * 16 + digit;
                            *pos += 1;
                            digits += 1;
                        }
                        out.push(value as u8);
                    }
                    other => out.push(other),
                }
            }
            _ => {
                out.push(byte);
                *pos += 1;
            }
        }
    }
    Err(format!("unterminated string at byte {}", *pos))
}

/// Skip whitespace and `//` line comments.
fn skip_trivia(bytes: &[u8], pos: &mut usize) -> Result<(), String> {
    loop {
        while bytes
            .get(*pos)
            .is_some_and(|byte| byte.is_ascii_whitespace())
        {
            *pos += 1;
        }
        if bytes.get(*pos) == Some(&b'/') && bytes.get(*pos + 1) == Some(&b'/') {
            *pos += 2;
            while bytes.get(*pos).is_some_and(|byte| *byte != b'\n') {
                *pos += 1;
            }
        } else if bytes.get(*pos) == Some(&b'/') && bytes.get(*pos + 1) == Some(&b'*') {
            *pos += 2;
            loop {
                let Some(&byte) = bytes.get(*pos) else {
                    return Err(format!("unterminated block comment at byte {}", *pos));
                };
                *pos += 1;
                if byte == b'*' && bytes.get(*pos) == Some(&b'/') {
                    *pos += 1;
                    break;
                }
            }
        } else {
            return Ok(());
        }
    }
}

fn expect(bytes: &[u8], pos: &mut usize, expected: u8) -> Result<(), String> {
    if bytes.get(*pos) == Some(&expected) {
        *pos += 1;
        Ok(())
    } else {
        Err(format!(
            "expected {:?} at byte {}, found {:?}",
            expected as char,
            pos,
            bytes.get(*pos).copied().map(char::from)
        ))
    }
}
