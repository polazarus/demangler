//! Pure-Rust Java JNI native-method name demangling.
//!
//! A Java native method is bound to a C function whose name the JVM derives
//! from the method's declaring class and name. This module parses that name,
//! decodes the JNI escapes and renders a Java-like display form. There is no
//! canonical JNI demangler, so the spelling is this crate's contract.
//!
//! No external demangler is used and there is no runtime dependency.
//!
//! # Supported forms
//!
//! - short names: `Java_com_example_Foo_plain`, emitted when the method is not
//!   overloaded;
//! - long names: `Java_com_example_Foo_bar__I`, emitted for an overloaded
//!   method; the part after `__` is the mangled argument list, which is empty
//!   for the no-argument overload;
//! - every JNI escape: `_1` (`_`) in a name, `_2` (`;`) and `_3` (`[`) in an
//!   argument list, and `_0xxxx` (a UTF-16 code unit, including a surrogate
//!   pair for a supplementary character) in both;
//! - every descriptor type: the primitives `B C D F I J S Z`, object types
//!   (`L...;`) and arrays (`_3` per dimension).
//!
//! The mangled argument list is the descriptor's argument types with the
//! parentheses dropped: `([Ljava/lang/String;I)` becomes
//! `_3Ljava_lang_String_2I` and `()` becomes the empty string.
//!
//! # Output contract
//!
//! | Input | Output |
//! |---|---|
//! | `Java_com_example_Foo_plain` | `com.example.Foo.plain` |
//! | `Java_com_example_Foo_bar___3Ljava_lang_String_2I` | `com.example.Foo.bar(java.lang.String[], int)` |
//! | `Java_com_example_Foo_under_1score__` | `com.example.Foo.under_score()` |
//! | `Java_com_example_Weird_11_m__` | `com.example.Weird_1.m()` |
//!
//! The mangling writes both the package separator (`.`) and the binary-name
//! separator (`/`) as a bare `_`, and escapes a literal `_` as `_1`, so the
//! class path is recoverable: every unescaped `_` is a component boundary. The
//! last component before the descriptor delimiter is the method, and everything
//! before it is the class path, which is why the display form is a debugger
//! name rather than a Java source declaration.
//!
//! The long form encodes the argument types only: the JVM appends it to
//! disambiguate overloads, and Java forbids return-type-only overloading, so
//! the return type is redundant and absent from the symbol.
//!
//! # Raw bytes
//!
//! A symbol name is a byte string, so the parser matches the grammar on bytes
//! and never requires the input to be UTF-8. A byte that the mangling leaves
//! unescaped must be an ASCII alphanumeric, so a non-ASCII character can only
//! appear through a `_0xxxx` escape; decoded characters are printed as UTF-8.
//!
//! # Detection
//!
//! [`crate::Scheme::detect`] claims a name only when the whole symbol parses as
//! a short or long JNI name ([`is_jni`](crate::jni::is_jni)). The check runs before the legacy C++
//! dialects, which would otherwise claim a long name because it contains `__`.
//! `JNI_OnLoad`, `JNI_CreateJavaVM` and other `JNI_`-prefixed helpers are plain
//! C and stay [`Scheme::C`], as do registration names passed to
//! `RegisterNatives`: those are ordinary identifiers, not mangled names.
//!
//! # Budgets
//!
//! Untrusted input stays bounded by
//! [`MAX_INPUT_LEN`](crate::jni::MAX_INPUT_LEN),
//! [`MAX_DEPTH`](crate::jni::MAX_DEPTH),
//! [`MAX_WORK`](crate::jni::MAX_WORK) and
//! [`MAX_OUTPUT_LEN`](crate::jni::MAX_OUTPUT_LEN). Exceeding a budget is an
//! error, never a partial result.
//!
//! The input, depth and work limits are runtime-configurable.
//! [`parse`](crate::jni::parse) keeps the compiled-in defaults, and a
//! [`crate::Demangler`] passes the configured budget, clamped to the hard
//! ceilings of 16 MiB input, 1024 array dimensions and 64M work units. A parsed
//! [`Type`](crate::jni::Type) stores its dimension count in a `u16`, comfortably
//! above the parser ceiling.
//! Input and work are independent: raising the input limit alone keeps the
//! default work allowance, so a caller that raises the input for a long symbol
//! must raise [`crate::Budget::work`] as well when the scan exceeds it. One
//! name holds one argument list, so there is no node dimension to configure.
//!
//! Printing is not a budget: the printer walks each node once, so the output is
//! linear in the parsed name and [`MAX_OUTPUT_LEN`](crate::jni::MAX_OUTPUT_LEN)
//! is a fixed backstop sized for the hard input ceiling. No accepted parse can
//! exceed it.
//!
//! # Gaps
//!
//! - The class/method boundary is the last unescaped `_`, so a symbol cannot
//!   distinguish a class whose path ends in several components from a method
//!   name; that ambiguity is inherent to the mangling and is resolved by the
//!   documented rule instead of a guess.
//! - A `_` followed by a digit is always an escape or a malformed escape: no
//!   Java identifier starts with a digit, so `_4` is rejected rather than read
//!   as a separator plus a name.
//! - Decoded characters are not validated as Java identifier characters; the
//!   mangling is defined on escape sequences, not on identifier syntax.
//! - The return type is absent from the symbol, so it is not printed.

