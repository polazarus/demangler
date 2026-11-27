use alloc::vec::Vec;

use super::*;

#[test]
fn reports_linkage_flags() {
    let function = Function {
        path: b"pkg.Foo",
        name: b"bar",
        receiver: None,
        context_params: Vec::new(),
        params: Vec::new(),
        type_params: Vec::new(),
        ret: None,
        is_static: false,
        is_suspend: false,
        is_internal: true,
        is_impl: true,
        companion: None,
        platform: None,
        signature: false,
    };
    assert!(function.is_internal());
    assert!(function.is_impl());
    assert!(!function.is_static());
    assert!(!function.is_suspend());
}

#[test]
fn variance_keywords_are_complete() {
    assert_eq!(Variance::Invariant.as_str(), "");
    assert_eq!(Variance::Out.as_str(), "out");
    assert_eq!(Variance::In.as_str(), "in");
}
