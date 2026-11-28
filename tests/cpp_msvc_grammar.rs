//! MSVC C++ productions that the vendored LLVM test corpus does not exercise.
//!
//! The table pins the rarer function classes, thunk adjustors, special names,
//! member-pointer template arguments and pointer qualifiers, and the numeric
//! guards, so they stay tested in the published crate too.
#![cfg(feature = "cpp-msvc")]

use demangler::cpp;

/// `(mangled, expected)` pairs printed in the LLVM `undname` style.
const CASES: &[(&str, &str)] = &[
    // Function classes, near and far.
    ("?f@C@@AAEXXZ", "private: void __thiscall C::f(void)"),
    ("?f@C@@BAEXXZ", "private: void __thiscall C::f(void)"),
    ("?f@C@@DAXXZ", "private: static void __cdecl C::f(void)"),
    ("?f@C@@LAXXZ", "protected: static void __cdecl C::f(void)"),
    ("?f@C@@RAEXXZ", "public: void __thiscall C::f(void)"),
    ("?f@C@@TAXXZ", "public: static void __cdecl C::f(void)"),
    // Adjustor thunks.
    (
        "?f@C@@GBA@AEXXZ",
        "[thunk]: private: void __thiscall C::f`adjustor{16}'(void)",
    ),
    (
        "?f@C@@HBA@AEXXZ",
        "[thunk]: private: void __thiscall C::f`adjustor{16}'(void)",
    ),
    (
        "?f@C@@OBA@AEXXZ",
        "[thunk]: protected: virtual void __thiscall C::f`adjustor{16}'(void)",
    ),
    (
        "?f@C@@WBA@AEXXZ",
        "[thunk]: public: virtual void __thiscall C::f`adjustor{16}'(void)",
    ),
    // `vtordisp` and `vtordispex` thunks, one per access code.
    (
        "?f@C@@$0PPPPPPPM@A@AEXXZ",
        "[thunk]: private: virtual void __thiscall C::f`vtordisp{-4, 0}'(void)",
    ),
    (
        "?f@C@@$1PPPPPPPM@A@AEXXZ",
        "[thunk]: private: virtual void __thiscall C::f`vtordisp{-4, 0}'(void)",
    ),
    (
        "?f@C@@$2PPPPPPPM@A@AEXXZ",
        "[thunk]: protected: virtual void __thiscall C::f`vtordisp{-4, 0}'(void)",
    ),
    (
        "?f@C@@$3PPPPPPPM@A@AEXXZ",
        "[thunk]: protected: virtual void __thiscall C::f`vtordisp{-4, 0}'(void)",
    ),
    (
        "?f@C@@$5PPPPPPPM@A@AEXXZ",
        "[thunk]: public: virtual void __thiscall C::f`vtordisp{-4, 0}'(void)",
    ),
    (
        "?f@C@@$R0A@B@C@D@AEXXZ",
        "[thunk]: private: virtual void __thiscall C::f`vtordispex{0, 1, 2, 3}'(void)",
    ),
    // Calling conventions and builtins.
    ("?f@@YMXXZ", "void __clrcall f(void)"),
    ("?f@@YNXXZ", "void __clrcall f(void)"),
    ("?f@@YAX_L@Z", "void __cdecl f(__int128)"),
    // Special member names.
    (
        "??_XC@@QAEPAXI@Z",
        "public: void * __thiscall C::`placement delete closure'(unsigned int)",
    ),
    (
        "??_YC@@QAEPAXI@Z",
        "public: void * __thiscall C::`placement delete[] closure'(unsigned int)",
    ),
    (
        "??__AC@@QAEXXZ",
        "public: void __thiscall C::`managed vector ctor iterator'(void)",
    ),
    (
        "??__BC@@QAEXXZ",
        "public: void __thiscall C::`managed vector dtor iterator'(void)",
    ),
    (
        "??__CC@@QAEXXZ",
        "public: void __thiscall C::`eh vector copy ctor iterator'(void)",
    ),
    (
        "??__DC@@QAEXXZ",
        "public: void __thiscall C::`eh vector vbase copy ctor iterator'(void)",
    ),
    (
        "??__GC@@QAEXXZ",
        "public: void __thiscall C::`vector copy ctor iterator'(void)",
    ),
    (
        "??__HC@@QAEXXZ",
        "public: void __thiscall C::`vector vbase copy ctor iterator'(void)",
    ),
    (
        "??__IC@@QAEXXZ",
        "public: void __thiscall C::`managed vector vbase copy ctor iterator'(void)",
    ),
    // Variable storage classes.
    ("?x@@0HA", "private: static int x"),
    ("?x@@1HA", "protected: static int x"),
    ("?x@@2HA", "public: static int x"),
    ("?x@@4HA", "int x"),
    // Pointer qualifiers, including `__ptrauth` after another qualifier.
    (
        "?foo@@YAXPEAPEI__ptrauth20OK@AH@Z",
        "void __cdecl foo(int *__restrict __ptrauth(3, 1, 234) *)",
    ),
    (
        "?foo@@YAXPEAPEF__ptrauth20OK@AH@Z",
        "void __cdecl foo(int *__unaligned __ptrauth(3, 1, 234) *)",
    ),
    ("?f@@YAX$$QAH@Z", "void __cdecl f(int &&)"),
    ("?f@@YAXAIAH@Z", "void __cdecl f(int &__restrict)"),
    ("?f@@YAXUA@@@Z", "void __cdecl f(struct A)"),
    (
        "?f@@YAXP8A@@EAEXXZ@Z",
        "void __cdecl f(void (__thiscall A::*)(void))",
    ),
    // Member-pointer and member-data-pointer template arguments.
    ("??$f@$1?x@@3HA@@YAXXZ", "void __cdecl f<&int x>(void)"),
    ("??$f@$H?x@@3HA0@@YAXXZ", "void __cdecl f<{int x, 1}>(void)"),
    (
        "??$f@$I?x@@3HA0A@@@YAXXZ",
        "void __cdecl f<{int x, 1, 0}>(void)",
    ),
    (
        "??$f@$J?x@@3HA0A@A@@@YAXXZ",
        "void __cdecl f<{int x, 1, 0, 0}>(void)",
    ),
    ("??$f@$F0A@@@YAXXZ", "void __cdecl f<{1, 0}>(void)"),
    ("??$f@$G0A@B@@@YAXXZ", "void __cdecl f<{1, 0, 1}>(void)"),
];

