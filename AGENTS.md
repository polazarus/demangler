# AGENTS.md

Operating instructions for agents working on this repository.

## Project

`demangler` is a universal demangler for binary symbol names written in pure Rust.

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
- **C++**: implemented in-tree (`src/cpp/`) as a pure-Rust, `no_std` +
  `alloc` demangler with one shared AST (`cpp::Ast`, `cpp::Node`,
  `cpp::Kind`) and per-dialect parsers and printers: Itanium (`_Z`), MSVC
  (`?`), legacy GNU v2 (`foo__Fiv`), ARM, EDG and HP aCC. `cpp::Dialect` reports which dialect
  an AST came from. Legacy dialects have no unambiguous prefix, so
  `cpp::parse` and `Scheme::detect` select one only when its parser consumes
  the whole symbol; never claim a dialect from a prefix alone, and never emit
  `Kind::Opaque` for a legacy production. Corpus gates:
  `tests/cpp_text_corpus.rs`, `tests/cpp_msvc_corpus.rs` and
  `tests/cpp_legacy_corpus.rs` (GNU v2, ARM, EDG, HP aCC). MSVC retains
  member-pointer offsets and dynamic-initializer signatures; the lit gate has
  zero parser/printer errors. libiberty's legacy demangler and its test suite
  are GPL: consult them read-only as behavioural reference, never vendor or
  copy them (see `refs/cpp.md`). Remaining vendor dialects are documented
  evidence blockers, not guessed parsers.
- **Go**: implemented in-tree (`src/go/`) as a pure-Rust, `no_std` + `alloc`
  name parser/printer. Go has no mangling format and the toolchain ships no
  demangler, so the backend follows the linker's name layout: package path
  (`%XX` escapes), local name, closures/wrappers, generic shapes, `type:` and
  `go:` symbols. Printing decodes escapes, normalises `·` to `.` and strips
  ABI markers (reported by `go::Ast::abi`). Never add an external Go demangler.
  Coverage is measured by `tests/go_corpus.rs` against `refs/go/`; raise the
  floor when coverage improves, never lower it.
- **Objective-C**: implemented in-tree (`src/objc/`) as a pure-Rust, `no_std`
  metadata/method parser. Objective-C has no single mangling; the backend
  recognises Apple/Mach-O metadata (`_OBJC_`, `__OBJC_`, bare `OBJC_`),
  GNUstep/GNU runtime metadata (`__objc_*`, GNUstep 2.0 `_OBJC_*`, including
  the real ELF `._OBJC_*` and COFF `$_OBJC_*` decorations), method display
  forms (`-[Class selector:]`, `+[Class selector:]`) and block
  descriptors. It prints a project-defined canonical spelling such as
  `Objective-C class Foo` and `Objective-C category Foo (Bar)`. Class,
  category and protocol names are preserved verbatim; unknown metadata
  prefixes are rejected, never guessed. Objective-C++ `_Z` names stay C++.
  Coverage is measured by `tests/objc_corpus.rs` against `refs/objc/`; raise
  the corpus counts when coverage improves, never lower them.
- **Kotlin/Native**: implemented in-tree (`src/kotlin_native/`) as a pure-Rust,
  `no_std` + `alloc` linker-symbol parser/printer for the compiler's `k*:`
  prefixes (`kfun:`, `ktype:`, `kclass:`, `kproperty:`, `kobj:`, `kfield:`,
  `ktypealias:`, `kpackage:`, `knamespace:`, `kvar:`) and Mach-O
  leading-underscore variants. Output is a project-defined Kotlin-like debugger
  spelling (`kfun:com.example.Foo#bar(kotlin.Int){}` ->
  `com.example.Foo.bar(kotlin.Int)`). Detection claims only known `k*:`
  prefixes, before Go and plain C, and never steals C++/ObjC/Swift/Rust/Go.
  Unknown markers are rejected, not guessed. Kotlin/JVM and Kotlin/JS are out
  of scope. Coverage is measured by `tests/kotlin_native_corpus.rs` against the
  pinned Kotlin/Native 2.4.21 corpus in `refs/kotlin-native/`; raise the floor
  when coverage improves, never lower it.
