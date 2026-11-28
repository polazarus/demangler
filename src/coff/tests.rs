use super::*;

#[test]
fn decodes_stdcall_and_fastcall() {
    let stdcall = parse(b"_foo@8").unwrap();
    assert_eq!(stdcall.symbol(), b"foo");
    assert_eq!(stdcall.kind(), DecorationKind::ConventionSuffix);
    assert_eq!(stdcall.convention(), CallingConvention::Stdcall);
    assert_eq!(stdcall.stack_bytes(), Some(8));
    assert!(!stdcall.import());
    assert!(!stdcall.weak_alias());

    let fastcall = parse(b"@foo@8").unwrap();
    assert_eq!(fastcall.symbol(), b"foo");
    assert_eq!(fastcall.convention(), CallingConvention::Fastcall);
}

#[test]
fn decodes_imports() {
    let import = parse(b"__imp__foo").unwrap();
    assert_eq!(import.symbol(), b"foo");
    assert_eq!(import.kind(), DecorationKind::Import);
    assert!(import.import());

    let cpp = parse(b"__imp_?foo@@YAXXZ").unwrap();
    assert_eq!(cpp.symbol(), b"?foo@@YAXXZ");
    assert!(cpp.import());
}

#[test]
fn decodes_imported_calling_conventions() {
    let stdcall = parse(b"__imp__foo@8").unwrap();
    assert_eq!(stdcall.symbol(), b"foo");
    assert_eq!(stdcall.kind(), DecorationKind::Import);
    assert_eq!(stdcall.convention(), CallingConvention::Stdcall);
    assert_eq!(stdcall.stack_bytes(), Some(8));
    assert!(stdcall.import());

    let fastcall = parse(b"__imp_@foo@8").unwrap();
    assert_eq!(fastcall.symbol(), b"foo");
    assert_eq!(fastcall.convention(), CallingConvention::Fastcall);
    assert_eq!(fastcall.stack_bytes(), Some(8));
    assert!(fastcall.import());
}

#[test]
fn preserves_mangled_import_targets() {
    for (symbol, inner) in [
        ("__imp__Z3foov", "_Z3foov"),
        ("__imp__ZN3Foo3barEv", "_ZN3Foo3barEv"),
        ("__imp__RNvCs1234_4test4main", "_RNvCs1234_4test4main"),
        ("__imp___Z12imported_cppi", "_Z12imported_cppi"),
        ("__imp___RNvCs1234_4test4main", "_RNvCs1234_4test4main"),
        ("__imp_?foo@@YAXXZ", "?foo@@YAXXZ"),
        ("__imp_.?AVType@@", ".?AVType@@"),
    ] {
        let import = parse(symbol.as_bytes()).unwrap_or_else(|| panic!("{symbol}"));
        assert_eq!(import.symbol(), inner.as_bytes(), "{symbol}");
        assert!(import.import(), "{symbol}");
    }
    // The underscore is a C decoration when it does not introduce a
    // mangled name.
    let plain = parse(b"__imp__Zone").unwrap();
    assert_eq!(plain.symbol(), b"Zone");
}

#[test]
fn peels_every_nested_import() {
    // Each prefix consumes input, so the walk is linear and needs no cap.
    let nested = std::format!("{}foo", "__imp_".repeat(64));
    assert_eq!(parse(nested.as_bytes()).unwrap().symbol(), b"foo");
    assert_eq!(
        parse_for(nested.as_bytes(), Target::X86).unwrap().symbol(),
        b"foo"
    );
    assert!(parse("__imp_".repeat(64).as_bytes()).is_none());
}

#[test]
fn rejects_malformed_stack_suffixes() {
    for symbol in [
        "_foo@",
        "_foo@x",
        "_foo@8x",
        "_foo@@8",
        "_foo@8@",
        "_foo@8@9",
        "_foo@-1",
        "@foo@",
        "@@foo@8",
        "@foo@8@9",
        "_foo@65536",
        "_foo@18446744073709551616",
    ] {
        assert!(parse(symbol.as_bytes()).is_none(), "{symbol}");
    }
    // The largest representable stack size is accepted.
    assert_eq!(parse(b"_foo@65535").unwrap().stack_bytes(), Some(u16::MAX));
}

