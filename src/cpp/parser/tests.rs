use alloc::string::String;
use alloc::vec::Vec;

use super::super::{DEFAULT_BUDGET, Kind, MAX_INPUT_LEN, NodeRef, itanium_state};
use super::*;
use crate::cpp::parse_bytes_with_budget;

fn root_kind(symbol: &str) -> Kind {
    parse_bytes_with_budget(symbol.as_bytes(), DEFAULT_BUDGET)
        .unwrap()
        .root()
        .kind()
}

fn child_kinds(node: &NodeRef<'_>) -> Vec<Kind> {
    node.children().iter().map(|child| child.kind()).collect()
}

fn find_text<'a>(node: &NodeRef<'a>, text: &str) -> Option<NodeRef<'a>> {
    if node.text() == Some(text) {
        return Some(Rc::clone(node));
    }
    node.children()
        .iter()
        .find_map(|child| find_text(child, text))
}

/// Find the first node of `kind` in the graph.
fn find_kind<'a>(node: &NodeRef<'a>, kind: Kind) -> Option<NodeRef<'a>> {
    if node.kind() == kind {
        return Some(Rc::clone(node));
    }
    node.children()
        .iter()
        .find_map(|child| find_kind(child, kind))
}

/// True when the graph holds a nested name qualified by a template-id.
fn has_nested_template_id(node: &NodeRef<'_>) -> bool {
    (node.kind() == Kind::NestedName
        && node
            .children()
            .iter()
            .any(|child| child.kind() == Kind::Name && child.child_count() == 2))
        || node.children().iter().any(has_nested_template_id)
}

fn count_nodes(node: &NodeRef<'_>) -> usize {
    1 + node.children().iter().map(count_nodes).sum::<usize>()
}

#[test]
fn parses_data_name() {
    let ast = parse_bytes_with_budget(b"_Z1A", DEFAULT_BUDGET).unwrap();
    let encoding = ast.root();
    assert_eq!(encoding.kind(), Kind::Encoding);
    let data = encoding.child(0).unwrap();
    assert_eq!(data.kind(), Kind::Data);
    let name = data.child(0).unwrap();
    assert_eq!(name.kind(), Kind::UnqualifiedName);
    assert_eq!(name.child(0).unwrap().kind(), Kind::Identifier);
    assert_eq!(name.child(0).unwrap().text(), Some("A"));
}

#[test]
fn parses_function_with_parameters() {
    let ast = parse_bytes_with_budget(b"_Z1A1B1C", DEFAULT_BUDGET).unwrap();
    let function = ast.root().child(0).unwrap();
    assert_eq!(function.kind(), Kind::Function);
    assert_eq!(
        child_kinds(&function),
        alloc::vec![Kind::UnqualifiedName, Kind::ParameterList]
    );
    let parameters = function.child(1).unwrap();
    assert_eq!(parameters.child_count(), 2);
    assert_eq!(parameters.child(0).unwrap().kind(), Kind::Parameter);
    assert_eq!(
        parameters
            .child(0)
            .unwrap()
            .child(0)
            .unwrap()
            .child(0)
            .unwrap()
            .text(),
        Some("B")
    );
}

#[test]
fn parses_void_parameter_list() {
    let ast = parse_bytes_with_budget(b"_Z1Av", DEFAULT_BUDGET).unwrap();
    let function = ast.root().child(0).unwrap();
    let parameters = function.child(1).unwrap();
    assert_eq!(parameters.kind(), Kind::ParameterList);
    assert!(parameters.is_leaf());
}

#[test]
fn parses_nested_names() {
    let ast = parse_bytes_with_budget(b"_ZN3Foo3barEv", DEFAULT_BUDGET).unwrap();
    let function = ast.root().child(0).unwrap();
    let name = function.child(0).unwrap();
    assert_eq!(name.kind(), Kind::NestedName);
    assert_eq!(
        child_kinds(&name),
        alloc::vec![Kind::UnqualifiedName, Kind::UnqualifiedName]
    );
    assert_eq!(name.child(0).unwrap().child(0).unwrap().text(), Some("Foo"));
    assert_eq!(name.child(1).unwrap().child(0).unwrap().text(), Some("bar"));
}