- **D**: implemented in-tree (`src/d/`) as a pure-Rust, `no_std` + `alloc`
  ABI parser/printer. It follows the current D ABI grammar and `c++filt -s
  dlang` spelling, with `MAX_INPUT_LEN`/depth/work/node/output budgets and
  explicit rejection of unsupported value/template forms. Never add an
  external D demangler. The documented oracle gaps are closed: `_DThn` thunks,
  `return scope` parameters and back-referenced function types are exact.
  Coverage is `tests/d_corpus.rs` against `refs/d/`.
- **Java JNI**: implemented in-tree (`src/jni/`) as a pure-Rust, `no_std` +
  `alloc` parser for short and long `Java_` names. It decodes JNI escapes,
  class/method boundaries and overload argument descriptors; there is no
  canonical demangler, so the display form is the project contract in
  `refs/jni.md`. Coverage is `tests/jni_corpus.rs`.
- **Ada/GNAT**: implemented in-tree (`src/ada/`) as a pure-Rust, `no_std` +
  `alloc` external-name parser. The first slice covers user-level GNAT names,
  protected-operation suffixes, `___elabb`/`___elabs`, entry barrier/body
  routines and task body helpers; other runtime suffixes are rejected. Automatic detection is precision-only (`ada__`, `system__`,
  `gnat__`); general `pkg__subp` names are available through `ada::parse`.
  Coverage is `tests/ada_corpus.rs` against `refs/ada/`.
- **Fortran**: implemented in-tree (`src/fortran/`) for gfortran
  `__module_MOD_member` names, default trailing-underscore externals,
  `MAIN__` and generated derived-type helpers. Detection is precision-only (`__*_MOD_*`); general externals are
  available through `fortran::parse` or `Demangler::demangle_with_scheme`.
  Coverage is `tests/fortran_corpus.rs` against `refs/fortran/`.
- **Delphi/Object Pascal**: implemented in-tree (`src/delphi/`) for Free
  Pascal/Delphi `$`-separated linker names, including compiler data symbols
  (`U_$`, `VMT_$`, `RTTI_$`, `INIT_$`, `INIT$_$`, `FINALIZE$_$`) and unit
  entry routines. Output is a project-defined Pascal-like spelling; ambiguous
  nested/CRC forms are rejected. Coverage is `tests/delphi_corpus.rs` against
  `refs/delphi/`.
- **OCaml**: implemented in-tree (`src/ocaml/`) for `camlModule.member_stamp`
  linker names, module records, static-data numeric labels, runtime markers
  and safe `.cold`/`.part.N` fragments. The detection check runs before Go so
  dotted names are not stolen. Coverage is `tests/ocaml_corpus.rs` against
  `refs/ocaml/`.
- **PE/COFF**: `src/coff.rs` is an alloc-free decoration parser. The
  platform-agnostic path decodes `_name@N`, `@name@N` and `__imp_`; the
  target-aware path adds x86/x64 vectorcall, COMDAT weak aliases, ARM64
  undecorated names, bare leading underscores and `.cold`/`.part.N` fragments.
  `Demangler::demangle_macho*` provides the explicit Mach-O C leading-underscore
  path. Decorations are orthogonal
  to the language scheme: `Demangled::coff_decoration()` reports convention,
  kind, stack bytes, import and weak-alias state, while `Display` prints the
  undecorated name. Recognition runs on raw input before GNU `@` version
  stripping. Coverage is `tests/coff_decoration.rs` against `refs/coff/`.
- **Explicit schemes and encodings**: `Demangler::demangle_with_scheme` (text)
  and `Demangler::demangle_bytes_with_scheme` (bytes) bypass detection for
  deliberately conservative names (general Ada/Fortran/Delphi/OCaml and
  target-aware COFF). The `encoding` feature decodes reversible `\xNN` escapes
  as UTF-8, Latin-1, Windows-1252, MacRoman or EBCDIC CP037; it does not
  transcode inside ASTs and does not support Shift-JIS/EUC-JP, UTF-16/32 or
  other EBCDIC pages. Keep `--no-default-features` alloc-free: `encoding` is
  optional and alloc-gated.
