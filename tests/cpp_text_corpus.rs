//! Itanium C++ text corpus harness.
//!
//! Reads the vendored libc++abi `DemangleTestCases.inc` corpus and compares the
//! in-tree printer's output with the expected demangled text. The corpus is
//! reference material: when `refs/` is absent the test skips instead of
//! failing.
//!
//! Three classes are reported:
//!
//! - `exact`: the printer reproduced the expected text byte for byte;
//! - `wrong`: the symbol parsed and printed, but the text differs;
//! - `error`: the parser or the printer rejected the symbol.
//!
//! `error` must stay empty: the corpus is fully covered. The one documented
//! exception is [`BARE_TYPE_MANGLINGS`], which lists corpus entries that are
//! not symbol names at all. The floors below record the measured coverage.
//! Raise `MIN_EXACT` when coverage improves; never lower it. Setting
//! `CPP_TEXT_MISMATCHES` in the environment dumps the mismatches, the printer
//! refusals and the parse rejections (with the failing byte offset) to
//! `/tmp/cppexp/`.
#![cfg(feature = "cpp-itanium")]

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::string::String;
use std::vec::Vec;

use demangler::cpp;
use demangler::cpp::ParseErrorKind;

const CORPUS: &str = "refs/cpp/itanium/DemangleTestCases.inc";

/// Measured floor for byte-identical output over the whole corpus.
const MIN_EXACT: usize = 29_929;
/// The printer must not produce text that differs from the expected output.
const MAX_KNOWN_WRONG: usize = 0;
/// Every symbol that parses must also print.
const MAX_PRINT_FAILURES: usize = 0;

/// Corpus entries that are not symbol names, excluded on purpose.
///
/// The corpus also lists bare type manglings, under the comment "mangled names
/// can include type manglings too, which don't start with `_Z`". [`cpp::parse`]
/// demangles symbol names: it requires an encoding, and accepting a bare type
/// would make an ordinary C identifier such as `i` parse as C++.
///
/// The harness asserts that each entry below is still in the corpus and is
/// still rejected as [`ParseErrorKind::NotItanium`], and that nothing else
/// fails, so the exclusion cannot silently grow to cover new rejections.
const BARE_TYPE_MANGLINGS: &[&str] = &["i", "PKFvRiE", "PVFvRmOE", "PFvRmOE"];

#[test]
fn itanium_text_corpus() {
    let Ok(source) = std::fs::read_to_string(CORPUS) else {
        eprintln!("skipping Itanium text corpus: {CORPUS} not found");
        return;
    };
    let pairs = parse_corpus(&source).expect("corpus should be well formed");
    let total = pairs.len();
    let mut exact = 0_usize;
    let mut wrong = 0_usize;
    let mut error = 0_usize;
    let mut excluded = 0_usize;
    let mut parse_failures: BTreeMap<String, usize> = BTreeMap::new();
    let mut print_failures = 0_usize;
    let mut mismatches: Vec<String> = Vec::new();
    let mut failures: Vec<String> = Vec::new();
    let mut parse_list: Vec<String> = Vec::new();
    let dump = std::env::var_os("CPP_TEXT_MISMATCHES").is_some();

    for (mangled, expected) in &pairs {
        let ast = match cpp::parse(mangled) {
            Ok(ast) => ast,
            Err(parse_error) => {
                if BARE_TYPE_MANGLINGS.contains(&mangled.as_str()) {
                    assert_eq!(
                        parse_error.kind(),
                        ParseErrorKind::NotItanium,
                        "{mangled} is excluded as a bare type mangling"
                    );
                    excluded += 1;
                    continue;
                }
                error += 1;
                *parse_failures
                    .entry(format!("{:?}", parse_error.kind()))
                    .or_default() += 1;
                parse_list.push(format!(
                    "{:?}\t{}\t{}\t{}",
                    parse_error.kind(),
                    parse_error.position(),
                    mangled,
                    expected
                ));
                continue;
            }
        };
        let mut printed = String::new();
        if write!(&mut printed, "{ast}").is_err() {
            error += 1;
            print_failures += 1;
            if dump {
                failures.push(format!("{mangled}\n  want: {expected}"));
            }
            continue;
        }
        if printed == *expected {
            exact += 1;
            continue;
        }
        wrong += 1;
        if dump && mismatches.len() < 4000 {
            mismatches.push(format!("{mangled}\n  want: {expected}\n  got:  {printed}"));
        }
    }

    eprintln!(
        "cpp text corpus: total={total} exact={exact} wrong={wrong} error={error} \
         (parse failures: {parse_failures:?}, print failures: {print_failures}, \
         documented exclusions: {excluded})"
    );
    for mismatch in &mismatches {
        eprintln!("{mismatch}");
    }
    if dump {
        std::fs::write("/tmp/cppexp/mismatches.txt", mismatches.join("\n")).unwrap();
        std::fs::write("/tmp/cppexp/failures.txt", failures.join("\n")).unwrap();
        std::fs::write("/tmp/cppexp/parsefailures.txt", parse_list.join("\n")).unwrap();
    }

    assert_eq!(total, 29_933, "corpus pair count changed");
    assert_eq!(exact + wrong + error + excluded, total);
    assert_eq!(
        excluded,
        BARE_TYPE_MANGLINGS.len(),
        "a documented exclusion is missing from the corpus"
    );
    assert_eq!(error, 0, "undocumented parse failures: {parse_failures:?}");
    assert!(
        exact >= MIN_EXACT,
        "exact count {exact} is below the measured floor {MIN_EXACT}"
    );
    assert_eq!(
        wrong, MAX_KNOWN_WRONG,
        "the printer produced text that differs from the corpus"
    );
    assert_eq!(
        print_failures, MAX_PRINT_FAILURES,
        "a symbol parsed but the printer refused to render it"
    );
}

/// Parse the C++ array literal in `DemangleTestCases.inc`.
///
/// The file is C++ source: pairs may span lines, adjacent string literals are
/// concatenated, and `//` comments may appear between tokens. This mirrors the
/// parser in `tests/cpp_corpus.rs`.
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

/// Skip whitespace and `//` or `/* */` comments.
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
