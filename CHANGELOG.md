# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Raw-byte parsing in the Go, Objective-C and Kotlin/Native backends. Each
  parser matches its grammar on `&[u8]` through the shared `crate::parse`
  cursor and work budget, so a symbol name no longer has to be UTF-8, and each
  backend gains a `pub(crate)` `parse_bytes` entry point (`go::parse_bytes`,
  `objc::parse_bytes`, `kotlin_native::parse_bytes`) alongside its existing
  `parse(&str)` wrapper. Detection gained byte entry points
  (`go::is_go_name_bytes`, `objc::is_objc_bytes`,
  `kotlin_native::is_kotlin_native_bytes`).
- Document unsupported language/ABI schemes, platform symbol decorations and
  non-UTF-8 symbol-name behavior in `docs/unsupported-schemes.md`.
- `Demangler::demangle_bytes` and `Scheme::detect_bytes` accept raw symbol
  byte strings. Valid UTF-8 is demangled normally; invalid UTF-8 is rendered
  as escaped plain text (`\xNN`) with `Scheme::Unknown`.
- Expanded Kotlin/Native corpora across compiler releases: 2.0.21, 2.1.21,
  2.2.21, 2.3.21, 2.4.0 and 2.4.21 (8,917 measured symbols total). Parser
  support now handles older `-trampoline` and local `-N:M` suffixes.
- Kotlin/Native symbol support: `Scheme::KotlinNative`,
  `Language::KotlinNative` and a default-enabled `kotlin-native` feature. The
  in-tree `src/kotlin_native/` backend parses the compiler's `kfun:`,
  `ktype:`, `kclass:`, `kproperty:`, `kobj:`, `kfield:`, `ktypealias:`,
  `kpackage:`, `knamespace:` and `kvar:` linker symbols (plus Mach-O
  leading-underscore variants) and prints a documented Kotlin-like debugger
  spelling (`kfun:com.example.Foo#bar(kotlin.Int){}` ->
  `com.example.Foo.bar(kotlin.Int)`). It handles `#` members and private `.`
  separators, parameter and type-parameter lists, upper bounds, type
  arguments, variance, nullable/star/vararg/flexible/dynamic types, extension
  receivers (`@`/`__at__`), `#static`, `#suspend`, `#companion`, `#internal`,
  `-impl`, `$` local indexes, compiler bridges and `%` platform suffixes.
  Detection claims only the known `k*:` prefixes, before Go and plain C, and
  does not steal C++/ObjC/Swift/Rust/Go. Parsing is bounded by input, path
  segment, depth, work, parameter, type-argument, type-parameter and output
  budgets.
- Kotlin/Native corpus gate (`tests/kotlin_native_corpus.rs`) over 8,917
  linker symbols from fixtures built with Kotlin/Native 2.0.21 through 2.4.21
  and 33
  hand-authored cases for prefixes and markers the Linux executable does not
  emit, vendored in `refs/kotlin-native/corpus/` with release URL, SHA256,
  Apache-2.0 license, extraction script and provenance in
  `refs/kotlin-native.md`. The test skips when `refs/` is absent.

- Expanded Objective-C corpora: added three more generated classes,
  categories, protocols, properties, ivars and blocks, growing Apple metadata
  from 32 to 88 names, method display forms from 6 to 26, and GNUstep records
  from 8 to 21. Added 18,931 real metadata names from the Apache-2.0
  Mattermost 2.46.0 iOS IPA. Hardened metadata name bytes, rejected terminal
  control bytes in block descriptors, stripped version decorations from dotted
  metadata, and added exact-output spot checks.
- Objective-C symbol support: `Scheme::ObjC`, `Language::ObjectiveC` and an
  `objc` feature (enabled by default). The in-tree `src/objc/` backend
  recognises Apple/Mach-O metadata (`_OBJC_CLASS_$_Foo`,
  `__OBJC_$_INSTANCE_METHODS_Foo`, categories, protocols, protocol refs,
  property/ivar lists, class refs, block descriptors), GNUstep/GNU runtime
  metadata (`__objc_class_name_Foo`, `__objc_ivar_offset_*`, GNUstep 2.0
  `_OBJC_CLASS_*`) and method display forms (`-[Class selector:]`,
  `+[Class selector:]`). Output follows the project-defined canonical spelling
  (`Objective-C class Foo`, `Objective-C category Foo (Bar)`); names are
  preserved verbatim and unknown metadata prefixes are rejected. Detection
  runs before Go and the legacy C++ dialects so dotted ivars are not stolen,
  while Objective-C++ `_Z` names stay C++. Parsing is bounded by input,
  bracket depth and work budgets.
