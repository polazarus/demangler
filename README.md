# demangle

A universal demangler for binary symbol names written in pure Rust.

## Features

- **Scheme detection**: automatically detects the mangling scheme from the symbol name.
- **Multi-language**: C, C++ (Itanium, MSVC, legacy GNU v2/ARM/EDG), Rust, Swift, Go, and Objective-C.
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
| Rust     | rustc v0, legacy | in-tree pure-Rust demangler (`src/rust/`) | `rust` |
| C++      | Itanium ABI | in-tree pure-Rust demangler (`src/cpp/`) | `cpp-itanium` / `cpp` |
| C++      | MSVC | in-tree pure-Rust demangler (`src/cpp/`) | `cpp-msvc` / `cpp` |
| C++      | legacy GNU v2 / ARM / EDG | in-tree pure-Rust demangler (`src/cpp/legacy.rs`) | `cpp-gnu-v2` / `cpp` |
| Swift    | `$s`, `$S`, `_$s`, `_$S`, `_T0`, `_T` | in-tree pure-Rust demangler (`src/swift/`) | `swift` |
| Go       | Go linker names | in-tree pure-Rust demangler (`src/go/`) | `go` |
| Objective-C | Apple/Mach-O and GNUstep metadata, method display forms | in-tree pure-Rust demangler (`src/objc/`) | `objc` |

Feature matrix:

| Features | `no_std` | needs `alloc` | needs `std` |
|----------|----------|---------------|-------------|
| *(none)* | ✅ | ❌ | ❌ |
| `rust` | ✅ | ✅ | ❌ |
| `swift` | ✅ | ✅ | ❌ |
| `go` | ✅ | ✅ | ❌ |
| `objc` | ✅ | ❌ | ❌ |
| `cpp-itanium` / `cpp-msvc` / `cpp-gnu-v2` | ✅ | ✅ | ❌ |
| `cpp` (all C++ dialects) | ✅ | ✅ | ❌ |
| default (`cpp`, `rust`, `swift`, `go`, `objc`) | ✅ | ✅ | ❌ |

The Swift demangler is enabled by the `swift` feature (on by default); it is
pure Rust and needs no `std`.
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

`cargo test --test go_corpus` validates **3,326** Go names from the ELF symbol
table of a generated test binary, **79/79** generated compiler-object names,
and **12,626/12,626** archive symbols from multiple standard-library packages.
The archive corpus adds static temps, jump tables, `eqfunc`/`hashfunc`,
qualified methods and object-version suffixes. All corpora are extracted from
Go 1.24.4. The test skips when `refs/` is absent and also checks that printed
names contain no `·`, no leftover `%XX` escapes, no ABI marker, and are
idempotent. Bare assembly/cgo names (`aeshashbody`, `_cgo_init`) carry no
package and stay out of Go.

### Swift coverage

Validated against the official corpus vendored in `refs/swift/manglings.txt`:

- **522/524** symbols demangle exactly with zero wrong outputs: builtins and
  integer types, identifiers (word and Punycode substitutions), nominal/bound
  generic types, compound associated types, tuples, functions and accessors,
  standard substitutions and sugar, metatypes, generic
  parameters/signatures/requirements, impl (SIL) function types (`Is`/`II`
  substitutions, conventions, differentiability, coroutines), thunks (partial
  apply, reabstraction, autodiff, key-path, witness, outlined, distributed,
  resume partial), specializations, opaque types, macros
  (freestanding/attached/unique/location), anonymous contexts, packs/variadic
  generics and retroactive/dependent conformances. The two remaining symbols
  are Mach-O `_$s` names that use raw identifiers (`` `3 times` ``), which are
  rejected rather than printed wrongly;
- all **248/248** pre-stable `_T` manglings demangle exactly: legacy types
  (`_Tt`), function and accessor entities (`_TF`), metadata (`_TM`), witness
  tables (`_TW`), value witnesses (`_Tw`), thunks and specializations (`_TT`),
  variables, subscripts and partial apply forwarders. The pre-stable grammar is
  a prefix grammar unrelated to `Mangling.rst`; it is ported in
  `src/swift/legacy.rs` from the reference old demangler and builds the same
  node graph as the postfix parser.

`cargo test --test swift_corpus` runs both gates (all prefixes and the legacy
subset) and skips when `refs/` is absent. An additional 23,975-symbol corpus
extracted from the Swift 6.4.2 Linux toolchain is tracked by `cargo test --test
swift_toolchain_corpus` (also skips without `refs/`); all 23,975 currently
demangle exactly.

The in-tree Rust AST parser (`rust::parse`) handles both legacy `_ZN` and v0
`_R` encodings. Its corpus gate covers all 115 upstream unit-test pairs, plus
6,958 legacy and v0 symbols from Rust 1.75.0 and 1.100.0 `libstd` and
`librustc_driver`, with zero wrong outputs. `rustc-demangle` is now only a
test oracle, not a runtime dependency.

The in-tree C++ printer covers the Itanium, Microsoft and legacy GNU v2/ARM/EDG
ASTs. Its Itanium text gate is **29,929/29,933 exact, zero wrong, zero printer
refusals**; the four excluded corpus entries are bare type manglings, not
encodings. The MSVC real-binary gate is **59,248/59,248 token-equivalent**.
The MSVC lit gate is 975/1,219 exact against `msvc-demangler`, with the
remaining differences pinned to LLVM lit-test spellings.

