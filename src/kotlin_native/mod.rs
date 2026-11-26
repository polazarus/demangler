//! Pure-Rust Kotlin/Native symbol demangling.
//!
//! Kotlin/Native is an LLVM backend with its own linker-symbol mangling,
//! unrelated to Kotlin/JVM descriptors and Kotlin/JS names. The compiler's
//! `KonanMangler` builds a declaration spelling from a qualified path, a `#`
//! member separator, value and type parameter lists, a return type and linkage
//! markers; the LLVM backend wraps it in a `k*:` prefix.
//!
//! No external demangler is used and there is no runtime dependency.
//!
//! # Supported forms
//!
//! - `kfun:` functions, constructors, property accessors, local functions,
//!   lambdas and compiler bridges;
//! - `ktype:` type symbols;
//! - `kclass:` classes, interfaces and objects;
//! - `kproperty:`, `kobj:`, `kfield:`, `ktypealias:`, `kvar:`;
//! - `kpackage:` and `knamespace:` package/namespace symbols;
//! - Mach-O leading-underscore variants (`_kfun:`, `__kfun:`);
//! - extension receivers (`@` in klib spellings, `__at__` in ELF symbols),
//!   `#static`, `#suspend`, `#companion`, `#internal`, `-impl`, `$` local and
//!   synthetic indexes, and `%` platform-specific suffixes;
//! - type parameters, bounds, type arguments, variance, nullable, star,
//!   vararg, flexible, `<dynamic>` and `<ERROR CLASS>` types.
//!
//! # Canonical output
//!
//! There is no standard Kotlin/Native demangler, so the printed text is this
//! crate's contract:
//!
//! | Input | Output |
//! |---|---|
//! | `kfun:com.example.foo#bar(kotlin.Int){}` | `com.example.foo.bar(kotlin.Int)` |
//! | `kfun:com.example.Box#<init>(kotlin.Int){}` | `com.example.Box.<init>(kotlin.Int)` |
//! | `kfun:com.example.Box#get(){}` | `com.example.Box.get()` |
//! | `kfun:com.example#topLevel(){}` | `com.example.topLevel()` |
//! | `ktype:com.example.Box<kotlin.Int>` | `com.example.Box<kotlin.Int>` |
//!
//! The spelling is a debugger name, not re-parsable Kotlin source:
//!
//! - `#` becomes `.`;
//! - `#static` and `#suspend` become `static ` and `suspend ` prefixes;
//! - `#internal` and `-impl` are linkage markers and are not printed;
//! - an extension receiver is printed as `name@receiver(...)`;
//! - type parameters are declared as `T<index>` and referenced as
//!   `T<container>:<index>`;
//! - variance is printed as `out ` / `in `, vararg as `vararg `, nullable as
//!   `?`, star as `*` and flexible types as `lower..upper`;
//! - a return type is appended as `: <type>`;
//! - a `%` platform suffix is appended as ` %<suffix>`.
//!
//! Unknown prefixes and unknown `#` markers are rejected, never guessed.
//!
//! # Raw bytes
//!
//! A symbol name is a byte string, so the parser matches the grammar on bytes
//! and never requires the whole name to be UTF-8. Text that is valid UTF-8
//! prints verbatim (so Unicode identifiers survive); bytes that are not valid
//! UTF-8 print escaped as `\xNN`. A backslash in a name prints as `\\`, and a
//! `%` platform suffix keeps its payload as written.
//!
//! # Detection
//!
//! [`crate::Scheme::detect`] claims only the known `k*:` prefixes (and their
//! leading-underscore variants) and tries them before Go and plain C. It does
//! not claim arbitrary `k...` names, and C++/Rust/Swift/Objective-C keep their
//! existing precedence.
//!
//! # Budgets
//!
//! Untrusted input stays bounded by [`MAX_INPUT_LEN`](crate::kotlin_native::MAX_INPUT_LEN), [`MAX_DEPTH`](crate::kotlin_native::MAX_DEPTH),
//! [`MAX_WORK`](crate::kotlin_native::MAX_WORK), [`MAX_PATH_SEGMENTS`](crate::kotlin_native::MAX_PATH_SEGMENTS), [`MAX_PARAMETERS`](crate::kotlin_native::MAX_PARAMETERS),
//! [`MAX_TYPE_ARGUMENTS`](crate::kotlin_native::MAX_TYPE_ARGUMENTS), [`MAX_TYPE_PARAMETERS`](crate::kotlin_native::MAX_TYPE_PARAMETERS) and [`MAX_OUTPUT_LEN`](crate::kotlin_native::MAX_OUTPUT_LEN).
//! Exceeding a budget is an error, never a partial result.
//!
//! The input, depth and work limits are runtime-configurable.
//! [`parse`](crate::kotlin_native::parse) keeps the compiled-in defaults, and a
//! [`crate::Demangler`] passes the configured budget, clamped to the hard
//! ceilings of 16 MiB input, 1024 nesting levels and 64M work units. Input and
//! work are independent: raising the input limit alone keeps the default work
//! allowance, so a caller that raises the input for a long symbol must raise
//! [`crate::Budget::work`] as well when the scan exceeds it. The parser builds
//! one symbol with its own bounded lists, so there is no node dimension to
//! configure, and the segment, parameter, type-argument and type-parameter caps
//! are fixed backend extras.
//!
//! Printing is not a budget: the printer walks each node once, so the output is
//! linear in the parsed symbol and [`MAX_OUTPUT_LEN`](crate::kotlin_native::MAX_OUTPUT_LEN)
//! is a fixed backstop sized for the hard input ceiling. No accepted parse can
//! exceed it.
//!
//! # Gaps
//!
//! - Kotlin/JVM descriptors, Kotlin/JS names and Kotlin metadata blobs are out
//!   of scope.
//! - Kotlin/Native internal table symbols (`kintf:`, `kifacetable:`,
//!   `kifacevtable:`, `kexttype:`, `kextoff:`, `kextname:`, `krefs:`) are not
//!   declarations and are left unclaimed.
//! - The `%` platform suffix is preserved verbatim; its ObjC-export payload is
//!   not decoded.
//! - `ktype:`, `kproperty:`, `kobj:`, `ktypealias:`, `kpackage:` and
//!   `knamespace:` are implemented from the mangler's marker constants; Kotlin
//!   2.4.21 executables emit only `kfun:`, `kclass:` and `kvar:` symbols, so
//!   those prefixes are covered by hand-authored compatibility cases rather
//!   than compiler output.
mod node;
mod parser;
mod printer;
mod text;