mod node;
mod parser;
mod printer;

use crate::error::BudgetKind;
use core::fmt;

use crate::budget::{EffectiveBudget, Limit};
use crate::parse::LimitedWriter;
use node::ParseLimits;
pub use node::{
    Ast, Descriptor, Element, MAX_DEPTH, MAX_INPUT_LEN, MAX_OUTPUT_LEN, MAX_WORK, Primitive, Type,
};

/// Why a symbol could not be parsed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ParseError {
    /// The symbol does not match the JNI name grammar.
    Invalid,
    /// A descriptor nests arrays deeper than [`MAX_DEPTH`].
    RecursedTooDeep,
    /// The input, work or output budget was exceeded.
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
    /// Render the parsed name into `out`, enforcing [`MAX_OUTPUT_LEN`].
    pub(crate) fn print<W: fmt::Write + ?Sized>(&self, out: &mut W) -> fmt::Result {
        let mut limited = LimitedWriter::new(out, MAX_OUTPUT_LEN);
        printer::print(self, &mut limited)
    }
}

/// Parse a JNI native-method name into a reusable [`Ast`].
///
/// # Errors
///
/// Returns [`ParseError::Invalid`] when the prefix is missing, an escape is
/// truncated or malformed, a surrogate is unpaired, or a descriptor byte is
/// unknown; [`ParseError::RecursedTooDeep`] when a descriptor nests arrays
/// deeper than [`MAX_DEPTH`]; and [`ParseError::BudgetExceeded`] when the input
/// or work budget is exceeded.
pub fn parse(symbol: &str) -> Result<Ast<'_>, ParseError> {
    parse_bytes(symbol.as_bytes())
}

/// Parse a JNI native-method name from raw bytes.
///
/// This is the entry point the demangler dispatch uses once symbol bytes, not
/// text, are the input. See [`parse`] for the accepted forms.
pub(crate) fn parse_bytes(symbol: &[u8]) -> Result<Ast<'_>, ParseError> {
    parse_bytes_with_budget(symbol, DEFAULT_BUDGET)
}

/// The budget the public entry point starts from.
///
/// [`parse`] uses it, so it keeps exactly the compiled-in limits; a
/// [`crate::Demangler`] resolves its configured [`crate::Budget`] for
/// [`Scheme::Jni`](crate::Scheme::Jni) instead.
pub(crate) const DEFAULT_BUDGET: EffectiveBudget = EffectiveBudget {
    input_bytes: Some(Limit::Bounded(MAX_INPUT_LEN)),
    parse_depth: Some(Limit::Bounded(MAX_DEPTH)),
    work: Some(Limit::Bounded(MAX_WORK)),
    nodes: Some(Limit::Unbounded),
    expansion_visits: None,
    expansion_bytes: None,
};