#[test]
fn parses_cv_and_ref_qualified_names() {
    let ast = parse_bytes_with_budget(b"_ZNK1A1fEv", DEFAULT_BUDGET).unwrap();
    let name = ast.root().child(0).unwrap().child(0).unwrap();
    assert_eq!(name.kind(), Kind::NestedName);
    assert_eq!(name.text(), Some("const"));

    let ast = parse_bytes_with_budget(b"_ZNKO1A1hEv", DEFAULT_BUDGET).unwrap();
    let name = ast.root().child(0).unwrap().child(0).unwrap();
    assert_eq!(name.text(), Some("const &&"));
}

#[test]
fn parses_builtins_pointers_and_qualifiers() {
    let ast = parse_bytes_with_budget(b"_Z1fPKc", DEFAULT_BUDGET).unwrap();
    let parameters = ast.root().child(0).unwrap().child(1).unwrap();
    let parameter = parameters.child(0).unwrap();
    let pointer = parameter.child(0).unwrap();
    assert_eq!(pointer.kind(), Kind::PointerType);
    let qualified = pointer.child(0).unwrap();
    assert_eq!(qualified.kind(), Kind::QualifiedType);
    assert_eq!(qualified.text(), Some("const"));
    assert_eq!(qualified.child(0).unwrap().text(), Some("char"));
}

#[test]
fn parses_arrays_and_function_types() {
    let ast = parse_bytes_with_budget(b"_Z1fA5_i", DEFAULT_BUDGET).unwrap();
    let array = ast
        .root()
        .child(0)
        .unwrap()
        .child(1)
        .unwrap()
        .child(0)
        .unwrap()
        .child(0)
        .unwrap();
    assert_eq!(array.kind(), Kind::ArrayType);
    assert_eq!(array.text(), Some("5"));
    assert_eq!(array.child(0).unwrap().text(), Some("int"));

    let ast = parse_bytes_with_budget(b"_Z1fPFivE", DEFAULT_BUDGET).unwrap();
    let pointer = ast
        .root()
        .child(0)
        .unwrap()
        .child(1)
        .unwrap()
        .child(0)
        .unwrap()
        .child(0)
        .unwrap();
    assert_eq!(pointer.kind(), Kind::PointerType);
    let function = pointer.child(0).unwrap();
    assert_eq!(function.kind(), Kind::FunctionType);
    assert_eq!(function.child(0).unwrap().text(), Some("int"));
    assert!(function.child(1).unwrap().is_leaf());
}

#[test]
fn parses_member_pointer() {
    let ast = parse_bytes_with_budget(b"_Z1fM1AFivE", DEFAULT_BUDGET).unwrap();
    let member = ast
        .root()
        .child(0)
        .unwrap()
        .child(1)
        .unwrap()
        .child(0)
        .unwrap()
        .child(0)
        .unwrap();
    assert_eq!(member.kind(), Kind::MemberPointerType);
    assert_eq!(member.child(0).unwrap().child(0).unwrap().text(), Some("A"));
    assert_eq!(member.child(1).unwrap().kind(), Kind::FunctionType);
}

