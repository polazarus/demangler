//! Seed-based robustness coverage for the byte-native backends that
//! `tests/robustness.rs` cannot reach.
//!
//! `tests/robustness.rs` fuzzes every enabled parser with random token and
//! character streams. Its alphabet cannot form a D, JNI, Ada, Fortran, Delphi,
//! OCaml or PE/COFF name often enough to be meaningful, and inflating the
//! shared alphabet is not a bounded way to fix that. This file mutates a small
//! seed table instead: every byte-prefix truncation, one-byte substitution of
//! each byte class at every position, one invalid-UTF-8 insertion, and a
//! doubled `_`/`__`/`__imp_` prefix.
//!
//! Every case goes through the public byte API ([`Demangler::demangle_bytes`]),
//! so detection, dispatch, parsing and printing are all exercised. The
//! assertions are the invariants that hold for untrusted input:
//!
//! - no parser or printer panics;
//! - a successful render stays inside the 1 MiB backend print budget;
//! - valid UTF-8 produces identical text through [`Demangler::demangle`].
//!
//! The adversarial corpus harness (`tests/adversarial_corpus.rs`) owns the
//! 2-second/8 MiB timeout bound for oversized bombs. This file bounds its own
//! work by construction: the seed table is constant, mutations cover at most
//! [`MAX_MUTATED_BYTES`] positions and [`SUBSTITUTIONS`] bytes per position,
//! and no case is longer than its seed plus one byte.

#![cfg(any(
    feature = "d",
    feature = "jni",
    feature = "ada",
    feature = "fortran",
    feature = "delphi",
    feature = "ocaml",
    feature = "coff"
))]

use demangler::{Demangler, Scheme};

/// Every backend's declared print budget is at most 1 MiB. Rust's output
/// budget is 1,000,000 bytes plus the `{size limit reached}` marker, which
/// still fits under this ceiling.
const MAX_OUTPUT_BYTES: usize = 1 << 20;

/// Longest seed prefix that is mutated. Longer seeds are still truncated and
/// doubled in full; only the per-position substitution loop is capped.
const MAX_MUTATED_BYTES: usize = 64;

/// One byte per byte class: NUL, whitespace, every grammar separator and
/// marker these backends use, a digit, an uppercase and a lowercase letter, and
/// a high byte from each UTF-8 lead/invalid class.
const SUBSTITUTIONS: &[u8] = &[
    0x00, b'\t', b' ', b'$', b'@', b'_', b'.', b'0', b'A', b'a', b'?', b':', b'[', b'{', b'<',
    b'*', b'#', 0x7f, 0x80, 0xc3, 0xff,
];

/// Prefixes that a linker, a Mach-O symbol table or an import library can add.
/// Duplicating them exercises the normalization paths these backends share.
const DOUBLED_PREFIXES: &[&[u8]] = &[b"_", b"__", b"__imp_"];

/// Demangle one byte string and check the shared invariants.
fn check_bytes(symbol: &[u8]) {
    let demangler = Demangler::new();
    let _ = Scheme::detect_bytes(symbol);
    let Ok(demangled_symbol) = demangler.demangle_bytes(symbol) else {
        return;
    };
    let rendered = demangled_symbol.to_string();
    assert!(
        rendered.len() <= MAX_OUTPUT_BYTES,
        "{symbol:?} rendered {} bytes",
        rendered.len()
    );
    if let Ok(text) = core::str::from_utf8(symbol) {
        let from_text = demangler
            .demangle(text)
            .map(|demangled_symbol| (demangled_symbol.scheme(), demangled_symbol.to_string()));
        assert_eq!(
            from_text,
            Ok((demangled_symbol.scheme(), rendered)),
            "{text:?}: the text and byte APIs disagree"
        );
    }
}

/// Apply every mutation to `seed` and check each result.
fn sweep(seeds: &[&[u8]]) {
    for seed in seeds {
        // Byte-prefix truncations, not character boundaries: the parsers take
        // bytes, so a truncated multi-byte sequence is a valid input.
        for end in 0..=seed.len() {
            check_bytes(&seed[..end]);
        }
        let positions = seed.len().min(MAX_MUTATED_BYTES);
        for position in 0..positions {
            for &byte in SUBSTITUTIONS {
                let mut mutated = seed.to_vec();
                mutated[position] = byte;
                check_bytes(&mutated);
            }
            let mut inserted = seed.to_vec();
            inserted.insert(position, 0xff);
            check_bytes(&inserted);
        }
        for prefix in DOUBLED_PREFIXES {
            let mut doubled = prefix.to_vec();
            doubled.extend_from_slice(seed);
            check_bytes(&doubled);
        }
    }
}

/// Assert that a valid seed is still claimed by its backend and renders the
/// documented text, so a detection regression cannot make the mutation sweep
/// pass vacuously.
fn assert_claimed(symbol: &[u8], scheme: Scheme, expected: &str) {
    let demangled = Demangler::new()
        .demangle_bytes(symbol)
        .unwrap_or_else(|error| panic!("{symbol:?}: {error}"));
    assert_eq!(demangled.scheme(), scheme, "{symbol:?}");
    assert_eq!(demangled.to_string(), expected, "{symbol:?}");
}

