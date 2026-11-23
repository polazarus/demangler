# PE/COFF decorations

Reference material for the `coff` feature: the explicit PE/COFF decoration
markers that are orthogonal to the language mangling.

COFF itself has no demangler to use as an oracle. The expected text in
`refs/coff/symbols.tsv` and `refs/coff/targets.tsv` is the project contract: the
decoration is removed, the undecorated name is handed to the existing language
backend, and the decoration is reported separately through
`Demangled::coff_decoration()`. `c++filt` is not used because it echoes these
names unchanged.

Two contracts are recorded because the parser has two kinds of entry point:

- **Platform-agnostic** (`coff::parse_bytes`, `coff::parse`, `coff::is_coff`).
  This is what `Scheme::detect` and `Demangler::demangle_bytes` call. It claims
  only markers that no other platform spells the same way.
- **Target-aware** (`coff::parse_bytes_for`, `coff::parse_for`) plus the
  dedicated `coff::parse_leading_underscore*` helper. These decode the forms
  that are only unambiguous once the caller declares the machine the object was
  built for.

## Provenance

| Item | Value |
|------|-------|
| Corpus kind | Generated symbol tables from compiler-produced COFF objects |
| Generator source | `refs/coff/probe.c`, `probe.cpp`, `probe-import.cpp`, `probe-vectorcall.c`, `probe-weak.c`, `probe-arm64.c` (original project code) and `refs/coff/probe-cold.ll`, `probe-cold-stdcall.ll` (original LLVM IR) |
| Generator script | `refs/coff/generate.sh` |
| Compiler | Debian clang 19.1.7 (`clang-19 --version`: `Debian clang version 19.1.7 (3+b1)`), package `1:19.1.7-3+b1` |
| Compiler upstream | <https://github.com/llvm/llvm-project>, release tag `llvmorg-19.1.7` |
| IR pipeline (fragments) | `opt-19` / `llc-19` 19.1.7 from the same LLVM release, Debian package `1:19.1.7-3+b1` |
| Symbol extractor (x86/x64) | GNU objdump 2.44 (`GNU objdump (GNU Binutils for Debian) 2.44`), Debian package `2.44-3` |
| Extractor upstream (x86/x64) | <https://sourceware.org/git/binutils-gdb.git>, release tag `binutils-2_44` |
| Symbol extractor (ARM64) | `llvm-nm-19` 19.1.7, Debian package `1:19.1.7-3+b1`; GNU objdump 2.44 does not read ARM64 COFF |
| Generated on | 2026-11-23 |
| License (probe sources) | MIT OR Apache-2.0, the project license |
| License (tools) | LLVM is Apache-2.0 WITH LLVM-exception; binutils is GPL-3.0-or-later. Neither tool is vendored; only their text output is recorded. |

The objects are compiled for three COFF targets because they expose different
decoration conventions:

- `i686-pc-windows-msvc` gives the MSVC 32-bit spelling (`?name@@...`,
  `_name@N`, `@name@N`, `name@@N`, `__imp_...`, `.weak.name.default`);
- `i686-pc-windows-gnu` gives the MinGW 32-bit spelling, where an Itanium name
  is decorated with an extra leading underscore (`__Z...`) and imports become
  `__imp___Z...`;
- `x86_64-pc-windows-msvc` gives the x64 spelling, which has no calling-
  convention decoration except `name@@N` for `__vectorcall`;
- `aarch64-pc-windows-msvc` gives the ARM64 spelling, which has no
  calling-convention decoration at all; `__imp_` is the only marker.

## Extraction

From the repository root:

```sh
refs/coff/generate.sh
```

The script runs the following commands from `refs/coff/`, writes one symbol
table transcript per object, and removes the temporary `.obj` files:

```sh
clang-19 --target=i686-pc-windows-msvc -c probe.c -o probe-c-msvc.obj
clang-19 --target=i686-pc-windows-msvc -c probe.cpp -o probe-cpp-msvc.obj
clang-19 --target=i686-pc-windows-msvc -c probe-import.cpp -o probe-import-msvc.obj
clang-19 --target=i686-pc-windows-msvc -c probe-vectorcall.c -o probe-vectorcall-msvc.obj
clang-19 --target=i686-pc-windows-msvc -c probe-weak.c -o probe-weak-msvc.obj
clang-19 --target=x86_64-pc-windows-msvc -c probe-vectorcall.c -o probe-vectorcall-x64-msvc.obj
clang-19 --target=x86_64-pc-windows-msvc -c probe-weak.c -o probe-weak-x64-msvc.obj
clang-19 --target=i686-pc-windows-gnu -c probe.cpp -o probe-cpp-gnu.obj
clang-19 --target=i686-pc-windows-gnu -c probe-import.cpp -o probe-import-gnu.obj
clang-19 --target=aarch64-pc-windows-msvc -c probe-arm64.c -o probe-arm64-msvc.obj
objdump -t <object> > <object>.objdump.txt
llvm-nm-19 probe-arm64-msvc.obj > probe-arm64-msvc.llvm-nm.txt
```

The hot/cold fragment transcripts come from LLVM IR instead of C, because
`HotColdSplitting` is an IR pass and clang's `-fsplit-machine-functions` is
x86-ELF only. `opt` outlines the cold block (the probe carries the branch
weights and function entry count the pass needs) and `llc` emits the object;
nothing is linked and no C runtime is involved:

```sh
opt-19 -passes=hotcoldsplit -S probe-cold.ll -o - \
  | llc-19 -mtriple=i686-pc-windows-msvc -filetype=obj -o probe-cold-x86-msvc.obj
opt-19 -passes=hotcoldsplit -S probe-cold.ll -o - \
  | llc-19 -mtriple=x86_64-pc-windows-msvc -filetype=obj -o probe-cold-x64-msvc.obj
opt-19 -passes=hotcoldsplit -S probe-cold.ll -o - \
  | llc-19 -mtriple=aarch64-pc-windows-msvc -filetype=obj -o probe-cold-arm64-msvc.obj
opt-19 -passes=hotcoldsplit -S probe-cold-stdcall.ll -o - \
  | llc-19 -mtriple=i686-pc-windows-msvc -filetype=obj -o probe-cold-stdcall-x86-msvc.obj
```

`probe-cold-stdcall.ll` is separate because only 32-bit x86 accepts the
`x86_stdcallcc` calling convention; the other probe uses names that are valid
on all three triples.

The transcripts are normalized (trailing whitespace removed, blank lines
dropped) and, for `objdump`, keep the object basename in the first line
(`probe-c-msvc.obj: file format pe-i386`) so they are reproducible.

## Vendored files

| File | Contents |
|------|----------|
| `refs/coff/probe.c` | C `cdecl`/`stdcall`/`fastcall` definitions and `dllimport` references |
| `refs/coff/probe.cpp` | MSVC and MinGW C++ import and `stdcall` definitions |
| `refs/coff/probe-import.cpp` | Imported `stdcall` C++ function |
| `refs/coff/probe-vectorcall.c` | x86 and x64 `__vectorcall` definitions and an import |
| `refs/coff/probe-weak.c` | COMDAT weak aliases of `cdecl`/`stdcall`/`fastcall` definitions |
| `refs/coff/probe-arm64.c` | ARM64 undecorated definitions and an import |
| `refs/coff/probe-cold.ll` | Hot/cold-split IR for MSVC C++, Itanium and C names (all COFF triples) |
| `refs/coff/probe-cold-stdcall.ll` | Hot/cold-split IR for the x86 `stdcall` spelling `_name@N` |
| `refs/coff/generate.sh` | Reproducible generation command |
| `refs/coff/probe-*.objdump.txt` | `objdump -t` transcripts of the thirteen x86/x64 objects |
| `refs/coff/probe-arm64-msvc.llvm-nm.txt` | `llvm-nm` transcript of the ARM64 object |
| `refs/coff/probe-cold-arm64-msvc.llvm-nm.txt` | `llvm-nm` transcript of the ARM64 hot/cold object |
| `refs/coff/symbols.tsv` | Extracted symbols and the platform-agnostic decoration contract |
| `refs/coff/targets.tsv` | The same symbols under their own target's decoration rules |

