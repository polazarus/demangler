# Features

Optional exhaustive feature combinations can be checked with the third-party
[`cargo-all-features`](https://crates.io/crates/cargo-all-features) tool:

```sh
cargo all-features check
```

This is documented in [`docs/feature-matrix.md`](docs/feature-matrix.md). It is
not a required quality gate because the feature matrix creates a large target
directory; the standard `--all-features`, `--no-default-features`, and
individual-feature checks remain mandatory.

Feature matrix:

| Features | `no_std` | needs `alloc` | needs `std` |
|----------|----------|---------------|-------------|
| *(none)* | ✅ | ❌ | ❌ |
| `rust` | ✅ | ✅ | ❌ |
| `swift` | ✅ | ✅ | ❌ |
| `go` | ✅ | ✅ | ❌ |
| `objc` | ✅ | ✅ | ❌ |
| `kotlin-native` | ✅ | ✅ | ❌ |
| `d` | ✅ | ✅ | ❌ |
| `jni` | ✅ | ✅ | ❌ |
| `ada` | ✅ | ✅ | ❌ |
| `fortran` | ✅ | ❌ | ❌ |
| `delphi` | ✅ | ✅ | ❌ |
| `ocaml` | ✅ | ❌ | ❌ |
| `coff` | ✅ | ❌ | ❌ |
| `encoding` | ✅ | ✅ | ❌ |
| `cpp-itanium` / `cpp-msvc` / `cpp-gnu-v2` | ✅ | ✅ | ❌ |
| `cpp` (all C++ dialects) | ✅ | ✅ | ❌ |
| default (`cpp`, `rust`, `swift`, `go`, `objc`, `kotlin-native`, `d`, `jni`, `ada`, `fortran`, `delphi`, `ocaml`, `coff`, `encoding`) | ✅ | ✅ | ❌ |


## Feature selection

All schemes are in-tree and can be enabled individually; there are no runtime
dependencies. The default feature set enables every backend. The crate is
`no_std` + `alloc` when any backend is enabled.

```toml
# only Rust, no C++/Swift/Go/Objective-C
demangle = { version = "0.1", default-features = false, features = ["rust"] }

# only Objective-C metadata/method parsing
demangle = { version = "0.1", default-features = false, features = ["objc"] }

# only the MSVC C++ parser/printer
demangle = { version = "0.1", default-features = false, features = ["cpp-msvc"] }

# only Itanium C++ and Go
demangle = { version = "0.1", default-features = false, features = ["cpp-itanium", "go"] }

# D and Java JNI, no C++/Swift/Go
demangle = { version = "0.1", default-features = false, features = ["d", "jni"] }

# PE/COFF decorations only (no allocator needed)
demangle = { version = "0.1", default-features = false, features = ["coff"] }
```

