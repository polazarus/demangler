# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Add `Error::Budget { scheme, kind }` and `BudgetKind` so callers can tell a
  capacity failure from malformed input. C++ reports the exact dimension;
  coarse backend errors report `BudgetKind::Other`.
- Add a runtime budget policy: `Budget` carries caller overrides for the
  input, depth, work and node limits, `Scheme::default_budget` and
  `Scheme::hard_budget` expose the compiled-in defaults and provisional
  ceilings, `Budget::effective` resolves one budget for one scheme, and
  `Demangler` stores a budget with `new`/`with_budget`/`budget`/`set_budget`
  and `effective_budget` while staying `Copy`, `Default` and `const`.
  Every backend reads the overrides.
  Printing is not a budget, so no field configures a printer cap.
- Add the parse-time expansion gates `Budget::expansion_visits` and
  `Budget::expansion_bytes` with the `with_expansion_visits` and
  `with_expansion_bytes` builders, the matching `EffectiveBudget` fields and
  the `BudgetKind::ExpansionVisits`/`BudgetKind::ExpansionBytes` variants. The
  policy tables cover the schemes whose spelling can expand: 4M visits and
  1 MiB for Rust, 131,072 visits and 1 MiB for Swift, 262,144 visits and 1 MiB
  for Itanium/MSVC, 65,536 visits and 1 MiB for the legacy C++ dialects, and
  262,144 visits and 262,144 bytes for D, with 16M (1M for D) visits and
  64 MiB as the hard ceilings. The linear backends have no expansion gate and
  report `None`. These gates bound the expansion of an accepted AST at parse
  time; they are never printer caps, and the Rust, Swift, C++ and D parsers
  enforce them.

### Changed

- D counts back-reference expansions on both symbol and type paths and
  rejects a parse that exceeds the resolved expansion visit/byte gates;
  the printer itself has no output cap.
- Linear printers (Ada, Fortran, Delphi, OCaml, Go, ObjC, Kotlin/Native,
  JNI) and the D printer no longer enforce fixed output caps. `Display` is
  total for parser-built ASTs except the sink; `parse::LimitedWriter` is
  now test-only.
- C++ parsing enforces the expansion gates in the demangler: retained ASTs
  must fit the resolved `expansion_visits`/`expansion_bytes`, and the
  Itanium, legacy and MSVC printers no longer enforce depth, visit or
  output caps. `Display` is total for parser-built ASTs except the sink.
- Swift parsing now enforces the parse-time expansion gates: a shared-DAG tree
  is rejected when its expanded visits or bounded rendered bytes exceed the
  resolved `expansion_visits`/`expansion_bytes`, and the printer keeps no
  depth, visit or output cap, so `Display` is total for a parser-built AST
  except for the sink. A fixed parser-side print-depth ceiling of 128 keeps
  the recursive printer stack-safe, and `IndexSubset` text is charged at the
  expansion its printer writes so the byte estimate stays an upper bound.
- Rust parsing now enforces the parse-time expansion gates: a v0 shared-DAG
  tree is rejected when its expanded visits or bounded rendered bytes exceed
  the resolved `expansion_visits`/`expansion_bytes`, and the printer keeps no
  depth or output cap. A fixed parser-side print-depth ceiling of 512 keeps
  the recursive printer stack-safe.
- **BREAKING**: every `Budget` field and builder is `usize`
  (`Option<usize>` for the fields) instead of mixing `usize` and `u32`, so one
  type spells one dimension. Backends convert to their internal counter types
  at the parser boundary.
- Selection returns a candidate's budget error instead of
  `NoMatchingScheme`, so a capacity failure is not treated as a selection
  failure.
- JNI stores its array-dimension count in a `u16` and uses the generic 1024
  depth cap; the previous 255 representation-specific cap is gone.
- Detection prefilters now try the configured parse limits first. If that parse
  fails, classification falls back to the compiled-in defaults and the dispatch
  re-parses retained trees under the configured budget, so a tightened budget
  rejects instead of silently returning plain C.
- D parsing honors the runtime budget: `d::parse` keeps the compiled-in limits,
  while `Demangler` resolves the configured budget for `Scheme::D` and passes
  the clamped input, depth, work and node limits to the parser, including the
  tree detection retains. The parser clamps every dimension to its hard ceiling
  (16 MiB input, depth 1024, 64M work units, 1M nodes) and counts nodes against
  the resolved allowance. Input, work and nodes are independent, so raising the
  input alone keeps the default work and node allowances. Printing is not a
  budget: the printer walks each owned node once and has no output cap.
  Detection stays default-only.
- Clarify that the parse-time print envelope applies only to shared-DAG printers
  (Rust, Swift, C++). Linear printers visit each owned node once, so parser
  input/depth/work/node budgets already bound their output; they need no AST
  print counters.
