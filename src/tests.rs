#[test]
fn version_matches_cargo() {
    let version = env!("CARGO_PKG_VERSION");
    assert_ne!(version, "");
}
