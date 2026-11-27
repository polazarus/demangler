use alloc::string::String;

use super::super::MAX_INPUT_LEN;
use super::*;
use crate::cpp::{self, Dialect};

fn parse(symbol: &str) -> Result<Ast<'_>, ParseError> {
    parse_bytes_with_budget(symbol.as_bytes(), DEFAULT_BUDGET)
}

/// The compiled-in legacy limits, for tests that pick a dialect directly.
fn default_limits() -> ParseLimits {
    ParseLimits::legacy(DEFAULT_BUDGET)
}

fn rendered(symbol: &str) -> String {
    let ast = parse(symbol).unwrap_or_else(|error| panic!("{symbol}: {error}"));
    alloc::format!("{ast}")
}

#[test]
fn recognises_legacy_names() {
    for symbol in [
        "foo__Fiv",
        "bar__3foo",
        "__3foo",
        "_$_3foo",
        "_3foo$bar",
        "_vt$foo",
        "__ti3foo",
        "__thunk_4__$_7ostream",
        "a__b__c",
        "__ct__3fooFv",
        "__vtbl__3foo",
        "__sti__foo",
        "foo__pt__2_i",
        "foo__Fi_v",
        "foo__tm__2_i",
        "foo__XSP5_Fv",
        "foo__XT3bar_Fv",
        "foo__Fi_1",
        "foo__FG3bar",
        "_GLOBAL_$I$foo__Fv",
    ] {
        assert!(
            is_legacy_name_bytes(symbol.as_bytes()),
            "{symbol} should be legacy C++"
        );
    }
    for symbol in [
        "main",
        "foo",
        "foo__bar",
        "foo__",
        "__libc_start_main",
        "_ZN3Foo3barEv",
        "?x@@3HA",
        "foo_bar",
        "$s4main3foo",
        "__ct__3fooFzzz",
        "__sti__",
        "foo__Fi_ii",
    ] {
        assert!(
            !is_legacy_name_bytes(symbol.as_bytes()),
            "{symbol} should not be legacy C++"
        );
    }
}

#[test]
fn parses_function_suffixes() {
    assert_eq!(rendered("foo__Fiv"), "foo(int, void)");
    assert_eq!(rendered("foo__Fv"), "foo(void)");
    assert_eq!(rendered("foo__F"), "foo(void)");
    assert_eq!(rendered("foo__Fi"), "foo(int)");
    assert_eq!(rendered("foo__Fii"), "foo(int, int)");
    assert_eq!(rendered("foo__Fve"), "foo(void, ...)");
    assert_eq!(rendered("foo_bar__Fv"), "foo_bar(void)");
    assert_eq!(rendered("foo__Fi3bar3baz"), "foo(int, bar, baz)");
}

#[test]
fn parses_types_and_declarators() {
    for (symbol, expected) in [
        ("foo__FPi", "foo(int *)"),
        ("foo__FRi", "foo(int &)"),
        ("foo__FOi", "foo(int &&)"),
        ("foo__FPPc", "foo(char **)"),
        ("foo__FCi", "foo(int const)"),
        ("foo__FPCc", "foo(char const *)"),
        ("foo__FPCi", "foo(int const *)"),
        ("foo__FUi", "foo(unsigned int)"),
        ("foo__FSs", "foo(signed short)"),
        ("foo__FJc", "foo(__complex char)"),
        ("foo__FI2c_", "foo(int44_t)"),
        ("foo__FA5_i", "foo(int [5])"),
        ("foo__FA3_A4_c", "foo(char [3][4])"),
        ("foo__FPA3_c", "foo(char (*)[3])"),
        ("foo__FPFv", "foo(void (*)(void))"),
        ("foo__FPPFiv", "foo(void (**)(int, void))"),
        ("foo__FPFi_v", "foo(void (*)(int))"),
        ("foo__FPFib_v", "foo(void (*)(int, bool))"),
        ("foo__FPFiPc_i", "foo(int (*)(int, char *))"),
        ("foo__FP4foo_", "foo(foo_ *)"),
        ("foo__FM3fooFv_", "foo(void (foo::*)(void))"),
        ("foo__FM3fooCFv_", "foo(void (foo::*)(void) const)"),
        ("foo__FM3fooFi_v", "foo(void (foo::*)(int))"),
        ("foo__FG3bar", "foo(bar)"),
    ] {
        assert_eq!(rendered(symbol), expected, "{symbol}");
    }
}