/// Inputs the grammar rejects, including the numeric guards.
const REJECTED: &[&str] = &[
    // Unknown `??__` operator, and one cut short.
    "??__ZC@@QAEXXZ",
    "??__",
    // `$6` is no vtordisp access code.
    "?f@C@@$6PPPPPPPM@A@AEXXZ",
    // `5` is no variable storage class.
    "?x@@5HA",
    // A union tag needs a name; `W` must be `W4`.
    "?f@@YAXT@Z",
    "?f@@YAXW5A@@Z",
    // A member function pointer cannot take a far class code here.
    "?f@@YAXP8A@@BEAEXXZ@Z",
    // `$1` names a symbol.
    "??0AvgPool2dImpl@nn@torch@@QEAA@V?$ExpandingArray@$1_J@2@@Z",
    // `$K` is no member-pointer template argument.
    "??$f@$K0@@YAXXZ",
    // A hex number one digit past 64 bits overflows.
    "??$f@$0PPPPPPPPPPPPPPPPP@@@YAXXZ",
    // A thread-safe static guard number must be present and fit 64 bits.
    "?$TSS@?1??f@@YAXXZ@4HA",
    "?$TSS99999999999999999999@?1??f@@YAXXZ@4HA",
    // A string literal with no CRC, and a wide one whose length disagrees.
    "??_C@_1A@ABC@@",
    "??_C@_13ABC@?$AAa@",
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

#[test]
fn thread_safe_static_guards_parse() {
    for symbol in ["?$TSS0@?1??f@@YAXXZ@4HA", "?$TSS0@?1??f@A@@YAXXZ@4HA"] {
        let ast = cpp::parse(symbol.as_bytes(), demangler::Budget::default())
            .unwrap_or_else(|error| panic!("{symbol}: {error}"));
        assert!(
            ast.to_string().starts_with("TSS0 `void __cdecl "),
            "{symbol}"
        );
    }
}