#[test]
fn rejects_versions_and_plain_underscores() {
    for symbol in [
        "",
        "_foo",
        "foo@8",
        "@foo",
        "main@@GLIBC_2.2.5",
        "main@VERS_1.0",
        "_ZN4test4main17h1234567890abcdefE@@16",
        "__Z3foov.llvm.9D1C9369@@16",
        "_Z3foov.llvm.9D1C9369",
        "_foo@",
        "__imp_",
        "__imp__",
        "__imp__foo@",
        "__imp__foo@8x",
        "__imp__ZN4test4main17h1234567890abcdefE@@16",
        "__imp__Z3foov@@16",
        "__imp__foo@@GLIBC_2.2.5",
        // Vectorcall and COMDAT weak aliases need a declared target.
        "coff_vectorcall@@8",
        ".weak._weak_cdecl.default",
    ] {
        assert!(parse(symbol.as_bytes()).is_none(), "{symbol}");
    }
}

#[test]
fn decodes_vectorcall_for_declared_targets() {
    for (symbol, target, inner, stack) in [
        ("coff_vectorcall@@8", Target::X86, "coff_vectorcall", 8_u16),
        (
            "coff_vectorcall_none@@0",
            Target::X86,
            "coff_vectorcall_none",
            0,
        ),
        (
            "coff_vectorcall_one@@4",
            Target::X86,
            "coff_vectorcall_one",
            4,
        ),
        ("coff_vectorcall@@16", Target::X64, "coff_vectorcall", 16),
        (
            "coff_vectorcall_three@@24",
            Target::X64,
            "coff_vectorcall_three",
            24,
        ),
    ] {
        let parsed = parse_for(symbol.as_bytes(), target).unwrap_or_else(|| panic!("{symbol}"));
        assert_eq!(parsed.symbol(), inner.as_bytes(), "{symbol}");
        assert_eq!(parsed.kind(), DecorationKind::ConventionSuffix, "{symbol}");
        assert_eq!(
            parsed.convention(),
            CallingConvention::Vectorcall,
            "{symbol}"
        );
        assert_eq!(parsed.stack_bytes(), Some(stack), "{symbol}");
        assert!(!parsed.import(), "{symbol}");
        assert!(!parsed.weak_alias(), "{symbol}");
    }

    // Imports compose with the suffix.
    let imported = parse_for(b"__imp_imported_vectorcall@@16", Target::X64).unwrap();
    assert_eq!(imported.symbol(), b"imported_vectorcall");
    assert_eq!(imported.kind(), DecorationKind::Import);
    assert_eq!(imported.convention(), CallingConvention::Vectorcall);
    assert_eq!(imported.stack_bytes(), Some(16));
    assert!(imported.import());
}

#[test]
fn decodes_arm64_and_x64_undecorated_names() {
    for (symbol, target) in [
        ("arm_plain", Target::Arm64),
        ("arm_stdcall", Target::Arm64),
        ("arm_fastcall", Target::Arm64),
        ("_arm_local", Target::Arm64),
        ("?cpp@@YAXXZ", Target::X64),
        ("_Z3foov", Target::X64),
    ] {
        let parsed = parse_for(symbol.as_bytes(), target).unwrap_or_else(|| panic!("{symbol}"));
        assert_eq!(parsed.symbol(), symbol.as_bytes(), "{symbol}");
        assert_eq!(parsed.kind(), DecorationKind::Undecorated, "{symbol}");
        assert_eq!(parsed.convention(), CallingConvention::Unknown, "{symbol}");
        assert_eq!(parsed.stack_bytes(), None, "{symbol}");
        assert!(!parsed.import(), "{symbol}");
    }

    // The import prefix is the only ARM64 decoration.
    let imported = parse_for(b"__imp_imported_arm", Target::Arm64).unwrap();
    assert_eq!(imported.symbol(), b"imported_arm");
    assert_eq!(imported.kind(), DecorationKind::Import);
    assert_eq!(imported.convention(), CallingConvention::Cdecl);
    assert!(imported.import());
}