## Corpus format

### `symbols.tsv`

Tab-separated. Lines beginning with `#` are comments.

| Column | Meaning |
|--------|---------|
| `symbol` | Linker symbol name exactly as printed by `objdump -t` / `llvm-nm` |
| `decoration` | `none`, `cdecl`, `stdcall`, `fastcall` or `vectorcall` |
| `convention` | The `CallingConvention` expected from `coff::parse` (`-` when none) |
| `stack` | `@N`/`@@N` size, or `-` |
| `import` | `yes` for one or more `__imp_` prefixes |
| `inner` | Expected `Decoration::symbol`, or `-` when there is no decoration |
| `requires` | Language feature needed for the full text (`-`, `cpp-itanium`, `cpp-msvc`, `rust`) |
| `scheme` | Expected `Scheme` when that feature is enabled |
| `display` | Expected `Display` text when that feature is enabled |

45 original symbols plus 13 hot/cold-fragment symbols, 58 in total: 13
decorated and 45 undecorated controls. The controls include bare `_name` C
names, MSVC C++ names whose calling convention is encoded in the mangling, the
`__vectorcall` and COMDAT weak-alias spellings that the platform-agnostic
parser deliberately does not claim, the ARM64 undecorated names and the
`.cold.1` fragments.

### `targets.tsv`

Tab-separated. Lines beginning with `#` are comments.

| Column | Meaning |
|--------|---------|
| `symbol` | Linker symbol name exactly as printed by `objdump -t` / `llvm-nm` |
| `target` | `x86`, `x64` or `arm64`: the machine the object was built for |
| `kind` | Expected `DecorationKind` (`undecorated`, `leadingunderscore`, `conventionsuffix`, `import`, `weakalias`, `fragment`) |
| `convention` | Expected `CallingConvention` (`cdecl`, `stdcall`, `fastcall`, `vectorcall`, `unknown`) |
| `stack` | Expected `Decoration::stack_bytes`, or `-` |
| `import` | `yes` when the symbol carries `__imp_` |
| `weak` | `yes` when the symbol is a `.weak.<name>.default` wrapper |
| `fragment` | Expected `Decoration::fragment`: `-`, `cold`, `cold.<N>` or `part.<N>` |
| `inner` | Expected `Decoration::symbol` |

68 rows over 58 distinct symbols (`coff_vectorcall_none@@0` is emitted by both
the x86 and the x64 object, and the hot/cold probe names appear in three
objects). Every external symbol in every vendored transcript must appear in
both corpora; the test asserts that.

## Supported decoration grammar

### Platform-agnostic (`coff::parse`)

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
  (`.llvm.<hash>@@...`), `name@@N`, `.weak.<name>.default` and any non-MSVC
  inner name containing `@` are rejected.

### Target-aware (`coff::parse_for`)

The rules classify every non-empty input; a name without a marker is returned
as `DecorationKind::Undecorated` with its bytes unchanged. `None` means the
input is empty, or starts with a marker that the target's rules cannot decode.

`Target::X86` (32-bit):

- `_name` — `cdecl` (`LeadingUnderscore`), the same form the
  `coff::parse_leading_underscore*` helper decodes;
- `_name@N` — `stdcall`; `@name@N` — `fastcall`;
- `name@@N` — `vectorcall`, where `N` is the argument-block size
  (4 bytes per parameter; measured: 0, 4, 8, 12);
- `__imp_` composes with all of the above. Because x86 already decorates
  `cdecl` with `_`, an Itanium or Rust name carries one decoration underscore:
  `__Z17call_imported_cppi` decodes to `_Z17call_imported_cppi` and
  `__imp___Z12imported_cppi` to `_Z12imported_cppi`;
