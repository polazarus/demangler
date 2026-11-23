# Unsupported symbol schemes and encodings

This document tracks symbol schemes and platform encodings that are **not**
implemented yet. “Unsupported” means the crate does not claim the name: it is
left to another backend, returned as escaped plain bytes, or rejected rather
than guessed.

The current supported set is C pass-through, Itanium C++, MSVC C++, legacy
GNU v2/ARM/EDG/HP aCC C++, Rust legacy/v0, Swift stable and legacy, Go,
Objective-C, Kotlin/Native, D, Java JNI, Ada/GNAT, Fortran, Delphi/Object
Pascal, OCaml, and PE/COFF decorations. See the README feature matrix for
details.

## Language and ABI schemes

| Scheme | Typical form | Compilers / platforms | Status | Main blocker |
|---|---|---|---|---|
| Haskell / GHC | `Module_foo_info`, `zd...` | GHC native code | not implemented | symbol names are mostly runtime labels, not a stable mangling |
| Zig | mostly C ABI / Itanium for C++ | Zig compiler | not implemented | little distinct native mangling to decode |
| Nim | C-like generated names | Nim | not implemented | no stable ABI mangling |
| Crystal | C/Itanium-style plus runtime names | Crystal | not implemented | no separate stable native encoding |

### Implemented schemes with known gaps

D, Java JNI, Ada/GNAT, Fortran, Delphi/Object Pascal and OCaml are implemented
by the `d`, `jni`, `ada`, `fortran`, `delphi` and `ocaml` features. Their
supported slices, corpora and remaining gaps are documented in `refs/d.md`,
`refs/jni.md`, `refs/ada.md`, `refs/fortran.md`, `refs/delphi.md` and
`refs/ocaml.md`. The sections below list schemes and encodings that remain
unsupported.

## Legacy C++ dialects

The shared C++ AST and legacy dispatcher cover GNU v2, ARM, EDG and HP aCC.
The following dialects remain unsupported:

| Dialect | Typical form | Platform | Status | Evidence/corpus |
|---|---|---|---|---|
| Sun/Oracle Studio | `__1c...` | Solaris, Studio compilers | not implemented | Solaris binaries, Studio docs |
| IBM XL C++ | vendor-specific `__...` | AIX/XCOFF, z/OS | not implemented | AIX libraries, XL docs |
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
| PE/COFF | `_foo@8`, `@foo@8`, `__imp_foo` | platform-agnostic x86 decorations decoded with `coff`; bare `_foo` is available through `coff::parse_leading_underscore*` |
| PE/COFF | `name@@N` vectorcall, `.weak.`/COMDAT aliases, ARM64 | decoded by target-aware `coff::parse_bytes_for`; not auto-claimed globally |
| PE/COFF | `.weak.<name>.default.<assoc>`, `.cold`, free-standing `thiscall` | not decoded |
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