/// Every `<operator-name>` code and the text it must print.
///
/// This table is the oracle for the parser's direct two-byte match: it
/// pins both the accepted codes and their spelling, so a code cannot be
/// dropped, invented or re-spelled unnoticed.
const OPERATOR_SPELLINGS: &[(&[u8; 2], &str)] = &[
    (b"nw", "operator new"),
    (b"na", "operator new[]"),
    (b"dl", "operator delete"),
    (b"da", "operator delete[]"),
    (b"ps", "operator+"),
    (b"ng", "operator-"),
    (b"ad", "operator&"),
    (b"de", "operator*"),
    (b"ds", "operator.*"),
    (b"co", "operator~"),
    (b"pl", "operator+"),
    (b"mi", "operator-"),
    (b"ml", "operator*"),
    (b"dv", "operator/"),
    (b"rm", "operator%"),
    (b"an", "operator&"),
    (b"or", "operator|"),
    (b"eo", "operator^"),
    (b"aS", "operator="),
    (b"pL", "operator+="),
    (b"mI", "operator-="),
    (b"mL", "operator*="),
    (b"dV", "operator/="),
    (b"rM", "operator%="),
    (b"aN", "operator&="),
    (b"oR", "operator|="),
    (b"eO", "operator^="),
    (b"ls", "operator<<"),
    (b"rs", "operator>>"),
    (b"lS", "operator<<="),
    (b"rS", "operator>>="),
    (b"eq", "operator=="),
    (b"ne", "operator!="),
    (b"lt", "operator<"),
    (b"gt", "operator>"),
    (b"le", "operator<="),
    (b"ge", "operator>="),
    (b"ss", "operator<=>"),
    (b"nt", "operator!"),
    (b"aa", "operator&&"),
    (b"oo", "operator||"),
    (b"pp", "operator++"),
    (b"mm", "operator--"),
    (b"cm", "operator,"),
    (b"pm", "operator->*"),
    (b"pt", "operator->"),
    (b"cl", "operator()"),
    (b"ix", "operator[]"),
    (b"qu", "operator?"),
    (b"st", "operator sizeof"),
    (b"sz", "operator sizeof..."),
    (b"at", "operator alignof"),
    (b"az", "operator alignof..."),
    (b"nx", "operator noexcept"),
    (b"aw", "operator co_await"),
];

/// The spelling `OPERATOR_SPELLINGS` expects for one byte pair.
fn expected_operator_spelling(first: u8, second: u8) -> Option<&'static str> {
    OPERATOR_SPELLINGS
        .iter()
        .find(|(code, _)| code[0] == first && code[1] == second)
        .map(|&(_, spelling)| spelling)
}

#[test]
fn operator_code_table_has_no_duplicate_codes() {
    for (index, (code, _)) in OPERATOR_SPELLINGS.iter().enumerate() {
        let earlier = OPERATOR_SPELLINGS.get(..index).unwrap_or(&[]);
        assert!(
            !earlier.iter().any(|(other, _)| other == code),
            "duplicate operator code {code:?}"
        );
    }
}

#[test]
fn operator_code_table_is_exhaustive() {
    // Both bytes together enumerate every code the match may accept, so a
    // missing, invented or re-spelled code fails here.
    for first in 0..=u8::MAX {
        for second in 0..=u8::MAX {
            assert_eq!(
                operator_code_spelling(first, second),
                expected_operator_spelling(first, second),
                "operator code {first:#04x} {second:#04x}"
            );
        }
    }
}

/// A parser over `bytes` positioned at `prefix`, with the compiled-in
/// Itanium limits.
fn itanium_parser<'a, 's>(bytes: &'a [u8], prefix: usize, state: &'s mut State) -> Parser<'a, 's> {
    Parser::new(bytes, prefix, state, ParseLimits::itanium(DEFAULT_BUDGET)).unwrap()
}

#[test]
fn operator_lookup_reads_two_bytes_at_the_cursor() {
    for (code, spelling) in OPERATOR_SPELLINGS {
        // A code at the very end of the input is still a complete code.
        let mut state = itanium_state(code.len());
        let parser = itanium_parser(*code, 0, &mut state);
        assert_eq!(parser.operator_spelling(), Some(*spelling));

        // The lookup itself does not move the cursor; the accepting caller
        // advances exactly two bytes, leaving a following name in place.
        let mut bytes = code.to_vec();
        bytes.extend_from_slice(b"3foo");
        let mut state = itanium_state(bytes.len());
        let mut parser = itanium_parser(&bytes, 0, &mut state);
        assert_eq!(parser.operator_spelling(), Some(*spelling));
        assert_eq!(parser.input.pos(), 0, "lookup consumed input");
        let name = parser.parse_unqualified_name().unwrap();
        assert_eq!(name.kind(), Kind::OperatorName);
        assert_eq!(name.text(), Some(*spelling));
        assert_eq!(parser.input.pos(), 2, "{code:?} must consume two bytes");
    }
}

