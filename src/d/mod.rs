//! Pure-Rust D ABI symbol demangling.
//!
//! D mangles a symbol as `_D QualifiedName Type` (or `_D QualifiedName Z` for
//! internal symbols) so that functions, variables, classes and templates are
//! link-time type safe. This backend parses that grammar on bytes and prints
//! the same spelling as `c++filt -s dlang`.
//!
//! No external demangler is used and there is no runtime dependency.
//!
//! # Supported forms
//!
//! - qualified names and nested functions
//!   (`_D3app5outerFiZ5innerMFNaNbNiNfiZi` -> `app.outer(int).inner(int)`);
//! - primitives, `const`/`immutable`/`shared`/`inout`/`__vector`, pointers,
//!   dynamic, static and associative arrays, function and delegate types,
//!   class/struct/enum/typedef/identifier types, tuples and `typeof(*null)`;
//! - function parameters (`in`, `out`, `ref`, `lazy`, and the `scope`/`return`
//!   markers in either encoded order), function attributes and calling
//!   conventions;
//! - templates `__T`/`__U`, including nested instances, type, value, symbol
//!   and externally mangled arguments, specialized (`H`) arguments and
//!   template symbol parameters;
//! - identifier and type back references (`Q` + base-26), including the
//!   back-referenced `M Q<ref>` function type of a nested function;
//! - value literals: integers, characters, booleans, reals, complex numbers,
//!   strings, arrays, associative arrays, struct literals and function
//!   symbols;
//! - compiler specials: `_Dmain`, `__ctor`, `__dtor`, `__postblitMFZ`,
//!   `__initZ`, `__vtblZ`, `__ClassZ`, `__InterfaceZ`, `__ModuleInfoZ` and
//!   `__moduleRefZ`;
//! - `TypeInfo_*` names, which the ABI encodes as ordinary identifiers.
//!
//! A Mach-O leading underscore (`__D...`) is accepted as well as `_D...`. An
//! LDC this-adjusting thunk (`_DThn<offset>_` followed by the symbol it thunks
//! to) is accepted and prints the wrapped name; see `refs/d.md` for the
//! provenance of that non-ABI wrapper.
//!
//! # Canonical output
//!
//! The printed text matches `c++filt -s dlang` for the supported grammar. A
//! function's encoded return type is not printed at symbol level, because the
//! parameter list is the useful signature; a function *type* prints its return
//! type (`int(int) function`). Names are decoded as text: valid UTF-8 is
//! written verbatim, invalid bytes as `\xNN`, and a backslash in identifier
//! text as `\\`.
//!
//! # Detection
//!
//! `is_d` parses the whole symbol, so `_D` followed by malformed or
//! unsupported syntax is not claimed. This keeps plain C identifiers that
//! merely start with `_D` out of the D backend.
//!
//! # Budgets
//!
//! Untrusted input stays bounded by [`MAX_INPUT_LEN`](crate::d::MAX_INPUT_LEN),
//! [`MAX_DEPTH`](crate::d::MAX_DEPTH), [`MAX_WORK`](crate::d::MAX_WORK),
//! [`MAX_NODES`](crate::d::MAX_NODES) and
//! [`MAX_OUTPUT_LEN`](crate::d::MAX_OUTPUT_LEN). Back references re-scan
//! earlier input, so work is metered separately from input length. Exceeding a
//! budget is an error, never a partial result.
//!
//! The input, depth, work and node limits are runtime-configurable.
//! [`parse`](crate::d::parse) keeps the compiled-in defaults, and a
//! [`crate::Demangler`] passes the configured budget, clamped to the hard
//! ceilings of 16 MiB input, 1024 nesting levels, 64M work units and 1M
//! nodes. The four dimensions are independent: raising the input limit alone
//! keeps the default work allowance and the default node allowance, so a caller
//! that raises the input for a long symbol must raise [`crate::Budget::work`]
//! and [`crate::Budget::nodes`] as well when the parse needs more than they
//! allow.
//!
//! Printing is not a budget. The printer walks each owned node once, so the
//! output is linear in the parsed tree and no [`crate::Budget`] field
//! configures it. [`MAX_OUTPUT_LEN`](crate::d::MAX_OUTPUT_LEN) is a fixed
//! backstop at four times the default input cap: it does not scale with the
//! configured budget. It can bind for a symbol the parser accepted, because the
//! spelling is not linear in the input — an `immutable` modifier prints ten
//! bytes and a back reference re-parses an earlier part of the symbol — so the
//! demangler renders eagerly and reports the overflow as
//! [`ParseError::Invalid`](crate::d::ParseError) instead of returning a
//! [`crate::Demangled`] that would fail later in [`core::fmt::Display`].
//!
//! # Gaps
//!
//! - `X` externally mangled template arguments are printed verbatim; their
//!   payload is not demangled.
//! - Anonymous symbols (`0` components) are skipped; a symbol made only of
//!   anonymous components is rejected.
//! - A compiler special that prepends a phrase (`initializer for `, …) is only
//!   accepted as the final qualified-name component.
//! - The D ABI has no canonical demangler; `c++filt` is the generation-time
//!   oracle and its echo/placeholder output is recorded as an oracle gap in
//!   `refs/d.md`, not as coverage. The pinned corpus has no remaining gaps.

