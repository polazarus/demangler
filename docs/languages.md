# Language and compiler coverage

`demangler` recognises the symbol conventions of these language runtimes and
toolchains. The detailed per-backend grammar, output contract, detection rules
and corpus counts are in [coverage](coverage.md); validation gates are described
in [validation](validation.md).

## Scheme and language

`Scheme` names one exact grammar or encoding. `Language` groups the schemes that
belong to one source language. `Scheme::language()` and `Language::schemes()` are
exhaustive and consistent mappings; a language group never spans unrelated
manglings.

| Language (`Language::as_str`) | Schemes | Typical forms | Feature |
|---|---|---|---|
| `C` | `C` | `main` | always |
| `C++` | `CppItanium`, `CppMsvc`, `CppGnuV2`, `CppArm`, `CppEdg`, `CppHp` | `_Z3foov`, `?x@@3HA`, `foo__Fiv`, `__ct__3fooFv`, `foo__Fi_v`, `foo__XSP5_Fv` | `cpp-itanium`, `cpp-msvc`, `cpp-gnu-v2` (`cpp` for all) |
| `Objective-C` | `ObjC` | `_OBJC_CLASS_$_NSString`, `-[Foo bar:baz:]` | `objc` |
| `Rust` | `RustLegacy`, `RustV0` | `_ZN3foo17h0123456789abcdefE`, `_RNvCs1234_4main4main` | `rust` |
| `Swift` | `SwiftStable`, `SwiftLegacy` | `$s4main5helloSSyYaKF`, `_TtSi` | `swift` |
| `Go` | `Go` | `fmt.Println`, `type:int` | `go` |
| `Kotlin/Native` | `KotlinNative` | `kfun:com.example#topLevel(){}` | `kotlin-native` |
| `D` | `D` | `_D3foo3barFZv` | `d` |
| `Java` | `Jni` | `Java_com_example_Foo_bar` | `jni` |
| `Ada` | `Ada` | `pkg__subp`, `pkg___elabb` | `ada` |
| `Fortran` | `Fortran` | `__mod_MOD_member`, `MAIN__` | `fortran` |
| `Delphi/Object Pascal` | `Delphi` | `U_$DELPHI_PROBE_$$_UNITCOUNTER`, `DELPHI_PROBE$_$TPROBECLASS_$__$$_ADD$LONGINT$LONGINT$$LONGINT` | `delphi` |
| `OCaml` | `OCaml` | `camlStdlib__List.map_569` | `ocaml` |
| `Unknown` | `Unknown` | names no backend claims | always |

`Cpp` is the C++ group; there is no `Cxx`. Rust legacy/v0 and Swift
stable/legacy are separate schemes because they have separate encodings and
parsers, and each C++ dialect has its own scheme because the backend already
reports the winning `cpp::Dialect`.

Two schemes stay single even though they cover several platform conventions;
the grouping is by backend contract, not by prefix:

- `ObjC` covers Apple/Mach-O and GNUstep/GNU runtime metadata and method display
  forms, because both share one backend contract.
- `Go` covers the linker's name layout (packages, closures, generics, `type:`
  and `go:` records), which is one layout rather than a family of manglings.

Scheme labels used by `Scheme::as_str()` and `Display`:

| Scheme | Label |
|---|---|
| `C` | `C` |
| `CppItanium` | `C++ (Itanium)` |
| `CppMsvc` | `C++ (MSVC)` |
| `CppGnuV2` | `C++ (GNU v2)` |
| `CppArm` | `C++ (ARM)` |
| `CppEdg` | `C++ (EDG)` |
| `CppHp` | `C++ (HP aCC)` |
| `RustLegacy` | `Rust (legacy)` |
| `RustV0` | `Rust (v0)` |
| `SwiftStable` | `Swift` |
| `SwiftLegacy` | `Swift (legacy)` |
| `ObjC` | `Objective-C` |
| `Go` | `Go` |
| `KotlinNative` | `Kotlin/Native` |
| `D` | `D` |
| `Jni` | `JNI` |
| `Ada` | `Ada` |
| `Fortran` | `Fortran` |
| `Delphi` | `Delphi/Object Pascal` |
| `OCaml` | `OCaml` |
| `Unknown` | `Unknown` |

`Language::as_str()` keeps the group name, so the C++ group is `C++` and the
Java/JNI group is `Java`.

## Detection

`Scheme::detect` and `Scheme::detect_bytes` return a precise scheme. Linker
decorations are removed before detection: COFF decorations are peeled first,
then a ThinLTO `.llvm.<hex>` suffix and a GNU `@`/`@@` version suffix. Detection
then runs in three stages, prefix tests first:

1. **Unambiguous prefixes** — Rust v0 `_R`, Rust legacy `_ZN` with the trailing
   `17h<hex>` hash, C++ Itanium `_Z`/`__Z`, MSVC `?` and Swift stable
   `$s`/`$S`/`_$s`/`_$S`. These are byte-prefix tests and do not parse.
2. **Distinctive scheme checks** — D `_D`, Objective-C metadata and method
   display forms, Kotlin/Native `k*:`, Delphi, OCaml, Go, JNI, Ada and Fortran.
   Each backend must recognise its own grammar, not merely a prefix.
   Objective-C runs before Go because dotted metadata such as
   `_OBJC_IVAR_$_Foo._ivar` looks package-qualified; OCaml runs before Go for
   the same reason; and Kotlin/Native claims only its `k*:` prefixes, before Go
   and plain C.
3. **Bounded full parse** — legacy C++ (GNU v2, ARM, EDG, HP aCC) and legacy
   Swift `_T` have no unambiguous prefix, so they are claimed only when a
   parser consumes the whole symbol. Plain C is the final fallback, and an
   unclaimed name is `Unknown`. `foo__bar` stays plain C, `_Time` is not
   claimed as legacy Swift, `a__b__c` is GNU v2 and `_TtSi` is legacy Swift.

Detection is conservative for names that overlap with ordinary C identifiers.
When the source language is known, bypass detection with
`Demangler::demangle_with_scheme` or a `Selection`; see
[usage](usage.md#selection).

Indicative detection cost (release mode, 100,000 calls per case, one machine, no
statistical harness — orders of magnitude, not a benchmark gate):

| Operation | Detect | Parse that follows |
|---|---:|---:|
| Itanium C++ `_ZN3Foo3barEv` | ~70 ns | ~1.6 µs |
| MSVC C++ `?x@@3HA` | ~46 ns | ~0.66 µs |
| legacy C++ `foo__Fiv` | ~1.46 µs | ~0.94 µs |
| Rust v0 `_RNvCs1234_4main4main` | ~52 ns | ~0.82 µs |
| Rust legacy `_ZN3foo17h0123456789abcdefE` | ~93 ns | ~0.48 µs |
| Swift stable `$s…` | ~62 ns | ~1.03 µs |
| Swift legacy `_TtSi` | ~0.94 µs | ~1.09 µs |
| plain C `main` | ~0.24 µs | pass-through |

Prefix checks are effectively free next to the parse they select. Legacy C++ and
legacy Swift cost about as much as demangling, because their dialect is only
known after a full parse; `Scheme::detect` is therefore not the cheap path for
those families, and callers that already know the dialect should force the
scheme.

Every backend is implemented in this crate. External demanglers and reference
implementations are test oracles only; they are never runtime dependencies.

## C++ dialects

The C++ backend shares one AST and separate per-dialect parsers and printers,
and each dialect has its own scheme:

- Itanium (`_Z` and underscore variants): `CppItanium`, selected by prefix;
- MSVC (`?`): `CppMsvc`, selected by prefix;
- legacy GNU v2, ARM, EDG and HP aCC: `CppGnuV2`, `CppArm`, `CppEdg` and
  `CppHp`. They have no unambiguous prefix, so detection parses the symbol once
  and reports the winning `cpp::Dialect`; a dialect is accepted only when its
  parser consumes the whole symbol.

Forced C++ schemes are exact: `demangle_with_scheme(Scheme::CppMsvc, "_Z3foov")`
is rejected instead of returning Itanium text. `Language::Cpp` expands to the
six schemes in the fixed order `CppItanium`, `CppMsvc`, `CppGnuV2`, `CppArm`,
`CppEdg`, `CppHp`; disabled dialect features stay in the group and their
candidates simply fail.

C++ printing has no caps. The demangler retains a tree only when it fits the
resolved expansion gates and a printer depth bound derived from the dialect's
hard parse-depth ceiling. See [C++ coverage](coverage.md#c-coverage) and the
[cross-dialect validation notes](validation.md#c-expansion-gates).

## Platform decorations

PE/COFF decorations are orthogonal to the language scheme: the underlying
scheme is `C` or the precise C++ scheme (for example `CppMsvc`), while
`Demangled::coff_decoration()` reports calling convention, stack bytes, import
state, weak aliases and fragments. Mach-O C names use an explicit entry point
because the leading underscore is ambiguous with ordinary C identifiers.