#[test]
fn operator_lookup_rejects_truncated_and_unknown_codes() {
    // Empty input has no code at all.
    let mut state = itanium_state(0);
    let parser = itanium_parser(b"", 0, &mut state);
    assert_eq!(parser.operator_spelling(), None);

    // One byte of a real code is a truncated code, not a match.
    for (code, _) in OPERATOR_SPELLINGS {
        let mut state = itanium_state(1);
        let parser = itanium_parser(&code[..1], 0, &mut state);
        assert_eq!(parser.operator_spelling(), None, "truncated {code:?}");
    }

    // Two bytes that are not a code are rejected, including codes spelled
    // with the wrong case and bytes outside ASCII.
    for code in [b"zz", b"nW", b"NN", b"a ", b" w", b"\xff\xff"] {
        let mut state = itanium_state(code.len());
        let parser = itanium_parser(code, 0, &mut state);
        assert_eq!(parser.operator_spelling(), None, "unknown {code:?}");
    }

    // A code is only recognised at the cursor: `nw` after the first byte is
    // not at the current position.
    let mut state = itanium_state(3);
    let parser = itanium_parser(b"xnw", 0, &mut state);
    assert_eq!(parser.operator_spelling(), None);
    let mut state = itanium_state(3);
    let parser = itanium_parser(b"xnw", 1, &mut state);
    assert_eq!(parser.operator_spelling(), Some("operator new"));
    // A truncated code at the cursor stays rejected even when an earlier
    // byte would have completed it.
    let mut state = itanium_state(2);
    let parser = itanium_parser(b"xn", 1, &mut state);
    assert_eq!(parser.operator_spelling(), None);
}

#[test]
fn templated_conversion_operator_has_no_return_type() {
    let ast = parse_bytes_with_budget(b"_ZN1AcviIiEEv", DEFAULT_BUDGET).unwrap();
    let function = ast.root().child(0).unwrap();
    assert_eq!(function.kind(), Kind::Function);
    assert_eq!(
        child_kinds(&function),
        alloc::vec![Kind::Name, Kind::ParameterList]
    );
}

#[test]
fn parses_template_arguments_and_parameters() {
    let ast = parse_bytes_with_budget(b"_Z3fooIiEvT_", DEFAULT_BUDGET).unwrap();
    let function = ast.root().child(0).unwrap();
    let name = function.child(0).unwrap();
    assert_eq!(name.kind(), Kind::Name);
    assert_eq!(name.child(1).unwrap().kind(), Kind::TemplateArgs);
    assert_eq!(function.child(1).unwrap().text(), Some("void"));
    let parameter = function.child(2).unwrap().child(0).unwrap();
    assert_eq!(parameter.child(0).unwrap().kind(), Kind::TemplateParam);
    assert_eq!(parameter.child(0).unwrap().text(), Some("T_"));
}

#[test]
fn parses_std_names() {
    let ast = parse_bytes_with_budget(b"_ZSt4cout", DEFAULT_BUDGET).unwrap();
    let name = ast.root().child(0).unwrap().child(0).unwrap();
    assert_eq!(name.kind(), Kind::NestedName);
    assert_eq!(name.child(0).unwrap().text(), Some("std"));
    assert_eq!(
        name.child(1).unwrap().child(0).unwrap().text(),
        Some("cout")
    );
}

#[test]
fn substitutions_share_nodes() {
    let ast = parse_bytes_with_budget(b"_Z1f1AS_S_", DEFAULT_BUDGET).unwrap();
    let parameters = ast.root().child(0).unwrap().child(1).unwrap();
    let first = parameters.child(0).unwrap().child(0).unwrap();
    let second = parameters.child(1).unwrap().child(0).unwrap();
    assert!(Rc::ptr_eq(&first, &second));
}

