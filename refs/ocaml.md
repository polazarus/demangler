# OCaml references

OCaml's native compiler (`ocamlopt`) gives every compilation unit and every
exported value one linker symbol. The name is the `caml` prefix, the
compilation-unit path, the member name and a numeric stamp that keeps the symbol
unique; the path is joined with `.`, and every byte outside `[A-Za-z0-9_.]` is
written as a `$XX` escape. The name carries no type, arity or signature
information at all.

OCaml ships no demangler, so this backend is implemented in-tree (`src/ocaml/`)
and the printed text is this crate's contract. No compiler binary or upstream
source is vendored: the fixture is authored for this repository, and the corpora
are derived from it and from the installed standard library with the installed
compiler.

## Toolchain snapshot

| Field | Value |
|-------|-------|
| Compiler | OCaml `5.3.0` (`ocamlopt -version`), Debian `ocaml` `5.3.0-3` |
| Target | `x86_64-linux` (`ocamlopt -config`: `architecture: amd64`, `system: linux`) |
| Standard library | `/usr/lib/x86_64-linux-gnu/ocaml/5.3.0/stdlib.a` (67 units) |
| Runtime | `/usr/lib/x86_64-linux-gnu/ocaml/5.3.0/libasmrun.a` (1119 `caml_*` globals) |
| Retrieved | 2026-11-23 |
| License | Fixture and scripts: MIT OR Apache-2.0, same as this crate. OCaml (LGPL-2.1 with linking exception) is used as an installed tool, not redistributed. Symbol names and the text derived from them are facts. |

Behavioural references (read-only, not vendored):

- OCaml 5.3.0 compiler, tag `5.3.0`:
  <https://github.com/ocaml/ocaml/tree/5.3.0> — `asmcomp/` emits the native
  symbols, `runtime/` defines the `caml_*` helpers.
- The escape spelling is confirmed on the installed compiler: the `$%02x`
  format string appears in `ocamlopt.opt`
  (`strings /usr/bin/ocamlopt.opt | grep '\$%02x'`).

The grammar below was **measured** from the compiler's output on the fixture and
on the installed standard library archive; the upstream sources were not read
offline, so nothing here depends on a source-level claim that the output does
not show.

## Grammar

```text
symbol   := "_"? ( module | helper )
module   := "caml" path [ "." member ]
path     := segment ( "." segment )*            (pack / compilation-unit path)
segment  := upper ( alnum | "_" | escape )*
member   := marker | body "_" digits            (stamp removed by the printer)
body     := ( alnum | "_" | "."-escape | escape )+
helper   := "caml_" name [ "." marker ]
name     := alnum+ ( "_" alnum+ )*
escape   := "$" lower-hex lower-hex             (every byte outside [A-Za-z0-9_.])
```

Measured facts the grammar rests on:

- the path/member separator is `.` on this target: `camlProbe.unit_proc_271`,
  `camlStdlib__List.map_569`; a pack adds one path segment per level
  (`camlOcaml_pack.Ocaml_inner.Ocaml_pack_inner.deep_fun_271`);
- `__` inside a name is an ordinary identifier byte, not a separator: the
  standard library units are literally `Stdlib__List`, `Stdlib__Map`, …;
- the numeric stamp is `<member>_<digits>` and only exists on members, never on
  module records or markers;
- the seven members without a stamp are the compiler's section markers:
  `entry`, `code_begin`, `code_end`, `data_begin`, `data_end`, `frametable`,
  `gc_roots` (67 units × 7 markers in `stdlib.a`);
- a member may start with an uppercase letter: a functor is a module, so the
  functor `F` links as `camlOcaml_probe.F_310`;
- the compiler escapes every byte outside `[A-Za-z0-9_.]` as `$XX` with
  lowercase hexadecimal digits: `+!` is `$2b$21`, `'` is `$27`, `$` is `$24`,
  `@@` is `$40$40`, and `.%()` is `.$25$28$29` (the leading `.` is not escaped);
- a value of a submodule is emitted under the enclosing compilation unit: with
  `module Nested = struct module Deep = struct let deep_fun … end end` in unit
  `Ocaml_probe`, the symbol is `camlOcaml_probe.deep_fun_301`, so the submodule
  path is not recoverable;
- the runtime's C-level pseudo-units are lowercase and keep the `caml_` prefix:
  `caml_startup.code_begin`, `caml_hot.code_begin`, `caml_system.frametable`;
- runtime helpers are `caml_` plus `[A-Za-z0-9]+` segments joined by single `_`
  (`caml_alloc`, `caml_ml_open_descriptor_out`, `caml_MD5Init`, `caml_program`).

## Display contract

