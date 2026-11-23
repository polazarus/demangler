//! Pure-Rust Ada/GNAT external-name demangling.
//!
//! GNAT does not use a C++-style mangling for Ada declarations. It builds an
//! *external name* from the fully-qualified Ada name by replacing every `.`
//! with `__` and lowercasing the identifier, then appends compiler suffixes
//! for protected operations, elaboration procedures, tasks and runtime
//! helpers. This backend implements the user-level spelling and the two
//! documented suffixes in the first slice.
//!
//! No external demangler is used and there is no runtime dependency.
//!
//! # Supported forms
//!
//! - qualified user names: lowercase identifiers joined by `__`, printed with
//!   `.` (`ada__exceptions__triggered_by_abort` ->
//!   `ada.exceptions.triggered_by_abort`);
//! - `_` inside an identifier (`ada__foo_bar` -> `ada.foo_bar`);
//! - protected-operation suffixes `N`/`P` on the last identifier
//!   (`pkg__prot__setN` and `pkg__prot__setP` -> `pkg.prot.set`);
//! - elaboration body/spec suffixes `___elabb`/`___elabs`
//!   (`pkg___elabb` -> `pkg'Elab_Body`, `pkg___elabs` -> `pkg'Elab_Spec`);
//! - a Mach-O leading platform underscore on the high-confidence `ada__`
//!   family (`_ada__foo` and `__ada__foo` -> `ada.foo`);
//! - a trailing linker decoration (`.llvm.<hex>` or `@version`) is stripped
//!   before parsing, matching the other backends.
//!
//! # Canonical output
//!
//! There is no standard Ada demangler, so the printed text is this crate's
//! contract:
//!
//! | Input | Output |
//! |---|---|
//! | `ada__exceptions__triggered_by_abort` | `ada.exceptions.triggered_by_abort` |
//! | `pkg__child__nested` | `pkg.child.nested` |
//! | `pkg__prot__setN` | `pkg.prot.set` |
//! | `pkg___elabb` | `pkg'Elab_Body` |
//! | `pkg___elabs` | `pkg'Elab_Spec` |
//!
//! The spelling is a debugger name, not re-parsable Ada source: `__` becomes
//! `.`, and the elaboration suffix becomes an Ada attribute-style apostrophe.
//!
//! # Detection
//!
//! [`crate::Scheme::detect`] claims only high-confidence GNAT prefixes:
//! `ada__`, `_ada__`, `__ada__`, `system__` and `gnat__`. This is a prefix
//! prefilter; [`parse`](crate::ada::parse) still rejects unsupported bodies (task, runtime and
//! other compiler suffixes), so a malformed high-confidence name becomes an
//! error rather than a guessed rendering. General `pkg__subp` names are valid
//! GNAT but indistinguishable from ordinary C names (`foo__bar`), so they are
//! available through [`parse`](crate::ada::parse) and are deliberately not auto-detected.
//!
//! # Budgets
//!
//! Untrusted input stays bounded by [`MAX_INPUT_LEN`](crate::ada::MAX_INPUT_LEN),
//! [`MAX_SEGMENTS`](crate::ada::MAX_SEGMENTS), [`MAX_WORK`](crate::ada::MAX_WORK)
//! and [`MAX_OUTPUT_LEN`](crate::ada::MAX_OUTPUT_LEN). Exceeding a budget is an error, never a
//! partial result. The grammar is flat, so there is no recursive depth to
//! bound; the segment limit is the node budget.
//!
//! # Gaps
//!
//! The first slice rejects, rather than guesses:
//!
//! - task bodies/types and their runtime helpers (`_TB`, `_VIP`, `_Z`);
//! - package/child elaboration flags (`_E`) and protected-type descriptors;
//! - record initialization helpers and enum spec helpers (`_S`);
//! - protected entry barriers (`_B4s`/`_E3s`);
//! - local-symbol `.N` suffixes and overload disambiguators (`__N`);
//! - foreign-convention, operator, and C++/other-language external names;
//! - names whose last identifier has an unknown uppercase compiler suffix
//!   (for example `enumS` or `setX`).
//!
//! An enum initialization helper such as `pkg__enumN` is accepted because it
//! uses the same trailing `N` as a protected operation.
//!
//! These classes are counted in `refs/ada.md`.

mod node;
mod parser;
mod printer;

use core::fmt;

pub use node::{Ast, MAX_INPUT_LEN, MAX_OUTPUT_LEN, MAX_SEGMENTS, MAX_WORK};
pub use parser::is_ada;

/// Why an Ada/GNAT symbol could not be parsed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ParseError {
    /// The symbol does not match a supported GNAT external-name form.
    Invalid,
    /// The input, work or segment budget was exceeded.
    BudgetExceeded,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Invalid => "invalid syntax",
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
    /// Render the parsed name into `out`, enforcing [`MAX_OUTPUT_LEN`].
    pub(crate) fn print<W: fmt::Write + ?Sized>(&self, out: &mut W) -> fmt::Result {
        let mut limited = LimitedWriter {
            inner: out,
            written: 0,
        };
        printer::print(&self.name, &mut limited)
    }
}

/// A writer that refuses to grow the output past [`MAX_OUTPUT_LEN`].
struct LimitedWriter<'a, W: fmt::Write + ?Sized> {
    inner: &'a mut W,
    written: usize,
}

impl<W: fmt::Write + ?Sized> fmt::Write for LimitedWriter<'_, W> {
    fn write_str(&mut self, text: &str) -> fmt::Result {
        let Some(total) = self.written.checked_add(text.len()) else {
            return Err(fmt::Error);
        };
        if total > MAX_OUTPUT_LEN {
            return Err(fmt::Error);
        }
        self.written = total;
        self.inner.write_str(text)
    }
}

