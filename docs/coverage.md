# Coverage and corpus gates

Detailed supported forms, output contracts, detection rules and pinned corpora
for each backend. The short language matrix is in
[languages](languages.md); the test strategy is in [validation](validation.md).

### Go coverage

The in-tree Go backend classifies the names the Go linker emits and prints them
as a Go programmer writes them:

- package, function and method names (`fmt.Println`, `fmt.(*pp).doPrintln`);
- closures and wrappers (`.funcN`, `-rangeN`, `.gowrapN`, `.deferwrapN`, `-fm`,
  `·f`, nested closure indices, inlined function names);
- generic instantiations and compiler shape types (`Sum[go.shape.int]`,
  `go.shape.struct { … }`, `..dict.T[int]`);
- runtime type descriptors (`type:`, `type:*`, `type:.eq.`, `type:.hash.`,
  `type:.gcprog.`, `type:.namedata.`, `type:.importpath.`, `type:noalg.`);
- linker specials (`go:itab.<T>,<I>`, `go:string.<payload>`, `go:info.<T>`,
  `go:buildid`, `go:func.*`, …);
- compiler-generated data symbols (`..inittask`, `..interfaceSwitch.N`,
  `..typeAssert.N`, `..gobytes.N`, `glob..funcN`, `gcbits.<hex>`,
  `gclocals.<base64>`);
- compiler constants (`$f32.`, `$f64.`, `$i32.`, `$i64.`, `$i128.`).

`go::parse` returns an inspectable `go::Ast`. Printing decodes the `%XX`
package-path escapes that `cmd/internal/objabi.PathToPrefix` introduces,
normalises the `·` separator to `.`, and strips ABI markers (`.abi0`,
`.abiinternal`, `.abiwrap`), which `go::Ast::abi` reports separately. Unescaping
is for humans: `%2e` inside the last package path segment becomes `.`, which is
ambiguous with the package/local separator, so printed text is not guaranteed
to re-parse to the same structure. `reflect.name` blobs (`type:.namedata.`) and
`go:string.` payloads are opaque and printed verbatim.

Parsing is bounded by input length (64 KiB), nesting depth, work units and
suffix count; the printed output is never longer than the input.

`cargo test --test go_corpus` validates **3,326** Go names from the ELF symbol
table of a generated test binary, **79/79** generated compiler-object names,
and **12,626/12,626** archive symbols from multiple standard-library packages.
The archive corpus adds static temps, jump tables, `eqfunc`/`hashfunc`,
qualified methods and object-version suffixes. All corpora are extracted from
Go 1.24.4. The test skips when `refs/` is absent and also checks that printed
names contain no `·`, no leftover `%XX` escapes, no ABI marker, and are
idempotent. Bare assembly/cgo names (`aeshashbody`, `_cgo_init`) carry no
package and stay out of Go.

### Swift coverage

Validated against the official corpus vendored in `refs/swift/manglings.txt`:

- **524/524** symbols demangle exactly with zero wrong outputs: builtins and
  integer types, identifiers (word and Punycode substitutions), nominal/bound
  generic types, compound associated types, tuples, functions and accessors,
  standard substitutions and sugar, metatypes, generic
  parameters/signatures/requirements, impl (SIL) function types (`Is`/`II`
  substitutions, conventions, differentiability, coroutines), thunks (partial
  apply, reabstraction, autodiff, key-path, witness, outlined, distributed,
  resume partial), specializations, opaque types, macros
  (freestanding/attached/unique/location), anonymous contexts, packs/variadic
  generics and retroactive/dependent conformances, including the raw-identifier
  `_$s` spellings (`` `3 times` `` and `` `100` ``).
- all **248/248** pre-stable `_T` manglings demangle exactly: legacy types
  (`_Tt`), function and accessor entities (`_TF`), metadata (`_TM`), witness
  tables (`_TW`), value witnesses (`_Tw`), thunks and specializations (`_TT`),
  variables, subscripts and partial apply forwarders. The pre-stable grammar is
  a prefix grammar unrelated to `Mangling.rst`; it is ported in
  `src/swift/legacy.rs` from the reference old demangler and builds the same
  node graph as the postfix parser.

