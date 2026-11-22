# C++ references

This snapshot covers the C++ mangling schemes the crate supports: the Itanium
C++ ABI (GCC/Clang), Microsoft Visual C++, and legacy GNU v2 (old GNU).

It is **reference material only**. The crate does not depend on `refs/` at
build time; tests may read it and must skip when the directory is absent.

## Vendored snapshot

| Field | Value |
|-------|-------|
| Upstream repo | <https://github.com/llvm/llvm-project> |
| Pinned commit | `33d006002a464a75b11b080be21aca55f622e3b3` |
| Retrieved | 2026-11-20 |
| License | Apache-2.0 WITH LLVM-exception (`refs/cpp/LICENSE.txt`) |

| Directory | Upstream path | Contents |
|-----------|---------------|----------|
| `refs/cpp/itanium/` | `libcxxabi/test/` and `llvm/unittests/Demangle/` | Itanium demangler corpus and unit test |
| `refs/cpp/msvc/` | `llvm/test/Demangle/` and `llvm/unittests/Demangle/` | MSVC demangler lit tests and unit test |
| `refs/cpp/LICENSE.txt` | `LICENSE.TXT` | LLVM project license |

### Itanium corpus

| File | Upstream path | Size | SHA256 |
|------|---------------|------|--------|
| `refs/cpp/itanium/DemangleTestCases.inc` | `libcxxabi/test/DemangleTestCases.inc` | 5,247,598 B, 29,933 pairs | `d37e20fbaa94203fac90c5a0dc6edb32eb40592ba98677f4fb871ac8db8f0b9e` |
| `refs/cpp/itanium/ItaniumDemangleTest.cpp` | `llvm/unittests/Demangle/ItaniumDemangleTest.cpp` | 3,553 B | `2996701fa63deb2bce60bb40e6e08dffe7142bb1dae851dbf4cf6da90a3067b0` |

`DemangleTestCases.inc` is the libc++abi test corpus: one
`{"<mangled>", "<expected>"}` pair per line, with comments and blank lines.
It is the closest thing to an official Itanium demangler conformance corpus.
It includes compiler/runtime extensions such as `__alloc_token_` and
pointer-authentication manglings, so it also pins pass-through behaviour.

### MSVC corpus

| File | Upstream path | Size |
|------|---------------|------|
| `refs/cpp/msvc/MicrosoftDemangleTest.cpp` | `llvm/unittests/Demangle/MicrosoftDemangleTest.cpp` | 18,739 B |
| `refs/cpp/msvc/*.test` | `llvm/test/Demangle/*.test` | 27 files, includes `ms-basic`, `ms-mangle`, `ms-templates`, `ms-thunks`, `invalid-manglings`, `warn-trailing` |

The lit tests use `llvm-undname` with `; CHECK:` expectations. They cover
`?`-prefixed names, x86/x64/ARM conventions, RTTI, thunks, templates,
back-references, C++11/14/17/20 features, MD5 names, pointer authentication,
string literals, and malformed-input diagnostics.

### Real binary export corpora

The lit tests are synthetic. These additional corpora are export tables dumped
from real, stripped Microsoft-built DLLs in the pinned PyTorch 2.5.1 Windows
wheel:

| File | Source DLL | Symbols | SHA256 |
|------|------------|---------|--------|
| `refs/cpp/msvc/real-torch-2.5.1-cpu-win64-c10.txt` | `c10.dll` | 831 | `0ce966527a6f5e0f594999fbce332a7681436ae68c64eab3be2e5ab391d4fed7` |
| `refs/cpp/msvc/real-torch-2.5.1-cpu-win64-fbgemm.txt` | `fbgemm.dll` | 941 | `4c0fd79088a8af5558c38d99bde41583e40bccdd878d66d05d9a13f753eea3ca` |
| `refs/cpp/msvc/real-torch-2.5.1-cpu-win64-torch_cpu.txt` | `torch_cpu.dll` | 57,476 | `e1ff32b8b2dd4d7a7714c3ee937c4f7103488fa578e8c17bf6e27f68cef55f73` |

