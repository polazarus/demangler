//! Rust v0 and legacy productions that the vendored corpus does not exercise.
//!
//! Each symbol is compared against the `rustc-demangle` oracle at test time:
//! both demanglers must print the same text, or both must reject.
#![cfg(feature = "rust")]

use demangler::{Demangler, Scheme};

const SYMBOLS: &[&str] = &[
    // A `dyn` bound with a named lifetime.
    "_RINvC1a1fFG_DNtC1b1TEL0_EuE",
    // Binders past `'z` name their lifetimes `'_26`, `'_27`, ...
    "_RINvC1a1fFGq_RL0_uEuE",
    "_RINvC1a1fFGq_RLq_uEuE",
    "_RINvC1a1fFGr_RLr_RL0_uEuE",
    "_RINvC1a1fFG0_RL0_RL1_uEuE",
    // Punycode identifiers, short and long enough to adapt the bias.
    "_RNvC1au5a_zzz",
    "_RNvC1au42ncd_ame_with_many_ccents__3bc1uxaa6mscui0h",
    "_RNvC1au39_28jaa8cbb9dcc9add17bee7891jfaf4281qgag",
    // Malformed punycode: an empty encoded part, or a bad digit sequence.
    "_RNvC1au3ab_",
    "_RNvC1au2_b",
    "_RNvC1au3_99",
    "_RNvC1au4_999",
    // An erased const placeholder, and constants of the wrong shape.
    "_RINvC1a1fKpE",
    "_RINvC1a1fKVNtC1a1SE",
    "_RINvC1a1fKVNtC1a1SzE",
    "_RINvC1a1fWiPE",
    // Legacy `$u` escapes need hex digits.
    "_ZN3foo$u$17h0123456789abcdefE",
    "_ZN3foo$u20$3bar17h0123456789abcdefE",
];

#[test]
fn rare_productions_match_the_oracle() {
    let demangler = Demangler::new();
    for &symbol in SYMBOLS {
        let ours = demangler
            .demangle_with_scheme(Scheme::RustV0, symbol)
            .or_else(|_| demangler.demangle_with_scheme(Scheme::RustLegacy, symbol))
            .ok()
            .map(|demangled| demangled.to_string());
        let oracle = rustc_demangle::try_demangle(symbol)
            .ok()
            .map(|demangled| format!("{demangled}"));
        assert_eq!(ours, oracle, "{symbol}");
    }
}
