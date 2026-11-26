# Usage

Library:

```rust
use demangler::{Demangler, Language, Scheme};

let demangler = Demangler::new();
let demangled = demangler.demangle("_ZN3Foo3barEv").unwrap();

assert_eq!(demangled.scheme(), Scheme::CppItanium);
assert_eq!(demangled.language(), Language::Cpp);
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

Raw symbol bytes:

```rust
use demangler::Demangler;

let demangler = Demangler::new();
let raw = demangler.demangle_bytes(b"_ZN3Foo3barEv")?;
assert_eq!(raw.to_string(), "Foo::bar()");
```

Object-file symbol names are byte strings, not guaranteed UTF-8. `demangle`
takes `&str`; `demangle_bytes` accepts `&[u8]` and feeds every backend through
the raw-byte parser. Invalid UTF-8 in identifier text is retained and printed
as `\xNN`; a name that no backend recognizes is returned as escaped plain text
with `Scheme::Unknown`.

Explicit schemes (detection bypassed):

```rust
use demangler::{Demangler, Scheme};

let demangler = Demangler::new();

// Text form, matching `demangle`.
let ada = demangler.demangle_with_scheme(Scheme::Ada, "pkg__subp")?;
assert_eq!(ada.to_string(), "pkg.subp");

// Byte form, matching `demangle_bytes`.
let raw = demangler.demangle_bytes_with_scheme(Scheme::C, b"plain\xff")?;
assert_eq!(raw.to_string(), "plain\\xff");
```

`Scheme` names one exact grammar or encoding, so a forced scheme is exact: the
selected backend must accept the input, and the result reports that same
scheme. `CppMsvc` never demangles an Itanium name, and a forced scheme never
falls back to another language:

```rust
use demangler::{Demangler, Error, Scheme};

let demangler = Demangler::new();
assert_eq!(
    demangler
        .demangle_with_scheme(Scheme::CppMsvc, "_Z3foov")
        .unwrap_err(),
    Error::InvalidSymbol(Scheme::CppMsvc),
);
```

`Scheme::C` and `Scheme::Unknown` are pass-through schemes and return the input
as text; they never parse. A scheme whose backend is not compiled into the
build fails with `Error::InvalidSymbol` for that scheme.

## Languages and candidate lists

`Language` groups the schemes of one source language; `Scheme::language()` and
`Language::schemes()` are the exhaustive mapping between them. The group entry
points expand the language to its schemes:

```rust
use demangler::{Demangler, Language, Scheme};

let demangler = Demangler::new();

// Any C++ grammar; the precise scheme is reported on success.
let msvc = demangler.demangle_with_language(Language::Cpp, "?x@@3HA")?;
assert_eq!(msvc.scheme(), Scheme::CppMsvc);

// Ordered candidates: the first scheme that demangles wins.
let rust = demangler.demangle_with_schemes(
    &[Scheme::RustV0, Scheme::RustLegacy],
    "_RNvCs1234_4main4main",
)?;
assert_eq!(rust.scheme(), Scheme::RustV0);
```

`demangle_with_language`/`demangle_with_languages` and
`demangle_with_schemes` try the allowed candidates in the order given, each with
the exact-scheme rules above. Errors:

- one allowed candidate fails: `Error::InvalidSymbol(scheme)`, or
  `Error::Budget { scheme, kind }` when it hit a parser limit;
- several allowed candidates all fail: `Error::NoMatchingScheme`;
- `Error::scheme()` returns `Scheme::Unknown` for `NoMatchingScheme`.

The byte twins are `demangle_bytes_with_language`,
`demangle_bytes_with_languages` and `demangle_bytes_with_schemes`.

## Selection

`Demangler::selection()` starts a reusable, `Copy` `Selection`. `allow` adds a
scheme or every scheme of a language; `prefer` also moves the expanded schemes
to the front:

```rust
use demangler::{Demangler, Language, Scheme};

// One exact grammar.
let msvc = Demangler::new()
    .selection()
    .allow(Scheme::CppMsvc)
    .demangle("?x@@3HA")?;
assert_eq!(msvc.scheme(), Scheme::CppMsvc);

// Any C++ grammar, preferring MSVC.
let preferred = Demangler::new()
    .selection()
    .allow(Language::Cpp)
    .prefer(Scheme::CppMsvc)
    .demangle("foo__Fiv")?;
assert_eq!(preferred.scheme(), Scheme::CppGnuV2);

// Language-level ambiguity: Go or C++, Go first.
let go_or_cpp = Demangler::new()
    .selection()
    .allow(Language::Go)
    .allow(Language::Cpp)
    .prefer(Language::Go)
    .demangle("fmt.Println")?;
