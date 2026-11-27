use super::*;

#[cfg(feature = "cpp-gnu-v2")]
#[test]
fn legacy_cpp_detection_retains_the_tree() {
    let detection = detect(b"foo__Fiv");
    assert_eq!(detection.scheme, Scheme::CppGnuV2);
    assert!(detection.parsed.is_some());
    assert_eq!(detection.symbol, b"foo__Fiv");
    assert_eq!(detection.parse_symbol, b"foo__Fiv");
}

#[cfg(feature = "d")]
#[test]
fn d_detection_retains_the_tree() {
    let detection = detect(b"_D3foo3barFZv");
    assert_eq!(detection.scheme, Scheme::D);
    assert!(detection.parsed.is_some());
}

#[cfg(feature = "d")]
#[test]
fn a_tiny_budget_keeps_the_detected_scheme_for_the_dispatch_reparse() {
    // The configured parse fails, so classification falls back to the
    // default limits; the retained tree lets dispatch re-apply the budget
    // and reject instead of silently treating the name as plain C.
    let detection = detect_with(b"_D3foo3barFZv", Budget::new().with_input_bytes(0));
    assert_eq!(detection.scheme, Scheme::D);
    assert!(detection.parsed.is_some());
}

#[cfg(feature = "ada")]
#[test]
fn a_raised_input_budget_reaches_a_long_ada_prefilter() {
    use crate::ada::MAX_INPUT_LEN;
    use alloc::format;

    let symbol = format!("ada__{}", "a".repeat(MAX_INPUT_LEN + 1));
    assert_eq!(detect(symbol.as_bytes()).scheme, Scheme::C);
    let budget = Budget::new().with_input_bytes(2 * MAX_INPUT_LEN);
    assert_eq!(detect_with(symbol.as_bytes(), budget).scheme, Scheme::Ada);
}

#[cfg(feature = "swift")]
#[test]
fn legacy_swift_detection_retains_the_tree() {
    let detection = detect(b"_TtSi");
    assert_eq!(detection.scheme, Scheme::SwiftLegacy);
    assert!(detection.parsed.is_some());
}

#[test]
fn dispatch_view_keeps_stripped_suffixes() {
    let detection = detect(b"main.llvm.9D1C9369");
    assert_eq!(detection.scheme, Scheme::C);
    assert_eq!(detection.symbol, b"main.llvm.9D1C9369");
    assert_eq!(detection.parse_symbol, b"main");
}

#[cfg(feature = "coff")]
#[cfg(feature = "cpp-gnu-v2")]
#[test]
fn coff_detection_retains_the_inner_tree_and_outer_decoration() {
    let detection = detect(b"__imp_foo__Fiv");
    assert_eq!(detection.scheme, Scheme::CppGnuV2);
    assert!(detection.parsed.is_some());
    assert_eq!(detection.symbol, b"foo__Fiv");
    assert_eq!(detection.parse_symbol, b"foo__Fiv");
    assert!(detection.decoration.is_some());
}
