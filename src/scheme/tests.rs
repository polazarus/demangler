use super::*;

#[test]
fn every_language_has_a_stable_name() {
    for (language, name) in [
        (Language::C, "C"),
        (Language::Cpp, "C++"),
        (Language::ObjectiveC, "Objective-C"),
        (Language::Rust, "Rust"),
        (Language::Swift, "Swift"),
        (Language::Go, "Go"),
        (Language::KotlinNative, "Kotlin/Native"),
        (Language::D, "D"),
        (Language::Java, "Java"),
        (Language::Ada, "Ada"),
        (Language::Fortran, "Fortran"),
        (Language::Delphi, "Delphi/Object Pascal"),
        (Language::OCaml, "OCaml"),
        (Language::Unknown, "Unknown"),
    ] {
        assert_eq!(language.as_str(), name);
    }
}

#[test]
fn every_scheme_maps_to_its_language() {
    for (scheme, language) in [
        (Scheme::C, Language::C),
        (Scheme::CppItanium, Language::Cpp),
        (Scheme::CppMsvc, Language::Cpp),
        (Scheme::CppGnuV2, Language::Cpp),
        (Scheme::CppArm, Language::Cpp),
        (Scheme::CppEdg, Language::Cpp),
        (Scheme::CppHp, Language::Cpp),
        (Scheme::RustLegacy, Language::Rust),
        (Scheme::RustV0, Language::Rust),
        (Scheme::SwiftStable, Language::Swift),
        (Scheme::SwiftLegacy, Language::Swift),
        (Scheme::ObjC, Language::ObjectiveC),
        (Scheme::Go, Language::Go),
        (Scheme::KotlinNative, Language::KotlinNative),
        (Scheme::D, Language::D),
        (Scheme::Jni, Language::Java),
        (Scheme::Ada, Language::Ada),
        (Scheme::Fortran, Language::Fortran),
        (Scheme::Delphi, Language::Delphi),
        (Scheme::OCaml, Language::OCaml),
        (Scheme::Unknown, Language::Unknown),
    ] {
        assert_eq!(scheme.language(), language);
    }
}

#[test]
fn every_scheme_has_a_stable_label() {
    for (scheme, label) in [
        (Scheme::C, "C"),
        (Scheme::CppItanium, "C++ (Itanium)"),
        (Scheme::CppMsvc, "C++ (MSVC)"),
        (Scheme::CppGnuV2, "C++ (GNU v2)"),
        (Scheme::CppArm, "C++ (ARM)"),
        (Scheme::CppEdg, "C++ (EDG)"),
        (Scheme::CppHp, "C++ (HP aCC)"),
        (Scheme::RustLegacy, "Rust (legacy)"),
        (Scheme::RustV0, "Rust (v0)"),
        (Scheme::SwiftStable, "Swift"),
        (Scheme::SwiftLegacy, "Swift (legacy)"),
        (Scheme::ObjC, "Objective-C"),
        (Scheme::Go, "Go"),
        (Scheme::KotlinNative, "Kotlin/Native"),
        (Scheme::D, "D"),
        (Scheme::Jni, "JNI"),
        (Scheme::Ada, "Ada"),
        (Scheme::Fortran, "Fortran"),
        (Scheme::Delphi, "Delphi/Object Pascal"),
        (Scheme::OCaml, "OCaml"),
        (Scheme::Unknown, "Unknown"),
    ] {
        assert_eq!(scheme.as_str(), label);
        assert_eq!(std::format!("{scheme}"), label);
    }
}

#[test]
fn the_scheme_taxonomy_is_complete() {
    assert_eq!(Scheme::ALL.len(), Scheme::COUNT);
    for (index, scheme) in Scheme::ALL.iter().enumerate() {
        assert_eq!(
            Scheme::ALL.iter().filter(|other| *other == scheme).count(),
            1,
            "{scheme} is listed once"
        );
        assert_eq!(Scheme::ALL[index], *scheme);
    }
}

#[test]
fn language_groups_cover_every_scheme_exactly_once() {
    let mut grouped = 0_usize;
    for language in [
        Language::C,
        Language::Cpp,
        Language::ObjectiveC,
        Language::Rust,
        Language::Swift,
        Language::Go,
        Language::KotlinNative,
        Language::D,
        Language::Java,
        Language::Ada,
        Language::Fortran,
        Language::Delphi,
        Language::OCaml,
        Language::Unknown,
    ] {
        for scheme in language.schemes() {
            assert_eq!(scheme.language(), language);
            grouped += 1;
        }
    }
    assert_eq!(grouped, Scheme::COUNT);
}

#[test]
fn cpp_group_is_ordered_by_dialect() {
    let expected: &[Scheme] = &[
        Scheme::CppItanium,
        Scheme::CppMsvc,
        Scheme::CppGnuV2,
        Scheme::CppArm,
        Scheme::CppEdg,
        Scheme::CppHp,
    ];
    assert_eq!(Language::Cpp.schemes(), expected);
}

#[test]
fn scheme_set_holds_every_scheme() {
    let mut set = SchemeSet::EMPTY;
    assert!(set.is_empty());
    for scheme in Scheme::ALL {
        assert!(!set.contains(scheme));
        set.insert(scheme);
        assert!(set.contains(scheme));
    }
    assert!(!set.is_empty());
    for scheme in Scheme::ALL {
        assert!(set.contains(scheme));
    }
}

