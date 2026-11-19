# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Fixed

- Swift bound generic functions/constructors print bound arguments before the
  parameter list (`testit<A1, B1>(…)`), and impl pattern substitutions print
  `@substituted <…> … for <…>` in reference position.
- Added protocol self-conformance witness thunk printing and corrected the
  autodiff subset "to type" placement.


- Swift anonymous runtime contexts (`XZ`) print as
  `parent.(unknown context at name)` and participate in context resolution.


- Macro expansion contexts are resolved through the context fallback, and
  attached macro expansions get their own node/role wording instead of being
  mistaken for unique-name expansions.


- Swift bound generics whose context is an extension now bind the extended
  type and keep the extension signature in reference order.


- Swift bound-generic rebinding now preserves all children of nested
  function/constructor contexts (local `Myalias`-style symbols demangle again).


- Swift robustness: `Arena::text` no longer panics when a non-text node is
  printed, and shared substitution nodes are cloned whenever they are already
  linked so sibling lists are not truncated (silent wrong output).
- `A_` (substitution index 27) is accepted after the repeat-count grammar fix.

### Added

- Swift Punycode identifiers are decoded in-tree (no allocation), and macro
  expansion locations participate in context resolution.


- Swift function parameter labels for stable (`$s`/`$S`) manglings: labels are
  popped into a `LabelList` after the function type and printed with the
  argument tuple. `_T` old-style manglings keep their inline labels.
- Swift type annotations `Yi` (`isolated`), `Yt` (`_const`) and `Yg`
  (`@const`).
- Swift thunk markers are modelled as top-level function attributes: partial
  apply forwarders, async/coro function pointers, distributed thunks and
  accessors, dynamically replaceable functions, merged functions, outlined
  variables/bridged methods and autodiff resume functions. Reabstraction
  thunks parse their signature and parameter types.
- Swift protocol conformances (including runtime records, witness tables and
  witness thunks) and associated-type/conformance descriptors.
- Swift local, private and related-entity declaration names, numeric
  discriminators and operator identifiers.
- Generic specializations with dropped arguments, and the reference printer's
  quirk of leaving closure-propagation brackets open.
- Swift declaration entities: allocating initializers, constructors with
  parameter labels, property-wrapper backing/field/inits, generic type
  parameter declarations, macros, and closure indices printed 1-based.
- Swift builtin types `Builtin.Borrow<T>`, `Builtin.FixedArray<N, T>` and
  integer literal types (`$n...`).
- The corpus test strips `swift-demangle -classify` prefixes (`{T:...}`,
  `{C}`, `{N}`) before comparing, since classification is not demangling.
- Swift autodiff thunks (`TJ...`) and differentiability witnesses (`WJ...`),
  including index subsets and reordering/subset-parameter variants.
- Swift opaque return types (`Qr`/`QR`/`QO`), macro expansions (freestanding,
  unique-name and source-location forms) and inverse generics requirements
  (`Ri`/`Rj`/`RI`/`RJ`).
- Swift protocol list types now use the protocol fallback, so bare protocol
  operands (`P_p`) demangle.
- Swift generic signatures print their `where` clauses via
  `DependentGenericType`, subscripts print accessors and result types, and
  constructors keep private discriminators.
- Swift impl function types print `@caller_isolated`, sending results and
  parameter markers like the reference printer.
- Fixed Swift substitution numbering: standard-library substitutions no longer
  consume a substitution slot, which realigns every following `A...`
  reference.
- Full Swift generic-requirement table (`R`): associated/compound subjects,
  substitutions, layouts, inverse and Self requirements, plus compound
  associated types and constrained existentials.
- Fixed impl function types consuming their error/yield convention twice,
  which unblocked most reabstraction thunks.
- Swift functions no longer lose their type when a generic context is bound
  (correct reference word splitting, associated-type substitutions, protocol,
  concrete and dependent conformances, opaque types and retroactive
  conformances).
- Swift generic signatures print separate angle groups per depth (`<A><B…>`)
  and constructors print their generic signature before the parameter list.
