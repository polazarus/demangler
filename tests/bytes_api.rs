//! Public raw-byte API coverage for every migrated backend.
//!
//! These inputs are not valid UTF-8. Each backend pins its own contract for a
//! non-UTF-8 identifier byte:
//!
//! - C++, Rust, Swift, Go, Objective-C, Kotlin/Native and D parse the bytes and
//!   escape the invalid byte as `\xNN` in the rendered text;
//! - Ada and Fortran reject it. Their grammars are ASCII and their detection
//!   prefilters claim the name, so [`Demangler::demangle_bytes`] reports
//!   [`demangle::Error::InvalidSymbol`] for the detected scheme;
//! - JNI, Delphi and OCaml validate the whole symbol during detection, so a
//!   name with an invalid identifier byte is not claimed at all: it stays plain
//!   escaped text, and the forced-scheme entry point rejects it;
//! - a PE/COFF decoration is peeled from raw bytes and keeps its metadata, so
//!   the inner name is escaped like any other pass-through name.
//!
//! [`Demangler::demangle`] and [`Demangler::demangle_bytes`] must also agree on
//! valid UTF-8 input.

use demangle::{Demangler, Scheme};

#[cfg(feature = "cpp-itanium")]
#[test]
fn cpp_parses_invalid_source_name_bytes() {
    let demangled = Demangler::new().demangle_bytes(b"_ZN4F\xffooE").unwrap();
    assert_eq!(demangled.scheme(), Scheme::Cpp);
    assert_eq!(demangled.to_string(), "F\\xffoo");
}

#[cfg(feature = "rust")]
#[test]
fn rust_parses_invalid_identifier_bytes() {
    let demangled = Demangler::new().demangle_bytes(b"_ZN3f\xffo17h1234567890abcdefE").unwrap();
    assert_eq!(demangled.scheme(), Scheme::Rust);
    assert_eq!(demangled.to_string(), "f\\xffo::h1234567890abcdef");
}

#[cfg(feature = "swift")]
#[test]
fn swift_parses_invalid_identifier_bytes() {
    let demangled = Demangler::new()
        .demangle_bytes(b"$s4main3f\xffo")
        .unwrap();
    assert_eq!(demangled.scheme(), Scheme::Swift);
    assert_eq!(demangled.to_string(), "mainf\\xffo");
}

#[cfg(feature = "go")]
#[test]
fn go_parses_invalid_identifier_bytes() {
    let demangled = Demangler::new()
        .demangle_bytes(b"type:.eq.main.F\xffoo")
        .unwrap();
    assert_eq!(demangled.scheme(), Scheme::Go);
    assert_eq!(demangled.to_string(), "type:.eq.main.F\\xffoo");
}

#[cfg(feature = "objc")]
#[test]
fn objc_parses_invalid_identifier_bytes() {
    let demangled = Demangler::new()
        .demangle_bytes(b"_OBJC_CLASS_$_F\xffoo")
        .unwrap();
    assert_eq!(demangled.scheme(), Scheme::ObjC);
    assert_eq!(demangled.to_string(), "Objective-C class F\\xffoo");
}

#[cfg(feature = "kotlin-native")]
#[test]
fn kotlin_parses_invalid_identifier_bytes() {
    let demangled = Demangler::new()
        .demangle_bytes(b"kclass:com.example.F\xffoo")
        .unwrap();
    assert_eq!(demangled.scheme(), Scheme::KotlinNative);
    assert_eq!(demangled.to_string(), "com.example.F\\xffoo");
}

#[cfg(feature = "d")]
#[test]
fn d_parses_invalid_identifier_bytes() {
    let demangled = Demangler::new()
        .demangle_bytes(b"_D8foo\xff\xfebarFZv")
        .unwrap();
    assert_eq!(demangled.scheme(), Scheme::D);
    assert_eq!(demangled.to_string(), "foo\\xff\\xfebar()");
}

#[cfg(feature = "ada")]
#[test]
fn ada_rejects_invalid_identifier_bytes() {
    // The `ada__` prefilter claims the name, and the ASCII identifier grammar
    // then rejects it, so the byte API fails instead of guessing an identifier.
    let symbol = b"ada__foo__b\xffar";
    assert_eq!(Scheme::detect_bytes(symbol), Scheme::Ada);
    let error = Demangler::new()
        .demangle_bytes(symbol)
        .expect_err("a non-ASCII Ada identifier byte must be rejected");
    assert_eq!(error, demangle::Error::InvalidSymbol(Scheme::Ada));
}

#[cfg(feature = "fortran")]
#[test]
fn fortran_rejects_invalid_identifier_bytes() {
    // The `__<module>_MOD_<member>` shape prefilter claims the name, and the
    // ASCII identifier grammar then rejects it.
    let symbol = b"__fprobe_MOD_a\xffd";
    assert_eq!(Scheme::detect_bytes(symbol), Scheme::Fortran);
    let error = Demangler::new()
        .demangle_bytes(symbol)
        .expect_err("a non-ASCII gfortran identifier byte must be rejected");
    assert_eq!(error, demangle::Error::InvalidSymbol(Scheme::Fortran));
}

