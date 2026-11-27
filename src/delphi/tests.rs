use alloc::string::ToString;

use super::*;

#[test]
fn parse_errors_describe_themselves() {
    assert_eq!(ParseError::Invalid.to_string(), "invalid syntax");
    assert_eq!(ParseError::BudgetExceeded.to_string(), "size limit reached");
}

#[test]
fn parses_unit_scope_routines() {
    for (symbol, expected) in [
        ("DELPHI_PROBE_$$_NOARGS", "DELPHI_PROBE.NOARGS()"),
        (
            "DELPHI_PROBE_$$_UNITPROC$LONGINT",
            "DELPHI_PROBE.UNITPROC(LONGINT)",
        ),
        (
            "DELPHI_PROBE_$$_UNITFUNC$ANSISTRING$$ANSISTRING",
            "DELPHI_PROBE.UNITFUNC(ANSISTRING): ANSISTRING",
        ),
        (
            "DELPHI_PROBE_$$_MANYPARAMS$LONGINT$INT64$ANSISTRING$POINTER$DOUBLE$BOOLEAN$CHAR",
            "DELPHI_PROBE.MANYPARAMS(LONGINT, INT64, ANSISTRING, POINTER, DOUBLE, BOOLEAN, CHAR)",
        ),
        (
            "DELPHI_PROBE_$$_$plus$TPROBEREC$TPROBEREC$$TPROBEREC",
            "DELPHI_PROBE.$plus(TPROBEREC, TPROBEREC): TPROBEREC",
        ),
    ] {
        assert_eq!(
            parse(symbol)
                .unwrap_or_else(|error| panic!("{symbol}: {error:?}"))
                .to_string(),
            expected,
            "{symbol}"
        );
    }
}

#[test]
fn parses_methods_and_nested_types() {
    for (symbol, expected) in [
        (
            "DELPHI_PROBE$_$TPROBECLASS_$__$$_PLAIN",
            "DELPHI_PROBE.TPROBECLASS.PLAIN()",
        ),
        (
            "DELPHI_PROBE$_$TPROBECLASS_$__$$_ADD$LONGINT$LONGINT$$LONGINT",
            "DELPHI_PROBE.TPROBECLASS.ADD(LONGINT, LONGINT): LONGINT",
        ),
        (
            "DELPHI_PROBE$_$TPROBECLASS_$__$$_CREATE$$TPROBECLASS",
            "DELPHI_PROBE.TPROBECLASS.CREATE(): TPROBECLASS",
        ),
        (
            "DELPHI_PROBE$_$TPROBECLASS_$_TNESTED_$__$$_DEEP",
            "DELPHI_PROBE.TPROBECLASS.TNESTED.DEEP()",
        ),
        (
            "DELPHI_PROBE$_$TPROBEREC_$__$$_MOVEBY$LONGINT$LONGINT",
            "DELPHI_PROBE.TPROBEREC.MOVEBY(LONGINT, LONGINT)",
        ),
    ] {
        assert_eq!(
            parse(symbol)
                .unwrap_or_else(|error| panic!("{symbol}: {error:?}"))
                .to_string(),
            expected,
            "{symbol}"
        );
    }
}

#[test]
fn prints_open_arrays_and_namespaced_units() {
    for (symbol, expected) in [
        (
            "DELPHI_PROBE_$$_UNITOPENARRAY$array_of_ANSISTRING",
            "DELPHI_PROBE.UNITOPENARRAY(array of ANSISTRING)",
        ),
        (
            "DELPHI_PROBE$_$TPROBECLASS_$__$$_OPENARRAYCONST$array_of_const",
            "DELPHI_PROBE.TPROBECLASS.OPENARRAYCONST(array of const)",
        ),
        (
            "NS.ESC$_$TFOO_BAR_$__$$_METHODTWO$LONGINT",
            "NS.ESC.TFOO_BAR.METHODTWO(LONGINT)",
        ),
        (
            "P$DELPHI_PROBE_PROG_$$_PROGPROC$LONGINT",
            "P$DELPHI_PROBE_PROG.PROGPROC(LONGINT)",
        ),
    ] {
        assert_eq!(
            parse(symbol)
                .unwrap_or_else(|error| panic!("{symbol}: {error:?}"))
                .to_string(),
            expected,
            "{symbol}"
        );
    }
}

#[test]
fn strips_linker_decorations() {
    assert_eq!(
        parse("DELPHI_PROBE_$$_NOARGS.llvm.1234")
            .unwrap()
            .to_string(),
        "DELPHI_PROBE.NOARGS()"
    );
    assert_eq!(
        parse("DELPHI_PROBE_$$_NOARGS@@FPC_3.2")
            .unwrap()
            .to_string(),
        "DELPHI_PROBE.NOARGS()"
    );
}

