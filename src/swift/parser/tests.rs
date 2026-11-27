use super::*;
use crate::parse::{Limits, State};
use crate::swift::DEFAULT_BUDGET;
use alloc::rc::Rc;

fn state() -> State {
    State::new(Limits {
        input: 64,
        depth: 4,
        work: 1024,
    })
}

fn parser<'a>(state: &mut State) -> Parser<'a, '_> {
    let cursor = Cursor::new(b"", 0, state).expect("empty input fits the budget");
    Parser::new(cursor, false, ParseLimits::from_effective(DEFAULT_BUDGET))
}

/// Render a stable Swift symbol, or `None` when it is rejected.
fn render(symbol: &str) -> Option<std::string::String> {
    let ast = crate::swift::parse(symbol).ok()?;
    Some(std::format!("{ast}"))
}

/// Find the first node of `kind` in the parsed tree.
fn find<'a>(ast: &crate::swift::Ast<'a>, kind: Kind) -> Option<NodeRef<'a>> {
    let mut stack = std::vec![ast.root()];
    while let Some(node) = stack.pop() {
        if node.kind() == kind {
            return Some(node);
        }
        stack.extend(node.children().iter().cloned());
    }
    None
}

#[test]
fn macro_expansion_roles_include_preamble() {
    // `fMq` is an attached role the grammar defines; an unknown role letter
    // must reject instead of defaulting to a known one.
    let rendered = render("$s1a1b1cfMq0_").expect("preamble macro expands");
    assert!(rendered.contains("preamble macro"), "{rendered}");
    assert!(render("$s1a1b1cfMx0_").is_none());
}

#[test]
fn layout_constraint_codes_are_validated() {
    let ast = crate::swift::parse("$s1a1bRlzE3_5_1c").expect("layout requirement parses");
    let node = find(&ast, Kind::DependentGenericLayoutRequirement).expect("layout node");
    assert_eq!(node.layout_code(), LayoutCode::TrivialExactSizeAndAlignment);
    assert_eq!(node.child_count(), 3, "subject, size and alignment");
    assert_eq!(node.child(1).map(|size| size.number_value()), Some(4));
    assert_eq!(
        node.child(2).map(|alignment| alignment.number_value()),
        Some(6)
    );

    let ast = crate::swift::parse("$s1a1bRlzS3_1c").expect("stride layout parses");
    let node = find(&ast, Kind::DependentGenericLayoutRequirement).expect("layout node");
    assert_eq!(node.layout_code(), LayoutCode::TrivialStride);
    assert_eq!(node.child_count(), 2, "subject and size");

    assert!(render("$s1a1bRlzx1c").is_none(), "unknown letters reject");
}

#[test]
fn differentiable_function_type_codes_are_validated() {
    for (symbol, code) in [
        ("$sSiyYjfc", AutoDiffCode::Forward),
        ("$sSiyYjrc", AutoDiffCode::Reverse),
        ("$sSiyYjdc", AutoDiffCode::Differential),
        ("$sSiyYjlc", AutoDiffCode::Linear),
    ] {
        let ast = crate::swift::parse(symbol).expect("annotation parses");
        let node = find(&ast, Kind::DifferentiableFunctionType).expect("annotation node");
        assert_eq!(node.autodiff_code(), code);
    }
    // `p` belongs to `AutoDiffFunctionKind` only; other bytes are invalid.
    assert!(render("$sSiyYjpc").is_none());
    assert!(render("$sSiyYjXc").is_none());
}

#[test]
fn specialization_kinds_are_closed() {
    assert_eq!(
        specialization_kind(b'g'),
        Some(GenericSpecializationKind::Generic)
    );
    assert_eq!(
        specialization_kind(b'G'),
        Some(GenericSpecializationKind::NotReabstracted)
    );
    assert_eq!(
        specialization_kind(b'B'),
        Some(GenericSpecializationKind::ResilienceDomain)
    );
    assert_eq!(
        specialization_kind(b's'),
        Some(GenericSpecializationKind::Prespecialized)
    );
    assert_eq!(
        specialization_kind(b'i'),
        Some(GenericSpecializationKind::Inlined)
    );
    for code in *b"tfpPx" {
        assert_eq!(specialization_kind(code), None, "{code:?}");
    }
}

#[test]
fn representation_changed_flag_is_recorded() {
    let ast = crate::swift::parse("$s1a1b1cSiyTgr5").expect("specialization parses");
    let node = find(&ast, Kind::GenericSpecialization).expect("specialization node");
    assert_eq!(
        node.generic_specialization(),
        Some((GenericSpecializationKind::Generic, true))
    );

    let ast = crate::swift::parse("$s1a1b1cSiyTfr5").expect("specialization parses");
    let node = find(&ast, Kind::FunctionSignatureSpecialization).expect("spec node");
    assert!(node.function_signature_representation_changed());

    assert!(
        render("$s1a1b1cSiyTp5").is_none(),
        "partial specializations reject"
    );
}

/// Push `elements` nodes below `marker`, so the loop pops them in reverse.
fn push_list(parser: &mut Parser<'_, '_>, elements: usize) {
    let bottom = parser.alloc(Kind::Identifier).unwrap();
    let marker = parser.alloc(Kind::FirstElementMarker).unwrap();
    parser.stack.push(bottom);
    parser.stack.push(marker);
    for _ in 1..elements {
        let node = parser.alloc(Kind::Identifier).unwrap();
        parser.stack.push(node);
    }
}

#[test]
fn reverse_list_pops_elements_in_reverse_order() {
    let mut state = state();
    let mut parser = parser(&mut state);
    let first = parser.alloc(Kind::Identifier).unwrap();
    let marker = parser.alloc(Kind::FirstElementMarker).unwrap();
    let last = parser.alloc(Kind::Identifier).unwrap();
    parser.stack.push(first.clone());
    parser.stack.push(marker);
    parser.stack.push(last.clone());

    let elements = parser
        .pop_reverse_list(Parser::pop_any)
        .expect("well-formed list");
    assert_eq!(elements.len(), 2);
    assert!(Rc::ptr_eq(&elements[0], &last));
    assert!(Rc::ptr_eq(&elements[1], &first));
}

#[test]
fn reverse_list_length_is_bounded_by_the_node_budget_alone() {
    let mut state = state();
    let mut parser = parser(&mut state);

    push_list(&mut parser, 100);
    let elements = parser
        .pop_reverse_list(Parser::pop_any)
        .expect("a list has no length cap of its own");
    assert_eq!(elements.len(), 100);
}