#[test]
fn decodes_leading_underscore_explicitly() {
    let leading = |symbol: &'static str| {
        parse_for(symbol.as_bytes(), Target::X86)
            .filter(|decoration| decoration.kind() == DecorationKind::LeadingUnderscore)
    };
    let parsed = leading("_foo").unwrap();
    assert_eq!(parsed.symbol(), b"foo");
    assert_eq!(parsed.kind(), DecorationKind::LeadingUnderscore);
    assert_eq!(parsed.convention(), CallingConvention::Cdecl);
    assert_eq!(parsed.stack_bytes(), None);
    assert!(!parsed.import());
    assert!(!parsed.weak_alias());

    // The x86 target rules decode the same form.
    let x86 = parse_for(b"_foo", Target::X86).unwrap();
    assert_eq!(x86.symbol(), b"foo");
    assert_eq!(x86.kind(), DecorationKind::LeadingUnderscore);

    for symbol in ["foo", "", "_", "_foo@8", "_foo@", "_foo@bar", "_Z3foov@8"] {
        assert!(leading(symbol).is_none(), "{symbol}");
    }
}

#[test]
fn decodes_comdat_weak_aliases() {
    for (symbol, target, inner, convention, stack) in [
        (
            ".weak._weak_cdecl.default",
            Target::X86,
            "weak_cdecl",
            CallingConvention::Cdecl,
            None,
        ),
        (
            ".weak._weak_stdcall@8.default",
            Target::X86,
            "weak_stdcall",
            CallingConvention::Stdcall,
            Some(8),
        ),
        (
            ".weak.@weak_fastcall@8.default",
            Target::X86,
            "weak_fastcall",
            CallingConvention::Fastcall,
            Some(8),
        ),
        (
            ".weak.weak_stdcall.default",
            Target::X64,
            "weak_stdcall",
            CallingConvention::Unknown,
            None,
        ),
        (
            ".weak.weak_stdcall.default",
            Target::Arm64,
            "weak_stdcall",
            CallingConvention::Unknown,
            None,
        ),
        (
            ".weak.?weak_cpp@@YAHH@Z.default",
            Target::X64,
            "?weak_cpp@@YAHH@Z",
            CallingConvention::Unknown,
            None,
        ),
    ] {
        let parsed = parse_for(symbol.as_bytes(), target).unwrap_or_else(|| panic!("{symbol}"));
        assert_eq!(parsed.symbol(), inner.as_bytes(), "{symbol}");
        assert_eq!(parsed.kind(), DecorationKind::WeakAlias, "{symbol}");
        assert_eq!(parsed.convention(), convention, "{symbol}");
        assert_eq!(parsed.stack_bytes(), stack, "{symbol}");
        assert!(parsed.weak_alias(), "{symbol}");
        assert!(!parsed.import(), "{symbol}");
    }

    // The x86 MinGW spelling keeps the Itanium mangling underscore.
    let itanium = parse_for(b".weak.__Z8weak_cppi.default", Target::X86).unwrap();
    assert_eq!(itanium.symbol(), b"_Z8weak_cppi");
    assert_eq!(itanium.convention(), CallingConvention::Cdecl);
}

#[test]
fn rejects_malformed_target_decorations() {
    for (symbol, target) in [
        ("", Target::X86),
        ("", Target::X64),
        ("", Target::Arm64),
        (".weak.", Target::X86),
        (".weak.foo", Target::X86),
        (".weak..default", Target::X86),
        // The irregular associative spelling clang emits for some C++
        // weak COMDATs is not decoded.
        (
            ".weak.?weak_cpp@@YAHH@Z.default.??1WeakClass@@UEAA@XZ",
            Target::X64,
        ),
        // Only one weak-alias wrapper is decoded.
        (".weak..weak.foo.default.default", Target::X86),
        ("__imp_", Target::X86),
        ("_foo@", Target::X86),
        ("_foo@x", Target::X86),
        ("_foo@65536", Target::X86),
        ("@foo@8@9", Target::X86),
        ("_foo@@8", Target::X86),
        ("@@foo@8", Target::X86),
        ("foo@@", Target::X86),
        ("foo@@x", Target::X64),
        ("_foo@@8", Target::X64),
    ] {
        assert!(
            parse_for(symbol.as_bytes(), target).is_none(),
            "{symbol} {target:?}"
        );
    }
}

