# demangle

A universal demangler for binary symbol names written in pure Rust.

## Features

- **Scheme detection**: automatically detects the mangling scheme from the symbol name.
- **Multi-language**: C, C++ (Itanium, MSVC), Rust, Swift, and Go (planned).
- **Fast**: delegation to battle-tested pure Rust backends today; hot paths can be rewritten later.
- **Pure Rust**: no C/C++ FFI, minimal unsafe, `#![no_std]`-ready with feature-gated dependencies.
- **Basic CLI**: `demangle <symbol>` for quick one-off demangling.

## Supported Languages

| Language | Convention | Backend |
|----------|------------|---------|
| C        | —          | identity (no mangling) |
| C++      | Itanium ABI, MSVC | `itanium-demangle`, `msvc-demangler` (feature `cpp`) |
| Rust     | rustc v0, legacy | `rustc-demangle` (feature `rust`) |
| Swift    | Swift mangled (`$s`) | custom pure-Rust submodule (planned) |
| Go       | Go symbol naming | planned (no pure-Rust backend exists) |

## Usage

Library:

```rust
use demangle::{Demangler, Scheme, Language};

let demangler = Demangler::new();
let result = demangler.demangle("_ZN3Foo3barEv");

assert_eq!(result.scheme, Scheme::Cpp);
assert_eq!(result.language(), Language::Cxx);
assert_eq!(result.name, "Foo::bar");
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
```

## Project Layout

See `AGENTS.md` for agent operating guidelines and the project structure.

## Changelog

See `CHANGELOG.md` for the version history.