- `.weak.<name>.default` composes with all of the above (see below);
- `.cold`, `.cold.N` and `.part.N` compose with all of the above (see below);
- MSVC `?`/`.?` names are reported as `Undecorated` (their convention is
  inside the mangling);
- a name that still contains `@` after every rule fails is rejected rather
  than reported as undecorated.

`Target::X64`:

- `name@@N` — `vectorcall`, where `N` is 8 bytes per parameter (measured: 0,
  8, 16, 24);
- `__imp_`, `.weak.<name>.default` and the fragment suffixes compose with it;
- MSVC `?`/`.?` names are `Undecorated`;
- the x86 stack and leading-underscore decorations are not applied: `_foo@8`
  and `@foo@8` are rejected (the target does not decorate that way), and
  `_foo` is `Undecorated` with the underscore kept.

`Target::Arm64`:

- `__imp_` is the only marker; `.weak.<name>.default` and the fragment
  suffixes compose with it;
- every other non-empty input is `Undecorated` with its bytes unchanged,
  including `?name@@...` MSVC names and `name@@N`, because ARM64 decorates no
  calling convention. This is the only target that does not reject a leftover
  `@`.

### Composed metadata

`Decoration::kind()` names the outermost marker (`WeakAlias`, then `Import`,
then the name decoration), while `convention()`, `stack_bytes()`, `import()`
and `weak_alias()` describe the rest. `__imp__foo@8` is therefore
`kind == Import`, `convention == Stdcall`, `stack_bytes == Some(8)`,
`import == true`, `symbol == b"foo"`.

`stack_bytes()` is the decimal `@N`/`@@N` suffix: the callee-popped stack size
for `stdcall`/`fastcall`, and the argument-block size the compiler encodes for
`vectorcall`.

### Hot/cold and partial-inlining fragments

A compiler that splits a function appends the fragment to the name it emits, so
the suffix follows every other decoration and is the outermost marker:
`_coff_stdcall@8.cold.1` is the cold half of `_coff_stdcall@8`. The decoder
peels the fragment first and then applies the target's rules to the base name,
which is why the fragment of a `stdcall` definition keeps
`convention == Stdcall` and `stack_bytes == Some(8)`.

Measured with the vendored probes:

| Spelling | Producer | `Fragment` |
|----------|----------|------------|
| `_coff_stdcall@8.cold.1` | LLVM `HotColdSplitting`, x86 | `Cold { index: Some(1) }` |
| `?cold_cpp@@YAHH@Z.cold.1` | LLVM, x64 and ARM64 (MSVC name) | `Cold { index: Some(1) }` |
| `_Z11cpp_stdcallii.cold.1` | LLVM, x64 and ARM64 | `Cold { index: Some(1) }` |
| `cold_plain.cold.1` | LLVM, x64 and ARM64 (C name) | `Cold { index: Some(1) }` |
| `__Z11cpp_stdcallii.cold.1` | LLVM, x86 (Itanium name with the decoration underscore) | `Cold { index: Some(1) }` |
| `name.cold` | GCC `-freorder-blocks-and-partition` | `Cold { index: None }` |
| `name.part.N` | partial inlining | `Part { index: N }` |

The base keeps its own rules, so `__Z11cpp_stdcallii.cold.1` (x86) decodes to
`_Z11cpp_stdcallii` through the leading-underscore rule, and
`?cold_cpp@@YAHH@Z.cold.1` stays an undecorated MSVC name.

The index is a decimal `u16`; a value above 65535 is rejected. A marker with an
empty base (`.cold`, `.part.0`), an empty or non-decimal index
(`name.cold.`, `name.cold.x`), a missing `.part` index (`name.part`) and a
nested fragment (`name.cold.1.cold.2`) are rejected rather than decoded.

The bare `.cold` form is not reproducible with the pinned toolchain —
`HotColdSplitting` always appends an index — so it is claimed from the GCC
spelling that both `c++filt` and `llvm-cxxfilt` accept
(`_Z3foov.cold` -> `foo() [clone .cold]` / `foo() (.cold)`). The same two
oracles accept `_Z3foov.part.0`; LLVM's `PartialInliner` did not trigger for the
probes, so no COFF object with a `.part.N` symbol is vendored and the grammar is
recorded as the shared clone-suffix contract.

