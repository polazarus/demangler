use alloc::string::String;

use super::super::parse;

fn render(symbol: &str) -> String {
    let ast = parse(symbol.as_bytes()).unwrap_or_else(|error| panic!("{symbol}: {error}"));
    let mut out = String::new();
    core::fmt::write(&mut out, format_args!("{ast}"))
        .unwrap_or_else(|_| panic!("{symbol} did not print"));
    out
}

#[test]
fn function_parameters_print_as_fp_and_this() {
    assert_eq!(
        render("_Z1fIiEDTeqfp_LDnEEPT_"),
        "decltype(fp == nullptr) f<int>(int*)"
    );
    assert_eq!(
        render("_Z1fIiEDTeqfp1_LDnEEicPT_"),
        "decltype(fp1 == nullptr) f<int>(int, char, int*)"
    );
    assert_eq!(
        render("_ZN5test31aINS_1XEMS1_PiEEvT_T0_DTdsfL0p_fL0p0_E"),
        "void test3::a<test3::X, int* test3::X::*>(test3::X, int* test3::X::*, \
         decltype(fp.*fp0))"
    );
    // `fpT` is the explicit object parameter of an enclosing function.
    assert_eq!(
        render("_ZN1A1gIiEEDTcldtptfpT1b1fIT_EEEv"),
        "decltype(this->b.f<int>()) A::g<int>()"
    );
}

#[test]
fn unresolved_names_print_qualified_and_global_forms() {
    assert_eq!(
        render("_ZN5test81fIiEEvNS_5int_cIXsrNS_4metaIT_E4typeE5valueEEE"),
        "void test8::f<int>(test8::int_c<test8::meta<int>::type::value>)"
    );
    assert_eq!(
        render("_Z3ft7IiEN11__enable_ifIXsr16__is_scalar_typeIT_EE7__valueEvE6__typeEv"),
        "__enable_if<__is_scalar_type<int>::__value, void>::__type ft7<int>()"
    );
    assert_eq!(
        render("_ZN6test181fINS_1AEEEvNS_1SIXadsrT_plEEE"),
        "void test18::f<test18::A>(test18::S<&test18::A::operator+>)"
    );
    assert_eq!(
        render("_ZN6test191gINS_1AEEEvNS_1SIXadsrT_cviEEE"),
        "void test19::g<test19::A>(test19::S<&test19::A::operator int>)"
    );
    assert_eq!(
        render("_ZN6test191gINS_1AEEEvNS_1SIXadsrT_miIdEEEE"),
        "void test19::g<test19::A>(test19::S<&test19::A::operator-<double>>)"
    );
    assert_eq!(
        render(
            "_ZNK3Ncr6Silver7Utility6detail12CallOnThreadIZ53-[DeploymentSetupController handleManualServerEntry:]E3$_5EclIJEEEDTclclL_ZNS2_4getTIS4_EERT_vEEspclsr3stdE7forwardIT_Efp_EEEDpOSA_"
        ),
        "decltype(-[DeploymentSetupController handleManualServerEntry:]::$_5& \
         Ncr::Silver::Utility::detail::getT<-[DeploymentSetupController \
         handleManualServerEntry:]::$_5>()()()) \
         Ncr::Silver::Utility::detail::CallOnThread<-[DeploymentSetupController \
         handleManualServerEntry:]::$_5>::operator()<>() const"
    );
}

#[test]
fn global_unresolved_names_and_operators() {
    assert_eq!(
        render("_ZN2FnIXgs4BaseEX4BaseEEEvv"),
        "void Fn<::Base, Base>()"
    );
    assert_eq!(
        render("_ZN2FnIXgsnw_iEEXna_ipiLi4EEEEEvv"),
        "void Fn<::new int, new[] int(4)>()"
    );
    assert_eq!(
        render("_ZN2FnIXdlLj4EEXgsdaLj4EEEEvv"),
        "void Fn<delete 4u, ::delete[] 4u>()"
    );
}