/// Parse a JNI native-method name with one scheme's resolved budget.
///
/// [`parse_bytes`] wraps this with [`DEFAULT_BUDGET`]; a [`crate::Demangler`]
/// passes [`Demangler::effective_budget`](crate::Demangler::effective_budget)
/// for [`Scheme::Jni`](crate::Scheme::Jni), so a caller's overrides reach the
/// parser. The input, depth and work values are clamped to this module's hard
/// ceilings, and the grammar has no node dimension to resolve.
pub(crate) fn parse_bytes_with_budget(
    symbol: &[u8],
    budget: EffectiveBudget,
) -> Result<Ast<'_>, ParseError> {
    parser::parse_bytes(symbol, ParseLimits::from_effective(budget))
}

/// True when `symbol` is a fully valid JNI native-method name.
///
/// Detection validates the whole symbol rather than accepting the `Java_`
/// prefix, because ordinary C functions share it: a name that does not parse is
/// not JNI and stays [`crate::Scheme::C`].
#[must_use]
pub fn is_jni(symbol: &[u8]) -> bool {
    parse_bytes(symbol).is_ok()
}

#[cfg(test)]
mod tests {
    use alloc::string::{String, ToString};
    use alloc::vec::Vec;

    use super::*;

    #[test]
    fn parses_short_and_long_names() {
        for (symbol, expected) in [
            ("Java_com_example_Foo_plain", "com.example.Foo.plain"),
            ("Java_Foo_bar", "Foo.bar"),
            (
                "Java_com_example_Foo_bar___3Ljava_lang_String_2I",
                "com.example.Foo.bar(java.lang.String[], int)",
            ),
            ("Java_com_example_Foo_bar__", "com.example.Foo.bar()"),
            (
                "Java_com_example_Foo_under_1score__",
                "com.example.Foo.under_score()",
            ),
            ("Java_com_example_Weird_11_m__", "com.example.Weird_1.m()"),
        ] {
            assert_eq!(parse(symbol).unwrap().to_string(), expected, "{symbol}");
        }
    }

    #[test]
    fn decodes_every_escape() {
        for (symbol, expected) in [
            // `_1` is a literal underscore, in the class and in the method.
            ("Java_a_1b_c_1d", "a_b.c_d"),
            // `_0xxxx` decodes to one UTF-16 code unit.
            ("Java_p_T_baz_000e9", "p.T.bazé"),
            // Uppercase hexadecimal digits are the same escape.
            ("Java_p_T_baz_0004A", "p.T.bazJ"),
            // A surrogate pair decodes to one supplementary character.
            ("Java_p_T_baz_0d835_0dd38", "p.T.baz𝔸"),
            // `_2` terminates an object type and `_3` is one array dimension.
            (
                "Java_p_T_baz__Lq_1w_x_2_3_3I",
                "p.T.baz(q_w.x, int[][])",
            ),
            // A bare `_` in a signature is the binary-name separator.
            (
                "Java_p_T_baz__Ljava_lang_String_2",
                "p.T.baz(java.lang.String)",
            ),
        ] {
            assert_eq!(parse(symbol).unwrap().to_string(), expected, "{symbol}");
        }
    }

    #[test]
    fn rejects_malformed_names() {
        for symbol in [
            "",
            "Java_",
            "Java_foo",
            "Java__bar",
            "Java_foo_",
            "Java_foo_bar_0",
            "Java_foo_bar_0zzzz",
            "Java_foo_bar_0d835",
            "Java_foo_bar_0dd38",
            "Java_foo_bar_0d835_0000",
            "Java_foo_bar_0d835_z",
            "Java_foo_2bar_baz",
            "Java_foo_3bar_baz",
            "Java_foo_4",
            "Java_foo_bar__X",
            "Java_foo_bar__V",
            "Java_foo_bar__(I",
            "Java_foo_bar__L_2",
            "Java_foo_bar__Ljava",
            "Java_foo_bar__Ljava_lang_String",
            "Java_foo_bar__Ix",
            "Java_foo_bar__[I",
            "Java_foo_bar__(int)",
            "Java_com_example_Foo_plain()",
            "JNI_OnLoad",
            "JNI_CreateJavaVM",
            "jni_onload",
            "_Java_com_example_Foo_plain",
        ] {
            assert_eq!(parse(symbol).map(|_| ()), Err(ParseError::Invalid), "{symbol}");
            assert!(!is_jni(symbol.as_bytes()), "{symbol}");
        }
    }

