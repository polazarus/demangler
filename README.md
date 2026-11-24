# demangle

A universal demangler for binary symbol names, written entirely in pure Rust.
`no_std` + `alloc`, no runtime dependencies, no C/C++ FFI. Every backend is
in-tree; parsers are bounded and input is raw bytes.

Supports C/C++, Rust, Swift, Go, Objective-C, Kotlin/Native, D, JNI, Ada,
Fortran, Delphi/Object Pascal, OCaml, PE/COFF decorations and Mach-O C names.
See [language and compiler coverage](docs/languages.md) for the full matrix.

## Quick start

```rust
use demangle::{Demangler, Language, Scheme};

let demangled = Demangler::new().demangle("_ZN3Foo3barEv").unwrap();
assert_eq!(demangled.scheme(), Scheme::Cpp);
assert_eq!(demangled.language(), Language::Cxx);
assert_eq!(demangled.to_string(), "Foo::bar()");
```

Raw bytes, forced schemes, `no_std` usage and the CLI are covered in
[usage](docs/usage.md). Feature selection is in [features](docs/features.md).

## Validation

Correctness is enforced by pinned real-world corpora with exact expected
output, zero-wrong oracle comparisons, explicit rejection gates, adversarial
recursive-bomb inputs, and all-features plus allocator-free builds under
`clippy -D warnings`. Corpus tests skip cleanly when `refs/` is absent.
[Read the full validation strategy](docs/validation.md).

## Details

- [Language and compiler coverage](docs/languages.md)
- [Per-backend contracts and corpus counts](docs/coverage.md)
- [Validation strategy](docs/validation.md)
- [Unsupported schemes and encodings](docs/unsupported-schemes.md)
- [Release history](CHANGELOG.md)