#[cfg(feature = "jni")]
#[test]
fn jni_does_not_claim_invalid_identifier_bytes() {
    // Detection is a full parse, so an unescaped non-ASCII byte means the name
    // is not JNI at all. It must not be decoded into a guessed Java name.
    let symbol = b"Java_com_example_Foo_b\xffar__I";
    assert_ne!(Scheme::detect_bytes(symbol), Scheme::Jni);
    let demangled = Demangler::new()
        .demangle_bytes(symbol)
        .expect("an unclaimed name is plain text");
    assert_eq!(demangled.scheme(), Scheme::Unknown);
    assert_eq!(demangled.to_string(), "Java_com_example_Foo_b\\xffar__I");
    let error = Demangler::new()
        .demangle_as(Scheme::Jni, symbol)
        .expect_err("the forced JNI backend must reject the byte");
    assert_eq!(error, demangle::Error::InvalidSymbol(Scheme::Jni));
}

#[cfg(feature = "delphi")]
#[test]
fn delphi_does_not_claim_invalid_identifier_bytes() {
    // Free Pascal has no unambiguous prefix, so detection is a full parse and
    // an invalid byte leaves the name unclaimed and escaped.
    let symbol = b"DELPHI_PROBE_$$_UNITPROC$LONGIN\xffT";
    assert_ne!(Scheme::detect_bytes(symbol), Scheme::Delphi);
    let demangled = Demangler::new()
        .demangle_bytes(symbol)
        .expect("an unclaimed name is plain text");
    assert_eq!(demangled.scheme(), Scheme::Unknown);
    assert_eq!(
        demangled.to_string(),
        "DELPHI_PROBE_$$_UNITPROC$LONGIN\\xffT"
    );
    let error = Demangler::new()
        .demangle_as(Scheme::Delphi, symbol)
        .expect_err("the forced Delphi backend must reject the byte");
    assert_eq!(error, demangle::Error::InvalidSymbol(Scheme::Delphi));
}

#[cfg(feature = "ocaml")]
#[test]
fn ocaml_does_not_claim_invalid_identifier_bytes() {
    // OCaml detection is a full parse as well: `caml` alone is not a prefix.
    let symbol = b"camlPro\xffbe";
    assert_ne!(Scheme::detect_bytes(symbol), Scheme::OCaml);
    let demangled = Demangler::new()
        .demangle_bytes(symbol)
        .expect("an unclaimed name is plain text");
    assert_eq!(demangled.scheme(), Scheme::Unknown);
    assert_eq!(demangled.to_string(), "camlPro\\xffbe");
    let error = Demangler::new()
        .demangle_as(Scheme::OCaml, symbol)
        .expect_err("the forced OCaml backend must reject the byte");
    assert_eq!(error, demangle::Error::InvalidSymbol(Scheme::OCaml));
}

#[cfg(feature = "coff")]
#[test]
fn coff_escapes_invalid_name_bytes_and_keeps_decoration() {
    // A PE/COFF decoration is peeled from the raw bytes; the inner name keeps
    // its invalid byte and is escaped, and the decoration metadata survives.
    let symbol = b"_f\xffoo@8";
    let demangled = Demangler::new()
        .demangle_bytes(symbol)
        .expect("a decorated name is decodable from bytes");
    assert_eq!(demangled.scheme(), Scheme::Unknown);
    assert_eq!(demangled.to_string(), "f\\xffoo");
    let decoration = demangled
        .coff_decoration()
        .expect("the stdcall decoration must be retained");
    assert_eq!(decoration.symbol(), b"f\xffoo");
    assert_eq!(
        decoration.kind(),
        demangle::coff::DecorationKind::ConventionSuffix
    );
    assert_eq!(
        decoration.convention(),
        demangle::coff::CallingConvention::Stdcall
    );
    assert_eq!(decoration.stack_bytes(), Some(8));
    assert!(!decoration.import());

    let imported = Demangler::new()
        .demangle_bytes(b"__imp__f\xffoo@8")
        .expect("an imported decorated name is decodable from bytes");
    assert_eq!(imported.to_string(), "f\\xffoo");
    let decoration = imported
        .coff_decoration()
        .expect("the import decoration must be retained");
    assert_eq!(decoration.kind(), demangle::coff::DecorationKind::Import);
    assert!(decoration.import());
}

#[test]
fn text_and_byte_apis_agree_on_valid_utf8() {
    // `Demangler::demangle` is the UTF-8 wrapper around `demangle_bytes`; a
    // backend that pre-renders or pre-validates must not diverge between them.
    let demangler = Demangler::new();
    for symbol in [
        "_ZN3Foo3barEv",
        "_RNvCs1234_4test4main",
        "$s4main3fooyySiF",
        "fmt.Println",
        "_OBJC_CLASS_$_NSString",
        "kclass:com.example.Box",
        "_D3foo3barFZv",
        "Java_com_example_Foo_bar__I",
        "ada__exceptions__triggered_by_abort",
        "__fprobe_MOD_add",
        "DELPHI_PROBE_$$_UNITPROC$LONGINT",
        "camlStdlib__List.map_569",
        "_foo@8",
        "__imp__foo@4",
        "main",
    ] {
        let from_text = demangler
            .demangle(symbol)
            .map(|demangled| (demangled.scheme(), demangled.to_string()));
        let from_bytes = demangler
            .demangle_bytes(symbol.as_bytes())
            .map(|demangled| (demangled.scheme(), demangled.to_string()));
        assert_eq!(from_text, from_bytes, "{symbol}");
    }
}

#[test]
fn unknown_invalid_bytes_are_escaped() {
    let demangled = Demangler::new().demangle_bytes(b"plain\xff").unwrap();
    assert_eq!(demangled.scheme(), Scheme::Unknown);
    assert_eq!(demangled.to_string(), "plain\\xff");
}