#[test]
fn parses_nested_and_qualified_names() {
    assert_eq!(rendered("bar__3foo"), "foo::bar(void)");
    assert_eq!(rendered("foo__3barFv"), "bar::foo(void)");
    assert_eq!(rendered("foo__C3barFv"), "bar::foo(void) const");
    assert_eq!(rendered("foo__S3barFv"), "bar::foo(void) static");
    assert_eq!(rendered("foo__Q23foo3barFv"), "foo::bar::foo(void)");
    assert_eq!(rendered("bar__Q23foo3bar"), "foo::bar::bar(void)");
    assert_eq!(rendered("foo__FiQ23foo3bar"), "foo(int, foo::bar)");
}

#[test]
fn parses_constructors_destructors_and_operators() {
    assert_eq!(rendered("__3foo"), "foo::foo(void)");
    assert_eq!(rendered("__3fooFv"), "foo::foo(void)");
    assert_eq!(rendered("_$_3foo"), "foo::~foo(void)");
    assert_eq!(rendered("__Q23foo3bar"), "foo::bar::bar(void)");
    assert_eq!(rendered("_$_Q23foo3bar"), "foo::bar::~bar(void)");
    assert_eq!(rendered("op$pl__Fv"), "operator+(void)");
    assert_eq!(rendered("op$assign_pl__Fv"), "operator+=(void)");
    assert_eq!(rendered("op$eq__Fv"), "operator==(void)");
    assert_eq!(rendered("__as__3fooR3foo"), "foo::operator=(foo &)");
    assert_eq!(rendered("__apl__3fooPCc"), "foo::operator+=(char const *)");
    assert_eq!(rendered("__opPc__Fv"), "operator char *(void)");
    assert_eq!(rendered("type$3foo__Fv"), "operator foo(void)");
}

#[test]
fn parses_templates_and_back_references() {
    assert_eq!(rendered("foo__t6vector1Zii"), "vector<int>::foo(int)");
    assert_eq!(
        rendered("foo__t6vector2ZiZc"),
        "vector<int, char>::foo(void)"
    );
    assert_eq!(rendered("__t6vector1Zii"), "vector<int>::vector(int)");
    assert_eq!(rendered("foo__FiT0Pc"), "foo(int, int, char *)");
    assert_eq!(rendered("foo__t6vector1b1"), "vector<true>::foo(void)");
    assert_eq!(rendered("foo__t6vector1b0"), "vector<false>::foo(void)");
    assert_eq!(
        rendered("bar__3fooFPiN51"),
        "foo::bar(int *, int *, int *, int *, int *, int *)"
    );
}

#[test]
fn parses_special_names() {
    assert_eq!(rendered("_3foo$bar"), "foo::bar");
    assert_eq!(rendered("_Q23foo3bar$baz"), "foo::bar::baz");
    assert_eq!(rendered("_vt$foo"), "foo virtual table");
    assert_eq!(rendered("_vt$foo$bar"), "foo::bar virtual table");
    assert_eq!(rendered("__vt_3foo"), "foo virtual table");
    assert_eq!(rendered("__ti3foo"), "foo type_info node");
    assert_eq!(
        rendered("__tft6vector1Zi"),
        "vector<int> type_info function"
    );
    assert_eq!(
        rendered("__thunk_4__$_7ostream"),
        "virtual function thunk (delta:-4) for ostream::~ostream(void)"
    );
    assert_eq!(
        rendered("_GLOBAL_$I$foo__Fv"),
        "global constructors keyed to foo(void)"
    );
    assert_eq!(
        rendered("_GLOBAL_.D.foo__Fv"),
        "global destructors keyed to foo(void)"
    );
    assert_eq!(
        rendered("_GLOBAL_$D$__3foo"),
        "global destructors keyed to foo::foo(void)"
    );
}