mod node;
mod parser;
mod printer;

use core::fmt;

use crate::budget::{EffectiveBudget, Limit};
use crate::parse::LimitedWriter;
use node::ParseLimits;
use node::Root;
pub use node::{MAX_DEPTH, MAX_INPUT_LEN, MAX_NODES, MAX_OUTPUT_LEN, MAX_WORK};

/// Why a D symbol could not be parsed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ParseError {
    /// The symbol does not match the D ABI grammar.
    Invalid,
    /// A type, qualified name or template nests deeper than `MAX_DEPTH`.
    RecursedTooDeep,
    /// The input, work or node budget was exceeded.
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

/// A parsed D symbol.
#[derive(Debug)]
pub struct Ast<'a> {
    root: Root<'a>,
}

impl fmt::Display for Ast<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.print(f)
    }
}

impl Ast<'_> {
    /// Render the parsed symbol, enforcing `MAX_OUTPUT_LEN`.
    pub(crate) fn print<W: fmt::Write + ?Sized>(&self, out: &mut W) -> fmt::Result {
        let mut limited = LimitedWriter::new(out, MAX_OUTPUT_LEN);
        printer::print(&self.root, &mut limited)
    }
}

/// Parse a D symbol from UTF-8.
///
/// # Errors
///
/// Returns [`ParseError::Invalid`] when the symbol does not match the D ABI
/// grammar, [`ParseError::RecursedTooDeep`] when nesting exceeds
/// `MAX_DEPTH`, and [`ParseError::BudgetExceeded`] when an input, work or
/// node budget is exceeded.
pub fn parse(symbol: &str) -> Result<Ast<'_>, ParseError> {
    parse_bytes(symbol.as_bytes())
}

/// Parse a D symbol from raw bytes.
///
/// This is the entry point the demangler dispatches to once symbol bytes, not
/// text, are the input. See [`parse`] for the accepted forms; the only
/// difference is that the symbol does not have to be UTF-8.
///
/// # Errors
///
/// See [`parse`].
pub(crate) fn parse_bytes(symbol: &[u8]) -> Result<Ast<'_>, ParseError> {
    parse_bytes_with_budget(symbol, DEFAULT_BUDGET)
}

/// The budget the public entry point starts from.
///
/// [`parse`] uses it, so it keeps exactly the compiled-in limits; a
/// [`crate::Demangler`] resolves its configured [`crate::Budget`] for
/// [`Scheme::D`](crate::Scheme::D) instead.
pub(crate) const DEFAULT_BUDGET: EffectiveBudget = EffectiveBudget {
    input_bytes: Some(Limit::Bounded(MAX_INPUT_LEN)),
    parse_depth: Some(Limit::Bounded(MAX_DEPTH)),
    work: Some(Limit::Bounded(MAX_WORK)),
    nodes: Some(Limit::Bounded(MAX_NODES)),
};

/// Parse a D symbol with one scheme's resolved budget.
///
/// The public [`parse`] wraps this with [`DEFAULT_BUDGET`]; a
/// [`crate::Demangler`] passes
/// [`Demangler::effective_budget`](crate::Demangler::effective_budget) for
/// [`Scheme::D`](crate::Scheme::D), so a caller's overrides reach the parser.
/// Every dimension is clamped to this module's hard ceiling and the input,
/// depth, work and node limits are independent of each other.
pub(crate) fn parse_bytes_with_budget(
    symbol: &[u8],
    budget: EffectiveBudget,
) -> Result<Ast<'_>, ParseError> {
    parser::parse_bytes(
        crate::scheme::strip_decorations_bytes(symbol),
        ParseLimits::from_effective(budget),
    )
}

