use super::*;

#[test]
fn print_envelope_counts_shared_references() {
    let mut context = Context::new(MAX_NODES);
    let leaf = context.builder_text(Kind::Identifier, "A").build();
    let mut parent = context.builder(Kind::Tuple);
    parent.push(leaf.clone());
    parent.push(leaf);
    let parent = parent.build();
    assert_eq!(parent.print_visits(), 3);
    assert_eq!(parent.print_depth(), 2);
    assert!(parent.print_bytes() > 2 * PRINT_NODE_OVERHEAD);
}
#[test]
fn nodes_are_immutable_and_shared() {
    let mut context = Context::new(MAX_NODES);
    let child = context.builder_text(Kind::Identifier, "main").build();
    let mut parent = context.builder(Kind::Tuple);
    parent.push(child.clone());
    let parent = parent.build();
    assert_eq!(parent.child_count(), 1);
    assert!(Rc::ptr_eq(&parent.child(0).unwrap(), &child));

    let substitution = parent.clone();
    assert!(Rc::ptr_eq(&substitution, &parent));
    assert_eq!(substitution.kind(), Kind::Tuple);
}

#[test]
fn payload_accessors_read_the_data() {
    let mut context = Context::new(MAX_NODES);

    let number = context.builder_value(Kind::Number, 7).build();
    assert_eq!(number.number_value(), 7);
    assert_eq!(number.number_len(), 0);

    let index = context.builder_index(Kind::ExplicitClosure, 4).build();
    assert_eq!(index.entity_index(), 4);

    let slot = context
        .builder_global(Global::FieldOffset, GlobalOperand::Directness(true))
        .build();
    assert_eq!(slot.global_code(), Some(Global::FieldOffset));
    assert_eq!(slot.global_operand(), GlobalOperand::Directness(true));

    let accessor = context
        .builder_accessor(Kind::Variable, Accessor::Getter)
        .build();
    assert_eq!(accessor.accessor_code(), Some(Accessor::Getter));

    let role = context
        .builder_macro_role(Kind::AttachedMacroExpansion, MacroRole::Preamble)
        .build();
    assert_eq!(role.macro_role(), Some(MacroRole::Preamble));

    let layout = context.builder_layout(b'T').unwrap().build();
    assert_eq!(layout.layout_code(), LayoutCode::Trivial);

    assert!(LayoutCode::from_byte(b'x').is_none());
}

#[test]
fn relabel_and_append_preserve_payload_and_children() {
    let mut context = Context::new(MAX_NODES);
    let child = context.builder_text(Kind::Identifier, "A").build();
    let mut parent = context.builder_accessor(Kind::Subscript, Accessor::Setter);
    parent.push(child.clone());
    let parent = parent.build();

    let rebuilt = context.relabel(&parent, Kind::Variable);
    assert_eq!(rebuilt.kind(), Kind::Variable);
    assert_eq!(rebuilt.accessor_code(), Some(Accessor::Setter));
    assert!(Rc::ptr_eq(&rebuilt.child(0).unwrap(), &child));

    let extra = context.builder_text(Kind::Identifier, "B").build();
    let appended = context.append(&parent, extra.clone());
    assert_eq!(appended.child_count(), 2);
    assert!(Rc::ptr_eq(&appended.child(1).unwrap(), &extra));
    assert_eq!(parent.child_count(), 1, "the source stays immutable");
}

#[test]
fn budget_is_enforced() {
    let mut context = Context::new(MAX_NODES);
    for _ in 0..MAX_NODES {
        let _ = context.builder(Kind::Identifier);
    }
    assert!(!context.failed());
    let _ = context.builder(Kind::Identifier);
    assert!(context.failed());
}

#[test]
fn every_payload_family_relabels() {
    let mut context = Context::new(MAX_NODES);
    let nodes = [
        context.builder_text(Kind::Identifier, "Foo").build(),
        context.builder_value(Kind::Number, 1).build(),
        context.builder_index(Kind::ExplicitClosure, 2).build(),
        context.builder(Kind::Tuple).build(),
        context
            .builder_global(Global::TypeMetadata, GlobalOperand::None)
            .build(),
        context
            .builder_accessor(Kind::Subscript, Accessor::Getter)
            .build(),
        context.builder_impl(Kind::ImplConvention, b'g').build(),
        context
            .builder_macro_role(Kind::AttachedMacroExpansion, MacroRole::Peer)
            .build(),
        context
            .builder_autodiff(Kind::AutoDiffFunctionKind, AutoDiffCode::Reverse)
            .build(),
        context
            .builder_generic_specialization(GenericSpecializationKind::Inlined, false)
            .build(),
        context
            .builder_function_signature_specialization(true)
            .build(),
        context.builder_function_spec(true).build(),
        context.builder_keypath(KeyPathKind::AppliedMethod).build(),
        context.builder_layout(b'e').unwrap().build(),
        context
            .builder_deinit(DeinitializerKind::Deallocating)
            .build(),
        context.builder_attribute(AttributeKind::Override).build(),
    ];
    for node in &nodes {
        let _ = core::format_args!("{node:?}");
        let relabelled = context.relabel(node, Kind::Tuple);
        assert_eq!(relabelled.kind(), Kind::Tuple);
    }
    let _ = core::format_args!("{context:?}");
}
