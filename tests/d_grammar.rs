//! D productions that the vendored corpora do not exercise.
//!
//! Function attributes, calling conventions, parameter storage classes and
//! string template arguments with escapes and wide encodings. Each expected
//! text is what `c++filt -s dlang` prints, except `return scope`, which the
//! D ABI specifies and the binutils 2.44 oracle predates.
#![cfg(feature = "d")]

use demangler::{Demangler, Scheme};

const CASES: &[(&str, &str)] = &[
    ("_D3foo3barFNcZi", "foo.bar()"),
    ("_D3foo3barFNdZi", "foo.bar()"),
    ("_D3foo3barFNjZi", "foo.bar()"),
    ("_D3foo3barFNlZi", "foo.bar()"),
    ("_D3foo3barFNmZi", "foo.bar()"),
    ("_D3foo3barFNaNbNiNfZv", "foo.bar()"),
    ("_D3foo3barYZv", "foo.bar()"),
    ("_D3foo3barUZv", "foo.bar()"),
    ("_D3foo3barWZv", "foo.bar()"),
    ("_D3foo3barRZv", "foo.bar()"),
    (
        "_D3foo__T3barVAyaa3_616263Z3bazFZv",
        "foo.bar!(\"abc\").baz()",
    ),
    (
        "_D3foo__T3barVAyaa3_090d0cZ3bazFZv",
        "foo.bar!(\"\\t\\r\\f\").baz()",
    ),
    (
        "_D3foo__T3barVAyaa2_0b0aZ3bazFZv",
        "foo.bar!(\"\\v\\n\").baz()",
    ),
    (
        "_D3foo__T3barVAyaa1_00Z3bazFZv",
        "foo.bar!(\"\\x00\").baz()",
    ),
    (
        "_D3foo__T3barVAyuw2_6100Z3bazFZv",
        "foo.bar!(\"a\\x00\"w).baz()",
    ),
    ("_D3foo__T3barVAywd1_61Z3bazFZv", "foo.bar!(\"a\"d).baz()"),
    ("_D3foo__T3barVii5Z3bazFZv", "foo.bar!(5).baz()"),
    ("_D3foo__T3barViN5Z3bazFZv", "foo.bar!(-5).baz()"),
    (
        "_D3foo__T3barVAyaa3_414243Z3bazFZv",
        "foo.bar!(\"ABC\").baz()",
    ),
    (
        "_D3foo__T3barVAyaa3_7f8081Z3bazFZv",
        "foo.bar!(\"\\x7f\\x80\\x81\").baz()",
    ),
    ("_D3foo3barFZv", "foo.bar()"),
    ("_D3foo3barFNnZv", "foo.bar(typeof(*null))"),
    ("_D3foo3barFNkKiZv", "foo.bar(return ref int)"),
    ("_D3foo3barFJiZv", "foo.bar(out int)"),
    ("_D3foo3barFLiZv", "foo.bar(lazy int)"),
    ("_D3foo3barFMiZv", "foo.bar(scope int)"),
    ("_D3foo3barFxiZv", "foo.bar(const(int))"),
    ("_D3foo3barFyiZv", "foo.bar(immutable(int))"),
    ("_D3foo3barFOiZv", "foo.bar(shared(int))"),
    ("_D3foo3barFNgiZv", "foo.bar(inout(int))"),
    ("_D3foo3barFNkMiZv", "foo.bar(return scope int)"),
];

/// Inputs both this crate and the oracle reject.
const REJECTED: &[&str] = &[
    "_D3foo__T3barVxw1_61Z3bazFZv",
    "_D3foo__T3barVixN5Z3bazFZv",
    "_D3foo3barFMZv",
    "_D3foo3barFNgZv",
    "_D3foo3barFZNnv",
];

#[test]
fn rare_productions_print() {
    let demangler = Demangler::new();
    for &(symbol, expected) in CASES {
        let result = demangler
            .demangle_with_scheme(Scheme::D, symbol)
            .unwrap_or_else(|error| panic!("{symbol}: {error}"));
        assert_eq!(result.to_string(), expected, "{symbol}");
    }
}

#[test]
fn malformed_productions_are_rejected() {
    let demangler = Demangler::new();
    for &symbol in REJECTED {
        assert!(
            demangler.demangle_with_scheme(Scheme::D, symbol).is_err(),
            "{symbol} should be rejected"
        );
    }
}