/// True when `symbol` is a complete D ABI symbol.
///
/// This parses the whole symbol, so a bare `_D` prefix, a truncated name or a
/// grammar production this backend rejects is not claimed.
///
/// Detection is default-only: it parses with the compiled-in budget, and a
/// [`crate::Demangler`] re-parses the retained bytes with the configured
/// budget instead of reusing this tree.
#[must_use]
pub fn is_d(symbol: &[u8]) -> bool {
    parse_bytes(symbol).is_ok()
}

#[cfg(test)]
mod tests {
    use alloc::string::{String, ToString};

    use super::*;

    fn demangle(symbol: &str) -> String {
        parse(symbol).unwrap().to_string()
    }

    #[test]
    fn strips_linker_decorations() {
        for symbol in [
            "_D3foo3barFZv.llvm.9D1C9369",
            "_D3foo3barFZv@GLIBC_2.2.5",
            "_D3foo3barFZv@@GLIBC_2.2.5",
            "_D3foo3barFZv@16",
        ] {
            assert_eq!(demangle(symbol), "foo.bar()", "{symbol}");
            assert!(is_d(symbol.as_bytes()), "{symbol}");
        }
    }

    #[test]
    fn parses_main() {
        assert_eq!(demangle("_Dmain"), "D main");
    }

    #[test]
    fn parses_function_names() {
        assert_eq!(demangle("_D3foo3barFZv"), "foo.bar()");
        assert_eq!(demangle("_D4test3fooFiZi"), "test.foo(int)");
    }

    #[test]
    fn parses_arrays_and_delegates() {
        assert_eq!(
            demangle("_D3foo3barFAiG5iHAyaiZi"),
            "foo.bar(int[], int[5], int[immutable(char)[]])"
        );
        assert_eq!(demangle("_D3foo3barFDFiZiZi"), "foo.bar(int(int) delegate)");
    }

    #[test]
    fn parses_templates_and_backrefs() {
        assert_eq!(
            demangle("_D3app__T3BoxTiZQh3getMFZi"),
            "app.Box!(int).Box.get()"
        );
    }

    #[test]
    fn parses_specials() {
        assert_eq!(demangle("_D4test3Foo6__initZ"), "initializer for test.Foo");
        assert_eq!(demangle("_D4test3Foo6__vtblZ"), "vtable for test.Foo");
        assert_eq!(demangle("_D4test3Foo7__ClassZ"), "ClassInfo for test.Foo");
        assert_eq!(demangle("_D4test12__ModuleInfoZ"), "ModuleInfo for test");
        assert_eq!(
            demangle("_D4test3Foo11__moduleRefZ"),
            "test.Foo.__moduleRef"
        );
    }

    #[test]
    fn parses_parameter_marker_order() {
        // The mangler emits `M` (scope) and `Nk` (return) in either order and
        // the printed text follows the encoded order.
        assert_eq!(demangle("_D3foo3barFMiZv"), "foo.bar(scope int)");
        assert_eq!(demangle("_D3foo3barFNkiZv"), "foo.bar(return int)");
        assert_eq!(demangle("_D3foo3barFMNkiZv"), "foo.bar(scope return int)");
        assert_eq!(demangle("_D3foo3barFNkMiZv"), "foo.bar(return scope int)");
        assert_eq!(demangle("_D3foo3barFNkKiZv"), "foo.bar(return ref int)");
        assert_eq!(demangle("_D3foo3barFNkJiZv"), "foo.bar(return out int)");
        assert_eq!(
            demangle("_D3foo3barFMNkKiZv"),
            "foo.bar(scope return ref int)"
        );
        assert_eq!(
            demangle("_D3foo3barFNkMKiZv"),
            "foo.bar(return scope ref int)"
        );
    }

    #[test]
    fn parses_thunk_wrappers() {
        // A this-adjusting thunk is named `_DThn<offset>_` plus the symbol it
        // thunks to; the wrapper does not change the name.
        assert_eq!(demangle("_DThn16_3foo3barFZv"), "foo.bar()");
        assert_eq!(demangle("__DThn24_3foo3barFiZi"), "foo.bar(int)");
        assert!(is_d(b"_DThn16_3foo3barFZv"));
        assert!(parse("_DThn_3foo3barFZv").is_err());
        assert!(parse("_DThn16_").is_err());
    }

