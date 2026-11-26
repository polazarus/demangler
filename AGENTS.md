# AGENTS.md

Operating instructions for agents working on this repository. User-facing
behaviour lives in `docs/`; grammar provenance and corpora in `refs/`.

## Project

`demangler` is a universal demangler for binary symbol names, in pure Rust.

### Invariants

- **In-tree backends only.** Every scheme is implemented in `src/<backend>/`
  as a `no_std` + `alloc` parser and printer. Never add a runtime demangler
  dependency (`rustc-demangle`, `cpp_demangle`, `msvc-demangler` are
  dev-only oracles; `swift-demangle` and external Go/D demanglers are never
  added).
- **Schemes are exact.** `Scheme` names one grammar/encoding and
  `Scheme::language()` maps it to a `Language` group (`Language::Cpp` holds
  `CppItanium`, `CppMsvc`, `CppGnuV2`, `CppArm`, `CppEdg`, `CppHp`; Rust and
  Swift split into `RustLegacy`/`RustV0` and `SwiftStable`/`SwiftLegacy`).
  `ObjC` and `Go` are one scheme each by design. Never add an umbrella scheme.
- **Detection is precise.** It is prefix-first, ordered in `src/detection.rs`,
  and a heuristic never steals another scheme's names (Objective-C++ `_Z`
  stays C++; OCaml, Delphi and Kotlin/Native run before Go). Legacy C++
  dialects have no prefix: claim one only when its parser consumes the whole
  symbol. Legacy C++ and legacy Swift get one bounded parse, never a second. Ada, Fortran, Delphi and
  OCaml auto-detect only their unambiguous forms; general names go through
  `demangle_with_scheme`.
- **Forced schemes and `Selection` are exact**: a parse from another
  dialect/encoding is `Error::InvalidSymbol(scheme)`; only the detected path
  keeps the C++-then-Rust fallback. Several allowed candidates that all fail
  give `Error::NoMatchingScheme`; an empty `Selection` is exactly `demangle`.
- **Reject, never guess.** Unknown productions, prefixes and markers are
  errors; legacy C++ never emits `Kind::Opaque`, and unsupported vendor
  dialects stay documented gaps (`docs/unsupported-schemes.md`). Project-defined display
  forms (Objective-C, Kotlin/Native, JNI, Delphi, ...) are documented
  contracts in `refs/<backend>.md`.
- **Spec over reference.** Only the mangling spec constrains the grammar;
  a reference demangler's implementation limits are not copied. A corpus row
  that differs for that reason is a documented divergence in its corpus test.
- **Bounded untrusted input.** Every parser limit is a `Budget` dimension
  (`input_bytes`, `parse_depth`, `work`, `nodes`, plus the parse-time
  `expansion_visits`/`expansion_bytes` gates for Rust, Swift, C++ and D),
  resolved per scheme through `Scheme::default_budget`/`hard_budget` and
  `Budget::effective`. Exhaustion is `Error::Budget { scheme, kind }`. Never
  add a fixed limit outside these dimensions: bound lists by input/work/nodes,
  share the budget with nested re-parses, and charge input-driven repeat
  counts as work. Printing is not a budget and no printer has a cap.
- **Encodings and decorations are orthogonal.** PE/COFF decorations
  (`src/coff.rs`, alloc-free) are reported by `Demangled::coff_decoration()`;
  the `encoding` feature only decodes reversible `\xNN` escapes.
- **Core is alloc-free.** `--no-default-features` builds without `alloc`;
  `std` is only for the CLI (`src/bin/demangle.rs`) and tests.
- **Corpus floors only rise.** Each backend has a corpus test in `tests/`
  against `refs/`; raise a floor when coverage improves, never lower it.
- **GPL references are read-only.** libiberty's demangler and test suite may
  be consulted as behaviour, never vendored or copied (see `refs/cpp.md`).

## API Naming

- Text entry points are unsuffixed (`parse`, `demangle`); raw-byte twins end
  in `_bytes`. Predicates are `is_<scheme>` / `is_<scheme>_bytes`.
