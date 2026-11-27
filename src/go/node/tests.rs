use alloc::string::ToString;

use super::*;

#[test]
fn abi_suffixes_round_trip() {
    for abi in [Abi::Abi0, Abi::Internal, Abi::Wrap] {
        assert!(abi.suffix().starts_with('.'));
    }
}

#[test]
fn package_decodes() {
    let pkg = Package::new(b"example.com/x%2ey");
    assert_eq!(pkg.raw(), "example.com/x%2ey");
    assert_eq!(pkg.decoded().to_string(), "example.com/x.y");
}

#[test]
fn package_escapes_invalid_bytes() {
    let pkg = Package::new(b"example.com/x%ffy");
    assert_eq!(pkg.raw(), "example.com/x%ffy");
    assert_eq!(pkg.decoded(), "example.com/x\\xffy");
}
