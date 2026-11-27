//! Legacy C++ productions that the vendored corpora do not exercise.
//!
//! Pins every GNU v2 operator code of the libiberty operator table, the
//! `assign_` and `op$` operator spellings, member-function pointers with
//! each qualifier and class form, `I` sized integers and integral template
//! arguments, plus the inputs the legacy grammar rejects.
#![cfg(feature = "cpp-gnu-v2")]

use demangler::cpp;

/// `(mangled, expected)` pairs in the libiberty style.
const CASES: &[(&str, &str)] = &[
    ("__nw__3Fooi", "Foo::operator new(int)"),
    ("__new__3Fooi", "Foo::operator new(int)"),
    ("__dl__3Fooi", "Foo::operator delete(int)"),
    ("__delete__3Fooi", "Foo::operator delete(int)"),
    ("__vn__3Fooi", "Foo::operator new[](int)"),
    ("__vd__3Fooi", "Foo::operator delete[](int)"),
    ("__as__3Fooi", "Foo::operator=(int)"),
    ("__ne__3Fooi", "Foo::operator!=(int)"),
    ("__eq__3Fooi", "Foo::operator==(int)"),
    ("__ge__3Fooi", "Foo::operator>=(int)"),
    ("__gt__3Fooi", "Foo::operator>(int)"),
    ("__le__3Fooi", "Foo::operator<=(int)"),
    ("__lt__3Fooi", "Foo::operator<(int)"),
    ("__plus__3Fooi", "Foo::operator+(int)"),
    ("__pl__3Fooi", "Foo::operator+(int)"),
    ("__convert__3Fooi", "Foo::operator+(int)"),
    ("__apl__3Fooi", "Foo::operator+=(int)"),
    ("__minus__3Fooi", "Foo::operator-(int)"),
    ("__mi__3Fooi", "Foo::operator-(int)"),
    ("__negate__3Fooi", "Foo::operator-(int)"),
    ("__ami__3Fooi", "Foo::operator-=(int)"),
    ("__mult__3Fooi", "Foo::operator*(int)"),
    ("__ml__3Fooi", "Foo::operator*(int)"),
    ("__indirect__3Fooi", "Foo::operator*(int)"),
    ("__amu__3Fooi", "Foo::operator*=(int)"),
    ("__aml__3Fooi", "Foo::operator*=(int)"),
    ("__md__3Fooi", "Foo::operator%(int)"),
    ("__amd__3Fooi", "Foo::operator%=(int)"),
    ("__dv__3Fooi", "Foo::operator/(int)"),
    ("__adv__3Fooi", "Foo::operator/=(int)"),
    ("__aa__3Fooi", "Foo::operator&&(int)"),
    ("__oo__3Fooi", "Foo::operator||(int)"),
    ("__nt__3Fooi", "Foo::operator!(int)"),
    ("__postincrement__3Fooi", "Foo::operator++(int)"),
    ("__pp__3Fooi", "Foo::operator++(int)"),
    ("__postdecrement__3Fooi", "Foo::operator--(int)"),
    ("__mm__3Fooi", "Foo::operator--(int)"),
    ("__bit_ior__3Fooi", "Foo::operator|(int)"),
    ("__or__3Fooi", "Foo::operator|(int)"),
    ("__aor__3Fooi", "Foo::operator|=(int)"),
    ("__bit_xor__3Fooi", "Foo::operator^(int)"),
    ("__er__3Fooi", "Foo::operator^(int)"),
    ("__aer__3Fooi", "Foo::operator^=(int)"),
    ("__bit_and__3Fooi", "Foo::operator&(int)"),
    ("__ad__3Fooi", "Foo::operator&(int)"),
    ("__addr__3Fooi", "Foo::operator&(int)"),
    ("__aad__3Fooi", "Foo::operator&=(int)"),
    ("__bit_not__3Fooi", "Foo::operator~(int)"),
    ("__co__3Fooi", "Foo::operator~(int)"),
    ("__call__3Fooi", "Foo::operator()(int)"),
    ("__cl__3Fooi", "Foo::operator()(int)"),
    ("__alshift__3Fooi", "Foo::operator<<(int)"),
    ("__ls__3Fooi", "Foo::operator<<(int)"),
    ("__als__3Fooi", "Foo::operator<<=(int)"),
    ("__arshift__3Fooi", "Foo::operator>>(int)"),
    ("__rs__3Fooi", "Foo::operator>>(int)"),
    ("__ars__3Fooi", "Foo::operator>>=(int)"),
    ("__component__3Fooi", "Foo::operator->(int)"),
    ("__pt__3Fooi", "Foo::operator->(int)"),
    ("__rf__3Fooi", "Foo::operator->(int)"),
    ("__method_call__3Fooi", "Foo::operator->()(int)"),
    ("__array__3Fooi", "Foo::operator[](int)"),
    ("__vc__3Fooi", "Foo::operator[](int)"),
    ("__compound__3Fooi", "Foo::operator, (int)"),
    ("__cm__3Fooi", "Foo::operator, (int)"),
    ("__cond__3Fooi", "Foo::operator?:(int)"),
    ("__cn__3Fooi", "Foo::operator?:(int)"),
    ("__max__3Fooi", "Foo::operator>?(int)"),
    ("__mx__3Fooi", "Foo::operator>?(int)"),
    ("__min__3Fooi", "Foo::operator<?(int)"),
    ("__mn__3Fooi", "Foo::operator<?(int)"),
    ("__nop__3Fooi", "Foo::operator(int)"),
    ("__rm__3Fooi", "Foo::operator->*(int)"),
    ("__sz__3Fooi", "Foo::operator sizeof (int)"),
    ("__assign_plus__3Fooi", "Foo::operator+=(int)"),
    ("__assign_minus__3Fooi", "Foo::operator-=(int)"),
    ("__assign_mult__3Fooi", "Foo::operator*=(int)"),
    ("__assign_trunc_mod__3Fooi", "Foo::operator%=(int)"),
    ("__assign_trunc_div__3Fooi", "Foo::operator/=(int)"),
    ("__assign_bit_ior__3Fooi", "Foo::operator|=(int)"),
    ("__assign_bit_xor__3Fooi", "Foo::operator^=(int)"),
    ("__assign_bit_and__3Fooi", "Foo::operator&=(int)"),
    ("__assign_alshift__3Fooi", "Foo::operator<<=(int)"),
    ("__assign_arshift__3Fooi", "Foo::operator>>=(int)"),
    ("__assign_apl__3Fooi", "Foo::operator+=(int)"),
    ("__assign_aml__3Fooi", "Foo::operator*=(int)"),
    // Operators whose code starts with `t` read as a template-class
    // constructor after `__`; the `op$` and `op.` spellings reach them.
    ("op$trunc_mod__3Fooi", "Foo::operator%(int)"),
    ("op$truth_not__3Foo", "Foo::operator!(void)"),
    ("op.trunc_div__3Fooi", "Foo::operator/(int)"),
    ("op$assign_trunc_mod__3Fooi", "Foo::operator%=(int)"),
    ("op$assign_plus__3Fooi", "Foo::operator+=(int)"),
    // Member-function pointers: qualifiers and class forms.
    ("foo__FM3fooVFv_", "foo(void (foo::*)(void) volatile)"),
    ("foo__FM3foouFv_", "foo(void (foo::*)(void) __restrict)"),
    (
        "foo__FM3fooCVFv_",
        "foo(void (foo::*)(void) const volatile)",
    ),
    ("foo__FMQ21A1BFv_", "foo(void (A::B::*)(void))"),
    ("foo__FMt3Bar1ZiFv_", "foo(void (Bar<int>::*)(void))"),
    // Sized integers and integral template arguments.
    ("foo__FI20_", "foo(int32_t)"),
    ("foo__FCI20_", "foo(int32_t const)"),
    ("foo__t3Bar1i5", "Bar<5>::foo(void)"),
    ("foo__t3Bar1i-5", "Bar<-5>::foo(void)"),
    (
        "foo__FUlUlUl",
        "foo(unsigned long, unsigned long, unsigned long)",
    ),
    (
        "_GLOBAL_$D$foo__Fv",
        "global destructors keyed to foo(void)",
    ),
];