#[test]
fn nested_prefix_substitution_shares_nodes() {
    let ast = parse_bytes_with_budget(b"_ZN1A1B1CES0_1D", DEFAULT_BUDGET).unwrap();
    // The first parameter is the substituted `A::B` prefix.
    let parameters = ast.root().child(0).unwrap().child(1).unwrap();
    let first = parameters.child(0).unwrap().child(0).unwrap();
    assert_eq!(first.kind(), Kind::NestedName);
    assert_eq!(first.child(0).unwrap().child(0).unwrap().text(), Some("A"));
    assert_eq!(first.child(1).unwrap().child(0).unwrap().text(), Some("B"));
}

#[test]
fn identifier_text_borrows_the_symbol() {
    let symbol = String::from("_ZN3Foo3barEv");
    let ast = parse_bytes_with_budget(symbol.as_bytes(), DEFAULT_BUDGET).unwrap();
    let foo = find_text(&ast.root(), "Foo").unwrap();
    let text = foo.text().unwrap();
    let symbol_start = symbol.as_ptr() as usize;
    let text_start = text.as_ptr() as usize;
    assert!(text_start >= symbol_start);
    assert!(text_start + text.len() <= symbol_start + symbol.len());
}

#[test]
fn rejects_malformed_input() {
    for symbol in ["_Z", "_Z1", "_Z1fP", "_Z1fS_", "_Zabc", "_Z0", "_Z1fZ"] {
        assert!(
            parse_bytes_with_budget(symbol.as_bytes(), DEFAULT_BUDGET).is_err(),
            "{symbol} should fail"
        );
    }
    assert!(parse_bytes_with_budget(b"_Z1f1A", DEFAULT_BUDGET).is_ok());
}

#[test]
fn parses_vector_types() {
    let ast = parse_bytes_with_budget(b"_Z1fDv2_i", DEFAULT_BUDGET).unwrap();
    let vector = ast
        .root()
        .child(0)
        .unwrap()
        .child(1)
        .unwrap()
        .child(0)
        .unwrap()
        .child(0)
        .unwrap();
    assert_eq!(vector.kind(), Kind::VectorType);
    assert_eq!(vector.text(), Some("2"));
    assert_eq!(vector.child(0).unwrap().text(), Some("int"));
}

#[test]
fn parses_structured_binding_names() {
    let ast = parse_bytes_with_budget(b"_ZDC1a1bE", DEFAULT_BUDGET).unwrap();
    let name = ast.root().child(0).unwrap().child(0).unwrap();
    assert_eq!(name.kind(), Kind::StructuredBinding);
    assert_eq!(name.child_count(), 2);
    assert_eq!(name.child(0).unwrap().text(), Some("a"));
    assert_eq!(name.child(1).unwrap().text(), Some("b"));
}

#[test]
fn parses_explicit_object_parameters() {
    let ast = parse_bytes_with_budget(b"_ZNH3Foo3fooES_i", DEFAULT_BUDGET).unwrap();
    let parameters = ast.root().child(0).unwrap().child(1).unwrap();
    let first = parameters.child(0).unwrap().child(0).unwrap();
    assert_eq!(first.kind(), Kind::ExplicitObjectParameter);
    assert_eq!(
        first.child(0).unwrap().child(0).unwrap().text(),
        Some("Foo")
    );
    let second = parameters.child(1).unwrap().child(0).unwrap();
    assert_eq!(second.kind(), Kind::BuiltinType);
}

