//! Pure-Rust Go symbol name demangling.
//!
//! Go does not mangle names in the C++ sense: the compiler emits
//! `package.Local` names and decorates them with compiler-generated
//! components. This module classifies those names, decodes the `%XX` package
//! path escapes that `cmd/internal/objabi.PathToPrefix` introduces, normalises
//! the `·` separator to `.`, and strips ABI markers (`.abi0`, `.abiinternal`,
//! `.abiwrap`), which are linker plumbing rather than part of the name.
//!
//! No external demangler is used and there is no runtime dependency.
//!
//! # Supported forms
//!
//! - package/function/method names: `fmt.Println`, `fmt.(*pp).doPrintln`,
//!   including methods promoted from another package
//!   (`(*Template).text/template/parse.action`);
//! - closures and wrappers: `.funcN`, `-rangeN`, `.gowrapN`, `.deferwrapN`,
//!   `-fm`, `·f`, nested closure indices, inlined function names and inlined
//!   methods such as `(*mheap).freeSpan`;
//! - generic instantiations and compiler shape types: `Sum[go.shape.int]`,
//!   `go.shape.struct { … }`, `..dict.T[int]`;
//! - runtime type descriptors: `type:`, `type:*`, `type:.eq.`, `type:.eqfunc`,
//!   `type:.hash.`, `type:.hashfunc`, `type:.gcprog.`, `type:.namedata.`,
//!   `type:.importpath.`, `type:noalg.`;
//! - linker specials: `go:itab.<T>,<I>`, `go:string.<payload>`, `go:info.<T>`,
//!   `go:buildid`, `go:func.*` and the other `go:` group symbols;
//! - compiler-generated data symbols: `..inittask`, `..interfaceSwitch.N`,
//!   `..typeAssert.N`, `..gobytes.N`, `..stmp_N`, `glob..funcN`,
//!   `gcbits.<hex>`, `gclocals.<base64>`;
//! - compiler-generated function suffixes: `.jumpN` jump tables, `.arginfoN`,
//!   `.stkobj` and the other internal suffixes;
//! - compiler constants: `$f32.`, `$f64.`, `$i32.`, `$i64.`, `$i128.`.
//!
//! Object files and export archives also carry a `<N>` symbol version that
//! `go tool nm` appends to static symbols; it is kept in the printed name and
//! exposed as [`crate::go::Ast::version`].
//!
//! # Normalisation
//!
//! The printed form is the name as a Go programmer writes it: the package
//! path is unescaped and `·` becomes `.`. Unescaping is lossy for one corner:
//! `%2e` inside the last package path segment becomes `.`, which is
//! indistinguishable from the package/local separator, so the printed text is
//! meant for humans rather than for re-parsing. [`crate::go::Package::decoded`] exposes
//! the same decoding structurally.
//!
//! `reflect.name` blobs (`type:.namedata.`) and `go:string.` payloads are
//! opaque: struct tags and string data may contain `%` and arbitrary bytes, so
//! they are printed verbatim.
//!
//! # Raw bytes
//!
//! A symbol name is a byte string, so the parser matches the grammar on bytes
//! and never requires the whole name to be UTF-8. Text that is valid UTF-8
//! prints verbatim (so `·` and Unicode type strings survive); bytes that are
//! not valid UTF-8 print escaped as `\xNN`, including bytes produced by a
//! `%XX` escape. A backslash in identifier text prints as `\\` so the
//! spelling stays reversible.
//!
//! # Budgets
//!
//! Untrusted input stays bounded by [`crate::go::MAX_INPUT_LEN`], [`crate::go::MAX_DEPTH`],
//! [`crate::go::MAX_WORK`] and [`crate::go::MAX_SUFFIXES`]. Exceeding a budget is an error, never a
//! partial result. Printed text is never longer than four times the input,
//! because every byte that is not valid UTF-8 costs four bytes as `\xNN`, so
//! the input budget also bounds printing.
//!
//! # Gaps
//!
//! - `.llvm.` suffixes are not Go and are left to the other backends.
//! - `%XX` escapes that do not decode to valid UTF-8 print as `\xNN` escapes.
//! - ABI markers are reported by [`crate::go::Ast::abi`] but not printed.
//! - Bare assembly and cgo names (`aeshashbody`, `_cgo_init`) carry no package
//!   and are indistinguishable from C, so detection leaves them out of Go.
//!
mod node;
mod parser;
mod printer;
mod text;

use core::fmt;

pub use node::{
    Abi, Ast, Constant, ConstantClass, DataName, Function, Local, MAX_DEPTH, MAX_INPUT_LEN,
    MAX_SUFFIXES, MAX_WORK, Name, Package, Receiver, Special, Suffix, Symbol, TypeDescriptor,
    TypeKind,
};

/// Why a symbol could not be parsed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ParseError {
    /// The symbol does not match a known Go name form.
    Invalid,
    /// The symbol nests `()`, `[]` or `{}` deeper than [`crate::go::MAX_DEPTH`].
    RecursedTooDeep,
    /// The symbol exceeds the input, work or suffix budget.
    BudgetExceeded,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Invalid => "invalid syntax",
            Self::RecursedTooDeep => "recursion limit reached",
            Self::BudgetExceeded => "size limit reached",
        })
    }
}

impl core::error::Error for ParseError {}

impl fmt::Display for Ast<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.print(f)
    }
}

