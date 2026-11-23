# PE/COFF decorations

Reference material for the `coff` feature: the explicit x86 COFF decoration
markers that are orthogonal to the language mangling.

COFF itself has no demangler to use as an oracle. The expected text in
`refs/coff/symbols.tsv` is the project contract: the decoration is removed,
the undecorated name is handed to the existing language backend, and the
decoration is reported separately through `Demangled::coff_decoration()`.
`c++filt` is not used because it echoes these names unchanged.

## Provenance

| Item | Value |
|------|-------|
| Corpus kind | Generated symbol tables from compiler-produced 32-bit COFF objects |
| Generator source | `refs/coff/probe.c`, `probe.cpp`, `probe-import.cpp`, `probe-vectorcall.c` (original project code) |
| Generator script | `refs/coff/generate.sh` |
| Compiler | Debian clang 19.1.7 (`clang-19 --version`: `Debian clang version 19.1.7 (3+b1)`), package `1:19.1.7-3+b1` |
| Compiler upstream | <https://github.com/llvm/llvm-project>, release tag `llvmorg-19.1.7` |
| Symbol extractor | GNU objdump 2.44 (`GNU objdump (GNU Binutils for Debian) 2.44`), Debian package `2.44-3` |
| Extractor upstream | <https://sourceware.org/git/binutils-gdb.git>, release tag `binutils-2_44` |
| Generated on | 2026-11-23 |
| License (probe sources) | MIT OR Apache-2.0, the project license |
| License (tools) | LLVM is Apache-2.0 WITH LLVM-exception; binutils is GPL-3.0-or-later. Neither tool is vendored; only their text output is recorded. |

The objects are compiled with two 32-bit COFF targets because they expose
different decoration conventions:

- `i686-pc-windows-msvc` gives the MSVC spelling (`?name@@...`, `_name@N`,
  `@name@N`, `__imp_...`);
- `i686-pc-windows-gnu` gives the MinGW spelling, where an Itanium name is
  decorated with an extra leading underscore (`__Z...`) and imports become
  `__imp___Z...`.

## Extraction

From the repository root:

```sh
refs/coff/generate.sh
```

The script runs the following commands from `refs/coff/`, writes one
`objdump -t` transcript per object, and removes the temporary `.obj` files:

```sh
clang-19 --target=i686-pc-windows-msvc -c probe.c -o probe-c-msvc.obj
clang-19 --target=i686-pc-windows-msvc -c probe.cpp -o probe-cpp-msvc.obj
clang-19 --target=i686-pc-windows-msvc -c probe-import.cpp -o probe-import-msvc.obj
clang-19 --target=i686-pc-windows-msvc -c probe-vectorcall.c -o probe-vectorcall-msvc.obj
clang-19 --target=i686-pc-windows-gnu -c probe.cpp -o probe-cpp-gnu.obj
clang-19 --target=i686-pc-windows-gnu -c probe-import.cpp -o probe-import-gnu.obj
objdump -t <object> > <object>.objdump.txt
```

The `.objdump.txt` transcripts are normalized to the object basename so the
first line (`probe-c-msvc.obj: file format pe-i386`) is reproducible.

## Vendored files

| File | Contents |
|------|----------|
| `refs/coff/probe.c` | C `cdecl`/`stdcall`/`fastcall` definitions and `dllimport` references |
| `refs/coff/probe.cpp` | MSVC and MinGW C++ import and `stdcall` definitions |
| `refs/coff/probe-import.cpp` | Imported `stdcall` C++ function |
| `refs/coff/probe-vectorcall.c` | x64 `vectorcall` spelling used as a negative control |
| `refs/coff/generate.sh` | Reproducible generation command |
| `refs/coff/probe-*.objdump.txt` | `objdump -t` transcripts of the six objects |
| `refs/coff/symbols.tsv` | Extracted symbols and the expected decoration/text contract |

## Corpus format

`symbols.tsv` is tab-separated. Lines beginning with `#` are comments.

| Column | Meaning |
|--------|---------|
| `symbol` | Linker symbol name exactly as printed by `objdump -t` |
| `decoration` | `none`, `cdecl`, `stdcall` or `fastcall` |
| `convention` | The `CallingConvention` expected from `coff::parse` (`-` when none) |
| `stack` | `@N` stack bytes, or `-` |
| `import` | `yes` for one or more `__imp_` prefixes |
| `inner` | Expected `Decoration::symbol`, or `-` when there is no decoration |
| `requires` | Language feature needed for the full text (`-`, `cpp-itanium`, `cpp-msvc`, `rust`) |
| `scheme` | Expected `Scheme` when that feature is enabled |
| `display` | Expected `Display` text when that feature is enabled |