    #[test]
    fn parses_backreferenced_function_type() {
        // `M Q<ref>` stands for the whole function type of a nested function,
        // return type included, so no type follows it.
        assert_eq!(
            demangle("_D3foo3barFDFZvZ1aMQh"),
            "foo.bar(void() delegate).a()"
        );
        // A reference to a length digit is an identifier back reference, not
        // a function type, so the signature cannot start there.
        assert!(parse("_D3foo3barFDFZvZ1aMQd").is_err());
    }

    #[test]
    fn rejects_bare_prefix_and_truncation() {
        assert!(parse("_D").is_err());
        assert!(parse("_Dfoo").is_err());
        assert!(parse("_D3foo3bar").is_err());
        assert!(!is_d(b"_D"));
        assert!(!is_d(b"_Dfoo"));
    }

    #[test]
    fn accepts_macho_prefix() {
        assert_eq!(demangle("__D3foo3barFZv"), "foo.bar()");
    }

    #[test]
    fn parses_raw_bytes() {
        let ast = parse_bytes(b"_D8foo\xff\xfebarFZv").unwrap();
        assert_eq!(ast.to_string(), "foo\\xff\\xfebar()");
    }

    #[test]
    fn budgets_are_bounded() {
        let long = alloc::format!("_D{}", "A".repeat(MAX_INPUT_LEN));
        assert!(matches!(
            parse_bytes(long.as_bytes()),
            Err(ParseError::BudgetExceeded)
        ));

        let deep = alloc::format!("_D3foo3barF{}iZv", "P".repeat(MAX_DEPTH + 1));
        assert!(matches!(
            parse_bytes(deep.as_bytes()),
            Err(ParseError::RecursedTooDeep)
        ));
    }

    #[test]
    fn work_budget_bounds_back_reference_rescans() {
        // Each parameter type is a tuple of two back references to the previous
        // parameter type, so one re-scan costs twice its target and the work
        // grows as 2^n while the input stays a few hundred bytes. The work
        // budget, not the input or depth budget, has to stop the re-scan.
        assert!(parse(&nested_tuple_params(4)).is_ok());

        let symbol = nested_tuple_params(20);
        assert!(symbol.len() < MAX_INPUT_LEN);
        assert!(matches!(
            parse_bytes(symbol.as_bytes()),
            Err(ParseError::BudgetExceeded)
        ));
    }

    /// One function parameter per level; each level is `B2` plus two back
    /// references to the level below it.
    fn nested_tuple_params(levels: usize) -> String {
        let mut symbol = String::from("_D3foo3barF");
        let mut target = symbol.len();
        symbol.push('i');
        for _ in 0..levels {
            let start = symbol.len();
            let mut level = String::from("B2");
            for _ in 0..2 {
                let value = start + level.len() - target;
                level.push('Q');
                level.push_str(&backref(value));
            }
            symbol.push_str(&level);
            target = start;
        }
        symbol.push_str("Zv");
        symbol
    }

    /// Encode a positive base-26 back-reference position.
    fn backref(value: usize) -> String {
        let mut digits = alloc::vec![b'a' + u8::try_from(value % 26).unwrap()];
        let mut rest = value / 26;
        while rest > 0 {
            digits.push(b'A' + u8::try_from(rest % 26).unwrap());
            rest /= 26;
        }
        digits.reverse();
        String::from_utf8(digits).unwrap()
    }

    #[test]
    fn output_budget_is_enforced() {
        // One long name followed by many identifier back references: the
        // input is under 64 KiB but the expanded spelling exceeds the
        // absolute output ceiling.
        let name_len = 20_000;
        let mut symbol = alloc::format!("_D{name_len}{}", "a".repeat(name_len));
        for _ in 0..5_000 {
            let value = symbol.len() - 2;
            symbol.push('Q');
            symbol.push_str(&backref(value));
        }
        symbol.push('i');

        let ast = parse(&symbol).unwrap();
        let mut out = String::new();
        assert!(ast.print(&mut out).is_err());
    }

