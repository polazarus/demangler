# Language and compiler coverage

`demangler` recognises the symbol conventions of these language runtimes and
toolchains. The detailed per-backend grammar, output contract, detection rules
and corpus counts are in [coverage](coverage.md); validation gates are described
in [validation](validation.md).

| Language / platform | Typical forms | Feature |
|---|---|---|
| C | plain identifiers | always |
| C++ | Itanium, MSVC, GNU v2, ARM, EDG, HP aCC | `cpp` |
| Rust | legacy `_ZN`, v0 `_R` | `rust` |
| Swift | `$s`, `$S`, `_$s`, `_$S`, legacy `_T` | `swift` |
| Go | linker names, closures, generics, `type:`/`go:` records | `go` |
| Objective-C | Apple/GNUstep metadata and method display forms | `objc` |
| Kotlin/Native | `kfun:`, `ktype:`, `kclass:` and related symbols | `kotlin-native` |
| D | `_D` ABI names | `d` |
| Java JNI | short and long `Java_` native-method names | `jni` |
| Ada/GNAT | external names, protected/task helpers, elaboration | `ada` |
| Fortran | gfortran modules, externals and generated helpers | `fortran` |
| Delphi/Object Pascal | Free Pascal/Delphi linker names | `delphi` |
| OCaml | `caml...` linker names, labels, runtime helpers | `ocaml` |
| PE/COFF | target-aware `_name@N`, vectorcall, imports, weak aliases, fragments | `coff` |
| Mach-O | explicit C `_name` leading-underscore path | always |
| Encodings | UTF-8, Latin-1, Windows-1252, MacRoman, EBCDIC CP037 | `encoding` |

## Detection

Detection is conservative for names that overlap with ordinary C identifiers:
a backend must recognise its own grammar, not merely a prefix. When the source
language is known, bypass detection with `Demangler::demangle_with_scheme` or
`Demangler::demangle_bytes_with_scheme`.

Every backend is implemented in this crate. External demanglers and reference
implementations are test oracles only; they are never runtime dependencies.

## C++ dialects

The C++ backend shares one AST and separate per-dialect parsers and printers:

- Itanium (`_Z` and underscore variants), selected by prefix;
- MSVC (`?`), selected by prefix;
- legacy GNU v2, ARM, EDG and HP aCC, which have no unambiguous prefix and are
  accepted only when a parser consumes the whole symbol.

C++ printing has explicit depth, node-visit and output budgets. The Itanium and
MSVC parsers reject a symbol before those print budgets bind on deep nesting;
legacy parsers have a smaller AST budget, which is the source of the lower
legacy printer caps. See [C++ coverage](coverage.md#c-coverage) and the
[cross-dialect validation notes](validation.md#c-printer-budgets).

## Platform decorations

PE/COFF decorations are orthogonal to the language scheme: the underlying
scheme is C or C++, while `Demangled::coff_decoration()` reports calling
convention, stack bytes, import state, weak aliases and fragments. Mach-O C
names use an explicit entry point because the leading underscore is ambiguous
with ordinary C identifiers.
