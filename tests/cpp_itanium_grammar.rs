//! Itanium C++ productions that the vendored corpora do not exercise.
//!
//! The libc++abi and libiberty corpora cover the common grammar; this table
//! pins the rarer productions (thunks, local-name discriminators, vendor and
//! module names, the `D` builtins, vector types, expression forms) and the
//! numeric overflow guards, so they stay tested in the published crate too.
#![cfg(feature = "cpp-itanium")]

use demangler::cpp;

/// `(mangled, expected)` pairs printed in the libc++abi style.
const CASES: &[(&str, &str)] = &[
    // Block invocations and thunks.
    (
        "___Z3foov_block_invoke_12",
        "invocation function for block in foo()",
    ),
    ("_ZTch0_h4_3foov", "covariant return thunk to foo()"),
    ("_ZTcv0_n8_h4_3foov", "covariant return thunk to foo()"),
    ("_ZTh0_3foov", "non-virtual thunk to foo()"),
    ("_ZTv0_n8_3foov", "virtual thunk to foo()"),
    // Special names.
    ("_ZGR3foo", "reference temporary for foo"),
    ("_ZGR3foo_", "reference temporary for foo"),
    ("_ZGR3foo0_", "reference temporary for foo"),
    ("_ZGR3fooA_", "reference temporary for foo"),
    ("_ZGIW3foo", "initializer for module foo"),
    ("_ZGIW3fooWP3bar", "initializer for module foo:bar"),
    ("_ZTC3Foo0_3Bar", "construction vtable for Bar-in-Foo"),
    ("_ZTAXtl3FooLi1EEE", "template parameter object for Foo{1}"),
    // Local names and their discriminators.
    ("_ZZ3foovEs", "foo()::string literal"),
    ("_ZZ3foovEs_0", "foo()::string literal"),
    ("_ZZ3foovEs__12_", "foo()::string literal"),
    ("_ZZ3foovE1a__12_", "foo()::a"),
    ("_ZZ3foovE1a_3", "foo()::a"),
    ("_ZZ3foovE1a3", "foo()::a"),
    // Nested-name qualifiers, `std` and module names.
    ("_ZNrVK3Foo3barEv", "Foo::bar() const volatile restrict"),
    ("_ZNrK3Foo3barEv", "Foo::bar() const restrict"),
    ("_ZSt3fooIiEvv", "void std::foo<int>()"),
    ("_ZNSt3fooIiE3barEv", "std::foo<int>::bar()"),
    ("_ZW3FOO3FooIiEvv", "void Foo@FOO<int>()"),
    ("_ZN3FooW3BAR3bazEv", "Foo::baz@BAR()"),
    ("_Z3fooUa9enable_ifIXLb1EEEv", "foo() [enable_if:true]"),
    // `D` builtins.
    ("_Z1fDs", "f(char16_t)"),
    ("_Z1fDi", "f(char32_t)"),
    ("_Z1fDh", "f(half)"),
    ("_Z1fDd", "f(decimal64)"),
    ("_Z1fDe", "f(decimal128)"),
    ("_Z1fDf", "f(decimal32)"),
    ("_Z1fDF16_", "f(_Float16)"),
    ("_Z1fDF16b", "f(std::bfloat16_t)"),
    ("_Z1fDB8_", "f(_BitInt(8))"),
    ("_Z1fDU8_", "f(unsigned _BitInt(8))"),
    ("_Z1fw", "f(wchar_t)"),
    // Vector types, with a numeric and an expression dimension.
    ("_Z1fDv4_i", "f(int vector[4])"),
    ("_Z1fDvLi4E_i", "f(int vector[4])"),
    // Qualified pointees and function types.
    ("_Z1fPri", "f(int restrict*)"),
    ("_Z1fPrKi", "f(int const restrict*)"),
    ("_Z1fPrVi", "f(int volatile restrict*)"),
    ("_Z1fPrVKi", "f(int const volatile restrict*)"),
    ("_Z1fPKFvvE", "f(void (*)() const)"),
    ("_Z1fPrFvvE", "f(void (*)() restrict)"),
    ("_Z1fU13objcproto3Fooi", "f(int<Foo>)"),
    // Expression forms inside `decltype`.
    (
        "_Z3fooIiEDTclfp_EES0_",
        "decltype(fp()) foo<int>(decltype(fp()))",
    ),
    (
        "_Z3fooIiEDTtrES0_",
        "decltype(throw) foo<int>(decltype(throw))",
    ),
    (
        "_Z3fooIiEDTtiT_ES0_",
        "decltype(typeid (int)) foo<int>(int)",
    ),
    (
        "_Z3fooIiEDTtefp_ES0_",
        "decltype(typeid (fp)) foo<int>(decltype(typeid (fp)))",
    ),
    (
        "_Z3fooIiEDTcvT_fp_ES0_",
        "decltype((int)(fp)) foo<int>(int)",
    ),
    ("_Z3fooIiEDTsrT_1xES0_", "decltype(int::x) foo<int>(int)"),
    (
        "_Z3fooIiEDTsrDtfp_E1xES0_",
        "decltype(decltype(fp)::x) foo<int>(decltype(fp))",
    ),
    ("_Z3fooIiEDTdnT_ES0_", "decltype(~int) foo<int>(int)"),
    ("_Z3fooIiEDTdn1xES0_", "decltype(~x) foo<int>(decltype(~x))"),
    ("_Z3fooIiEDTsrT_dn1xES0_", "decltype(int::~x) foo<int>(int)"),
    (
        "_Z3fooIiEDTsrT_onplES0_",
        "decltype(int::operator+) foo<int>(int)",
    ),
    (
        "_Z3fooIiEDTsrT_onplIiEES0_",
        "decltype(int::operator+<int>) foo<int>(int)",
    ),
    // More local names, block literals and packs.
    ("_ZZ3foovEd_1a", "foo()::a"),
    ("_ZZ3foovEd0_1a", "foo()::a"),
    ("_ZN3FooUb1_E", "Foo::'block-literal'"),
    ("_ZN3FooUb_E", "Foo::'block-literal'"),
    ("_Z1fIJiiEEvv", "void f<int, int>()"),
    ("_Z1fIJEEvv", "void f<>()"),
    ("_ZTW3foo", "thread-local wrapper routine for foo"),
    ("_Z1fDpi", "f(int...)"),
    ("_Z1fDv_i", "f(int vector[])"),
    ("_Z1fU3fooKi", "f(int const foo)"),
    ("_Z1fU3fooKFvvE", "f(void () const foo)"),
    ("_Z1fDAi", "f(_Accum)"),
    ("_Z1fDSDAi", "f(_Sat _Accum)"),
    ("_Z1fDSDRj", "f(_Sat unsigned _Fract)"),
    ("_Z1fFYvvE", "f(void ())"),
    ("_Z1fFYvvOE", "f(void () &&)"),
    // Calls, conversions, casts and initializers.
    ("_Z1fIiEvDTcpfp_fp_EE", "void f<int>(decltype((fp)(fp)))"),
    ("_Z1fIiEvDTclfp_fp_EE", "void f<int>(decltype(fp(fp)))"),
    (
        "_Z1fIiEvDTcvT__fp_fp_EE",
        "void f<int>(decltype((int)(fp, fp)))",
    ),
    ("_Z1fIiEvDTcvT__EE", "void f<int>(decltype((int)()))"),
    ("_Z1fIiEvDTtlT_fp_EE", "void f<int>(decltype(int{fp}))"),
    ("_Z1fIiEvDTilfp_EE", "void f<int>(decltype({fp}))"),
    ("_Z1fIiEvDTilEE", "void f<int>(decltype({}))"),
    (
        "_Z1fIJiEEvDTsPiEE",
        "void f<int>(decltype(sizeof... (int)))",
    ),
    (
        "_Z1fIiEvDTsZT_E",
        "void f<int>(decltype(sizeof...(int...)))",
    ),
    (
        "_Z1fIiEvDTsZfp_E",
        "void f<int>(decltype(sizeof...(fp...)))",
    ),
    (
        "_Z1fIiEvDTscT_fp_E",
        "void f<int>(decltype(static_cast<int>(fp)))",
    ),
    (
        "_Z1fIiEvDTdcT_fp_E",
        "void f<int>(decltype(dynamic_cast<int>(fp)))",
    ),
    (
        "_Z1fIiEvDTccT_fp_E",
        "void f<int>(decltype(const_cast<int>(fp)))",
    ),
    (
        "_Z1fIiEvDTrcT_fp_E",
        "void f<int>(decltype(reinterpret_cast<int>(fp)))",
    ),
    (
        "_Z1fIiEvDTmcM1AFvvEfp_EE",
        "void f<int>(decltype((void (A::*)())(fp)))",
    ),
    (
        "_Z1fIiEvDTmcM1AFvvEfp_8EE",
        "void f<int>(decltype((void (A::*)())(fp)))",
    ),
    (
        "_Z1fIiEvDTsoT_fp_EE",
        "void f<int>(decltype(fp.<int at offset 0>))",
    ),
    (
        "_Z1fIiEvDTsoT_fp_8_0pEE",
        "void f<int>(decltype(fp.<int at offset 8>))",
    ),
    (
        "_Z1fIiEvDTsoT_fp_n8EE",
        "void f<int>(decltype(fp.<int at offset -8>))",
    ),
    (
        "_Z1fIiEvDTu8__uuidoftiE",
        "void f<int>(decltype(__uuidof(int)))",
    ),
    (
        "_Z1fIiEvDTu8__uuidofzfp_E",
        "void f<int>(decltype(__uuidof(fp)))",
    ),
    ("_Z1fIiEvDTu3fooiEE", "void f<int>(decltype(foo(int)))"),
    // new, delete and requires.
    ("_Z1fIiEvDTnw_iEE", "void f<int>(decltype(new int))"),
    ("_Z1fIiEvDTnw_ipiEE", "void f<int>(decltype(new int))"),
    ("_Z1fIiEvDTgsnw_ipiEE", "void f<int>(decltype(::new int))"),
    (
        "_Z1fIiEvDTnwfp__ipiEE",
        "void f<int>(decltype(new(fp) int))",
    ),
    (
        "_Z1fIiEvDTnafp__iEE",
        "void f<int>(decltype(new[](fp) int))",
    ),
    ("_Z1fIiEvDTdlfp_E", "void f<int>(decltype(delete fp))"),
    ("_Z1fIiEvDTgsdafp_E", "void f<int>(decltype(::delete[] fp))"),
    (
        "_Z1fIiEvDTrqXfp_EE",
        "void f<int>(decltype(requires { fp; }))",
    ),
    (
        "_Z1fIiEvDTrQi_Xfp_EE",
        "void f<int>(decltype(requires (int) { fp; }))",
    ),
    (
        "_Z1fIiEvDTrQ_Xfp_EE",
        "void f<int>(decltype(requires () { fp; }))",
    ),
    // Unresolved names.
    ("_Z1fIiEvDTsrT_IiE1xE", "void f<int>(decltype(int<int>::x))"),
    ("_Z1fIiEvDTsr1A1BE1xE", "void f<int>(decltype(A::B::x))"),
    ("_Z1fIiEvDTgssr1A1BE1xE", "void f<int>(decltype(::A::B::x))"),
    ("_Z1fIiEvDTgssr1AE1xE", "void f<int>(decltype(::A::x))"),
    (
        "_Z1fIiEvDTsr1AIiE1BE1xE",
        "void f<int>(decltype(A<int>::B::x))",
    ),
    ("_Z1fIiEvDTsrS_1xE", "void f<int>(decltype(f::x))"),
    // Folds and member access.
    (
        "_Z1fIiEvDTflplfp_E",
        "void f<int>(decltype((... + (fp...))))",
    ),
    (
        "_Z1fIiEvDTfLplfp_fp_E",
        "void f<int>(decltype((fp + ... + (fp...))))",
    ),
    (
        "_Z1fIiEvDTfrplfp_E",
        "void f<int>(decltype(((fp...) + ...)))",
    ),
    (
        "_Z1fIiEvDTfRplfp_fp_E",
        "void f<int>(decltype(((fp...) + ... + fp)))",
    ),
    (
        "_Z1fIiEvDTfLdsfp_fp_E",
        "void f<int>(decltype((fp .* ... .* (fp...))))",
    ),
    ("_Z1fIiEvDTdtfp_1xE", "void f<int>(decltype(fp.x))"),
    ("_Z1fIiEvDTptfp_1xE", "void f<int>(decltype(fp->x))"),
    ("_Z1fIiEvDTdsfp_fp_E", "void f<int>(decltype(fp.*fp))"),
    ("_Z1fIiEvDTstiE", "void f<int>(decltype(sizeof (int)))"),
    ("_Z1fIiEvDTszfp_E", "void f<int>(decltype(sizeof (fp)))"),
    ("_Z1fIiEvDTatiE", "void f<int>(decltype(alignof (int)))"),
    ("_Z1fIiEvDTazfp_E", "void f<int>(decltype(alignof (fp)))"),
    ("_Z1fIiEvDTnxfp_E", "void f<int>(decltype(noexcept (fp)))"),
    (
        "_Z1fIiEvDTqufp_fp_fp_E",
        "void f<int>(decltype(fp ? fp : fp))",
    ),
    ("_Z1fIiEvDTcmfp_fp_E", "void f<int>(decltype(fp, fp))"),
];

