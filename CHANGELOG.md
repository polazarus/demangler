# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- `Display` for `Language` and `BudgetKind`, `Hash` for `BudgetKind`,
  `Clone` for `Demangled` and every backend `Ast`, and `PartialEq`/`Eq` for
  `Selection`.

### Changed

- **Breaking:** the `budget`, `demangler`, `encoding`, `error`, `result`,
  `scheme` and `selection` modules are private; import their types from the
  crate root (`demangler::Budget`, `demangler::Demangler`, ...), where they were
  already re-exported. The module documentation moved onto `Budget`,
  `Encoding` and `Selection`.
- **Breaking:** remove `Demangler::demangle_with_schemes`,
  `demangle_with_languages` and their `_bytes` twins; use
  `demangler.selection().prefer_schemes(..)` / `prefer_languages(..)`.
- **Breaking:** remove `Demangler::demangle_bytes_to_string_with_encoding`;
  use `demangle_bytes(..)?.to_string_with_encoding(..)`.
- **Breaking:** remove `Error::Unsupported`, which was never produced.
- **Breaking:** `Scheme`, `Language` and `Encoding` are `#[non_exhaustive]`, so
  a new scheme is no longer a breaking change; `Scheme::ALL` is a slice and
  `Scheme::COUNT` is gone (use `Scheme::ALL.len()`).
- **Breaking:** rename `Language::ObjectiveC` to `Language::ObjC`, matching
  `Scheme::ObjC` and the `objc` feature.

## [0.3.1] - 2026-11-27

### Fixed

- `Demangler` no longer returns an Itanium C++ symbol that names a template
  parameter with no argument (`_Z1fIiEvT0_`); printing such a result used to
  fail, so `to_string()` panicked.
- Itanium C++: a template parameter declaration in a template argument list
  must be followed by the argument it qualifies.
- MSVC C++: a `$1` template argument must name a symbol.
- Itanium C++: print `pp_`/`mm_` as prefix `++x`/`--x` and give postfix
  increments postfix precedence.
- Itanium C++: parse the operand of `tw` (throw) as an expression, not a type.
- Itanium C++: print `ix` array subscript expressions instead of failing.
- Swift: demangle `Xo`, `Xu` and `Xw` as `unowned`, `unowned(unsafe)` and
  `weak` instead of `__owned` and `__shared`.
- Swift: spell the observer accessors `willset` and `didset`, as Swift does.
- Swift: print an escaping autoclosure as `@autoclosure`, without `@escaping`.
- Swift: parse `WZ`/`Wz` as a context and a variable list and print
  `one-time initialization function for x`; name a vtable thunk's base method
  first; use Swift's texts for the dynamically replaceable variable, key and
  thunk.
- Swift: print `.Protocol` for the metatype of an existential (including
  `Any`) and `.Type` for a bare protocol or an existential metatype, and
  parenthesize multi-protocol compositions under a metatype or optional.
- Swift: label an unnamed parameter `_:` in an old-style (`_T0`) function
  type whose other parameters are labeled.
- Swift: print a function or initializer whose type is not a plain function
  type as Swift does: `name @convention(c) ...` for a C function pointer and
  `name : type` otherwise, instead of an invented `() -> ()`.
- Swift: demangle the conformance witness tables (`Wa`, `Wp`, `WG`, `WI`,
  `Wr`) and the lazy witness table accessor and cache (`Wl`, `WL`) from a
  protocol conformance instead of printing stray fragments.
- Swift: use Swift's texts for `Mr` (`type metadata completion function for`)
  and for module, extension and anonymous descriptors.

## [0.3.0] - 2026-11-26

### Added

- `Budget` overrides the input, depth, work and node limits and the parse-time
  `expansion_visits`/`expansion_bytes` gates; `Demangler::with_budget`,
  `set_budget` and `effective_budget`, `Scheme::default_budget`/`hard_budget`
  and `Budget::effective` resolve it per scheme. Every backend honors it.
- `Error::Budget { scheme, kind }` and `BudgetKind` tell a capacity failure
  from a malformed symbol.
- Swift payload inspection: `Node::data()`, `NodeData` and typed payload enums
  (`Global`, `Accessor`, `LayoutCode`, ...), all `#[non_exhaustive]`.

### Changed

- **BREAKING**: `Demangler` is a struct carrying a `Budget`; `new()` and
  `Default` behave as before.
- **BREAKING**: Swift nodes are immutable `Rc<Node>` values: `Node::text()`
  returns `&str`, `Node::children()` returns `&[NodeRef]` and `Ast::root()`
  returns a `NodeRef`; the `Children` guard and `with_text` are removed.
- Backends keep no fixed limits outside `Budget`: list lengths, nesting and
  input-driven repeat counts are charged to input, work or nodes, and
  printers have no output caps.
- Grammars follow their specs, not a reference demangler's implementation
  limits: Swift prints builtin widths above 4096 and every generic parameter
  name (no `...` after 128), and Rust v0 decodes punycode names longer than
  128 chars.
- A `Selection` returns a candidate's budget error instead of
  `NoMatchingScheme`.
- Detection reuses the trees it parses, and the Itanium, MSVC and Go paths
  allocate less.

### Removed

