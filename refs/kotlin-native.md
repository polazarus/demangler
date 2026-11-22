# Kotlin/Native references

Kotlin/Native is an LLVM backend with its own linker-symbol mangling. It is
unrelated to Kotlin/JVM descriptors and Kotlin/JS names. The in-tree backend
implements the mangler's declaration spelling and the LLVM backend's `k*:`
symbol prefixes; the corpus is a set of real symbol names extracted from a
pinned compiler.

## Compiler snapshot

| Field | Value |
|-------|-------|
| Release | Kotlin `v2.4.21`, published `2026-10-08T09:25:15Z` |
| Tag commit | `9180ce6a7a7d3627f88b424717583f28ddca052d` |
| Compiler | `kotlinc-native 2.4.21 (JRE 21.0.13.1+2-1-deb13u1-Debian)` |
| Kotlin/Native | `2.4.21` |
| Distribution | `kotlin-native-prebuilt-linux-x86_64-2.4.21.tar.gz` |
| Upstream | <https://github.com/JetBrains/kotlin/releases/tag/v2.4.21> |
| Archive URL | <https://github.com/JetBrains/kotlin/releases/download/v2.4.21/kotlin-native-prebuilt-linux-x86_64-2.4.21.tar.gz> |
| Archive SHA256 | `4935d8c8da90a2d2a8ff51da7d2c264c0d6026e1d35a8f169e218d9209965678` |
| Retrieved | 2026-11-22 |
| License | Apache-2.0 (`refs/kotlin-native/LICENSE.txt`) |

The archive SHA256 matches the release's
`kotlin-native-prebuilt-linux-x86_64-2.4.21.tar.gz.sha256` asset.

## Mangler sources

The backend follows these files at the pinned tag. They are recorded as links
plus SHA256 values rather than vendored, because the compiler is large and the
symbol names are the material the corpus needs.

| File | SHA256 | What it defines |
|------|--------|-----------------|
| `compiler/ir/serialization.native/src/org/jetbrains/kotlin/backend/konan/serialization/KonanMangler.kt` | `af0b89531051a361fa8efacb2d949e27efb76073009922bf8b8ee6c2f1e139ea` | `KonanManglerIr`, `#suspend` platform mark, return-type mode |
| `compiler/ir/serialization.native/src/org/jetbrains/kotlin/backend/konan/serialization/ObjCFunctionNameMangleComputer.kt` | `c8cd5c8305f0ba09cc16d25bfd7200c0c1d10874a3c12fc3addb7e012379c13b` | `objc:` platform-specific names, `#Constructor`, `#Accessor` |
| `compiler/ir/serialization.common/src/org/jetbrains/kotlin/backend/common/serialization/mangle/BaseKotlinMangleComputer.kt` | `c76914bf54bf76bc51d2519891f69cec256da5f17f891e04d43433e0ededba72` | path/`#`/parameter/type-parameter/return-type order |
| `compiler/ir/serialization.common/src/org/jetbrains/kotlin/backend/common/serialization/mangle/MangleConstant.kt` | `6b5cc380feb437b9bd16be931d78f3b8c4ecf1ec0da60c916edb6dda0db030c0` | separators, markers, `§`, `|`, `...`, `*`, `?`, `@`, `$`, `%` |
| `compiler/ir/serialization.common/src/org/jetbrains/kotlin/backend/common/serialization/mangle/MangleMode.kt` | `c52a38c9219b6acc8ac8933a4feea6f97d8f8fdd13cf8aa17bcb11b666e3d569` | signature/FQ-name modes |
| `compiler/ir/serialization.common/src/org/jetbrains/kotlin/backend/common/serialization/mangle/ir/IrMangleComputer.kt` | `a79d4cf68e601ee957daf1f55bc1d37e0073a16e9e1a3a93c89ad277b8d11c67` | `kfun`/`kclass`/`kfield` symbol names, private-symbol dot separator |
| `compiler/ir/ir.tree/src/org/jetbrains/kotlin/ir/util/KotlinMangler.kt` | `0ba921ea410cf4c1384681cc139196bd65ef3a00e57282a1eb2839255ac03232` | `mangleString` contract |
| `compiler/ir/serialization.common/src/org/jetbrains/kotlin/backend/common/serialization/mangle/mangleUtils.kt` | `6ae3e17d63225de11edb223bee22c59c80197b4edb7dd300b8cba04fb5b2fdfa` | list separator elision |

