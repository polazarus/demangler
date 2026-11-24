//! Objective-C detection and feature-subset tests.
//!
//! Objective-C metadata is checked before Go and the legacy C++ dialects, so
//! these tests pin that dotted metadata is not stolen by Go, that Objective-C++
//! `_Z` names stay C++, and that disabling `objc` removes the scheme without
//! breaking the other backends.

use demangler::Scheme;
#[cfg(feature = "objc")]
use demangler::{Demangler, Language};

#[cfg(feature = "objc")]
const OBJC_NAMES: &[&str] = &[
    "_OBJC_CLASS_$_NSString",
    "__OBJC_CLASS_$_NSString",
    "OBJC_CLASS_$_NSString",
    "_OBJC_METACLASS_$_NSString",
    "__OBJC_$_INSTANCE_METHODS_Foo",
    "__OBJC_$_CLASS_METHODS_Foo",
    "__OBJC_$_CATEGORY_Foo_$_Bar",
    "__OBJC_$_PROP_LIST_Foo",
    "__OBJC_$_INSTANCE_VARIABLES_Foo",
    "__OBJC_PROTOCOL_$_NSCopying",
    "__OBJC_PROTOCOL_REFERENCE_$_NSCopying",
    "_OBJC_IVAR_$_Foo._ivar",
    "_OBJC_METH_VAR_NAME_0",
    "__objc_class_name_Foo",
    "__objc_ivar_offset_Foo._ivar",
    "-[Foo bar:baz:]",
    "+[Foo classThing]",
    "-[Foo(Cat) catMethod]",
    "___block_descriptor_40_e8_32r_e5_v8?0l",
];

#[cfg(feature = "objc")]
#[test]
fn detects_objc_names() {
    let demangler = Demangler::new();
    for &name in OBJC_NAMES {
        assert_eq!(Scheme::detect(name), Scheme::ObjC, "{name}");
        assert_eq!(
            Scheme::detect(name).language(),
            Language::ObjectiveC,
            "{name}"
        );
        let demangled = demangler
            .demangle(name)
            .unwrap_or_else(|error| panic!("{name}: {error}"));
        assert_eq!(demangled.scheme(), Scheme::ObjC, "{name}");
        assert_eq!(demangled.language(), Language::ObjectiveC, "{name}");
    }
}

#[cfg(all(feature = "objc", feature = "go"))]
#[test]
fn dotted_metadata_is_not_go() {
    // `is_go_name` accepts these as package-qualified names; Objective-C has
    // to win before Go for the ivar and block forms.
    for name in [
        "_OBJC_IVAR_$_Foo._ivar",
        "__objc_ivar_offset_Foo._ivar",
        "__objc_ivar_offset_value_Foo._ivar",
    ] {
        assert_eq!(Scheme::detect(name), Scheme::ObjC, "{name}");
    }
    // Ordinary Go names still win.
    for name in [
        "fmt.Println",
        "runtime.(*mLockProfile).captureStack",
        "example.com/x/pkg%2ev1.Make",
        "go:itab.*os.File,io.Writer",
    ] {
        assert_eq!(Scheme::detect(name), Scheme::Go, "{name}");
    }
}

#[cfg(all(feature = "objc", feature = "cpp-itanium"))]
#[test]
fn objective_cpp_stays_cpp() {
    for name in [
        "_Z3foov",
        "__Z3foov",
        "_ZN3Foo3barEv",
        "__ZN3Foo3barEv",
        "___Z3foov_block_invoke",
        "____Z3foov_block_invoke",
    ] {
        assert_eq!(Scheme::detect(name), Scheme::CppItanium, "{name}");
    }
}

#[cfg(all(feature = "objc", feature = "rust"))]
#[test]
fn objc_does_not_steal_rust() {
    for name in ["_RNvCs1234_4test4main", "__RNvCs1234_4test4main"] {
        assert_eq!(Scheme::detect(name), Scheme::RustV0, "{name}");
    }
    assert_eq!(
        Scheme::detect("_ZN4test4main17h1234567890abcdefE"),
        Scheme::RustLegacy
    );
}

#[cfg(all(feature = "objc", feature = "swift"))]
#[test]
fn objc_does_not_steal_swift() {
    for name in ["$s4main3fooSi_yF", "_$s4main3fooSi_yF"] {
        assert_eq!(Scheme::detect(name), Scheme::SwiftStable, "{name}");
    }
    assert_eq!(Scheme::detect("_T04main3fooSi_yF"), Scheme::SwiftStable);
}

#[cfg(feature = "objc")]
#[test]
fn unknown_objc_metadata_is_rejected() {
    let demangler = Demangler::new();
    // A known prefix with a malformed body is claimed, but the record is an
    // error.
    for name in [
        "__OBJC_$_NOPE_Foo",
        "_OBJC_CLASS_$_",
        "__objc_class_name_",
    ] {
        assert_eq!(Scheme::detect(name), Scheme::ObjC, "{name}");
        assert!(
            demangler.demangle(name).is_err(),
            "{name} should be rejected"
        );
    }
    // An unknown Apple metadata kind is not claimed as Objective-C.
    for name in ["_OBJC_NOPE_$_Foo", "_OBJC_Foo.Bar"] {
        assert_ne!(Scheme::detect(name), Scheme::ObjC, "{name}");
    }
    // The namespace guard keeps unknown records out of legacy C++ guessing.
    for name in ["_OBJC_NOPE_$_Foo", "__objc_exec_class"] {
        let demangled = demangler
            .demangle(name)
            .unwrap_or_else(|error| panic!("{name}: {error}"));
        assert_eq!(demangled.scheme(), Scheme::Unknown, "{name}");
        assert_eq!(demangled.to_string(), *name, "{name}");
    }
    // Generic GNUstep runtime helpers are not metadata.
    for name in ["__objc_exec_class", "__objc_load", "__objc_category_name_Foo_Bar"] {
        assert_ne!(Scheme::detect(name), Scheme::ObjC, "{name}");
    }
}

#[cfg(feature = "objc")]
#[test]
fn decorations_are_stripped() {
    let demangler = Demangler::new();
    for (name, expected) in [
        ("_OBJC_CLASS_$_Foo.llvm.9D1C9369", "Objective-C class Foo"),
        ("_OBJC_CLASS_$_Foo@@VERS_1.0", "Objective-C class Foo"),
        ("__objc_class_name_Foo.llvm.9D1C9369", "Objective-C class Foo"),
    ] {
        assert_eq!(Scheme::detect(name), Scheme::ObjC, "{name}");
        let demangled = demangler
            .demangle(name)
            .unwrap_or_else(|error| panic!("{name}: {error}"));
        assert_eq!(demangled.to_string(), expected, "{name}");
    }
}

#[cfg(not(feature = "objc"))]
#[test]
fn objc_is_not_detected_without_its_feature() {
    for name in [
        "_OBJC_CLASS_$_NSString",
        "__OBJC_$_INSTANCE_METHODS_Foo",
        "__objc_class_name_Foo",
        "-[Foo bar:]",
        "___block_descriptor_40_e8_32r_e5_v8?0l",
    ] {
        assert_ne!(Scheme::detect(name), Scheme::ObjC, "{name}");
    }
}

#[test]
fn plain_c_is_unchanged() {
    for name in ["main", "foo_bar", "__libc_start_main", "a__b__c"] {
        assert_ne!(Scheme::detect(name), Scheme::ObjC, "{name}");
    }
    assert_eq!(Scheme::detect("main"), Scheme::C);
}