| Field | Value |
|-------|-------|
| Wheel | <https://download.pytorch.org/whl/cpu/torch-2.5.1%2Bcpu-cp312-cp312-win_amd64.whl> |
| Wheel SHA256 | `a6b720410350765d3d77c01a5ce098a6c45af446284e45e87a98b8a16e7d564d` |
| DLL SHA256 | `c10.dll` `ec509ef192276bfc59b76b752263913d63a12981310d4518a9e66cc2cce7829c`; `fbgemm.dll` `078a6785613dba45c1277559b9b0a6fcea5538402134d334883f2055448004e2`; `torch_cpu.dll` `ddca75af7eaaa3f0f88d1dcb506215cfee97ccfb9c527333d935a1ce9c8c8343` |
| Retrieved | 2026-11-22 |
| License | PyTorch BSD-style; see <https://github.com/pytorch/pytorch/blob/v2.5.1/LICENSE> |
| Tooling | `refs/cpp/extract-pe-msvc-symbols.py` |

A stripped DLL has no COFF symbol table; only exported decorated names are
visible. That is the set a runtime consumer sees. The files contain only
mangled inputs, not expected text. `tests/cpp_msvc_corpus.rs` asserts that all
59,248 exported names parse into the shared AST and that the printer is
token-equivalent to `msvc-demangler` after normalizing whitespace and the
lit-test `default ctor closure` spelling. The token-equivalent counts are
831/831 (`c10`), 941/941 (`fbgemm`) and 57,476/57,476 (`torch_cpu`).

Regenerate:

```sh
python3 refs/cpp/extract-pe-msvc-symbols.py \
  --binary /tmp/torch/lib/c10.dll \
  --output refs/cpp/msvc/real-torch-2.5.1-cpu-win64-c10.txt
python3 refs/cpp/extract-pe-msvc-symbols.py \
  --binary /tmp/torch/lib/fbgemm.dll \
  --output refs/cpp/msvc/real-torch-2.5.1-cpu-win64-fbgemm.txt
python3 refs/cpp/extract-pe-msvc-symbols.py \
  --binary /tmp/torch/lib/torch_cpu.dll \
  --output refs/cpp/msvc/real-torch-2.5.1-cpu-win64-torch_cpu.txt
```

### Retrieval

```sh
SHA=33d006002a464a75b11b080be21aca55f622e3b3
BASE="https://raw.githubusercontent.com/llvm/llvm-project/$SHA"
mkdir -p refs/cpp/itanium refs/cpp/msvc

curl -fsSL "$BASE/LICENSE.TXT" -o refs/cpp/LICENSE.txt
curl -fsSL "$BASE/libcxxabi/test/DemangleTestCases.inc" \
  -o refs/cpp/itanium/DemangleTestCases.inc
curl -fsSL "$BASE/llvm/unittests/Demangle/ItaniumDemangleTest.cpp" \
  -o refs/cpp/itanium/ItaniumDemangleTest.cpp
curl -fsSL "$BASE/llvm/unittests/Demangle/MicrosoftDemangleTest.cpp" \
  -o refs/cpp/msvc/MicrosoftDemangleTest.cpp

for f in invalid-manglings.test warn-trailing.test \
         ms-arg-qualifiers.test ms-auto-templates.test ms-back-references.test \
         ms-basic.test ms-conversion-operators.test ms-cxx11.test ms-cxx14.test \
         ms-cxx17-noexcept.test ms-cxx20.test ms-mangle.test ms-md5.test \
         ms-nested-scopes.test ms-operators.test ms-options.test \
         ms-placeholder-return-type.test ms-ptrauth.test \
         ms-return-qualifiers.test ms-string-literals.test \
         ms-template-callback.test ms-templates-memptrs-2.test \
         ms-templates-memptrs.test ms-templates.test ms-thunks.test \
         ms-windows.test; do
  curl -fsSL "$BASE/llvm/test/Demangle/$f" -o "refs/cpp/msvc/$f"
done
```

Refresh deliberately: update `SHA`, re-download, update this note and the file
hashes in the same commit.

## Canonical links

### Itanium C++ ABI

- Specification: <https://itanium-cxx-abi.github.io/cxx-abi/abi.html>
- Mangling chapter: <https://itanium-cxx-abi.github.io/cxx-abi/abi-mangling.html>
- Specification repo: <https://github.com/itanium-cxx-abi/cxx-abi>
  (commit `fae5e46c86147a457a057c6372fa95ec38209fe8` at retrieval time)
- LLVM reference demangler:
  `llvm/lib/Demangle/ItaniumDemangle.cpp`
- LLVM demangler unit tests:
  `llvm/unittests/Demangle/ItaniumDemangleTest.cpp`
- libc++abi conformance corpus:
  `libcxxabi/test/DemangleTestCases.inc`

The specification repository has no explicit license file, so it is linked,
not vendored. The LLVM implementation and corpora are Apache-2.0 WITH
LLVM-exception and are vendored above.

### Microsoft C++