- Objective-C corpus gate (`tests/objc_corpus.rs`) over 88 Apple/Mach-O
  metadata names and 26 method display forms emitted by Clang 19.1.7, 21
  GNUstep/GNU-runtime metadata names, 51 metadata records from Debian GCC 14's
  `libobjc.a`, and 17 rejected runtime helpers, local labels and truncated
  prefixes, vendored in
  `refs/objc/corpus/` with provenance in `refs/objc.md`. The test skips when
  `refs/` is absent.

- Itanium C++ expression and type coverage: the parser now handles unresolved
  names (`sr`, `srN`, `gs`, `on`, `dn`), function-parameter references (`fp`,
  `fpT`, `fL...p`), member access (`dt`, `pt`, `ds`, `pm`), new/delete with
  placement and initializer lists, subobject (`so`) and designated
  initializer (`di`/`dx`/`dX`) expressions, fold expressions, parenthesized
  callees (`cp`), `sP` pack sizes, vendor extended expressions (`u
  <source-name> ... E`), local-entity literals (`LZ ... E`) and lambda
  literals. Types gained vector types (`Dv`), constrained placeholders
  (`Dk`/`DK`), elaborated type specifiers (`Ts`/`Tu`/`Te`), vendor extended
  qualifiers (`U`, including Objective-C protocols), dependent `_BitInt`
  sizes and function-type exception specifications (`Do`/`DO`/`Dw`/`Dx`).
  Names gained structured bindings (`DC`), explicit object parameters (`NH`),
  module initializers (`GI`), ABI tags on well-known substitutions and
  `Ua9enable_if` function attributes; `GR` sequence ids, `Ub` block literals
  and `___Z..._block_invoke`/`.suffix` symbols parse correctly. The Itanium
  text corpus records 29,893 exact, zero wrong and zero printer refusals.

- Cross-backend robustness and detection-normalization tests
  (`tests/robustness.rs`): deterministic token/character fuzzing, truncation,
  budget checks and ambiguous-prefix detection for every feature-gated
  backend.
- Legacy Swift (`_T`) demangling: `swift::is_swift`/`Scheme::detect` now
  recognise the pre-stable `_T` prefix, and `src/swift/legacy.rs` ports the
  reference old demangler as a bounded recursive-descent parser that builds the
  shared node graph. It covers legacy types (`_Tt`), function and accessor
  entities (`_TF`), metadata (`_TM`), witness tables (`_TW`), value witnesses
  (`_Tw`), thunks and specializations (`_TT`), variables, subscripts, partial
  apply forwarders and extensions, mapping old-only productions onto existing
  node kinds where the printer already renders them. Unimplemented forms are
  rejected rather than printed wrongly, and no runtime dependency was added.
- `_$s`/`_$S` (Mach-O underscore) prefixes are recognised alongside `$s`/`$S`.
- Over-long legacy Swift specialization payloads are rejected before they can
  exceed the legacy parser/printer budgets.
- Swift legacy corpus gate: `tests/swift_corpus.rs` now also runs a dedicated
  pre-stable `_T` gate over `refs/swift/manglings.txt`; all 248 legacy pairs
  demangle exactly with zero wrong outputs.

- Go symbol support: `Scheme::Go` detection and an in-tree parser/printer
  (`src/go/`) for package, function, method, closure, wrapper, generic shape,
  `type:`, `go:` and compiler-generated names. `%XX` package escapes are
  decoded, `·` is normalised to `.`, and `.abi0`/`.abiinternal`/`.abiwrap`
  markers are reported by `go::Ast::abi` and stripped from the printed name.
  Parsing is bounded by input, depth, work and suffix budgets. No runtime
  dependency was added.