### COMDAT weak aliases

clang/LLVM wraps a weak definition in a COMDAT section whose symbol is
`.weak.<decorated name>.default`. The wrapper is decoded only by the
target-aware entry points, because the inner name needs the target's rules:
`.weak._weak_stdcall@8.default` (x86) is `weak_stdcall`/`stdcall`/8, while
`.weak.weak_stdcall.default` (x64 and ARM64) is `weak_stdcall`/unknown.

The irregular spelling clang emits when a weak C++ function shares a section
with an out-of-line member is rejected: the wrapper must end in `.default`.
The observed form was produced by

```c
/* weak2.cpp */
__declspec(dllexport) __attribute__((weak)) int weak_cpp(int a) { return a; }
struct __declspec(dllexport) WeakClass { virtual ~WeakClass(); int m; };
WeakClass::~WeakClass() {}
```

with

```sh
clang-19 --target=x86_64-pc-windows-msvc -c weak2.cpp -o weak2.obj
llvm-nm-19 weak2.obj
```

which prints `.weak.?weak_cpp@@YAHH@Z.default.??1WeakClass@@UEAA@XZ`. The probe
is not vendored because the form is not decoded.

## Unsupported forms

These are deliberately not claimed; they stay with the language backend or pass
through as plain text:

- `thiscall` as a free-standing COFF decoration: there is no C decoration for
  it, MSVC encodes the convention inside the `?` mangling, and `_name@N` stays
  `stdcall`;
- the irregular `.weak.<name>.default.<assoc>` COMDAT spelling above, and more
  than one weak-alias wrapper or fragment suffix;
- COFF section symbols and constant symbols (`.text$foo`, `.debug$S`,
  `__real@...`, `__xmm@...`): the target rules report a dotted section name as
  an undecorated name, because no decoration marker is involved;
- `name@@N` where `N` is not a decimal `u16`, and any `@N`/`@@N` value above
  65535;
- a vectorcall name that starts with the x86 decoration underscore
  (`_name@@N`) or contains another `@`;
- GNU symbol versions and ThinLTO `.llvm.<hex>` suffixes on any target;
- ARM64 spellings beyond `__imp_` and `.weak.`: the target has no
  calling-convention decoration, so `_name@N` and `name@@N` are just
  undecorated ARM64 names.

## Integration with the crate dispatcher

`Scheme::detect_bytes` and `Demangler::demangle_bytes` call the
platform-agnostic `coff::parse_bytes`, which has no target parameter. The
forms that need a target therefore stay undecorated in the crate's default
path:

- `_coff_cdecl` is pass-through (`Scheme::Unknown`), not `cdecl`;
- `coff_vectorcall@@8` is pass-through, and ELF `_ZN...@@16` keeps its GNU
  version (`Scheme::Rust`);
- `.weak._weak_cdecl.default` is pass-through (`Scheme::Unknown`);
- `.cold`/`.part.N` fragments are pass-through too. The Itanium backend already
  prints its own clone suffix (`_Z11cpp_stdcallii.cold.1` ->
  `cpp_stdcall(int, int) (.cold.1)`), and the MSVC backend drops it; a dotted
  C name such as `cold_plain.cold.1` is claimed by the Go prefilter with
  unchanged text. Decoding the fragment into metadata therefore requires the
  declared target, which also gives the base name without the suffix.

These names are exposed through `coff::parse_bytes_for` /
`parse_leading_underscore*` and then `Demangler::demangle_bytes_with_scheme` with the decoded
`Decoration::symbol`; global auto-detection intentionally stays conservative.

## Checksums

