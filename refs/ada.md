# Ada/GNAT references

GNAT does not use a C++-style mangling for Ada declarations. It builds an
*external name* from the fully-qualified Ada name by replacing every `.` with
`__` and lowercasing the identifier, then appends compiler suffixes for
protected operations, entry barriers and bodies, task bodies, elaboration
procedures and runtime helpers. The in-tree backend implements the user-level
spelling and the suffixes whose reading is unambiguous; everything else is
rejected rather than guessed.

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
root package, child and grandchild packages, protected operations, a protected
entry family, a generic package instance, a generic procedure instance,
elaboration procedures, a task type, a package-level task object, a record, an
enum and overloaded/local subprograms.

| File | SHA256 |
|------|--------|
| `refs/ada/fixture/ada_probe.ads` | `e80a31022ffb81328b083c3d4573eea327c5fd918b92fbd04e2dd87224449e60` |
| `refs/ada/fixture/ada_probe.adb` | `c3216a404ed05d808883e2807002439afe5c2cca71c5a745ac9b43cc0adc3aa8` |
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

SHA256: `b3edcba71408617ab63d9a7c2d8fd00175cc54f782d889de4f70d6283e5b1a58`.

### Measured counts

| Source | Exact | Unsupported | Oracle gap | Total |
|--------|------:|------------:|-----------:|------:|
| Generated (GNAT 14.2 fixture) | 26 | 27 | 0 | 53 |
| Hand-authored | 17 | 37 | 2 | 56 |
| **Total** | **43** | **64** | **2** | **109** |

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
- protected entry barriers and entry bodies, including the entry family
  `Fam (Idx : Integer)` (`ada_probe__prot__wait_B7s`,
  `ada_probe__prot__wait_E6s`, `ada_probe__prot__fam_B9s`,
  `ada_probe__prot__fam_E8s`);
- the body procedure of the package-level task object
  (`ada_probe__singletonTKB`), next to the task object itself
  (`ada_probe__singleton`);
- a generic package instance (`ada_probe__int_gen__do_it`) and a generic
  procedure instance (`ada_probe__int_swap`);
- elaboration body and spec (`ada_probe___elabb`,
  `ada_probe___elabs`);
- an enum initialization helper whose `N` suffix is the same
  protected-operation suffix (`ada_probe__enumN`).

The generated unsupported names, by class:

| Class | Count | Examples |
|-------|------:|----------|
| Runtime/personality helpers | 3 | `DW.ref.__gnat_personality_v0` |
| Main subprogram rename | 1 | `_ada_main` |
| Package/child elaboration flags (`_E`) | 3 | `ada_probe_E`, `ada_probe__child_E` |
| Package runtime helpers (`___master`, `___chain`) | 2 | `ada_probe___master` |
| Enum spec helper (`S`) | 1 | `ada_probe__enumS` |
| Overload disambiguator (`__N`) | 1 | `ada_probe__over__2` |
| Protected-type descriptors | 6 | `ada_probe__protA`, `ada_probe__protVIP` |
| Protected-object elaboration helper (`_P<n>sIP`) | 1 | `ada_probe__P2sIP` |
| Record initialization helper | 1 | `ada_probe__recIP` |
| Task type body/type helpers (`E`, `TB`, `VIP`, `Z`) | 4 | `ada_probe__workerTB`, `ada_probe__workerZ` |
| Task object helpers (`T1`, `TKE`, `TKVIP`, `TKZ`) | 4 | `ada_probe__singletonTKE`, `ada_probe__singletonT1` |

The hand-authored exact cases add the detection-prefix families
(`ada__...`, `system__...`, `gnat__...`), protected `N`/`P` on a
high-confidence name, entry barrier and body suffixes (with and without a
family number), the task-body suffix, elaboration suffixes on those prefixes,
and one general `pkg__subp` name (`foo__bar`) that is valid through
`ada::parse` but is not auto-detected. The hand-authored unsupported cases
cover task suffixes, `_E` markers, overload/local disambiguators, unknown
uppercase suffixes, malformed `__` chains, a protected-operation-plus-
elaboration combination, malformed entry suffixes, protected-type descriptors
and package runtime helpers.

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

