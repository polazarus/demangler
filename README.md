# demangle

A universal demangler for binary symbol names written in pure Rust.

## Features

- **Scheme detection**: automatically detects the mangling scheme from the symbol name.
- **Multi-language**: C, C++ (Itanium, MSVC), Rust, Swift, and Go.
- **Fast**: delegation to battle-tested pure Rust backends today; hot paths can be rewritten later.
- **Pure Rust**: no C/C++ FFI.
- **`no_std` with `alloc`**: the core crate is `no_std` and uses `alloc`.
  Demangled text is exposed through `core::fmt::Display`; parsed trees and
  owned text use `alloc`.
- **Basic CLI**: `demangle <symbol>` for quick one-off demangling.

## Supported Languages

| Language | Convention | Backend | Feature |
|----------|------------|---------|---------|
| C        | —          | identity (no mangling) | always |
| Rust     | rustc v0, legacy | in-tree pure-Rust demangler (`src/rust/`) | always |
| C++      | Itanium ABI | in-tree pure-Rust demangler (`src/cpp/`) | always |
| C++      | MSVC | in-tree pure-Rust demangler (`src/cpp/`) | always |
| Swift    | `$s`, `$S`, `_T0` | in-tree pure-Rust demangler (`src/swift/`) | always |
| Go       | Go linker names | in-tree pure-Rust demangler (`src/go/`) | always |

Feature matrix:

| Features | `no_std` | needs `alloc` | needs `std` |
|----------|----------|---------------|-------------|
| *(none)* | ✅ | ✅ | ❌ |

The Swift demangler is always available: it is pure Rust and needs no `std`.
It builds an `Rc`-owned node tree; contiguous identifiers borrow the input,
while reconstructed punycode/word-substituted text is owned by `Cow`. Parser depth/work
limits and substitution-clone budgets keep untrusted input bounded. Parsed
Swift trees are retained by `Demangled` and printed lazily. `swift::parse`
also exposes the retained tree as `swift::Ast` with read-only `swift::Node`
views; the internal `RefCell` borrows never escape.

### Go coverage

The in-tree Go backend classifies the names the Go linker emits and prints them
as a Go programmer writes them:

- package, function and method names (`fmt.Println`, `fmt.(*pp).doPrintln`);
- closures and wrappers (`.funcN`, `-rangeN`, `.gowrapN`, `.deferwrapN`, `-fm`,
  `·f`, nested closure indices, inlined function names);
- generic instantiations and compiler shape types (`Sum[go.shape.int]`,
  `go.shape.struct { … }`, `..dict.T[int]`);
- runtime type descriptors (`type:`, `type:*`, `type:.eq.`, `type:.hash.`,
  `type:.gcprog.`, `type:.namedata.`, `type:.importpath.`, `type:noalg.`);
- linker specials (`go:itab.<T>,<I>`, `go:string.<payload>`, `go:info.<T>`,
  `go:buildid`, `go:func.*`, …);
- compiler-generated data symbols (`..inittask`, `..interfaceSwitch.N`,
  `..typeAssert.N`, `..gobytes.N`, `glob..funcN`, `gcbits.<hex>`,
  `gclocals.<base64>`);
- compiler constants (`$f32.`, `$f64.`, `$i32.`, `$i64.`, `$i128.`).

`go::parse` returns an inspectable `go::Ast`. Printing decodes the `%XX`
package-path escapes that `cmd/internal/objabi.PathToPrefix` introduces,
normalises the `·` separator to `.`, and strips ABI markers (`.abi0`,
`.abiinternal`, `.abiwrap`), which `go::Ast::abi` reports separately. Unescaping
is for humans: `%2e` inside the last package path segment becomes `.`, which is
ambiguous with the package/local separator, so printed text is not guaranteed
to re-parse to the same structure. `reflect.name` blobs (`type:.namedata.`) and
`go:string.` payloads are opaque and printed verbatim.

Parsing is bounded by input length (64 KiB), nesting depth, work units and
suffix count; the printed output is never longer than the input.

`cargo test --test go_corpus` validates 3,326 Go names from the ELF symbol table
of a generated test binary (stdlib, methods, closures, generics, shapes, type
descriptors, linker specials, ABI markers, escaped package paths) and all 79
names from a generated compiler object file (including the `go:string.`,
`type:.namedata.` and `gclocals·` names the linker hides), extracted from Go
1.24.4. The test skips when `refs/` is absent and also checks that printed
names contain no `·`, no leftover `%XX` escapes, no ABI marker, and are
idempotent. Bare assembly/cgo names (`aeshashbody`, `_cgo_init`) carry no
package and stay out of Go.

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

The in-tree Rust AST parser (`rust::parse`) handles both legacy `_ZN` and v0
`_R` encodings. Its corpus gate covers all 115 upstream unit-test pairs, plus
6,958 legacy and v0 symbols from Rust 1.75.0 and 1.100.0 `libstd` and
`librustc_driver`, with zero wrong outputs. `rustc-demangle` is now only a
test oracle, not a runtime dependency.

The in-tree C++ printer covers the Itanium and Microsoft ASTs. Its Itanium
text gate is 29,737/29,933 exact; the MSVC gate is 971/1,219 exact against
`msvc-demangler`, with the remaining differences pinned to LLVM lit-test
spellings.

All schemes are in-tree and always available; there are no runtime
dependencies. The crate is `no_std` + `alloc`.

## Usage

Library:

```rust
use demangle::{Demangler, Language, Scheme};

let demangler = Demangler::new();
let demangled = demangler.demangle("_ZN3Foo3barEv").unwrap();

assert_eq!(demangled.scheme(), Scheme::Cpp);
assert_eq!(demangled.language(), Language::Cxx);
assert_eq!(demangled.to_string(), "Foo::bar()"); // allocates only here, in the caller

// Swift is demangled in-tree:
let swift = demangler.demangle("$s4main5helloSSyYaKF").unwrap();
assert_eq!(swift.to_string(), "main.hello() async throws -> Swift.String");

// Go names are classified and normalised in-tree:
let go = demangler.demangle("runtime.(*mLockProfile).captureStack").unwrap();
assert_eq!(go.scheme(), Scheme::Go);
assert_eq!(go.to_string(), "runtime.(*mLockProfile).captureStack");
assert_eq!(
    demangler
        .demangle("example.com/x/pkg%2ev1.Make")
        .unwrap()
        .to_string(),
    "example.com/x/pkg.v1.Make",
);
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
# demangle = "0.1"
```

## Project Layout

See `AGENTS.md` for agent operating guidelines and the project structure.

## References

`refs/` holds offline reference material (official specs and test corpora) with
pinned provenance. It is not distributed: `Cargo.toml` excludes it from the
published crate. See `refs/README.md`, `refs/swift.md` and `refs/go.md`.

## Changelog

See `CHANGELOG.md` for the version history.