`cargo test --test swift_corpus` runs both gates (all prefixes and the legacy
subset) and skips when `refs/` is absent. An additional 23,975-symbol corpus
extracted from the Swift 6.4.2 Linux toolchain is tracked by `cargo test --test
swift_toolchain_corpus` (also skips without `refs/`); all 23,975 demangle
exactly. `Demangled::scheme()` reports `SwiftStable` for the `$s`/`$S` grammar
and `SwiftLegacy` for the `_T` grammar; `swift::Ast::encoding()` exposes the
same distinction.

### Rust coverage

The in-tree Rust AST parser (`rust::parse`) handles both legacy `_ZN` and v0
`_R` encodings. Its corpus gate covers all 115 upstream unit-test pairs, plus
6,958 legacy and v0 symbols from Rust 1.75.0 and 1.100.0 `libstd` and
`librustc_driver`, with zero wrong outputs. `rustc-demangle` is a test
oracle only, not a runtime dependency. `Demangled::scheme()` reports
`RustLegacy` or `RustV0` from `rust::Ast::encoding()`, so callers see which
encoding was decoded.

### C++ coverage

The in-tree C++ printer covers the Itanium, Microsoft and legacy GNU
v2/ARM/EDG/HP aCC ASTs. Each dialect is reported as its own scheme —
`CppItanium`, `CppMsvc`, `CppGnuV2`, `CppArm`, `CppEdg` or `CppHp` — from
`cpp::Ast::dialect()`. Its Itanium text gate is **29,929/29,933 exact, zero wrong, zero printer
refusals**; the four excluded corpus entries are bare type manglings, not
encodings. The MSVC real-binary gate is **59,248/59,248 token-equivalent**.
The MSVC lit gate is 975/1,219 exact against `msvc-demangler`, with 122
documented spelling differences, zero parse/print errors and 122 symbols the
pinned oracle rejects. Covered forms include pointer-to-member template
arguments (`$F`…`$J`) and dynamic-initializer signatures.

Legacy GNU v2 names (`foo__Fiv`, `bar__3foo`, `_$_3foo`, `_3foo$bar`) share the
same AST and are parsed by `src/cpp/legacy.rs`. Because the `__` separator is
ambiguous, detection validates the whole symbol: `foo__bar` stays plain C while
`a__b__c` demangles as `a__b(char)`. `cpp::legacy::parse_if_legacy` performs that
bounded parse once and returns the winning `Dialect`; detection maps it to
`CppGnuV2`, `CppArm`, `CppEdg` or `CppHp`. Supported: `__`-separated function names,
`Q<n>`-qualified names, templates, operators, constructors/destructors,
pointer/reference/array/function/member-pointer types, back-references and the
`__vt_`/`__ti`/`__tf`/`__thunk_`/`_<name>$<variable>` special names. Squangling
(`K`/`B`/`n`), member templates (`H`) and template template parameters are
rejected rather than approximated; `_GLOBAL_$I$`/`$D$` and `_GLOBAL_.I.`/
`.D.` names are supported.
ARM and EDG forms share the same legacy path: ARM reserved names (`__ct__`,
`__dt__`, `__vtbl__`, `__sti__`, `__std__`) are tried before GNU v2 and fall
back on a full-parse failure; EDG adds its trailing return-type form.
HP aCC is selected by full parse after GNU v2/ARM/EDG, adds `X` template
specs, integral/named-literal arguments, `_<digits>` alternate entries and
declines ARM-only `__pt__`/`__tm__`. `cargo test --test cpp_legacy_corpus`
gates **1,610 GNU v2 pairs** (87 hand-audited plus a 1,523-pair sample from the
permissively licensed `gnuv2_demangle` corpus), **45 ARM** pairs, **14 EDG**
pairs and **99 HP aCC** rows (82 accepted, 17 documented rejections).

### Objective-C coverage

Objective-C has no single mangling format. The in-tree backend recognises the
runtime metadata that appears in linker symbol tables and the method display
forms that crash reports and developer tools show:

- Apple/Mach-O metadata with `_OBJC_`, `__OBJC_` or bare `OBJC_` prefixes:
  classes, metaclasses, ivars, categories, instance/class method lists,
  property lists, instance variables, protocols, protocol references/labels/
  refs/method types, class and category protocol lists, read-only class data,
  selector references and method-name/type strings;
