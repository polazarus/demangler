//! Boundary probes for the C++ parser-depth budgets.
//!
//! These are the shapes the adversarial corpus overshoots: a pointer chain of
//! `n` levels. The parser budgets (`256` for Itanium/MSVC, `64` for legacy)
//! stop the chain before the printer's larger depth cap, so the print-cap
//! difference is grounded in the parser's AST envelope rather than an
//! unexplained per-printer number.

#[cfg(feature = "cpp-itanium")]
#[test]
fn itanium_depth_budget_stops_at_254() {
    use demangle::cpp;
    let accepted = format!("_Z1f{}i", "P".repeat(254));
    let rejected = format!("_Z1f{}i", "P".repeat(255));
    assert!(cpp::parse(&accepted).is_ok());
    assert!(cpp::parse(&rejected).is_err());
}

#[cfg(feature = "cpp-gnu-v2")]
#[test]
fn legacy_depth_budget_stops_at_60() {
    use demangle::cpp;
    let accepted = format!("foo__F{}i", "P".repeat(60));
    let rejected = format!("foo__F{}i", "P".repeat(61));
    assert!(cpp::parse(&accepted).is_ok());
    assert!(cpp::parse(&rejected).is_err());
}

#[cfg(feature = "cpp-msvc")]
#[test]
fn msvc_depth_budget_stops_at_254() {
    use demangle::cpp;
    let accepted = format!("?x@@3{}HA", "PEA".repeat(254));
    let rejected = format!("?x@@3{}HA", "PEA".repeat(255));
    assert!(cpp::parse(&accepted).is_ok());
    assert!(cpp::parse(&rejected).is_err());
}