- Go corpus gate (`tests/go_corpus.rs`) over 3,326 names from the ELF symbol
  table of a generated Go 1.24.4 test binary and 79 names from a generated
  compiler object file, vendored in `refs/go/corpus/` with provenance in
  `refs/go.md`. The test checks detection, parsing, escape/ABI removal and
  idempotence, and skips when `refs/` is absent.
- Go archive corpus gate (`tests/go_corpus.rs`): `refs/go/corpus/go1.24.4-archive.txt`
  holds 12,626 names from the `go list -export` archives of 14 standard library
  packages and the generated corpus packages, adding static temporaries, jump
  tables, equality/hash closures, package-qualified promoted methods and the
  object-file `<N>` symbol version. The gate pins floors and per-category
  coverage for static temps, jump tables, `eqfunc`/`hashfunc`, qualified
  methods and object versions.
- Legacy C++ support, starting with the GNU v2 (old GNU) dialect:
  `cpp::Dialect::GnuV2`, a new `src/cpp/legacy.rs` parser and printer over the
  shared C++ AST, and `cpp::parse` dispatch that selects the dialect only when
  the legacy parser consumes the whole symbol.
- Legacy GNU v2 grammar: `__`-separated function names, `Q<n>`-qualified and
  nested names, templates, operators and conversion operators, constructors and
  destructors, pointers/references/rvalue references, cv qualifiers, arrays,
  function types, member-function pointers, `T`/`N` back-references, and the
  `__vt_`/`_vt$`/`__ti`/`__tf`/`__thunk_`/`_<name>$<variable>` special names,
  with explicit input, depth, node, work and print budgets.
- `Scheme::detect` recognises legacy GNU v2 names, so valid legacy C++ is no
  longer classified as plain C, while names that only look legacy (such as
  `foo__bar`) stay plain C.
- `tests/cpp_legacy_corpus.rs` gates the vendored `refs/cpp/legacy/gnu-v2.txt`
  corpus with byte-exact output and skips when `refs/` is absent.
- Legacy C++ ARM and EDG dialects: `cpp::Dialect::Arm` and `cpp::Dialect::Edg`,
  implemented in `src/cpp/legacy.rs` over the same AST and printer as GNU v2.
  Supported productions: `__ct__`/`__dt__` constructors and destructors,
  `__vtbl__` virtual tables, `__sti__`/`__std__` static initialiser and
  destructor wrappers (`cpp::Kind::LegacySpecial`), the `__pt__` (ARM) and
  `__tm__`/`__ps__` (EDG) parameterized-type form, `F`-marked argument lists
  with one-based `T`/`N` back-references, ARM data names, and EDG's trailing
  return type. Unsupported cfront productions (HP template extensions, EDG
  recursive qualified-name components, local-variable and thunk forms) are
  rejected rather than guessed at.
- Reserved ARM/EDG special-name markers (`__ct__`, `__dt__`, `__vtbl__`,
  `__sti__`, `__std__`, `__pt__`, `__tm__`, `__ps__`) are tried as ARM/EDG
  candidates before GNU v2 and fall back to GNU v2 when that parse is not
  complete, so `__ct__3fooFv` demangles as `foo::foo(void)` instead of
  `foo::__ct(void)`. Every other legacy name keeps GNU v2 precedence, and no
  user-definable identifier is affected.
- `tests/cpp_legacy_corpus.rs` also gates `refs/cpp/legacy/arm.txt` and
  `refs/cpp/legacy/edg.txt`, asserting the reported dialect as well as the
  printed text.
- GNU v2 large corpus: `refs/cpp/legacy/gnu-v2-large.txt` holds a deterministic
  1,523-pair sample derived from the permissively licensed `gnuv2_demangle`
  0.6.0 snapshots of real GNU v2 symbol dumps, with provenance, source hashes
  and exclusions in `refs/cpp.md`. `tests/cpp_legacy_corpus.rs` gates it with
  byte-exact output and skips when `refs/` is absent.

### Changed