- GNUstep/GNU runtime metadata: `__objc_class_name_*`, `__objc_class_ref_*`,
  `__objc_ivar_offset_*`, the GNUstep 2.0 `_OBJC_CLASS_*`, `_OBJC_REF_CLASS_*`,
  `_OBJC_INIT_CLASS_*` and `_OBJC_PROTOCOL_*` forms, and old GNU/NeXT
  `_OBJC_Class_*` records;
- method display forms `-[Class selector:]`, `+[Class selector:]` and
  `-[Class(Category) selector]`;
- block descriptors `__block_descriptor_…` and `___block_descriptor_…`.

The printed text is a project contract, because no standard Objective-C
demangler defines one:

| Input | Output |
|---|---|
| `_OBJC_CLASS_$_NSString` | `Objective-C class NSString` |
| `_OBJC_METACLASS_$_NSString` | `Objective-C metaclass NSString` |
| `__OBJC_$_CATEGORY_NSString_$_Drawing` | `Objective-C category NSString (Drawing)` |
| `__OBJC_$_INSTANCE_METHODS_Foo` | `Objective-C instance methods Foo` |
| `__OBJC_$_CLASS_METHODS_Foo` | `Objective-C class methods Foo` |
| `__OBJC_$_PROP_LIST_Foo` | `Objective-C property list Foo` |
| `__OBJC_$_INSTANCE_VARIABLES_Foo` | `Objective-C instance variables Foo` |
| `__OBJC_PROTOCOL_$_NSCopying` | `Objective-C protocol NSCopying` |
| `__objc_class_name_Foo` | `Objective-C class Foo` |
| `-[Foo bar:baz:]` | `-[Foo bar:baz:]` |

Class, category and protocol names are preserved verbatim. Both one and two
leading underscores are accepted; metadata prefixes are case-sensitive.
Objective-C++ `_Z` names stay C++, and unknown metadata prefixes are rejected
rather than guessed. `objc::parse` returns an inspectable `objc::Ast`, and
`objc::is_objc` is the detection prefilter.

`cargo test --test objc_corpus` gates the corpora in `refs/objc/corpus/`:
88 Apple/Mach-O metadata names and 26 method display forms emitted by Clang
19.1.7, 21 GNUstep/GNU-runtime metadata names, 14 ELF and 28 COFF decorated
GNUstep ABI 2 names, 51 metadata records from Debian GCC 14's `libobjc.a`,
**18,931 metadata names from the shipped Apache-2.0 Mattermost 2.46.0 iOS app**
(aggregate binary with bundled third-party frameworks), and 18 runtime
helpers/truncated prefixes that must not render as Objective-C. The test skips when `refs/` is absent. Parsing is
bounded by input length, bracket depth and work units; output stays within a
small constant of the input.

### Kotlin/Native coverage

Kotlin/Native is an LLVM backend with its own linker-symbol mangling,
unrelated to Kotlin/JVM descriptors and Kotlin/JS names. The in-tree backend
recognises the compiler's `kfun:`, `ktype:`, `kclass:`, `kproperty:`, `kobj:`,
`kfield:`, `ktypealias:`, `kpackage:`, `knamespace:` and `kvar:` prefixes
(plus Mach-O leading-underscore variants) and prints a stable, Kotlin-like
debugger spelling:

| Input | Output |
|---|---|
| `kfun:com.example.foo#bar(kotlin.Int){}` | `com.example.foo.bar(kotlin.Int)` |
| `kfun:com.example.Box#<init>(kotlin.Int){}` | `com.example.Box.<init>(kotlin.Int)` |
| `kfun:com.example.Box#get(){}` | `com.example.Box.get()` |
| `kfun:com.example#topLevel(){}` | `com.example.topLevel()` |
| `ktype:com.example.Box<kotlin.Int>` | `com.example.Box<kotlin.Int>` |

`#` becomes `.`; `#static` and `#suspend` become prefixes; `#internal` and
`-impl` are linkage markers and are omitted. Extension receivers print as
`name@receiver(...)`, type parameters as `T<index>` and references as
`T<container>:<index>`, variance as `out `/`in `, vararg as `vararg `, and
return types as `: <type>`. Unknown prefixes and unknown `#` markers are
rejected rather than guessed. Detection claims only the known `k*:` prefixes
and runs before Go and plain C, so C++/Rust/Swift/Objective-C keep their
precedence.