/// Inputs the legacy grammar rejects.
const REJECTED: &[&str] = &[
    // `__H` is the unsupported member-template form.
    "__H1Zi_3Foo",
    "f__H1Zi_v",
    // A member pointer names a class.
    "foo__FMiFv_",
    // A sized integer needs hex digits.
    "foo__FI_",
    "foo__FIzz_",
    // Template template parameters are unsupported.
    "foo__t3Bar1zi",
    "foo__t3Bar1Zz",
    "foo__Ftz3Bar",
    // A count past `usize::MAX` overflows.
    "foo__F99999999999999999999999i",
    "foo__Fi99999999999999999999999",
    // Old operator names starting with `t` are template constructors here.
    "__trunc_mod__3Fooi",
    "__truth_not__3Fooi",
];

#[test]
fn rare_productions_print() {
    for &(symbol, expected) in CASES {
        let ast = cpp::parse(symbol.as_bytes(), demangler::Budget::default())
            .unwrap_or_else(|error| panic!("{symbol}: {error}"));
        assert_eq!(ast.to_string(), expected, "{symbol}");
    }
}

#[test]
fn malformed_productions_are_rejected() {
    for &symbol in REJECTED {
        assert!(
            cpp::parse(symbol.as_bytes(), demangler::Budget::default()).is_err(),
            "{symbol} should be rejected"
        );
    }
}
