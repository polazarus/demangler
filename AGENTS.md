# AGENTS.md

Operating instructions for agents working on this repository.

## Project

`demangle` is a universal demangler for binary symbol names written in pure Rust.

Design decisions (current):

- **Backend strategy**: all schemes are in-tree. Existing Rust/C++ demanglers
  are dev-only test oracles.
- **Scheme model**: a flat `Scheme` enum, plus a `language()` method for language-level identification (`Language`).
- **Swift**: implemented in-tree (`src/swift/`) as a pure-Rust, `no_std` +
  `alloc` demangler: postfix operator parser building an `Rc`-owned node tree
  plus a `Display`-style printer. Raw identifier text borrows the input;
  reconstructed text is owned by `Cow`. Never add `swift-demangle`/`swift-demangler` (both
  are non-pure: native Swift runtime / CMake C++). Coverage is measured by
  `tests/swift_corpus.rs`; raise the floor when coverage improves, never lower
  it.
- **Rust**: implemented in-tree (`src/rust/`) as a pure-Rust, `no_std` +
  `alloc` demangler for both legacy `_ZN` and v0 `_R` encodings. Keep
  `rustc-demangle` as a dev-dependency oracle; do not add it back to runtime
  dependencies.
- **Go**: implemented in-tree (`src/go/`) as a pure-Rust, `no_std` + `alloc`
  name parser/printer. Go has no mangling format and the toolchain ships no
  demangler, so the backend follows the linker's name layout: package path
  (`%XX` escapes), local name, closures/wrappers, generic shapes, `type:` and
  `go:` symbols. Printing decodes escapes, normalises `·` to `.` and strips
  ABI markers (reported by `go::Ast::abi`). Never add an external Go demangler.
  Coverage is measured by `tests/go_corpus.rs` against `refs/go/`; raise the
  floor when coverage improves, never lower it.
- **no_std with alloc**: the core crate is `#![no_std]` and uses `alloc`.
  Demangled text is exposed through `core::fmt::Display` (`Demangled`); backends
  may store owned `String`s. `std`-only dependencies remain feature-gated.
  Allocation is allowed in new code, but untrusted input must stay bounded:
  parsers need explicit depth/work/node budgets.
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

## References (`refs/`)

- `refs/` holds offline reference material (official specs, test corpora) for
  implementers and agents. It is **not distributed**: `Cargo.toml` sets
  `exclude = ["refs"]`, so it never ships in the published crate.
- Every vendored item must have a note in `refs/<scheme>.md` with upstream URL,
  pinned commit SHA, retrieval date and license. Never fetch by `main`.
- Reference material is read-only: no build-time dependency. Tests may read it.
- Prefer links in `refs/<scheme>.md` for small or rarely used references.
- See `refs/README.md` for the rules and refresh procedure.

## Dependencies

- Pure Rust only. No C/C++ FFI and no `build.rs` that compiles C/C++.
- Reuse well-maintained crates rather than reinventing; pin to a minor version (e.g. `0.1`, `0.2`).
- Dependencies may use `alloc` freely. New `std`-requiring dependencies must
  be gated behind a feature flag. Runtime dependencies: none; test oracles are
  dev-dependencies.
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
│   ├── swift/          # in-tree Swift demangler (Rc node tree, parser, printer)
│   ├── go/             # in-tree Go name parser, AST and printer
│   └── bin/demangle.rs # CLI (std, separate target)
├── refs/               # offline reference material (never published)
│   ├── README.md       # provenance + refresh rules
│   ├── swift.md        # Swift links, vendored grammar + corpus
│   └── go.md           # Go naming rules, toolchain corpus + provenance
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
   - `cargo build --lib --no-default-features` (core-only)
   - `cargo build --lib --no-default-features --target thumbv7em-none-eabi` (bare-metal proof)
   - `cargo build --release`
4. Commit with a Conventional Commits message.
5. Update `CHANGELOG.md` in the same commit.
6. Remove the corresponding `.plans/` file once the task is done.
7. When upstream reference material changes, refresh `refs/` by pinned SHA and
   update the provenance note in the same commit.
8. Ask the user when requirements are ambiguous.