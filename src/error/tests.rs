use std::string::ToString;

use super::*;

/// A parse error of `kind` for `scheme`, with an optional offset.
fn parse(scheme: Scheme, kind: ParseErrorKind, offset: Option<usize>) -> Error {
    let error = offset.map_or_else(
        || ParseError::new(kind),
        |offset| ParseError::at(kind, offset),
    );
    Error::Parse { scheme, error }
}

#[test]
fn reports_the_failing_scheme() {
    let invalid = parse(Scheme::RustV0, ParseErrorKind::Invalid, None);
    assert_eq!(invalid.scheme(), Some(Scheme::RustV0));
    assert_eq!(invalid.budget_kind(), None);
    let budget = parse(Scheme::D, ParseErrorKind::Budget(BudgetKind::Input), None);
    assert_eq!(budget.scheme(), Some(Scheme::D));
    assert_eq!(budget.budget_kind(), Some(BudgetKind::Input));
    assert_eq!(
        budget.parse_error().map(|error| error.kind()),
        Some(ParseErrorKind::Budget(BudgetKind::Input))
    );
    assert_eq!(Error::NoMatchingScheme.scheme(), None);
    assert_eq!(Error::NoMatchingScheme.parse_error(), None);
    assert_eq!(Error::NoMatchingScheme.budget_kind(), None);
}

#[test]
fn displays_every_error_shape() {
    assert_eq!(
        parse(Scheme::CppItanium, ParseErrorKind::UnexpectedEnd, Some(7)).to_string(),
        "invalid C++ symbol: unexpected end of input at byte 7"
    );
    assert_eq!(
        parse(Scheme::Go, ParseErrorKind::Invalid, None).to_string(),
        "invalid Go symbol: invalid mangling"
    );
    assert_eq!(
        parse(Scheme::D, ParseErrorKind::Budget(BudgetKind::Work), None).to_string(),
        "D symbol exceeded its work budget"
    );
    assert_eq!(
        Error::NoMatchingScheme.to_string(),
        "no selected scheme could demangle the symbol"
    );
    for (kind, text) in [
        (ParseErrorKind::NotThisScheme, "not in this scheme"),
        (ParseErrorKind::Unsupported, "unsupported production"),
    ] {
        assert_eq!(ParseError::new(kind).to_string(), text);
    }
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
    ] {
        assert_eq!(kind.as_str(), name);
    }
    assert_eq!(
        parse(
            Scheme::RustV0,
            ParseErrorKind::Budget(BudgetKind::ExpansionBytes),
            None
        )
        .to_string(),
        "Rust symbol exceeded its expansion bytes budget"
    );
}

#[test]
fn budget_kind_displays_its_name() {
    assert_eq!(
        BudgetKind::ParseDepth.to_string(),
        BudgetKind::ParseDepth.as_str()
    );
}

/// Every parser production returns a `ParseError`, so it stays as small as
/// the cursor's internal failure.
#[test]
fn parse_errors_stay_small() {
    assert!(size_of::<ParseError>() <= 8);
    assert_eq!(
        ParseError::at(ParseErrorKind::Invalid, usize::MAX).offset(),
        Some(u32::MAX as usize - 1)
    );
}
