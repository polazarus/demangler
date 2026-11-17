# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Initial scaffolding: universal symbol demangler library in pure Rust.
- Flat [`Scheme`] detection with a [`Language`] convenience method.
- Demangler delegating to existing pure Rust backends (rustc-demangle, itanium-demangle, msvc-demangler) behind feature flags.
- Basic CLI for single symbol demangling.

## [0.1.0] - 2026-11-17

### Added

- Initial crate scaffold with `Demangler`, `Scheme`, and `DemangledResult` types.
- C, C++, Rust, Swift, and Go scheme detection stubs.

[Unreleased]: https://github.com/polazarus/demangle/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/polazarus/demangle/releases/tag/v0.1.0