#[test]
fn parses_compiler_data_symbols() {
    for (symbol, expected) in [
        (
            "U_$DELPHI_PROBE_$$_UNITCOUNTER",
            "U_$DELPHI_PROBE.UNITCOUNTER",
        ),
        ("U_$P$DELPHI_PROBE_PROG_$$_G", "U_$P$DELPHI_PROBE_PROG.G"),
        (
            "VMT_$DELPHI_PROBE_$$_TPROBECLASS",
            "VMT_$DELPHI_PROBE.TPROBECLASS",
        ),
        (
            "VMT_$DELPHI_PROBE_$$_TPROBECLASS$indirect",
            "VMT_$DELPHI_PROBE.TPROBECLASS$indirect",
        ),
        (
            "VMT_$DELPHI_PROBE$_$TPROBECLASS_$__$$_TNESTED",
            "VMT_$DELPHI_PROBE.TPROBECLASS.TNESTED",
        ),
        (
            "RTTI_$DELPHI_PROBE_$$_TPROBEENUM_o2s",
            "RTTI_$DELPHI_PROBE.TPROBEENUM_o2s",
        ),
        (
            "RTTI_$DELPHI_PROBE.NS_$$_TNAMESPACED$indirect",
            "RTTI_$DELPHI_PROBE.NS.TNAMESPACED$indirect",
        ),
        (
            "INIT_$DELPHI_PROBE_$$_TPROBECLASS",
            "INIT_$DELPHI_PROBE.TPROBECLASS",
        ),
        ("INIT$_$DELPHI_PROBE", "INIT$_$DELPHI_PROBE"),
        ("FINALIZE$_$DELPHI_PROBE.NS", "FINALIZE$_$DELPHI_PROBE.NS"),
        ("DELPHI_PROBE_$$_init$", "DELPHI_PROBE.init$()"),
        (
            "DELPHI_PROBE_$$_finalize_implicit$",
            "DELPHI_PROBE.finalize_implicit$()",
        ),
        // A unit whose name matches a data prefix keeps its routines: no
        // unit name can contain `$`, so the routine grammar wins when the
        // data grammar does not match.
        ("U_$$_PROC$LONGINT", "U.PROC(LONGINT)"),
        ("INIT_$$_PROC", "INIT.PROC()"),
        ("RTTI_$$_FOO$LONGINT", "RTTI.FOO(LONGINT)"),
    ] {
        assert_eq!(
            parse(symbol)
                .unwrap_or_else(|error| panic!("{symbol}: {error:?}"))
                .to_string(),
            expected,
            "{symbol}"
        );
    }
}

#[test]
fn rejects_unsupported_forms() {
    for symbol in [
        // Nested routines: the enclosing chain is `_`-joined and ambiguous.
        "DELPHI_PROBE$_$NESTEDOWNER_$$_LOCALPROC$LONGINT",
        "DELPHI_PROBE$_$OVERLOADED$LONGINT_$$_LOCALINOVERLOAD",
        "DELPHI_PROBE$_$TPROBECLASS_$_WITHLOCAL_$$_LOCALOFMETHOD$LONGINT",
        // Generic instantiations carry an undecodable `$<n>$CRC<hex>`.
        "DELPHI_PROBE$_$TPROBEBOX$1$CRC9F312717_$__$$_PUT$LONGINT",
        "DELPHI_PROBE_$$_USESGENERIC$TPROBEBOX$1$CRC9F312717",
        "VMT_$DELPHI_PROBE_$$_TPROBEBOX$1$CRC9F312717",
        "RTTI_$DELPHI_PROBE_$$_TPROBEBOX$1$CRC9F312717$indirect",
        "INIT_$DELPHI_PROBE_$$_TPROBEBOX$1$CRC9F312717",
        // Hash substitutions.
        "DELPHI_PROBE_$$_MANYPARAMS$crc8415F53F",
        "DELPHI_PROBE$_$$CRCF62B7D7C_$$_INNEROFLONGOUTER",
        // Unit-wide tables and runtime data, which name no type or variable.
        "DBG_$DELPHI_PROBE_$$_TPROBECLASS",
        "DEBUGSTART_$DELPHI_PROBE",
        "DEBUGEND_$DELPHI_PROBE.NS",
        "THREADVARLIST_$DELPHI_PROBE",
        "FPC_THREADVARTABLES",
        "INITFINAL",
        "__fpc_ident",
        // File types and the error placeholder.
        "DELPHI_PROBE_$$_P$FILE$OF$BYTE",
        "DELPHI_PROBE_$$_P$FILE",
        "DELPHI_PROBE_$$_P$error",
        // The classic Delphi form has no supported signature decoding.
        "@System@@LStrClr$QQRPV",
        "@System@TObject@Create",
    ] {
        assert!(parse(symbol).is_err(), "{symbol} should be rejected");
    }
}

