# Rust references

Rust has two symbol encodings in use:

- **legacy** (`_ZN...E`, Itanium-like): length-prefixed path elements with a
  trailing hash. rustc's long-standing default encoding, still emitted unless
  v0 is requested.
- **v0** (`_R...`, including `_RN`): the scheme specified by RFC 2603,
  selectable with `-C symbol-mangling-version=v0`.

This note covers both. It is **reference material only**: the crate does not
depend on `refs/` at build time, tests may read it, and tests must skip when
the directory is absent (it is excluded from the published crate).

## Vendored snapshot

| Field | Value |
|-------|-------|
| Upstream repo | <https://github.com/rust-lang/rustc-demangle> |
| Pinned commit | `4023665a9bafb7cd9808033fd76fa50f83b9542c` |
| Commit date | 2026-10-29 |
| Retrieved | 2026-11-20 |
| License | MIT OR Apache-2.0 (`refs/rust/rustc-demangle/LICENSE-{APACHE,MIT}`) |

`rustc-demangle` is the reference *decoder*: it defines the exact text that
`rustc`, backtrace printers and `rustfilt` produce, including the lenient
handling of malformed symbols. It has no separate `tests/` directory; the test
cases live in `#[cfg(test)]` modules inside `src/lib.rs`, `src/v0.rs` and
`src/legacy.rs`.

| File | Upstream path | Size | SHA256 |
|------|---------------|------|--------|
| `refs/rust/rustc-demangle/src/lib.rs` | `src/lib.rs` | 18,927 B, 588 lines | `607fe60c1e65da3f86a0b4b8fececb7db79049a0cd4cb316492e8e6593bf39c6` |
| `refs/rust/rustc-demangle/src/v0.rs` | `src/v0.rs` | 54,615 B, 1,638 lines | `4314e7f75088999f23bff85abe8495f2ca0940154ca33c9a76367d957cbb0bd9` |
| `refs/rust/rustc-demangle/src/legacy.rs` | `src/legacy.rs` | 15,859 B, 428 lines | `0bdae63d0db40373dc65621d38a4e5d786c505f096d8484638402b1d3368c73c` |
| `refs/rust/rustc-demangle/src/v0-large-test-symbols/early-recursion-limit` | same path | 51,501 B, 4 symbols + comments | `50482e3500f7233095cbe1961a71946f144af09fa6e4a30b283c522e79f16bbd` |
| `refs/rust/rustc-demangle/README.md` | `README.md` | 1,763 B | `80516de273102806427d87f02043b657ba0b327a7c651b0c33a94c0cd395f3de` |
| `refs/rust/rustc-demangle/CHANGELOG.md` | `CHANGELOG.md` | 1,124 B | `aab7e2a6faa7b823a7ccadd38296ed23cd22e628fa70951e18a14c86064d5220` |
| `refs/rust/rustc-demangle/LICENSE-APACHE` | `LICENSE-APACHE` | 10,847 B | `a60eea817514531668d7e00765731449fe14d059d3249e0bc93b36de45f759f2` |
| `refs/rust/rustc-demangle/LICENSE-MIT` | `LICENSE-MIT` | 1,057 B | `378f5840b258e2779c39418f3f2d7b2ba96f1c7917dd6be0713f88305dbda397` |

`early-recursion-limit` is the fuzzer-found stress fixture for the
`demangling_limits` test: every listed symbol must hit
`ParseError::RecursedTooDeep`, so it is also a useful negative corpus for an
in-tree parser's depth budget.

### `rust-lang/rust` — definitive encoder

| Field | Value |
|-------|-------|
| Upstream repo | <https://github.com/rust-lang/rust> |
| Pinned commit | `5c56312f346d7cc8209d9fc0fd660c72e6fc6950` |
| Commit date | 2026-11-20 |
| Retrieved | 2026-11-20 |
| License | MIT OR Apache-2.0 (`refs/rust/rustc/LICENSE-{APACHE,MIT}`) |

The encoder is authoritative for *what rustc emits*: which namespace tags and
disambiguators appear, how legacy `$` escapes are produced, how v0 compresses
paths/types/consts, and which symbol kinds are exported. The decoder alone
cannot tell you whether a construct is still produced.

