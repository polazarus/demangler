# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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

[Unreleased]: https://github.com/polazarus/demangler/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/polazarus/demangler/releases/tag/v0.1.0