| Input | Output |
|---|---|
| `camlProbe` | `Probe` |
| `camlStdlib__List` | `Stdlib__List` |
| `camlOcaml_prime$27` | `Ocaml_prime'` |
| `camlProbe.unit_proc_271` | `Probe.unit_proc` |
| `camlStdlib__List.map_569` | `Stdlib__List.map` |
| `camlProbe.foo__bar_285` | `Probe.foo__bar` |
| `camlOcaml_ops.$2b$21_270` | `Ocaml_ops.+!` |
| `camlOcaml_ops..$25$28$29_291` | `Ocaml_ops..%()` |
| `camlOcaml_probe.F_310` | `Ocaml_probe.F` |
| `camlOcaml_pack.Ocaml_inner.Ocaml_pack_inner.deep_fun_271` | `Ocaml_pack.Ocaml_inner.Ocaml_pack_inner.deep_fun` |
| `camlProbe.entry` | `Probe.entry` |
| `caml_alloc` | `caml_alloc` |
| `caml_startup.code_begin` | `startup.code_begin` |

Rules:

- the path and the member are joined with `.`, and the stamp is dropped;
- `$XX` escapes are decoded to the byte they stand for when that byte is
  printable ASCII. A `$2e` (`.`) is kept verbatim: the compiler never escapes
  `.`, and decoding it would add a separator to the printed path. An escape for
  a non-printable byte is kept verbatim as well, because there is no readable
  spelling for it;
- a runtime helper is printed as it stands: `caml_alloc` already is the C
  identifier, so the backend only classifies it. The corpus records those rows
  as `identity`, never as reconstructed text;
- a runtime pseudo-unit marker prints as `<unit>.<marker>`;
- one optional leading platform underscore (Darwin, PE) is accepted and dropped;
- a trailing ThinLTO `.llvm.<hex>` decoration is stripped before parsing.

## Fixture

| File | SHA256 |
|------|--------|
| `refs/ocaml/fixture/ocaml_probe.ml` | `adda308a2160a62f257894c0dd4b92e22d5df409c61cc13444324296229328cc` |
| `refs/ocaml/fixture/ocaml_ops.ml` | `8027de273ffb984da8e2816724a0ce85234725e516a4c149631dadf393e84a3f` |
| `refs/ocaml/fixture/ocaml_names.ml` | `3741b59c7d58fa41a91f1c8abe0298ee4a0c5431480fd7e9f882a312b6c88645` |
| `refs/ocaml/fixture/ocaml_prime'.ml` | `f5d4605641afcf2f078f1c3b6f17c022d86d29568b9f2cff12fd0761bb733e11` |
| `refs/ocaml/fixture/ocaml_pack_base.ml` | `55555cb72b84a5d07710b0cfb0cc59cbcdfaa71081203287640ef39de832288a` |
| `refs/ocaml/fixture/ocaml_pack_inner.ml` | `5e0d882e1876818034e5849e75eacb2786d917bb1526fc32b23f45cfaec265db` |
| `refs/ocaml/fixture/ocaml_main.ml` | `479742bb08786088de22ca2a9ac305e9ce758a1a43345436f1ba8c07b674d9e9` |
| `refs/ocaml/extract-symbols.sh` | `25a8ba0b069594710c9a43dcfea5cf85270313fc9879c0d0d57c2e93ac29197b` |
| `refs/ocaml/build-corpus.py` | `d41dc399ae9728445914e4ff0dae693aa5a9a71fad6a2fcedcb5a4c9e0cbcc7b` |

The fixture covers unit-scope values, a three-argument function, a recursive
function, a closure, a unit-scope ref, submodules and a nested submodule, a
functor and its application, an exception, an anonymous closure, eight
operators (including a dot operator and an operator containing `$`), value names
with `__`, with `'`, with digits and with a leading `_`, a unit whose own name
needs the `$27` escape, a packed unit, a twice-packed unit, and a program that
links all of them.

## Extraction

```sh
refs/ocaml/extract-symbols.sh > /tmp/ocaml-symbols.txt
python3 refs/ocaml/build-corpus.py          # writes refs/ocaml/corpus/*.tsv
python3 refs/ocaml/build-corpus.py --check  # fails when a corpus is stale
```

`extract-symbols.sh` copies the fixture into a temporary directory, compiles
each unit with `ocamlopt -c`, builds the pack with `-for-pack`/`-pack`, links
the program, and prints the defined `caml` symbols (`nm --defined-only`, types
`T`/`D`/`R`/`C`) of the object files, the packed object, the program and the
installed `stdlib.a`, sorted with `LC_ALL=C`. It writes nothing into `refs/`.

`build-corpus.py` restates the display contract above and writes four TSVs. The
fixture rows are looked up in a table authored from the fixture sources, so a
symbol whose printed text is not a source-level name is a hard error; the
standard library rows are contract-derived and are therefore classified
`contract`, not `exact`.