#[test]
fn parses_function_type_exception_specifications() {
    let ast = parse_bytes_with_budget(b"_Z1bPDoFivE", DEFAULT_BUDGET).unwrap();
    let function = ast
        .root()
        .child(0)
        .unwrap()
        .child(1)
        .unwrap()
        .child(0)
        .unwrap()
        .child(0)
        .unwrap()
        .child(0)
        .unwrap();
    assert_eq!(function.kind(), Kind::FunctionType);
    let spec = function.child(2).unwrap();
    assert_eq!(spec.kind(), Kind::ExceptionSpec);
    assert_eq!(spec.text(), Some("noexcept"));
    assert!(spec.is_leaf());
}

#[test]
fn parses_unresolved_names_in_template_arguments() {
    // `srN <substitution> <template-id> E <base>`: the qualifier levels
    // build a nested name whose prefix is the `test8::meta<int>`
    // template-id.
    let ast = parse_bytes_with_budget(
        b"_ZN5test81fIiEEvNS_5int_cIXsrNS_4metaIT_E4typeE5valueEEE",
        DEFAULT_BUDGET,
    )
    .unwrap();
    assert!(has_nested_template_id(&ast.root()), "{:?}", ast.root());
    // The base of the unresolved name is a bare identifier.
    assert_eq!(
        find_text(&ast.root(), "value").unwrap().kind(),
        Kind::Identifier
    );
}

#[test]
fn parses_block_invocation_functions() {
    let ast = parse_bytes_with_budget(b"___Z10blocksNRVOv_block_invoke", DEFAULT_BUDGET).unwrap();
    let invoke = ast.root().child(0).unwrap();
    assert_eq!(invoke.kind(), Kind::BlockInvoke);
    assert_eq!(invoke.child(0).unwrap().kind(), Kind::Function);
}

#[test]
fn parses_module_initializers() {
    let ast = parse_bytes_with_budget(b"_ZGIW3FooWP3BarW3Baz", DEFAULT_BUDGET).unwrap();
    let initializer = ast.root().child(0).unwrap();
    assert_eq!(initializer.kind(), Kind::ModuleInitializer);
    let module = initializer.child(0).unwrap();
    assert_eq!(module.kind(), Kind::ModuleName);
    assert_eq!(module.text(), Some("010"));
    assert_eq!(module.child_count(), 3);
}

#[test]
fn parses_friend_function_names() {
    let ast = parse_bytes_with_budget(b"_ZN5test21AIiEF1fEzQ4TrueIT_E", DEFAULT_BUDGET).unwrap();
    let friend = find_kind(&ast.root(), Kind::FriendName).expect("friend name");
    assert_eq!(friend.child_count(), 1);
    let identifier = find_kind(&friend, Kind::Identifier).expect("identifier");
    assert_eq!(identifier.text(), Some("f"));
}

#[test]
fn parses_encoding_requires_clauses() {
    let ast = parse_bytes_with_budget(b"_ZN5test21hIvEEvzQ4TrueITL0__E", DEFAULT_BUDGET).unwrap();
    let function = ast.root().child(0).unwrap();
    let clause = function.child(function.child_count() - 1).unwrap();
    assert_eq!(clause.kind(), Kind::RequiresClause);
    // The parameter list is the last child before the clause.
    assert_eq!(
        function.child(function.child_count() - 2).unwrap().kind(),
        Kind::ParameterList
    );
    // Constraint parameters print their encoded numbering.
    assert_eq!(
        find_text(&clause, "TL0_").map(|node| node.kind()),
        Some(Kind::Identifier)
    );
}

#[test]
fn parses_template_head_requires_clauses() {
    // `Tt Ty Tn TL0__ Q1CIS1_E E`: the clause sits inside the template
    // template parameter's own declaration list.
    let ast = parse_bytes_with_budget(
        b"_ZN5test51gITtTyTnTL0__Q1CIS1_EENS_1XEEEvv",
        DEFAULT_BUDGET,
    )
    .unwrap();
    let declaration = find_text(&ast.root(), "Tt").expect("template template parameter");
    assert_eq!(declaration.kind(), Kind::TemplateParamDecl);
    assert!(
        declaration
            .children()
            .iter()
            .any(|child| child.kind() == Kind::RequiresClause),
        "the clause belongs to the parameter list"
    );
}