#[cfg(feature = "rust")]
#[test]
fn detects_rust_v0() {
    for symbol in [
        "_RN4main6hello17he3c1b56a8e1b2df5E",
        "_RNvCs1234_4main4main",
    ] {
        assert_eq!(Scheme::detect(symbol), Scheme::RustV0, "{symbol}");
    }
}

#[cfg(feature = "rust")]
#[test]
fn detects_rust_legacy() {
    for symbol in [
        "_ZN4test4main17h1234567890abcdefE",
        "_ZN3foo17h0123456789abcdefE",
    ] {
        assert_eq!(Scheme::detect(symbol), Scheme::RustLegacy, "{symbol}");
    }
}

#[cfg(feature = "cpp-itanium")]
#[test]
fn detects_cpp_itanium() {
    for symbol in ["_ZN3Foo3barEv", "_Z3foov", "__Z3foov"] {
        assert_eq!(Scheme::detect(symbol), Scheme::CppItanium, "{symbol}");
    }
}

#[cfg(feature = "cpp-msvc")]
#[test]
fn detects_cpp_msvc() {
    for symbol in ["?x@@3HA", "?foo@@YAXXZ", ".?AVFoo@@"] {
        assert_eq!(Scheme::detect(symbol), Scheme::CppMsvc, "{symbol}");
    }
}

#[cfg(feature = "swift")]
#[test]
fn detects_swift() {
    for symbol in [
        "$s4main3fooSi_yF",
        "$S3foo3barC3bas3zimyAaEC_tFTo",
        "_$s4main3fooSi_yF",
        "_T0",
    ] {
        assert_eq!(Scheme::detect(symbol), Scheme::SwiftStable, "{symbol}");
    }
}

#[cfg(feature = "swift")]
#[test]
fn detects_swift_legacy() {
    for symbol in ["_TtSi", "_TF3foog3barSi", "_TtBf32_"] {
        assert_eq!(Scheme::detect(symbol), Scheme::SwiftLegacy, "{symbol}");
    }
}

#[cfg(feature = "swift")]
#[test]
fn swift_legacy_prefix_does_not_claim_plain_c_names() {
    assert_ne!(Scheme::detect("_Time"), Scheme::SwiftLegacy);
    assert_eq!(Scheme::detect("_TtSi"), Scheme::SwiftLegacy);
}

#[test]
fn strips_only_complete_linker_suffixes() {
    assert_eq!(strip_thin_lto_bytes(b"_Z3foov.llvm.9D1C9369"), b"_Z3foov");
    assert_eq!(
        strip_thin_lto_bytes(b"_Z3foov.llvm.9D1C9369@@16"),
        b"_Z3foov"
    );
    assert_eq!(
        strip_thin_lto_bytes(b"_Z3foov.llvm.lowercase"),
        b"_Z3foov.llvm.lowercase"
    );
    assert_eq!(strip_thin_lto_bytes(b"_Z3foov.llvm."), b"_Z3foov.llvm.");
    assert_eq!(strip_thin_lto_bytes(b"_Z3foov"), b"_Z3foov");

    assert_eq!(strip_version_suffix_bytes(b"main@@GLIBC_2.2.5"), b"main");
    assert_eq!(strip_version_suffix_bytes(b"foo@1"), b"foo");
    assert_eq!(strip_version_suffix_bytes(b"main.@GLIBC"), b"main.@GLIBC");
}

#[cfg(feature = "cpp-gnu-v2")]
#[test]
fn detects_legacy_cpp_dialects() {
    for (symbol, scheme) in [
        ("foo__Fiv", Scheme::CppGnuV2),
        ("__ct__3fooFv", Scheme::CppArm),
        ("foo__Fi_v", Scheme::CppEdg),
        ("foo__XSP5_Fv", Scheme::CppHp),
    ] {
        assert_eq!(Scheme::detect(symbol), scheme, "{symbol}");
        assert_eq!(Scheme::detect_bytes(symbol.as_bytes()), scheme, "{symbol}");
        assert_eq!(scheme.language(), Language::Cpp, "{symbol}");
    }
}

#[cfg(any(feature = "cpp-itanium", feature = "cpp-msvc", feature = "cpp-gnu-v2"))]
#[test]
fn cpp_dialects_map_to_precise_schemes() {
    for (dialect, scheme) in [
        (crate::cpp::Dialect::Itanium, Scheme::CppItanium),
        (crate::cpp::Dialect::Msvc, Scheme::CppMsvc),
        (crate::cpp::Dialect::GnuV2, Scheme::CppGnuV2),
        (crate::cpp::Dialect::Arm, Scheme::CppArm),
        (crate::cpp::Dialect::Edg, Scheme::CppEdg),
        (crate::cpp::Dialect::Hp, Scheme::CppHp),
    ] {
        assert_eq!(scheme_for_cpp_dialect(dialect), scheme);
    }
}

#[cfg(feature = "rust")]
#[test]
fn rust_encodings_map_to_precise_schemes() {
    for (encoding, scheme) in [
        (crate::rust::Encoding::Legacy, Scheme::RustLegacy),
        (crate::rust::Encoding::V0, Scheme::RustV0),
    ] {
        assert_eq!(scheme_for_rust_encoding(encoding), scheme);
    }
}

#[cfg(feature = "swift")]
#[test]
fn swift_encodings_map_to_precise_schemes() {
    for (encoding, scheme) in [
        (crate::swift::Encoding::Stable, Scheme::SwiftStable),
        (crate::swift::Encoding::Legacy, Scheme::SwiftLegacy),
    ] {
        assert_eq!(scheme_for_swift_encoding(encoding), scheme);
    }
}
