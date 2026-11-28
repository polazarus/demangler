use alloc::string::ToString;

use super::*;

#[test]
fn shape_errors_reject_the_whole_symbol() {
    // The grammar classifies the whole symbol, so a rejection has no offset.
    let error = parse_bytes_with_budget(b"foo", DEFAULT_BUDGET).unwrap_err();
    assert_eq!(error.kind(), crate::ParseErrorKind::Invalid);
    assert_eq!(error.offset(), None);
}

#[test]
fn parses_module_symbols() {
    for (symbol, expected) in [
        ("__fprobe_MOD_add", "fprobe.add"),
        ("__fprobe_MOD_counter", "fprobe.counter"),
        ("__fprobe_MOD_scale_factor", "fprobe.scale_factor"),
        ("__fprobe_child_MOD_do_child", "fprobe_child.do_child"),
        (
            "__fprobe_generic_MOD_apply_real",
            "fprobe_generic.apply_real",
        ),
        ("__fprobe_MOD_trailing_", "fprobe.trailing_"),
        ("__fprobe_MOD_with_entry2", "fprobe.with_entry2"),
        ("__my_mod__MOD_var_x_", "my_mod_.var_x_"),
        ("__m1_MOD_x2", "m1.x2"),
        ("__a_mod_MOD_x", "a_mod.x"),
        ("__mod_MOD_mod", "mod.mod"),
        ("__m_MOD_od_x", "m.od_x"),
    ] {
        assert_eq!(
            parse_bytes_with_budget(symbol.as_bytes(), DEFAULT_BUDGET)
                .unwrap()
                .to_string(),
            expected,
            "{symbol}"
        );
    }
}

#[test]
fn parses_external_names() {
    for (symbol, expected) in [
        ("ext_sub_", "ext_sub"),
        ("my_ext_sub_", "my_ext_sub"),
        ("ext_with_digits_1_", "ext_with_digits_1"),
        ("probe_blk_", "probe_blk"),
        ("probe_block_", "probe_block"),
        // One appended underscore to a name that already ends with `_`.
        ("extd__", "extd_"),
        ("x1__", "x1_"),
    ] {
        assert_eq!(
            parse_bytes_with_budget(symbol.as_bytes(), DEFAULT_BUDGET)
                .unwrap()
                .to_string(),
            expected,
            "{symbol}"
        );
    }
}

#[test]
fn parses_main_program() {
    assert_eq!(
        parse_bytes_with_budget(b"MAIN__", DEFAULT_BUDGET)
            .unwrap()
            .to_string(),
        "main"
    );
}

#[test]
fn strips_platform_underscore_on_module_form() {
    assert_eq!(
        parse_bytes_with_budget(b"___fprobe_MOD_add", DEFAULT_BUDGET)
            .unwrap()
            .to_string(),
        "fprobe.add"
    );
    assert_eq!(
        parse_bytes_with_budget(b"___m_MOD_x", DEFAULT_BUDGET)
            .unwrap()
            .to_string(),
        "m.x"
    );
}

#[test]
fn strips_linker_decorations() {
    assert_eq!(
        parse_bytes_with_budget(b"__m_MOD_x.llvm.1234", DEFAULT_BUDGET)
            .unwrap()
            .to_string(),
        "m.x"
    );
    assert_eq!(
        parse_bytes_with_budget(b"__m_MOD_x@@GFORTRAN_1", DEFAULT_BUDGET)
            .unwrap()
            .to_string(),
        "m.x"
    );
    assert_eq!(
        parse_bytes_with_budget(b"ext_sub_@@GFORTRAN_1", DEFAULT_BUDGET)
            .unwrap()
            .to_string(),
        "ext_sub"
    );
}

#[test]
fn parses_derived_type_helpers() {
    for (symbol, expected) in [
        ("__fprobe_MOD___vtab_fprobe_Point", "fprobe.Point'vtab"),
        (
            "__fprobe_MOD___def_init_fprobe_Point",
            "fprobe.Point'def_init",
        ),
        ("__fprobe_MOD___copy_fprobe_Point", "fprobe.Point'copy"),
        ("__fprobe_MOD___final_fprobe_Point", "fprobe.Point'final"),
        ("__m_MOD___vtab_m_My_type", "m.My_type'vtab"),
        ("__m_MOD___copy_m_Camelcase", "m.Camelcase'copy"),
        (
            "__m_MOD___def_init_m_T_with_underscore_",
            "m.T_with_underscore_'def_init",
        ),
        ("___m_MOD___vtab_m_T", "m.T'vtab"),
        // A parameterized derived-type instantiation keeps the shape.
        (
            "__holder_MOD___vtab_holder_Pdtkdt_2",
            "holder.Pdtkdt_2'vtab",
        ),
    ] {
        assert_eq!(
            parse_bytes_with_budget(symbol.as_bytes(), DEFAULT_BUDGET)
                .unwrap()
                .to_string(),
            expected,
            "{symbol}"
        );
    }
}