- Methods taking a scheme or language use `with_scheme` / `with_language`;
  plural forms take slices and preserve their order.
- Encoding variants use code-page names (`Windows1252`, `MacRoman`,
  `Ebcdic037`).

## Rust Standards

- Edition 2024. Format with `cargo fmt --all`.
- Lint levels live in `Cargo.toml` (`[lints.clippy]`, including `pedantic`,
  and `[lints.rust]`), which only raises levels: never add a crate-wide
  `allow`. Fix a warning when it is pertinent; otherwise use a local
  `#[expect(lint, reason = "...")]`. Use `#[allow(lint, reason = "...")]` only
  for a lint that fires in some feature sets and not others.
- All public items have `///` docs (`missing_docs = "deny"`).
- `unsafe` is rare, needs a `// SAFETY:` comment and human review.
- Unit tests sit in `#[cfg(test)]` modules next to the code; integration tests
  in `tests/`.

## Dependencies

- Pure Rust: no FFI, no `build.rs` compiling C/C++.
- No runtime dependencies. Dev-dependencies are pinned to a minor version.

## Workflow

1. Read the surrounding code and match its patterns; keep changes focused.
2. Plans go in `.plans/` (git-ignored); delete the file when the task is done.
3. Verify:
   - `cargo fmt --all --check`
   - `cargo clippy --workspace --all-targets --all-features -- -D warnings`
   - the same clippy with `--no-default-features`, and with
     `--no-default-features --features <f>` for each of `rust`, `swift`,
     `go`, `objc`, `kotlin-native`, `d`, `jni`, `ada`, `fortran`, `delphi`,
     `ocaml`, `coff`, `encoding`, `cpp-itanium`, `cpp-msvc`, `cpp-gnu-v2`
   - `cargo test --all-features`, `cargo test --no-default-features` and the
     same per-feature list
   - `cargo build --lib --no-default-features --target thumbv7em-none-eabi`
   - `cargo build --release`
4. Update `CHANGELOG.md` ([Keep a Changelog 1.1.0](https://keepachangelog.com/en/1.1.0/):
   `## [Unreleased]` first, short imperative entries) in the same commit.
5. Commit each logical unit with
   [Conventional Commits](https://www.conventionalcommits.org/en/v1.0.0/)
   (`feat(scheme): ...`, `fix(cpp): ...`, `refactor!: ...` for breaking).
6. When upstream reference material changes, refresh `refs/` by pinned SHA
   and update its provenance note (`refs/README.md`) in the same commit.
7. Ask the user when requirements are ambiguous.

## Git

- Primary branch `main`; remote `origin` =
  `git@github.com:polazarus/demangler.git`.
- Never amend, rebase, force-push or hard-reset without explicit permission.
- Do not tag, push or open PRs unless asked.

## `refs/`

Offline specs and corpora for implementers and tests; excluded from the
published crate and never a build dependency. Every vendored item has a note
in `refs/<backend>.md` with upstream URL, pinned commit SHA (never `main`),
retrieval date and license. Prefer links for small or rarely used material.

## Layout

```
src/
  lib.rs, demangler.rs   crate root; Demangler and backend dispatch
  scheme.rs, detection.rs, selection.rs
                         Scheme/Language taxonomy, detection, Selection
  budget.rs, error.rs, result.rs
                         Budget policy, Error, Demangled
  parse/                 shared byte cursor, limits, escapes, combinators
  cpp/ rust/ swift/ go/ objc/ kotlin_native/ d/ jni/ ada/ fortran/ delphi/ ocaml/
                         one backend each (parser, AST, printer)
  coff.rs, encoding.rs   PE/COFF decorations; optional escape decoding
  bin/demangle.rs        CLI
refs/                    reference material; README.md indexes it
tests/                   integration and corpus tests
docs/                    user documentation (usage, coverage, validation, ...)
.agents/skills/          add-demangling-scheme, prepare-release
```