#[test]
fn new_expressions_print_placement_and_initializers() {
    assert_eq!(
        render("_ZN2FnIXnwLj4E_iEEXgsnaLj4E_ipiLi4EEEEEvv"),
        "void Fn<new(4u) int, ::new[](4u) int(4)>()"
    );
    assert_eq!(
        render("_ZN5test43tf1INS_1XEEEvDTnw_T_piLi1EEE"),
        "void test4::tf1<test4::X>(decltype(new test4::X(1)))"
    );
    assert_eq!(
        render("_ZN5test43tf2INS_1XEEEvDTnw_T_piilLi1EEEE"),
        "void test4::tf2<test4::X>(decltype(new test4::X({1})))"
    );
    assert_eq!(
        render("_ZN5test31iIiEEvDTnw_Dk1CpicvT__EEE"),
        "void test3::i<int>(decltype(new C auto((int)())))"
    );
}

#[test]
fn fold_expressions_print_their_operator_and_pack() {
    assert_eq!(
        render("_Z6foldl1IJLi1ELi2ELi3EEEv1AIXfLplLi1ET_EE"),
        "void foldl1<1, 2, 3>(A<(1 + ... + (1, 2, 3))>)"
    );
    assert_eq!(
        render("_Z6foldr1IJLi1ELi2ELi3EEEv1AIXfRplT_Li1EEE"),
        "void foldr1<1, 2, 3>(A<((1, 2, 3) + ... + 1)>)"
    );
    assert_eq!(
        render("_Z5foldlIJLi1ELi2ELi3EEEv1AIXflplT_EE"),
        "void foldl<1, 2, 3>(A<(... + (1, 2, 3))>)"
    );
    assert_eq!(
        render("_Z5foldrIJLi1ELi2ELi3EEEv1AIXfrplT_EE"),
        "void foldr<1, 2, 3>(A<((1, 2, 3) + ...)>)"
    );
    assert_eq!(
        render("_Z1fIXfLpm1x1yEEvv"),
        "void f<(x ->* ... ->* (y...))>()"
    );
    assert_eq!(
        render("_Z1fIXfLds1x1yEEvv"),
        "void f<(x .* ... .* (y...))>()"
    );
}

#[test]
fn subobject_and_braced_expressions_print() {
    assert_eq!(
        render("_Z1fIXtl1BadsoiL_Z6nestedE_EEEEvv"),
        "void f<B{&nested.<int at offset 0>}>()"
    );
    assert_eq!(
        render("_Z1fIXtl1BadsoiL_Z6nestedE16_0pEEEEvv"),
        "void f<B{&nested.<int at offset 16>}>()"
    );
    assert_eq!(
        render("_Z1fIXtl1Xdi1adi1bdxLi3ELi1EEEE"),
        "f<X{.a.b[3] = 1}>"
    );
    assert_eq!(
        render("_ZN15designated_init1fINS_1AEEEvDTtlT_di1adi1bdxLi3EdXLi1ELi4ELi9EEE"),
        "void designated_init::f<designated_init::A>(\
         decltype(designated_init::A{.a.b[3][1 ... 4] = 9}))"
    );
}

#[test]
fn vendor_extended_expressions_print_as_calls() {
    assert_eq!(
        render("_Z2f2IiEvDTu11__alignof__T_EE"),
        "void f2<int>(decltype(__alignof__(int)))"
    );
    assert_eq!(
        render("_Z15test_uuidofTypeI10TestStructEvDTu8__uuidoftT_E"),
        "void test_uuidofType<TestStruct>(decltype(__uuidof(TestStruct)))"
    );
    assert_eq!(
        render("_Z15test_uuidofExprI9HasMemberEvDTu8__uuidofXsrT_6memberEEE"),
        "void test_uuidofExpr<HasMember>(decltype(__uuidof(HasMember::member)))"
    );
}

#[test]
fn parenthesized_callee_and_sizeof_pack_list_print() {
    assert_eq!(
        render("_ZN5test11bIsEEDTcp3foocvT__EEES1_"),
        "decltype((foo)((short)())) test1::b<short>(short)"
    );
    assert_eq!(
        render("_Z2f8IiJ8identityIiES0_IfEEEvRAsPiDpT0_T_DpNS3_4typeEE_i"),
        "void f8<int, identity<int>, identity<float>>(int (&) [sizeof... (int, \
         identity<int>, identity<float>, int, identity<int>::type, \
         identity<float>::type)])"
    );
}