The corpus has 18 symbols: 10 decorated and 8 undecorated controls. The
undecorated controls include bare `_name` C names, MSVC C++ names whose
calling convention is encoded in the mangling, and the unsupported
`vectorcall` spelling.

## Supported decoration grammar

- `_name@N` — x86 `stdcall`; strips the decoration underscore and `@N`;
  `name` may not contain `@`. `N` is a decimal `u16`; values above 65535 are
  rejected.
- `@name@N` — x86 `fastcall`; exactly two `@` separators.
- `__imp_` / `__imp__` — import pointer. Prefixes are peeled iteratively and
  capped at eight. The inner name is then decoded as C or as an imported
  mangled name (`__imp_?name`, `__imp___Zname`, `__imp___Rname`).
- `__imp_` followed by a C name strips one decoration underscore
  (`__imp__foo` imports `foo`). MSVC names carry no underscore
  (`__imp_?foo@@...`). A directly spelled `__imp__Z...` / `__imp__R...` keeps
  the underscore only when the following byte starts a plausible Itanium or
  rustc v0 encoding, so `__imp__Zone` still imports the C name `Zone`.
- Bare `_name`, `foo@8`, `@name`, `__imp_` and empty names are rejected.
- GNU symbol versions (`name@@VERSION`), ThinLTO suffixes
  (`.llvm.<hash>@@...`) and any non-MSVC inner name containing `@` are
  rejected.

## Unsupported forms

These are deliberately not claimed; they stay with the language backend or
pass through as plain text:

- x64 `vectorcall` `name@@N` and `__imp_name@@N` (no explicit marker this
  module decodes);
- ARM64 COFF spellings (no decoration);
- `thiscall` as a free-standing COFF decoration (MSVC encodes it in the `?`
  mangling);
- MinGW single-underscore `_Z...@N` is treated as a C `stdcall` name
  (`Z...`); the real MinGW form is `__Z...@N` and is supported;
- synthetic single-underscore imported Itanium operators whose first byte
  after `_Z` is not a digit or `N`/`S`/`T`/`L`/`Z` are treated as C imports.

## Checksums

```text
80673b042d3403956a3b0fddc61b6509a572fe7d0964fcfb55de61d98388a763  probe-c-msvc.objdump.txt
824325c78ee7a95625099918f45f29e030a10813910fd3b76a614826bbbb0b97  probe-cpp-gnu.objdump.txt
107207a106186a0a557cc2771cc1210b9edeb399f7744eb43f708b32043798c5  probe-cpp-msvc.objdump.txt
459cec611dccc93e826e403e7676f61776d44a4455c122787b7dab74e05070c4  probe-import-gnu.objdump.txt
4739b9f3c8f710dda23ad3435ea24ac909561da8889c59fccd4b8da830f5606d  probe-import-msvc.objdump.txt
cbb015df52b9551c4210ceabe5fde4eb32194d120c2b54c1385aa9f939d1d81f  probe-vectorcall-msvc.objdump.txt
3de5454a643adfa2e773a09f2538d04ce7080d688ced8cf57cdd4d9ebdf424f1  probe-vectorcall.c
bd157ed5951a516491c56dcdf61af46e6b20f525362451a876c95ca201377c40  probe.c
447e576d1faeab465b51474a656acf01f394f0b28284e6e950e5f429094760f6  probe-import.cpp
cf36fe84d4115010b100a47ae6ef0228d8df1315b62ea14be0e4f7137d1b3727  probe.cpp
367ae67f7ccf35331568ec4303245d31dd91424b372895f89da2ef9cbfe4cf14  generate.sh
991c9a550b05ab1244d3157be40c5e060761374a1a592deacbe57168df13f06f  symbols.tsv
```

## Refresh

1. Run `refs/coff/generate.sh` with the pinned clang-19 and binutils versions.
2. Review the new `objdump -t` transcripts.
3. Update `symbols.tsv` for any new external symbol and re-run
   `cargo test --no-default-features --features coff --test coff_decoration`.
4. Update the date and tool versions in this note in the same commit.