Legacy GNU v2 names (`foo__Fiv`, `bar__3foo`, `_$_3foo`, `_3foo$bar`) share the
same AST and are parsed by `src/cpp/legacy.rs`. Because the `__` separator is
ambiguous, detection validates the whole symbol: `foo__bar` stays plain C while
`a__b__c` demangles as `a__b(char)`. Supported: `__`-separated function names,
`Q<n>`-qualified names, templates, operators, constructors/destructors,
pointer/reference/array/function/member-pointer types, back-references and the
`__vt_`/`__ti`/`__tf`/`__thunk_`/`_<name>$<variable>` special names. Squangling
(`K`/`B`/`n`), member templates (`H`), template template parameters and
`_GLOBAL_$I$`/`$D$` names are rejected rather than approximated.
ARM and EDG forms share the same legacy path: ARM reserved names (`__ct__`,
`__dt__`, `__vtbl__`, `__sti__`, `__std__`) are tried before GNU v2 and fall
back on a full-parse failure; EDG adds its trailing return-type form.
`cargo test --test cpp_legacy_corpus` gates **1,610 GNU v2 pairs** (87
hand-audited plus a 1,523-pair sample from the permissively licensed
`gnuv2_demangle` corpus), **45 ARM** pairs and **14 EDG** pairs.

All schemes are in-tree and can be enabled individually; there are no runtime
dependencies. The default feature set enables every backend. The crate is
`no_std` + `alloc` when any backend is enabled.

```toml
# only Rust, no C++/Swift/Go/Objective-C
demangle = { version = "0.1", default-features = false, features = ["rust"] }

# only Objective-C metadata/method parsing (no allocator needed)
demangle = { version = "0.1", default-features = false, features = ["objc"] }

# only the MSVC C++ parser/printer
demangle = { version = "0.1", default-features = false, features = ["cpp-msvc"] }

# only Itanium C++ and Go
demangle = { version = "0.1", default-features = false, features = ["cpp-itanium", "go"] }
```

### Objective-C coverage

Objective-C has no single mangling format. The in-tree backend recognises the
runtime metadata that appears in linker symbol tables and the method display
forms that crash reports and developer tools show:

- Apple/Mach-O metadata with `_OBJC_`, `__OBJC_` or bare `OBJC_` prefixes:
  classes, metaclasses, ivars, categories, instance/class method lists,
  property lists, instance variables, protocols, protocol references/labels/
  refs/method types, class and category protocol lists, read-only class data,
  selector references and method-name/type strings;
- GNUstep/GNU runtime metadata: `__objc_class_name_*`, `__objc_class_ref_*`,
  `__objc_ivar_offset_*`, the GNUstep 2.0 `_OBJC_CLASS_*`, `_OBJC_REF_CLASS_*`,
  `_OBJC_INIT_CLASS_*` and `_OBJC_PROTOCOL_*` forms, and old GNU/NeXT
  `_OBJC_Class_*` records;
- method display forms `-[Class selector:]`, `+[Class selector:]` and
  `-[Class(Category) selector]`;
- block descriptors `__block_descriptor_…` and `___block_descriptor_…`.

The printed text is a project contract, because no standard Objective-C
demangler defines one:

| Input | Output |
|---|---|
| `_OBJC_CLASS_$_NSString` | `Objective-C class NSString` |
| `_OBJC_METACLASS_$_NSString` | `Objective-C metaclass NSString` |
| `__OBJC_$_CATEGORY_NSString_$_Drawing` | `Objective-C category NSString (Drawing)` |
| `__OBJC_$_INSTANCE_METHODS_Foo` | `Objective-C instance methods Foo` |
| `__OBJC_$_CLASS_METHODS_Foo` | `Objective-C class methods Foo` |
| `__OBJC_$_PROP_LIST_Foo` | `Objective-C property list Foo` |
| `__OBJC_$_INSTANCE_VARIABLES_Foo` | `Objective-C instance variables Foo` |
| `__OBJC_PROTOCOL_$_NSCopying` | `Objective-C protocol NSCopying` |
| `__objc_class_name_Foo` | `Objective-C class Foo` |
| `-[Foo bar:baz:]` | `-[Foo bar:baz:]` |

Class, category and protocol names are preserved verbatim. Both one and two
leading underscores are accepted; metadata prefixes are case-sensitive.
Objective-C++ `_Z` names stay C++, and unknown metadata prefixes are rejected
rather than guessed. `objc::parse` returns an inspectable `objc::Ast`, and
`objc::is_objc` is the detection prefilter.

`cargo test --test objc_corpus` gates the corpora in `refs/objc/corpus/`:
32 Apple/Mach-O metadata names and 6 method display forms emitted by Clang
19.1.7, 8 GNUstep/GNU-runtime metadata names, 51 metadata records from Debian
GCC 14's `libobjc.a`, and 17 runtime helpers/truncated prefixes that must not
render as Objective-C. The test skips when `refs/` is absent. Parsing is
bounded by input length, bracket depth and work units; output stays within a
small constant of the input.


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

// Objective-C metadata and method display forms are rendered in-tree:
let objc = demangler.demangle("_OBJC_CLASS_$_NSString").unwrap();
assert_eq!(objc.scheme(), Scheme::ObjC);
assert_eq!(objc.to_string(), "Objective-C class NSString");
assert_eq!(
    demangler.demangle("-[Foo bar:baz:]").unwrap().to_string(),
    "-[Foo bar:baz:]",
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
published crate. See `refs/README.md`, `refs/swift.md`, `refs/go.md` and
`refs/objc.md`.

## Changelog

See `CHANGELOG.md` for the version history.

