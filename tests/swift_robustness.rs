//! Robustness regressions for the in-tree Swift demangler.
#![cfg(feature = "swift")]

use std::fmt::Write as _;

use demangler::Demangler;

fn render(symbol: &str) -> Option<String> {
    let demangler = Demangler::new();
    let demangled = demangler.demangle(symbol).ok()?;
    let mut out = String::new();
    write!(out, "{demangled}").ok()?;
    Some(out)
}

#[test]
fn non_text_builtin_vector_does_not_panic() {
    // The element of `Builtin.Vec` is an integer literal, not a text node.
    let _ = render("$s$99999_Bv1_");
}

#[test]
fn shared_substitution_keeps_siblings() {
    // The same substituted type appears twice as a generic argument and once
    // more as a tuple element. Sharing the node must keep the earlier sibling
    // list intact rather than truncating it.
    let out = render("$s4main1AVSDyACAC_G_ACt").expect("demangles");
    assert!(
        out.contains("main.A, main.A"),
        "dropped a duplicated substitution: {out}"
    );
}

#[test]
fn trailing_substitution_index_26_is_accepted() {
    // `A_` is substitution 27 (the reference uses `-1 + 27`).
    let _ = render("$s1A1BA_");
}

#[test]
fn shared_substitution_display_is_stable() {
    let demangler = Demangler::new();
    let demangled = demangler
        .demangle("$s4main1AVSDyACAC_G_ACt")
        .expect("demangles");
    let first = demangled.to_string();
    assert_eq!(first, demangled.to_string());
    let mut out = String::new();
    write!(out, "{demangled}").expect("renders");
    assert_eq!(out, first);
}

#[test]
fn generic_signature_rejects_too_many_requirements() {
    // Sixteen requirements print; a seventeenth must be rejected instead of
    // being silently truncated.
    let accepted = format!("$s{}l", "SiRsz".repeat(16));
    assert!(render(&accepted).is_some());
    let rejected = format!("$s{}l", "SiRsz".repeat(17));
    assert!(render(&rejected).is_none());
}

#[test]
fn generic_signature_rejects_too_many_depth_groups() {
    // Eight depth groups print; a ninth must be rejected instead of being
    // silently truncated.
    let accepted = format!("$sr{}l", "z".repeat(8));
    assert!(render(&accepted).is_some());
    let rejected = format!("$sr{}l", "z".repeat(9));
    assert!(render(&rejected).is_none());
}

#[test]
fn legacy_nesting_is_bounded() {
    // Every `R` adds an `inout` layer; the depth budget must reject the name
    // instead of recursing without bound.
    let mut symbol = String::from("_Tt");
    symbol.push_str(&"R".repeat(200));
    symbol.push_str("Si");
    assert!(render(&symbol).is_none());
}

#[test]
fn legacy_generic_param_count_is_bounded() {
    // A malformed signature may claim an enormous number of parameters; the
    // printer stops after 128 and writes `...`.
    let symbol = format!("_Ttu{}_rFxSi", "9".repeat(20));
    let out = render(&symbol).expect("demangles");
    assert!(out.contains("..."), "{out}");
    assert!(
        out.len() < 2048,
        "unbounded generic parameter list: {}",
        out.len()
    );
}

#[test]
fn legacy_rejects_symbolic_references() {
    // Control bytes introduce symbolic references, which must be refused for
    // untrusted input.
    assert!(render("_Tt\u{1}0123").is_none());
}

#[test]
fn legacy_trailing_text_is_a_suffix() {
    assert_eq!(
        render("_TtSiXYZ").as_deref(),
        Some("Swift.Int with unmangled suffix \"XYZ\"")
    );
}

#[test]
fn legacy_long_payload_is_rejected() {
    // A specialization payload is re-demangled; one that looks like a symbol
    // but exceeds the payload bound must reject the whole name rather than
    // print the payload undemangled.
    let payload = format!("_TtSi{}", "A".repeat(300));
    let symbol = format!("_TTSf1cpfr{}_{payload}_TF3foofT_T_", payload.len());
    assert!(render(&symbol).is_none());
}

#[test]
fn byte_truncations_never_panic() {
    // Truncate at every byte, not just every char boundary: Swift lengths are
    // byte counts and the byte entry point accepts non-UTF-8 names.
    const SEEDS: &[&str] = &[
        "$s4main3fooyySiF",
        "$s4main1AVSDyACAC_G_ACt",
        "$s4main3fooyySiF.ig",
        "$s4main5föö",
        "$s1A1BA_",
        "_TtGSqGSaC5sugar7MyClass__",
        "_TtSiXYZ",
        "_TTSf1cpfr10abc_5abc_TF3foofT_T_",
    ];
    let demangler = Demangler::new();
    for &seed in SEEDS {
        for end in 0..=seed.len() {
            let _ = demangler.demangle_bytes(&seed.as_bytes()[..end]);
        }
    }
}
