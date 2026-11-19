# demangle

A universal demangler for binary symbol names written in pure Rust.

## Features

- **Scheme detection**: automatically detects the mangling scheme from the symbol name.
- **Multi-language**: C, C++ (Itanium, MSVC), Rust, Swift, and Go (planned).
- **Fast**: delegation to battle-tested pure Rust backends today; hot paths can be rewritten later.
- **Pure Rust**: no C/C++ FFI.
- **`no_std` with `alloc`**: the core crate is `no_std` and uses `alloc`.
  Demangled text is exposed through `core::fmt::Display`; allocation-based
  backends store owned text. `std`-only backends remain opt-in feature flags.
- **Basic CLI**: `demangle <symbol>` for quick one-off demangling.

## Supported Languages

| Language | Convention | Backend | Feature |
|----------|------------|---------|---------|
| C        | —          | identity (no mangling) | always |
| Rust     | rustc v0, legacy | `rustc-demangle` | `rust` |
| C++      | Itanium ABI | `itanium-demangle` | `cpp` |
| C++      | MSVC | `msvc-demangler` | `cpp` |
| Swift    | `$s`, `$S`, `_T0` | in-tree pure-Rust demangler (`src/swift/`) | always |
| Go       | Go symbol naming | — | planned |

Feature matrix:

| Features | `no_std` | needs `alloc` | needs `std` |
|----------|----------|---------------|-------------|
| *(none)* | ✅ | ✅ | ❌ |
| `rust`   | ✅ | ✅ | ❌ |
| `cpp`    | ✅ crate | ✅ | ✅ via `itanium-demangle` |

The Swift demangler is always available: it is pure Rust and needs no `std`.
It uses a growable arena backed by `alloc`; parser depth/work limits and
substitution-clone budgets keep untrusted input bounded.

### Swift coverage

Validated against the official corpus vendored in `refs/swift/manglings.txt`
(Swift `$s`/`$S`/`_T0` prefixes only):

- all **248/248** corpus symbols with recognized prefixes demangle exactly
  (zero near-misses): builtins and integer types, identifiers (word and
  Punycode substitutions), nominal/bound generic types, compound associated
  types, tuples, functions and accessors, standard substitutions and sugar,
  metatypes, generic parameters/signatures/requirements, impl (SIL) function
  types (`Is`/`II` substitutions, conventions, differentiability, coroutines),
  thunks (partial apply, reabstraction, autodiff, key-path, witness, outlined,
  distributed, resume partial), specializations, opaque types, macros
  (freestanding/attached/unique/location), anonymous contexts, packs/variadic
  generics and retroactive/dependent conformances;
- legacy `_Tt`/`_TF`/`_TT`/`_TW` manglings are not detected yet.

`cargo test --test swift_corpus` reports the exact pass count and skips when
`refs/` is absent. An additional 23,975-symbol corpus extracted from the Swift
6.4.2 Linux toolchain is tracked by `cargo test --test
swift_toolchain_corpus` (also skips without `refs/`); all 23,975 currently
demangle exactly.

Default features are `["rust", "cpp"]`. For a `no_std`, no-`std` build use
`default-features = false` (optionally with `features = ["rust"]`).

## Usage

Library:

```rust
use demangle::{Demangler, Language, Scheme};

let demangler = Demangler::new();
let demangled = demangler.demangle("_ZN3Foo3barEv").unwrap();

assert_eq!(demangled.scheme(), Scheme::Cpp);
assert_eq!(demangled.language(), Language::Cxx);
assert_eq!(demangled.to_string(), "Foo::bar"); // allocates only here, in the caller

// Swift is demangled in-tree:
let swift = demangler.demangle("$s4main5helloSSyYaKF").unwrap();
assert_eq!(swift.to_string(), "main.hello() async throws -> Swift.String");
```

`no_std` caller:

```rust
use core::fmt::Write;
use demangle::Demangler;

let demangler = Demangler::new();
let mut out = MyFixedBuffer::new();
let _ = write!(out, "{}", demangler.demangle("_ZN3Foo3barEv")?);
```

CLI:

```sh
# single symbol
demangle "_ZN3Foo3barEv"

# from stdin
cat symbols.txt | demangle -
```

## Installation

Add to your `Cargo.toml`:

```toml
[dependencies]
demangle = "0.1"
# or, for a no_std core:
# demangle = { version = "0.1", default-features = false, features = ["rust"] }
```

## Project Layout

See `AGENTS.md` for agent operating guidelines and the project structure.

## References

`refs/` holds offline reference material (official specs and test corpora) with
pinned provenance. It is not distributed: `Cargo.toml` excludes it from the
published crate. See `refs/README.md` and `refs/swift.md`.

## Changelog

See `CHANGELOG.md` for the version history.