    #[test]
    fn exposes_the_parsed_structure() {
        let ast = parse("Java_com_example_Foo_bar___3Ljava_lang_String_2I").unwrap();
        assert_eq!(ast.class(), "com.example.Foo");
        assert_eq!(ast.method(), "bar");
        let descriptor = ast.descriptor().unwrap();
        assert_eq!(descriptor.args().len(), 2);

        let array = &descriptor.args()[0];
        assert_eq!(array.dimensions(), 1);
        assert_eq!(
            array.element(),
            &Element::Object("java.lang.String".into())
        );
        assert_eq!(array.to_string(), "java.lang.String[]");

        let int = &descriptor.args()[1];
        assert_eq!(int.dimensions(), 0);
        assert_eq!(int.element(), &Element::Primitive(Primitive::Int));
        assert_eq!(int.to_string(), "int");
        assert_eq!(Primitive::Int.java_name(), "int");
        assert_eq!(Primitive::Boolean.java_name(), "boolean");

        // A short name carries no descriptor.
        assert!(parse("Java_com_example_Foo_plain").unwrap().descriptor().is_none());
    }

    #[test]
    fn budgets_are_enforced() {
        // An array nested deeper than MAX_DEPTH.
        let mut deep = String::from("Java_p_T_baz__");
        for _ in 0..=MAX_DEPTH {
            deep.push_str("_3");
        }
        deep.push('I');
        assert_eq!(parse(&deep).map(|_| ()), Err(ParseError::RecursedTooDeep));

        // One dimension less parses.
        let mut shallow = String::from("Java_p_T_baz__");
        for _ in 0..MAX_DEPTH {
            shallow.push_str("_3");
        }
        shallow.push('I');
        assert!(parse(&shallow).is_ok());

        // An input longer than MAX_INPUT_LEN.
        let mut long = String::from("Java_p_T_baz__");
        while long.len() <= MAX_INPUT_LEN {
            long.push('I');
        }
        assert_eq!(parse(&long).map(|_| ()), Err(ParseError::BudgetExceeded));
        assert!(!is_jni(long.as_bytes()));
    }

    #[test]
    fn output_stays_within_the_budget() {
        // The widest expansion is a boolean argument (`Z` -> `boolean, `).
        let mut symbol = String::from("Java_p_T_baz__");
        while symbol.len() < MAX_INPUT_LEN {
            symbol.push('Z');
        }
        let printed = parse(&symbol).unwrap().to_string();
        assert!(printed.len() <= MAX_OUTPUT_LEN);
        assert!(printed.starts_with("p.T.baz(boolean, boolean"));
    }

    #[test]
    fn never_panics_on_truncated_names() {
        for name in [
            "Java_com_example_Foo_bar___3Ljava_lang_String_2I",
            "Java_com_example_Foo_under_1score__",
            "Java_p_T_baz_0d835_0dd38",
            "Java_p_T_baz__Lq_1w_x_2_3_3I",
            "Java_p__0d835_0dd38_C_m",
        ] {
            for end in 0..=name.len() {
                if let Some(prefix) = name.get(..end) {
                    let _ = parse(prefix);
                    let _ = is_jni(prefix.as_bytes());
                }
            }
        }
    }

