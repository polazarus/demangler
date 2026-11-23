# Fortran references

Fortran has no standard mangling. gfortran (GNU Fortran) decorates a lowercased
Fortran name in one of three ways: a module entity gets a `__<module>_MOD_`
prefix, an external procedure or common block gets one appended underscore
(`-funderscoring`, the default), and the main program is the fixed name
`MAIN__`. Other compilers use unrelated schemes (Intel `m_mp_x_`, NAG, LLVM
Flang `_QMmEx`), so this backend implements the gfortran scheme only.

No demangler for gfortran symbols exists in GNU Binutils: `c++filt` has no
Fortran mode, so there is **no oracle** for the expected text. The display form
is a project contract, documented below and in `src/fortran/mod.rs`. No
compiler binary, GCC source tree or runtime library is vendored; the fixture is
authored for this repository.

## Toolchain snapshot

| Field | Value |
|-------|-------|
| Compiler | `GNU Fortran (Debian 14.2.0-19) 14.2.0` |
| Distribution | Debian `gfortran-14` `14.2.0-19` |
| Target | `x86_64-linux-gnu` |
| Symbol reader | `GNU nm (GNU Binutils for Debian) 2.44` |
| Retrieved | 2026-11-23 |
| License | Fixture, corpus and script: MIT OR Apache-2.0, same as this crate. GCC and Binutils are used as installed tools, not redistributed. |

Upstream sources used as behavioural reference (read-only, not vendored):

- GCC 14.2 `gcc/fortran/trans-decl.cc`, the gfortran symbol mangling:
  `sym_identifier` (`MAIN__`), `mangled_identifier`
  (`__<module>_MOD_<name>`), and `gfc_sym_mangled_function_id`
  (`<name>_`, `<name>__` with `-fsecond-underscore`, the unmangled
  `-fno-underscoring` and `bind(c)` forms):
  <https://gcc.gnu.org/git/?p=gcc.git;a=blob_plain;f=gcc/fortran/trans-decl.cc;hb=refs/tags/releases/gcc-14.2.0>
- GCC 14.2 `gcc/fortran/invoke.texi`, `-funderscoring`,
  `-fno-underscoring`, `-fsecond-underscore` (implied by `-ff2c`):
  <https://gcc.gnu.org/git/?p=gcc.git;a=blob_plain;f=gcc/fortran/invoke.texi;hb=refs/tags/releases/gcc-14.2.0>

The Debian packages do not carry upstream VCS revision metadata, so the
installed version string is the pin. The fixture, corpus and script are pinned
by SHA256 below.

## Fixture

A small Fortran project authored for this repository. It covers module
procedures, module variables, a derived type with a type-bound procedure and a
finalizer, two further derived types (one whose name contains `_`, one written
in mixed case), a generic interface, a module-level `entry`, an elemental procedure,
an internal procedure, a `bind(c)` procedure, a child module, a generic module
with a procedure pointer, external procedures (including one whose name ends
with `_`), a block-data unit, a named common block and a main program.

| File | SHA256 |
|------|--------|
| `refs/fortran/fixture/fprobe.f90` | `676e7f10689982f06ea33fffbf46de99ace089b5e8bb934dacaa2f9f395b00a6` |
| `refs/fortran/fixture/fprobe_child.f90` | `4ac383ebea362764ccabd4a480706e995978ea61db7687ec7bb3b4b00ca4a976` |
| `refs/fortran/fixture/fprobe_generic.f90` | `173e7852a7222963892390ae8579aa86334b8fe62aeee0975026b80ae1076beb` |
| `refs/fortran/fixture/probe_ext.f90` | `fdc78a355549731c232aff7a87ae2238644dbc0b231a872a159f20e6a26401af` |
| `refs/fortran/fixture/probe_main.f90` | `02ef5c92d463502bb485b9b93720a173d606bd3eb9ee60b2ea6bbe3721bd1d75` |
| `refs/fortran/corpus/gfortran-14.2-linux-x86_64.tsv` | `c85b50f1b8d2a798196d335066bcade735a9f2de91768c453404de3100604e45` |
| `refs/fortran/extract-fortran-symbols.sh` | `f20349664e626129b00a11594a2e97fe2ad1958d5d24843dd58317cdaf9e4730` |

## Extraction

```sh
refs/fortran/extract-fortran-symbols.sh > /tmp/fortran-symbols.tsv
```

The script copies the fixture to a temporary directory, compiles it three times
and dumps every defined symbol with `nm --defined-only`:

| Mode | Flags | External name for `ext_sub` |
|------|-------|------------------------------|
| `default` | none | `ext_sub_` |
| `second-underscore` | `-fsecond-underscore` | `ext_sub__` |
| `no-underscoring` | `-fno-underscoring` | `ext_sub` |

Module symbols are identical in all three modes. Output is
`mode<TAB>object<TAB>type<TAB>symbol`, sorted; the script never writes into
`refs/`. The curated corpus is reviewed from that output and is not generated
automatically, because the expected text is a project contract rather than an
oracle result.

## Corpus

`refs/fortran/corpus/gfortran-14.2-linux-x86_64.tsv` is a tab-separated table:

```text
symbol<TAB>source<TAB>class<TAB>expected
```

- `source`: the underscoring mode the symbol was compiled with (`default`,
  `second-underscore`, `no-underscoring`) or `hand` for the hand-authored
  detection and rejection cases.
- `class`:
  - `exact`: `fortran::parse` prints `expected`.
  - `unsupported`: `fortran::parse` must reject; `expected` is empty.

There is no `oracle` column: `c++filt` has no Fortran mode, so every expected
text is the documented contract.

### Measured counts

| Source | Exact | Unsupported | Total |
|--------|------:|------------:|------:|
| Generated, `default` | 36 | 6 | 42 |
| Generated, `second-underscore` | 0 | 8 | 8 |
| Generated, `no-underscoring` | 0 | 7 | 7 |
| Hand-authored | 19 | 22 | 41 |
| **Total** | **55** | **43** | **98** |

The 36 generated `default` exact names cover:

- module procedures and functions (`__fprobe_MOD_hello`, `__fprobe_MOD_add`,
  `__fprobe_MOD_outer`, `__fprobe_MOD_point_norm`, `__fprobe_MOD_point_final`,
  `__fprobe_MOD_total_one`, `__fprobe_MOD_total_two`, `__fprobe_MOD_twice`,
  `__fprobe_MOD_with_entry`, `__fprobe_generic_MOD_set_op`,
  `__fprobe_child_MOD_do_child`, `__fprobe_child_MOD_child_value`);
- a module-level `entry` (`__fprobe_MOD_with_entry2`) and generic-specific
  procedures (`__fprobe_generic_MOD_apply_real`,
  `__fprobe_generic_MOD_apply_int`);
- module variables, including a procedure pointer
  (`__fprobe_MOD_counter`, `__fprobe_MOD_scale_factor`,
  `__fprobe_generic_MOD_op`);
- a member whose name ends with `_` (`__fprobe_MOD_trailing_`);
- the ten generated derived-type helpers of `point`, `my_type` and
  `CamelCase` (`__fprobe_MOD___vtab_fprobe_Point`,
  `__fprobe_MOD___def_init_fprobe_My_type`,
  `__fprobe_MOD___copy_fprobe_Camelcase`,
  `__fprobe_MOD___final_fprobe_Point`);
- external procedures (`ext_sub_`, `my_ext_sub_`, `ext_fun_`,
  `ext_with_digits_1_`), a named common block (`probe_blk_`) and a block-data
  unit (`probe_block_`);
- the main program (`MAIN__`).

The 43 unsupported rows, by class:

| Class | Count | Examples |
|-------|------:|----------|
| Derived-type helper shape rejects | 6 | `__holder_MOD___vtab___iso_c_binding_C_ptr`, `__outer.inner_MOD___vtab_outer.inner_Deep_t` |
| `bind(c)` names | 2 | `c_proc_binding`, `ext_c` |
| Local internal/host symbols | 4 | `inner.0`, `master.0.with_entry_`, `master.0.with_entry__` |
| C `main` wrapper | 1 | `main` |
| Ambiguous trailing `__` | 10 | `ext_underscored__`, `my_ext_sub__`, `probe_block__`, `x_y__`, `a_b__` |
| `-fno-underscoring` externals | 7 | `ext_sub`, `my_ext_sub`, `probe_blk`, `foo` |
| Uppercase / malformed | 12 | `Foo_`, `MAIN___`, `__m_MOD_X`, `__MOD_x`, `m_MOD_x`, `__m_MOD_x.y` |
| Mach-O external | 1 | `__ext_sub_` |

### Why the trailing-underscore rule is conservative

`gfc_sym_mangled_function_id` appends one underscore, or two when
`-fsecond-underscore` (implied by `-ff2c`) is active **and** the name already
contains an underscore. Both modes can therefore produce a trailing `__`:

| Mangled | Default reading | Second-underscore reading | Backend |
|---------|-----------------|---------------------------|---------|
| `ext_sub_` | `ext_sub` | impossible (`ext_sub` contains `_` -> `ext_sub__`) | `ext_sub` |
| `extd__` | `extd_` | impossible (`extd` has no `_`) | `extd_` |
| `x_y__` | `x_y_` | `x_y` | rejected: ambiguous |
| `ext_sub` | impossible | impossible | rejected: `-fno-underscoring` |

`-fno-underscoring` is a global mode switch with no per-symbol evidence, so an
underscored name is always read as the default mode. One consequence is
recorded here rather than in the corpus: a `-fno-underscoring` name that
happens to end with `_` (for example `ext_underscored_`) has the shape of a
default-underscored name and is read as `ext_underscored`.

### Other rejected gfortran productions

`trans-decl.cc` emits three further module-related forms that this slice
rejects, because their reading is a compiler internal rather than a Fortran
entity:

- `__<module>_MOD__<proc>_PROC_<name>`: an entity local to a module procedure
  that appears in a result specification expression;
- `.__<module>_MOD_<name>` and, for a saved length variable,
  `GFC_PREFIX("<module>_MOD_<name>")`: character length variables of
  deferred-length strings. The automatic form starts with `.`, and the static
  form has neither the `__` prefix nor a trailing underscore;