The LLVM backend's `KonanBinaryInterface` (compiled class in the prebuilt
compiler) defines the actual prefixes: `kfun`, `kclass`, `kfield`, and
`kvar`; `DeclarationsGeneratorVisitor` adds `kvar` for global storage and the
internal table prefixes (`kintf`, `kifacetable`, `kifacevtable`, `kexttype`,
`kextoff`, `kextname`, `krefs`). `BinaryInterfaceKt.replaceSpecialSymbols`
rewrites `@` to `__at__` in final linker symbols.

## Corpora

Each file lists one symbol name per line, sorted with `LC_ALL=C sort -u`.
`refs/` is excluded from the published crate, so corpus tests skip when it is
absent.

| File | Names | SHA256 | Source |
|------|-------|--------|--------|
| `refs/kotlin-native/corpus/kotlin-2.4.21-linux-x86_64.txt` | 1,371 | `fcee7ad5317945b677ba3e8699734ab4b1308ee798d1ab4c73711a0d933cd184` | `nm` on the fixture executable built with Kotlin/Native 2.4.21 |
| `refs/kotlin-native/corpus/hand-authored.txt` | 33 | `d074d56710393bc1ec8ffa9b736e141081848e0b95c0204424f54a5a9cd93e09` | Marker constants and prefix forms not emitted by this executable |

The measured corpus contains 1,136 `kfun:` names, 199 `kclass:` names and 36
`kvar:` names. It covers:

- top-level, member, constructor, accessor and local functions;
- extension functions (`#shout__at__kotlin.String`), suspend functions
  (`#suspend`), static members (`#static`) and virtual trampolines (`-impl`);
- inline function declarations are present in the fixture, but inline bodies
  are lowered away and leave no `kfun:` symbol;
- lambdas, function references, coroutine state machines and compiler bridges
  (`$<bridge-…>`), including private-symbol `.` separators and `#internal`
  suffixes;
- classes, interfaces, objects, companions, enums, sealed hierarchies and
  nested classes;
- generics, type parameters, upper bounds, type arguments, variance,
  nullable, star and vararg types;
- synthetic `$` indexes and the LLVM numeric disambiguator after `#internal`.

Kotlin/Native 2.4.21 executables emit only `kfun:`, `kclass:` and `kvar:`
linker symbols. `ktype:`, `kproperty:`, `kobj:`, `kfield:`, `ktypealias:`,
`kpackage:` and `knamespace:` are implemented from the mangler's marker
constants (`MangleConstant`, `KonanBinaryInterface`, `IrMangleComputer`), but
the pinned executable does not contain them; the hand-authored file covers
those forms, the Mach-O leading-underscore variants and the `%` platform
suffix. `kfield:` is defined by `KonanBinaryInterface.getSymbolName(IrField)`
but fields are not exported by the fixture.

Only symbol names are vendored; no compiler binary, object file, klib or Kotlin
source from the compiler is stored.

## Extraction

```sh
refs/kotlin-native/extract-kotlin-native-symbols.sh
```

The script downloads the pinned prebuilt compiler to `/tmp` (or
`$KOTLIN_NATIVE_CACHE`), verifies its SHA256, compiles
`refs/kotlin-native/fixture/main.kt` with `-Xno-inline -g`, dumps the ELF
symbol table with `nm`, keeps the known `k*:` prefixes, sorts with
`LC_ALL=C sort -u`, and writes the corpus file. It requires `curl`, `tar`,
`sha256sum`, `java` (a JRE is not bundled in the prebuilt archive) and `nm`.

## Refresh

1. Pick a new Kotlin release tag and update `KOTLIN_VERSION`, `KOTLIN_SHA256`
   and the URLs in `refs/kotlin-native/extract-kotlin-native-symbols.sh`.
2. Run the script.
3. Update the counts, SHA256 values, commit SHA, compiler version and
   retrieval date in this file.
4. Run `cargo test --test kotlin_native_corpus`; raise the floors in the test
   when coverage improves, never lower them.
5. Re-check the mangler source SHA256 values if the upstream files changed.