#[test]
fn dependent_types_print() {
    assert_eq!(
        render("_Z13JVTLib_103270ILi1EEvPsDv2_xS1_"),
        "void JVTLib_103270<1>(short*, long long vector[2], long long vector[2])"
    );
    assert_eq!(
        render("_Z1fIfEvDUstPT__"),
        "void f<float>(unsigned _BitInt(sizeof (float*)))"
    );
    assert_eq!(
        render("_Z1fIiEvDBstPT__"),
        "void f<int>(_BitInt(sizeof (int*)))"
    );
    assert_eq!(render("_ZN5test31fITnDk1CLi0EEEvv"), "void test3::f<0>()");
    assert_eq!(
        render("_ZN5test31jIiEEvDTnw_DK1CpicvT__EEE"),
        "void test3::j<int>(decltype(new C decltype(auto)((int)())))"
    );
    assert_eq!(
        render("_ZN6test471fINS_1SEEEvPTsNT_1cE"),
        "void test47::f<test47::S>(struct test47::S::c*)"
    );
    assert_eq!(
        render("_ZN6test481fINS_1SEEEvPTuNT_1uE"),
        "void test48::f<test48::S>(union test48::S::u*)"
    );
    assert_eq!(
        render("_ZN6test451fINS_1SEEEvPTeNT_1eE"),
        "void test45::f<test45::S>(enum test45::S::e*)"
    );
}

#[test]
fn vendor_extended_qualifiers_print_after_the_type() {
    assert_eq!(render("_Z2f0PU3AS1c"), "f0(char AS1*)");
    assert_eq!(
        render("_Z1fPU3AS1KiS0_"),
        "f(int const AS1*, int const AS1*)"
    );
    assert_eq!(
        render("_ZTIU4_farrVKPi"),
        "typeinfo for int* const volatile restrict _far"
    );
    assert_eq!(
        render("_Z3fooILi79EEbU7_ExtIntIXT_EEi"),
        "bool foo<79>(int _ExtInt<79>)"
    );
    assert_eq!(render("_Z1fPU11objcproto1A11objc_object"), "f(id<A>)");
    assert_eq!(
        render("_Z1fPKU11objcproto1A7NSArray"),
        "f(NSArray<A> const*)"
    );
}

#[test]
fn function_type_exception_specifications_print() {
    assert_eq!(render("_Z1bPDoFivE"), "b(int (*)() noexcept)");
    assert_eq!(render("_Z1pM1SDoFivE"), "p(int (S::*)() noexcept)");
    assert_eq!(
        render("_Z1fILb1EEvPDOT_EFivE"),
        "void f<true>(int (*)() noexcept(true))"
    );
    assert_eq!(
        render("_Z1gIJfEEvPDwDpT_EFivE"),
        "void g<float>(int (*)() throw(float))"
    );
    assert_eq!(
        render("_Z1pILb1EEiM1SKDOT_EFivRE"),
        "int p<true>(int (S::*)() const & noexcept(true))"
    );
}

#[test]
fn local_entity_literals_print_the_entity() {
    assert_eq!(
        render("_ZN5test12f0ENS_1TILZNS_1xEEEE"),
        "test1::f0(test1::T<test1::x>)"
    );
    assert_eq!(
        render("_ZN5test52f2ENS_2t2ILZ4mainEEE"),
        "test5::f2(test5::t2<main>)"
    );
    assert_eq!(
        render("_ZN5test12f1ENS_2t1ILZNS_2f0EfEEE"),
        "test1::f1(test1::t1<test1::f0(float)>)"
    );
}

#[test]
fn structured_binding_names_print_as_a_list() {
    assert_eq!(render("_ZDC2a12a2E"), "[a1, a2]");
    assert_eq!(render("_ZN2NSDC1x1yEE"), "NS::[x, y]");
    assert_eq!(render("_ZGRDC1x1yE_"), "reference temporary for [x, y]");
    assert_eq!(render("_ZW3FooDC1a1bE"), "[a, b]@Foo");
    assert_eq!(render("_ZN1NW3FooDC1a1bEE"), "N::[a, b]@Foo");
}

