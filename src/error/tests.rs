use std::string::ToString;

use super::*;

#[test]
fn reports_the_failing_scheme() {
    assert_eq!(
        Error::InvalidSymbol(Scheme::RustV0).scheme(),
        Scheme::RustV0
    );
    assert_eq!(
        Error::Budget {
            scheme: Scheme::D,
            kind: BudgetKind::Input
        }
        .scheme(),
        Scheme::D
    );
    assert_eq!(Error::NoMatchingScheme.scheme(), Scheme::Unknown);
}

#[test]
fn displays_both_error_variants() {
    assert_eq!(
        Error::InvalidSymbol(Scheme::CppItanium).to_string(),
        "invalid C++ symbol"
    );
    assert_eq!(
        Error::Budget {
            scheme: Scheme::D,
            kind: BudgetKind::Work
        }
        .to_string(),
        "D symbol exceeded its work budget"
    );
    assert_eq!(
        Error::Budget {
            scheme: Scheme::Jni,
            kind: BudgetKind::Other
        }
        .budget_kind(),
        Some(BudgetKind::Other)
    );
    assert_eq!(
        Error::NoMatchingScheme.to_string(),
        "no selected scheme could demangle the symbol"
    );
}

#[test]
fn names_every_budget_dimension() {
    for (kind, name) in [
        (BudgetKind::Input, "input"),
        (BudgetKind::ParseDepth, "parse depth"),
        (BudgetKind::Work, "work"),
        (BudgetKind::Nodes, "nodes"),
        (BudgetKind::ExpansionVisits, "expansion visits"),
        (BudgetKind::ExpansionBytes, "expansion bytes"),
        (BudgetKind::Other, "parser"),
    ] {
        assert_eq!(kind.as_str(), name);
    }
    assert_eq!(
        Error::Budget {
            scheme: Scheme::RustV0,
            kind: BudgetKind::ExpansionBytes
        }
        .to_string(),
        "Rust symbol exceeded its expansion bytes budget"
    );
}