/// Parse an Ada/GNAT external name into a reusable [`Ast`].
///
/// General GNAT user names are accepted, including names that are not
/// auto-detected by [`crate::Scheme::detect`]. A Mach-O leading underscore on
/// the `ada__` family and a trailing linker decoration are stripped first.
///
/// # Errors
///
/// Returns [`ParseError::Invalid`] when the symbol is not a supported GNAT
/// external name (including task, protected-type, runtime and other
/// compiler-generated suffixes), and [`ParseError::BudgetExceeded`] when the
/// input, work or segment budget is exceeded.
pub fn parse(symbol: &str) -> Result<Ast<'_>, ParseError> {
    parse_bytes(symbol.as_bytes())
}

/// Parse an Ada/GNAT external name from raw bytes.
///
/// This is the entry point the demangler dispatch uses once symbol bytes, not
/// text, are the input. See [`parse`] for the accepted forms; the only
/// difference is that the name does not have to be UTF-8.
pub(crate) fn parse_bytes(symbol: &[u8]) -> Result<Ast<'_>, ParseError> {
    parser::parse_bytes(symbol)
}

#[cfg(test)]
mod tests {
    use alloc::string::{String, ToString};

    use super::*;

    #[test]
    fn parses_qualified_names() {
        for (symbol, expected) in [
            (
                "ada__exceptions__triggered_by_abort",
                "ada.exceptions.triggered_by_abort",
            ),
            ("pkg__child__nested", "pkg.child.nested"),
            ("ada__foo_bar", "ada.foo_bar"),
            ("pkg__x1_y2", "pkg.x1_y2"),
        ] {
            assert_eq!(parse(symbol).unwrap().to_string(), expected, "{symbol}");
        }
    }

    #[test]
    fn strips_protected_suffixes() {
        for (symbol, expected) in [
            ("pkg__prot__setN", "pkg.prot.set"),
            ("pkg__prot__setP", "pkg.prot.set"),
            ("pkg__prot__getN", "pkg.prot.get"),
            ("pkg__prot__getP", "pkg.prot.get"),
            ("pkg__enumN", "pkg.enum"),
            ("pkg__set2N", "pkg.set2"),
        ] {
            assert_eq!(parse(symbol).unwrap().to_string(), expected, "{symbol}");
        }
    }

    #[test]
    fn prints_elaboration_suffixes() {
        for (symbol, expected) in [
            ("pkg___elabb", "pkg'Elab_Body"),
            ("pkg___elabs", "pkg'Elab_Spec"),
            ("pkg__child___elabb", "pkg.child'Elab_Body"),
            ("ada__foo___elabs", "ada.foo'Elab_Spec"),
        ] {
            assert_eq!(parse(symbol).unwrap().to_string(), expected, "{symbol}");
        }
    }

    #[test]
    fn strips_platform_underscores() {
        for symbol in ["_ada__foo", "__ada__foo"] {
            assert_eq!(parse(symbol).unwrap().to_string(), "ada.foo", "{symbol}");
        }
    }

    #[test]
    fn strips_linker_decorations() {
        assert_eq!(parse("ada__foo.llvm.1234").unwrap().to_string(), "ada.foo");
        assert_eq!(parse("ada__foo@@GNAT_1").unwrap().to_string(), "ada.foo");
    }

    #[test]
    fn rejects_unsupported_suffixes() {
        for symbol in [
            "pkg__workerTB",
            "pkg__workerVIP",
            "pkg__workerZ",
            "pkg_E",
            "pkg__child_E",
            "pkg__recIP",
            "pkg__protVIP",
            "pkg__protA",
            "pkg__prot__wait_B4s",
            "pkg__prot__wait_E3s",
            "pkg__enumS",
            "pkg__setX",
            "pkg__setNP",
            "pkg__over__2",
            "pkg__nested__inner.2",
            "pkg__setN___elabb",
            "_ada_main",
            "DW.ref.__gnat_personality_v0",
        ] {
            assert!(parse(symbol).is_err(), "{symbol} should be rejected");
        }
    }

    #[test]
    fn rejects_malformed_names() {
        for symbol in [
            "",
            "ada__",
            "ada__Foo",
            "ada__9foo",
            "ada__foo_",
            "ada__foo__",
            "ada__foo___elabx",
            "ada__foo___elabb__2",
            "ada__foo-bar",
            "ada__foo bar",
        ] {
            assert!(parse(symbol).is_err(), "{symbol:?} should be rejected");
        }
    }

    #[test]
    fn budgets_are_enforced() {
        let long = "a".repeat(MAX_INPUT_LEN + 1);
        assert!(matches!(parse(&long), Err(ParseError::BudgetExceeded)));

        let mut many = String::from("a");
        for _ in 0..MAX_SEGMENTS {
            many.push_str("__a");
        }
        assert!(matches!(parse(&many), Err(ParseError::BudgetExceeded)));
    }

    #[test]
    fn detection_is_precision_only() {
        for symbol in [
            "ada__foo",
            "ada__exceptions__triggered_by_abort",
            "system__foo",
            "gnat__foo",
            "_ada__foo",
            "__ada__foo",
            "ada__",
            "ada__workerTB",
            "ada__foo__2",
            "system__workerTB",
        ] {
            assert!(is_ada(symbol.as_bytes()), "{symbol} should be Ada");
        }
        for symbol in ["foo__bar", "pkg__hello", "a__b__c", "_ada_main"] {
            assert!(!is_ada(symbol.as_bytes()), "{symbol} should not be Ada");
        }
    }
}