- **no_std with alloc**: the core crate is `#![no_std]` and uses `alloc`.
  Demangled text is exposed through `core::fmt::Display` (`Demangled`); backends
  may store owned `String`s. `std`-only dependencies remain feature-gated.
  Allocation is allowed in new code, but untrusted input must stay bounded:
  parsers need explicit depth/work/node budgets.
- **CLI**: a basic `demangle` binary lives in `src/bin/demangle.rs`.

## Git Discipline

- **Use Git for all changes**: commit after each logical unit of work.
- **Branch**: the primary branch is `main`.
- **Remote**: `origin` = `git@github.com:polazarus/demangler.git`.
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
demangler/
├── src/
│   ├── lib.rs          # crate root, re-exports
│   ├── demangler.rs    # orchestrator + backend dispatch
│   ├── scheme.rs       # Scheme / Language detection
│   ├── result.rs       # Demangled (Display-based, no alloc)
│   ├── error.rs        # Error type
│   ├── parse/          # shared raw-byte cursor, budgets, escapes and combinators
│   ├── cpp/            # in-tree C++ AST + Itanium/MSVC/GNU v2/ARM/EDG parsers and printers
│   ├── swift/          # in-tree Swift demangler (Rc node tree, parser, printer)
│   ├── go/             # in-tree Go name parser, AST and printer
│   ├── objc/           # in-tree Objective-C metadata/method parser and printer
│   ├── kotlin_native/  # in-tree Kotlin/Native linker-symbol parser and printer
│   ├── d/              # in-tree D ABI parser, AST and printer
│   ├── jni/            # in-tree Java JNI native-method parser and printer
│   ├── ada/            # in-tree Ada/GNAT external-name parser and printer
│   ├── fortran/        # in-tree gfortran external-name parser and printer
│   ├── delphi/         # in-tree Free Pascal/Delphi name parser and printer
│   ├── ocaml/          # in-tree OCaml external-name parser and printer
│   ├── coff.rs         # PE/COFF decoration parser (no alloc)
│   ├── encoding.rs     # optional reversible-escape encoding decoder
│   └── bin/demangle.rs # CLI (std, separate target)
├── refs/               # offline reference material (never published)
│   ├── README.md       # provenance + refresh rules
│   ├── cpp.md          # C++ links, vendored corpora, legacy reference rules
│   ├── swift.md        # Swift links, vendored grammar + corpus
│   ├── go.md           # Go naming rules, toolchain corpus + provenance
│   ├── objc.md         # Objective-C runtime layout, symbol corpora + provenance
│   ├── kotlin-native.md # Kotlin/Native mangler provenance + symbol corpora
│   ├── d.md            # D ABI, LDC/libphobos and c++filt provenance
│   ├── jni.md          # JNI grammar, JDK fixture and display contract
│   ├── ada.md          # GNAT external-name grammar, fixture and corpus
│   ├── fortran.md      # gfortran fixture, underscoring modes and corpus
│   ├── delphi.md       # Free Pascal fixture, grammar and corpus
│   ├── ocaml.md        # OCaml fixture, stdlib/runtime corpora and contract
│   └── coff.md         # PE/COFF decoration corpus and generator
├── .agents/skills/     # project-local agent skills (scheme onboarding)
├── docs/               # user-facing documentation
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
   - `cargo test --no-default-features` and each language/dialect feature alone
     (`rust`, `swift`, `go`, `objc`, `kotlin-native`, `d`, `jni`, `ada`,
     `fortran`, `delphi`, `ocaml`, `coff`, `encoding`, `cpp-itanium`,
     `cpp-msvc`, `cpp-gnu-v2`)
   - `cargo build --release`
   - Optional, not a required gate: `cargo all-features check` when the
     third-party `cargo-all-features` tool is installed (see
     `docs/feature-matrix.md`; it creates a large target directory).
4. Commit with a Conventional Commits message.
5. Update `CHANGELOG.md` in the same commit.
6. Remove the corresponding `.plans/` file once the task is done.
7. When upstream reference material changes, refresh `refs/` by pinned SHA and
   update the provenance note in the same commit.
8. Ask the user when requirements are ambiguous.