#[test]
fn explicit_object_parameters_print_as_this() {
    assert_eq!(render("_ZNH3Foo3fooES_i"), "Foo::foo(this Foo, int)");
    assert_eq!(
        render("_ZNH3Foo3fooERKS_i"),
        "Foo::foo(this Foo const&, int)"
    );
    assert_eq!(
        render("_ZNH1X3fooIRS_EEvOT_i"),
        "void X::foo<X&>(this X&, int)"
    );
    assert_eq!(
        render("_ZNH3FooclERKS_"),
        "Foo::operator()(this Foo const&)"
    );
}

#[test]
fn misc_special_names_and_suffixes_print() {
    assert_eq!(render("_ZGR1bIvE2_"), "reference temporary for b<void>");
    assert_eq!(
        render("_Z1fSsB1XS_"),
        "f(std::string[abi:X], std::string[abi:X])"
    );
    assert_eq!(render("_Z2tfIZUb_E1SEiv"), "int tf<'block-literal'::S>()");
    assert_eq!(render("_ZGIW3Foo"), "initializer for module Foo");
    assert_eq!(render("_ZGIW3FooW3Bar"), "initializer for module Foo.Bar");
    assert_eq!(
        render("_ZGIW3FooWP3BarW3Baz"),
        "initializer for module Foo:Bar.Baz"
    );
    assert_eq!(
        render("___Z10blocksNRVOv_block_invoke"),
        "invocation function for block in blocksNRVO()"
    );
    assert_eq!(
        render("___Z3foo_block_invoke.25"),
        "invocation function for block in foo"
    );
    assert_eq!(
        render("_ZNK10__cxxabiv111__libcxxabi5__sub20first_demangled_nameEPc.eh"),
        "__cxxabiv1::__libcxxabi::__sub::first_demangled_name(char*) const (.eh)"
    );
}

#[test]
fn enable_if_attributes_print_after_the_parameters() {
    assert_eq!(render("_Z1fUa9enable_ifIXLi1EEEv"), "f() [enable_if:1]");
    assert_eq!(
        render("_ZN5test4IdE1fEUa9enable_ifIXeqfL0p_Li1EEXeqfL0p0_Li2EEEi"),
        "test4<double>::f(int) [enable_if:fp == 1, fp0 == 2]"
    );
    assert_eq!(
        render("_Z3quxUa9enable_ifIXLi1EEXL_Z9TRUEFACTSEEEi"),
        "qux(int) [enable_if:1, TRUEFACTS]"
    );
}

#[test]
fn local_name_discriminator_does_not_eat_a_source_name() {
    assert_eq!(render("_ZZ4testvEN1g3fooE5Point"), "test()::g::foo(Point)");
}

#[test]
fn cv_qualified_function_type_takes_one_substitution_slot() {
    assert_eq!(
        render("_Z1fM1XKFivOEMS_FOKivEMS_VFS3_RS2_E"),
        "f(int (X::*)() const &&, int const&& (X::*)(), \
         int const&& (X::*)(int const&) volatile)"
    );
}

#[test]
fn module_substitution_forms_one_entity() {
    assert_eq!(
        render("_ZNW3FOO3TPLIS_3OneE1MEPS1_"),
        "TPL@FOO<One@FOO>::M(One@FOO*)"
    );
    assert_eq!(
        render("_ZNW3FOO3TPLIS_3OneE1NIS_3TwoEEvPS1_PT_"),
        "void TPL@FOO<One@FOO>::N<Two@FOO>(One@FOO*, Two@FOO*)"
    );
}

#[test]
fn constrained_parameter_prints_its_numbering() {
    assert_eq!(
        render("_ZN3FooIiE6methodITk4TrueIT_EiEEvS3_"),
        "void Foo<int>::method<int>(T)"
    );
}

#[test]
fn conversion_operator_keeps_its_template_arguments() {
    assert_eq!(
        render("_ZN8Blizzard6Memory12voidp_returncvPT_IcEEv"),
        "Blizzard::Memory::voidp_return::operator char*<char>()"
    );
    assert_eq!(render("_Zcv1BIRT_EIS1_E"), "operator B<><>");
}