`cargo test --test kotlin_native_corpus` gates **8,917** symbols from the ELF
symbol tables of the fixture built with Kotlin/Native 2.0.21, 2.1.21, 2.2.21,
2.3.21, 2.4.0 and 2.4.21, covering top-level,
member, constructor, accessor, local, extension, suspend and static functions,
lambdas, bridges, classes, interfaces, objects, generics, nullable, star and
vararg types. The fixture also declares an inline function; inline bodies are
lowered away and leave no `kfun:` symbol. The pinned executable emits only `kfun:`, `kclass:` and
`kvar:`; a 33-name hand-authored corpus covers the remaining prefixes, the
Mach-O underscore variants and the `%` platform suffix. The test skips when
`refs/` is absent. Parsing is bounded by the input, depth and work budgets;
every path segment, parameter and type argument consumes input.

### D coverage

The in-tree D backend (`src/d/`) parses the D ABI name grammar on raw bytes and
prints the `c++filt -s dlang` spelling. It covers qualified and nested names,
primitive and compound types, functions/delegates, basic templates and
back-references, template values, and compiler specials (`__initZ`, `__vtblZ`,
`__ClassZ`, `__ModuleInfoZ`, `__moduleRefZ`, `_Dmain`).

`tests/d_corpus.rs` gates **10,927 exact** rows with zero wrong: 173
hand-authored spec cases, 36 symbols from an LDC 1.40 probe and 10,718 symbols
from pinned libphobos 2.110.0. The corpus covers `_DThn<offset>_` thunks,
`return scope` markers and back-referenced function types, which `c++filt`
cannot render exactly; two hand-authored productions are rejected. Unsupported
productions are enumerated in `refs/d.md`.

### Java JNI coverage

The in-tree JNI backend (`src/jni/`) decodes short and long `Java_` native
method names on raw bytes: class/package boundaries, `_1`/`_2`/`_3`/`_0xxxx`
escapes (including surrogate pairs), and overload argument descriptors. It
prints Java-like debugger names such as
`com.example.Foo.bar(java.lang.String[], int)`.

`tests/jni_corpus.rs` gates 42 generated JDK 21 rows, 16 hand rows and 35
rejected names; it also parses and re-encodes 1,037 real `Java_*` names from the
JDK's native libraries. The display contract is defined in `refs/jni.md`; JNI
has no canonical demangler.

### Ada/GNAT coverage

The in-tree Ada backend (`src/ada/`) decodes GNAT user-level external names
(`pkg__child__subp`), protected-operation suffixes and the `___elabb`/
`___elabs` elaboration forms, printing dotted Ada spelling. Task/runtime
suffixes outside the supported subset are rejected rather than guessed.

`tests/ada_corpus.rs` gates 43 exact symbols from a GNAT 14.2 fixture and
hand cases, with 64 explicitly unsupported forms and 2 recorded `c++filt`
oracle gaps. Entry barrier/body routines and task body helpers are decoded. Auto-detection is precision-only (`ada__`, `system__`, `gnat__`);
general `pkg__subp` names are available through `ada::parse` because they are
indistinguishable from ordinary C identifiers.

### Fortran coverage

The in-tree Fortran backend (`src/fortran/`) covers gfortran module symbols
(`__module_MOD_member`), external names with the default trailing underscore,
`MAIN__`, and the documented one-underscore ambiguity rules. It deliberately
does not auto-claim ordinary C names ending in `_`; auto-detection is limited
to the distinctive `__*_MOD_*` form, and general external names are available
through `fortran::parse` or `Demangler::demangle_with_scheme`.

`tests/fortran_corpus.rs` gates **55 exact / 43 unsupported** rows from a
gfortran 14.2 fixture and hand cases. Generated derived-type helpers
(`__vtab_`, `__def_init_`, `__copy_`, `__final_`) are decoded; `bind(c)`,
internal/host symbols, `-fno-underscoring` externals and ambiguous trailing
`__` forms remain rejected and documented in `refs/fortran.md`.

### Delphi/Object Pascal coverage

