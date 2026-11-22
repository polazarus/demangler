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
//! # Budgets
//!
//! Untrusted input stays bounded by [`crate::go::MAX_INPUT_LEN`], [`crate::go::MAX_DEPTH`],
//! [`crate::go::MAX_WORK`] and [`crate::go::MAX_SUFFIXES`]. Exceeding a budget is an error, never a
//! partial result. The printed output is never longer than the input, so the
//! input budget also bounds printing.
//!
//! # Gaps
//!
//! - `.llvm.` suffixes are not Go and are left to the other backends.
//! - `%XX` escapes that do not decode to valid UTF-8 are printed verbatim.
//! - ABI markers are reported by [`crate::go::Ast::abi`] but not printed.
//! - Bare assembly and cgo names (`aeshashbody`, `_cgo_init`) carry no package
//!   and are indistinguishable from C, so detection leaves them out of Go.
//!
mod node;
mod parser;
mod printer;

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
    parser::parse(symbol)
}

/// True when `name` has the shape of a Go linker symbol name.
///
/// This is the cheap prefilter used by [`crate::Scheme::detect`]; it validates
/// the shape but not every name form, so [`parse`] may still reject a name
/// that this accepts.
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
pub fn is_go_name(name: &str) -> bool {
    if name.is_empty() || name.len() > MAX_INPUT_LEN {
        return false;
    }
    if name.starts_with("type:") || name.starts_with("go:") || name.starts_with("go.shape.") {
        return true;
    }
    if name.starts_with("$f32.")
        || name.starts_with("$f64.")
        || name.starts_with("$i32.")
        || name.starts_with("$i64.")
        || name.starts_with("$i128.")
    {
        return true;
    }
    if name.starts_with('?')
        || name.starts_with('$')
        || name.starts_with("_Z")
        || name.starts_with("_RN")
        || name.contains(".llvm.")
    {
        return false;
    }
    if name.bytes().any(|byte| byte.is_ascii_control()) {
        return false;
    }

    // `·` is Go's identifier separator, used by assembly and by the compiler
    // for `gclocals·<hash>` and `pkg.func·f`.
    if name.contains('·') && name.bytes().any(|byte| byte.is_ascii_alphanumeric()) {
        return true;
    }

    let (package, local) = parser::split_package(name);
    if package.is_empty() || local.is_empty() {
        return false;
    }

    // Type strings inside generic arguments and dictionaries can contain
    // spaces (`interface {}`, `struct { x int }`); nothing else can.
    if name.chars().any(char::is_whitespace)
        && !(name.contains("go.shape.") || name.contains("..dict.") || local.contains('['))
    {
        return false;
    }
    if !matches!(
        local.as_bytes().first(),
        Some(b'A'..=b'Z' | b'a'..=b'z' | b'_' | b'(' | b'[' | b'.')
    ) {
        return false;
    }

    // A `(` is either the receiver opener at the start of the local part or
    // part of a bracketed type string such as `F[go.shape.func()]`. Anything
    // else (`foo.bar(baz)`) is not a Go name.
    let mut depth = 0_usize;
    for (index, byte) in local.bytes().enumerate() {
        match byte {
            b'(' if depth == 0 && index != 0 && local.as_bytes()[index - 1] != b'.' => {
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
            assert!(is_go_name(name), "{name}");
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
            assert!(!is_go_name(name), "{name}");
        }
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
                let _ = is_go_name(text);
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
            Name::Shape("int")
        ));
        assert!(matches!(
            parse("main..stmp_3").unwrap().name(),
            Name::Symbol(Symbol {
                local: Local::StaticTemp { index: "3" },
                ..
            })
        ));
    }
}
