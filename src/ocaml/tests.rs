use alloc::format;
use alloc::string::{String, ToString};

use super::*;

#[test]
fn parse_errors_describe_themselves() {
    assert_eq!(Failure::Invalid.to_string(), "invalid syntax");
    assert_eq!(Failure::BudgetExceeded.to_string(), "size limit reached");
}

#[test]
fn parses_module_records() {
    for (symbol, expected) in [
        ("camlProbe", "Probe"),
        ("camlStdlib__List", "Stdlib__List"),
        ("camlCamlinternalFormatBasics", "CamlinternalFormatBasics"),
        ("camlFoo$27", "Foo'"),
        (
            "camlOcaml_pack.Ocaml_pack_base",
            "Ocaml_pack.Ocaml_pack_base",
        ),
        ("_camlProbe", "Probe"),
    ] {
        assert_eq!(render(symbol), expected, "{symbol}");
    }
}

#[test]
fn strips_the_stamp_from_values() {
    for (symbol, expected) in [
        ("camlProbe.unit_proc_271", "Probe.unit_proc"),
        ("camlStdlib__List.map_569", "Stdlib__List.map"),
        ("camlProbe.foo__bar_270", "Probe.foo__bar"),
        ("camlProbe.upper_Case_Fun_274", "Probe.upper_Case_Fun"),
        ("camlProbe.fun_350", "Probe.fun"),
        (
            "camlOcaml_pack.Ocaml_pack_base.f_270",
            "Ocaml_pack.Ocaml_pack_base.f",
        ),
        ("camlProbe.foo_2_271", "Probe.foo_2"),
    ] {
        assert_eq!(render(symbol), expected, "{symbol}");
    }
}

#[test]
fn decodes_operator_escapes() {
    for (symbol, expected) in [
        ("camlOps.$2b$2b_270", "Ops.++"),
        ("camlOps.$3d$3d$21_274", "Ops.==!"),
        ("camlOps.$40$40_278", "Ops.@@"),
        ("camlOps.$7e$2d_348", "Ops.~-"),
        ("camlOps..$25$28$29_289", "Ops..%()"),
        // A functor is a module, so its member starts with an uppercase
        // letter; the path only grows when the rest is not a member.
        ("camlOcaml_probe.F_310", "Ocaml_probe.F"),
        (
            "camlOcaml_pack.Ocaml_inner.Ocaml_pack_inner.deep_fun_271",
            "Ocaml_pack.Ocaml_inner.Ocaml_pack_inner.deep_fun",
        ),
        ("camlCamlinternalFormat.k$27_3650", "CamlinternalFormat.k'"),
        ("camlFoo$27.get_271", "Foo'.get"),
    ] {
        assert_eq!(render(symbol), expected, "{symbol}");
    }
}

#[test]
fn escapes_backslash_reversibly() {
    // `$5c` is a decoded backslash, not literal payload text. It must use
    // the reversible identifier escape so the encoding layer can decode it.
    assert_eq!(render("camlProbe.a$5cb_1"), "Probe.a\\\\b");
    assert_eq!(render("camlProbe.a$5c$78$34$31_1"), "Probe.a\\\\x41");
}

#[test]
fn keeps_a_hand_written_dot_escape() {
    // The compiler never escapes `.`; a `$2e` stays verbatim so the printed
    // path keeps exactly one separator per segment.
    assert_eq!(render("camlProbe.a$2eb_1"), "Probe.a$2eb");
    assert_eq!(render("camlProbe.a$00b_1"), "Probe.a$00b");
}

#[test]
fn prints_section_markers() {
    for (symbol, expected) in [
        ("camlProbe.entry", "Probe.entry"),
        ("camlProbe.code_begin", "Probe.code_begin"),
        ("camlProbe.code_end", "Probe.code_end"),
        ("camlProbe.data_begin", "Probe.data_begin"),
        ("camlProbe.data_end", "Probe.data_end"),
        ("camlProbe.frametable", "Probe.frametable"),
        ("camlProbe.gc_roots", "Probe.gc_roots"),
        (
            "camlOcaml_pack.Ocaml_pack_base.entry",
            "Ocaml_pack.Ocaml_pack_base.entry",
        ),
    ] {
        assert_eq!(render(symbol), expected, "{symbol}");
    }
}

#[test]
fn prints_runtime_helpers_as_they_stand() {
    for symbol in [
        "caml_alloc",
        "caml_alloc1",
        "caml_ml_open_descriptor_out",
        "caml_program",
        "caml_startup",
        "caml_main",
        "caml_MD5Init",
        "caml_ba_get_N",
    ] {
        assert_eq!(render(symbol), symbol, "{symbol}");
        assert_eq!(render(&format!("_{symbol}")), symbol, "_{symbol}");
    }
}

#[test]
fn prints_runtime_pseudo_unit_markers() {
    for (symbol, expected) in [
        ("caml_startup.code_begin", "startup.code_begin"),
        ("caml_startup.frametable", "startup.frametable"),
        ("caml_hot.code_begin", "hot.code_begin"),
        ("caml_hot.code_end", "hot.code_end"),
        ("caml_system.frametable", "system.frametable"),
        // The assembler's doubled-underscore spelling keeps both underscores,
        // so it stays distinguishable from the dotted marker.
        ("caml_system__code_begin", "system__code_begin"),
        ("caml_system__code_end", "system__code_end"),
    ] {
        assert_eq!(render(symbol), expected, "{symbol}");
    }
}