#[test]
fn inherited_constructor_uses_the_inherited_template_arguments() {
    assert_eq!(render("_ZN1DCI21CIiEET_"), "D::D(int)");
}

#[test]
fn argument_pack_and_expansion_are_distinguished_by_text() {
    assert_eq!(render("_Z1fIJR1SS0_EEiDpOT_"), "int f<S&, S>(S&, S&&)");
}

#[test]
fn requires_clauses_print_after_the_parameters() {
    assert_eq!(
        render("_ZN5test21AIiEF1fEzQ4TrueIT_E"),
        "test2::A<int>::friend f(...) requires True<T>"
    );
    assert_eq!(
        render("_ZN5test21AIiEF1gIvEEvzQaa4TrueIT_E4TrueITL0__E"),
        "void test2::A<int>::friend g<void>(...) requires True<T> && True<TL0_>"
    );
    // A template-head clause is part of the parameter list, not the name.
    assert_eq!(
        render("_ZN5test51gITtTyTnTL0__Q1CIS1_EENS_1XEEEvv"),
        "void test5::g<test5::X>()"
    );
    // A lambda signature prints its own clause.
    assert_eq!(
        render("_ZZN5test71fIiEEvvENKUlTyQaa1CIT_E1CITL0__ET0_E_clIiiEEDaS3_Q1CIDtfp_EE"),
        "auto void test7::f<int>()::'lambda'<typename $T> requires C<T> && C<TL0_> \
         (auto)::operator()<int, int>(auto) const requires C<decltype(fp)>"
    );
}

#[test]
fn requires_expressions_print_their_requirements() {
    assert_eq!(
        render("_Z1gIiEviQrQT__XplfL0p_fp_E"),
        "void g<int>(int) requires requires (T) { fp + fp; }"
    );
    assert_eq!(
        render(
            "_Z1fIiEviQrqXcvT__EXfp_Xeqfp_cvS0__EXplcvS0__ELi1ER5SmallXmicvS0__ELi1E\
             NXmlcvS0__ELi2ENR11SmallerThanILi1234EETS0_T1XIS0_ETNS3_4typeETS2_IiEQ11SmallerThanIS0_Li256EEE"
        ),
        "void f<int>(int) requires requires { (T)(); fp; fp == (T)(); \
         {(T)() + 1} -> Small; {(T)() - 1} noexcept; \
         {(T)() * 2} noexcept -> SmallerThan<1234>; typename T; typename X<T>; \
         typename X<T>::type; typename X<int>; requires SmallerThan<T, 256>; }"
    );
}

#[test]
fn pack_subscripts_and_member_casts_print() {
    assert_eq!(
        render("_Z3fooILi0ETpTnDaJLi1ELi2EEEDTsyT0_T_Ev"),
        "decltype((1, 2...)[0]) foo<0, 1, 2>()"
    );
    assert_eq!(
        render("_Z1gILi0EJciEEDyT0_T_v"),
        "(char, int)[0] g<0, char, int>()"
    );
    assert_eq!(
        render("_Z1fIXtl1DmcM7DerivedKiadL_ZN11MoreDerived1zEEn8EEEEvv"),
        "void f<D{(int const Derived::*)(&MoreDerived::z)}>()"
    );
}

#[test]
fn module_qualifier_chains_print_after_the_name() {
    assert_eq!(
        render("_ZN4Quux4TotoW3FooW3Bar3BazEPNS0_S2_5PlughE"),
        "Quux::Toto::Baz@Foo.Bar(Quux::Toto::Plugh@Foo.Bar*)"
    );
    assert_eq!(
        render("_ZN3BobW3FOOW3BAR3BarEPS1_1APNS_S1_1BE"),
        "Bob::Bar@FOO.BAR(A@FOO.BAR*, Bob::B@FOO.BAR*)"
    );
    assert_eq!(
        render("_ZW3FOOW3BAZ3BarPS_W3BAR1APN3BobS1_1BE"),
        "Bar@FOO.BAZ(A@FOO.BAR*, Bob::B@FOO.BAR*)"
    );
    assert_eq!(
        render("_ZN3NMSW3FOO3TPLINS_S0_3OneEE1NINS_S0_3TwoEEEvPS2_PT_"),
        "void NMS::TPL@FOO<NMS::One@FOO>::N<NMS::Two@FOO>(NMS::One@FOO*, NMS::Two@FOO*)"
    );
}