| File | Upstream path | Size | SHA256 |
|------|---------------|------|--------|
| `refs/rust/rustc/compiler/rustc_symbol_mangling/src/lib.rs` | same path | 17,649 B, 369 lines | `7f7337a367b8c8dd5ea9dbd7761966ef52fcd2755739060560aea808c630ff36` |
| `refs/rust/rustc/compiler/rustc_symbol_mangling/src/v0.rs` | same path | 42,281 B, 1,125 lines | `790001988b531d1a4e7712db52e2f0120d1ad2a038f142e241d27b0297ddee40` |
| `refs/rust/rustc/compiler/rustc_symbol_mangling/src/legacy.rs` | same path | 21,698 B, 586 lines | `0d2e944ce2f20755a562735a5916e041efa5c0172fe0651bc280fa8616fc0b40` |
| `refs/rust/rustc/LICENSE-APACHE` | `LICENSE-APACHE` | 9,723 B | `62c7a1e35f56406896d7aa7ca52d0cc0d272ac022b5d2796e7d6905db8a3636a` |
| `refs/rust/rustc/LICENSE-MIT` | `LICENSE-MIT` | 1,068 B | `b71bd43a069ca0641a9ecfe585ca7b3c53b5cc1608f8b68321168698e28b5ea1` |

`tests/ui/symbol-names/*.{v0,legacy}.stderr` in the same commit are **not**
vendored as a corpus: the UI test harness runs with `-Z ui-testing`, so the
mangled names contain `CRATE_HASH`, `<REF>` and `[HASH]` placeholders and
cannot be parsed. They are linked below as printer-shape examples only.

### RFC 2603 — v0 specification

| Field | Value |
|-------|-------|
| Upstream repo | <https://github.com/rust-lang/rfcs> |
| Pinned commit | `f43d5789b838fb0d80962e3d4b45fb9ddfcc9a07` |
| Commit date | 2026-11-14 |
| Retrieved | 2026-11-20 |
| License | MIT OR Apache-2.0 (`refs/rust/rfcs/LICENSE-{APACHE,MIT}`) |

| File | Upstream path | Size | SHA256 |
|------|---------------|------|--------|
| `refs/rust/rfcs/text/2603-rust-symbol-name-mangling-v0.md` | `text/2603-rust-symbol-name-mangling-v0.md` | 48,061 B, 1,179 lines | `3cd7d067d2f50c657a547ff217e91e5eb1f058e1c693545b3990782bf6ea3807` |
| `refs/rust/rfcs/LICENSE-APACHE` | `LICENSE-APACHE` | 10,847 B | `a60eea817514531668d7e00765731449fe14d059d3249e0bc93b36de45f759f2` |
| `refs/rust/rfcs/LICENSE-MIT` | `LICENSE-MIT` | 1,023 B | `23f18e03dc49df91622fe2a76176497404e46ced8a715d9d2b67a7446571cca3` |

RFC 2603 contains the EBNF grammar, the compression algorithm and the
"Suggested Demangling" appendix. It does **not** pin every printer detail
(spacing, `{closure#0}`, const type suffixes, `{:#}` alternate mode); those are
pinned by `rustc-demangle`.

## Retrieval

```sh
RD=4023665a9bafb7cd9808033fd76fa50f83b9542c
B="https://raw.githubusercontent.com/rust-lang/rustc-demangle/$RD"
for f in LICENSE-APACHE LICENSE-MIT README.md CHANGELOG.md \
         src/lib.rs src/v0.rs src/legacy.rs \
         src/v0-large-test-symbols/early-recursion-limit; do
  curl -fsSL "$B/$f" -o "refs/rust/rustc-demangle/$f"
done

R=5c56312f346d7cc8209d9fc0fd660c72e6fc6950
B="https://raw.githubusercontent.com/rust-lang/rust/$R"
for f in LICENSE-APACHE LICENSE-MIT \
         compiler/rustc_symbol_mangling/src/lib.rs \
         compiler/rustc_symbol_mangling/src/v0.rs \
         compiler/rustc_symbol_mangling/src/legacy.rs; do
  curl -fsSL "$B/$f" -o "refs/rust/rustc/$f"
done

F=f43d5789b838fb0d80962e3d4b45fb9ddfcc9a07
B="https://raw.githubusercontent.com/rust-lang/rfcs/$F"
for f in LICENSE-APACHE LICENSE-MIT text/2603-rust-symbol-name-mangling-v0.md; do
  curl -fsSL "$B/$f" -o "refs/rust/rfcs/$f"
done
```

Refresh deliberately: update each `SHA`, re-download, regenerate the derived
corpus, and update this note (sizes, hashes, pin dates) in the same commit.

## Derived corpus

`refs/rust/extract-rustc-demangle-tests.py` is **local tooling**, not upstream
material. It expands the `t!`, `t_nohash!`, `t_err!`, `t_nohash_type!`,
`t_const!` and `t_const_suffixed!` assertions in the vendored test modules into
normalized corpora. Regenerate with:

```sh
python3 refs/rust/extract-rustc-demangle-tests.py
```