impl Ast<'_> {
    /// Render the parsed name into `out`.
    pub(crate) fn print<W: fmt::Write + ?Sized>(&self, out: &mut W) -> fmt::Result {
        printer::print(&self.name, out)?;
        printer::print_version(self.version, out)
    }
}

/// Parse a Go symbol name into a reusable [`Ast`].
///
/// # Errors
///
/// Returns [`ParseError::Invalid`] when the symbol matches no known Go name
/// form, [`ParseError::RecursedTooDeep`] when a receiver or generic argument
/// list nests too deeply, and [`ParseError::BudgetExceeded`] when the input,
/// work or suffix budget is exceeded.
pub fn parse(symbol: &str) -> Result<Ast<'_>, ParseError> {
    parse_bytes(symbol.as_bytes())
}

/// Parse a Go symbol name from raw bytes.
///
/// This is the entry point the other backends and the demangler dispatch to
/// once symbol bytes, not text, are the input. See [`parse`] for the accepted
/// forms; the only difference is that the name does not have to be UTF-8.
pub(crate) fn parse_bytes(symbol: &[u8]) -> Result<Ast<'_>, ParseError> {
    parser::parse_bytes(symbol)
}

/// True when `name` has the shape of a Go linker symbol name.
///
/// This is the cheap prefilter used by [`crate::Scheme::detect`]; it validates
/// the shape but not every name form, so [`parse`] may still reject a name
/// that this accepts.
///
/// Detection is a byte test: only ASCII whitespace and control bytes are
/// rejected directly, so a valid Unicode identifier is never refused merely
/// because a byte is `>= 0x80`.
///
/// The rule is:
///
/// - explicit prefixes `type:`, `go:` and `go.shape.` are Go;
/// - `$f32.`/`$f64.`/`$i32.`/`$i64.`/`$i128.` compiler constants are Go;
/// - names containing `·`, Go's identifier separator, are Go;
/// - otherwise the name must be dotted: the part before the package separator
///   must be a non-empty escaped import path and the local part must start
///   with an identifier, `(`, `[` or `.`;
/// - whitespace is only allowed in type strings (`go.shape.`, `..dict.`, or
///   inside a generic argument list);
/// - a `(` is only valid as a receiver opener (at the start of the local part
///   or after a `.`) or inside a bracketed type string;
/// - names handled by another scheme (`?`, `_Z`, `_RN`, `$`, `.llvm.`) are
///   never Go.
#[must_use]
pub fn is_go(name: &str) -> bool {
    is_go_bytes(name.as_bytes())
}

/// True when `name` has the shape of a Go linker symbol name, as raw bytes.
///
/// Detection is a byte test: only ASCII whitespace and control bytes are
/// rejected directly, so a valid Unicode identifier is never refused merely
/// because a byte is `>= 0x80`.
pub(crate) fn is_go_bytes(name: &[u8]) -> bool {
    /// The UTF-8 encoding of `·`, Go's identifier separator.
    const MIDDLE_DOT: &[u8] = "·".as_bytes();

    if name.is_empty() || name.len() > MAX_INPUT_LEN {
        return false;
    }
    if name.starts_with(b"type:") || name.starts_with(b"go:") || name.starts_with(b"go.shape.") {
        return true;
    }
    if name.starts_with(b"$f32.")
        || name.starts_with(b"$f64.")
        || name.starts_with(b"$i32.")
        || name.starts_with(b"$i64.")
        || name.starts_with(b"$i128.")
    {
        return true;
    }
    if name.starts_with(b"?")
        || name.starts_with(b"$")
        || name.starts_with(b"_Z")
        || name.starts_with(b"_RN")
        || contains(name, b".llvm.")
    {
        return false;
    }
    if name.iter().any(u8::is_ascii_control) {
        return false;
    }

    // `·` is Go's identifier separator, used by assembly and by the compiler
    // for `gclocals·<hash>` and `pkg.func·f`.
    if contains(name, MIDDLE_DOT) && name.iter().any(u8::is_ascii_alphanumeric) {
        return true;
    }

    let (package, local) = parser::split_package(name);
    if package.is_empty() || local.is_empty() {
        return false;
    }

    // Type strings inside generic arguments and dictionaries can contain
    // spaces (`interface {}`, `struct { x int }`); nothing else can. Only ASCII
    // whitespace is a separator here, so Unicode identifiers pass through.
    if name.iter().any(u8::is_ascii_whitespace)
        && !(contains(name, b"go.shape.")
            || contains(name, b"..dict.")
            || local.contains(&b'['))
    {
        return false;
    }
    if !matches!(
        local.first(),
        Some(b'A'..=b'Z' | b'a'..=b'z' | b'_' | b'(' | b'[' | b'.')
    ) {
        return false;
    }

    // A `(` is either the receiver opener at the start of the local part or
    // part of a bracketed type string such as `F[go.shape.func()]`. Anything
    // else (`foo.bar(baz)`) is not a Go name.
    let mut depth = 0_usize;
    for (index, byte) in local.iter().enumerate() {
        match byte {
            b'(' if depth == 0 && index != 0 && local[index - 1] != b'.' => {
                return false;
            }
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => {
                if depth == 0 {
                    return false;
                }
                depth -= 1;
            }
            _ => {}
        }
    }
    true
}

/// True when `haystack` contains `needle`.
fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    crate::parse::find(haystack, needle).is_some()
}

#[cfg(test)]
mod tests {
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
}