#[test]
fn decodes_cold_and_part_fragments() {
    for (symbol, target, inner, kind, convention, stack, fragment) in [
        (
            "_coff_stdcall@8.cold.1",
            Target::X86,
            "coff_stdcall",
            DecorationKind::Fragment,
            CallingConvention::Stdcall,
            Some(8),
            Fragment::Cold { index: Some(1) },
        ),
        (
            "_coff_cdecl.cold.2",
            Target::X86,
            "coff_cdecl",
            DecorationKind::Fragment,
            CallingConvention::Cdecl,
            None,
            Fragment::Cold { index: Some(2) },
        ),
        (
            "cold_plain.cold.1",
            Target::X64,
            "cold_plain",
            DecorationKind::Fragment,
            CallingConvention::Unknown,
            None,
            Fragment::Cold { index: Some(1) },
        ),
        (
            "cold_plain.cold",
            Target::X64,
            "cold_plain",
            DecorationKind::Fragment,
            CallingConvention::Unknown,
            None,
            Fragment::Cold { index: None },
        ),
        (
            "cold_plain.part.0",
            Target::Arm64,
            "cold_plain",
            DecorationKind::Fragment,
            CallingConvention::Unknown,
            None,
            Fragment::Part { index: 0 },
        ),
        (
            "?cold_cpp@@YAHH@Z.cold.1",
            Target::X64,
            "?cold_cpp@@YAHH@Z",
            DecorationKind::Fragment,
            CallingConvention::Unknown,
            None,
            Fragment::Cold { index: Some(1) },
        ),
        (
            "__Z11cpp_stdcallii.cold.1",
            Target::X86,
            "_Z11cpp_stdcallii",
            DecorationKind::Fragment,
            CallingConvention::Cdecl,
            None,
            Fragment::Cold { index: Some(1) },
        ),
    ] {
        let parsed = parse_for(symbol.as_bytes(), target).unwrap_or_else(|| panic!("{symbol}"));
        assert_eq!(parsed.symbol(), inner.as_bytes(), "{symbol}");
        assert_eq!(parsed.kind(), kind, "{symbol}");
        assert_eq!(parsed.convention(), convention, "{symbol}");
        assert_eq!(parsed.stack_bytes(), stack, "{symbol}");
        assert_eq!(parsed.fragment(), Some(fragment), "{symbol}");
        assert!(!parsed.import(), "{symbol}");
        assert!(!parsed.weak_alias(), "{symbol}");
    }
}

#[test]
fn rejects_malformed_fragments() {
    for (symbol, target) in [
        (".cold", Target::X64),
        (".cold.1", Target::X64),
        (".part.0", Target::X64),
        ("cold_plain.cold.", Target::X64),
        ("cold_plain.cold.x", Target::X64),
        ("cold_plain.cold.65536", Target::X64),
        ("cold_plain.part", Target::X64),
        ("cold_plain.part.", Target::X64),
        ("cold_plain.part.x", Target::X64),
        ("cold_plain.part.65536", Target::X64),
        ("cold_plain..cold", Target::X64),
        ("cold_plain..cold.1", Target::X64),
        // Only one fragment suffix is decoded.
        ("cold_plain.cold.1.cold.2", Target::X64),
        ("cold_plain.part.0.cold.1", Target::X64),
    ] {
        assert!(
            parse_for(symbol.as_bytes(), target).is_none(),
            "{symbol} {target:?}"
        );
    }
}

#[test]
fn auto_rules_do_not_claim_fragments() {
    for symbol in [
        "cold_plain.cold",
        "cold_plain.cold.1",
        "cold_plain.part.0",
        "_coff_stdcall@8.cold.1",
        "?cold_cpp@@YAHH@Z.cold.1",
        "__Z11cpp_stdcallii.cold.1",
    ] {
        assert!(parse(symbol.as_bytes()).is_none(), "{symbol}");
    }
}

#[test]
fn auto_rules_do_not_claim_ambiguous_forms() {
    for symbol in [
        "_foo",
        "coff_vectorcall@@8",
        "__imp_imported_vectorcall@@8",
        ".weak._weak_cdecl.default",
        ".weak._weak_stdcall@8.default",
        "_ZN4test4main17h1234567890abcdefE@@16",
        "__Z3foov.llvm.9D1C9369@@16",
    ] {
        assert!(parse(symbol.as_bytes()).is_none(), "{symbol}");
        assert!(!is_coff(symbol.as_bytes()), "{symbol}");
    }
}