The in-tree Delphi backend (`src/delphi/`) handles Free Pascal and Delphi
linker names: unit procedures/functions, class methods, nested classes,
namespaced units, open-array and typed parameters, constructors and overload
suffixes. Output is a project-defined Pascal-like spelling such as
`DELPHI_PROBE.TPROBECLASS.ADD(LONGINT, LONGINT): LONGINT`.

`tests/delphi_corpus.rs` gates **92 exact / 51 unsupported** rows from a Free
Pascal 3.2.2 fixture and hand cases. Compiler data symbols (`U_$`, `VMT_$`,
`RTTI_$`, `INIT_$`, `INIT$_$`/`FINALIZE$_$`, unit entry routines) are decoded;
nested routines, CRC-substituted generics and runtime tables remain rejected;
`refs/delphi.md` records the grammar and gaps.

### OCaml coverage

The in-tree OCaml backend (`src/ocaml/`) decodes native linker names such as
`camlStdlib__List.map_569` -> `Stdlib__List.map`, module records, escaped
operator names and `caml_*` runtime helpers. The numeric stamp is removed;
`$XX` escapes are decoded only when printable.

`tests/ocaml_corpus.rs` gates **4,781 accepted / 5 unsupported** rows from an
OCaml 5.3.0 fixture, stdlib and runtime archives. Detection runs before Go so
dotted `caml...` names are not stolen by the Go backend. Static-data numeric
labels, `caml_<unit>__<marker>` runtime markers and safe `.cold`/`.part.N`
fragments are decoded; bytecode tables, OCaml 4.x and unverified Mach-O/PE
spellings are documented unsupported in `refs/ocaml.md`.

### PE/COFF decorations

The `coff` feature decodes PE/COFF decorations before scheme detection:
platform-agnostic `_name@N` (stdcall), `@name@N` (fastcall) and `__imp_`
imports, including imported C++ and Rust names. Decoration metadata is
orthogonal to the underlying language: `Demangled::scheme()` remains `C` or the
precise C++ scheme (for example `CppMsvc`), and
`Demangled::coff_decoration()` reports kind, convention, stack bytes,
import and weak-alias state. `Display` prints the undecorated name without
annotations.

The target-aware entry point (`coff::parse_for`) decodes x86/x64 vectorcall (`name@@N`), COMDAT
`.weak.` aliases, ARM64 undecorated names and bare leading underscores without
changing the global detection contract. `.cold`/`.cold.N`/`.part.N` fragments
are also decoded on the target-aware path. `tests/coff_decoration.rs` gates 58
real symbols / 68 target rows plus malformed and depth-cap regressions.
`refs/coff.md` records the generator and provenance.

### Mach-O C names

`Demangler::demangle_macho` / `demangle_macho_bytes` strip one Mach-O leading
underscore only when no scheme claims the name as written and the remainder is
a plain C identifier: `_main` -> `main`. C++, Swift, Objective-C, Rust and
other scheme names keep their detection; ordinary `demangle` never strips the
prefix.

### Explicit schemes, selection and encodings

`Demangler::demangle_with_scheme` (`&str`) and
`Demangler::demangle_bytes_with_scheme` (`&[u8]`) bypass detection when the
caller already knows the exact grammar. Forced schemes are exact: a name parsed
by a different dialect or encoding is rejected with
`Error::InvalidSymbol(requested_scheme)` instead of being retagged, and the
detected C++-then-Rust fallback does not apply. `Scheme::C` and
`Scheme::Unknown` are pass-through. This is the supported route for general Ada
`pkg__subp`, gfortran external `foo_`, namespaced Free Pascal units, and
target-aware COFF names that are intentionally not auto-claimed.

When the language is known but the dialect is not, `Demangler::selection()`
(and the `demangle_with_language` shorthand) tries
the allowed schemes in order and reports `Error::NoMatchingScheme` when none
matches. A language group expands to its schemes; a feature-disabled scheme
fails its candidate and the next one is tried. See
[usage](usage.md#selection).

The `encoding` feature adds `Encoding::{Utf8, Latin1, Windows1252, MacRoman,
Ebcdic037}` and `Demangled::to_string_with_encoding`: it decodes the crate's
reversible `\xNN` identifier escapes after demangling. Shift-JIS/EUC-JP,
UTF-16/32 and other EBCDIC pages remain unsupported; no backend silently
transcodes.