    #[test]
    fn fake_parent_chain_is_stack_safe() {
        // `4__S0` is one length-prefixed fake parent. 12 000 of them fit in
        // the input budget and used to cost one `parse_identifier` frame each;
        // the chain is consumed in a loop, so it must parse on a small stack.
        let mut symbol = String::from("_D");
        for _ in 0..12_000 {
            symbol.push_str("4__S0");
        }
        symbol.push_str("1aZ");

        let rendered = std::thread::Builder::new()
            .stack_size(512 * 1024)
            .spawn(move || {
                parse(&symbol)
                    .expect("fake-parent chain parses")
                    .to_string()
            })
            .expect("spawn")
            .join()
            .expect("no stack overflow");
        assert_eq!(rendered, "a");
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
}

#[cfg(test)]
mod budget_tests {
    use alloc::format;
    use alloc::string::{String, ToString};

    use super::node::{HARD_DEPTH, HARD_INPUT_LEN, HARD_NODES, HARD_WORK, ParseLimits};
    use super::*;
    use crate::budget::Limit;
    use crate::{Budget, Demangler, Error, Scheme};

    /// Parse `symbol` with `budget` resolved for [`Scheme::D`].
    fn parse_with(symbol: &[u8], budget: Budget) -> Result<Ast<'_>, ParseError> {
        parse_bytes_with_budget(symbol, budget.effective(Scheme::D))
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

    /// A symbol whose only printed component is `name_len` bytes long.
    fn long_name(name_len: usize) -> String {
        format!("_D{name_len}{}Z", "a".repeat(name_len))
    }

    #[test]
    fn the_compiled_in_budget_matches_the_policy_defaults() {
        let limits = ParseLimits::from_effective(DEFAULT_BUDGET);
        assert_eq!(limits.input, MAX_INPUT_LEN);
        assert_eq!(limits.depth, MAX_DEPTH);
        assert_eq!(limits.work, MAX_WORK);
        assert_eq!(limits.nodes, MAX_NODES);
        assert_eq!(
            Scheme::D.default_budget().effective(Scheme::D),
            DEFAULT_BUDGET
        );

        // The policy table and this module agree on the ceilings, and every
        // hard cap still fits the shared cursor's narrower fields.
        let hard = Scheme::D.hard_budget();
        assert_eq!(hard.input_bytes, Some(Limit::Bounded(HARD_INPUT_LEN)));
        assert_eq!(hard.parse_depth, Some(Limit::Bounded(HARD_DEPTH)));
        assert_eq!(hard.work, Some(Limit::Bounded(HARD_WORK)));
        assert_eq!(hard.nodes, Some(Limit::Bounded(HARD_NODES)));

        let hard_limits = ParseLimits::from_effective(hard);
        let cursor = hard_limits.cursor();
        assert_eq!(usize::try_from(cursor.input).unwrap(), HARD_INPUT_LEN);
        assert_eq!(usize::from(cursor.depth), HARD_DEPTH);
        assert_eq!(usize::try_from(cursor.work).unwrap(), HARD_WORK);
    }

    #[test]
    fn the_default_budget_matches_the_public_parse() {
        for symbol in [
            "_Dmain",
            "_D3foo3barFZv",
            "_D3foo3barFAiG5iHAyaiZi",
            "_D3app__T3BoxTiZQh3getMFZi",
            "_DThn16_3foo3barFZv",
            "_D4test3Foo6__initZ",
            "_D3foo3barFDFZvZ1aMQh",
            "_D",
            "_D3foo3bar",
        ] {
            let expected = parse(symbol);
            let explicit = parse_with(symbol.as_bytes(), Budget::new());
            assert_eq!(
                expected.is_ok(),
                explicit.is_ok(),
                "{symbol}: the default budget changes acceptance"
            );
            if let (Ok(expected), Ok(explicit)) = (expected, explicit) {
                assert_eq!(expected.to_string(), explicit.to_string(), "{symbol}");
            }
        }
    }

    #[test]
    fn each_dimension_rejects_one_unit_below_what_the_symbol_needs() {
        // `_Dmain` is recognised before the grammar runs, so it charges no
        // work, depth or nodes and has no non-zero boundary to test.
        for symbol in [
            "_D3foo3barFZv",
            "_D3foo3barFAiG5iHAyaiZi",
            "_D3app__T3BoxTiZQh3getMFZi",
        ] {
            let ceiling = u32::try_from(symbol.len()).unwrap_or(u32::MAX) + 8;
            assert_dimension_boundary(symbol, "input", ceiling, |value| {
                Budget::new().with_input_bytes(value as usize)
            });
            assert_dimension_boundary(symbol, "work", 256, |value| {
                Budget::new().with_work(value)
            });
            assert_dimension_boundary(symbol, "nodes", 16, |value| {
                Budget::new().with_nodes(value)
            });
        }

        // A type, a qualified name and a template each enter the depth
        // dimension once.
        assert_dimension_boundary("_D3foo3barFZv", "depth", 8, |value| {
            Budget::new().with_parse_depth(value)
        });
        assert_dimension_boundary("_D3app__T3BoxTiZQh3getMFZi", "depth", 16, |value| {
            Budget::new().with_parse_depth(value)
        });
    }