use crate::error::BudgetKind;
use core::fmt;

use crate::budget::{EffectiveBudget, Limit};
use crate::parse::LimitedWriter;
use node::ParseLimits;
pub use node::{
    Ast, Function, MAX_DEPTH, MAX_INPUT_LEN, MAX_OUTPUT_LEN, MAX_PARAMETERS, MAX_PATH_SEGMENTS,
    MAX_TYPE_ARGUMENTS, MAX_TYPE_PARAMETERS, MAX_WORK, Symbol, Type, TypeArgument, TypeParameter,
    Variance,
};
pub use parser::is_kotlin_native;
pub(crate) use parser::is_kotlin_native_bytes;

/// Why a Kotlin/Native symbol could not be parsed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ParseError {
    /// The symbol does not match a known Kotlin/Native form.
    Invalid,
    /// A type nests deeper than [`MAX_DEPTH`].
    RecursedTooDeep,
    /// The input, work or list budget was exceeded.
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
    /// Render the parsed symbol into `out`, enforcing [`MAX_OUTPUT_LEN`](crate::kotlin_native::MAX_OUTPUT_LEN).
    pub(crate) fn print<W: fmt::Write + ?Sized>(&self, out: &mut W) -> fmt::Result {
        let mut limited = LimitedWriter::new(out, MAX_OUTPUT_LEN);
        printer::print(&self.symbol, &mut limited)
    }
}