#[test]
fn reports_cfront_dialects() {
    for (symbol, dialect) in [
        ("foo__Fiv", Dialect::GnuV2),
        ("bar__3foo", Dialect::GnuV2),
        ("_$_3foo", Dialect::GnuV2),
        ("__ct__3fooFv", Dialect::Arm),
        ("__dt__3foo", Dialect::Arm),
        ("__vtbl__3foo__3bar", Dialect::Arm),
        ("__sti__foo", Dialect::Arm),
        ("__std__foo", Dialect::Arm),
        ("foo__pt__2_i", Dialect::Arm),
        ("foo__FiT1", Dialect::Arm),
        ("foo__Fi_v", Dialect::Edg),
        ("foo__F_i", Dialect::Edg),
        ("foo__tm__2_i", Dialect::Edg),
        ("foo__ps__3_ii", Dialect::Edg),
        ("__ct__3fooFi_v", Dialect::Edg),
        // HP aCC is tried last, so only its own productions reach it.
        ("foo__XSP5_Fv", Dialect::Hp),
        ("foo__XT3bar_Fv", Dialect::Hp),
        ("foo__XSP5", Dialect::Hp),
        ("bar__XSP5", Dialect::Hp),
        ("3baz__XSP5", Dialect::Hp),
        ("foo__F3barXSP5", Dialect::Hp),
        ("foo__Fi_1", Dialect::Hp),
        ("bar__3foo_1", Dialect::Hp),
        ("__ct__3fooXSP5_Fv", Dialect::Hp),
        ("__vtbl__3fooXSP5", Dialect::Hp),
    ] {
        let ast = cpp::parse(symbol.as_bytes()).unwrap_or_else(|error| panic!("{symbol}: {error}"));
        assert_eq!(ast.dialect(), dialect, "{symbol}");
    }
}

#[test]
fn parses_hp_template_specs() {
    assert_eq!(rendered("foo__XSP5_Fv"), "foo<5>(void)");
    assert_eq!(rendered("foo__XSP5"), "foo<5>");
    assert_eq!(rendered("foo__X"), "foo<>");
    assert_eq!(rendered("foo__XSP5_"), "foo<5>");
    assert_eq!(rendered("foo__XT3bar_Fv"), "foo<bar>(void)");
    assert_eq!(rendered("foo__XT3barT3baz_Fv"), "foo<bar, baz>(void)");
    assert_eq!(rendered("foo__XTP3bar_Fv"), "foo<bar *>(void)");
    assert_eq!(rendered("foo__XT3barXSP6_Fv"), "foo<bar<6>>(void)");
    assert_eq!(rendered("foo__XSP5T3bar"), "foo<5, bar>");
    // The base name is decoded like the parameterized-type base.
    assert_eq!(rendered("3baz__XSP5"), "baz<5>");
    assert_eq!(rendered("Q2_3foo3bar__XSP5_Fv"), "foo::bar<5>(void)");
    // A spec may follow a class name in type position too.
    assert_eq!(rendered("foo__F3barXSP5"), "foo(bar<5>)");
    assert_eq!(rendered("foo__FP3barXSP5"), "foo(bar<5> *)");
    assert_eq!(rendered("foo__FQ2_3bar3bazXSP5"), "foo(bar::baz<5>)");
    assert_eq!(rendered("foo__F3barXSP5_i"), "foo(bar<5>, int)");
    assert_eq!(rendered("__ct__3fooXSP5_Fv"), "foo<5>::foo(void)");
    assert_eq!(rendered("__dt__3fooXSP5_Fv"), "foo<5>::~foo(void)");
    assert_eq!(rendered("__vtbl__3fooXSP5"), "foo<5> virtual table");
}