    #[test]
    fn raising_the_input_budget_accepts_a_longer_symbol() {
        let name_len = MAX_INPUT_LEN + 1024;
        let symbol = long_name(name_len);
        assert!(matches!(
            parse_with(symbol.as_bytes(), Budget::new()),
            Err(ParseError::BudgetExceeded)
        ));

        let raised = Budget::new().with_input_bytes(HARD_INPUT_LEN);
        let ast = parse_with(symbol.as_bytes(), raised).expect("raised input budget");
        assert_eq!(ast.to_string().len(), name_len);
    }

    #[test]
    fn raising_the_depth_budget_accepts_a_deeper_type() {
        let depth = MAX_DEPTH + 8;
        let symbol = format!("_D3foo3barF{}iZv", "P".repeat(depth));
        assert!(matches!(
            parse_with(symbol.as_bytes(), Budget::new()),
            Err(ParseError::RecursedTooDeep)
        ));

        let raised = Budget::new().with_parse_depth(u32::try_from(depth).unwrap() + 2);
        assert!(parse_with(symbol.as_bytes(), raised).is_ok());
    }

    #[test]
    fn raising_the_node_budget_accepts_a_wider_signature() {
        // Each `i` parameter is one node, so the signature needs more nodes
        // than the default allowance while staying far below the hard input
        // ceiling.
        let params = MAX_NODES + 8;
        let mut symbol = String::from("_D3foo3barF");
        symbol.push_str(&"i".repeat(params));
        symbol.push_str("Zv");
        assert!(symbol.len() < HARD_INPUT_LEN);

        // The default input cap rejects it first, so raise the input and show
        // that the node allowance is an independent dimension.
        let input_only = Budget::new().with_input_bytes(HARD_INPUT_LEN);
        assert!(matches!(
            parse_with(symbol.as_bytes(), input_only),
            Err(ParseError::BudgetExceeded)
        ));

        let nodes = u32::try_from(params).unwrap() + 8;
        assert!(parse_with(symbol.as_bytes(), input_only.with_nodes(nodes)).is_ok());
    }

    /// The input and work dimensions are independent: raising the input alone
    /// keeps the default work allowance, so a scan that exceeds it still fails.
    #[test]
    fn raising_the_input_budget_does_not_raise_the_work_budget() {
        // Every byte of the identifier is scanned and charged once, so this
        // symbol needs more work than the default allowance.
        let symbol = long_name(MAX_WORK + 1024);
        assert!(symbol.len() < HARD_INPUT_LEN);

        let input_only = Budget::new().with_input_bytes(HARD_INPUT_LEN);
        assert!(matches!(
            parse_with(symbol.as_bytes(), input_only),
            Err(ParseError::BudgetExceeded)
        ));

        let work = u32::try_from(symbol.len()).unwrap() + 64;
        assert!(parse_with(symbol.as_bytes(), input_only.with_work(work)).is_ok());
    }

    #[test]
    fn clamping_keeps_the_hard_ceilings() {
        let budget = Budget::new()
            .with_input_bytes(usize::MAX)
            .with_parse_depth(u32::MAX)
            .with_work(u32::MAX)
            .with_nodes(u32::MAX);
        let effective = budget.effective(Scheme::D);
        assert_eq!(effective.input_bytes, Some(Limit::Bounded(HARD_INPUT_LEN)));
        assert_eq!(effective.parse_depth, Some(Limit::Bounded(HARD_DEPTH)));
        assert_eq!(effective.work, Some(Limit::Bounded(HARD_WORK)));
        assert_eq!(effective.nodes, Some(Limit::Bounded(HARD_NODES)));

        let limits = ParseLimits::from_effective(effective);
        assert_eq!(limits.input, HARD_INPUT_LEN);
        assert_eq!(limits.depth, HARD_DEPTH);
        assert_eq!(limits.work, HARD_WORK);
        assert_eq!(limits.nodes, HARD_NODES);

        // A symbol past the hard input cap stays rejected even when the caller
        // asks for more.
        let too_long = long_name(HARD_INPUT_LEN);
        assert!(matches!(
            parse_with(too_long.as_bytes(), budget),
            Err(ParseError::BudgetExceeded)
        ));
    }