#[test]
fn prints_static_data_labels() {
    for (symbol, expected) in [
        ("camlStdlib.11", "Stdlib.11"),
        ("camlBc.1", "Bc.1"),
        ("camlOcaml_probe.3", "Ocaml_probe.3"),
        ("camlCamlinternalFormat.116", "CamlinternalFormat.116"),
        (
            "camlOcaml_pack.Ocaml_pack_base.2",
            "Ocaml_pack.Ocaml_pack_base.2",
        ),
        ("camlOcaml_prime$27.4", "Ocaml_prime'.4"),
    ] {
        assert_eq!(render(symbol), expected, "{symbol}");
    }
}

#[test]
fn peels_host_compiler_fragments() {
    for (symbol, expected) in [
        ("caml_ba_compare.cold", "caml_ba_compare"),
        (
            "caml_fatal_uncaught_exception.cold",
            "caml_fatal_uncaught_exception",
        ),
        ("caml_ephe_clean.part.0", "caml_ephe_clean"),
        ("caml_sys_check_path.part.1", "caml_sys_check_path"),
        ("_caml_ba_hash.cold", "caml_ba_hash"),
    ] {
        assert_eq!(render(symbol), expected, "{symbol}");
    }
}

#[test]
fn strips_linker_decorations() {
    assert_eq!(render("camlProbe.entry.llvm.1234"), "Probe.entry");
    assert_eq!(render("camlProbe.unit_proc_271.llvm.9F"), "Probe.unit_proc");
}

#[test]
fn rejects_unsupported_forms() {
    for symbol in [
        // Fragments are only peeled from a runtime helper.
        "camlProbe.f_1.cold",
        "camlProbe.entry.part.0",
        // A fragment suffix must be well formed.
        "caml_ba_compare.cold.1",
        "caml_ba_compare.part.",
        "caml_ba_compare.part.x",
        "caml_ba_compare.coldx",
        // A pseudo-unit that the runtime does not have, and a non-marker.
        "caml_other.entry",
        "caml_hot.entry_1",
        "caml_other__code_begin",
        "caml_hot__entry_1",
    ] {
        assert!(
            parse_bytes_with_budget(symbol.as_bytes(), DEFAULT_BUDGET).is_err(),
            "{symbol} should be rejected"
        );
    }
}

#[test]
fn rejects_malformed_names() {
    for symbol in [
        "",
        "caml",
        "caml_",
        "caml__alloc",
        "caml_alloc_",
        "caml_alloc__x",
        "camlProbe.",
        "camlProbe.entry_1_",
        "camlProbe._271",
        "camlOps..foo_1",
        "camlProbe.foo.1",
        "camlProbe.foo",
        "camlProbe.-foo_1",
        "camlProbe.foo_1x",
        "camlProbe.1x",
        "camlProbe.1.2",
        "camlProbe.11_",
        "camlprobe.foo_1",
        "caml1Probe.foo_1",
        "camlProbe.foo$2_1",
        "camlProbe.foo$2B_1",
        "camlProbe.foo$ZZ_1",
        "camlProbe.foo bar_1",
        "camlProbe.foo/bar_1",
        "camelProbe.foo_1",
        "camlist.x",
        "_Z3foov",
        "_RNvNtCs1234_7mycrate3foo",
    ] {
        assert!(
            parse_bytes_with_budget(symbol.as_bytes(), DEFAULT_BUDGET).is_err(),
            "{symbol:?} should be rejected"
        );
    }
}

#[test]
fn budgets_are_enforced() {
    let long = format!("caml{}", "A".repeat(MAX_INPUT_LEN));
    assert!(matches!(
        parse_bytes_with_budget(long.as_bytes(), DEFAULT_BUDGET),
        Err(Failure::BudgetExceeded)
    ));

    // The module path is bounded by the input budget alone.
    let deep = alloc::format!("camlA{}", ".A".repeat(4096));
    assert!(parse_bytes_with_budget(deep.as_bytes(), DEFAULT_BUDGET).is_ok());
}

#[test]
fn rejects_non_identifier_bytes() {
    // The compiler escapes every byte outside `[A-Za-z0-9_.]`, so a raw
    // non-ASCII byte is not part of a name.
    for symbol in [
        b"camlProbe.fo\xffo_1".as_slice(),
        b"camlPro\xc3\xa9be.entry".as_slice(),
        b"caml_alloc\x00".as_slice(),
    ] {
        assert!(
            parse_bytes_with_budget(symbol, DEFAULT_BUDGET).is_err(),
            "{symbol:?}"
        );
    }
    assert_eq!(
        parse_bytes_with_budget(b"camlProbe.unit_proc_271", DEFAULT_BUDGET)
            .unwrap()
            .to_string(),
        "Probe.unit_proc"
    );
}

#[test]
fn detection_requires_a_full_parse() {
    for symbol in [
        "camlProbe",
        "camlStdlib__List.map_569",
        "camlProbe.entry",
        "caml_alloc",
        "caml_program",
        "caml_startup.code_begin",
    ] {
        assert!(is_ocaml(symbol.as_bytes()), "{symbol} should be OCaml");
    }
    for symbol in [
        "main",
        "caml",
        "camel",
        "camel_alloc",
        "camlist.x",
        "caml_",
        "caml__alloc",
        "camlProbe.foo",
        "camlProbe.foo.1",
        "caml_ba_compare.cold.1",
        "_Z3foov",
        "_RNvNtCs1234_7mycrate3foo",
        "kfun:com.example.Foo#bar",
        "Java_com_example_Foo_bar",
    ] {
        assert!(!is_ocaml(symbol.as_bytes()), "{symbol} should not be OCaml");
    }
}

/// Render a symbol, panicking with the symbol on a parse error.
fn render(symbol: &str) -> String {
    parse_bytes_with_budget(symbol.as_bytes(), DEFAULT_BUDGET)
        .unwrap_or_else(|error| panic!("{symbol}: {error:?}"))
        .to_string()
}