#[test]
fn parses_hp_integral_and_literal_arguments() {
    assert_eq!(rendered("foo__XSP0_Fv"), "foo<0>(void)");
    assert_eq!(rendered("foo__XSN0_Fv"), "foo<-0>(void)");
    assert_eq!(rendered("foo__XSN5_Fv"), "foo<-5>(void)");
    assert_eq!(rendered("foo__XUP7_Fv"), "foo<7U>(void)");
    assert_eq!(rendered("foo__XUN7_Fv"), "foo<-7U>(void)");
    assert_eq!(rendered("foo__XSM_Fv"), "foo<-2147483648>(void)");
    // `M` prints without the unsigned suffix, as the reference does.
    assert_eq!(rendered("foo__XUM_Fv"), "foo<-2147483648>(void)");
    assert_eq!(rendered("foo__XSP4294967295_Fv"), "foo<4294967295>(void)");
    assert_eq!(rendered("foo__XA3bar_Fv"), "foo<&bar>(void)");
    assert_eq!(rendered("foo__XA4_bar_Fv"), "foo<&_bar>(void)");
    // A payload that is itself a legacy symbol is demangled.
    assert_eq!(rendered("foo__XA10bar__3baz_Fv"), "foo<&baz::bar>(void)");
    assert_eq!(rendered("foo__XA9_3foo$bar_Fv"), "foo<&_3foo$bar>(void)");
}

#[test]
fn hp_drops_alternate_entry_points() {
    assert_eq!(rendered("foo__Fi_1"), "foo(int)");
    assert_eq!(rendered("foo__Fi_12"), "foo(int)");
    assert_eq!(rendered("foo__Fi_1_2"), "foo(int)");
    assert_eq!(rendered("foo__Fi__"), "foo(int)");
    assert_eq!(rendered("foo__XSP5_Fv_1"), "foo<5>(void)");
    assert_eq!(rendered("bar__3foo_1"), "foo::bar");
    assert_eq!(
        rendered("__sti__bar__Fv_1"),
        "global constructors keyed to bar(void)"
    );
    // Only an underscore run and digits may follow the signature.
    assert!(parse_with(b"foo__Fi_1x", Dialect::Hp, default_limits()).is_err());
    assert!(parse("foo__F3barXSP5_1").is_err());
}

#[test]
fn hp_rejects_productions_it_does_not_support() {
    // The reference demangler rejects the parameterized-type form in HP
    // mode, so HP never reads `__pt__` even though ARM does.
    assert!(parse_with(b"foo__pt__2_i", Dialect::Hp, default_limits()).is_err());
    assert!(parse_with(b"foo__tm__2_i", Dialect::Hp, default_limits()).is_err());
    assert_eq!(
        parse("foo__pt__2_i").map(|ast| ast.dialect()),
        Ok(Dialect::Arm)
    );
    for symbol in [
        "foo__FL1_3bar",
        "foo__XSP_Fv",
        "foo__XA0",
        "foo__XA3ba",
        "foo__XSP5XSP6_Fv",
        "foo__XTA3bar_Fv",
        "foo__FXT3bar",
        "foo__FPXSP5",
        "foo__XAP5_Fv",
        "__ct__XSP5_Fv",
    ] {
        assert!(parse(symbol).is_err(), "{symbol} should be rejected");
    }
}

fn nested_literal(levels: usize) -> String {
    let mut symbol = String::from("bar__3baz");
    for _ in 0..levels {
        let payload = symbol.clone();
        symbol = alloc::format!("foo__XA{}", payload.len());
        symbol.push_str(&payload);
    }
    symbol
}

#[test]
fn hp_literal_nesting_renders_every_level() {
    let symbol = nested_literal(3);
    let ast = parse_with(symbol.as_bytes(), Dialect::Hp, default_limits())
        .expect("nested literals parse");
    let text = alloc::format!("{ast}");
    assert_eq!(text.matches("foo<&").count(), 3, "{text}");
    assert!(text.contains("baz::bar"), "{text}");
}