    #[test]
    fn a_zero_budget_rejects_through_every_entry_point() {
        // A qualified name with a signature charges work, enters depth and
        // builds nodes, so every dimension rejects this symbol.
        let symbol = "_D3foo3barFZv";
        assert!(Demangler::new().demangle(symbol).is_ok());
        for budget in [
            Budget::new().with_input_bytes(0),
            Budget::new().with_parse_depth(0),
            Budget::new().with_work(0),
            Budget::new().with_nodes(0),
        ] {
            let demangler = Demangler::with_budget(budget);
            // Detection stays default-only, so the scheme is still D; the
            // configured budget rejects the re-parse rather than
            // misclassifying the symbol as plain C.
            assert_eq!(
                demangler.demangle(symbol).unwrap_err(),
                Error::InvalidSymbol(Scheme::D),
                "{budget:?}"
            );
            assert_eq!(
                demangler
                    .demangle_with_scheme(Scheme::D, symbol)
                    .unwrap_err(),
                Error::InvalidSymbol(Scheme::D),
                "{budget:?}"
            );
            assert_eq!(
                demangler
                    .selection()
                    .allow(Scheme::D)
                    .demangle(symbol)
                    .unwrap_err(),
                Error::InvalidSymbol(Scheme::D),
                "{budget:?}"
            );
        }
    }

    /// Printing is not a budget: the printer walks each owned node once, so the
    /// output is linear in the parsed tree and no [`Budget`] field configures
    /// it.
    ///
    /// [`MAX_OUTPUT_LEN`] is a fixed backstop sized from the default input cap,
    /// so raising the input budget does not widen it. It can bind for a symbol
    /// the parser accepted, because the text is not linear in the input: an
    /// `immutable` modifier prints ten bytes, and a back reference re-parses an
    /// earlier part of the symbol. The demangler therefore renders eagerly and
    /// reports the overflow as a rejection instead of returning a value whose
    /// `Display` would fail.
    #[test]
    fn the_fixed_output_backstop_is_not_a_budget() {
        assert_eq!(MAX_OUTPUT_LEN, 4 * MAX_INPUT_LEN);

        // Twenty thousand `immutable int` parameters print fifteen bytes each,
        // past the backstop while the input, depth, work and node budgets all
        // stay at their defaults.
        let mut symbol = String::from("_D3foo3barF");
        symbol.push_str(&"yi".repeat(20_000));
        symbol.push_str("Zv");
        assert!(symbol.len() < MAX_INPUT_LEN);

        let ast = parse(&symbol).expect("the parser accepts the signature");
        let mut out = String::new();
        assert!(ast.print(&mut out).is_err());
        assert!(out.len() <= MAX_OUTPUT_LEN);

        assert_eq!(
            Demangler::new().demangle(&symbol).unwrap_err(),
            Error::InvalidSymbol(Scheme::D)
        );
    }

    /// The backstop does not scale with the configured budget, so a symbol
    /// accepted under a raised input budget still prints as long as its
    /// spelling fits the fixed ceiling.
    #[test]
    fn a_raised_input_budget_prints_within_the_fixed_backstop() {
        let name_len = MAX_INPUT_LEN + 1024;
        let symbol = long_name(name_len);
        let raised = Budget::new().with_input_bytes(HARD_INPUT_LEN);
        let rendered = parse_with(symbol.as_bytes(), raised)
            .expect("the raised budget parses")
            .to_string();
        assert_eq!(rendered.len(), name_len);
        assert!(rendered.len() <= MAX_OUTPUT_LEN);
    }

    #[test]
    fn the_default_budget_parses_the_corpus_like_parse() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("refs/d");
        if !root.is_dir() {
            std::eprintln!("refs/d is absent; skipping the D budget corpus parity check");
            return;
        }

        let mut compared = 0_usize;
        for corpus in ["hand.tsv", "ldc-1.40-app.tsv", "libphobos-2.110.0.tsv"] {
            let path = root.join(corpus);
            let data = std::fs::read_to_string(&path)
                .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()));
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