- overload disambiguators (`ada__foo__2` -> `ada.foo`,
  `ada_probe__over__2` -> `ada_probe.over`);
- local-symbol suffixes (`ada__foo.2` -> `ada.foo`,
  `ada__nested__inner.2` -> `ada.nested.inner`);
- unknown trailing uppercase suffixes (`ada__setX` -> `ada.set`);
- the main rename (`_ada_main` -> `main`) and the general `_ada_` escape
  (`_ada_foo` -> `foo`, `_ada_pkg__subp` -> `pkg.subp`);
- operator and controlled-type helpers: `pkg__Oeq` -> `pkg."="`,
  `pkg___assign` -> `pkg.":="`, `pkg___size` -> `pkg'Size`,
  `pkg__tDA`/`pkg__tDF` -> `pkg.t.Adjust`/`pkg.t.Finalize`, and
  `pkg__tSI`/`SO`/`SR`/`SW` -> `pkg.t'Input`/`'Output`/`'Read`/`'Write`.

Echo and `<...>` output is never treated as exact coverage.

### Accepted generated suffixes

Two generated-subprogram suffixes are accepted because `c++filt -s gnat`
renders the entry or task name for them and GNAT 14.2 emits them for the
fixture:

| Form | Generated by | Backend |
|------|--------------|---------|
| `<entry>_B<digits>s` | entry barrier procedure | prints `<entry>` |
| `<entry>_E<digits>s` | entry body procedure | prints `<entry>` |
| `<task>TKB` | body procedure of a task object | prints `<task>` |

The digits in `_B<digits>s`/`_E<digits>s` are the entry's number (they are
absent for some entries: `pkg__prot__wait_Bs` is accepted, `pkg__prot__wait_B4`
is not). The backend does not interpret the number, so the barrier and the
body of one entry print the same name, exactly as `c++filt` does.

### Rejected classes the oracle cannot validate

The classes the plan named as candidates — package/child elaboration flags
(`_E`), task-type bodies and runtime helpers (`_TB`, `_VIP`, `_Z`, `_T1`,
`_TKE`, `_TKVIP`, `_TKZ`, `_E`), protected-type descriptors (`_A`, `_F`,
`_VDF`, `_VDI`, `_VFD`, `_VIP`), the protected-object elaboration helper
(`_P<n>sIP`), record initialization (`_IP`), enum spec helpers (`_S`) and the
package runtime helpers (`___master`, `___chain`) — stay rejected, because
`c++filt -s gnat` prints a `<...>` placeholder for every one of them:

| Symbol | `c++filt -s gnat` |
|--------|-------------------|
| `ada_probe_E` | `<ada_probe_E>` |
| `ada_probe__child_E` | `<ada_probe__child_E>` |
| `ada_probe__workerTB` | `<ada_probe__workerTB>` |
| `ada_probe__workerE` | `<ada_probe__workerE>` |
| `ada_probe__workerVIP` | `<ada_probe__workerVIP>` |
| `ada_probe__workerZ` | `<ada_probe__workerZ>` |
| `ada_probe__singletonT1` | `<ada_probe__singletonT1>` |
| `ada_probe__singletonTKE` | `<ada_probe__singletonTKE>` |
| `ada_probe__protA` | `<ada_probe__protA>` |
| `ada_probe__protVIP` | `<ada_probe__protVIP>` |
| `ada_probe__P2sIP` | `<ada_probe__P2sIP>` |
| `ada_probe__recIP` | `<ada_probe__recIP>` |
| `ada_probe__enumS` | `<ada_probe__enumS>` |
| `ada_probe___master` | `<ada_probe___master>` |
| `ada_probe___chain` | `<ada_probe___chain>` |

Without an oracle or a documented spelling for the generated entity, printing
one would be a guess, so these rows stay `unsupported`. The same rule rejects
the trailing `X` suffix: `c++filt` strips it (`pkg__fooX` -> `pkg.foo`), but no
symbol in the fixture carries it and its meaning is undocumented here.

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
