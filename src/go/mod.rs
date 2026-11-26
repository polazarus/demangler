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
//! Untrusted input stays bounded by [`crate::go::MAX_INPUT_LEN`], [`crate::go::MAX_DEPTH`] and
//! [`crate::go::MAX_WORK`]. Exceeding a budget is an error, never a
//! partial result. Printed text is never longer than four times the input,
//! because every byte that is not valid UTF-8 costs four bytes as `\xNN`, so
//! the input budget also bounds printing.
//!
//! The input, depth and work limits are runtime-configurable. [`parse`](crate::go::parse)
//! keeps the compiled-in defaults, and a [`crate::Demangler`] passes the
//! configured budget, clamped to the hard ceilings of 16 MiB input, 1024
//! nesting levels and 64M work units. The three dimensions are independent:
//! raising the input limit alone keeps the default work allowance, so a caller
//! that raises the input for a long symbol must raise
//! [`crate::Budget::work`] as well when the scan exceeds it. A Go name is one
//! classified symbol, so there is no node dimension to configure; every
//! closure or wrapper suffix consumes input, so the input budget bounds the
//! suffix list.
//!
//! Printing is not a budget: the printer walks each node once, so the output is
//! linear in the parsed name and needs no cap of its own. The four-times-input
//! bound above is a property of the escaping, not a configurable limit.
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

use crate::error::BudgetKind;
use core::fmt;

use crate::budget::{EffectiveBudget, Limit};
use node::ParseLimits;