There is no published, versioned MSVC mangling grammar comparable to the
Itanium ABI. LLVM's implementation and tests are the practical reference.

- LLVM implementation:
  `llvm/lib/Demangle/MicrosoftDemangle.cpp`,
  `llvm/lib/Demangle/MicrosoftDemangleNodes.cpp`
- LLVM tests:
  `llvm/unittests/Demangle/MicrosoftDemangleTest.cpp`,
  `llvm/test/Demangle/ms-*.test`
- Microsoft decorated-name overview:
  <https://learn.microsoft.com/en-us/cpp/cpp/decorated-names>
- Microsoft `undname` utility:
  <https://learn.microsoft.com/en-us/cpp/build/reference/undname>

### Legacy GNU v2 (old GNU)

GNU v2 is the mangling GCC used before the Itanium C++ ABI. It has no version
prefix: the name is spelled out and the signature follows a `__` separator, as
in `foo__Fiv` (`foo(int, void)`), `bar__3foo` (`foo::bar(void)`), `_$_3foo`
(`foo::~foo(void)`) or `_3foo$bar` (`foo::bar`). `$` (the historical
`CPLUS_MARKER`) separates components of special names; `.` is accepted as an
alternative marker.

| Field | Value |
|-------|-------|
| Reference implementation | <https://github.com/gcc-mirror/gcc/blob/eafe83f2f20ef0c1e7703c361ba314b44574523c/libiberty/cplus-dem.c> |
| Pinned commit | `eafe83f2f20ef0c1e7703c361ba314b44574523c` (`releases/gcc-8.5.0`, tag object `b4a0799d5b1bd8cba43b1d18ffe13e4327377d46`) |
| Retrieved | 2026-11-22 |
| License | LGPL-2.1-or-later (`libiberty/COPYING.LIB`) |
| Test suite | `libiberty/testsuite/demangle-expected`, GPL-3.0-or-later |

**The reference is consulted read-only and is never vendored or copied.** GCC
removed the legacy demanglers from libiberty after 8.5.0, so this pinned
revision is the behavioural reference for the grammar. `src/cpp/legacy.rs` is a
clean Rust implementation over the shared C++ AST: it was written against the
encoding rules and validated name by name against a locally built copy of the
pinned `cplus-dem.c`, which stays in `/tmp` and is not part of this repository.
No libiberty code, test case, or generated output is checked in.

The same pinned revision is the behavioural reference for the ARM and EDG
front ends (`ARM_DEMANGLING` and `EDG_DEMANGLING` in the same file): the
`__ct__`/`__dt__`/`__vtbl__`/`__sti__`/`__std__`/`__pt__`/`__tm__`/`__ps__`
productions, the mandatory `F` marker, one-based argument back-references and
EDG's trailing return type were validated name by name against the same local
build. It remains read-only and unvendored.

### Legacy corpus

`refs/cpp/legacy/` holds one `<mangled><TAB><expected>` corpus per supported
legacy dialect. `gnu-v2.txt`, `arm.txt` and `edg.txt` are **hand-authored for
this repository** from the grammar above and cross-checked against the pinned
reference demangler. `gnu-v2-large.txt` is a deterministic sample derived from
the permissively licensed `gnuv2_demangle` crate (below). In every file the
expected text is this crate's pinned spelling, not raw reference output. They
are licensed like the rest of this repository (MIT OR Apache-2.0) and are gated
by `tests/cpp_legacy_corpus.rs`, which asserts the reported `cpp::Dialect` as
well as the printed text, fails on any mismatch and skips when `refs/` is
absent.

| File | Dialect | Pairs | Floor |
|------|---------|------:|------:|
| `refs/cpp/legacy/gnu-v2.txt` | `cpp::Dialect::GnuV2` | 87 | 80 |
| `refs/cpp/legacy/gnu-v2-large.txt` | `cpp::Dialect::GnuV2` | 1,523 | 1,500 |
| `refs/cpp/legacy/arm.txt` | `cpp::Dialect::Arm` | 45 | 40 |
| `refs/cpp/legacy/edg.txt` | `cpp::Dialect::Edg` | 14 | 12 |

The hand-authored corpus applies the following normalisations, documented in
`src/cpp/legacy.rs`:

| Form | Reference spelling | This crate |
|------|--------------------|------------|
| unnamed function-pointer parameter | ` (*)(void)` | `void (*)(void)` |
| member-function pointer parameter | ` (foo::)(void)` | `void (foo::*)(void)` |
| trailing ellipsis | `int,...` | `int, ...` |

The ARM/EDG corpora add three normalisations, documented in the same module:

| Form | Reference spelling | This crate |
|------|--------------------|------------|
| parameterized-type argument separator | `foo<int,char *>` | `foo<int, char *>` |
| `__pt__`/`__sti__` name payload | raw encoding (`3baz<int>`) | decoded (`baz<int>`) |
| empty parameter list | `foo()` | `foo(void)` |

#### Large GNU v2 sample

`refs/cpp/legacy/gnu-v2-large.txt` is derived from the permissively licensed
`gnuv2_demangle` crate. Its `tests/mangled_lists/*.txt` files are five real
GNU v2 symbol lists; `tests/snapshots.rs` records c++filt-compatible
demanglings of them in `*_cfilt_plain.snap`. Those snapshots are the source
for this corpus, not libiberty or its GPL test suite.

| Field | Value |
|-------|-------|
| Upstream crate | `gnuv2_demangle` 0.6.0 |
| crates.io | <https://crates.io/crates/gnuv2_demangle/0.6.0> |
| Repository | <https://github.com/Decompollaborate/gnuv2_demangle> |
| Pinned commit | `b956763673c8a6a03370c52cae240a2874ec5c26` (`.cargo_vcs_info.json`) |
| Retrieved | 2026-11-22 |
| License | MIT OR Apache-2.0 |
| Crate archive SHA256 | `206f103a93ef7066189a8518d0e57a3c084967b400cbb90c183332bb99add151` |
| Generated file | 1,523 pairs, 122,298 B, SHA256 `30d9a087957b19897fd5992c971814ff21b91202d643cdfdb6bb56824b42723e` |

| Source snapshot | Size | SHA256 |
|-----------------|-----:|--------|
| `snapshots__snapshot_mangled_list_ff2_cfilt_plain.snap` | 1,449,611 B | `0f79203cae41f9cad5580d0f59424d86bba74513fa3036f436fcbfacd8f848e8` |
| `snapshots__snapshot_mangled_list_hit_and_run_cfilt_plain.snap` | 2,496,408 B | `d8991180ad043335beddbc144caaed99ac52a88121e0bf7622484d15647ded26` |
| `snapshots__snapshot_mangled_list_most_wanted_cfilt_plain.snap` | 3,623,027 B | `59e1cfcde5dd3babbcec2f9b482fe02b08f81c80b0196b12310c14ca0b4b1890` |
| `snapshots__snapshot_mangled_list_parappa2_cfilt_plain.snap` | 146,512 B | `31f08344a34f96f22fa88ae7429b12a8f47406daebd1635dafa5549ce784dfa0` |
| `snapshots__snapshot_mangled_list_ty_july_first_cfilt_plain.snap` | 1,040,399 B | `f6c07169c0c8c01f84f141f95d5682c362bd45b574b87d64f6e8971ea3f7f449` |

Extraction and selection (deterministic):

1. Read the five `*_cfilt_plain.snap` files. They contain 83,567 unique
   `(mangled, expected)` pairs, of which 55,554 have expected text different
   from the input.
2. Normalise the expected text to this crate's spelling (table below) and
   demangle the input with `cpp::parse`/the legacy printer. 54,505 pairs match
   byte for byte and report `cpp::Dialect::GnuV2`.
3. Keep the 51,765 of those that `Scheme::detect` reports as `Scheme::Cpp` when
   every backend is enabled.
4. Sort by mangled name and take 0-based indices 0, 34, 68, ...: 1,523 pairs.

Normalisations applied to the upstream text:

| Form | Upstream spelling | This crate |
|------|-------------------|------------|
| nested template closing brackets | `> >` | `>>` |
| trailing ellipsis | `int,...` | `int, ...` |
| array `new`/`delete` | `operator new []` | `operator new[]` |

Excluded from the sample:

- 1,040 symbols whose productions this crate rejects: member templates (`H`),
  squangling (`B`) and template parameter references (`X`/`Y`).
- 9 pairs where the upstream c++filt-compatible text is a historical defect:
  4 member-function-pointer parameters whose return type c++filt moves after
  the following argument, and 5 `_GLOBAL_$I$`/`_GLOBAL_.I.` names whose
  qualified-class payload c++filt prints without the `global constructors
  keyed to` wrapper.
- 2,740 names that another scheme claims when every backend is enabled: Go
  `_._` destructors, `_vt.` virtual tables and `_<class>.<member>` static data
  members, plus Rust `_R` prefixes. The GNU v2 parser and printer handle these,
  but `Scheme::detect` gives the other scheme precedence, so they cannot be
  asserted through the public `Demangler` in an all-features build.