- **BREAKING**: the fixed-limit constants `go::MAX_SUFFIXES`,
  `ada::MAX_SEGMENTS`, `ocaml::MAX_PATH_SEGMENTS`, `delphi::MAX_SCOPES`,
  `delphi::MAX_PARAMS`, the `kotlin_native::MAX_*` list caps,
  `fortran::MAX_DEPTH` and every backend's `MAX_OUTPUT_LEN`.

### Fixed

- Kotlin/Native names longer than 64 KiB are detected instead of passing
  through as plain text.
- `swift::print` applies the expansion gates, and Swift repeated
  substitutions and node copies are charged to the budget.
- Nested payloads (Swift legacy specializations, HP aCC literals) share the
  containing parse's budget and reject the symbol when it runs out instead of
  printing undemangled text.
- MSVC back-reference deduplication compares names of any depth.
- Swift accepts `fMq` preamble macro expansions, and rejects unknown layout
  letters and `Yj` codes outside `f|r|d|l`.

## [0.2.1] - 2026-11-25

### Added

- Staged single-symbol Criterion latency benchmarks for detection, backend
  parsing, full demangling, sink rendering and owned-string rendering.

### Changed

- Reuse the legacy C++/Swift tree parsed during detection for dispatch instead
  of parsing the symbol a second time, removing a full second parse from those
  paths.
- Replace the Itanium operator-name linear scan with a direct two-byte lookup,
  removing a per-name table scan while preserving every operator spelling.

### Fixed

- Reject oversized iterative Swift wrapper chains before building a graph that
  a rejected parse would have to tear down recursively.

## [0.2.0] - 2026-11-24

### Added

- Precise scheme variants: `CppItanium`, `CppMsvc`, `CppGnuV2`, `CppArm`,
  `CppEdg`, `CppHp`, `RustLegacy`, `RustV0`, `SwiftStable` and `SwiftLegacy`
  report the exact grammar or encoding that was decoded.
- Scheme and language selection: `Demangler::selection()` with a reusable `Copy`
  `Selection`, `Selector`, `allow`/`prefer` by scheme or language, and the
  `demangle_with_language(s)`/`demangle_with_schemes` entry points plus their
  byte twins.
- `Error::NoMatchingScheme` when every candidate allowed by a selection fails.

### Changed

- **Breaking:** `Language::Cxx` is renamed to `Language::Cpp`; the C++ group
  maps to the six precise C++ schemes.
- **Breaking:** the umbrella `Scheme::Cpp`, `Scheme::Rust` and `Scheme::Swift`
  variants are replaced by the precise variants above.
- Forced schemes and `Selection` are exact: a name parsed by another dialect or
  encoding returns `Error::InvalidSymbol(requested_scheme)` instead of being
  retagged. Only the detected path keeps the C++-then-Rust fallback.
- Detection stays prefix-first; legacy C++ and legacy Swift still require one
  bounded full parse before their scheme is known.
- **Breaking:** API naming consistency: `Demangler::demangle_to_string_with_encoding`
  is now `demangle_bytes_to_string_with_encoding`, `go::is_go_name` is
  `go::is_go`, and `Encoding::Cp1252` is `Encoding::Windows1252`.

## [0.1.0] - 2026-11-24

### Added

- Pure-Rust, `no_std` + `alloc` demangler with no runtime dependencies or
  C/C++ FFI.
- In-tree backends for C, C++ (Itanium, MSVC, GNU v2, ARM, EDG, HP aCC), Rust
  (legacy and v0), Swift (stable and legacy), Go, Objective-C, Kotlin/Native,
  D, Java JNI, Ada/GNAT, Fortran, Delphi/Object Pascal and OCaml.
- PE/COFF decorations and explicit Mach-O C name handling.
- Optional decoding of UTF-8, Latin-1, Windows-1252, MacRoman and EBCDIC CP037
  identifier escapes.
- `Demangler`, `Scheme`, `Language` and `Demangled` APIs, raw-byte entry points
  and explicit-scheme dispatch.
- Inspectable AST APIs and in-tree parsers/printers for every backend.
- A `demangle` CLI for arguments and line-oriented stdin.
- Per-backend Cargo features for smaller builds, with a default all-backend
  configuration.

### Changed

- `Demangled` renders through `core::fmt::Display`; C++ and D keep validated
  text, while Rust and Swift retain their parsed trees.
- `Demangled` is `!Send + !Sync` when it retains Rust or Swift `Rc`-owned trees;
  demangle again on the target thread instead of moving the value.

### Security

- Bounded input, depth, work, node and output budgets on untrusted symbols.
- Reversible `\xNN` escaping for invalid bytes and escaped control characters
  in identifier text.
- Adversarial recursive-bomb and oversized-input corpora in CI.

### Removed

- Runtime demangler dependencies and native FFI.
- Unreleased deprecation aliases; use the named scheme entry points.

[Unreleased]: https://github.com/polazarus/demangler/compare/v0.3.1...HEAD
[0.3.1]: https://github.com/polazarus/demangler/releases/tag/v0.3.1
[0.3.0]: https://github.com/polazarus/demangler/releases/tag/v0.3.0
[0.2.1]: https://github.com/polazarus/demangler/releases/tag/v0.2.1
[0.2.0]: https://github.com/polazarus/demangler/releases/tag/v0.2.0
[0.1.0]: https://github.com/polazarus/demangler/releases/tag/v0.1.0