- Swift bound generics with no arguments are refused, matching the reference
  demangler, and key-path specialization payloads keep their reference order.
- Swift multi-kind function-specialization params (`p…`), differentiable
  function propagation, cached decl-name contexts (constructors, allocators)
  and non-function macro entities (`name : Type`).
- Swift key-path thunk helpers (`TK`/`Tk`, serialized form) and SIL box
  layouts (`Xx…` with generic signature and fields), plus impl
  pattern/invocation substitutions.
- Swift nominal entities whose context itself prints a type (subscripts,
  functions, variables, accessors) use the reference suffix form
  (`name in context`) instead of a dotted prefix.
- In-tree pure-Rust Swift demangler (`src/swift/`): postfix operator parser
  over a fixed-capacity arena, `no_std`, no `alloc`, printing into any
  `core::fmt::Write` sink. Supports `$s`, `$S` and `_T0` prefixes; builtins,
  identifiers with word substitutions, substitutions, nominal types, tuples,
  functions/accessors, standard substitutions and sugar, bound generics,
  metatypes, generic signatures, impl (SIL) function types (`I...`) with
  callee/parameter/result conventions, and common metadata/witness/thunk
  wrappers.
- Corpus test against the vendored official Swift manglings
  (`tests/swift_corpus.rs`, skips when `refs/` is absent).
- `refs/` reference material for implementers, excluded from the published
  crate (`exclude = ["refs"]`): Swift ABI mangling grammar and the official
  demangle corpus, pinned to upstream commit `fa6c2608`.
- `no_std` core with **no `alloc` requirement**: demangled text is exposed via
  `Demangled` / `core::fmt::Display`, so callers pick their own sink.
- Swift top-level symbol assembly: attributes (`@objc`/`@nonobjc`, dynamic,
  override), generic specializations, vtable thunks and coroutine continuation
  prototypes; duplicate substitution references are cloned in the arena.
- Function signature specializations (`Tf...`): constant/closure propagation,
  same-as-argument, option-set flags, pass attributes and returns.
- Multi-level bound generics: argument lists now attach to the nested nominal
  context (`Outer<A>.Foo<Int>`), matching the reference binding algorithm.
- Fixed multi-substitution pushes (the terminating substitution was pushed
  twice, duplicating symbols).
- Parser work budget and list guards: pathological or partial manglings fail
  cleanly instead of looping.
- `Error` type with `Unsupported` / `InvalidSymbol` variants.
- Basic CLI (`demangle <symbol>`, stdin batch).
- `Scheme::detect` fast prefix detection and `Scheme::language` / `Language::as_str`.
- Feature flags: `rust` (`no_std`, no `alloc`) and `cpp` (needs `alloc`/`std`).

### Changed

- The Swift corpus test now also bounds near-misses (parsed but wrong
  output), so silent wrong-output regressions cannot hide behind the pass
  floor.


- Swift substitution nodes are copied instead of mutated when promoted to a
  module, so shared substitutions no longer corrupt sibling links.
- Replaced the allocating `DemangledResult` with the `Display`-based `Demangled`.
- Swift detection now recognises only official prefixes (`$s`, `$S`, `_T0`)
  instead of any `$`-prefixed name, and rejects symbolic references.

### Known limitations

- Swift: generic specializations and most `T...` thunks, multi-level bound
  generics on member types, `L` local discriminators, punycode identifiers,
  packs and macros are not demangled yet; legacy `_Tt`/`_TF`/`_TT`/`_TW`
  prefixes are not detected.
- C++ backends moved behind the `cpp` feature and flagged for a future rewrite.

### Removed

- `DemangledResult` and its owned `String` output.

## [0.1.0] - 2026-11-17

### Added

- Initial crate scaffold with `Demangler`, `Scheme`, and `Language` types.
- C, C++, Rust, Swift, and Go scheme detection stubs.

[Unreleased]: https://github.com/polazarus/demangle/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/polazarus/demangle/releases/tag/v0.1.0