/// Inputs the grammar rejects, including every numeric overflow guard.
const REJECTED: &[&str] = &[
    // A `_` after `_block_invoke` must be followed by a number.
    "___Z3foov_block_invoke_",
    // `GI` names a module, which starts with `W`.
    "_ZGI3foo",
    // An `objcproto` qualifier must wrap one complete source name.
    "_Z1fU11objcprotoxxi",
    "_Z1fU12objcprotoabci",
    // A `<seq-id>` of `usize::MAX` overflows when biased by one, and one more
    // base-36 digit overflows the accumulator.
    "_ZN3FooS3W5E11264SGSF_E",
    "_ZN3FooS3W5E11264SGSG_E",
    "_ZGR3foo3W5E11264SGSF_",
    // A closure number of `usize::MAX` overflows when biased by two.
    "_ZZ3foovEUlvE18446744073709551615_",
    // A source-name length of `usize::MAX` overflows past the cursor.
    "_Z18446744073709551615",
    "_Z18446744073709551614x",
    // A nested name cannot hold a `decltype`.
    "_ZN3FooDtfp_E3barEv",
    // A call offset is `h` or `v`.
    "_ZTcx0_3foov",
    // `fx` is no fold, and a fold takes a binary operator.
    "_Z1fIiEvDTfxplfp_E",
    "_Z1fIiEvDTflclfp_E",
    // `DS` saturates only `DA`/`DR` fixed-point types.
    "_Z1fDSDXi",
];

