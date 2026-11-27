use super::*;

fn render(symbol: &str) -> Option<alloc::string::String> {
    let ast = crate::swift::parse(symbol).ok()?;
    Some(alloc::format!("{ast}"))
}

#[test]
fn value_witness_codes_are_known() {
    assert_eq!(
        value_witness_name(Some(b'a'), Some(b'l')),
        Some("allocateBuffer")
    );
    assert_eq!(value_witness_name(Some(b'a'), Some(b'x')), None);
}

#[test]
fn legacy_types_render() {
    assert_eq!(render("_TtSi").as_deref(), Some("Swift.Int"));
    assert_eq!(render("_TtGSaSS_").as_deref(), Some("[Swift.String]"));
    assert_eq!(
        render("_TtGSqGSaC5sugar7MyClass__").as_deref(),
        Some("[sugar.MyClass]?")
    );
}

#[test]
fn legacy_accessors_use_storage_entities() {
    // The old grammar nests the storage under an accessor node; the shared
    // printer carries the accessor code on the storage node instead.
    assert_eq!(
        render("_TF3fooau3barSi").as_deref(),
        Some("foo.bar.unsafeMutableAddressor : Swift.Int")
    );
    assert_eq!(
        render("_TiC4Meow5MyCls9subscriptFT1iSi_Sf").as_deref(),
        Some("Meow.MyCls.subscript(i: Swift.Int) -> Swift.Float")
    );
}

#[test]
fn legacy_globals_map_to_shared_codes() {
    assert_eq!(
        render("_TWvdvC3foo3bar3basSi").as_deref(),
        Some("direct field offset for foo.bar.bas : Swift.Int")
    );
    assert_eq!(
        render("_TwalC3foo3bar").as_deref(),
        Some("allocateBuffer value witness for foo.bar")
    );
    assert_eq!(
        render("_TMnC3foo3bar").as_deref(),
        Some("nominal type descriptor for foo.bar")
    );
}

#[test]
fn unsupported_forms_are_rejected() {
    // Malformed and unimplemented legacy forms must not print.
    for symbol in ["_TtZZ", "_TtB", "_Tw", "_TTSgS"] {
        assert_eq!(render(symbol), None, "{symbol}");
    }
}

#[test]
fn typed_payloads_replace_the_old_codes() {
    use crate::swift::node::LayoutCode;

    fn payload_of<T>(symbol: &str, kind: Kind, read: impl Fn(NodeRef<'_>) -> T) -> Option<T> {
        let ast = crate::swift::parse(symbol).ok()?;
        let mut stack = alloc::vec![ast.root()];
        let mut found = None;
        while let Some(node) = stack.pop() {
            if node.kind() == kind {
                found = Some(read(node.clone()));
            }
            stack.extend(node.children().iter().cloned());
        }
        found
    }

    for (symbol, expected) in [
        ("_TFC3foo3bard", DeinitializerKind::Deinit),
        ("_TFC3foo3barD", DeinitializerKind::Deallocating),
        ("_TFC3foo3barZ", DeinitializerKind::IsolatedDeallocating),
    ] {
        let found = payload_of(symbol, Kind::Deinitializer, |node| {
            node.deinitializer_kind()
        });
        assert_eq!(found, Some(expected), "{symbol}");
    }

    for (symbol, expected) in [
        ("_TtuRxlUrFxwx5Mince", LayoutCode::UnknownLayout),
        ("_TtuRxlRrFxwx5Mince", LayoutCode::RefCountedObject),
        ("_TtuRxlNrFxwx5Mince", LayoutCode::NativeRefCountedObject),
        ("_TtuRxle64rFxwx5Mince", LayoutCode::TrivialExactSize),
        (
            "_TtuRxlE64_16rFxwx5Mince",
            LayoutCode::TrivialExactSizeAndAlignment,
        ),
        ("_TtuRxlm64rFxwx5Mince", LayoutCode::TrivialAtMostSize),
        (
            "_TtuRxlM64_16rFxwx5Mince",
            LayoutCode::TrivialAtMostSizeAndAlignment,
        ),
    ] {
        let found = payload_of(symbol, Kind::DependentGenericLayoutRequirement, |node| {
            node.layout_code()
        });
        assert_eq!(found, Some(expected), "{symbol}");
    }

    // Letters the old grammar never accepted stay rejected, even though
    // the typed enum knows them.
    for symbol in ["_TtuRxlCrFxwx5Mince", "_TtuRxlDrFxwx5Mince"] {
        assert_eq!(render(symbol), None, "{symbol}");
    }
}
