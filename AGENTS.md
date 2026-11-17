# AGENTS.md

Operating instructions for agents working on this repository.

## Project

`demangle` is a universal demangler for binary symbol names written in pure Rust.

Design decisions (current):

- **Backend strategy**: delegate to existing pure Rust crates for now. Performance-critical paths may be rewritten later; keep the delegation layer thin so it can be swapped.
- **Scheme model**: a flat `Scheme` enum, plus a `language()` method for language-level identification (`Language`).
- **Swift**: not delegated to an external crate — it will be implemented in a dedicated pure-Rust submodule. Do not add `swift-demangle`/`swift-demangler` (both are non-pure: native Swift runtime / CMake C++).
- **Go**: planned; no pure-Rust backend exists yet.
- **no_std, no alloc**: the core crate is `#![no_std]` and does **not** link
  `alloc`. Demangled text is exposed through `core::fmt::Display` (`Demangled`),
  never as an owned `String`. Every backend that needs `alloc` or `std` sits
  behind a feature flag (`cpp`) and is a rewrite candidate. Keep new code
  `core`-only.
- **CLI**: a basic `demangle` binary lives in `src/bin/demangle.rs`.

## Git Discipline

- **Use Git for all changes**: commit after each logical unit of work.
- **Branch**: the primary branch is `main`.
- **Remote**: `origin` = `git@github.com:polazarus/demangle.git`.
- **Conventional Commits**: all commit messages follow [Conventional Commits v1.0.0](https://www.conventionalcommits.org/en/v1.0.0/).
  - Format: `<type>(<scope>): <description>`
  - Types: `feat`, `fix`, `docs`, `style`, `refactor`, `perf`, `test`, `build`, `ci`, `chore`, `revert`
  - Breaking changes: `!` after type/scope OR a `BREAKING CHANGE:` footer
  - Examples:
    - `feat(scheme): add Go symbol detection`
    - `fix(demangler): route _ZN names to the C++ backend`
    - `docs: document the CLI`
    - `refactor(scheme)!: rename Scheme variants`
    - `chore: bump msvc-demangler to 0.11`
- **No history rewriting**: never `git commit --amend`, `git rebase`, `git push --force`, or `git reset --hard` without explicit user permission.
- Do not create tags, push, or open PRs unless the user asks.

## Rust Standards

- **Edition 2024**.
- **Lints live in `Cargo.toml`** (`[lints.clippy]`, `[lints.rust]`), not in source attributes.
  - Code must build clean with `cargo clippy --workspace --all-targets --all-features -- -D warnings`.
  - `cargo test --all-features` must pass.
- **Unsafe**: rare and always human-validated. Any `unsafe` block needs a `// SAFETY:` comment and explicit review. Do not add crate-level `#![allow(unsafe_code)]`.
- **Documentation**: all public items have `///` docs (`missing_docs = "deny"`).
- **Tests**: unit tests in `#[cfg(test)]` modules next to the code; integration tests in `tests/`.

## CHANGELOG

- Maintain `CHANGELOG.md` following [Keep a Changelog 1.1.0](https://keepachangelog.com/en/1.1.0/).
- Structure: `## [Unreleased]` at top, then `## [X.Y.Z] - YYYY-MM-DD` (newest first) with `### Added`, `### Changed`, `### Deprecated`, `### Removed`, `### Fixed`, `### Security`.
- Update the changelog in the same commit as the change it describes.
- Short, imperative entries.

## Plans

- Plans go in `.plans/` and are **transient**.
- Once a plan is complete, remove the file and do **not** commit `.plans/` to history (it is git-ignored).
- Plans are working documents, not project artifacts.

## Dependencies

- Pure Rust only. No C/C++ FFI and no `build.rs` that compiles C/C++.
- Reuse well-maintained crates rather than reinventing; pin to a minor version (e.g. `0.1`, `0.2`).
- New `std`/`alloc`-requiring dependencies must be gated behind a feature flag
  and noted as rewrite candidates. Allowed today: `cpp` (`itanium-demangle`
  needs `std`, `msvc-demangler` needs `alloc`).
- Never add `alloc` to the core path. Prefer `core::fmt::Write` sinks.

## Project Structure

```
demangle/
├── src/
│   ├── lib.rs          # crate root, re-exports
│   ├── demangler.rs    # orchestrator + backend dispatch
│   ├── scheme.rs       # Scheme / Language detection
│   ├── result.rs       # Demangled (Display-based, no alloc)
│   ├── error.rs        # Error type
│   └── bin/demangle.rs # CLI (std, separate target)
├── tests/              # integration tests (as needed)
├── Cargo.toml          # package, features, lints
├── README.md
├── AGENTS.md
├── CHANGELOG.md
└── .plans/             # transient plans (git-ignored)
```

## Agent Workflow

1. Read existing code and match established patterns before changing anything.
2. Make minimal, focused changes.
3. Verify every feature combination:
   - `cargo test --all-features`
   - `cargo clippy --workspace --all-targets --all-features -- -D warnings`
   - `cargo build --lib --no-default-features` (core-only, no alloc)
   - `cargo build --lib --no-default-features --features rust --target thumbv7em-none-eabi` (bare-metal proof)
   - `cargo build --release`
4. Commit with a Conventional Commits message.
5. Update `CHANGELOG.md` in the same commit.
6. Remove the corresponding `.plans/` file once the task is done.
7. Ask the user when requirements are ambiguous.