#[test]
fn hp_literal_nesting_is_charged_to_the_parse_depth() {
    // Nested `A` payloads demangle with a nested parser that continues this
    // parse's depth counter, so a chain of payloads hits the depth budget
    // instead of falling back to undemangled text.
    let symbol = nested_literal(64);
    let error = parse_with(symbol.as_bytes(), Dialect::Hp, default_limits())
        .expect_err("nesting past the depth budget is rejected");
    assert_eq!(error.kind(), ParseErrorKind::TooDeep);
}

#[test]
fn parses_arm_constructors_and_destructors() {
    assert_eq!(rendered("__ct__3foo"), "foo::foo");
    assert_eq!(rendered("__ct__3fooFv"), "foo::foo(void)");
    assert_eq!(rendered("__ct__3fooFi"), "foo::foo(int)");
    assert_eq!(rendered("__dt__3foo"), "foo::~foo");
    assert_eq!(rendered("__dt__3fooFv"), "foo::~foo(void)");
    assert_eq!(rendered("__ct__Q23bar3bazFv"), "bar::baz::baz(void)");
    assert_eq!(rendered("__ct__Q23bar3baz"), "bar::baz::baz");
    assert_eq!(rendered("__dt__Q23bar3bazFv"), "bar::baz::~baz(void)");
    assert_eq!(rendered("__ct__t6vector1Zii"), "vector<int>::vector(int)");
}

#[test]
fn parses_arm_vtables_and_static_wrappers() {
    assert_eq!(rendered("__vtbl__3foo"), "foo virtual table");
    assert_eq!(rendered("__vtbl__3foo__3bar"), "bar::foo virtual table");
    assert_eq!(rendered("__sti__foo"), "global constructors keyed to foo");
    assert_eq!(rendered("__std__foo"), "global destructors keyed to foo");
    assert_eq!(
        rendered("__sti__bar__Fv"),
        "global constructors keyed to bar(void)"
    );
    assert_eq!(
        rendered("__std__bar__3foo"),
        "global destructors keyed to foo::bar"
    );
    assert_eq!(
        rendered("__sti__op$pl__Fv"),
        "global constructors keyed to operator+(void)"
    );
    assert_eq!(
        rendered("__sti__Q23bar3baz"),
        "global constructors keyed to bar::baz"
    );
    assert_eq!(
        rendered("__sti__bar__S3foo"),
        "global constructors keyed to foo::bar static"
    );
}

#[test]
fn parses_parameterized_types() {
    assert_eq!(rendered("foo__pt__2_i"), "foo<int>");
    assert_eq!(rendered("foo__pt__3_ii"), "foo<int, int>");
    assert_eq!(rendered("foo__pt__4_iPc"), "foo<int, char *>");
    assert_eq!(rendered("foo__pt__2_v"), "foo<void>");
    assert_eq!(rendered("Q2_3foo3bar__pt__2_i"), "foo::bar<int>");
    assert_eq!(rendered("3baz__pt__2_i"), "baz<int>");
    assert_eq!(rendered("foo__tm__2_i"), "foo<int>");
    assert_eq!(rendered("foo__ps__3_ii"), "foo<int, int>");
    // The argument list length must match the encoded count exactly.
    for symbol in ["foo__pt__2_Pi", "foo__pt__6_PiPi", "__pt__2_i"] {
        assert!(
            !matches!(parse(symbol).map(|ast| ast.dialect()), Ok(Dialect::Arm)),
            "{symbol} should not be an ARM parameterized type"
        );
    }
}

