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
    // more as a tuple element. Re-adding the shared node must clone it rather
    // than truncate the earlier sibling list.
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