/// Well-formed symbols whose every proper prefix must fail cleanly. Each one
/// ends inside a different list production, so the truncations reach the
/// end-of-input guards of those lists.
const TRUNCATION_SEEDS: &[&str] = &[
    "_Z3fooUa9enable_ifIXLb1EEEv",
    "_ZTcv0_n8_h4_3foov",
    "_Z1fIJiiEEvv",
    "_Z1fIiEvDTcpfp_fp_EE",
    "_Z1fIiEvDTclfp_fp_EE",
    "_Z1fIiEvDTcvT__fp_fp_EE",
    "_Z1fIiEvDTtlT_fp_EE",
    "_Z1fIiEvDTilfp_EE",
    "_Z1fIJiEEvDTsPiEE",
    "_Z1fIiEvDTrQi_Xfp_EE",
    "_Z1fIiEvDTnwfp__ipiEE",
    "_Z1fIiEvDTgssr1A1BE1xE",
    "_Z1fIiEvDTsr1AIiE1BE1xE",
    "_Z1fIiEvDTfLplfp_fp_E",
    "_Z1fIiEvDTu3fooiEE",
    "_Z1fIiEvDTsoT_fp_8_0pEE",
    "_Z1fDSDRj",
    "_Z1fU3fooKFvvE",
    "_ZZ3foovEd0_1a",
    "_ZN1AIiE1fIfEEvDTLUlTyTtTyTnTL1__ETL0_1_T_TL0__E_EE",
];

#[test]
fn rare_productions_print() {
    for &(symbol, expected) in CASES {
        let ast = cpp::parse(symbol).unwrap_or_else(|error| panic!("{symbol}: {error}"));
        assert_eq!(ast.to_string(), expected, "{symbol}");
    }
}

#[test]
fn truncations_fail_cleanly() {
    for &seed in TRUNCATION_SEEDS {
        assert!(cpp::parse(seed).is_ok(), "{seed} should parse");
        for end in 1..seed.len() {
            let prefix = &seed[..end];
            if let Ok(ast) = cpp::parse(prefix) {
                let _ = ast.to_string();
            }
        }
    }
}

#[test]
fn malformed_productions_are_rejected() {
    for &symbol in REJECTED {
        assert!(cpp::parse(symbol).is_err(), "{symbol} should be rejected");
    }
}