/// D ABI seeds: a qualified name with a function type, a name with invalid
/// identifier bytes, a truncated type and a nested function.
#[cfg(feature = "d")]
const D_SEEDS: &[&[u8]] = &[
    b"_D3foo3barFZv",
    b"_D8foo\xff\xfebarFZv",
    b"_D3foo3barFZ",
    b"_D3app5outerFiZ5innerMFNaNbNiNfiZi",
];

#[cfg(feature = "d")]
#[test]
fn d_seeds_survive_mutations() {
    assert_claimed(D_SEEDS[0], Scheme::D, "foo.bar()");
    sweep(D_SEEDS);
}

/// JNI seeds: a long overloaded name, an unknown descriptor byte, a decoded
/// surrogate pair and an invalid identifier byte.
#[cfg(feature = "jni")]
const JNI_SEEDS: &[&[u8]] = &[
    b"Java_com_example_Foo_bar___3Ljava_lang_String_2I",
    b"Java_com_example_Foo_bar__X",
    b"Java_p_T_baz_0d835_0dd38",
    b"Java_com_example_Foo_b\xffar__I",
];

#[cfg(feature = "jni")]
#[test]
fn jni_seeds_survive_mutations() {
    assert_claimed(
        JNI_SEEDS[0],
        Scheme::Jni,
        "com.example.Foo.bar(java.lang.String[], int)",
    );
    sweep(JNI_SEEDS);
}

/// Ada/GNAT seeds: a high-confidence prefix, an unknown compiler suffix, an
/// invalid identifier byte and a protected operation.
#[cfg(feature = "ada")]
const ADA_SEEDS: &[&[u8]] = &[
    b"ada__exceptions__triggered_by_abort",
    b"ada__foo__barX",
    b"ada__foo__b\xffar",
    b"pkg__prot__setN",
];

#[cfg(feature = "ada")]
#[test]
fn ada_seeds_survive_mutations() {
    assert_claimed(
        ADA_SEEDS[0],
        Scheme::Ada,
        "ada.exceptions.triggered_by_abort",
    );
    sweep(ADA_SEEDS);
}

/// gfortran seeds: the detected module form, a generated helper the backend
/// rejects, an invalid identifier byte and a forced-only external name.
#[cfg(feature = "fortran")]
const FORTRAN_SEEDS: &[&[u8]] = &[
    b"__fprobe_MOD_add",
    b"__fprobe_MOD___def_init_x",
    b"__fprobe_MOD_a\xffd",
    b"ext_sub_",
];

#[cfg(feature = "fortran")]
#[test]
fn fortran_seeds_survive_mutations() {
    assert_claimed(FORTRAN_SEEDS[0], Scheme::Fortran, "fprobe.add");
    sweep(FORTRAN_SEEDS);
}

/// Free Pascal seeds: a unit procedure, the rejected nested-owner form, an
/// invalid identifier byte and a program-scope routine.
#[cfg(feature = "delphi")]
const DELPHI_SEEDS: &[&[u8]] = &[
    b"DELPHI_PROBE_$$_UNITPROC$LONGINT",
    b"DELPHI_PROBE$_$TPROBECLASS_$$_LOCALPROC$LONGINT",
    b"DELPHI_PROBE_$$_UNITPROC$LONGIN\xffT",
    b"P$DELPHI_PROBE_PROG_$$_PROGPROC$LONGINT",
];

#[cfg(feature = "delphi")]
#[test]
fn delphi_seeds_survive_mutations() {
    assert_claimed(
        DELPHI_SEEDS[0],
        Scheme::Delphi,
        "DELPHI_PROBE.UNITPROC(LONGINT)",
    );
    sweep(DELPHI_SEEDS);
}

/// OCaml seeds: a value with a stamp, a rejected bare-number member, an
/// invalid path byte and an escaped operator member.
#[cfg(feature = "ocaml")]
const OCAML_SEEDS: &[&[u8]] = &[
    b"camlStdlib__List.map_569",
    b"camlStdlib.11",
    b"camlPro\xffbe",
    b"camlOps.$2b$2b_270",
];

#[cfg(feature = "ocaml")]
#[test]
fn ocaml_seeds_survive_mutations() {
    assert_claimed(OCAML_SEEDS[0], Scheme::OCaml, "Stdlib__List.map");
    sweep(OCAML_SEEDS);
}

/// PE/COFF seeds: a stdcall decoration, an import, a decorated name with an
/// invalid byte, a decorated D symbol and a malformed import marker.
#[cfg(feature = "coff")]
const COFF_SEEDS: &[&[u8]] = &[
    b"_foo@8",
    b"__imp__foo@4",
    b"_f\xffoo@8",
    b"_D3foo3barFZv@16",
    b"__imp__",
];

#[cfg(feature = "coff")]
#[test]
fn coff_seeds_survive_mutations() {
    // The decorated name is peeled, so the inner `foo` is plain C.
    assert_claimed(COFF_SEEDS[0], Scheme::C, "foo");
    sweep(COFF_SEEDS);
}