It resolves `let`-bound string variables and Rust string/raw-string literals.
Cases that are computed at test time (the generated recursion-limit symbols in
`recursion_limit_leaks`, `t_nohash!(&sym, expected)`) are skipped and remain
covered only by the upstream source.

| File | Cases | Format | SHA256 |
|------|-------|--------|--------|
| `refs/rust/corpus/rustc-demangle-tests.txt` | 31 (6 v0, 25 legacy) | `<mangled> ---> <Display output>` | `beb454d8ec7f47c1108bb45cfba3e2ea64e26cda0da7f35169ffb64311e1238a` |
| `refs/rust/corpus/rustc-demangle-tests-nohash.txt` | 84 (66 v0, 18 legacy) | `<mangled> ---> <{:#} output>` | `bdc11b2cdc96ef710fae6fbcfd7f6ef5c2ce62ffa9b30ce7ca91514527fe3332` |
| `refs/rust/corpus/rustc-demangle-tests-errors.txt` | 4 | one input per line; `try_demangle` must reject | `1b18cbc925402d273097eff847e2cd349c1d82622af7f521767a3fb90ba9c90d` |

Corpus conventions (shared by both pair files):

- one pair per line, separated by exactly ` ---> ` (same as
  `refs/swift/manglings.txt`);
- the mangled side is the input verbatim, including `.llvm.` suffixes;
- the expected side is the reference output with no trailing whitespace;
- `{:#}` (alternate) output omits the crate-root hash and the legacy trailing
  hash, and is what backtrace printers show by default;
- blank lines and lines without the separator are ignored by the test harness.

### Corpus gaps

- There is **no** upstream legacy corpus separate from the unit-test
  assertions above, and no upstream toolchain-derived corpus. Generating one
  (for example from a toolchain's `libstd-*.rlib` with `rustc-demangle` as the
  oracle) is a separate, deliberate step; until then the 115 pair lines plus 4
  rejection cases are the whole oracle. `.plans/rust-ast.md` treats this as a
  gate risk.
- The `early-recursion-limit` fixture is a negative corpus, not a pair corpus.
- **Pin/release drift:** the vendored decoder is `rustc-demangle` main at
  `4023665a...`, which is ahead of the `0.1.28` release in `Cargo.lock`. The
  eight `splat` cases in the `{:#}` corpus expect `#[rustc_splat]`, while
  0.1.28 prints `#[splat]`. Until the dependency is pinned to the vendored
  revision (or those expectations are explicitly reconciled), a phase-0 parity
  run against the current crate is 107/115, not 115/115. Do not silently edit
  the corpus to match the older release.

## Not vendored

### rustc-dev-guide symbol-mangling chapter

The v0 and legacy mangling chapters are expected in `rustc-dev-guide` under
`src/symbol-mangling/`. The pinned tree does **not** contain them:

| Field | Value |
|-------|-------|
| Upstream repo | <https://github.com/rust-lang/rustc-dev-guide> |
| Pinned commit | `d946d7402d269990f3b64baabaf6592409791c11` |
| Commit date | 2026-11-18 |
| Retrieved | 2026-11-20 |
| License | MIT OR Apache-2.0 (not vendored: nothing to vendor) |

Verified at that commit: `src/SUMMARY.md` has no mangling entry, the raw paths
`src/symbol-mangling.md`, `src/symbol-mangling/{index,v0,legacy}.md` return
404, and a full-tree grep for `mangling` finds only incidental mentions in
`src/llvm-coverage-instrumentation.md`, `src/compiler-debugging.md`,
`src/debugging-support-in-rustc.md` and `src/backend/libs-and-metadata.md`.

Re-check on refresh:

```sh
SHA=d946d7402d269990f3b64baabaf6592409791c11
curl -fsSL "https://raw.githubusercontent.com/rust-lang/rustc-dev-guide/$SHA/src/SUMMARY.md" \
  | grep -i mangling
```

If the chapter reappears, pin the new commit and vendor the files; RFC 2603
covers v0 syntax, but the guide is the only prose description of the legacy
scheme.

### Other linked-only material

- `tests/ui/symbol-names/` in `rust-lang/rust`: printer-shape examples with
  UI-testing placeholders (not parseable).
- `rust-lang/rustc-demangle`'s `crates/rustfilt` and `crates/native-c`: not
  needed for a pure-Rust in-tree demangler.
- The Rust Reference and the rustc book's codegen options page: linked below.

## What the references fix for us

### Accepted prefixes and dispatch

Both decoders accept variants produced by platform symbol handling. From
`rustc-demangle`:

| Encoding | Accepted prefixes | Additional checks |
|----------|-------------------|-------------------|
| legacy | `_ZN`, `ZN`, `__ZN` | remainder must be all ASCII; elements are `len` + bytes; final `E`; anything after `E` is a suffix |
| v0 | `_R`, `R`, `__R` | length must leave at least one byte; the byte after the prefix must be `A`..=`Z` (a path tag); optional decimal version |

Consequences for `Scheme::detect`:

- `_RN` is just `_R` + an `N` (nested path) tag, so `_RN` is not a special
  case, only a common one.
- `_Z`/`_ZN` is shared with Itanium C++ (`_ZN3Foo3barEv`). Prefix sniffing
  cannot decide; the legacy parser must *reject* C++ names. Rust legacy names
  end with `E`, are all ASCII, and every element is a decimal length followed
  by exactly that many bytes. `_ZN3Foo3barEv` parses as the legacy path
  `Foo::bar` and leaves `v`; the suffix rule (only `.`-prefixed, symbol-like
  text is kept) then rejects the symbol. `_ZN3fooE`, by contrast, is valid in
  both schemes, so only a full parse plus the suffix rule can decide.
- A Rust symbol may carry a trailing `.llvm.<hex>@@<n>` or `.0.0` suffix.
  `rustc-demangle` strips `.llvm.` plus trailing `[0-9A-F@]` before parsing,
  and after parsing keeps the suffix only if it starts with `.` and is
  symbol-like (ASCII alphanumeric/punctuation); otherwise the demangling is
  discarded and the input is printed verbatim.

### Lenient versus strict decoding

This distinction matters for API parity:

- `rustc_demangle::demangle(s)` **never fails**. Non-Rust input is printed
  unchanged; v0 syntax errors inside a symbol print `{invalid syntax}`,
  `{recursion limit reached}` or `?` for the failing sub-term, keeping the
  surrounding partial output.
- `rustc_demangle::try_demangle(s)` fails when neither decoder recognizes the
  symbol at the top level, including when the v0 decoder bails out with
  `RecursedTooDeep` before producing a style.

An in-tree parser needs both: a strict `Result` for `Demangler::demangle`, and
enough of the lenient printer behaviour to stay byte-compatible with the
reference while the backend is still delegated.

### Budgets

- v0 parsing has a recursion depth limit of 500 (`MAX_DEPTH`), incremented by
  paths, types, consts and patterns; exceeding it is
  `ParseError::RecursedTooDeep`.
- Every length/base-62/hex accumulation is checked (`checked_mul`/`checked_add`),
  so huge digit strings are rejected rather than overflowing.
- Backrefs must point strictly before the backref itself (`i < s_start`), which
  is what makes substitution-based expansion acyclic.
- legacy parsing is iterative (no recursion) but still checks length
  arithmetic.

### v0 grammar

The RFC grammar is vendored verbatim; the productions an AST needs are:

```text
<symbol-name> = "_R" [<decimal-number>] <path> [<instantiating-crate>] [<vendor-specific-suffix>]
<path>        = "C" <identifier> | "M" <impl-path> <type> | "X" <impl-path> <type> <path>
              | "Y" <type> <path> | "N" <namespace> <path> <identifier>
              | "I" <path> {<generic-arg>} "E" | <backref>
<impl-path>   = [<disambiguator>] <path>
<identifier>  = [<disambiguator>] <undisambiguated-identifier>
<disambiguator> = "s" <base-62-number>
<undisambiguated-identifier> = ["u"] <decimal-number> ["_"] <bytes>
<namespace>   = "C" | "S" | <A-Z> | <a-z>
<generic-arg> = <lifetime> | <type> | "K" <const>
<lifetime>    = "L" <base-62-number>
<binder>      = "G" <base-62-number>            // value + 1 bound lifetimes
<type>        = <basic-type> | <path> | "A" <type> <const> | "S" <type>
              | "T" {<type>} "E" | "R" [<lifetime>] <type> | "Q" [<lifetime>] <type>
              | "P" <type> | "O" <type> | "F" <fn-sig> | "D" <dyn-bounds> <lifetime>
              | <backref>
<fn-sig>      = [<binder>] ["U"] ["K" <abi>] {<type>} "E" <type>
<dyn-bounds>  = [<binder>] {<dyn-trait>} "E"
<dyn-trait>   = <path> {<dyn-trait-assoc-binding>}
<dyn-trait-assoc-binding> = "p" <undisambiguated-identifier> <type>
<const>       = <type> <const-data> | "p" | <backref>
<const-data>  = ["n"] {<hex-digit>} "_"
<backref>     = "B" <base-62-number>
<instantiating-crate> = <path>
<vendor-specific-suffix> = ("." | "$") <suffix>
```

