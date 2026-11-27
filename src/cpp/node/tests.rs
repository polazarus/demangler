use super::*;

#[test]
fn print_envelope_counts_shared_references() {
    let leaf = Node::new(Kind::Identifier, None, None, Vec::new());
    let root = Node::new(Kind::Encoding, None, None, alloc::vec![leaf.clone(), leaf]);
    assert_eq!(root.print_visits(), 3);
    assert_eq!(root.print_depth(), 2);
    assert!(root.print_bytes() > 2 * PRINT_NODE_OVERHEAD);
}

#[test]
fn debug_is_bounded() {
    let leaf = Node::new(
        Kind::BuiltinType,
        Some(Cow::Borrowed("int")),
        None,
        Vec::new(),
    );
    let parent = Node::new(Kind::PointerType, None, None, alloc::vec![leaf]);
    let rendered = alloc::format!("{parent:?}");
    assert!(rendered.contains("PointerType"));
    assert!(rendered.contains("children: 1"));
    assert!(!rendered.contains("BuiltinType"));
}

#[test]
fn children_are_sealed() {
    let leaf = Node::new(Kind::Identifier, Some(Cow::Borrowed("x")), None, Vec::new());
    let node = Node::new(Kind::UnqualifiedName, None, None, alloc::vec![leaf]);
    assert_eq!(node.child_count(), 1);
    assert_eq!(node.children().len(), 1);
    assert!(!node.is_leaf());
    assert!(node.child(1).is_none());
}