#[test]
fn rejects_unreadable_derived_type_helpers() {
    for symbol in [
        // The helper repeats a module that is not the symbol's own.
        "__holder_MOD___vtab___iso_c_binding_C_ptr",
        "__m_MOD___vtab_other_T",
        // The type part is missing or not a gfortran type spelling.
        "__m_MOD___vtab_m",
        "__m_MOD___vtab_m_",
        "__m_MOD___vtab_m_point",
        "__m_MOD___vtab_m_Point_X",
        "__m_MOD___vtab_m_Point.ext",
        "__m_MOD___vtabs_m_Point",
        // Submodule symbols carry a `.` in the module name.
        "__outer.inner_MOD___vtab_outer.inner_Deep_t",
    ] {
        assert!(
            parse_bytes_with_budget(symbol.as_bytes(), DEFAULT_BUDGET).is_err(),
            "{symbol:?} should be rejected"
        );
    }
}

#[test]
fn rejects_ambiguous_external_names() {
    for symbol in [
        // `x_y__` is `x_y_` (default) or `x_y` (`-fsecond-underscore`).
        "ext_underscored__",
        "ext_underscored___",
        "my_ext_sub__",
        "probe_block__",
        "a_b__",
    ] {
        assert!(
            parse_bytes_with_budget(symbol.as_bytes(), DEFAULT_BUDGET).is_err(),
            "{symbol} should be rejected"
        );
    }
}

#[test]
fn rejects_foreign_and_unsupported_forms() {
    for symbol in [
        // No underscoring: indistinguishable from C.
        "ext_sub",
        "main",
        "probe_blk",
        // `bind(c)` names carry no decoration.
        "c_proc_binding",
        "ext_c",
        // Uppercase cannot come from a gfortran Fortran name.
        "Foo_",
        "MAIN___",
        "MAIN",
        // Local symbols of internal and host-associated procedures.
        "inner.0",
        "master.0.with_entry_",
        // Mach-O external names are not claimed in this slice.
        "__ext_sub_",
        // Malformed module forms.
        "__m_MOD_",
        "__m_MOD_X",
        "__m_OD_x",
        "__MOD_x",
        "m_MOD_x",
        "_m_MOD_x",
        "___MOD_x",
        "__m_MOD_x.y",
        "__m__MOD_",
        // An uppercase marker inside the member cannot be a Fortran name.
        "__m_MOD_x_MOD_y",
    ] {
        assert!(
            parse_bytes_with_budget(symbol.as_bytes(), DEFAULT_BUDGET).is_err(),
            "{symbol:?} should be rejected"
        );
    }
}

#[test]
fn rejects_malformed_names() {
    for symbol in ["", "_", "__", "___", "MOD", "1x_"] {
        assert!(
            parse_bytes_with_budget(symbol.as_bytes(), DEFAULT_BUDGET).is_err(),
            "{symbol:?} should be rejected"
        );
    }
}

#[test]
fn rejects_invalid_utf8() {
    // A module shape is claimed by detection, then rejected on parse.
    assert!(parse_bytes_with_budget(b"__m_MOD_x\x80", DEFAULT_BUDGET).is_err());
    assert!(parse_bytes_with_budget(b"ext\x80_", DEFAULT_BUDGET).is_err());
    assert!(!is_fortran(b"ext\x80_"));
}

#[test]
fn budgets_are_enforced() {
    let long = "a".repeat(MAX_INPUT_LEN + 1);
    assert!(
        matches!(parse_bytes_with_budget(long.as_bytes(), DEFAULT_BUDGET), Err(e) if e.budget_kind().is_some())
    );
    let long_module = alloc::format!("__m_MOD_{long}");
    assert!(
        matches!(parse_bytes_with_budget(long_module.as_bytes(), DEFAULT_BUDGET), Err(e) if e.budget_kind().is_some())
    );
    assert!(!is_fortran(long_module.as_bytes()));
    // The longest accepted name still parses.
    let limit = alloc::format!("{}_", "a".repeat(MAX_INPUT_LEN - 1));
    assert_eq!(
        parse_bytes_with_budget(limit.as_bytes(), DEFAULT_BUDGET)
            .unwrap()
            .to_string()
            .len(),
        MAX_INPUT_LEN - 1
    );
}

#[test]
fn detection_is_precision_only() {
    for symbol in [
        "__fprobe_MOD_add",
        "__fprobe_MOD___vtab_fprobe_Point",
        "___fprobe_MOD_add",
        "__my_mod__MOD_var_x_",
        "__m_MOD_x.llvm.1234",
    ] {
        assert!(is_fortran(symbol.as_bytes()), "{symbol} should be Fortran");
    }
    for symbol in [
        "foo_",
        "foo__",
        "foo",
        "main",
        "MAIN__",
        "ext_sub_",
        "m_MOD_x",
        "_m_MOD_x",
        "__foo",
        "__m_OD_x",
        "c_proc_binding",
    ] {
        assert!(
            !is_fortran(symbol.as_bytes()),
            "{symbol} should not be Fortran"
        );
    }
}
