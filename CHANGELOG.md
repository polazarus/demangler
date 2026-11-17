# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- `refs/` reference material for implementers, excluded from the published
  crate (`exclude = ["refs"]`): Swift ABI mangling grammar and the official
  demangle corpus, pinned to upstream commit `fa6c2608`.
- `no_std` core with **no `alloc` requirement**: demangled text is exposed via
  `Demangled` / `core::fmt::Display`, so callers pick their own sink.
- `Error` type with `Unsupported` / `InvalidSymbol` variants.
- Basic CLI (`demangle <symbol>`, stdin batch).
- `Scheme::detect` fast prefix detection and `Scheme::language` / `Language::as_str`.
- Feature flags: `rust` (`no_std`, no `alloc`) and `cpp` (needs `alloc`/`std`).

### Changed

- Replaced the allocating `DemangledResult` with the `Display`-based `Demangled`.
- C++ backends moved behind the `cpp` feature and flagged for a future rewrite.

### Removed

- `DemangledResult` and its owned `String` output.

## [0.1.0] - 2026-11-17

### Added

- Initial crate scaffold with `Demangler`, `Scheme`, and `Language` types.
- C, C++, Rust, Swift, and Go scheme detection stubs.

[Unreleased]: https://github.com/polazarus/demangle/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/polazarus/demangle/releases/tag/v0.1.0
