# Delphi/Object Pascal references

Delphi and Free Pascal do not mangle types in the C++ sense: the compiler
builds a linker name from the symbol's owning scope chain, the symbol name and,
for a routine, one token per parameter type. Free Pascal's scheme is fully
specified by the compiler source and is implemented in-tree (`src/delphi/`),
including the compiler data symbols that carry the same chain behind a fixed
prefix; the classic 32-bit Delphi form is documented here but deliberately not
implemented in this slice (see [Gaps](#gaps)).

No compiler binary or upstream source is vendored. The fixture is authored for
this repository, and the corpus is derived from it with the installed compiler.

## Toolchain snapshot

| Field | Value |
|-------|-------|
| Compiler | Free Pascal `3.2.2` (`fpc -iV`), Debian `fp-compiler-3.2.2` `3.2.2+dfsg-46` |
| Target | `x86_64-linux` (`fpc -iTO -iTP`) |
| Modes covered | `{$mode objfpc}` and `{$mode delphi}` |
| Retrieved | 2026-11-23 |
| License | Fixture: MIT OR Apache-2.0, same as this crate. Free Pascal is used as an installed tool, not redistributed. |

Behavioural references (read-only, not vendored):

- Free Pascal 3.2.2 `compiler/symdef.pas`, functions `make_mangledname` and
  `tabstractprocdef.mangledprocparanames` (the exact grammar this backend
  implements), pinned at tag `release_3_2_2`:
  <https://gitlab.com/freepascal.org/fpc/source/-/blob/release_3_2_2/compiler/symdef.pas>
- Free Pascal 3.2.2 `compiler/psub.pas` and `compiler/pdecsub.pas` for the
  operator and class-constructor name spelling (`$plus`, `$create`).
- radare2 `libr/bin/mangling/pascal.c` (LGPL-3.0-only), an independent
  Free Pascal demangler consulted to cross-check the scope-chain layout:
  <https://github.com/radareorg/radare2/blob/master/libr/bin/mangling/pascal.c>

The mangling algorithm, as implemented here, is:

```text
symbol      := [ data-prefix ] unit [ "$_$" scope ] [ "_$$_" name ]
data-prefix := "U_$" | "VMT_$" | "RTTI_$" | "INIT_$" | "INIT$_$" | "FINALIZE$_$"
unit        := identifier | "P$" identifier        (program marker)
scope       := ( identifier "_$_" )*               (class/record scopes)
name        := identifier signature | entry-name
signature   := ( "$" type )* [ "$$" type ]
entry-name  := "init$" | "finalize$" | "finalize_implicit$"
```

`make_mangledname` appends the unit name, `$_$` plus the scope chain and `_$$_`
plus the symbol name; `mangledprocparanames` appends `$<type>` per parameter
and `$$<type>` for a result. Identifiers are uppercased in the linker name, and
`$` prefixes operator and class-constructor names. Over-long scope chains and
parameter lists are replaced by a CRC32 (`$CRC<hex>` / `$crc<hex>`).

## Compiler data symbols

Free Pascal emits one linker symbol per compiler-generated table and per unit
entry, and each reuses the scope chain of the symbol it belongs to behind a
fixed prefix. `delphi_probe.pas` and `delphi_probe_data.pas` measure them. The
prefix stays in the printed text, because it is the only part of the name that
says which table a symbol is.

| Prefix | Symbol | Example |
|---|---|---|
| `U_$` | unit, class or thread variable | `U_$DELPHI_PROBE_$$_UNITCOUNTER` |
| `VMT_$` | a class's virtual method table | `VMT_$DELPHI_PROBE_$$_TPROBECLASS` |
| `RTTI_$` | the runtime type information of a type | `RTTI_$DELPHI_PROBE_$$_TPROBEENUM_o2s` |
| `INIT_$` | a type's initialization table | `INIT_$DELPHI_PROBE_$$_TPROBECLASS` |
| `INIT$_$` | a unit's initialization entry | `INIT$_$DELPHI_PROBE` |
| `FINALIZE$_$` | a unit's finalization entry | `FINALIZE$_$DELPHI_PROBE` |

Measured facts:

- every `VMT_$`, `RTTI_$` and `INIT_$` table is emitted twice, once plain and
  once with a trailing `$indirect`; `$` is not an identifier byte, so the marker
  cannot be part of a type name. A `U_$` variable has no such variant;
- an enumeration adds `_o2s` and `_s2o` RTTI tables next to its plain one
  (`RTTI_$DELPHI_PROBE_$$_TPROBEENUM_o2s`); those are ordinary identifier bytes,
  so they stay part of the printed token;
- a compiler-generated anonymous type is named `def<hex>`
  (`INIT_$DELPHI_PROBE_$$_def0000001A`);
- the unit part keeps the program marker (`U_$P$DELPHI_PROBE_PROG_$$_G`) and
  namespace dots (`VMT_$DELPHI_PROBE.NS_$$_TNAMESPACED`);
- a nested type keeps the class scope chain
  (`VMT_$DELPHI_PROBE$_$TPROBECLASS_$__$$_TNESTED`);
- the unit entry routines carry a trailing `$` that no Pascal identifier can
  contain: `init$` for a unit with an `initialization` section, `finalize$` for
  a unit whose `finalization` section has a body, and `finalize_implicit$` for a
  unit whose managed variables need an implicit finalizer;
- a generic instantiation keeps its undecodable `$<n>$CRC<hex>` marker in the
  chain, so its tables are rejected like its routines.

A data symbol has no parameter list, and a unit entry has no member at all, so
neither is printed with one.

## Fixture

| File | SHA256 |
|------|--------|
| `refs/delphi/fixture/delphi_probe.pas` | `25e09c52a91bbb5e958401d5a94ce9d27714330ac70782a46632435371d767cd` |
| `refs/delphi/fixture/delphi_probe.ns.pas` | `3c291c8e4363d9c4c59ae970548b7cfd145b7b5cf4281d89eff4d56e91416123` |
| `refs/delphi/fixture/delphi_probe_delphi.pas` | `d3ff095348d92cae579ba56e24137e04b1fce305395ec0a7e55284eff019d33a` |
| `refs/delphi/fixture/delphi_probe_prog.pas` | `12199d7a5cd6fd1e9b5bba738471f737ef24dac75ae111fec564c7f274e52796` |
| `refs/delphi/fixture/delphi_probe_data.pas` | `4abdfcc640b3d78e1aa9f6c51099a416553ddd6d585fec939316629546619d94` |
| `refs/delphi/extract-delphi-symbols.sh` | `32d16b3cd912aa9ae5fb6485e9fa7488ac550c5ca7ac1a2ddf09295e7dd98bc6` |

The fixture covers a unit with classes, a class hierarchy, an advanced record,
a nested type, a generic class and one instantiation, an operator, class
constructor and destructor, overloads (each with a nested routine), a nested
routine of a unit-level routine, a nested routine of a method, a namespaced
unit (`delphi_probe.ns`), a `{$mode delphi}` unit, a program (`P$` marker), and
parameters of every shape Free Pascal accepts (open arrays, `array of const`,
records, pointers, enums, sets, method pointers, ordinal and floating types).
`delphi_probe_data.pas` is the compiler-data unit: one unit variable, one
thread variable and both an initialization and a finalization section with a
body, so the corpus covers `U_$`, the unit entry symbols and the unit-wide
tables.

## Extraction

```sh
refs/delphi/extract-delphi-symbols.sh > /tmp/fpc-symbols.txt
```

The script copies the fixture to a temporary directory, compiles every source
with `fpc -O- -gw`, extracts defined symbols with `nm --defined-only`, drops
local labels (`.L…`) and the C entry points (`main`, `PASCALMAIN`), and sorts
with `LC_ALL=C`. It writes nothing into `refs/`.

The curated corpus is derived from that output by reviewing each symbol against
the display contract below. It is not generated automatically: no Delphi or
Free Pascal demangler exists to serve as an oracle, so the expected text is a
reviewed project contract, not another tool's output.

## Corpus

`refs/delphi/corpus/fpc-3.2.2-linux-x86_64.tsv` is a tab-separated table:

```text
symbol<TAB>source<TAB>class<TAB>expected<TAB>note
```

- `source`: `generated` (from the fixture) or `hand` (hand-authored detection
  and boundary cases).
- `class`:
  - `exact`: `delphi::parse` prints `expected`.
  - `unsupported`: `delphi::parse` must reject; `expected` is empty.
- `note`: what the row demonstrates.

SHA256: `1aa12105dc28147cf1ab002ad9ebaf1fdcd852d7cf503096ea6957115ebe8456`.

### Measured counts

| Source | Exact | Unsupported | Total |
|--------|------:|------------:|------:|
| Generated (Free Pascal 3.2.2 fixture) | 92 | 36 | 128 |
| Hand-authored | 0 | 15 | 15 |
| **Total** | **92** | **51** | **143** |

The generated exact rows cover:

- unit-scope procedures, functions with a result, and overloads
  (`DELPHI_PROBE_$$_UNITPROC$LONGINT`,
  `DELPHI_PROBE_$$_UNITFUNC$ANSISTRING$$ANSISTRING`,
  `DELPHI_PROBE_$$_OVERLOADED$LONGINT`, `…$ANSISTRING`);
- methods of classes and of an advanced record, including constructor,
  destructor, class constructor, class destructor and class function
  (`DELPHI_PROBE$_$TPROBECLASS_$__$$_CREATE$$TPROBECLASS`,
  `…_$createclass`, `…_$destroyclass`, `DELPHI_PROBE$_$TPROBEREC_$__$$_MOVEBY…`);
- a nested-type scope chain (`DELPHI_PROBE$_$TPROBECLASS_$_TNESTED_$__$$_DEEP`);
- an operator name (`DELPHI_PROBE_$$_$plus$TPROBEREC$TPROBEREC$$TPROBEREC`);
- parameter shapes: open array, open array of `const`, record, pointer, enum,
  set, method pointer, and a seven-parameter signature;
- a namespaced unit (`DELPHI_PROBE.NS_$$_NAMESPACEDPROC$ANSISTRING`), a
  `{$mode delphi}` unit, and a program (`P$DELPHI_PROBE_PROG_$$_PROGPROC$LONGINT`);
- a name long enough to stay under the 100-byte scope-chain CRC limit;
- the compiler data symbols: `U_$` unit variables (including the program marker
  form `U_$P$…` and a thread variable), `VMT_$`, `RTTI_$` and `INIT_$` tables
  with and without the `$indirect` variant, the enum `_o2s`/`_s2o` RTTI tables,
  the unit entries `INIT$_$`/`FINALIZE$_$` and the entry routines `init$` and
  `finalize$`/`finalize_implicit$` (see [Compiler data symbols](#compiler-data-symbols)).

The generated unsupported rows, by class:

| Class | Count | Examples |
|-------|------:|----------|
| Nested routines (ambiguous `_`-joined chain) | 4 | `DELPHI_PROBE$_$NESTEDOWNER_$$_LOCALPROC$LONGINT` |
| CRC substitutions (`$CRC`/`$crc`) | 2 | `DELPHI_PROBE$_$$CRCC737F8A3_$$_LOCALOFLONGPROCEDURE` |
| Generic instantiations (`$<n>$CRC<hex>`) | 2 | `DELPHI_PROBE_$$_USESGENERIC$TPROBEBOX$1$CRC9F312717` |
| Unit/program entries and debug markers | 7 | `DELPHI_PROBE_$$_init$`, `DEBUGSTART_$DELPHI_PROBE` |
| Initialization and RTTI tables | 4 | `INIT_$DELPHI_PROBE_$$_TPROBECLASS`, `RTTI_$…_o2s` |
| Virtual method tables | 2 | `VMT_$DELPHI_PROBE_$$_TPROBECLASS` |
| Unit variables | 1 | `U_$DELPHI_PROBE_$$_UNITCOUNTER` |
| Compiler runtime tables | 4 | `FPC_THREADVARTABLES`, `__fpc_ident` |

The hand-authored unsupported rows cover the classic Delphi form
(`@System@@LStrClr$QQRPV`, `@System@TObject@Create`, `@Unit@Name`), malformed
Free Pascal names (empty routine, trailing separators, a missing class scope
separator, a lowercase type token, the ambiguous `$FILE$OF$` sequence and the
`error` placeholder), and names of other schemes (`main`, `foo__bar`,
`_Z3foov`, `_RNvNtCs1234_7mycrate3foo`, `kfun:com.example.Foo#bar`).

There are no hand-authored `exact` rows: every supported form is generated from
the fixture, and the one form that would need hand-authored expectations (the
classic Delphi signature) is not implemented.

## Display contract

```text
<unit>[.<class>…].<routine>(<type>, …)[: <result>]
```

| Input | Output |
|---|---|
| `DELPHI_PROBE_$$_NOARGS` | `DELPHI_PROBE.NOARGS()` |
| `DELPHI_PROBE_$$_UNITPROC$LONGINT` | `DELPHI_PROBE.UNITPROC(LONGINT)` |
| `DELPHI_PROBE_$$_UNITFUNC$ANSISTRING$$ANSISTRING` | `DELPHI_PROBE.UNITFUNC(ANSISTRING): ANSISTRING` |
| `DELPHI_PROBE$_$TPROBECLASS_$__$$_ADD$LONGINT$LONGINT$$LONGINT` | `DELPHI_PROBE.TPROBECLASS.ADD(LONGINT, LONGINT): LONGINT` |
| `DELPHI_PROBE$_$TPROBECLASS_$_TNESTED_$__$$_DEEP` | `DELPHI_PROBE.TPROBECLASS.TNESTED.DEEP()` |
| `DELPHI_PROBE_$$_UNITOPENARRAY$array_of_ANSISTRING` | `DELPHI_PROBE.UNITOPENARRAY(array of ANSISTRING)` |
| `DELPHI_PROBE.NS_$$_NAMESPACEDPROC$ANSISTRING` | `DELPHI_PROBE.NS.NAMESPACEDPROC(ANSISTRING)` |
| `P$DELPHI_PROBE_PROG_$$_PROGPROC$LONGINT` | `P$DELPHI_PROBE_PROG.PROGPROC(LONGINT)` |
| `DELPHI_PROBE_$$_$plus$TPROBEREC$TPROBEREC$$TPROBEREC` | `DELPHI_PROBE.$plus(TPROBEREC, TPROBEREC): TPROBEREC` |
| `U_$DELPHI_PROBE_$$_UNITCOUNTER` | `U_$DELPHI_PROBE.UNITCOUNTER` |
| `VMT_$DELPHI_PROBE_$$_TPROBECLASS$indirect` | `VMT_$DELPHI_PROBE.TPROBECLASS$indirect` |
| `RTTI_$DELPHI_PROBE_$$_TPROBEENUM_o2s` | `RTTI_$DELPHI_PROBE.TPROBEENUM_o2s` |
| `VMT_$DELPHI_PROBE$_$TPROBECLASS_$__$$_TNESTED` | `VMT_$DELPHI_PROBE.TPROBECLASS.TNESTED` |
| `INIT$_$DELPHI_PROBE` | `INIT$_$DELPHI_PROBE` |
| `DELPHI_PROBE_$$_init$` | `DELPHI_PROBE.init$()` |

Every name and type token is printed as mangled. Free Pascal uppercases
identifiers in linker names, so the declared spelling (`UnitProc`, `LongInt`)
is not recoverable, and neither is a type alias (`Cardinal` mangles as
`LONGWORD`). Operator and class-constructor names keep their `$` prefix. The
program marker `P$` is kept so a program and a unit of the same name stay
distinguishable. `array_of_X` prints as `array of X` and `array_of_const` as
`array of const`. A compiler data symbol keeps its prefix and its `$indirect`
marker, and a unit entry has no member to print, so the text names the unit
alone.

A Mach-O platform underscore is not stripped: Free Pascal prepends `_` to every
symbol on Darwin, and `_DELPHI_PROBE_$$_…` is indistinguishable from an
identifier that starts with `_`, so the underscore stays part of the printed
unit name.

Same-named types from different units are indistinguishable in Free Pascal
linker names: `qc.usesboth(qa.tfoo, qb.tfoo)` mangles as
`QC_$$_USEBOTH$TFOO$TFOO`. The printed text repeats the token, which is all the
linker name carries.

## Detection contract

Free Pascal names have no unambiguous prefix, so `Scheme::detect` claims one
only after the whole grammar parses. Ordinary C names never contain `$` and are
never claimed; malformed Free Pascal names are not claimed either, because
detection is a full parse rather than a prefix test.

A namespaced unit puts a `.` in the linker name (`DELPHI_PROBE.NS_$$_…`).
Detection runs Delphi before Go (a Go linker name never contains `_$$_`), so
these names are claimed end to end. A compiler data prefix ends with `$`, which
no unit name can contain, so `U_$`, `VMT_$`, `RTTI_$`, `INIT_$`, `INIT$_$` and
`FINALIZE$_$` cannot be confused with a unit that happens to start with the same
letters.

## Gaps

Rejected rather than guessed:

- local routines nested in routines: `make_mangledname` joins the enclosing
  routines with `_`, which is also the identifier separator, so
  `outer_local_$$_inner` cannot be split without guessing;
- generic instantiations, whose scope or type token carries an undecodable
  `$<n>$CRC<hex>` marker;
- the `$crc`/`$CRC` substitutions, which replace a parameter list or scope
  chain with a CRC32, including the `VMT_$`/`RTTI_$`/`INIT_$` table of a generic
  instantiation, whose type token carries the marker;
- the compiler symbols that name a table of the whole unit rather than one of
  its types or variables: the debug markers `DEBUGSTART_$`/`DEBUGEND_$`, the
  thread-variable tables `THREADVARLIST_$`/`THREADVARLIST_$…$indirect` and the
  `DBG_$` group, plus the runtime tables (`FPC_*`, `INITFINAL`, `__fpc_*`,
  `__stklen`, `__heapsize`);
- `file of X` parameter tokens (`FILE$OF$X`): Free Pascal rejects inline file
  types in parameter lists, and the token sequence is byte-identical to three
  parameters named `FILE`, `OF` and `X`, so `FILE` and `OF` tokens are rejected;
- anonymous procedural-type tokens (`$_nestedprovar…_$`) and the `error`
  placeholder;
- the classic 32-bit Delphi form.

### Classic 32-bit Delphi form

Delphi (and C++Builder) mangle native code as
`@<unit>@<class>@<name>$<signature>`, for example
`@System@@LStrClr$QQRPV`. Two things block a first slice here:

1. The `$`-signature encodes the calling convention and the parameter and
   result types with vendor codes. No Delphi or C++Builder compiler is
   installed here, no permissively licensed reference implementation is
   available offline, and there is no oracle to check a decoding against.
   Implementing it from memory would violate the "reject unknown suffixes
   rather than guess" rule.
2. `Scheme::detect` strips a GNU `@version` suffix before a backend is asked,
   which turns a leading-`@` Delphi name into an empty string. The form needs
   the raw-input check that COFF and MSVC already use, in `src/scheme.rs` and
   `src/demangler.rs`.

Both belong to a later slice together with a Delphi fixture.

## Refresh

1. Update the fixture sources under `refs/delphi/fixture/`.
2. Run `refs/delphi/extract-delphi-symbols.sh > /tmp/fpc-symbols.txt`.
3. Review every symbol against the display contract and update
   `refs/delphi/corpus/fpc-3.2.2-linux-x86_64.tsv`.
4. Update the SHA256 values, the measured counts and the toolchain versions in
   this note, and the `COUNTS` constant in `tests/delphi_corpus.rs`.
5. Run `cargo test --all-features --test delphi_corpus` and
   `cargo test --no-default-features --features delphi`.
