# C++ references

This snapshot covers the two C++ mangling schemes the crate currently
supports: the Itanium C++ ABI (GCC/Clang) and Microsoft Visual C++.

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

### Legacy and vendor schemes

These are **not supported by the crate and not vendored here**. They are
recorded so future work does not silently treat "C++" as a single scheme.

| Scheme | Typical prefix | Notes |
|--------|----------------|-------|
| GNU v2 / old GNU | `foo__F...` | pre-Itanium GCC; libiberty `gnu-v2` |
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

## Suggested use

1. Build a normalized `mangled ---> expected` corpus from
   `DemangleTestCases.inc` for the Itanium backend, excluding entries whose
   expected output is intentionally diagnostic rather than a demangling.
2. Extract `?symbol` / `; CHECK:` pairs from the MSVC lit tests into a second
   normalized corpus.
3. Gate each backend with exact-match counts and zero wrong outputs, mirroring
   `tests/swift_corpus.rs` and `tests/swift_toolchain_corpus.rs`.
4. Keep the raw upstream files unchanged so refreshes stay diffable.
