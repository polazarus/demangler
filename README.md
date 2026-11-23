# demangle

A universal demangler for binary symbol names, implemented entirely in pure
Rust. No runtime dependencies, no C/C++ FFI, `no_std` + `alloc`, raw-byte
input, bounded parsers and pinned corpora.

## Supported schemes

| Scheme | Typical forms | Feature |
|---|---|---|
| C | plain identifiers | always |
| C++ | Itanium, MSVC, GNU v2, ARM, EDG, HP aCC | `cpp` |
| Rust | legacy `_ZN`, v0 `_R` | `rust` |
| Swift | `$s`, `$S`, `_$s`, `_$S`, legacy `_T` | `swift` |
| Go | linker names, closures, generics, `type:`/`go:` records | `go` |
| Objective-C | Apple/GNUstep metadata and method display forms | `objc` |
| Kotlin/Native | `kfun:`, `ktype:`, `kclass:` and related symbols | `kotlin-native` |
| D | `_D` ABI names | `d` |
| Java JNI | short and long `Java_` native-method names | `jni` |
| Ada/GNAT | external names, protected/task helpers, elaboration | `ada` |
| Fortran | gfortran modules, externals and generated helpers | `fortran` |
| Delphi/Object Pascal | Free Pascal/Delphi linker names | `delphi` |
| OCaml | `caml...` linker names, labels, runtime helpers | `ocaml` |
| PE/COFF | target-aware `_name@N`, vectorcall, imports, weak aliases, fragments | `coff` |
| Mach-O | explicit C `_name` leading-underscore path | always |
| Encodings | UTF-8, Latin-1, Windows-1252, MacRoman, EBCDIC CP037 | `encoding` |

Detection is conservative for names that overlap with ordinary C identifiers.
Use `demangle_with_scheme` when the language is known.

## Quick start

```rust
use demangle::{Demangler, Language, Scheme};

let demangler = Demangler::new();
let demangled = demangler.demangle("_ZN3Foo3barEv")?;

assert_eq!(demangled.scheme(), Scheme::Cpp);
assert_eq!(demangled.language(), Language::Cxx);
assert_eq!(demangled.to_string(), "Foo::bar()");
```

Raw symbol bytes:

```rust
let demangled = Demangler::new().demangle_bytes(b"_ZN3Foo3barEv")?;
assert_eq!(demangled.to_string(), "Foo::bar()");
```

Explicit scheme:

```rust
use demangle::Scheme;

let demangled = Demangler::new().demangle_with_scheme(Scheme::Ada, "pkg__subp")?;
assert_eq!(demangled.to_string(), "pkg.subp");
```

The `demangle_as` / `demangle_as_str` aliases remain deprecated until 0.2.

## Design

- Every backend is in-tree; external demanglers are test-only oracles.
- Input is `&[u8]`; invalid identifier bytes are escaped as `\xNN`.
- Parsers enforce input, depth, work and output budgets.
- Corpus tests pin exact counts and reject unsupported forms; they skip when
  `refs/` is unavailable.
- `coff` and `--no-default-features` remain allocator-free; language backends
  use `alloc`.

Feature flags and feature-selection examples are in
[`docs/features.md`](docs/features.md). Optional pairwise build checks use
[`cargo-all-features`](docs/feature-matrix.md).

## CLI

```sh
demangle "_ZN3Foo3barEv"
cat symbols.txt | demangle -
```

## Documentation

- [`docs/coverage.md`](docs/coverage.md) — per-backend forms, contracts,
  detection rules and corpus counts
- [`docs/usage.md`](docs/usage.md) — API, raw bytes, forced schemes, `no_std`
  and CLI examples
- [`docs/features.md`](docs/features.md) — feature matrix and selection
- [`docs/unsupported-schemes.md`](docs/unsupported-schemes.md) — remaining
  schemes, dialects and encodings
- [`docs/feature-matrix.md`](docs/feature-matrix.md) — optional
  `cargo-all-features` workflow
- [`CHANGELOG.md`](CHANGELOG.md) — release history

`refs/` contains offline specifications and corpora with pinned provenance; it
is excluded from the published crate.
