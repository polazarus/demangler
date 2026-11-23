# Unsupported symbol schemes and encodings

This document tracks symbol schemes and platform encodings that are **not**
implemented yet. “Unsupported” means the crate does not claim the name: it is
left to another backend, returned as escaped plain bytes, or rejected rather
than guessed.

The current supported set is C pass-through, Itanium C++, MSVC C++, legacy
GNU v2/ARM/EDG C++, Rust legacy/v0, Swift stable and legacy, Go, Objective-C,
and Kotlin/Native. See the README feature matrix for details.

## Language and ABI schemes

| Scheme | Typical form | Compilers / platforms | Status | Main blocker |
|---|---|---|---|---|
| D | `_D3foo3barFiZi` | DMD, LDC, GDC; Linux/Windows/macOS | not implemented | new parser/printer; public ABI spec and compilers available |
| Ada / GNAT | `ada__package__procedure` | GNAT, GCC Ada; native targets | not implemented | language-specific encoding; GNAT corpus/toolchain |
| Java JNI | `Java_com_example_Foo_bar` | javac/JNI native libraries | not implemented | JNI name escaping (`_1`, `_2`, `_3`); no canonical demangler |
| Fortran | `foo_`, `foo__`, uppercase/lowercase variants | gfortran, Intel, XL, vendor compilers | not implemented | no single standard; compiler-specific decoration |
| Delphi / Object Pascal | `@Foo@Bar$...` | Delphi, Free Pascal, C++Builder | not implemented | vendor-specific grammar and corpora |
| Haskell / GHC | `Module_foo_info`, `zd...` | GHC native code | not implemented | symbol names are mostly runtime labels, not a stable mangling |
| OCaml | `camlModule__foo` | OCaml native compiler | not implemented | simple C-like prefix; low demand |
| Zig | mostly C ABI / Itanium for C++ | Zig compiler | not implemented | little distinct native mangling to decode |
| Nim | C-like generated names | Nim | not implemented | no stable ABI mangling |
| Crystal | C/Itanium-style plus runtime names | Crystal | not implemented | no separate stable native encoding |

### D

D has a documented native ABI mangling. It covers package paths, functions,
templates, arrays, delegates, classes and type encodings. It is the best next
language target because `ldc` and `gdc` are packaged for Debian and
`libphobos`/generated binaries provide a real corpus. `c++filt -s dlang` can
serve as a partial oracle.

### Ada / GNAT

GNAT emits Ada elaboration and package names using `__` separators. The
encoding is compiler-specific and includes elaboration procedures and
task/runtime names. A corpus can be produced with `gnatmake`; `c++filt -s
gnat` is a useful reference, but it may not cover all linker decorations.

### Java JNI

JNI native method names encode package, class and method with `_1` for `_`,
`_2` for `;`, `_3` for `[`, and `_0` for other escapes. They are not
C++-style mangling but are common in native libraries. Support should be a
dedicated `Scheme::Jni`, not folded into C++.

## Legacy C++ dialects

The shared C++ AST and legacy dispatcher already cover GNU v2, ARM and EDG.
The following dialects remain unsupported:

| Dialect | Typical form | Platform | Status | Evidence/corpus |
|---|---|---|---|---|
| Sun/Oracle Studio | `__1c...` | Solaris, Studio compilers | not implemented | Solaris binaries, Studio docs |
| IBM XL C++ | vendor-specific `__...` | AIX/XCOFF, z/OS | not implemented | AIX libraries, XL docs |
| HP aCC | `__...` / `__F` variants | HP-UX/SOM | not implemented | HP-UX archives |
| SGI MIPSpro | `__...` | IRIX | not implemented | IRIX binaries |
| Metrowerks CodeWarrior | `__...` | classic Mac OS, early macOS | not implemented | CodeWarrior binaries |
| Watcom C++ | `WPP...` / vendor forms | DOS, OS/2, Windows | not implemented | OpenWatcom output |
| Lucid | `__...` | historical Unix | not implemented | libiberty reference only |
| Borland C++ | `@foo$q...` | Windows/DOS | not implemented | C++Builder runtimes |
| Digital Mars C++ | `?foo@@...` | Windows/DOS | not implemented | DMC runtimes |

The hard part is obtaining permissively licensed binaries and a trustworthy
demangler oracle. libiberty’s test suite is GPL: it may be consulted read-only
but must not be vendored.

## Unsupported C++ productions

Even the supported C++ dialects reject some productions:

- GNU v2: squangling (`K`/`B`/`n`), member templates (`H`), template-template
  parameters, `X`/`Y` template parameters, `_GLOBAL_$I$`/`$D$` names.
- ARM/EDG: cfront productions outside the implemented subset.
- Itanium: production classes that are not modelled; the corpus gate documents
  the remaining classifications.
- MSVC: `?_A` typeof, `?_P` UDT-returning intrinsics, some legacy `?Q`
  interface forms, and unmodelled RTTI/thunk shapes.

## Platform symbol decorations

These are not language manglings, but consumers still encounter them:

| Platform | Decoration | Status |
|---|---|---|
| Mach-O | leading `_` (`_main`, `_OBJC_*`, `_$s`) | partially normalized; C `_name` not separately decoded |
| ELF | `@`/`@@` symbol versions | normalized for supported backends |
| ELF/ThinLTO | `.llvm.<hex>` | normalized where the backend defines the rule |
| PE/COFF | `_foo`, `_foo@8`, `@foo@8`, `__imp__foo` | not decoded as calling-convention metadata |
| PE/COFF | COMDAT/section suffixes | not decoded |
| AIX/XCOFF | platform name conventions | not handled |
| z/OS/GOFF | EBCDIC-era object conventions | not handled |
| Classic Mac/PEF | MacRoman-era names | not handled |

## Are symbol names always UTF-8?

**No.** ELF, Mach-O and COFF symbol tables store **byte strings**, not
guaranteed UTF-8. Linkers, assemblers and hand-written assembly can emit
arbitrary non-NUL bytes. The API therefore has two entry points:

- `Demangler::demangle(&str)` / `Scheme::detect(&str)` require valid UTF-8.
- `Demangler::demangle_bytes(&[u8])` / `Scheme::detect_bytes(&[u8])` accept
  raw bytes. Supported backends parse the bytes directly; invalid identifier
  bytes are escaped as `\xNN` in the rendered text. Only a name that no scheme
  recognizes is returned as escaped plain text with `Scheme::Unknown`. The
  crate does not transcode legacy character encodings or silently lossy-decode
  them.

### Where non-ASCII or non-UTF-8 names can occur

- **C and assembly labels**: symbol names are linker strings; arbitrary bytes
  are legal in object files.
- **Legacy 8-bit source encodings**: old compilers may pass through source
  identifiers in Latin-1, CP1252, Shift-JIS, MacRoman or another local
  codepage rather than UTF-8.
- **Windows ANSI/OEM codepages**: Borland/Delphi-era tools and hand-written
  DEF/ASM symbols may use local codepages.
- **EBCDIC-era platforms** such as z/OS and some legacy IBM object formats:
  symbol names may be EBCDIC or a mix of EBCDIC/ASCII conventions. Verify per
  object format; do not assume UTF-8.
- **Classic Mac/PEF and NeXT-era tooling**: MacRoman/NeXT encoding is possible
  in old identifier bytes.
- **Objective-C/GNUstep metadata**: modern Clang emits UTF-8, but legacy
  runtime class/category names may carry other encodings.
- **Damaged or truncated symbol tables**: arbitrary bytes may appear even when
  the original compiler emitted ASCII.

Modern compilers generally encode Unicode identifiers as UTF-8, and the
manglings implemented here are structurally ASCII (Swift uses Punycode; Rust
v0 and Kotlin/Native are ASCII; Go uses UTF-8). That is a convention, not a
guarantee at the object-file boundary.

### What the crate does

The byte API preserves the raw input until it must produce text. Invalid bytes
are escaped deterministically; no character encoding is assumed. Decoding
legacy encodings (Latin-1, CP1252, Shift-JIS, MacRoman, EBCDIC) would require
an explicit encoding hint or transcoding layer and is not implemented today.

## References

- D ABI: <https://dlang.org/spec/abi.html#name_mangling>
- JNI function names: <https://docs.oracle.com/en/java/javase/21/docs/specs/jni/design.html#resolving-native-method-names>
- GNAT/GCC: <https://gcc.gnu.org/onlinedocs/gnat_ugn/>
- libiberty demangler/test suite: <https://github.com/gcc-mirror/gcc/tree/master/libiberty>
- Itanium C++ ABI: <https://itanium-cxx-abi.github.io/cxx-abi/abi.html>
- Microsoft decorated names: <https://learn.microsoft.com/en-us/cpp/cpp/decorated-names>
- Object formats: ELF, Mach-O, PE/COFF, XCOFF and GOFF specifications.