```text
8cf1a9f95f71700e7c71cd561ec16f34adc80dd24b6b3f54ad7b1b774ea1548c  generate.sh
893d5261597a86de80ce5198919155309f68a6b86580eb19312e5c3844f069b0  probe-arm64-msvc.llvm-nm.txt
25a035e89260d2211c76c77a48afc8a7c72e6f5ed5abc928f5e43760a5c5efb3  probe-arm64.c
89c1df2a68a2b4b047b816cdcc576d017d3212f0f93b6717ea7a69d6ecece149  probe-c-msvc.objdump.txt
f36363cb1c434c075755e9327b46cb4f5c307fad5c17f2c577086f58d56a5ccf  probe-cold-arm64-msvc.llvm-nm.txt
1620adce093032dec1b097453dcde43ebadffaf10f2d813e80cf0cdc746ed0f1  probe-cold-stdcall-x86-msvc.objdump.txt
bb5f18d91ebd6af4f2af7f5cb96f03efa2bca6ee6eb0df7c8cc515814211ab86  probe-cold-stdcall.ll
489e3d87d44c34930d7d8ccab6d5bf6c2413ea645d8a72f81660e56ffeb6962d  probe-cold-x64-msvc.objdump.txt
8ebfb3c02a46adc27c76985b953a4954bb46cfe3b37bae65d286d026e7cd9ad2  probe-cold-x86-msvc.objdump.txt
17fb7bfb1309f2f1187dc01393aed0111fb2baa7c5ce70a8c5a9cb0efae571d5  probe-cold.ll
5481fda2f8796a6cab80c82651dd6fe3c0c7ae5bc82f95a7f230695fa29fc81c  probe-cpp-gnu.objdump.txt
30d1c990f614d00bcaaaf4a7c26e3869d6751101322a1b305eb098b3b207b76c  probe-cpp-msvc.objdump.txt
234659a654eab27af3f8ba51531bc5af67649eb9a50f2bae3ce998a6d4da8d24  probe-import-gnu.objdump.txt
f5641abab95c0e67a560eab0cbad664c2e97e92f4af6daa8c88aace2260ed992  probe-import-msvc.objdump.txt
447e576d1faeab465b51474a656acf01f394f0b28284e6e950e5f429094760f6  probe-import.cpp
f938f4c3e922aa7c43c456599878f7857191e5978b4252eb8baeb3f45befe739  probe-vectorcall-msvc.objdump.txt
5c7ebad9b570c7915c93a99e15753c24a2ec1d487c31a5b453bb661f51cb2389  probe-vectorcall-x64-msvc.objdump.txt
6161ec5b7513e5c356a307e93fd7ef756f933415db1a79c84088aa7e16bda829  probe-vectorcall.c
bf4f1cdda6068c61403498797f3585e7fedc3360a3358d36c0a88406447c88ba  probe-weak-msvc.objdump.txt
28c2f030e53e17eeca5838efe30a5108ee64ea7f685cdf55c3ea45f53070d963  probe-weak-x64-msvc.objdump.txt
da06da54a0458e9ffcfec6e36d0d16e252ec56c26f383d44670941e0ef25df36  probe-weak.c
bd157ed5951a516491c56dcdf61af46e6b20f525362451a876c95ca201377c40  probe.c
cf36fe84d4115010b100a47ae6ef0228d8df1315b62ea14be0e4f7137d1b3727  probe.cpp
62dd2f1f0707acfc00618534aa68d7434de8a0523655b20348595f8a5d5ca5f2  symbols.tsv
7a318ae97a26643ee88feb85d416e97e96cad0c97cf7e4e27554babe67c5ff8a  targets.tsv
```

## Refresh

1. Run `refs/coff/generate.sh` with the pinned clang-19, binutils and LLVM
   versions (clang, objdump, llvm-nm, opt and llc).
2. Review the new symbol-table transcripts; the hot/cold transcripts must keep
   the `.cold.<N>` suffix after the decoration.
3. Update `symbols.tsv` and `targets.tsv` for any new external symbol and
   re-run `cargo test --no-default-features --features coff --test
   coff_decoration`.
4. Update the date, tool versions and checksums in this note in the same
   commit.