- Rust parsing reads the symbol as bytes through the shared `parse` cursor and
  combinators. Length prefixes and backrefs are byte offsets, so an identifier
  or `s` string literal may hold bytes that are not UTF-8; the printer escapes
  them as `\xNN` instead of rejecting the symbol. Valid UTF-8 and ASCII keep
  their previous output, and every `rustc-demangle`-accepted input still
  demangles identically. **Breaking:** `rust::Node::text` and the `Fn` ABI
  payload (`NodeData::Function::abi`) now expose raw `&[u8]` / `Cow<[u8]>`
  instead of `&str`. A crate-internal `rust::parse_bytes` entry point backs the
  public `rust::parse`.
- Swift parsing reads `&[u8]` through the shared `crate::parse` cursor, work
  and depth budgets, and a `pub(crate)` byte entry point (`swift::parse_bytes`,
  `swift::print_bytes`) that `swift::parse`/`swift::print` now wrap. Identifier
  bytes that are not valid UTF-8 are kept and escaped as `\xNN` in node text
  instead of rejecting the symbol; compiler payloads (the unmangled suffix and
  legacy payload text) keep their bytes verbatim. Valid UTF-8 and ASCII output
  is byte-identical, including for backslashes. Swift also gains a 64 KiB input
  cap; depth (96), work (200,000), node (524,288) and print (1 MiB) budgets are
  unchanged.
