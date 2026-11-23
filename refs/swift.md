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

## Swift 6.4.2 toolchain corpus

A second corpus is extracted from the official Swift 6.4.2 Linux toolchain.
It contains exported `$s` symbols from the Swift runtime/standard-library
shared objects, with expectations produced by that toolchain's own
`swift-demangle`.

| Field | Value |
|-------|-------|
| Toolchain | Swift 6.4.2 (`swift-6.4.2-RELEASE`), released 2026-10-29 |
| Download | <https://download.swift.org/swift-6.4.2-release/debian13/swift-6.4.2-RELEASE/swift-6.4.2-RELEASE-debian13.tar.gz> |
| Tarball SHA256 | `8b6fde775b7cfc7a3758739d8abb09bb6fba519c6532d6b1a47faff791646c64` |
| Retrieved | 2026-11-19 |
| License | Apache-2.0 WITH LLVM-exception (`refs/swift/LICENSE.txt`; the tarball's `usr/share/swift/LICENSE.txt` is byte-identical) |
| File | `refs/swift/toolchain-6.4.2-manglings.txt` — 3.4 MB, 23,975 pairs |
| File SHA256 | `0dcf9d552740d5dea3ece1c9f33a9ed171f2aff4093f7957b72f18a2a2c8d4e0` |
| Oracle | bundled `swift-demangle` (LLVM 21.0.0) |

Only symbol names and the demangled text produced by the toolchain are
vendored; no binaries are stored.

Retrieval/extraction:

```sh
URL=https://download.swift.org/swift-6.4.2-release/debian13/swift-6.4.2-RELEASE/swift-6.4.2-RELEASE-debian13.tar.gz
curl -fL -o /tmp/swift-6.4.2-debian13.tar.gz "$URL"
sha256sum /tmp/swift-6.4.2-debian13.tar.gz  # must match the pin above

ROOT=swift-6.4.2-RELEASE-debian13
mkdir -p /tmp/swift-extract
tar -xzf /tmp/swift-6.4.2-debian13.tar.gz -C /tmp/swift-extract \
  "$ROOT/usr/bin/swift-demangle" \
  "$ROOT/usr/share/swift/LICENSE.txt" \
  $(tar -tzf /tmp/swift-6.4.2-debian13.tar.gz \
      | grep -E "^$ROOT/usr/lib/swift/linux/libswift[^/]*\.so$")

nm -D --defined-only /tmp/swift-extract/"$ROOT"/usr/lib/swift/linux/*.so \
  | awk '{print $3}' \
  | grep '^\$s' \
  | sort -u > /tmp/swift-symbols.txt

/tmp/swift-extract/"$ROOT"/usr/bin/swift-demangle < /tmp/swift-symbols.txt \
  > /tmp/swift-demangled.txt

python3 - <<'PY'
symbols = open('/tmp/swift-symbols.txt').read().splitlines()
demangled = open('/tmp/swift-demangled.txt').read().splitlines()
with open('refs/swift/toolchain-6.4.2-manglings.txt', 'w') as out:
    for mangled, expected in zip(symbols, demangled):
        if mangled != expected:
            out.write(f'{mangled} ---> {expected}\n')
PY
```

`tests/swift_toolchain_corpus.rs` reads this file and skips when `refs/` is
absent. All 23,975 symbols currently match exactly, with zero wrong outputs
and zero errors.

## Legacy (`_T`) mangling

Pre-stable Swift 2/3 names have no version prefix beyond `_T` and use a
prefix-ordered grammar that `Mangling.rst` does not describe. The reference
implementation routes every name without a stable prefix through its old
demangler (`Context::demangleSymbolAsNode` in `lib/Demangling/Context.cpp`),
which is what `src/swift/legacy.rs` ports. Behaviour was taken read-only from
the pinned upstream commit:

- `lib/Demangling/OldDemangler.cpp` — the grammar and node construction
- `lib/Demangling/Context.cpp` — when the old demangler is selected
- `lib/Demangling/NodePrinter.cpp` — print conventions for old-only nodes
- `include/swift/Demangling/ValueWitnessMangling.def` — value-witness codes

No copy is vendored: the Rust parser is an independent implementation whose
output is gated by the same `refs/swift/manglings.txt` corpus
(`tests/swift_corpus.rs::swift_legacy_corpus`).

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

**Identifiers** (`Mangling.rst`, "Identifiers"). The plain form is
`NATURAL IDENTIFIER-STRING`, where the natural is a byte count. A `00`
prefix selects the Punycode form, whose payload length counts only the encoded
string:

```
identifier ::= '00' natural '_'? IDENTIFIER-CHAR+   // '_' is inserted if the
                                                   // payload starts with a
                                                   // digit or '_'
```

So the separator underscore is *not* part of the length, and it must be
consumed before the payload: `0018_3times_pgaIGJCFbh` is length 18 followed by
the payload `3times_pgaIGJCFbh` (which decodes to `` `3<U+00A0>times` ``).
The decoded text keeps the raw identifier's backticks and its non-breaking
space; the printer does not add or rewrite them. Both raw-identifier symbols in
the official corpus depend on this rule, and `src/swift/parser.rs` charges the
separator to the work budget like any other byte.

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

`tests/swift_corpus.rs` recognises 524 of these 538 pairs and matches all 524
exactly, with zero wrong outputs and zero rejections; the pinned floor is that
exact count. The legacy (`_T`, pre-4.0) gate covers 248 pairs exactly. The
remaining 14 pairs use prefixes this backend does not claim:
`@__swiftmacro_…` filenames, `async_Main…` entry points, `_S$s…` pre-stable
spellings, `_$iW`, and one plain C symbol (`_SocketJoinOrLeaveMulticast`).

Suggested use: drive phase progress from this file (see `.plans/swift.md`),
counting exact matches so each phase has a measurable exit criterion.

## Notes

- The grammar describes *structure*; exact print conventions (spacing,
  `async throws`, sugar such as `[Int]`, `Int?`) live in the C++ node printer,
  so expectations must be pinned to the corpus rather than inferred from the
  grammar alone.
- No Swift toolchain is installed in this environment; the corpus is the
  oracle. Regenerating it requires `swift-demangle` from a Swift toolchain.