/// Parse a Kotlin/Native symbol into a reusable [`Ast`].
///
/// A Mach-O leading underscore (`_kfun:`) and a double underscore
/// (`__kfun:`) are accepted, as are klib `@` and ELF `__at__` extension
/// markers.
///
/// # Errors
///
/// Returns [`ParseError::Invalid`] when the prefix is unknown or a marker,
/// separator, list or type is malformed, [`ParseError::RecursedTooDeep`] when
/// a type nests deeper than [`MAX_DEPTH`], and
/// [`ParseError::BudgetExceeded`] when the input, work or list budget is
/// exceeded.
pub fn parse(symbol: &str) -> Result<Ast<'_>, ParseError> {
    parse_bytes(symbol.as_bytes())
}

/// Parse a Kotlin/Native symbol from raw bytes.
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
/// [`Scheme::KotlinNative`](crate::Scheme::KotlinNative) instead.
pub(crate) const DEFAULT_BUDGET: EffectiveBudget = EffectiveBudget {
    input_bytes: Some(Limit::Bounded(MAX_INPUT_LEN)),
    parse_depth: Some(Limit::Bounded(MAX_DEPTH)),
    work: Some(Limit::Bounded(MAX_WORK)),
    nodes: Some(Limit::Unbounded),
};

/// Parse a Kotlin/Native symbol with one scheme's resolved budget.
///
/// [`parse_bytes`] wraps this with [`DEFAULT_BUDGET`]; a [`crate::Demangler`]
/// passes [`Demangler::effective_budget`](crate::Demangler::effective_budget)
/// for [`Scheme::KotlinNative`](crate::Scheme::KotlinNative), so a caller's
/// overrides reach the parser. The input, depth and work values are clamped to
/// this module's hard ceilings, and the grammar has no node dimension to
/// resolve.
pub(crate) fn parse_bytes_with_budget(
    symbol: &[u8],
    budget: EffectiveBudget,
) -> Result<Ast<'_>, ParseError> {
    parser::parse_bytes(symbol, ParseLimits::from_effective(budget))
}

#[cfg(test)]
mod tests {
    use alloc::string::ToString;

    use super::*;

    #[test]
    fn parse_is_display() {
        assert_eq!(
            parse("kfun:com.example#main(){}").unwrap().to_string(),
            "com.example.main()"
        );
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
        let ast = parse("kfun:com.example.Box#get(){}kotlin.Int").unwrap();
        let Symbol::Function(function) = ast.symbol() else {
            panic!("not a function");
        };
        assert_eq!(function.path(), "com.example.Box");
        assert_eq!(function.name(), "get");
        assert!(function.params().is_empty());
        let Some(Type::Class { name, .. }) = function.return_type() else {
            panic!("not a class type");
        };
        assert_eq!(name, b"kotlin.Int");
    }

    #[test]
    fn depth_budget_reports_recursion() {
        // The shared cursor's depth budget is the only path to
        // `RecursedTooDeep` in this backend; nesting past `MAX_DEPTH` must not
        // fall through to a generic budget error.
        let mut symbol = alloc::string::String::from("ktype:kotlin.Array<");
        symbol.push_str(&"kotlin.Array<".repeat(MAX_DEPTH + 1));
        symbol.push('*');
        symbol.push_str(&">".repeat(MAX_DEPTH + 1));
        assert_eq!(parse(&symbol).unwrap_err(), ParseError::RecursedTooDeep);
    }

    #[test]
    fn printer_is_bounded() {
        // Every accepted input renders; the ceiling is enforced separately.
        let ast = parse("kfun:com.example#foo(kotlin.Int){}").unwrap();
        let mut out = alloc::string::String::new();
        ast.print(&mut out).unwrap();
        assert!(out.len() <= MAX_OUTPUT_LEN);
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

    /// Parse `symbol` with `budget` resolved for [`Scheme::KotlinNative`].
    fn parse_with(symbol: &[u8], budget: Budget) -> Result<Ast<'_>, ParseError> {
        parse_bytes_with_budget(symbol, budget.effective(Scheme::KotlinNative))
    }

    /// Whether `symbol` parses with one budget dimension set to `value`.
    fn parses_with(symbol: &[u8], dimension: impl Fn(u32) -> Budget, value: u32) -> bool {
        parse_with(symbol, dimension(value)).is_ok()
    }