assert_eq!(go_or_cpp.scheme(), Scheme::Go);
```

Selection semantics:

- an empty selection is exactly `Demangler::demangle`/`demangle_bytes`;
- `allow(Scheme)` adds one exact scheme; `allow(Language)` adds every scheme in
  the group; `allow_schemes`/`allow_languages` take slices;
- `prefer` adds the target and moves its schemes to the front; repeated calls
  keep call order; `prefer_schemes`/`prefer_languages` take slices;
- empty slices are no-ops and duplicate selectors are harmless;
- candidates are tried as preferred schemes in call order, then the remaining
  allowed schemes in stable detection order, with `C` and `Unknown` last;
- `prefer(Scheme::C)` or `prefer(Scheme::Unknown)` overrides that
  pass-through-last rule;
- every candidate uses the exact-scheme path, and COFF decorations are peeled
  once before the search;
- a feature-disabled scheme fails its candidate and the next one is tried;
- if the whole selection fails, the error is `Error::NoMatchingScheme`.

`Selection::demangle`/`demangle_bytes` are the text and byte entry points.

## Detected fallback

Only the detected path keeps the C++-then-Rust fallback: `demangle` and
`demangle_bytes` detect a precise scheme and, if a detected C++ candidate fails,
try the Rust parser and report `RustLegacy` or `RustV0` from the parsed
encoding. Forced schemes and `Selection` never retag a failed C++ parse as
Rust.

## Budgets

Every backend bounds untrusted input with compiled-in limits. `Budget` carries
caller overrides for those parser limits — input bytes, nesting depth, work
units and allocated nodes. `None` keeps the compiled-in default, and a value is
clamped to the scheme's hard cap:

```rust
use demangler::{Budget, Demangler, Limit, Scheme};

let demangler = Demangler::with_budget(Budget::new().with_input_bytes(8 * 1024));
let effective = demangler.effective_budget(Scheme::CppItanium);

assert_eq!(effective.input_bytes, Some(Limit::Bounded(8 * 1024)));
assert_eq!(Scheme::CppItanium.default_budget().input_bytes, Some(64 * 1024));
```

`Scheme::default_budget` reports the compiled-in values, `Scheme::hard_budget`
the provisional ceiling an override may reach, and `Budget::effective` resolves
one budget for one scheme. Every dimension is a `usize`. `0` rejects at a
bounded dimension; a dimension a backend does not have or does not cap ignores
the override. A dimension the backend has but does not cap reports
`Limit::Unbounded`.

The Rust, Swift, C++, flat-grammar (Ada, Fortran, Delphi, OCaml), Go,
Objective-C, Kotlin/Native, JNI and D backends enforce the resolved overrides.
The flat-grammar schemes have no depth or node dimension, Go, Kotlin/Native and
JNI have no node dimension, and Objective-C's grammar has a single bracket
level, which is both its default and its ceiling. Input and work are always
independent: raising the input limit alone keeps the default work allowance, so
raise `work` too when a long symbol needs more than the default scan. The D
node allowance is independent in the same way: raising the input alone keeps
the default node count.

The hard caps are per backend. Input is 16 MiB and work is 64M units. Depth is
1024 for the recursive and counter-based grammars, except where a backend's
grammar or representation is tighter: rustc v0 keeps a checkpoint per input byte
(1 MiB input), the C++ and Swift print envelopes bound the nesting their
parsers accept, Objective-C has one bracket level, and a JNI `Type` stores its
array-dimension count in a `u8`, so JNI depth clamps to 255.

Two further dimensions, `expansion_visits` and `expansion_bytes`, are parse-time
expansion gates for the schemes whose spelling can expand: the shared-DAG
backends (Rust, Swift and C++), where one parsed node is rendered once per
reference, and D, whose back references and modifiers re-expand earlier input.
They bound the expansion of an accepted AST, not the bytes a printer writes, so
no printer cap and no `Display` behaviour depends on them. The linear backends
(Go, Objective-C, Kotlin/Native, JNI, Ada, Fortran, Delphi and OCaml) print each
node once, so they have no expansion gate and report `None` for both. The gates
resolve and clamp like the parser dimensions, and the defaults are 4M visits and
1 MiB for Rust, 131,072 visits and 1 MiB for Swift, 262,144 visits and 1 MiB for
Itanium and MSVC, 65,536 visits and 1 MiB for the legacy C++ dialects, and
262,144 visits and 262,144 bytes for D. The shared-DAG visit ceiling is 16M and
the D ceiling is 1M; the byte ceiling is 64 MiB. These gates are part of the
policy tables and are reported by `Budget::effective` today, but no backend
enforces them yet.

Printing is not a budget: no `Budget` field configures a printer. A shared-DAG
backend rejects a tree the parser accepts but the printer cannot render within
its fixed parse-time envelope, instead of printing it partially. A linear
printer (the flat-grammar backends, Go, Objective-C, Kotlin/Native, JNI and D)
walks each owned node once, so its parser budgets already bound the output; the
fixed output caps it keeps are defensive backstops, not print budgets. The
flat-grammar, Go, Objective-C, Kotlin/Native and JNI backstops are sized from
the hard input ceiling, which no accepted parse can reach. The D backstop stays
at four times the default input cap and does not grow with the input budget: a
D spelling is not linear in the input (an `immutable` modifier prints ten bytes
and a back reference re-parses an earlier part of the symbol), so a symbol the
parser accepts can still exceed it, and the demangler reports that as a
rejection instead of truncating.

`Scheme::detect` keeps the compiled-in budget. A `Demangler` runs a
budget-aware detection pass: parse-based prefilters try the configured limits
first, and fall back to the defaults only for classification when that parse
fails, so a too-small budget cannot silently turn a known scheme into plain C.
The dispatch then re-parses any retained tree under the configured budget, so
enforcement is exact.

## `no_std` callers

```rust
use core::fmt::Write;
use demangler::Demangler;

let demangler = Demangler::new();
let mut out = MyFixedBuffer::new();
let _ = write!(out, "{}", demangler.demangle("_ZN3Foo3barEv")?);
```

## CLI

```sh
# single symbol
demangle "_ZN3Foo3barEv"

# from stdin
cat symbols.txt | demangle -
```