- Go, Objective-C, Kotlin/Native and JNI parsing honors the runtime budget:
  `go::parse`, `objc::parse`, `kotlin_native::parse` and `jni::parse` keep the
  compiled-in limits, while `Demangler` resolves the configured budget for
  `Go`, `ObjC`, `KotlinNative` and `Jni` and passes the clamped input, depth and
  work limits to the parser, including the JNI tree detection retains. Go's
  balanced-group scan uses a closer stack sized for the hard depth ceiling and
  enforces the resolved depth; Objective-C's bracket scan and JNI's array
  dimensions do the same. Input and work stay independent, so raising the input
  alone keeps the default work allowance. Their printers are linear, so no print
  counter was added.
- Lower the JNI depth hard cap to 255: a parsed `jni::Type` stores its
  array-dimension count in a `u8`, so a deeper budget could not change the tree.
  Raising the depth budget for `Scheme::Jni` therefore clamps to 255 instead of
  the generic 1024.
- **BREAKING**: `Demangler` is now a struct with a private `Budget` field instead
  of a unit struct; `Demangler::new()`/`Default` keep the old behavior.
- Ada/GNAT, Fortran, Delphi/Object Pascal and OCaml parsing honors the runtime
  budget: `parse` keeps the compiled-in limits, while `Demangler` resolves the
  configured budget for `Ada`, `Fortran`, `Delphi` or `OCaml` and passes the
  clamped input and work limits to the parser, including the trees detection
  retains for Delphi and OCaml. These grammars are flat, so the schemes have no
  depth or node dimension to configure; input and work are clamped to 16 MiB and
  64M units and stay independent, so raising the input alone keeps the default
  work allowance.
- C++ parsing honors the runtime budget: `cpp::parse` and `cpp::parse_msvc`
  keep the compiled-in limits, while `Demangler` resolves the configured budget
  for `CppItanium`, `CppMsvc` and each legacy dialect and passes the clamped
  input, depth, work and node limits to the parser. The work allowance stays
  derived from the symbol length and capped by the configured work budget; the
  fixed print envelope is unchanged. Detection is still default-only and a
  configured budget re-parses a retained tree. Hard caps: 16 MiB input,
  Itanium/MSVC depth 256, legacy depth 64, 64M work and 1M nodes.
- Swift parsing honors the runtime budget: `swift::parse` keeps the compiled-in
  limits, while `Demangler` resolves the configured budget for `SwiftStable` or
  `SwiftLegacy` and passes the clamped input, depth, work and node limits to
  both grammars, including the retained legacy `_T` tree. The parser clamps
  every dimension to its hard ceiling (16 MiB input, depth 128, 64M work units,
  1M nodes), so the printer's fixed envelope is never widened. Detection is
  still default-only: a configured budget re-parses a retained legacy tree
  rather than reusing one parsed with the defaults.
- Rust parsing honors the runtime budget: `rust::parse` keeps the compiled-in
  limits, while `Demangler` resolves the configured budget for `RustLegacy` or
  `RustV0` and passes the clamped input, depth, work and node limits to both
  encodings, including the detected path's C++ to Rust fallback. The parser
  keeps hard ceilings for the memory-derived v0 input cap, the recursion stack
  and the node/work allocations, and the printer keeps its fixed internal caps.
- Document the threat model: only parser-produced ASTs are covered by the
  untrusted-input and printability guarantees; caller-built ASTs and failing
  sinks are outside it.
- Rust and Swift parsing now rejects a tree over the printer envelope (expanded
  visits, depth and Swift output bytes) computed during construction, instead
  of rendering a validation pass. Rust's print-visit cap rises to 4M, and the
  autodiff-subset and lifetime-binder checks reject unprintable trees earlier.
- C++ demangling retains the parsed AST and checks a conservative print
  envelope (2x depth, 4x visits, 256 bytes/node) instead of eagerly rendering
  into a `String`; C++ `Demangled` values are `!Send + !Sync` again.
- The C++ print envelope's depth bound is the dialect's hard parse-depth
  ceiling from `Scheme::hard_budget` times its declarator factor (2 for MSVC),
  instead of literal 256/512/64.
- HP aCC `A` literal payloads are demangled by a nested parse that continues
  the outer parse's depth, work and node counters and charges its render to
  the expansion visit and byte budgets, instead of a fresh budget per level
  capped at eight levels. A budget failure inside a payload rejects the
  symbol rather than printing the payload undemangled. `cpp::ParseErrorKind`
  gains `TooManyExpansionVisits` and `TooManyExpansionBytes`.
- Document that retained-AST backends re-render on every `Display` call and
  that rendered-text caches are caller-owned.
- Add corpus-wide differential envelope tests: every accepted Rust, Swift and
  C++ corpus symbol must render within the parser's modelled visits, depth and
  bytes. Rust's print-depth cap rises to 1024 because back-reference expansion
  can descend further than the structural tree.

- Skip the Itanium lambda-parameter pre-pass when the parsed C++ tree contains
  no lambda, and remove per-construct `Vec`/`String` allocations from the
  Itanium and MSVC printers.
