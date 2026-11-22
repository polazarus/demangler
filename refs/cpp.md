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

### Legacy corpus

`refs/cpp/legacy/gnu-v2.txt` holds `<mangled><TAB><expected>` pairs. It is
**hand-authored for this repository** from the grammar above and cross-checked
against the pinned reference demangler; the expected text is this crate's
pinned spelling, not reference output. It is licensed like the rest of this
repository (MIT OR Apache-2.0) and is gated by `tests/cpp_legacy_corpus.rs`,
which fails on any mismatch and skips when `refs/` is absent.

The corpus normalises two historical defects, both documented in
`src/cpp/legacy.rs`:

| Form | Reference spelling | This crate |
|------|--------------------|------------|
| unnamed function-pointer parameter | ` (*)(void)` | `void (*)(void)` |
| member-function pointer parameter | ` (foo::)(void)` | `void (foo::*)(void)` |
| trailing ellipsis | `int,...` | `int, ...` |

### Other legacy and vendor schemes

These are **not supported by the crate and not vendored here**. They are
recorded so future work does not silently treat "C++" as a single scheme.

| Scheme | Typical prefix | Notes |
|--------|----------------|-------|
| HP aCC | `foo__F...` / `__...` | libiberty `hp` |
| EDG | `__...` | libiberty `edg` |
| ARM | `__...` | libiberty `arm` |
| Lucid | `__...` | libiberty `lucid` |
| Borland / Embarcadero | `@...` | C++Builder `@foo$q...` |
| IBM XL | vendor-specific | AIX/z/OS; newer toolchains may use Itanium |
| Sun/Oracle Studio | `__1c...` | legacy Studio ABI |
| Digital Mars | `?...` | close to MSVC but not identical |

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