- Go, Objective-C and Kotlin/Native text is rendered from symbol bytes: valid
  UTF-8 (Go's `·`, Kotlin and Objective-C Unicode names) prints verbatim, and
  bytes that are not valid UTF-8 print as `\xNN`, including bytes produced by a
  Go `%XX` escape, which no longer falls back to the raw spelling. A backslash
  in identifier text prints as `\\` so the spelling stays reversible; opaque
  compiler payloads (`go:string.`, `type:.namedata.`, block descriptors, `%`
  platform suffixes) keep their bytes as written. Objective-C names accept
  bytes from `0x80` up, so a Unicode class, category or selector name is no
  longer rejected. Because escaping costs four bytes per invalid input byte,
  `objc::MAX_OUTPUT_LEN` is now four times the input ceiling plus the fixed
  prefix overhead.
- The Objective-C, Go and Kotlin/Native parsers take their work budget from the
  shared `crate::parse::Cursor` instead of a per-backend counter, and Kotlin's
  hand-rolled signature cursor is replaced by the shared one. `alloc` is now
  enabled by the `objc` feature alone, so `--no-default-features --features
  objc` builds.
- Swift generic parameter names follow the reference for indices at or above
  26 (`AB`, `BB`, ...), generic signatures print `...` past 128 parameters,
  and layout requirements print the reference names (`_Trivial`,
  `_TrivialAtMost`, ...) with their legacy size/alignment.
- Swift function-signature specialization option sets print combined flags
  (`Dead and Owned To Guaranteed and Exploded`) instead of a single flag.
- Swift builtin type sizes and vector element counts are bounded at 4096, as
  the reference demangler does, so a malformed width cannot become work.
- Add language and C++ compiler features for lightweight builds: `rust`,
  `swift`, `go`, `cpp-itanium`, `cpp-msvc` and `cpp-gnu-v2`, with `cpp` as the
  C++ umbrella. The default feature set keeps every backend enabled; with no
  features the crate is a dependency-free C/plain-name pass-through.
- **Breaking:** C++ demangling now uses the in-tree AST, printer and feature
  set. The `cpp` feature was removed, `cpp_demangle` and `msvc-demangler` are
  dev-only test oracles, and the crate has no runtime dependencies.
- **Breaking:** Rust demangling now uses the in-tree parser and printer;
  `rustc-demangle` is a dev-dependency used only as the test oracle. The
  `rust` feature was removed; all schemes are now always available.
- Swift nodes now own their children as an ordered slice of shared `Rc`
  handles instead of a sibling-linked list. Substitutions reuse nodes instead
  of deep-cloning them, parsing is bounded by a node budget, and printing is
  bounded by node-visit and output-byte budgets in addition to depth.
- Swift node storage is category-based: text, numeric, global, accessor,
  impl, macro-role, autodiff, specialization, function-spec, key-path,
  layout-requirement, deinitializer, indexed-entity and attribute payloads
  are typed enum variants. The branch variant is structural-only (kind plus
  children), so no node carries generic `flags`/`value`/`len` fields.
- **Breaking:** `Demangled` is `!Send + !Sync` because Rust, Swift and C++
  symbols retain `Rc`-owned node graphs. Demangle again on the target thread
  instead of moving a value across threads.
- Printing borrows node text instead of cloning reconstructed `Cow` text per
  write, and debug builds assert that dedicated node categories are not built
  through the structural branch constructor.

### Fixed

- Objective-C metadata now strips `@`/`@@` version decorations on dotted
  ivar records, rejects terminal control bytes in block descriptors, and
  restricts metadata names to identifier bytes plus `.`. Added exact-output
  spot checks and ObjC coverage to the shared robustness harness.
- Go object-file names: parse the `<N>` symbol version that `go tool nm`
  appends to static symbols (`go::Ast::version`), `..stmp_N` static
  temporaries, `.jumpN` jump tables, `type:.eqfunc`/`type:.hashfunc` equality
  and hash closures, and package-qualified promoted methods such as
  `(*Template).text/template/parse.action`. Names keep their printed form, so
  demangling remains idempotent.
- Itanium C++ text coverage now reaches 29,929/29,933 with zero wrong and zero
  printer refusals. Added friend names, C++20 requires-clauses and
  requires-expressions, pack subscripts, pointer-to-member conversions, module
  qualifier chains, allocations-token decoration and substitution-in-noninitial
  nested-name support. The only excluded inputs are four bare type manglings,
  which are not encodings.
- Itanium C++ text output no longer diverges from the reference demangler on
  the vendored corpus: the text gate now records 29,754 exact, zero wrong and
  zero printer refusals. Local-name discriminators no longer consume a
  following source-name length, CV-qualified function types take one
  substitution slot, module-name substitutions form a single entity, and
  conversion operators, inherited constructors, constrained parameters,
  argument packs and multi-level lambda parameters print correctly. Malformed
  self-referential template parameters print as empty instead of failing.
- Microsoft C++ demangling now matches `msvc-demangler` on all 59,248 real
  PyTorch exports. Template arguments join the type back-reference table, so a
  later parameter back-reference counts earlier template arguments, and a
  variable's trailing cv qualifiers apply to the pointed-to type instead of
  the pointer. Trailing characters after a complete encoding are ignored, as
  in `llvm-undname`, while RTTI type descriptors and typeinfo names still
  require exact input.
- `Scheme::detect` recognises every rustc v0 `_R`/`__R` tag (not only
  `_RN`), platform leading underscores on Itanium and rustc prefixes, and
  strips ThinLTO `.llvm.<hex>` and GNU `@`/`@@` version suffixes before
  classifying. Legacy rustc crate hashes are now detected as Rust rather than
  C++.
- C++ detection is gated by dialect: `_Z` requires `cpp-itanium`, `?`
  requires `cpp-msvc`, and legacy GNU v2 requires `cpp-gnu-v2`, so a disabled
  backend no longer claims symbols it cannot demangle.
- C++ demangling strips the same `.llvm.`/version decorations before parsing,
  so decorated Itanium names render instead of failing.
- Legacy C++ names that GNU v2 rejects but ARM or EDG accept (`foo__FiT1`,
  `foo__Fi_v`, `__sti__foo`, `foo__pt__2_i`) are now demangled instead of
  falling through as plain text. `foo__Fiv_` and `foo__Fi_` moved from the GNU
  v2 pass-through set to the EDG corpus for the same reason.
- C++ demangling now prints complete function signatures, return types and
  parameter types. The previous `itanium-demangle` backend printed only the
  name and template arguments, so `_Z1fPFivE` became `f` instead of
  `f(int (*)())`.
- Legacy Rust symbols with a trailing crate hash are routed to the Rust
  backend instead of being printed as C++ data names.
- v0 parser stall when a cached path was reached again as a type; cache hits
  now advance the cursor to the production's end.
- Swift generic signatures with more than eight depth groups or sixteen
  requirements are rejected instead of being printed truncated.
- Swift property descriptors (`MV`/`ZMV`), enum cases (`WC`), direct/indirect
  field offsets, subscript label lists, `ProtocolListWithAnyObject`, and
  protocol metatypes (`Error.Protocol`).
- Full extended-corpus coverage: 23,975/23,975 exact and 248/248 on the ABI
  corpus, with zero wrong outputs.


- Full Swift corpus coverage for recognized prefixes (`$s`/`$S`/`_T0`):
  248/248 symbols demangle exactly, with zero near-misses.
- Rebinding a bound generic context now preserves the original node flags
  (subscript accessors), variadic packs parse as `Pack{…}`, and impl function
  signatures print in the reference position.
- Legacy `_T` manglings with multiple leftover top-level nodes are refused
  like the reference demangler.


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
- Legacy GNU v2 parser/printer gaps found by the large corpus: function-type
  `_<return type>` suffixes, pointer spacing after an identifier ending in
  `_`, `true`/`false` for `bool` template values, `__<code>__<class>` method
  operators, the `G` class marker, and `_GLOBAL_$I$`/`_GLOBAL_$D$` (or
  `_GLOBAL_.I.`/`_GLOBAL_.D.`) constructor and destructor wrappers.

### Security

- Cap C++ and Rust parser input at 64 KiB before allocating the v0 offset
  cache, legacy element list or C++ node/substitution vectors.
- Charge every C++ and Rust production entry against the work budget,
  including v0 cache hits, so repeated or nested parsing cannot run unbounded.

### Added

- Real Microsoft C++ export corpus: 59,248 decorated names from the pinned
  PyTorch 2.5.1 Windows DLLs (`c10`, `fbgemm`, `torch_cpu`). All parse into the
  shared C++ AST, and the text is token-equivalent to the oracle for 59,232
  names; the 16 pinned differences are documented oracle/lit spelling cases.
- In-tree Itanium and Microsoft C++ printers over the shared C++ AST, with
  bounded printing and text corpus gates.
- Rust toolchain-derived symbol corpora: 6,958 legacy and v0 symbols from
  Rust 1.75.0 and 1.100.0 `libstd` and `librustc_driver`, checked against
  `rustc-demangle` in both display modes.
- In-tree pure-Rust Itanium C++ AST parser (`cpp::parse`) with an immutable,
  `Rc`-owned node graph covering special names, local names, ABI tags,
  expressions, lambdas and the common type and name grammar. Coverage is
  measured by `tests/cpp_corpus.rs`: 29,753/29,933 exact, zero wrong.
- In-tree pure-Rust Rust AST parser (`rust::parse`) for both legacy `_ZN` and
  v0 `_R` encodings, with one shared node vocabulary, strict acceptance and
  explicit budgets. Coverage is measured by `tests/rust_corpus.rs`: 115/115
  exact, zero wrong.
- Exposed a read-only Swift AST (`swift::Ast`, `swift::Node`, `swift::Kind`) over the existing retained `Rc` graph. Children are returned as owned views and the internal `RefCell` stays private.
- Vendored pinned Rust mangling references under `refs/rust/`: the
  `rustc-demangle` decoder and its test fixtures, rustc's legacy and v0
  encoders, RFC 2603 (v0 grammar), and a normalized test corpus (115
  expectation pairs plus 4 rejection cases) extracted from the pinned decoder
  test modules. Provenance, licenses and refresh commands are recorded in
  `refs/rust.md`.
- Vendored pinned Itanium and MSVC C++ demangler references from LLVM
  (`refs/cpp/`): the 29,933-pair libc++abi Itanium corpus, LLVM MSVC
  demangler lit tests, unit tests, and provenance/license notes.
- Extended Swift validation corpus from the official Swift 6.4.2 Linux
  toolchain: 23,975 exported `$s` symbols with demangled expectations from the
  bundled `swift-demangle`, tracked by `tests/swift_toolchain_corpus.rs`.


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

- The Swift demangler now builds an `Rc`-owned node tree with no arena.
  Contiguous identifiers borrow the input symbol; punycode, word-substituted
  names and formatted builtins are owned `Cow` text. Substitution reuse remains
  bounded by clone depth and node budgets.

- Allow `alloc` throughout the core crate; the Swift demangler now uses a
  growable arena instead of fixed node/text capacities.
- Swift demangling retains the parsed node tree and renders it lazily, so
  repeated `Display` calls do not re-parse the symbol.
- Bound Swift substitution cloning by depth and node budget to reject
  demangling bombs instead of risking exponential work or stack overflow.

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