#[test]
fn cfront_back_references_are_one_based() {
    assert_eq!(rendered("foo__FiT1"), "foo(int, int)");
    assert_eq!(rendered("foo__FiT1T2"), "foo(int, int, int)");
    assert_eq!(rendered("foo__FiN21"), "foo(int, int, int)");
    assert_eq!(rendered("foo__FvN11"), "foo(void, void)");
    assert_eq!(rendered("foo__FPPiT1"), "foo(int **, int **)");
    assert_eq!(
        rendered("foo__FiR3fooT1T2T1T2"),
        "foo(int, foo &, foo &, foo &, foo &, foo &)"
    );
    // The cfront reading only applies when GNU v2 cannot parse the name.
    assert_eq!(
        rendered("__ct__3fooFiR3fooT1T2T1T2"),
        "foo::foo(int, foo &, int, foo &, int, foo &)"
    );
}

#[test]
fn parses_edg_return_types() {
    assert_eq!(rendered("foo__Fi_v"), "foo(int)");
    assert_eq!(rendered("foo__FiPc_"), "foo(int, char *)");
    assert_eq!(rendered("foo__Fi_"), "foo(int)");
    assert_eq!(rendered("foo__Fv_"), "foo(void)");
    assert_eq!(rendered("foo__F_i"), "foo(void)");
    assert_eq!(rendered("__ct__3fooFi_v"), "foo::foo(int)");
    assert_eq!(rendered("__dt__3fooFi_v"), "foo::~foo(int)");
    // ARM has no return-type suffix, so EDG is the only legacy reading.
    for symbol in ["foo__Fi_ii", "__std__bar__3foo__Fi_v"] {
        assert!(parse(symbol).is_err(), "{symbol} should be rejected");
    }
}

#[test]
fn cfront_markers_keep_the_gnu_v2_reading() {
    // A marker only selects a cfront dialect when that dialect consumes
    // the whole symbol; otherwise GNU v2 keeps its reading.
    assert_eq!(rendered("foo__pt__2_Pi"), "_P::foo__pt(int)");
    assert_eq!(rendered("__vtbl__Q23bar3baz"), "bar::baz::__vtbl(void)");
    assert_eq!(rendered("__vtbl__t6vector1Zii"), "vector<int>::__vtbl(int)");
    for symbol in ["__ct__3fooFzzz", "__vtbl__3foo__", "__sti__"] {
        assert!(parse(symbol).is_err(), "{symbol} should be rejected");
    }
}

#[test]
fn cfront_truncations_never_panic() {
    for symbol in [
        "__ct__3fooFiT1",
        "__vtbl__3foo__3bar",
        "__sti__bar__Fv",
        "foo__pt__3_ii",
        "foo__Fi_v",
        "foo__tm__5_PiPc",
        "Q2_3foo3bar__pt__2_i",
        "foo__XSP5_Fv",
        "foo__XT3barXSP6_Fv",
        "foo__F3barXSP5_i",
        "foo__XA10bar__3baz_Fv",
        "__ct__3fooXSP5_Fv",
        "foo__Fi_1_2",
    ] {
        for end in 0..symbol.len() {
            let _ = parse(&symbol[..end]);
        }
        for offset in 0..symbol.len() {
            let mut bytes = String::from(symbol);
            bytes.insert(offset, '_');
            let _ = parse(&bytes);
        }
    }
}

#[test]
fn cfront_budgets_are_enforced() {
    let mut symbol = String::from("__ct__3fooF");
    symbol.push_str(&"P".repeat(2 * MAX_DEPTH as usize));
    symbol.push('i');
    let error = parse(&symbol).unwrap_err();
    assert_eq!(error.kind(), ParseErrorKind::TooDeep);

    let mut symbol = String::from("foo__pt__");
    let arguments = "i".repeat(2 * MAX_NODES as usize);
    let _ = write!(symbol, "{}", arguments.len() + 1);
    symbol.push('_');
    symbol.push_str(&arguments);
    let error = parse(&symbol).unwrap_err();
    assert_eq!(error.kind(), ParseErrorKind::TooManyNodes);
}