- Reject non-dotted Go names before the full shape scan, hoist the encoding
  escape buffer, and copy non-escape runs in bulk.
- Decode already-owned C++/D text directly and render single-`Display` ASTs
  without `format_args!` in `Demangled`.
- Reuse the D, OCaml, Delphi and JNI trees parsed during detection instead of
  parsing those symbols a second time.

- Document that no backend keeps a fixed limit outside the `Budget`
  dimensions, and that only the mangling spec, not a reference demangler's
  implementation limits, constrains the grammar.

- Enable clippy `pedantic` and fix its warnings; justified exceptions use
  local `#[expect(..., reason)]`, and the remaining source `#[allow]`s carry
  reasons and cover only feature-dependent lints. `Cargo.toml` no longer
  allows any clippy lint: `redundant_pub_crate`, `missing_const_for_fn`,
  `similar_names` and `match_same_arms` are fixed or locally expected, and
  `single_call_fn`/`enum_variant_names` never fired. Every feature builds
  clippy-clean on its own. The Itanium and legacy C++ printers and parts of
  the Swift parser take node handles by reference.

### Removed

- Remove the dead C++ printer caps (`PrintLimits`, the Itanium/legacy/MSVC
  print depth and visit constants and the 1 MiB `MAX_OUTPUT_BYTES`); printing
  is bounded by the parse-time envelope alone.
- Remove the C++ fixed backend limits that were not budget dimensions: the
  64 GNU v2 split points (every attempt already charges the work budget), the
  128-byte MSVC narrow string literal cap (each byte is charged to work) and
  the depth-16 cutoff on MSVC back-reference keys.

- **BREAKING**: remove the fixed list caps of the linear backends and their
  public constants: `go::MAX_SUFFIXES`, `ada::MAX_SEGMENTS`,
  `ocaml::MAX_PATH_SEGMENTS`, `delphi::MAX_SCOPES`, `delphi::MAX_PARAMS`,
  `kotlin_native::MAX_PATH_SEGMENTS`, `kotlin_native::MAX_PARAMETERS`,
  `kotlin_native::MAX_TYPE_ARGUMENTS` and `kotlin_native::MAX_TYPE_PARAMETERS`.
  Every list element consumes input, so the input and work budgets bound them.
- **BREAKING**: remove the unenforced `MAX_OUTPUT_LEN` constants of the Ada,
  Fortran, Delphi, OCaml, Kotlin/Native, JNI, Objective-C and D backends, and
  `fortran::MAX_DEPTH`. Printing is not a budget; the module docs state each
  printer's linear output bound instead.

- Remove the Swift fixed limits that were not budget dimensions: the 128-entry
  operand stack, the 8/16/32/64-element caps on tuples, labels, type lists,
  conventions, requirements and specialization lists, the 8 generic depth
  groups and 16 requirements, the legacy 1024-entry substitution table, the
  4096 builtin width/count bound and the printer's `...` elision after 128
  generic parameters. Every element is a charged node or consumes input, and a
  generic parameter count is charged to the expansion budgets at parse time.
  The Swift corpus gates list the three pairs where this differs from
  `swift-demangle` as documented divergences.

- Remove the eight-prefix cap on nested PE/COFF `__imp_` imports; every
  prefix consumes input, so the walk stays linear and allocation-free.
- Remove the 128-char Rust v0 punycode identifier cap: a longer name decodes
  in full instead of printing the `punycode{...}` fallback, and every insertion
  shift is charged to the work budget. Rust's print-depth gate is defined as
  its hard parse-depth ceiling instead of a separate constant.

### Fixed

- Swift `relabel`/`append` node copies are charged to the node budget, and a
  stable-grammar parse that exhausts it is rejected even when no later builder
  checks the flag.
- `swift::print` applies the expansion gates like `swift::parse`; it parsed
  without them, so only the removed printer elision bounded its output.
- Swift repeated substitutions (`A<count>a`) charge their repeat count to the
  work budget; the removed operand stack cap was the only bound on them.
- Swift legacy specialization payloads are re-demangled within the remaining
  depth, work, node and expansion budgets of the containing parse, and a
  payload that runs out of budget rejects the symbol, instead of a fresh
  budget per payload capped at 256 bytes and four levels.
- Kotlin/Native detection no longer drops names longer than the default
  64 KiB input cap to plain text: the prefix test has no length gate, so a
  raised `input_bytes` budget demangles them and a too-small one reports a
  Kotlin/Native budget error.
- MSVC back-reference deduplication now compares names of any depth; the
  depth-16 cutoff silently kept duplicate deep names in the table. Rendering a
  key charges its expanded node visits to the work budget.

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

- Size the linear flat-backend output backstops from the hard input ceiling
  (Ada, Fortran, Delphi, OCaml), so raising the input budget cannot make a
  parser-built AST fail in `Display`.
- Drop the unused depth dimension from Ada, Fortran, Delphi and OCaml budgets;
  the flat parsers never enter a cursor depth, so an override had no effect.
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