#[test]
fn rejects_malformed_names() {
    for symbol in [
        "",
        "DELPHI_PROBE",
        "DELPHI_PROBE_$$_",
        "DELPHI_PROBE_$$_$",
        "DELPHI_PROBE_$$_NOARGS$",
        "DELPHI_PROBE_$$_NOARGS$LONGINT$",
        "DELPHI_PROBE_$$_NOARGS$$",
        "DELPHI_PROBE_$$_NOARGS$$$$LONGINT",
        "DELPHI_PROBE_$$_NOARGS$array_of_",
        "DELPHI_PROBE_$$_NOARGS$array_of_constX",
        "DELPHI_PROBE_$$_NOARGS$lowercase",
        "DELPHI_PROBE$_$TPROBECLASS_$$_PLAIN",
        "DELPHI_PROBE$_$_$$_PLAIN",
        "DELPHI_PROBE$_$TPROBECLASS_$__$$_",
        "P$_$$_PLAIN",
        "DELPHI_PROBE..X_$$_PLAIN",
        "DELPHI_PROBE_$$_NO-ARGS",
        "DELPHI_PROBE_$$_NO ARGS",
        // Compiler data symbols must carry the full chain.
        "U_$",
        "U_$DELPHI_PROBE",
        "U_$DELPHI_PROBE_$$_",
        "U_$DELPHI_PROBE_$$_X$indirect",
        "VMT_$DELPHI_PROBE_$$_TPROBECLASS$indirectX",
        "RTTI_$",
        "INIT_$DELPHI_PROBE_$$_TPROBECLASS$",
        // Unit entries name the unit alone.
        "INIT$_$",
        "INIT$_$DELPHI_PROBE_$$_X",
        "FINALIZE$_$DELPHI_PROBE$",
        // The entry routine names are matched exactly.
        "DELPHI_PROBE_$$_init$x",
    ] {
        assert!(parse(symbol).is_err(), "{symbol:?} should be rejected");
    }
}

#[test]
fn budgets_are_enforced() {
    let long = "A".repeat(MAX_INPUT_LEN + 1);
    assert!(matches!(parse(&long), Err(ParseError::BudgetExceeded)));

    // Parameter and scope lists are bounded by the input budget alone.
    let many = alloc::format!("UNIT_$$_PROC{}", "$LONGINT".repeat(1024));
    assert!(parse(&many).is_ok());
    let scopes = alloc::format!("UNIT$_${}_$$_PROC", "TCLASS_$_".repeat(1024));
    assert!(parse(&scopes).is_ok());
}

#[test]
fn rejects_non_identifier_bytes() {
    // Free Pascal identifiers are ASCII, so a non-ASCII byte is not a name.
    for symbol in [
        b"DELPHI_PROBE_$$_NO\xffARGS".as_slice(),
        b"DELPHI_PROBE_$$_\xc3\xa9".as_slice(),
        b"DELPHI_PROBE$_$TCLASS_$__$$_M\x80".as_slice(),
    ] {
        assert!(parse_bytes(symbol).is_err(), "{symbol:?}");
    }
    assert_eq!(
        parse_bytes(b"DELPHI_PROBE_$$_NOARGS").unwrap().to_string(),
        "DELPHI_PROBE.NOARGS()"
    );
}

#[test]
fn detection_requires_a_full_parse() {
    for symbol in [
        "DELPHI_PROBE_$$_NOARGS",
        "DELPHI_PROBE$_$TPROBECLASS_$__$$_PLAIN",
        "NS.ESC_$$_FOO_BAR$LONGINT",
        "P$DELPHI_PROBE_PROG_$$_PROGPROC$LONGINT",
    ] {
        assert!(is_delphi(symbol.as_bytes()), "{symbol} should be Delphi");
    }
    for symbol in [
        "main",
        "foo_bar",
        "foo__bar",
        "_Z3foov",
        "_RNvNtCs1234_7mycrate3foo",
        "DELPHI_PROBE_$$_NOARGS$",
        "DELPHI_PROBE_$$_USESGENERIC$TPROBEBOX$1$CRC9F312717",
        "@System@@LStrClr$QQRPV",
    ] {
        assert!(
            !is_delphi(symbol.as_bytes()),
            "{symbol} should not be Delphi"
        );
    }
}