## Corpus

| File | Rows | Class |
|------|------|-------|
| `refs/ocaml/corpus/fixture.tsv` | 94 | 93 `exact`, 1 `unsupported` |
| `refs/ocaml/corpus/runtime.tsv` | 1045 | 1045 `identity` |
| `refs/ocaml/corpus/link.tsv` | 11 | 9 `exact`, 2 `unsupported` |
| `refs/ocaml/corpus/stdlib.tsv` | 3619 | 3445 `contract`, 174 `unsupported` |
| **total** | **4769** | 4592 accepted, 177 unsupported |

| File | SHA256 |
|------|--------|
| `corpus/fixture.tsv` | `48b39fd02f09220b61a8d9cfdcbfa98e7d8954dec7bf407ef35ff3651446ee04` |
| `corpus/runtime.tsv` | `3951bea39edd1e3f6365a89cb4ca06a3e2e6408d65dfbf50f13864a59a74e2e4` |
| `corpus/link.tsv` | `0f2d18bf1b23ef739abb38d4335f5258e925be2f15a96d3e59b3b1e9e191aac6` |
| `corpus/stdlib.tsv` | `1df9f317a8534f43b0ba2e896699513c5227ebcb83d452a44722a45a97cbe4d9` |

Columns: `symbol<TAB>source<TAB>class<TAB>expected<TAB>note`.

- `exact`: the expected text is authored from the fixture sources, so it is an
  independent check of the contract;
- `contract`: the expected text is the documented transform of a real symbol
  (the 67 standard library units), which guards detection, stamp stripping and
  escape decoding at scale;
- `identity`: a runtime helper, printed as it stands — the row checks the
  classification and is never counted as reconstructed text;
- `unsupported`: the backend must reject the symbol.

`tests/ocaml_corpus.rs` asserts that every accepted row parses, prints the
expected text and re-parses stably, that every unsupported row is rejected and
never claimed, and that no accepted row is claimed by another scheme with
different text. It skips gracefully when `refs/` is absent.

## Detection

`Scheme::detect` claims a symbol only after `ocaml::parse` accepts the whole
name. The `caml` prefix alone is not enough:

- `camel`, `camel_alloc`, `camlist`, `camlist.x` are not OCaml;
- `caml_` with an empty or doubled segment (`caml_`, `caml__alloc`,
  `caml_alloc_`) is not a runtime helper;
- a dotted member without a stamp and without a marker name
  (`camlProbe.foo`, `camlProbe.11`) is not a name;
- `caml_system__code_begin` and `caml_ba_compare.cold` are not `caml<path>` or
  `caml_<name>` forms.

**Detection order.** `Scheme::detect` and `Scheme::detect_bytes` ask
`ocaml::is_ocaml` before the Go heuristic, so dotted `caml...` names are claimed
end to end rather than echoed as Go.

## Gaps

The first slice rejects, rather than guesses:

- compiler-generated static-data labels, whose member is a bare number
  (`camlStdlib.11`, `camlOcaml_probe.3`): 174 rows of `stdlib.tsv`;
- the runtime's double-underscore markers (`caml_system__code_begin`,
  `caml_system__code_end`), which are not `caml<path>` or `caml_<name>` forms;
- C section suffixes the host compiler adds to local runtime functions
  (`caml_ba_compare.cold`, `caml_foo.part.0`); they are local symbols in a
  linked program, so they never reach a linker-name demangler, and they are
  covered by unit tests rather than by the corpus;
- bytecode global-table names: they are not linker symbols, and the bytecode
  compiler's spelling is not verified here;
- the Mach-O/PE spelling of the path separator: only the `.` form the ELF
  fixture produces is verified, so a `camlFoo__bar_1` name is not claimed;
- OCaml 4.x: the corpus is pinned to 5.3.0 and nothing older was measured;
- type, arity and signature information, which the linker name does not carry;
- submodule paths, which the compiler flattens into the compilation-unit path;
- overloaded or shadowed values: the stamp distinguishes them, and the printed
  text repeats the source-level name.

## Refresh procedure

1. Install the target OCaml release and note `ocamlopt -version`, `ocamlopt
   -config` and the Debian package version in the snapshot table.
2. `python3 refs/ocaml/build-corpus.py` and review the diff: every changed or
   new row must be explained by a fixture change or a compiler change.
3. `python3 refs/ocaml/build-corpus.py --check` must be clean afterwards.
4. Update the SHA256 table, the corpus counts, the measured facts and the
   coverage numbers in this file.
5. Raise the `COUNTS` floor in `tests/ocaml_corpus.rs`; never lower it.