pub use node::{
    Abi, Ast, Constant, ConstantClass, DataName, Function, Local, MAX_DEPTH, MAX_INPUT_LEN,
    MAX_WORK, Name, Package, Receiver, Special, Suffix, Symbol, TypeDescriptor, TypeKind,
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

impl ParseError {
    /// The parser budget dimension this failure belongs to, when it is a
    /// capacity failure rather than a malformed symbol.
    #[must_use]
    pub const fn budget_kind(&self) -> Option<BudgetKind> {
        match self {
            Self::RecursedTooDeep => Some(BudgetKind::ParseDepth),
            Self::BudgetExceeded => Some(BudgetKind::Other),
            _ => None,
        }
    }
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
    parse_bytes_with_budget(symbol, DEFAULT_BUDGET)
}

/// The budget the public entry point starts from.
///
/// [`parse`] uses it, so it keeps exactly the compiled-in limits; a
/// [`crate::Demangler`] resolves its configured [`crate::Budget`] for
/// [`Scheme::Go`](crate::Scheme::Go) instead.
pub(crate) const DEFAULT_BUDGET: EffectiveBudget = EffectiveBudget {
    input_bytes: Some(Limit::Bounded(MAX_INPUT_LEN)),
    parse_depth: Some(Limit::Bounded(MAX_DEPTH)),
    work: Some(Limit::Bounded(MAX_WORK)),
    nodes: Some(Limit::Unbounded),
    expansion_visits: None,
    expansion_bytes: None,
};

/// Parse a Go symbol name with one scheme's resolved budget.
///
/// [`parse_bytes`] wraps this with [`DEFAULT_BUDGET`]; a [`crate::Demangler`]
/// passes [`Demangler::effective_budget`](crate::Demangler::effective_budget)
/// for [`Scheme::Go`](crate::Scheme::Go), so a caller's overrides reach the
/// parser. The input, depth and work values are clamped to this module's hard
/// ceilings, and the grammar has no node dimension to resolve.
pub(crate) fn parse_bytes_with_budget(
    symbol: &[u8],
    budget: EffectiveBudget,
) -> Result<Ast<'_>, ParseError> {
    parser::parse_bytes(symbol, ParseLimits::from_effective(budget))
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

    // A Go name is either explicitly prefixed (handled above) or carries a
    // package separator: `.` for dotted paths or `·` for assembly names.
    // Rejecting everything else here is the cheap prefilter for the common
    // plain-C symbol (`memcpy`, `_start`, ...).
    if !name.contains(&b'.') && !contains(name, MIDDLE_DOT) {
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
        && !(contains(name, b"go.shape.") || contains(name, b"..dict.") || local.contains(&b'['))
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
            "memcpy",
            "_start",
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

#[cfg(test)]
mod budget_tests {
    use alloc::format;
    use alloc::string::{String, ToString};

    use super::node::{HARD_DEPTH, HARD_INPUT_LEN, HARD_WORK, ParseLimits};
    use super::*;
    use crate::budget::Limit;
    use crate::{Budget, Demangler, Scheme};

    /// Parse `symbol` with `budget` resolved for [`Scheme::Go`].
    fn parse_with(symbol: &[u8], budget: Budget) -> Result<Ast<'_>, ParseError> {
        parse_bytes_with_budget(symbol, budget.effective(Scheme::Go))
    }

    /// Whether `symbol` parses with one budget dimension set to `value`.
    fn parses_with(symbol: &[u8], dimension: impl Fn(usize) -> Budget, value: usize) -> bool {
        parse_with(symbol, dimension(value)).is_ok()
    }

    /// Assert that `need` is the exact boundary of one dimension for `symbol`.
    ///
    /// The dimension is the only override, so a failure one unit below the
    /// boundary comes from that dimension and not from another limit.
    fn assert_dimension_boundary(
        symbol: &str,
        name: &str,
        ceiling: usize,
        dimension: impl Fn(usize) -> Budget,
    ) {
        let bytes = symbol.as_bytes();
        let need = (0..=ceiling)
            .find(|value| parses_with(bytes, &dimension, *value))
            .unwrap_or_else(|| panic!("{symbol}: no {name} budget parses"));
        assert!(need > 0, "{symbol}: the {name} boundary is zero");
        assert!(
            parses_with(bytes, &dimension, need),
            "{symbol}: {name} budget {need} must accept"
        );
        assert!(
            !parses_with(bytes, &dimension, need - 1),
            "{symbol}: {name} budget {} must reject",
            need - 1
        );
    }

    #[test]
    fn the_compiled_in_budget_matches_the_policy_defaults() {
        let limits = ParseLimits::from_effective(DEFAULT_BUDGET);
        assert_eq!(limits.input, MAX_INPUT_LEN);
        assert_eq!(limits.depth, MAX_DEPTH);
        assert_eq!(limits.work, MAX_WORK);
        assert_eq!(
            Scheme::Go.default_budget().effective(Scheme::Go),
            DEFAULT_BUDGET
        );

        // The policy table and this module agree on the ceilings, and a Go name
        // is one classified symbol, so there is no node dimension to raise.
        let hard = Scheme::Go.hard_budget();
        assert_eq!(hard.input_bytes, Some(Limit::Bounded(HARD_INPUT_LEN)));
        assert_eq!(hard.parse_depth, Some(Limit::Bounded(HARD_DEPTH)));
        assert_eq!(hard.work, Some(Limit::Bounded(HARD_WORK)));
        assert_eq!(hard.nodes, Some(Limit::Unbounded));
    }

    #[test]
    fn each_dimension_rejects_one_unit_below_what_the_symbol_needs() {
        for symbol in [
            "fmt.Println",
            "runtime.memmove.abi0",
            "type:.eq.main.Pair[string,int]",
            "pkg.F[go.shape.int]",
        ] {
            let ceiling = symbol.len() * 2 + 64;
            assert_dimension_boundary(symbol, "input", ceiling, |value| {
                Budget::new().with_input_bytes(value)
            });
        }

        // Only a construct the parser scans charges work: a balanced group, a
        // receiver, a suffix chain or an `itab` payload.
        for symbol in [
            "pkg.F[go.shape.int]",
            "runtime.(*mLockProfile).captureStack",
            "pkg.F.func1",
            "go:itab.*os.File,io.Writer",
        ] {
            let ceiling = symbol.len() * 2 + 64;
            assert_dimension_boundary(symbol, "work", ceiling, |value| {
                Budget::new().with_work(value)
            });
        }

        // A receiver or generic argument list enters the depth dimension once
        // per balanced group.
        assert_dimension_boundary("pkg.F[go.shape.int]", "depth", 8, |value| {
            Budget::new().with_parse_depth(value)
        });
    }

    #[test]
    fn raising_the_input_budget_accepts_a_longer_symbol() {
        let mut symbol = String::from("pkg.");
        symbol.push_str(&"a".repeat(MAX_INPUT_LEN));
        assert!(matches!(
            parse_with(symbol.as_bytes(), Budget::new()),
            Err(ParseError::BudgetExceeded)
        ));
        let raised = Budget::new().with_input_bytes(2 * MAX_INPUT_LEN);
        let ast = parse_with(symbol.as_bytes(), raised).expect("raised input budget");
        assert_eq!(ast.to_string().len(), symbol.len());
    }

    #[test]
    fn raising_the_depth_budget_accepts_a_deeper_group() {
        let mut symbol = String::from("pkg.F");
        symbol.push_str(&"[".repeat(MAX_DEPTH + 8));
        symbol.push_str(&"]".repeat(MAX_DEPTH + 8));
        assert!(matches!(
            parse_with(symbol.as_bytes(), Budget::new()),
            Err(ParseError::RecursedTooDeep)
        ));
        let raised = Budget::new().with_parse_depth(MAX_DEPTH + 8);
        assert!(parse_with(symbol.as_bytes(), raised).is_ok());
    }

    /// The input and work dimensions are independent: raising the input alone
    /// keeps the default work allowance, so a scan that exceeds it still fails.
    #[test]
    fn raising_the_input_budget_does_not_raise_the_work_budget() {
        // A balanced group is scanned byte by byte, so this name needs more
        // work than the default allowance while staying under the raised input
        // cap.
        let symbol = format!("pkg.F[{}]", "a".repeat(600_000));
        assert!(symbol.len() > MAX_WORK);
        let work = 2 * symbol.len();

        let input_only = Budget::new().with_input_bytes(HARD_INPUT_LEN);
        assert!(matches!(
            parse_with(symbol.as_bytes(), input_only),
            Err(ParseError::BudgetExceeded)
        ));

        assert!(parse_with(symbol.as_bytes(), input_only.with_work(work)).is_ok());
    }

    #[test]
    fn clamping_keeps_the_hard_ceilings() {
        let budget = Budget::new()
            .with_input_bytes(usize::MAX)
            .with_parse_depth(usize::MAX)
            .with_work(usize::MAX);
        let limits = ParseLimits::from_effective(budget.effective(Scheme::Go));
        assert_eq!(limits.input, HARD_INPUT_LEN);
        assert_eq!(limits.depth, HARD_DEPTH);
        assert_eq!(limits.work, HARD_WORK);
        assert_eq!(
            budget.effective(Scheme::Go).input_bytes,
            Some(Limit::Bounded(HARD_INPUT_LEN))
        );
        assert_eq!(
            budget.effective(Scheme::Go).parse_depth,
            Some(Limit::Bounded(HARD_DEPTH))
        );

        // A name past the hard input cap stays rejected even when the caller
        // asks for more.
        let mut too_long = String::from("pkg.");
        too_long.push_str(&"a".repeat(HARD_INPUT_LEN));
        assert!(matches!(
            parse_with(too_long.as_bytes(), budget),
            Err(ParseError::BudgetExceeded)
        ));
    }

    #[test]
    fn a_zero_budget_rejects_through_every_entry_point() {
        // A balanced group charges work and enters depth, so every dimension
        // rejects this symbol.
        let symbol = "pkg.F[go.shape.int]";
        assert!(Demangler::new().demangle(symbol).is_ok());
        for budget in [
            Budget::new().with_input_bytes(0),
            Budget::new().with_work(0),
        ] {
            let demangler = Demangler::with_budget(budget);
            assert!(demangler.demangle(symbol).is_err(), "{budget:?}");
            assert!(
                demangler.demangle_with_scheme(Scheme::Go, symbol).is_err(),
                "{budget:?}"
            );
            assert!(
                demangler
                    .selection()
                    .allow(Scheme::Go)
                    .demangle(symbol)
                    .is_err(),
                "{budget:?}"
            );
        }

        // The depth dimension only rejects a name that enters a group.
        let demangler = Demangler::with_budget(Budget::new().with_parse_depth(0));
        assert!(demangler.demangle(symbol).is_err());
        assert!(demangler.demangle("fmt.Println").is_ok());
    }

    /// Printing is not a budget: the printer walks each node once and has no
    /// writer cap, so the output is linear in the parsed name.
    ///
    /// A byte that is not valid UTF-8 prints as `\xNN`, the widest expansion
    /// the printer has, so four times the input bounds the text for any name
    /// the parser accepts, including one accepted under a raised input budget.
    #[test]
    fn printing_stays_within_four_times_the_input() {
        let mut symbol = b"pkg.F[".to_vec();
        symbol.resize(MAX_INPUT_LEN, 0xff);
        symbol.push(b']');
        let raised = Budget::new().with_input_bytes(2 * MAX_INPUT_LEN);
        let rendered = parse_with(&symbol, raised)
            .expect("the raised input budget parses")
            .to_string();
        assert!(rendered.len() <= 4 * symbol.len());
    }

    #[test]
    fn the_default_budget_parses_the_corpus_like_parse() {
        let corpora = [
            "/refs/go/corpus/go1.24.4-binary.txt",
            "/refs/go/corpus/go1.24.4-object.txt",
            "/refs/go/corpus/go1.24.4-archive.txt",
        ];
        let mut compared = 0_usize;
        for corpus in corpora {
            let path = std::format!("{}{corpus}", std::env!("CARGO_MANIFEST_DIR"));
            let Ok(data) = std::fs::read_to_string(&path) else {
                std::eprintln!("go corpus not present (published crate); skipping");
                return;
            };
            for line in data.lines() {
                let symbol = line.split('\t').next().unwrap_or_default();
                if symbol.is_empty() || symbol.starts_with('#') {
                    continue;
                }
                let expected = parse(symbol);
                let explicit = parse_with(symbol.as_bytes(), Budget::new());
                assert_eq!(
                    expected.is_ok(),
                    explicit.is_ok(),
                    "{symbol}: the default budget changes acceptance"
                );
                let (Ok(expected), Ok(explicit)) = (expected, explicit) else {
                    continue;
                };
                assert_eq!(expected.to_string(), explicit.to_string(), "{symbol}");
                compared += 1;
            }
        }
        assert!(compared > 0, "the default budget compared no symbols");
    }
}