The reference decoder implements more than the RFC documents. Worth knowing
before designing the AST:

- `w` is an unstable splat marker (`#[rustc_splat] `) before a type.
- `W` encodes `T is <pattern>` (`print_pat`: range `R`, or-pattern `O`, `!null`
  `N`).
- Consts support `V` (ADT consts: unit, tuple, struct), `A` (array), `T`
  (tuple), `R`/`Q` (reference, with `Re` = string literal), `e` (`str`
  literal, printed as `*"..."`), `b`, `c`, and integer leaves.
- Integer consts print the numeric value plus the basic-type name in
  `Display` mode (`-11i8`) and without the type in `{:#}` mode (`-11`);
  values that do not fit `u64` print as `0x<nibbles>`.
- Disambiguators print as `[hex]` after a crate-root identifier only in
  `Display` mode, and as `#n` for closures/shims in both modes.
- Namespace tags: `C` prints `{closure#n}`, `S` prints `{shim:name#n}`, other
  uppercase tags print verbatim; lowercase tags are implementation-internal
  and are omitted unless the identifier is non-empty.
- Lifetime indices are printed relative to the enclosing binder depth (`'_` for
  0, `'a`..`'z` then `'_123`), and binders print `for<'a, 'b> `.
- ABI names replace `-` with `_` in the mangling, so printing re-joins `_` as
  `-` inside `extern "..."`.
- `I` (generic args) in value position prints an extra `::` before `<`.
- `M`/`X`/`Y` print `<Self>` or `<Self as Trait>` and omit the impl path.
- Punycode identifiers use RFC 3492 with `-` replaced by `_`; the split point
  is the last `_`. Undecodable punycode prints `punycode{ascii-punycode}`.
- Backrefs are absolute byte offsets (`B<base62>`), not stack indices, and may
  point into any earlier part of the symbol.

### legacy grammar

No standalone spec exists; the vendored encoder and decoder define it:

- `_ZN` + repeated `<decimal length><bytes>` + `E`, optionally followed by
  `.llvm.<hex>` or another `.`-suffixed suffix.
- Element text is unescaped before printing:
  - `_$` at the start of an element is dropped (`_$LT$...` → `<...>`);
  - `..` becomes `::`, a single `.` stays `.`;
  - `$SP$`→`@`, `$BP$`→`*`, `$RF$`→`&`, `$LT$`→`<`, `$GT$`→`>`, `$LP$`→`(`,
    `$RP$`→`)`, `$C$`→`,`;
  - `$u<lowercase-hex>$` becomes that Unicode scalar if it is not a control
    character;
  - an unknown `$...$` escape ends unescaping and the rest is printed verbatim.
- The trailing path element is a hash when it is `h` followed by hex digits;
  `{:#}` omits it.
- Legacy has no structured types: everything is path text, which is why a
  single AST node kind (`LegacyPath`) is enough for the whole encoding.

## Canonical links

- RFC 2603 (v0 spec, rendered at the pinned commit):
  <https://github.com/rust-lang/rfcs/blob/f43d5789b838fb0d80962e3d4b45fb9ddfcc9a07/text/2603-rust-symbol-name-mangling-v0.md>
- rustc-dev-guide symbol mangling chapter (absent at the pinned SHA; check
  before linking in docs):
  <https://github.com/rust-lang/rustc-dev-guide/tree/master/src/symbol-mangling>
- `rustc-demangle` API docs: <https://docs.rs/rustc-demangle>
- rustc v0 encoder:
  `compiler/rustc_symbol_mangling/src/v0.rs`
- rustc legacy encoder:
  `compiler/rustc_symbol_mangling/src/legacy.rs`
- rustc symbol-name UI tests (placeholders, printer shapes):
  <https://github.com/rust-lang/rust/tree/master/tests/ui/symbol-names>
- `-C symbol-mangling-version` option:
  <https://doc.rust-lang.org/rustc/codegen-options/index.html#symbol-mangling-version>
- `rustfilt` (uses `rustc-demangle`): <https://github.com/luser/rustfilt>

## Notes

- `rustc-demangle` is a decoder, not a specification: when the RFC and the
  decoder disagree, the decoder wins for output parity. Design the printer
  against the reference and gate it with the corpus above.
- Both encodings can appear in the same binary (dependencies built by older
  compilers), so one demangler instance must handle either per symbol.
- The v0 version digit (`_R0`, `_R1`, ...) is currently always absent; the
  reference decoder rejects unknown digits only because they are not path tags.
  Treat a leading digit as forward-compatibility: parse it, but do not assume
  semantics.