    #[test]
    fn never_panics_on_arbitrary_bytes() {
        const CHARSET: &[u8] = b"Jav_pTboZIBLV0123456789_()/;[]\xC3\x9F\xFF";
        let mut state = 0x1234_5678_u64;
        let mut next = || {
            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            (state >> 33) as usize
        };
        for _ in 0..20_000 {
            let length = next() % 40;
            let mut bytes = Vec::with_capacity(length);
            for _ in 0..length {
                bytes.push(CHARSET[next() % CHARSET.len()]);
            }
            let _ = parse_bytes(&bytes);
            let _ = is_jni(&bytes);
        }
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

    use super::node::{HARD_DEPTH, HARD_INPUT_LEN, HARD_WORK, ParseLimits};
    use super::*;
    use crate::budget::Limit;
    use crate::{Budget, Demangler, Scheme};

    /// Parse `symbol` with `budget` resolved for [`Scheme::Jni`].
    fn parse_with(symbol: &[u8], budget: Budget) -> Result<Ast<'_>, ParseError> {
        parse_bytes_with_budget(symbol, budget.effective(Scheme::Jni))
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

    /// A long-form name whose single argument is an `dimensions`-deep array.
    fn array_of(dimensions: usize) -> String {
        let mut symbol = String::from("Java_p_T_baz__");
        for _ in 0..dimensions {
            symbol.push_str("_3");
        }
        symbol.push('I');
        symbol
    }

    #[test]
    fn the_compiled_in_budget_matches_the_policy_defaults() {
        let limits = ParseLimits::from_effective(DEFAULT_BUDGET);
        assert_eq!(limits.input, MAX_INPUT_LEN);
        assert_eq!(limits.depth, MAX_DEPTH);
        assert_eq!(limits.work, MAX_WORK);
        assert_eq!(
            Scheme::Jni.default_budget().effective(Scheme::Jni),
            DEFAULT_BUDGET
        );

        // The policy table and this module agree on the ceilings. One name
        // holds one argument list, so there is no node dimension.
        let hard = Scheme::Jni.hard_budget();
        assert_eq!(hard.input_bytes, Some(Limit::Bounded(HARD_INPUT_LEN)));
        assert_eq!(hard.parse_depth, Some(Limit::Bounded(HARD_DEPTH)));
        assert_eq!(hard.work, Some(Limit::Bounded(HARD_WORK)));
        assert_eq!(hard.nodes, Some(Limit::Unbounded));
        assert_eq!(HARD_DEPTH, 1024);
    }

    #[test]
    fn each_dimension_rejects_one_unit_below_what_the_symbol_needs() {
        for symbol in [
            "Java_com_example_Foo_plain",
            "Java_com_example_Foo_bar__",
            "Java_com_example_Foo_bar___3Ljava_lang_String_2I",
            "Java_p_T_baz_0d835_0dd38",
        ] {
            let ceiling = symbol.len() * 2 + 64;
            assert_dimension_boundary(symbol, "input", ceiling, |value| {
                Budget::new().with_input_bytes(value)
            });
            assert_dimension_boundary(symbol, "work", ceiling, |value| {
                Budget::new().with_work(value)
            });
        }

        // Every `_3` is one array dimension.
        assert_dimension_boundary(&array_of(3), "depth", 8, |value| {
            Budget::new().with_parse_depth(value)
        });
    }

    #[test]
    fn raising_the_input_budget_accepts_a_longer_symbol() {
        let mut symbol = String::from("Java_p_T_");
        symbol.push_str(&"a".repeat(MAX_INPUT_LEN));
        assert!(matches!(
            parse_with(symbol.as_bytes(), Budget::new()),
            Err(ParseError::BudgetExceeded)
        ));
        let raised = Budget::new().with_input_bytes(2 * MAX_INPUT_LEN);
        let ast = parse_with(symbol.as_bytes(), raised).expect("raised input budget");
        assert_eq!(ast.to_string().len(), "p.T.".len() + MAX_INPUT_LEN);
    }

    #[test]
    fn raising_the_depth_budget_accepts_a_deeper_array() {
        // One dimension past the default array depth.
        let symbol = array_of(MAX_DEPTH + 1);
        assert!(matches!(
            parse_with(symbol.as_bytes(), Budget::new()),
            Err(ParseError::RecursedTooDeep)
        ));
        let raised = Budget::new().with_parse_depth(MAX_DEPTH + 1);
        let ast = parse_with(symbol.as_bytes(), raised).expect("raised depth budget");
        assert_eq!(
            ast.descriptor().expect("a long-form name").args()[0].dimensions(),
            u16::try_from(MAX_DEPTH + 1).unwrap()
        );
    }

    /// The depth ceiling is the generic hard cap; the AST stores the count in
    /// a `u16`, so every accepted depth is representable.
    #[test]
    fn the_depth_ceiling_is_the_generic_hard_cap() {
        let over_hard = Budget::new().with_parse_depth(usize::MAX);
        assert_eq!(
            over_hard.effective(Scheme::Jni).parse_depth,
            Some(Limit::Bounded(1024))
        );
        let limits = ParseLimits::from_effective(over_hard.effective(Scheme::Jni));
        assert_eq!(limits.depth, 1024);

        let deepest = array_of(1024);
        let ast = parse_with(deepest.as_bytes(), over_hard).expect("1024 dimensions fit a `u16`");
        assert_eq!(
            ast.descriptor().expect("a long-form name").args()[0].dimensions(),
            1024
        );

        let too_deep = array_of(1025);
        assert!(matches!(
            parse_with(too_deep.as_bytes(), over_hard),
            Err(ParseError::RecursedTooDeep)
        ));
    }

    /// The input and work dimensions are independent: raising the input alone
    /// keeps the default work allowance, so a scan that exceeds it still fails.
    #[test]
    fn raising_the_input_budget_does_not_raise_the_work_budget() {
        // The parser charges one work unit per input byte before splitting.
        let mut symbol = String::from("Java_p_T_baz");
        symbol.push_str(&"a".repeat(600_000));
        assert!(4 * symbol.len() > MAX_WORK);
        let work = 4 * symbol.len();

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
            .with_work(usize::MAX);
        let limits = ParseLimits::from_effective(budget.effective(Scheme::Jni));
        assert_eq!(limits.input, HARD_INPUT_LEN);
        assert_eq!(limits.work, HARD_WORK);
        assert_eq!(
            budget.effective(Scheme::Jni).input_bytes,
            Some(Limit::Bounded(HARD_INPUT_LEN))
        );

        // A name past the hard input cap stays rejected even when the caller
        // asks for more.
        let mut too_long = String::from("Java_p_T_");
        too_long.push_str(&"a".repeat(HARD_INPUT_LEN));
        assert!(matches!(
            parse_with(too_long.as_bytes(), budget),
            Err(ParseError::BudgetExceeded)
        ));
    }

    #[test]
    fn a_zero_budget_rejects_through_every_entry_point() {
        // A long-form name charges work and carries one array dimension, so
        // every dimension rejects it.
        let symbol = "Java_p_T_baz___3I";
        assert!(Demangler::new().demangle(symbol).is_ok());
        for budget in [
            Budget::new().with_input_bytes(0),
            Budget::new().with_work(0),
            Budget::new().with_parse_depth(0),
        ] {
            let demangler = Demangler::with_budget(budget);
            assert!(demangler.demangle(symbol).is_err(), "{budget:?}");
            assert!(
                demangler
                    .demangle_with_scheme(Scheme::Jni, symbol)
                    .is_err(),
                "{budget:?}"
            );
            assert!(
                demangler
                    .selection()
                    .allow(Scheme::Jni)
                    .demangle(symbol)
                    .is_err(),
                "{budget:?}"
            );
        }

        // A short name carries no descriptor, so the depth dimension does not
        // apply to it.
        let demangler = Demangler::with_budget(Budget::new().with_parse_depth(0));
        assert!(demangler.demangle("Java_com_example_Foo_plain").is_ok());
    }

    /// The fixed output cap is a backstop, not a print budget.
    ///
    /// The printer walks each node once, and the widest expansion is a
    /// primitive argument (`Z` becomes `boolean, `), so sixteen times the input
    /// bounds the text. The cap is sized from the hard input ceiling, so a name
    /// accepted under a raised input budget still fits.
    #[test]
    fn the_output_cap_is_a_backstop_the_printer_never_reaches() {
        let mut symbol = String::from("Java_p_T_baz__");
        symbol.push_str(&"Z".repeat(MAX_INPUT_LEN));
        let raised = Budget::new().with_input_bytes(2 * MAX_INPUT_LEN);
        let rendered = parse_with(symbol.as_bytes(), raised)
            .expect("the raised input budget parses")
            .to_string();
        assert!(rendered.len() <= MAX_OUTPUT_LEN);
        assert!(rendered.len() <= 16 * symbol.len());
    }

    #[test]
    fn the_default_budget_parses_the_corpus_like_parse() {
        let corpora = [
            "/refs/jni/corpus/hand.txt",
            "/refs/jni/corpus/jdk21-javac-h.txt",
            "/refs/jni/corpus/jdk21-libraries.txt",
            "/refs/jni/corpus/rejected.txt",
        ];
        let mut compared = 0_usize;
        for corpus in corpora {
            let path = format!("{}{corpus}", std::env!("CARGO_MANIFEST_DIR"));
            let Ok(data) = std::fs::read_to_string(&path) else {
                std::eprintln!("jni corpus not present (published crate); skipping");
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