#[test]
fn parses_requires_expressions() {
    let ast = parse_bytes_with_budget(b"_Z1gIiEviQrQT__XplfL0p_fp_E", DEFAULT_BUDGET).unwrap();
    let expression = find_kind(&ast.root(), Kind::RequiresExpression).expect("requires");
    assert_eq!(expression.child(0).unwrap().kind(), Kind::ParameterList);
    let requirements: Vec<&NodeRef<'_>> = expression
        .children()
        .iter()
        .filter(|child| child.kind() == Kind::Requirement)
        .collect();
    assert_eq!(requirements.len(), 1);
    assert_eq!(requirements[0].text(), Some("simple"));
}

#[test]
fn parses_pack_subscripts() {
    let ast = parse_bytes_with_budget(b"_Z1gILi0EJciEEDyT0_T_v", DEFAULT_BUDGET).unwrap();
    let index = find_kind(&ast.root(), Kind::PackIndexType).expect("pack index type");
    assert_eq!(index.child(0).unwrap().kind(), Kind::TemplateParam);
    assert_eq!(index.child(1).unwrap().kind(), Kind::Expression);

    let ast = parse_bytes_with_budget(b"_Z3fooILi0ETpTnDaJLi1ELi2EEEDTsyT0_T_Ev", DEFAULT_BUDGET)
        .unwrap();
    let expression = find_kind(&ast.root(), Kind::Expression).expect("pack index");
    assert_eq!(expression.text(), Some("pack-index"));
    assert_eq!(expression.child(0).unwrap().kind(), Kind::TemplateParam);
}

#[test]
fn parses_member_pointer_conversions() {
    let ast = parse_bytes_with_budget(
        b"_Z1fIXtl1DmcM7DerivedKiadL_ZN11MoreDerived1zEEn8EEEEvv",
        DEFAULT_BUDGET,
    )
    .unwrap();
    let cast = find_text(&ast.root(), "member-cast").expect("member cast");
    assert_eq!(cast.kind(), Kind::Expression);
    assert_eq!(cast.child(0).unwrap().kind(), Kind::MemberPointerType);
    // The base-class adjustment is encoded but not part of the AST.
    assert_eq!(cast.child_count(), 2);
}

#[test]
fn parses_module_qualifier_chains() {
    // `W3FOO` and `W3FOO W3BAR` are the first two substitution entries; the
    // qualifier chain is reused through `S0_` and extended by `W3BAR`.
    let ast =
        parse_bytes_with_budget(b"_ZW3FOOW3BAR3FooPS0_1APN3BobS0_1BE", DEFAULT_BUDGET).unwrap();
    let parameters = ast.root().child(0).unwrap().child(1).unwrap();
    assert_eq!(parameters.child_count(), 2);
    let pointer = parameters.child(0).unwrap().child(0).unwrap();
    assert_eq!(pointer.kind(), Kind::PointerType);
    let target = pointer.child(0).unwrap();
    assert_eq!(target.kind(), Kind::NestedName);
    assert_eq!(
        find_kind(&target, Kind::Identifier).unwrap().text(),
        Some("A")
    );
}

#[test]
fn parses_allocation_token_decorations() {
    let ast = parse_bytes_with_budget(b"__alloc_token_123__Z3foov", DEFAULT_BUDGET).unwrap();
    let decoration = ast.root().child(0).unwrap();
    assert_eq!(decoration.kind(), Kind::AllocToken);
    assert_eq!(decoration.child(0).unwrap().kind(), Kind::Function);

    let ast = parse_bytes_with_budget(b"__alloc_token__Z3foov.llvm.1234", DEFAULT_BUDGET).unwrap();
    let decoration = ast.root().child(0).unwrap();
    assert_eq!(decoration.kind(), Kind::AllocToken);
    assert_eq!(decoration.child(0).unwrap().kind(), Kind::DotSuffix);
}