- `__<module>_MOD___vtab_*`, `___def_init_*`, `___copy_*`, `___final_*`: the
  generated derived-type helpers are read only in the shape documented under
  [Derived-type helpers](#derived-type-helpers); the remaining variants (a type
  imported from another module, a submodule type) are rejected.

The charlen and `_PROC_` forms do not appear in the fixture; they are rejected
by the identifier rule (a member must be a lowercase Fortran identifier, so it
cannot start with `_` or contain an uppercase marker).

## Display contract

| Input | Output | Rule |
|-------|--------|------|
| `__fprobe_MOD_add` | `fprobe.add` | `_MOD_` becomes `.` |
| `__fprobe_child_MOD_do_child` | `fprobe_child.do_child` | same |
| `__my_mod__MOD_var_x_` | `my_mod_.var_x_` | module/member names are verbatim |
| `__fprobe_MOD___vtab_fprobe_Point` | `fprobe.Point'vtab` | helper of type `point` |
| `__fprobe_MOD___def_init_fprobe_My_type` | `fprobe.My_type'def_init` | helper of type `my_type` |
| `___fprobe_MOD_add` | `fprobe.add` | one Mach-O platform underscore is stripped |
| `ext_sub_` | `ext_sub` | the appended underscore is dropped |
| `extd__` | `extd_` | one appended underscore is dropped |
| `MAIN__` | `main` | fixed main-program name |

The spelling is a debugger name, not re-parsable Fortran source. Module and
member names are printed verbatim because gfortran does not underscore them,
and an identifier that cannot be a gfortran name is escaped as `\xNN`.

### Derived-type helpers

A derived type declared at module level gets four generated helpers whose
symbol is `__<module>_MOD___<helper>_<module>_<Type>`:

| Helper | Emitted for | Output |
|--------|-------------|--------|
| `__vtab_` | every module-level type | `<module>.<Type>'vtab` |
| `__def_init_` | every module-level type | `<module>.<Type>'def_init` |
| `__copy_` | every module-level type | `<module>.<Type>'copy` |
| `__final_` | types with a `final` binding | `<module>.<Type>'final` |

`<Type>` is not the lowercase Fortran name: gfortran records the type name
with its first letter upper-cased and the rest lower-cased, and prefixes the
declaring module. The fixture shows the rule for three spellings, and a
scratch module compiled with the same gfortran shows it for a `bind(c)` type:

| Declared | Helper type part |
|----------|------------------|
| `type :: point` | `fprobe_Point` |
| `type :: my_type` | `fprobe_My_type` |
| `type :: CamelCase` | `fprobe_Camelcase` |
| `type, bind(c) :: c_t` | `holder_C_t` |

The parser accepts a helper only when the type part is exactly
`<symbol module>_<Type>` with `<Type>` matching `[A-Z][a-z0-9_]*`. That is what
makes the reading unambiguous, and it is why three generated variants are
rejected rather than guessed:

- a type imported from `iso_c_binding` produces
  `__holder_MOD___copy___iso_c_binding_C_ptr`, whose type part does not start
  with the symbol's module;
- a type declared in a submodule produces
  `__outer.inner_MOD___vtab_outer.inner_Deep_t`, whose module name contains a
  `.` and is therefore not a module symbol this slice reads;
- a type declared inside a module procedure gets no helper at all: gfortran
  14.2 rejects a `final` binding on such a type, and a local type without one
  produces no `__vtab_`/`__copy_` symbol.

A parameterized derived-type instantiation such as
`__holder_MOD___vtab_holder_Pdtkdt_2` keeps the shape and is therefore read as
`holder.Pdtkdt_2'vtab`; the type part is printed verbatim, and an instantiated
name that leaves the `[A-Z][a-z0-9_]*` shape is rejected.

## Detection contract

Automatic detection is precision-only: `Scheme::detect` claims a symbol as
Fortran only when it has the module form `__<module>_MOD_<member>` (module a
lowercase identifier, member non-empty), including the Mach-O spelling
`___<module>_MOD_<member>` and after ThinLTO `@version`/`.llvm.<hex>`
decoration stripping.

External names are **not** claimed. `foo_` is an ordinary C name as far as a
symbol table is concerned, and claiming every trailing-underscore name would
steal ordinary C symbols. External names, `MAIN__` and the explicit module
forms are available through `fortran::parse`. The module form is a shape
prefilter: a module symbol whose member this slice does not read is claimed as
Fortran and then rejected by `parse`, so it never renders as a C name.

## Refresh

1. Update the fixture sources under `refs/fortran/fixture/`.
2. Run `refs/fortran/extract-fortran-symbols.sh > /tmp/fortran-symbols.tsv`.
3. Review every symbol against the contract and update
   `refs/fortran/corpus/gfortran-14.2-linux-x86_64.tsv`.
4. Update the SHA256 values, counts and toolchain versions in this note.
5. Update `COUNTS` in `tests/fortran_corpus.rs`; raise a count only when
   coverage improves.
6. Run `cargo test --all-features --test fortran_corpus` and
   `cargo test --no-default-features --features fortran`.
