# Swift support matrix

Project-owned companion to [`swift.md`](swift.md); it is not upstream material
and is excluded from the published crate with the rest of `refs/`. Update it on
every grammar/toolchain refresh (see the refresh process in `swift.md`).

Status legend:

- **implemented** — parsed and printed, gated by the corpus tests.
- **legacy** — only in the pre-stable `_T` grammar (`legacy.rs`).
- **deferred** — stable grammar, not implemented yet; schedule deliberately.
- **main-only** — in the pinned main grammar but not in the pinned release
  toolchain; not implemented until a release ships it.
- **rejected** — deliberately not accepted (unstable, out of scope, or the
  reference rejects it).

## Input families

| Family | Grammar | Status |
|--------|---------|--------|
| `$s`, `$S`, `_$s`, `_$S` | `mangled-name` | implemented |
| `_T0` | `mangled-name` (Swift 4.0) | implemented |
| `_T` | legacy parser, not in `Mangling.rst` | legacy |
| `$e` Embedded Swift | `mangled-name` | rejected (unstable) |
| `@__swiftmacro_` | `mangled-name` | rejected (filename mangling) |
| `async_Main` | `mangled-name` | main-only (6.4.2 rejects) |

## Payload families

| Family | Grammar | Status |
|--------|---------|--------|
| layout constraints | `LAYOUT-CONSTRAINT` (1149-1160) | implemented: `U R N C D T B E e M m S`; `B`/`S` print empty (upstream printer) |
| macro roles | `macro-expansion-operator` (458-467) | implemented: 8 attached roles, including `q`/preamble |
| autodiff | `differentiable` (813-816), `DIFFERENTIABILITY-KIND` (911-914) | implemented: `f r d l` + `p`; `Yj` rejects other bytes |
| generic specialization | `specialization` (1368-1390) | implemented: `g G B s i t f`; **deferred**: `p P` |
| globals | 134 `global ::=` lines | 94 `Global` variants; gaps below |
| key-path thunks | 279-282 | implemented: getter/setter/unapplied/applied |
| deinitializers | 409-411 | implemented: `fd fD fZ` |
| accessors | `ACCESSOR` (427-…) | implemented: 15 stable codes + legacy `_T` codes |
| function attributes | `entity-spec` (`fo`/`fO`/`fD`/`fd`) | implemented: 5 codes |

## Global operator gaps

The pinned grammar has 134 `global ::=` productions; the implementation
exposes 94 `Global` variants. The grammar cross review classifies each
unmodeled production. Do **not** implement main-only forms without a release
that accepts them.

| Production | Grammar line | Classification |
|------------|--------------|----------------|
| `TC` continuation prototype | 254 | historical/unused (grammar says "not actually used for real symbols") |
| `TB` property behavior initializer thunk | 256 | historical/unused ("not used currently") |
| `Tb` property behavior setter thunk | 257 | historical/unused ("not used currently") |
| remaining unmodeled productions | 142-283 | pending classification by the cross review |

## Policy

- Recording a gap here is not a promise to implement it. Coverage follows
  released toolchains; a main snapshot is a forward reference.
- Unknown operators keep rejecting the symbol. Tolerant parsing (preserving the
  raw symbol as an opaque node) is a possible future mode, not part of the
  current contract.
