# Validation strategy

The project treats a wrong demangling as worse than a rejection. Validation is
therefore organised around three rules:

1. **Exact output where there is a contract.** A corpus row either demangles to
   the pinned expected text or is explicitly classified as unsupported.
2. **No guessing.** An unrecognised production is rejected, never approximated
   or passed off as another language.
3. **Bounds are part of correctness.** Untrusted input must terminate inside
   the input, depth, work, node and output budgets.

The corpus floors only rise. A change that lowers a floor or turns an exact row
into a mismatch fails review.

## Test layers

| Layer | What it checks |
|---|---|
| Unit tests | Cursor, budgets, escaping, display contracts, individual productions. |
| Integration tests | Public text/byte APIs, forced schemes, scheme detection, COFF decoration, encoding. |
| Corpus tests | Pinned symbol/expected-output pairs from real toolchains and specifications. |
| Oracle tests | Comparison with `cpp_demangle`, `rustc-demangle`, `msvc-demangler` or `c++filt` where an independent implementation exists. |
| Robustness tests | Oversized, malformed and recursive inputs terminate without panic or unbounded output. |
| Adversarial corpora | Deterministic symbol bombs: deep nesting, exponential DAGs, long escapes, huge lengths. |
| Build matrix | All features, no features, every language feature alone, `no_std` and bare metal. |

Corpus files live under `refs/`, which is excluded from the published crate.
Corpus tests skip with a message when `refs/` is unavailable, so a packaged
tarball still runs the remaining suite.

## Corpus gates

The detailed grammar and counts are in [coverage](coverage.md). The main gates:

| Backend | Pinned evidence |
|---|---|
| C++ Itanium | 29,929/29,933 exact, zero wrong, zero printer refusals; four bare type manglings excluded by name. |
| C++ MSVC | 1,219 lit symbols, 975 exact against the older oracle with 122 documented spelling differences; 59,248 real PE exports token-equivalent; zero parse/print errors. |
| C++ legacy | 1,610 GNU v2 pairs, 45 ARM, 14 EDG, 99 HP aCC rows (82 accepted, 17 documented rejections). |
| Rust | 115 upstream pairs plus 6,958 legacy/v0 symbols from Rust 1.75 and 1.100; zero wrong. |
| Swift | 524/524 stable, 248/248 legacy `_T`, 23,975/23,975 toolchain symbols. |
| Go | 79 compiler-object names, 3,326 binary names, 12,626 archive names; plus rejection and idempotence checks. |
| Objective-C | Apple/GNUstep metadata, method display forms, ELF/COFF ABI-2 names and 18,931 Mattermost app names. |
| Kotlin/Native | 8,917 toolchain symbols plus a 33-name hand corpus for the remaining markers. |
| D | 10,927 exact rows, zero oracle gaps, two documented unsupported productions. |
| Java JNI | 42 JDK-generated rows, 16 hand rows, 35 rejections, 1,037 real native names. |
| Ada/GNAT | 43 exact, 64 unsupported, 2 oracle gaps; GNAT 14.2 fixture. |
| Fortran | 55 exact, 43 unsupported; gfortran 14.2 fixture. |
| Delphi | 92 exact, 51 unsupported; FPC 3.2.2 fixture. |
| OCaml | 4,781 accepted, 5 unsupported; OCaml 5.3.0 fixture. |
| PE/COFF | 58 symbols and 68 target-aware rows. |

Corpus provenance, pinned commits, retrieval dates and licenses are recorded in
`refs/<scheme>.md` and indexed by `refs/README.md`.

## Adversarial and robustness

`refs/adversarial/generate.py` produces deterministic stress corpora for every
backend. `tests/adversarial_corpus.rs` requires each input to:

- terminate within two seconds;
- not panic;
- emit at most 8 MiB.

The inputs include deep pointer/nesting chains, oversized names, repeated split
markers, malformed lengths, exponential back-reference expansions and invalid
UTF-8. A backend may reject the symbol; it may not loop, overflow the stack or
allocate without a bound.

Focused robustness suites cover the language-specific risks:

- `swift_robustness.rs`, `objc_robustness.rs`,
  `kotlin_native_robustness.rs`, `new_backend_robustness.rs`;
- `robustness.rs` for the shared cursor/budget/escape layer.

## C++ printer budgets

C++ printing is bounded by depth, node visits and output bytes. Both the
Itanium/MSVC and legacy printers derive those caps from their parser's AST
budgets:

- print depth = 2 × parser depth;
- print visits = 16 × parser node budget;
- output = 1 MiB.

This leaves the remaining cap differences explained solely by the parsers'
different AST budgets, not by unexplained per-printer numbers. Generated deep
pointer chains pin the transition points: Itanium and MSVC accept 254 levels and
reject 255; legacy accepts 60 and rejects 61. These are parser-cap boundaries,
as asserted by `tests/cpp_nesting_limits.rs`, and they sit below the printer
caps.

## Build and feature matrix

Required checks:

```sh
cargo test --all-features
cargo test --no-default-features
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo clippy --workspace --all-targets --no-default-features -- -D warnings
```

Each language, dialect and encoding feature is also tested alone:

```sh
for feature in rust swift go objc kotlin-native d jni ada fortran delphi \
               ocaml coff encoding cpp-itanium cpp-msvc cpp-gnu-v2; do
  cargo test --no-default-features --features "$feature"
done
```

Core-only and bare-metal builds prove the allocator-free path:

```sh
cargo build --lib --no-default-features
cargo build --lib --no-default-features --target thumbv7em-none-eabi
cargo build --release
```

`coff`, `fortran` and `ocaml` remain allocator-free. `encoding` is optional and
alloc-gated. The full feature matrix and dependency policy are in
[features](features.md).

Optional exhaustive pairwise checks use
[`cargo-all-features`](feature-matrix.md):

```sh
cargo all-features check
```

That run is useful but not mandatory because it creates a large target
directory.

## Lints and documentation

`Cargo.toml` owns the lint policy. The required checks are clippy with
`-D warnings` for all features and for the core-only build, plus rustdoc with
`RUSTDOCFLAGS="-D warnings"`:

```sh
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --all-features
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --no-default-features
```

Public items require documentation (`missing_docs = "deny"`). Unsafe code is
not used; any future use needs a `// SAFETY:` explanation and explicit review.

Line/region coverage is measured periodically with `cargo llvm-cov` for
visibility, but coverage percentage is not a gate. Corpus exactness and
adversarial robustness are the primary quality signals.

## Oracles and reference material

- Runtime dependencies: none.
- External demanglers are dev-dependencies or test-only tools.
- GPL reference implementations are consulted read-only and never vendored.
- Vendored reference material is pinned by commit SHA with provenance in
  `refs/`.
- A corpus is never fetched from a moving branch during a test or build.

## Known limits

- Some formats have no canonical demangler (Objective-C, JNI, Kotlin/Native,
  Delphi); their printed spelling is a documented project contract.
- Some legacy C++ dialects, schemes and encodings are intentionally unsupported
  and rejected; see [unsupported schemes](unsupported-schemes.md).
- Corpus tests require `refs/`; without it they skip rather than fail.
- Adversarial checks prove termination and bounded output, not semantic
  equivalence for every malformed input.
- There is no continuous integration configuration in the repository; the
  commands above are the release checklist.