Retrieve and verify the source:

```sh
curl -fsSL https://static.crates.io/crates/gnuv2_demangle/gnuv2_demangle-0.6.0.crate \
  -o /tmp/gnuv2_demangle-0.6.0.crate
sha256sum /tmp/gnuv2_demangle-0.6.0.crate
mkdir -p /tmp/gnuv2_demangle
tar -xzf /tmp/gnuv2_demangle-0.6.0.crate -C /tmp/gnuv2_demangle
```

Refresh deliberately: pin a new crate version and commit, re-extract with the
same rule, then update this note and the file hashes in the same commit.

#### Dialect selection

Legacy names have no prefix, so the dialect is chosen by full parse: GNU v2
first, then ARM, then EDG, and a partial parse never selects a dialect. The
ARM/EDG special-name markers (`__ct__`, `__dt__`, `__vtbl__`, `__sti__`,
`__std__`, `__pt__`, `__tm__`, `__ps__`) are the one exception, because GNU
v2's permissive identifier grammar would otherwise claim them: those symbols
are tried as ARM and EDG first and fall back to GNU v2 when that parse is not
complete. libiberty's `AUTO_DEMANGLING` makes the opposite choice (it reads
`__ct__3fooFv` as `foo::__ct(void)`), so this is a deliberate divergence
recorded here; only names whose GNU v2 reading is a reserved identifier such as
`__ct` are affected.

### Other legacy and vendor schemes

These are **not supported by the crate and not vendored here**. They are
recorded so future work does not silently treat "C++" as a single scheme.

| Scheme | Typical prefix | Notes |
|--------|----------------|-------|
| HP aCC | `foo__F...` / `__...` | libiberty `hp`; the cfront `Xt1t2` template and `L`/`S`/`U` literal extensions are unimplemented |
| Lucid | `__...` | libiberty `lucid` |
| Borland / Embarcadero | `@...` | C++Builder `@foo$q...` |
| IBM XL | vendor-specific | AIX/z/OS; newer toolchains may use Itanium |
| Sun/Oracle Studio | `__1c...` | legacy Studio ABI |
| Digital Mars | `?...` | close to MSVC but not identical |

ARM and EDG are supported in-tree (`src/cpp/legacy.rs`, `cpp::Dialect::Arm` and
`cpp::Dialect::Edg`) with the measured corpora above. Their unimplemented
productions are listed in that module: HP cfront template extensions, EDG
recursive qualified-name components, ARM/EDG local-variable and thunk forms,
and the squangling forms shared with GNU v2.

GCC's libiberty test corpus (`libiberty/testsuite/demangle-expected`) contains
cases for the legacy formats. It is GPL-licensed, so it is linked rather than
copied into this Apache/MIT repository:

- <https://github.com/gcc-mirror/gcc/blob/master/libiberty/testsuite/demangle-expected>
- <https://github.com/gcc-mirror/gcc/blob/master/libiberty/cp-demangle.c>

## What this fixes for us

- **Itanium is the conformance baseline.** The 29,933-pair libc++abi corpus
  gives a broad, upstream-maintained oracle for `_Z` symbols, including
  newer C++ features.
- **MSVC has its own parser and AST.** `msvc-demangler` exposes
  `ParseResult`/`Symbol`/`Name`/`Type`, so an MSVC-to-common-AST adapter can
  be built without writing a parser. The lit tests pin exact printer output.
- **Detection is not the same as support.** `_Z` also prefixes legacy Rust
  symbols, and `?` can appear in non-MSVC vendor schemes. Scheme detection
  must keep a fallback path rather than assuming every `_Z`/`?` is a supported
  C++ symbol.
- **Legacy schemes have no prefix.** `foo__bar` is a plain C identifier while
  `foo__Fiv` is legacy C++, and `a__b__c` demangles as `a__b(char)`. A legacy
  dialect may only be claimed when its parser consumes the whole symbol, which
  is why `Scheme::detect` validates candidate legacy names instead of matching
  a prefix.

## Suggested use

1. Build a normalized `mangled ---> expected` corpus from
   `DemangleTestCases.inc` for the Itanium backend, excluding entries whose
   expected output is intentionally diagnostic rather than a demangling.
2. Extract `?symbol` / `; CHECK:` pairs from the MSVC lit tests into a second
   normalized corpus.
3. Gate each backend with exact-match counts and zero wrong outputs, mirroring
   `tests/swift_corpus.rs` and `tests/swift_toolchain_corpus.rs`.
4. Keep the raw upstream files unchanged so refreshes stay diffable.