#[test]
fn allocation_token_decorations_print_last() {
    assert_eq!(
        render("__alloc_token__Znwm"),
        "operator new(unsigned long) (.alloc_token)"
    );
    assert_eq!(render("__alloc_token_123__Z3foov"), "foo() (.alloc_token)");
    assert_eq!(
        render("__alloc_token__Znwm.llvm.1234"),
        "operator new(unsigned long) (.llvm.1234) (.alloc_token)"
    );
}

#[test]
fn lambda_template_parameter_levels_resolve() {
    assert_eq!(
        render("_ZN1C1fIiEEvDTtlNS_UlT_TL0__E_EEE"),
        "void C::f<int>(decltype(C::'lambda'(int, auto){}))"
    );
    assert_eq!(
        render("_ZNK1xMUlTyTtTyTnT_TpTnPA3_TL0__ETpTyvE_clIi1XJfEEEDav"),
        "auto x::'lambda'<typename $T, template<typename $T0, $T $N, \
         $T0 (*...$N0) [3]> typename $TT, typename ...$T1>()::operator()<int, X, float>() const"
    );
    assert_eq!(
        render("_ZN1AIiE1fIfEEvDTLUlTyTtTyTnTL1__ETL0_1_T_TL0__E_EE"),
        "void A<int>::f<float>(decltype([]<typename $T, \
         template<typename $T0, $T0 $N> typename $TT>(auto, float, $T){...}))"
    );
}

#[test]
fn increments_print_prefix_or_postfix() {
    // `pp_`/`mm_` are the prefix forms; plain `pp`/`mm` are postfix.
    assert_eq!(
        render("_Z3fooIiEDTpp_fp_ES0_"),
        "decltype(++fp) foo<int>(decltype(++fp))"
    );
    assert_eq!(
        render("_Z3fooIiEDTmm_fp_ES0_"),
        "decltype(--fp) foo<int>(decltype(--fp))"
    );
    assert_eq!(
        render("_Z3fooIiEDTmmfp_ES0_"),
        "decltype(fp--) foo<int>(decltype(fp--))"
    );
    // A postfix operator binds tighter than a unary operand; a prefix one
    // parenthesizes a unary operand.
    assert_eq!(
        render("_Z3fooIiEDTppngfp_ES0_"),
        "decltype((-fp)++) foo<int>(decltype((-fp)++))"
    );
    assert_eq!(
        render("_Z3fooIiEDTngppfp_ES0_"),
        "decltype(-fp++) foo<int>(decltype(-fp++))"
    );
    assert_eq!(
        render("_Z3fooIiEDTpp_ngfp_ES0_"),
        "decltype(++(-fp)) foo<int>(decltype(++(-fp)))"
    );
}

#[test]
fn throw_takes_an_expression_operand() {
    assert_eq!(
        render("_Z1fIiEvDTtwfp_E"),
        "void f<int>(decltype(throw fp))"
    );
    assert_eq!(
        render("_Z1fIiEvDTtwLi1EE"),
        "void f<int>(decltype(throw 1))"
    );
    assert_eq!(render("_Z1fIiEvDTtrE"), "void f<int>(decltype(throw))");
    // `tw` is not followed by a type.
    assert!(parse(b"_Z1fIiEvDTtwiE").is_err());
}

#[test]
fn array_subscripts_print() {
    assert_eq!(
        render("_Z1fIiEvDTixfp_fp_E"),
        "void f<int>(decltype(fp[fp]))"
    );
    // The base binds as a postfix operand; the index needs no parentheses.
    assert_eq!(
        render("_Z1fIiEvDTixngfp_cmfp_fp_E"),
        "void f<int>(decltype((-fp)[fp, fp]))"
    );
    assert_eq!(
        render("_Z1fIiEvDTngixfp_fp_E"),
        "void f<int>(decltype(-fp[fp]))"
    );
}
