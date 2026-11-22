# Go references

Go does not have a demangling format in the C++/Rust sense, and the toolchain
ships no demangler for its own names. The in-tree backend therefore implements
the linker's *name layout*, as defined by the compiler and linker sources
listed below, and the corpus is a set of real symbol names extracted from the
installed toolchain.

## Toolchain snapshot

| Field | Value |
|-------|-------|
| Toolchain | Go 1.24.4 (`go1.24.4`), release `2025-05-29T19:37:36Z` |
| Distribution | Debian `golang-1.24` (`/usr/lib/go-1.24`) |
| Upstream | <https://github.com/golang/go> — tag `go1.24.4` |
| Retrieved | 2026-11-22 |
| License | BSD-3-Clause plus patent grant (`refs/go/LICENSE`) |
| `GOROOT/VERSION` | `go1.24.4` / `time 2025-05-29T19:37:36Z` |
| `bin/go` SHA256 | `8992a5c41a8fefc6bd437117566ecb3f80ac89f78c009ccbb486fa6b9ba85d01` |

**Commit SHA:** the Debian-built toolchain is compiled with `-trimpath` and
without VCS stamping (`go version -m $(go env GOROOT)/bin/go` reports no
`vcs.revision`), so the upstream commit SHA is not available offline. The
release tag `go1.24.4` is immutable; the exact source content used is pinned by
the per-file SHA256 values below.

## Naming rules used by the backend

| File | SHA256 | What it defines |
|------|--------|-----------------|
| `src/cmd/internal/objabi/path.go` | `ff1e056f2f2b3aa1f757bf7be3da769f1f5241da65438e5ebb9f65a6716c8343` | `PathToPrefix`/`PrefixToPath`: `%XX` escaping of import paths |
| `src/cmd/compile/internal/ir/func.go` | `e7b75c82007ff6f791a9d5d643d425a12c5d471960b89be2173f85d51293f02d` | `splitPkg`, `LinkFuncName`, closure naming (`.funcN`, `-rangeN`, `.gowrapN`, `.deferwrapN`, inlined components, `glob..funcN`) |
| `src/cmd/compile/internal/staticdata/data.go` | `5fba14b6a40c3069684e76dcfdfe5b67bf30277dcdbe28e9d3590a7de340bfd3` | `go:string.` symbols and `.gobytes.N` |
| `src/cmd/compile/internal/reflectdata/reflect.go` | `b15421c4414fe7009b72ce0accd7190bc044433d05053f20ddd7b539f59562e1` | `type:.importpath.`, `type:.namedata.`, `type:*`, `type:noalg.`, `go:itab.` |
| `src/cmd/link/internal/ld/symtab.go` | `e33c47f1a349f517af8e08fe2109b8fd23a7d0a7c67daddeeeeb91b895c417f3` | `go:` group symbols, `·` → `.` rewrite, ABI-name mangling, symbol-table filtering |
| `src/cmd/internal/obj/sym.go` | `f4135c48be55998252b23c077835754855998f2b25e30a60e2077f47fe6609f5` | `$f32.`/`$f64.`/`$i32.`/`$i64.`/`$i128.` constants |
| `src/cmd/compile/internal/walk/expr.go` | `70f77f943dea0fff2cde2de3c175fc5710b815c13d305463650fa9a99ce94005` | `.typeAssert.N` |
| `src/cmd/compile/internal/walk/switch.go` | `b0fb9b2d3f988fdef0122f4ddd8cab2d05c4959bed0a9b424dd3922e22b56df2` | `.interfaceSwitch.N` |
| `src/cmd/internal/obj/objfile.go` | `da0c1f3a3e778c5a8d048e4fa11ee334cca8b82a98194c68f377a0c25af3600d` | `gclocals·` object-file symbols |

## Corpora

Both files list one symbol name per line, sorted with `LC_ALL=C sort -u`.
Names containing spaces are kept intact (Go type strings embed them); the
extraction script strips only the `go tool nm` address/type columns.

| File | Names | SHA256 | Source |
|------|-------|--------|--------|
| `refs/go/corpus/go1.24.4-binary.txt` | 3,363 | `97368b6ea5ed6bd96359912e365d5a508782641a81cdb18b26d84f45193e615d` | `go tool nm` on two generated test binaries (default inlining and `-gcflags=all=-l`) |
| `refs/go/corpus/go1.24.4-object.txt` | 79 | `6604ad804f9f78ca70ad18e76ae0cc66c8458b4c3ef94f1c4872a11823f21563` | `go tool nm` on `go tool compile` output for the generated `lib` package |

The binary corpus covers the standard library (`fmt`, `runtime`, `reflect`,
`internal/...`, `sync/atomic`, …) and the generated program: methods and
method values, closures and `defer`/`go`/range wrappers, generic
instantiations and `go.shape` types, `type:` descriptors, `go:` specials
(including `go:itab.*T,I`), ABI markers, `$` constants, compiler data symbols
and a package path whose last segment contains a dot (`pkg%2ev1`).

The object corpus additionally holds the names the linker hides from the final
symbol table: `go:string."…"`, `type:.namedata.…`, `type:.importpath.…`,
`runtime.memequal64·f` and `gclocals·<base64>`.

Of the 3,363 binary names, 3,326 have the shape of a Go name. The remaining 37
are bare assembly or cgo symbols (`aeshashbody`, `_cgo_init`, `debugCall128`,
`gogo`, …) that carry no package and are indistinguishable from C; detection
deliberately leaves them out of Go. All 79 object-file names are Go names.

Only symbol names are vendored; no binaries, object files or Go source are
stored.

## Extraction

```sh
refs/go/extract-go-symbols.sh
```

The script writes the generated program (stdlib imports, methods, closures,
generics, shapes, an interface implementation, a dotted package directory) to a
temporary module, builds it with and without inlining, compiles the library
package to an object file, and normalises `go tool nm` output into the two
corpus files. It warns when the installed toolchain is not `go1.24.4`.

## Refresh

1. Install the new Go release and update the version pin in
   `refs/go/extract-go-symbols.sh`.
2. Run `refs/go/extract-go-symbols.sh`.
3. Update the counts, SHA256 values and `VERSION` timestamp in this file, and
   re-check the per-file SHA256 pins.
4. Run `cargo test --test go_corpus`; raise the floors in the test when
   coverage improves, never lower them.
