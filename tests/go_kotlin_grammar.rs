//! Go and Kotlin/Native productions that the vendored corpora do not exercise.
//!
//! Go names print in their canonical spelling, so the expected text is the
//! input; Kotlin/Native prints the display form documented in
//! `refs/kotlin-native.md`.
#![cfg(all(feature = "go", feature = "kotlin-native"))]

use demangler::{Demangler, Scheme};

const CASES: &[(Scheme, &str, &str)] = &[
    // A GC program type descriptor.
    (Scheme::Go, "type:.gcprog.main.T", "type:.gcprog.main.T"),
    // Itabs whose concrete type nests brackets and braces around commas.
    (
        Scheme::Go,
        "go:itab.main.T[int,string],main.I",
        "go:itab.main.T[int,string],main.I",
    ),
    (
        Scheme::Go,
        "go:itab.main.T[map[string]int],main.I",
        "go:itab.main.T[map[string]int],main.I",
    ),
    (
        Scheme::Go,
        "go:itab.*main.T[a.B{x int}],main.I",
        "go:itab.*main.T[a.B{x int}],main.I",
    ),
    // Package-level closures and range-over-func bodies.
    (Scheme::Go, "main.glob..func1", "main.glob..func1"),
    (Scheme::Go, "main.main-range1", "main.main-range1"),
    (
        Scheme::Go,
        "main.main.func1-range2",
        "main.main.func1-range2",
    ),
    // Context parameters, with and without an extension receiver.
    (
        Scheme::KotlinNative,
        "kfun:com.example#foo(kotlin.Int)(kotlin.String){}kotlin.Unit",
        "com.example.foo context(kotlin.Int)(kotlin.String): kotlin.Unit",
    ),
    (
        Scheme::KotlinNative,
        "kfun:com.example#foo(kotlin.Int)@kotlin.Long(kotlin.String){}kotlin.Unit",
        "com.example.foo context(kotlin.Int)@kotlin.Long(kotlin.String): kotlin.Unit",
    ),
    (
        Scheme::KotlinNative,
        "kfun:com.example#foo(){}kotlin.Unit%linux_x64",
        "com.example.foo(): kotlin.Unit %linux_x64",
    ),
];

const REJECTED: &[(Scheme, &str)] = &[
    // A package closure needs a suffix of name characters.
    (Scheme::Go, "main.glob.."),
    (Scheme::Go, "main.glob..f$x"),
    // A range-over-func body is numbered.
    (Scheme::Go, "main.main-range"),
    // At most two parameter groups (context and value parameters).
    (
        Scheme::KotlinNative,
        "kfun:com.example#foo(kotlin.Int)(kotlin.String)(kotlin.Long){}kotlin.Unit",
    ),
    // One extension receiver.
    (
        Scheme::KotlinNative,
        "kfun:com.example#foo@kotlin.Int@kotlin.Long(){}kotlin.Unit",
    ),
    // `#internal` only ends a name; other markers are unknown.
    (
        Scheme::KotlinNative,
        "kfun:com.example#foo#internal(){}kotlin.Unit",
    ),
    (
        Scheme::KotlinNative,
        "kfun:com.example#foo#bogus(){}kotlin.Unit",
    ),
    // `#companion` names the companion's class.
    (
        Scheme::KotlinNative,
        "kfun:com.example#foo#companion(){}kotlin.Unit",
    ),
    // A platform suffix is not empty.
    (Scheme::KotlinNative, "kfun:com.example#foo(){}kotlin.Unit%"),
];

#[test]
fn rare_productions_print() {
    let demangler = Demangler::new();
    for &(scheme, symbol, expected) in CASES {
        let result = demangler
            .demangle_with_scheme(scheme, symbol)
            .unwrap_or_else(|error| panic!("{symbol}: {error}"));
        assert_eq!(result.to_string(), expected, "{symbol}");
    }
}

#[test]
fn malformed_productions_are_rejected() {
    let demangler = Demangler::new();
    for &(scheme, symbol) in REJECTED {
        assert!(
            demangler.demangle_with_scheme(scheme, symbol).is_err(),
            "{symbol} should be rejected"
        );
    }
}
