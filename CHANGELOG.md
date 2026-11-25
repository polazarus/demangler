# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Changed

- Skip the Itanium lambda-parameter pre-pass when the parsed C++ tree contains
  no lambda, and remove per-construct `Vec`/`String` allocations from the
  Itanium and MSVC printers.
- Reject non-dotted Go names before the full shape scan, hoist the encoding
  escape buffer, and copy non-escape runs in bulk.
- Decode already-owned C++/D text directly and render single-`Display` ASTs
  without `format_args!` in `Demangled`.
- Reuse the D, OCaml, Delphi and JNI trees parsed during detection instead of
  parsing those symbols a second time.

## [0.3.0] - 2026-11-25

### Added

- Public Swift payload inspection: `NodeData`, `Global`, `GlobalOperand`,
  `Accessor`, `MacroRole`, `AutoDiffCode`, `GenericSpecializationKind`,
  `KeyPathKind`, `LayoutCode`, `DeinitializerKind`, `AttributeKind`,
  `ResumeStyle`, and `Node::data()`; all `#[non_exhaustive]`.
- Containerised Swift demangler oracle (`refs/swift/Containerfile`) and a
  release-driven grammar refresh process (`refs/swift/refresh.sh`,
  `refs/swift/coverage.md`).

### Changed

- **BREAKING**: Swift AST nodes are immutable `Rc<Node>` values built by an
  internal builder; there is no `RefCell` and no post-construction mutation.
  `Node::text()` returns `&str`, `Node::children()` returns `&[NodeRef]`,
  `Ast::root()` returns a `NodeRef` handle, and the `Children` guard and
  `with_text` closure are removed. `Demangled` remains `!Send + !Sync`.
- Swift layout constraints are typed and validated (`LayoutCode`); unknown
  letters are rejected and `B`/`S` print nothing, matching the reference.
- Swift macro roles, autodiff codes, generic-specialization kinds, key-path
  flavours, deinitializer forms and function attributes are typed payloads
  instead of raw bytes.

### Fixed

- Accept `fMq` attached preamble macro expansions, which the printer already
  understood but the parser rejected.
- Reject `Yj` differentiability annotations outside `f|r|d|l` instead of
  printing a fallback.

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

[Unreleased]: https://github.com/polazarus/demangler/compare/v0.2.1...HEAD
[0.2.1]: https://github.com/polazarus/demangler/releases/tag/v0.2.1
[0.2.0]: https://github.com/polazarus/demangler/releases/tag/v0.2.0
[0.1.0]: https://github.com/polazarus/demangler/releases/tag/v0.1.0