#[test]
fn reports_the_dialect_and_shape() {
    let ast = cpp::parse(b"foo__Fiv").expect("legacy name should parse");
    assert_eq!(ast.dialect(), Dialect::GnuV2);
    assert_eq!(ast.root().kind(), Kind::Encoding);
    let function = ast.root().child(0).expect("encoding shape");
    assert_eq!(function.kind(), Kind::Function);
    let parameters = function.child(1).expect("parameter list");
    assert_eq!(parameters.kind(), Kind::ParameterList);
    assert_eq!(parameters.child_count(), 2);
    assert_eq!(function.span(), Some(0..8));
    let name = function.child(0).expect("function name");
    let identifier = name.child(0).expect("identifier");
    assert_eq!(identifier.span(), Some(0..3));
    assert_eq!(identifier.text(), Some("foo"));
}

#[test]
fn rejects_malformed_input() {
    for symbol in [
        "foo__",
        "__",
        "foo__Fzzz",
        "foo__FiA_",
        "foo__t6vector1Zi10",
        "foo__Q23foo",
        "_3foo$",
        "__ti",
        "__thunk_",
        "a__b__c__",
        "foo__FiT9",
        "foo__FGi",
        "_GLOBAL_$I$foo",
    ] {
        assert!(parse(symbol).is_err(), "{symbol} should be rejected");
    }
}

#[test]
fn rejects_unsupported_productions() {
    for symbol in ["foo__H3bar", "foo__K3bar", "foo__B0"] {
        let error = parse(symbol).unwrap_err();
        assert!(
            matches!(
                error.kind(),
                ParseErrorKind::Unsupported
                    | ParseErrorKind::UnexpectedByte
                    | ParseErrorKind::NotLegacy
            ),
            "{symbol}: {error:?}"
        );
    }
}

#[test]
fn cpp_parse_reports_non_legacy_names() {
    let error = cpp::parse(b"main").unwrap_err();
    assert_eq!(error.kind(), ParseErrorKind::NotItanium);
    let error = cpp::parse(b"foo__Fzzz").unwrap_err();
    assert_ne!(error.kind(), ParseErrorKind::NotItanium);
}

#[test]
fn truncations_never_panic() {
    for symbol in [
        "foo__Fiv",
        "bar__3fooFPiN51e",
        "__thunk_4__$_7ostream",
        "_vt$foo$bar",
        "foo__t6vector2ZiZc",
        "foo__FiM3fooCFv_",
    ] {
        for end in 0..symbol.len() {
            let _ = parse(&symbol[..end]);
        }
        for offset in 0..symbol.len() {
            let mut bytes = String::from(symbol);
            bytes.insert(offset, '_');
            let _ = parse(&bytes);
        }
    }
}

#[test]
fn input_budget_is_enforced() {
    let mut symbol = String::from("foo__F");
    symbol.push_str(&"i".repeat(MAX_INPUT_LEN));
    let error = parse(&symbol).unwrap_err();
    assert_eq!(error.kind(), ParseErrorKind::TooLong);
}

#[test]
fn depth_budget_is_enforced() {
    let mut symbol = String::from("foo__F");
    symbol.push_str(&"P".repeat(2 * MAX_DEPTH as usize));
    symbol.push('i');
    let error = parse(&symbol).unwrap_err();
    assert_eq!(error.kind(), ParseErrorKind::TooDeep);
}

#[test]
fn node_budget_is_enforced() {
    // Every `__` is a candidate split point, and each attempt allocates
    // nodes before it fails, so the node budget bounds the whole parse.
    let mut symbol = String::new();
    for _ in 0..40 {
        symbol.push_str(&"i".repeat(100));
        symbol.push_str("__");
    }
    symbol.push_str(&"i".repeat(100));
    let error = parse(&symbol).unwrap_err();
    assert_eq!(error.kind(), ParseErrorKind::TooManyNodes);
}

#[test]
fn long_valid_names_print() {
    let mut symbol = String::from("foo__F");
    for _ in 0..200 {
        symbol.push_str("Pi");
    }
    let text = rendered(&symbol);
    assert!(text.starts_with("foo(int *, int *"));
    assert_eq!(text.matches("int *").count(), 200);
    assert!(text.len() > 1_000);
}
