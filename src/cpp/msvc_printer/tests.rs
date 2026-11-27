use super::*;
use crate::cpp;
use crate::cpp::node::Node;

fn render(symbol: &str) -> Result<String, PrintError> {
    let ast = cpp::parse(symbol.as_bytes()).expect("symbol should parse");
    let mut out = String::new();
    print_msvc(&ast, &mut out)?;
    Ok(out)
}

fn rendered(symbol: &str) -> String {
    render(symbol).unwrap_or_else(|error| panic!("{symbol}: {error}"))
}

/// An encoding whose shape is a kind the printer has no rendering for,
/// with the shape's span starting at `position`.
fn unmodelled_ast(position: u32) -> Ast<'static> {
    let shape = Node::new(Kind::Lambda, None, Some(position..position + 1), Vec::new());
    Ast::from_root_msvc(Node::new(
        Kind::Encoding,
        None,
        Some(0..position + 1),
        alloc::vec![shape],
    ))
}

#[test]
fn prints_global_functions() {
    assert_eq!(rendered("?foo@@YAXXZ"), "void __cdecl foo(void)");
    assert_eq!(rendered("?x@@YAXMH@Z"), "void __cdecl x(float, int)");
    assert_eq!(rendered("?x@@YAXZZ"), "void __cdecl x(...)");
    assert_eq!(
        rendered("?foo_pbd@@YAXPEBD@Z"),
        "void __cdecl foo_pbd(char const *)"
    );
    assert_eq!(
        rendered("?foo_qad@@YAXQEAD@Z"),
        "void __cdecl foo_qad(char *const)"
    );
}

#[test]
fn prints_member_functions() {
    assert_eq!(
        rendered("?method@Struct@@QEAAXXZ"),
        "public: void __cdecl Struct::method(void)"
    );
    assert_eq!(
        rendered("??0klass@@QEAA@XZ"),
        "public: __cdecl klass::klass(void)"
    );
    assert_eq!(
        rendered("??1klass@@QEAA@XZ"),
        "public: __cdecl klass::~klass(void)"
    );
    assert_eq!(
        rendered("??4klass@@QEAAAEBV0@AEBV0@@Z"),
        "public: class klass const & __cdecl klass::operator=(class klass const &)"
    );
    assert_eq!(
        rendered("?fn@?$klass@H@ns@@QEBAIXZ"),
        "public: unsigned int __cdecl ns::klass<int>::fn(void) const"
    );
}

#[test]
fn prints_variables_and_types() {
    assert_eq!(rendered("?x@@3HA"), "int x");
    assert_eq!(rendered("?x@@3PEAPEAHEA"), "int **x");
    assert_eq!(rendered("?foo@@3Y123KA"), "unsigned long foo[3][4]");
    assert_eq!(rendered("?x@@3PEAY02HEA"), "int (*x)[3]");
    assert_eq!(
        rendered("?x@@3P6AHMNH@ZEA"),
        "int (__cdecl *x)(float, double, int)"
    );
    assert_eq!(rendered("?x@@3PEAUty@@EA"), "struct ty *x");
    assert_eq!(rendered("?x@@3PEAW4ty@@EA"), "enum ty *x");
    assert_eq!(
        rendered("?l@@3P8foo@@AEHH@ZQ1@"),
        "int (__thiscall foo::*l)(int)"
    );
}

#[test]
fn prints_special_names() {
    assert_eq!(rendered("??_7Base@@6B@"), "const Base::`vftable'");
    assert_eq!(
        rendered("??_7A@@6BB@@C@@D@@@"),
        "const A::`vftable'{for `B's `C's `D'}"
    );
    assert_eq!(rendered("??_SBase@@6B@"), "const Base::`local vftable'");
    assert_eq!(rendered("??_8Middle2@@7B@"), "const Middle2::`vbtable'");
    assert_eq!(
        rendered("??_R0?AUBase@@@8"),
        "struct Base `RTTI Type Descriptor'"
    );
    assert_eq!(
        rendered(".?AUBase@@"),
        "struct Base `RTTI Type Descriptor Name'"
    );
    assert_eq!(
        rendered("??_R1A@?0A@EA@Base@@8"),
        "Base::`RTTI Base Class Descriptor at (0, -1, 0, 64)'"
    );
    assert_eq!(rendered("??_R2Base@@8"), "Base::`RTTI Base Class Array'");
    assert_eq!(
        rendered("??_R3Base@@8"),
        "Base::`RTTI Class Hierarchy Descriptor'"
    );
    assert_eq!(
        rendered("??_R4Base@@6B@"),
        "const Base::`RTTI Complete Object Locator'"
    );
    assert_eq!(rendered("??_C@_02PCEFGMJL@hi?$AA@"), "`string'");
    // A narrow literal's length is bounded by the work budget only.
    let long = alloc::format!("??_C@_0BA@ABCDEFGH@{}?$AA@", "a".repeat(200));
    assert_eq!(rendered(&long), "`string'");
    assert_eq!(
        rendered("??@a6a285da2eea70dba6b578022be61d81@"),
        "??@a6a285da2eea70dba6b578022be61d81@"
    );
    assert_eq!(
        rendered("??_9Base@@$B7AA"),
        "[thunk]: __cdecl Base::`vcall'{8, {flat}}"
    );
    assert_eq!(
        rendered("??_B?1??getS@@YAAAUS@@XZ@51"),
        "`struct S & __cdecl getS(void)'::`2'::`local static guard'{2}"
    );
    assert_eq!(
        rendered("??__EFoo@@YAXXZ"),
        "void __cdecl `dynamic initializer for 'Foo''(void)"
    );
}