    /// Assert that `need` is the exact boundary of one dimension for `symbol`.
    ///
    /// The dimension is the only override, so a failure one unit below the
    /// boundary comes from that dimension and not from another limit.
    fn assert_dimension_boundary(
        symbol: &str,
        name: &str,
        ceiling: u32,
        dimension: impl Fn(u32) -> Budget,
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
            Scheme::KotlinNative
                .default_budget()
                .effective(Scheme::KotlinNative),
            DEFAULT_BUDGET
        );

        // The policy table and this module agree on the ceilings. The parser
        // builds one symbol with its own bounded lists, so there is no node
        // dimension to raise.
        let hard = Scheme::KotlinNative.hard_budget();
        assert_eq!(hard.input_bytes, Some(Limit::Bounded(HARD_INPUT_LEN)));
        assert_eq!(hard.parse_depth, Some(Limit::Bounded(HARD_DEPTH)));
        assert_eq!(hard.work, Some(Limit::Bounded(HARD_WORK)));
        assert_eq!(hard.nodes, Some(Limit::Unbounded));
    }

    #[test]
    fn each_dimension_rejects_one_unit_below_what_the_symbol_needs() {
        for symbol in [
            "kfun:com.example#main(){}",
            "ktype:kotlin.Array<kotlin.Int>",
            "kclass:com.example.Box#internal",
            "kfun:com.example.Box#get(){}kotlin.Int",
        ] {
            let ceiling = u32::try_from(symbol.len()).unwrap_or(u32::MAX) * 2 + 64;
            assert_dimension_boundary(symbol, "input", ceiling, |value| {
                Budget::new().with_input_bytes(value as usize)
            });
            assert_dimension_boundary(symbol, "work", ceiling, |value| {
                Budget::new().with_work(value)
            });
        }

        // Every parsed type enters one depth level, so a one-argument type
        // needs two.
        assert_dimension_boundary("ktype:kotlin.Array<kotlin.Int>", "depth", 8, |value| {
            Budget::new().with_parse_depth(value)
        });
    }

    #[test]
    fn raising_the_input_budget_accepts_a_longer_symbol() {
        let mut symbol = String::from("kclass:");
        symbol.push_str(&"a".repeat(MAX_INPUT_LEN));
        assert!(matches!(
            parse_with(symbol.as_bytes(), Budget::new()),
            Err(ParseError::BudgetExceeded)
        ));
        let raised = Budget::new().with_input_bytes(2 * MAX_INPUT_LEN);
        let ast = parse_with(symbol.as_bytes(), raised).expect("raised input budget");
        assert_eq!(ast.to_string().len(), MAX_INPUT_LEN);
    }

    #[test]
    fn raising_the_depth_budget_accepts_a_deeper_type() {
        let mut symbol = String::from("ktype:");
        symbol.push_str(&"kotlin.Array<".repeat(MAX_DEPTH + 8));
        symbol.push('*');
        symbol.push_str(&">".repeat(MAX_DEPTH + 8));
        assert!(matches!(
            parse_with(symbol.as_bytes(), Budget::new()),
            Err(ParseError::RecursedTooDeep)
        ));
        let raised = Budget::new().with_parse_depth(u32::try_from(MAX_DEPTH + 8).unwrap());
        assert!(parse_with(symbol.as_bytes(), raised).is_ok());
    }

    /// The input and work dimensions are independent: raising the input alone
    /// keeps the default work allowance, so a scan that exceeds it still fails.
    #[test]
    fn raising_the_input_budget_does_not_raise_the_work_budget() {
        // The parser charges one work unit per input byte before classifying.
        let mut symbol = String::from("kclass:");
        symbol.push_str(&"a".repeat(600_000));
        assert!(4 * symbol.len() > MAX_WORK);
        let work = u32::try_from(4 * symbol.len()).expect("work fits a u32");

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
            .with_parse_depth(u32::MAX)
            .with_work(u32::MAX);
        let limits = ParseLimits::from_effective(budget.effective(Scheme::KotlinNative));
        assert_eq!(limits.input, HARD_INPUT_LEN);
        assert_eq!(limits.depth, HARD_DEPTH);
        assert_eq!(limits.work, HARD_WORK);
        assert_eq!(
            budget.effective(Scheme::KotlinNative).input_bytes,
            Some(Limit::Bounded(HARD_INPUT_LEN))
        );
        assert_eq!(
            budget.effective(Scheme::KotlinNative).parse_depth,
            Some(Limit::Bounded(HARD_DEPTH))
        );

        // A name past the hard input cap stays rejected even when the caller
        // asks for more.
        let mut too_long = String::from("kclass:");
        too_long.push_str(&"a".repeat(HARD_INPUT_LEN));
        assert!(matches!(
            parse_with(too_long.as_bytes(), budget),
            Err(ParseError::BudgetExceeded)
        ));
    }

    #[test]
    fn a_zero_budget_rejects_through_every_entry_point() {
        // The parameter type enters a depth level, and the parser charges work
        // per input byte, so every dimension rejects this symbol.
        let symbol = "kfun:com.example#foo(kotlin.Int){}";
        assert!(Demangler::new().demangle(symbol).is_ok());
        for budget in [
            Budget::new().with_input_bytes(0),
            Budget::new().with_work(0),
        ] {
            let demangler = Demangler::with_budget(budget);
            assert!(demangler.demangle(symbol).is_err(), "{budget:?}");
            assert!(
                demangler
                    .demangle_with_scheme(Scheme::KotlinNative, symbol)
                    .is_err(),
                "{budget:?}"
            );
            assert!(
                demangler
                    .selection()
                    .allow(Scheme::KotlinNative)
                    .demangle(symbol)
                    .is_err(),
                "{budget:?}"
            );
        }

        // The depth dimension only rejects a symbol that declares a type.
        let demangler = Demangler::with_budget(Budget::new().with_parse_depth(0));
        assert!(demangler.demangle(symbol).is_err());
        assert!(demangler.demangle("kclass:com.example.Box").is_ok());
    }

    /// The fixed output cap is a backstop, not a print budget.
    ///
    /// The printer walks each node once, and the widest expansion is `\xNN`
    /// for a byte that is not valid UTF-8, so four times the input bounds the
    /// text. The cap is sized from the hard input ceiling, so a symbol accepted
    /// under a raised input budget still fits.
    #[test]
    fn the_output_cap_is_a_backstop_the_printer_never_reaches() {
        let mut symbol = b"kclass:".to_vec();
        symbol.resize(MAX_INPUT_LEN, 0xff);
        let raised = Budget::new().with_input_bytes(2 * MAX_INPUT_LEN);
        let rendered = parse_with(&symbol, raised)
            .expect("the raised input budget parses")
            .to_string();
        assert!(rendered.len() <= MAX_OUTPUT_LEN);
        assert!(rendered.len() <= 4 * symbol.len());
    }

    #[test]
    fn the_default_budget_parses_the_corpus_like_parse() {
        let corpora = [
            "/refs/kotlin-native/corpus/hand-authored.txt",
            "/refs/kotlin-native/corpus/kotlin-2.0.21-linux-x86_64.txt",
            "/refs/kotlin-native/corpus/kotlin-2.1.21-linux-x86_64.txt",
            "/refs/kotlin-native/corpus/kotlin-2.2.21-linux-x86_64.txt",
            "/refs/kotlin-native/corpus/kotlin-2.3.21-linux-x86_64.txt",
            "/refs/kotlin-native/corpus/kotlin-2.4.0-linux-x86_64.txt",
            "/refs/kotlin-native/corpus/kotlin-2.4.21-linux-x86_64.txt",
        ];
        let mut compared = 0_usize;
        for corpus in corpora {
            let path = format!("{}{corpus}", std::env!("CARGO_MANIFEST_DIR"));
            let Ok(data) = std::fs::read_to_string(&path) else {
                std::eprintln!("kotlin-native corpus not present (published crate); skipping");
                return;
            };
            for line in data.lines() {
                let symbol = line.split('\t').next().unwrap_or_default().trim_end();
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
