//! Robustness regressions for the in-tree Swift demangler.

use std::fmt::Write as _;

use demangle::Demangler;

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