#[test]
fn rejects_symbols_that_are_not_encodings() {
    // A bare type mangling carries no encoding prefix.
    for symbol in ["i", "PKFvRiE"] {
        let error = parse_bytes_with_budget(symbol.as_bytes(), DEFAULT_BUDGET).unwrap_err();
        assert_eq!(error.kind(), FailureKind::NotItanium, "{symbol}");
    }
    // A decoration still has to introduce a mangled name.
    assert_eq!(
        parse_bytes_with_budget(b"__alloc_token_0_Z3foov", DEFAULT_BUDGET)
            .unwrap_err()
            .kind(),
        FailureKind::NotItanium
    );
    assert_eq!(
        parse_bytes_with_budget(b"__alloc_token__Z", DEFAULT_BUDGET)
            .unwrap_err()
            .kind(),
        FailureKind::UnexpectedEnd
    );
    // A requirement has to start with `X`, `T` or `Q`.
    assert_eq!(
        parse_bytes_with_budget(b"_Z1fIiEviQrqZE", DEFAULT_BUDGET)
            .unwrap_err()
            .kind(),
        FailureKind::UnexpectedByte
    );
}

#[test]
fn input_budget_is_enforced() {
    let mut symbol = String::from("_Z");
    symbol.push_str(&"1".repeat(MAX_INPUT_LEN));
    let error = parse_bytes_with_budget(symbol.as_bytes(), DEFAULT_BUDGET).unwrap_err();
    assert_eq!(error.kind(), FailureKind::TooLong);
}

#[test]
fn depth_budget_is_enforced() {
    let mut symbol = String::from("_Z1f");
    symbol.push_str(&"P".repeat(300));
    symbol.push('i');
    let error = parse_bytes_with_budget(symbol.as_bytes(), DEFAULT_BUDGET).unwrap_err();
    assert_eq!(error.kind(), FailureKind::TooDeep);
}

#[test]
fn expression_and_type_depth_budgets_are_enforced() {
    let mut braced = String::from("_Z1fIXtl1B");
    for _ in 0..400 {
        braced.push_str("di1a");
    }
    braced.push_str("Li1EEEEvv");
    assert_eq!(
        parse_bytes_with_budget(braced.as_bytes(), DEFAULT_BUDGET)
            .unwrap_err()
            .kind(),
        FailureKind::TooDeep,
        "braced expressions"
    );

    let mut vendor = String::from("_Z1f");
    for _ in 0..400 {
        vendor.push_str("U1a");
    }
    vendor.push('i');
    assert_eq!(
        parse_bytes_with_budget(vendor.as_bytes(), DEFAULT_BUDGET)
            .unwrap_err()
            .kind(),
        FailureKind::TooDeep,
        "vendor extended qualifiers"
    );

    // A module name is a left-nested chain, so its length is a depth.
    let mut chain = String::from("_Z1fW1a");
    for _ in 0..400 {
        chain.push_str("W1a");
    }
    chain.push_str("1b");
    assert_eq!(
        parse_bytes_with_budget(chain.as_bytes(), DEFAULT_BUDGET)
            .unwrap_err()
            .kind(),
        FailureKind::TooDeep,
        "module name chains"
    );
}

#[test]
fn node_budget_is_enforced() {
    let mut symbol = String::from("_Z1f");
    symbol.push_str(&"i".repeat(20_000));
    let error = parse_bytes_with_budget(symbol.as_bytes(), DEFAULT_BUDGET).unwrap_err();
    assert_eq!(error.kind(), FailureKind::TooManyNodes);
}

#[test]
fn root_kind_is_encoding() {
    assert_eq!(root_kind("_Z1A"), Kind::Encoding);
    assert_eq!(root_kind("_Z1fv"), Kind::Encoding);
}

#[test]
fn node_count_is_bounded_by_input() {
    let ast = parse_bytes_with_budget(b"_Z1fPKc", DEFAULT_BUDGET).unwrap();
    assert!(count_nodes(&ast.root()) < 32);
}