#[test]
fn prints_thunks() {
    assert_eq!(
        rendered("?f@C@@WBA@EAAHXZ"),
        "[thunk]: public: virtual int __cdecl C::f`adjustor{16}'(void)"
    );
}

#[test]
fn prints_templates() {
    assert_eq!(
        rendered("?instance$initializer$@@3P6AXXZEA"),
        "void (__cdecl *instance$initializer$)(void)"
    );
    assert_eq!(rendered("?x@@3PEAV?$tmpl@H@@EA"), "class tmpl<int> *x");
    assert_eq!(
        rendered("??0?$IntTemplate@$0BAA@@@QEAA@XZ"),
        "public: __cdecl IntTemplate<256>::IntTemplate<256>(void)"
    );
    assert_eq!(
        rendered("?recref@@YAXU?$type1@$E?inst@@3Urecord@@B@@@Z"),
        "void __cdecl recref(struct type1<struct record const inst>)"
    );
}

#[test]
fn prints_llvm_template_and_type_spelling() {
    assert_eq!(
        rendered("?foo_abbb@@YAXV?$A@V?$B@D@@V1@V1@@@@Z"),
        "void __cdecl foo_abbb(class A<class B<char>, class B<char>, class B<char>>)"
    );
    assert_eq!(
        rendered("?callback_void@@3V?$C@$$A6AXXZ@@A"),
        "class C<void __cdecl(void)> callback_void"
    );
    assert_eq!(
        rendered("??0?$Class@$$BY0A@H@@QEAA@XZ"),
        "public: __cdecl Class<int[]>::Class<int[]>(void)"
    );
    assert_eq!(
        rendered("?f@@YAXW4<unnamed-enum-enumerator>@@@Z"),
        "void __cdecl f(enum <unnamed-enum-enumerator>)"
    );
    assert_eq!(
        rendered("?local@?1??extern_c_func@@9@4HA"),
        "int `extern \"C\" extern_c_func'::`2'::local"
    );
    assert_eq!(
        rendered("??$?BH@TemplateOps@@QAEHXZ"),
        "public: int __thiscall TemplateOps::operator<int> int(void)"
    );
}

#[test]
fn prints_pointer_to_member_values() {
    // `$F`/`$G` keep the decoded offsets only; `$H`/`$I`/`$J` keep the
    // referenced symbol and then its offsets.
    assert_eq!(
        rendered("??$ReadField@UV@@$F7A@@@YAHAAUV@@@Z"),
        "int __cdecl ReadField<struct V, {8, 0}>(struct V &)"
    );
    assert_eq!(
        rendered("?o2@@3U?$K@UO@@$FA@?0@@A"),
        "struct K<struct O, {0, -1}> o2"
    );
    assert_eq!(
        rendered("??$CallMethod@UM@@$H?f@1@QAEXXZA@@@YAXAAUM@@@Z"),
        "void __cdecl CallMethod<struct M, {public: void __thiscall M::f(void), 0}>(struct M &)"
    );
}

#[test]
fn prints_dynamic_initializer_signatures() {
    // A static data member keeps the encoded signature as the second child.
    assert_eq!(
        rendered("??__E?i@C@@0HA@@YAXXZ"),
        "void __cdecl `dynamic initializer for `private: static int C::i''(void)"
    );
    assert_eq!(
        rendered("??__EFoo@@YAXXZ"),
        "void __cdecl `dynamic initializer for 'Foo''(void)"
    );
}

#[test]
fn reports_unmodelled_nodes() {
    // A kind the printer has no rendering for is reported, never guessed.
    let ast = unmodelled_ast(3);
    let mut out = String::new();
    let error = print_msvc(&ast, &mut out).unwrap_err();
    assert_eq!(error.kind(), PrintErrorKind::Unsupported);
    assert_eq!(error.position(), 3);
    assert_eq!(out, "");
}

#[test]
fn prints_deep_scope_chains_without_recursing() {
    let mut symbol = String::from("?a");
    for _ in 0..600 {
        symbol.push_str("@a");
    }
    symbol.push_str("@@3HA");
    let text = rendered(&symbol);
    assert!(text.starts_with("int a::a::a::a"));
    assert_eq!(text.matches("::").count(), 600);
}

#[test]
fn error_reports_the_node_offset() {
    let ast = unmodelled_ast(17);
    let mut out = String::new();
    let error = print_msvc(&ast, &mut out).unwrap_err();
    assert_eq!(error.position(), 17);
    assert_eq!(error.kind(), PrintErrorKind::Unsupported);
    assert!(alloc::format!("{error}").contains("at byte 17"));
}
