# Swift references

## Vendored snapshot

| Field | Value |
|-------|-------|
| Upstream repo | <https://github.com/swiftlang/swift> |
| Pinned commit | `fa6c26084a4a0bf51016b7dcdbd0a3c23ed69ee7` |
| Retrieved | 2026-11-17 |
| License | Apache-2.0 WITH LLVM-exception (`refs/swift/LICENSE.txt`) |

| File | Upstream path | Size |
|------|---------------|------|
| `refs/swift/Mangling.rst` | `docs/ABI/Mangling.rst` | 80,150 B, 1,542 lines, 696 productions |
| `refs/swift/manglings.txt` | `test/Demangle/Inputs/manglings.txt` | 68,166 B, 538 pairs |
| `refs/swift/LICENSE.txt` | `LICENSE.txt` | 11,751 B |

Retrieval:

```sh
SHA=fa6c26084a4a0bf51016b7dcdbd0a3c23ed69ee7
curl -s "https://raw.githubusercontent.com/swiftlang/swift/$SHA/docs/ABI/Mangling.rst" \
  -o refs/swift/Mangling.rst
curl -s "https://raw.githubusercontent.com/swiftlang/swift/$SHA/test/Demangle/Inputs/manglings.txt" \
  -o refs/swift/manglings.txt
curl -s "https://raw.githubusercontent.com/swiftlang/swift/$SHA/LICENSE.txt" \
  -o refs/swift/LICENSE.txt
```

## Canonical links

- Mangling grammar (rendered): <https://github.com/swiftlang/swift/blob/main/docs/ABI/Mangling.rst>
- Symbolic references section (same file, anchor `symbolic-references`)
- Demangler implementation (C++, reference behaviour of the printer):
  - `lib/Demangling/Demangler.cpp`, `lib/Demangling/NodePrinter.cpp`,
    `lib/Demangling/Punycode.cpp`, `lib/Demangling/Remangler.cpp`
- Official demangler tests: <https://github.com/swiftlang/swift/tree/main/test/Demangle>
- `swift-demangle` CLI test harness: `test/Demangle/demangle.swift`
- Embedded Swift (unstable `$e` prefix): <https://github.com/swiftlang/swift/blob/main/docs/EmbeddedSwift/EmbeddedSwiftStatus.md>
- Swift ABI stability rationale: <https://github.com/swiftlang/swift/blob/main/docs/ABIStabilityManifesto.md>

## What the grammar fixes for us

**Prefixes** (from `Mangling.rst`):

```
mangled-name ::= '$s' global              // Swift stable mangling
mangled-name ::= '@__swiftmacro_' global  // mangling for filenames
mangled-name ::= '_T0' global             // Swift 4.0
mangled-name ::= '$S' global              // Swift 4.2
mangled-name ::= '$e' global              // Embedded Swift (unstable)
mangled-name ::= async-main-entry-point   // 'async_Main', no prefix
```

**Symbolic references**: `[\x01-\x1F]` + 4 bytes (relative) or 8/4 bytes
(absolute, pointer-width dependent), payload may contain NUL; `\xFF` is
alignment padding. The spec requires tools that interpret potentially
uncontrolled input to refuse to interpret symbolic references, so detection
must reject control bytes instead of parsing them.

**Structure**: postfix operator list over a single operator namespace;
run-length identifiers (`4Test3FooC`), word substitutions (max 26 words,
`0`-prefixed identifiers), back-reference substitutions with run compression
(`A`, `AaeB`, `A3a2B`), Punycode for non-ASCII identifiers, 49
`KNOWN-TYPE-KIND` standard types, 134 `global` suffixes.

## Corpus

`refs/swift/manglings.txt` format is `mangled ---> demangled`, one pair per
line. Prefix distribution in this snapshot:

| Prefix | Pairs | Note |
|--------|-------|------|
| `_Tt` | 96 | pre-stable builtin/type metadata |
| `_TF` | 55 | pre-stable function |
| `_T0` | 52 | Swift 4.0 |
| `_TT` | 46 | pre-stable type |
| `$s4` | 39 | stable, module length 4 |
| `$s1` | 27 | stable, module length 1 |
| `_$S` | 20 | pre-stable, old prefix with `_` |
| `_TW` | 19 | pre-stable witness table |

Suggested use: drive phase progress from this file (see `.plans/swift.md`),
counting exact matches so each phase has a measurable exit criterion.

## Notes

- The grammar describes *structure*; exact print conventions (spacing,
  `async throws`, sugar such as `[Int]`, `Int?`) live in the C++ node printer,
  so expectations must be pinned to the corpus rather than inferred from the
  grammar alone.
- No Swift toolchain is installed in this environment; the corpus is the
  oracle. Regenerating it requires `swift-demangle` from a Swift toolchain.
