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
//! Untrusted input stays bounded by `MAX_INPUT_LEN`, `MAX_DEPTH`,
//! `MAX_WORK`, `MAX_NODES` and `MAX_OUTPUT_LEN`. Back references re-scan
//! earlier input, so work is metered separately from input length. Exceeding a
//! budget is an error, never a partial result, and the printer refuses to emit
//! more than `MAX_OUTPUT_LEN` bytes.
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

use crate::parse::LimitedWriter;
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
    parser::parse_bytes(crate::scheme::strip_decorations_bytes(symbol))
}

/// True when `symbol` is a complete D ABI symbol.
///
/// This parses the whole symbol, so a bare `_D` prefix, a truncated name or a
/// grammar production this backend rejects is not claimed.
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
