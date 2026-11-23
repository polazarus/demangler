---
name: add-demangling-scheme
description: Add, validate, and document a new demangling scheme or symbol decoration in the demangle crate. Use when adding a language/ABI backend, a new Scheme/Language variant, or an orthogonal platform decoration such as COFF.
---

# Add a demangling scheme

Use this skill for a new language/ABI backend, a new symbol decoration, or a
new public `Scheme`/`Language` interpretation. `AGENTS.md` is authoritative;
this skill is the executable checklist derived from it.

Do not use this skill for:
- fixing an existing parser or printer (use the backend's tests directly);
- adding a new corpus to an existing scheme (follow `refs/<scheme>.md`);
- adding a runtime demangler dependency (forbidden).

## 0. Settle the contract first

Write a short transient plan in `.plans/<scheme>.md` and answer:

1. **Is this a language scheme or an orthogonal decoration?**
   - Language/ABI: add `Scheme` + `Language` variants and a dispatch arm.
   - Decoration: keep the underlying `Scheme`, add metadata and an accessor.
     Follow the COFF precedent: `Scheme::C`/`Cpp` stays, and
     `Demangled::coff_decoration()` carries the decoration.
2. **What is the exact input grammar?** Pin the specification URL and the
   grammar subset you will claim. Do not implement “looks like”.
3. **What is the canonical output?** If there is no standard demangler,
   document a project contract with input/output examples.
4. **What is the evidence?** Identify:
   - a permissively licensed corpus or generated binaries;
   - a generation-time oracle, if one exists;
   - the license and provenance of every source and tool.
5. **What will be rejected?** List unsupported productions up front. A clean
   rejection is acceptable; a guessed demangling is not.

Hard invariants:

- raw bytes (`&[u8]`) parser, no UTF-8 precondition;
- `no_std` + `alloc` for language backends; decorations may be alloc-free;
- bounded input, depth, work/node and output budgets;
- invalid identifier bytes escaped reversibly as `\xNN`;
- no partial output, no panics, no unbounded recursion;
- zero wrong output; corpus floors only rise, never fall.

## 1. Scaffold

Keep the scaffold in one commit so feature subsets compile before the parser
lands.

### Cargo

- Add a narrow feature, e.g. `d = []`, and add it to `default` if the crate's
  default policy includes every backend.
- Use an umbrella feature only when several dialects belong together.
- `coff`-style decorations must not require `alloc`; verify the allocator gate.

### Crate root

`src/lib.rs`:

- declare `pub mod <scheme>;` behind the feature;
- add the feature to `extern crate alloc` **only** if the backend uses
  `alloc`;
- use the shared parser layer in `src/parse/` (see §3) and add the feature to
  the `many0` gate in `src/parse/mod.rs` and `src/parse/combinator.rs` if the
  parser needs it;
- extend the supported-language docs.

### Public result and dispatch

- Add `Language::<Name>` and `Scheme::<Name>` variants if this is a language.
  Append variants; adding a non-`#[non_exhaustive]` enum variant is a breaking
  change, so use `feat!` and a `BREAKING CHANGE:` changelog note.
- Add `Language::as_str`, `Scheme::language`, `Demangled::Text` variant and
  constructor.
- Add a `Scheme::detect` and `Scheme::detect_bytes` check, and a
  `Demangler::demangle_bytes` arm.
- For a decoration, add the metadata field/accessor instead of a `Scheme`
  variant; apply it by consuming the inner `Demangled`.

### Backend API

Follow the existing shape:

```rust
pub struct Ast<'a> { /* retained parse */ }
pub enum ParseError { Invalid, RecursedTooDeep, BudgetExceeded }
pub fn parse(symbol: &str) -> Result<Ast<'_>, ParseError>;
pub(crate) fn parse_bytes(symbol: &[u8]) -> Result<Ast<'_>, ParseError>;
pub fn is_<scheme>(symbol: &[u8]) -> bool;
```

`Display` must be total for anything `parse` accepts. If the printer has an
output cap, either prove the cap is unreachable or pre-render in the dispatcher
and turn a `fmt` failure into `Error::InvalidSymbol` (see the C++ arm).

## 2. Detection

Detection is a precision problem, not a recall problem.

- Claim only after a full parse, unless the prefix uniquely identifies the
  scheme (`_R`, `_Z`, `?`, `$s`, explicit `k*:`).
- Put checks in the correct order:
  - decorations on raw input before GNU `@`/`@@` and ThinLTO stripping;
  - D before Itanium/legacy;
  - JNI before legacy C++ (long names contain `__`);
  - Objective-C/Kotlin before Go where dotted metadata can collide;
  - high-confidence Ada prefixes only; general `pkg__subp` stays C.
- Add all four tests:
  1. a positive symbol is detected;
  2. a malformed member is rejected;
  3. a nearby C/legacy name is **not** stolen;
  4. with the new feature disabled, the same symbol is not claimed.
- Test both `detect` and `detect_bytes`; keep UTF-8 and raw-byte behavior
  aligned.
- Test `Demangler::demangle_bytes` end to end, including scheme/language and
  invalid UTF-8 where the grammar allows it.

## 3. Implement safely

### Shared parser layer (`src/parse/`)

All byte-native backends parse through the same private layer; reuse it
instead of copying a cursor:

- `Cursor` reads a bounded `&[u8]`, tracks offsets/spans and restores position
  and depth through `Checkpoint`;
- `Limits`/`State` charge the input, depth, work and node budgets;
- `Escape`/`write_escaped` render identifier or payload bytes reversibly
  (`\\` and `\xNN`);
- `Parser` plus `tag`, `take`, `ident`, `decimal_length`, `alt`, `cut`, `eof`
  and `many0` express backtracking versus committed failure.

Parse from `&[u8]` through that cursor, or a cursor with equivalent
accounting, then budget every untrusted axis explicitly:

- input cap: usually 64 KiB;
- depth cap for every recursive production;
- work cap for quadratic/back-reference rescans;
- node cap for AST construction;
- output cap, usually `4 * MAX_INPUT_LEN` to allow `\xNN` escaping.

Also:

- never use recursion without `enter`/`leave` or an explicit depth check;
  prefer an iterative loop for tail recursion;
- use `Cow`/borrowed text for identifiers; only reconstruct when required;
- reject unknown productions; do not emit opaque/lossy nodes;
- keep `Drop` safe for deep ASTs: cap depth or make the drop iterative;
- add unit tests for malformed, truncated, over-budget and adversarial input.

The shared layer is stable: keep its module docs accurate, add a unit test for
every new primitive, and never add a module-wide `#![allow(dead_code)]`. The
core-only `--no-default-features` build has one narrow `cfg_attr` allowance,
and primitives used only by `src/parse/tests.rs` carry a per-item allowance
with a comment.

## 4. Build the evidence

Create `refs/<scheme>.md` and `refs/<scheme>/`:

- upstream URL, pinned commit/tag or package version, retrieval date, license;
- exact generation/extraction commands;
- SHA256 for vendored fixtures/corpora;
- a measured category breakdown;
- a refresh procedure.

Vendor only what is permissively licensed:

- symbol names and generated expected text are facts and may be stored;
- compiler binaries, JDK/GNAT/LLVM trees and GPL/LGPL test suites are not
  vendored;
- libiberty is read-only behavioural reference; never copy its test suite.

Corpus tests:

- use `#![cfg(feature = "<scheme>")]`;
- read corpora with `env!("CARGO_MANIFEST_DIR")`;
- **skip gracefully** when `refs/` is absent (the published tarball excludes
  it);
- pin measured exact counts and category coverage, not a loose `>= 500`;
- classify every row as `exact`, `documented-unsupported` or `oracle-gap`;
- never count an oracle echo (`output == input`) or `<...>` placeholder as
  exact;
- assert zero wrong output and require unsupported forms to be rejected.

Add adversarial inputs:

- `refs/adversarial/<scheme>.txt` (generated by
  `refs/adversarial/generate.py`);
- add the file to `tests/adversarial_corpus.rs` `CORPORA`;
- cover deep nesting, back-reference expansion, oversized length prefixes,
  truncation and output-expansion bombs.

## 5. Documentation

Update every surface in the same change:

- `README.md`: feature bullet, supported-language table, feature matrix,
  coverage section with measured counts and gaps.
- `src/lib.rs`: supported-language docs and feature notes.
- `src/result.rs`: `Demangled` docs if a new retained representation exists.
- `AGENTS.md`: design decisions, project tree, `refs/` index, and the
  per-feature workflow list.
- `docs/unsupported-schemes.md`: remove the implemented scheme, move its
  remaining gaps to a documented-unsupported section, and update decoration
  rows.
- `refs/README.md`: index row.
- `CHANGELOG.md`: short imperative entries under `[Unreleased]`; mark breaking
  enum/API additions.
- Module docs: supported forms, canonical output, detection, budgets, gaps.

Do not leave “implemented in this batch” headings or stale “not implemented”
rows.

## 6. Validate the whole matrix

Run from the repository root:

```sh
cargo test --all-features
cargo test --no-default-features
cargo test --no-default-features --features <scheme>
cargo clippy --workspace --all-targets --all-features -- -D warnings
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --all-features
cargo build --lib --no-default-features
cargo build --lib --no-default-features --features <scheme>
cargo build --lib --no-default-features --features <scheme> --target thumbv7em-none-eabi
cargo build --release
```

Also:

- run each existing feature alone to catch detection/feature-gate regressions;
- run the corpus test with and without `refs/` (use a scratch copy or rename
  `refs/` temporarily; restore it afterwards);
- `rustfmt --check` only the files you changed (the repo may have pre-existing
  formatting diffs);
- `git diff --check`;
- inspect the adversarial harness with the new corpus;
- confirm the D-like output-expansion case cannot panic through
  `Demangler::demangle(...).to_string()`.

## 7. Commit and clean up

- Use Conventional Commits, e.g. `feat(<scope>): add <scheme> demangling`,
  `test(<scope>): add <scheme> corpus`, `docs: document <scheme>`.
- One logical unit per commit; keep shared integration separate from the
  backend.
- Remove the `.plans/<scheme>.md` file when the work is complete; do not commit
  `.plans/`.
- Do not push, tag, or open a PR unless asked.

## Acceptance checklist

- [ ] Feature builds alone, with defaults, and without defaults.
- [ ] `coff`-style decorations remain alloc-free.
- [ ] Full-parse detection; no prefix-only claims.
- [ ] Positive, malformed, non-stealing and feature-disabled tests.
- [ ] Raw-byte and invalid-UTF-8 behavior tested.
- [ ] Input/depth/work/node/output budgets enforced and tested.
- [ ] Printer totality proven or failure converted to a parse error.
- [ ] Pinned corpus with provenance and zero wrong output.
- [ ] Corpus tests skip when `refs/` is absent.
- [ ] Adversarial corpus and harness updated.
- [ ] README, crate docs, AGENTS, docs, refs index, CHANGELOG updated.
- [ ] Full validation matrix green.
- [ ] Plan removed; no dead branches/worktrees left behind.
