//! Public raw-byte API coverage for every migrated backend.
//!
//! These inputs are not valid UTF-8; each backend must parse the bytes and
//! escape the invalid identifier byte in its rendered text.

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

#[test]
fn unknown_invalid_bytes_are_escaped() {
    let demangled = Demangler::new().demangle_bytes(b"plain\xff").unwrap();
    assert_eq!(demangled.scheme(), Scheme::Unknown);
    assert_eq!(demangled.to_string(), "plain\\xff");
}
