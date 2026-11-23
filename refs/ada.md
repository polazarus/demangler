# Ada/GNAT references

GNAT does not use a C++-style mangling for Ada declarations. It builds an
*external name* from the fully-qualified Ada name by replacing every `.` with
`__` and lowercasing the identifier, then appends compiler suffixes for
protected operations, elaboration procedures, tasks and runtime helpers. The
in-tree backend implements the user-level spelling and the first-slice
suffixes; everything else is rejected rather than guessed.

No upstream source or compiler binary is vendored. The fixture is authored for
this repository, and GNU `c++filt -s gnat` is used only as a generation-time
oracle. The backend never invokes it.

## Toolchain snapshot

| Field | Value |
|-------|-------|
| Compiler | `GNAT 14.2.0` (`gcc (Debian 14.2.0-19) 14.2.0`) |
| Distribution | Debian `gnat-14` / `gnat-14-x86-64-linux-gnu` / `libgnat-14` `14.2.0-19` |
| Target | `x86_64-linux-gnu` |
| Oracle | `GNU c++filt (GNU Binutils for Debian) 2.44` |
| Retrieved | 2026-11-23 |
| License | Fixture: MIT OR Apache-2.0, same as this crate. GNAT/GCC and Binutils are used as installed tools, not redistributed. |

Upstream documentation used as behavioural reference (not vendored):

- GCC 14.2 `gcc/ada/exp_dbug.ads`, the GNAT external-name encoding:
  <https://gcc.gnu.org/git/?p=gcc.git;a=blob;f=gcc/ada/exp_dbug.ads;hb=refs/tags/releases/gcc-14.2.0>
- GNU Binutils `libiberty` Ada demangler (`cplus-dem.c`, `ada_demangle`),
  consulted read-only as an oracle implementation:
  <https://sourceware.org/git/?p=binutils-gdb.git;a=blob;f=libiberty/cplus-dem.c;hb=binutils-2_44>

The Debian packages do not carry upstream VCS revision metadata, so the
installed version string is the pin. The fixture and the curated corpus are
pinned by SHA256 below.

## Fixture

The fixture is a small Ada project authored for this repository. It covers a
root package, child and grandchild packages, protected operations, a generic
package instance, a generic procedure instance, elaboration procedures, a
task, a record, an enum and overloaded/local subprograms.

| File | SHA256 |
|------|--------|
| `refs/ada/fixture/ada_probe.ads` | `f82a74070b14ef30b41ed6fe26eeb757ca1312ffd330c2494769764df00147b6` |
| `refs/ada/fixture/ada_probe.adb` | `3134ef3ecebb9a3ccfffce9708402f5311aa8136fb0050f792b3213d6d01129b` |
| `refs/ada/fixture/ada_probe-child.ads` | `c3ecf63a25d940f66ddb52d40235d8b31574314602fd27d793db038218359ab7` |
| `refs/ada/fixture/ada_probe-child.adb` | `2890a157f0d5017ca3e4c5c1ef39ce04fab9c423aebfce0f1587d84b1cdd74eb` |
| `refs/ada/fixture/ada_probe-child-grandchild.ads` | `a4a87e58004e1735d25cb3dc9cec8a4088cd99fad9672fb6179683f4fcc67e8d` |
| `refs/ada/fixture/ada_probe-child-grandchild.adb` | `62676588c7552e91e19c75b77ceea8277cac9ae3dada72d425149f3776ae1564` |
| `refs/ada/fixture/main.adb` | `3c2138766de6fbd7ce3c72b41a30be34bfc8d2f811a2bdf1e9dada573bd6b3fc` |

## Extraction

```sh
refs/ada/extract-ada-symbols.sh > /tmp/ada-oracle.tsv
```

The script copies the fixture to a temporary directory, compiles it with
`gnatmake -c -q main.adb`, extracts defined global symbols with
`nm -g --defined-only`, sorts them with `LC_ALL=C sort -u`, and records
`c++filt -s gnat` output for each. It does not write into `refs/`.

The curated corpus is derived from that output by reviewing each symbol against
the documented contract. It is not regenerated automatically, because
`c++filt` is not the contract and several GNAT suffixes are deliberately
unsupported.

## Corpus

`refs/ada/corpus/gnat-14.2-linux-x86_64.tsv` is a tab-separated table:

```text
symbol<TAB>source<TAB>class<TAB>expected<TAB>oracle
```

- `source`: `generated` (from the fixture) or `hand` (hand-authored detection
  and suffix cases).
- `class`:
  - `exact`: `ada::parse` prints `expected`, and `c++filt` agrees.
  - `unsupported`: `ada::parse` must reject; `expected` is empty.
  - `oracle-gap`: `ada::parse` prints `expected`, but `c++filt` echoes the
    input or prints a `<...>` placeholder.
- `oracle`: recorded `c++filt -s gnat` output, for review only.

SHA256: `57f287bed5c2f203d6ad94510f87afcab0ec2fdb9fafcda33a5019129e12276f`.

### Measured counts

| Source | Exact | Unsupported | Oracle gap | Total |
|--------|------:|------------:|-----------:|------:|
| Generated (GNAT 14.2 fixture) | 20 | 22 | 0 | 42 |
| Hand-authored | 11 | 25 | 2 | 38 |
| **Total** | **31** | **47** | **2** | **80** |

The generated exact names cover:

- root-package procedures and functions (`ada_probe__hello`,
  `ada_probe__add`, `ada_probe__alpha`, `ada_probe__beta`,
  `ada_probe__gamma`, `ada_probe__total`);
- child and grandchild packages (`ada_probe__child__do_child`,
  `ada_probe__child__child_value`,
  `ada_probe__child__grandchild__deep`,
  `ada_probe__child__grandchild__deep_value`);
- protected operations (`ada_probe__prot__setN`,
  `ada_probe__prot__setP`, `ada_probe__prot__getN`,
  `ada_probe__prot__getP`);
- a generic package instance (`ada_probe__int_gen__do_it`) and a generic
  procedure instance (`ada_probe__int_swap`);
- elaboration body and spec (`ada_probe___elabb`,
  `ada_probe___elabs`);
- an enum initialization helper whose `N` suffix is the same
  protected-operation suffix (`ada_probe__enumN`);
- a top-level nested subprogram (`ada_probe__nested`).

The generated unsupported names, by class:

| Class | Count | Examples |
|-------|------:|----------|
| Runtime/personality helpers | 3 | `DW.ref.__gnat_personality_v0` |
| Main subprogram rename | 1 | `_ada_main` |
| Package/child elaboration flags (`_E`) | 3 | `ada_probe_E`, `ada_probe__child_E` |
| Enum spec helper (`S`) | 1 | `ada_probe__enumS` |
| Overload disambiguator (`__N`) | 1 | `ada_probe__over__2` |
| Protected-type descriptors | 6 | `ada_probe__protA`, `ada_probe__protVIP` |
| Protected entry barriers (`_B4s`, `_E3s`) | 2 | `ada_probe__prot__wait_B4s` |
| Record initialization helper | 1 | `ada_probe__recIP` |
| Task body/type helpers (`E`, `TB`, `VIP`, `Z`) | 4 | `ada_probe__workerTB`, `ada_probe__workerZ` |

The hand-authored exact cases add the detection-prefix families
(`ada__...`, `system__...`, `gnat__...`), protected `N`/`P` on a
high-confidence name, elaboration suffixes on those prefixes, and one general
`pkg__subp` name (`foo__bar`) that is valid through `ada::parse` but is not
auto-detected. The hand-authored unsupported cases cover task suffixes, `_E`
markers, overload/local disambiguators, unknown uppercase suffixes, malformed
`__` chains, a protected-operation-plus-elaboration combination, entry
barriers and protected-type descriptors.

### Oracle gaps and disagreements

`c++filt -s gnat` cannot demangle the Mach-O platform-prefixed
high-confidence forms:

- `_ada__foo` -> `<_foo>`
- `__ada__foo` -> `<__ada__foo>`

The backend's documented contract prints `ada.foo` for both, so they are
classified `oracle-gap`, never `exact`.

`c++filt` is more permissive than this slice for several suffix classes. The
backend rejects them deliberately and they are classified `unsupported` even
when `c++filt` produces readable text:

- overload disambiguators (`ada__foo__2` -> `ada.foo`);
- local-symbol suffixes (`ada__foo.2` -> `ada.foo`);
- unknown trailing uppercase suffixes (`ada__setX` -> `ada.set`);
- protected entry barriers (`ada__prot__wait_B4s` -> `ada.prot.wait`);
- the main rename (`_ada_main` -> `main`).

Echo and `<...>` output is never treated as exact coverage.

## Detection contract

Automatic detection is precision-only. `Scheme::detect` claims a name as Ada
when it starts with `ada__`, `_ada__`, `__ada__`, `system__` or `gnat__`
(including one or two Mach-O leading underscores for the `ada__` family). This
is a prefix prefilter: `ada::parse` still rejects unsupported bodies, so an
`ada__...TB` task helper is detected as Ada and then fails instead of being
guessed or silently passed through as C. General `pkg__subp` GNAT names are
valid through `ada::parse` but are indistinguishable from plain C, so they are
not claimed. This keeps `foo__bar` a C name and leaves legacy GNU v2 `a__b__c`
to the C++ backend.

## Refresh

1. Update the fixture sources under `refs/ada/fixture/`.
2. Run `refs/ada/extract-ada-symbols.sh > /tmp/ada-oracle.tsv`.
3. Review every symbol against the contract and update
   `refs/ada/corpus/gnat-14.2-linux-x86_64.tsv`.
4. Update the SHA256 values, counts and toolchain versions in this note.
5. Run `cargo test --all-features --test ada_corpus` and
   `cargo test --no-default-features --features ada`.
