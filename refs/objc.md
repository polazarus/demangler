# Objective-C references

Objective-C has no single C++-style mangling. Linker symbol tables expose
runtime metadata under a small set of documented prefixes, and developer tools
also print method display forms such as `-[NSObject init]`. The in-tree backend
implements those name families; this note records the layout and the provenance
of the symbol-name corpora.

No upstream source, header, object file or SDK is vendored. The corpora contain
only symbol-name facts extracted from locally generated objects and from the
Debian GCC runtime archive.

## Runtime layout

| Runtime | Forms |
|---------|-------|
| Apple `objc4` / Mach-O | `_OBJC_CLASS_$_Class`, `_OBJC_METACLASS_$_Class`, `_OBJC_IVAR_$_Class.ivar`, `__OBJC_$_INSTANCE_METHODS_Class`, `__OBJC_$_CLASS_METHODS_Class`, `__OBJC_$_CATEGORY_Class_$_Category`, `__OBJC_$_PROP_LIST_Class`, `__OBJC_$_INSTANCE_VARIABLES_Class`, `__OBJC_PROTOCOL_$_Protocol`, `__OBJC_PROTOCOL_REFERENCE_$_Protocol`, `__OBJC_LABEL_PROTOCOL_$_Protocol`, `__OBJC_CLASS_RO_$_Class`, `__OBJC_METACLASS_RO_$_Class`, `__OBJC_CLASS_PROTOCOLS_$_Class`, `__OBJC_CATEGORY_PROTOCOLS_$_Class_$_Category`, `_OBJC_METH_VAR_NAME_N`, `_OBJC_METH_VAR_TYPE_N`, `_OBJC_SELECTOR_REFERENCES_`, `_OBJC_SELECTOR_TABLE`, `_OBJC_SYMBOLS`, `_OBJC_Module` |
| GNUstep ABI 2 (observed, Clang 19) | ELF: `._OBJC_CLASS_Class`, `._OBJC_METACLASS_Class`, `._OBJC_REF_CLASS_Class`, `._OBJC_INIT_CLASS_Class`, `._OBJC_PROTOCOL_Protocol`; COFF: `$_OBJC_…` (x86_64) and `_$_OBJC_…` (i686) for the same records |
| GNUstep ABI 2 (documented aliases) | `_OBJC_REF_CLASS_Class`, `_OBJC_INIT_CLASS_Class`, `_OBJC_PROTOCOL_Protocol` without the object-format decoration; no target in the pinned snapshot emits them |
| GNU runtime / GNUstep 1.x | `_OBJC_CLASS_Class`, `_OBJC_METACLASS_Class`, `__objc_class_name_Class`, `__objc_class_ref_Class`, `__objc_ivar_offset_Class.ivar`, `__objc_ivar_offset_value_Class.ivar` |
| Old GNU/NeXT metadata | `_OBJC_Class_Class`, `_OBJC_ClassName_Class`, `_OBJC_MetaClass_Class`, `_OBJC_InstanceMethods_Class`, `_OBJC_ClassMethods_Class`, `_OBJC_InstanceIvars_Class`, `_OBJC_ClassIvars_Class`, `_OBJC_Module`, `_OBJC_SYMBOLS`, `_OBJC_SELECTOR_TABLE`, `_OBJC_METH_VAR_NAME_N`, `_OBJC_METH_VAR_TYPE_N` |
| Blocks | `__block_descriptor_…` (ELF), `___block_descriptor_…` (Mach-O) |

Apple's runtime separator is `_$_`; it is the only structural separator the
parser relies on. Class, category and protocol names are kept verbatim and are
never split on `_` or `$`. Metadata prefixes are case-sensitive.

GNUstep ABI 2 metadata is decorated by the object format (see
[GNUstep ABI 2 decoration](#gnustep-abi-2-decoration)); the decoration is
stripped before the namespace is matched and never reaches the output.

Links (read-only, not vendored):

- Apple `objc4`: <https://github.com/apple-oss-distributions/objc4>
- GNUstep `libobjc2`: <https://github.com/gnustep/libobjc2>
- GCC Objective-C runtime: <https://gcc.gnu.org/onlinedocs/gcc/Objective-C.html>

## Toolchain snapshot

| Field | Value |
|-------|-------|
| Clang | Debian `clang` 19.1.7 (3+b1) |
| LLVM | 19.1.7 |
| Apple target | `x86_64-apple-macosx10.15` (Mach-O object only; no SDK headers) |
| COFF targets | `x86_64-w64-mingw32`, `i686-w64-mingw32` (object only; no sysroot) |
| GNU runtime | Debian GCC 14.2.0-19, `libobjc.a` at `/usr/lib/gcc/x86_64-linux-gnu/14/libobjc.a` |
| GNUstep runtime | Clang `-fobjc-runtime=gnustep-2.0` and `-fobjc-runtime=gcc` |
| Symbol reader | `llvm-nm-19` for Mach-O, GNU `nm` for ELF and COFF |
| Retrieved | 2026-11-22 |

**Commit SHA:** the Debian Clang/GCC packages are release builds without VCS
stamping. The versions above and the per-corpus SHA256 values pin the exact
symbol-name content. No upstream source is vendored, so no upstream license
file is copied; the license statements below apply to the tools whose output
was read.

Licenses:

- Clang/LLVM: Apache-2.0 WITH LLVM-exception. The generated objects contain
  code compiled from the hand-written test programs in
  `refs/objc/extract-objc-symbols.sh`; only their symbol names are stored.
- GCC `libobjc`: GPL-3.0-or-later WITH GCC-Runtime-Library-Exception. Only
  symbol names from the installed archive are stored; no runtime source or
  object is vendored.
- GNUstep naming: the runtime ABI is documented by the GNUstep project; only
  symbol names emitted by Clang are stored.

Symbol names are facts about an interface, not copyrightable expression. No
Apple SDK, GPL test suite or runtime source is copied.

## Corpora

Each file lists one symbol name per line, sorted with `LC_ALL=C sort -u`.
`?` and the `\x01` separator in block descriptors are part of the real
compiler-emitted names.

| File | Names | SHA256 | Source |
|------|-------|--------|--------|
| `refs/objc/corpus/apple-clang-19.txt` | 88 | `97d6146c98494ebedb0b7ad233df58adb93d0ddaf808ab4aea4cfd4a9475fdcb` | `llvm-nm` on four generated Mach-O objects |
| `refs/objc/corpus/gnustep-clang-19.txt` | 21 | `26171c3c257c5a87737b0abef43112ea42b2d52089e51ae35a96ae28f4ff72b1` | `nm` on the generated GNUstep 2.0 and GNU-runtime ELF objects (GNU-runtime spellings) |
| `refs/objc/corpus/gnustep-abi2-clang-19.txt` | 14 | `b2b41a8fdebc703cb1f0f53e107e7186537a6dbd571802ec09caa5a4d5899236` | `nm` on the generated `-fobjc-runtime=gnustep-2.0` ELF object (`.`-decorated ABI 2 records) |
| `refs/objc/corpus/gnustep-abi2-coff-clang-19.txt` | 28 | `43b9d1b7f8b146389f11972bb66ecf6477ded957899a2633d648af20463c421b` | `nm` on the generated `-fobjc-runtime=gnustep-2.0` COFF objects for `x86_64-w64-mingw32` and `i686-w64-mingw32` |
| `refs/objc/corpus/gcc-libobjc-14.txt` | 51 | `8924df4d1d980f5126c8a489c724a6404f85372eb9b58e5e899439e0422995ed` | metadata subset of Debian GCC 14 `libobjc.a` |
| `refs/objc/corpus/methods-clang-19.txt` | 26 | `7754daa30dbae907b3de7835fbaf102bece42503521af1b7371c5dcf88772e43` | `llvm-nm` method display names for the same Mach-O objects |
| `refs/objc/corpus/mattermost-2.46.0-ios.txt` | 18,931 | `c2b15e6f1bf035f88b41571e256a8746bec450d1b695b0d83b056b21b747495d` | Mach-O executable in the unsigned Mattermost 2.46.0 IPA |
| `refs/objc/corpus/rejected.txt` | 18 | `ea560889799c3ea3cf1e718d339a6260094203a4b55967750101e7e4bab002c2` | runtime helpers, local labels, decorated unknown kinds and truncated prefixes that must not render as Objective-C |

The Apple corpus covers classes, metaclasses, ivars, categories, instance and
class method lists, property lists, instance variables, protocols, protocol
references, protocol labels, protocol refs/method types, class and category
protocol lists, read-only class/metaclass data, selector references and a
block descriptor.

The Mattermost corpus is a real shipped app: the unsigned IPA for
**Mattermost Mobile 2.46.0** (Apache-2.0). It contributes 18,931 defined
Objective-C metadata names from a stripped arm64 executable, including
generated Swift-ObjC bridge classes, category lists, class property lists and
exception types. The binary is an aggregate build: it also contains bundled
third-party frameworks and libraries under their own MIT/BSD/Apache licences
(for example SDWebImage, MSAL/MSID, React Native, Alamofire). The corpus stores
only symbol-name facts from that aggregate binary, not code from any of them.

| Field | Value |
|-------|-------|
| Release | <https://github.com/mattermost/mattermost-mobile/releases/tag/v2.46.0> |
| IPA | <https://github.com/mattermost/mattermost-mobile/releases/download/v2.46.0/Mattermost-unsigned.ipa> |
| IPA SHA256 | `d05c1f9433b98cc402a0143ba3e8414919e6eab3d2dbf202d2348a50b4d3b056` |
| Executable SHA256 | `222bca90d1eff85075886e6de19c4a277cf88a838e8391a9663c6825a64ffbd9` |
| License | App source Apache-2.0; bundled third-party frameworks retain their own MIT/BSD/Apache licences |
| Retrieved | 2026-11-22 |
| Extraction | `llvm-nm-19 Mattermost.app/Mattermost`, defined `_OBJC_*`/`__OBJC_*`/`__objc_*`/`___block_descriptor_*` names, `LC_ALL=C sort -u` |

Only symbol names are stored; the IPA and executable are not vendored.

The GNUstep corpus covers the GNU-runtime spellings: `_OBJC_CLASS_Foo`,
`_OBJC_METACLASS_Foo`, `__objc_class_name_Foo`, `__objc_class_ref_Foo`,
`__objc_ivar_offset_Foo._x`, `__objc_ivar_offset_value_Foo._x`, the GNUstep
2.0 `.i`-suffixed ivar-offset form and both ELF block-descriptor spellings.
The two `gnustep-abi2-*` corpora cover the GNUstep ABI 2 records themselves,
including the object-format decoration (see below).

The libobjc corpus covers the old GNU/NeXT metadata records (`_OBJC_Class_*`,
`_OBJC_ClassName_*`, `_OBJC_MetaClass_*`, `_OBJC_InstanceMethods_*`,
`_OBJC_InstanceIvars_*`, `_OBJC_ClassIvars_*`, `_OBJC_METH_VAR_*`,
`_OBJC_Module`, `_OBJC_SYMBOLS`, `_OBJC_SELECTOR_TABLE`) and the classic
GNUstep `__objc_class_name_*` / `__objc_class_ref_*` records.

## GNUstep ABI 2 decoration

GNUstep ABI 2 metadata is emitted with an object-format decoration in front of
the `_OBJC_` namespace. The decoration is part of the real symbol name, not a
tooling artifact: Clang emits it as a global in the assembly it writes.

| Target | Object format | Emitted name |
|--------|---------------|--------------|
| `x86_64-unknown-linux-gnu` | ELF | `._OBJC_REF_CLASS_Foo` |
| `x86_64-w64-mingw32`, `x86_64-pc-windows-msvc` | COFF | `$_OBJC_REF_CLASS_Foo` |
| `i686-w64-mingw32` | COFF | `_$_OBJC_REF_CLASS_Foo` |
| `x86_64-apple-macosx10.15`, `arm64-apple-ios14`, `wasm32-wasi` | Mach-O/wasm | rejected by Clang for ABI 2 (`error: GNUstep Objective-C runtime version 2 incompatible with target binary format`) |

Reproduction (Clang 19.1.7, GNUstep ABI 2.0/2.1/2.2 all agree):

```sh
GCC_INCLUDE=$(gcc -print-file-name=include)
clang-19 -c -fblocks -fobjc-runtime=gnustep-2.0 -I"$GCC_INCLUDE" foo.m -o foo.o
clang-19 -S -fblocks -fobjc-runtime=gnustep-2.0 -I"$GCC_INCLUDE" foo.m -o foo.s
nm foo.o | grep -E '^\._OBJC_'
grep -E '^\.globl[[:space:]]+\._OBJC_' foo.s   # the decoration is a global name
clang-19 -target x86_64-w64-mingw32 -c -fblocks -fobjc-runtime=gnustep-2.0 \
    -I"$GCC_INCLUDE" foo.m -o foo-coff.o
nm foo-coff.o | grep -E '^(\$|_\$)_OBJC_'
```

Evidence status:

- `._OBJC_*` (ELF) and `$_OBJC_*` / `_$_OBJC_*` (COFF) are **observed** and are
  claimed by the backend; `refs/objc/corpus/gnustep-abi2-clang-19.txt` and
  `refs/objc/corpus/gnustep-abi2-coff-clang-19.txt` are the pinned evidence.
  The records are `CLASS_`, `METACLASS_`, `REF_CLASS_`, `INIT_CLASS_` and
  `PROTOCOL_`; the decoration never reaches the printed output.
- The undecorated `_OBJC_REF_CLASS_*`, `_OBJC_INIT_CLASS_*` and
  `_OBJC_PROTOCOL_*` spellings and the bare `OBJC_*` namespace are
  **documented-layout aliases only**: no target in the pinned snapshot emits
  them (the undecorated `_OBJC_CLASS_*` / `_OBJC_METACLASS_*` records in
  `gnustep-clang-19.txt` come from the GNU-runtime path,
  `-fobjc-runtime=gcc` / `gnustep-1.7` / `gnustep-1.8`). They are accepted for
  compatibility with name-stripped symbol tables and are covered by the
  exact-output test rows in `tests/objc_corpus.rs`, not by corpus rows.
- The decoration is only stripped in front of `_OBJC_`. Local `.objc_*`
  records, `l_OBJC_*` labels, `.__OBJC_*` and `$_OBJC_NOPE_$_*` stay
  unclaimed and are pinned in `rejected.txt`.

## Rejected forms

The backend rejects rather than guesses:

- generic GNUstep runtime helpers such as `__objc_exec_class`, `__objc_load`
  and `__objc_accessors_init`;
- `__objc_category_name_*`, whose class/category split uses `_` and is
  ambiguous because both names may contain underscores;
- local `.objc_*` symbols, `l_OBJC_LABEL_*` local labels and block helper
  functions (`___copy_helper_block_*`, `___destroy_helper_block_*`);
- decorated names outside the `_OBJC_` namespace (`.__OBJC_$_*`) and decorated
  unknown kinds (`$_OBJC_NOPE_$_Foo`);
- truncated known prefixes (`_OBJC_`, `_OBJC_CLASS_$_`, `__objc_class_name_`, …)
  are claimed and reported as invalid Objective-C symbols; unknown Apple
  metadata kinds (`_OBJC_NOPE_$_Foo`) and unknown `__objc_*` helpers are not
  claimed as Objective-C at all.

## Extraction

```sh
refs/objc/extract-objc-symbols.sh
```

The script writes the hand-written Objective-C test programs to a temporary
directory, compiles them for the Apple Mach-O, GNUstep ABI 2 (ELF and COFF) and
GNU runtimes, extracts symbol names with `llvm-nm`/`nm`, filters the metadata
families and prints the count and SHA256 of each corpus file. It never writes
the generated objects to `refs/`.

The real Mattermost corpus is extracted separately from the pinned unsigned
IPA:

```sh
curl -L -o /tmp/Mattermost-unsigned.ipa \
  https://github.com/mattermost/mattermost-mobile/releases/download/v2.46.0/Mattermost-unsigned.ipa
unzip -q /tmp/Mattermost-unsigned.ipa -d /tmp/mattermost-ipa
llvm-nm-19 /tmp/mattermost-ipa/Mattermost.app/Mattermost \
  | grep -E '^[0-9a-f]+ [tTdDbBsS] ' \
  | awk '{print $NF}' \
  | grep -E '^_OBJC_|^__OBJC_|^__objc_|^___block_descriptor_' \
  | sort -u > refs/objc/corpus/mattermost-2.46.0-ios.txt
```

## Refresh

1. Install the new Clang/LLVM and GCC runtime, then update the snapshot table
   and the `LLVM_NM`/`CC` defaults in the script if needed.
2. Run `refs/objc/extract-objc-symbols.sh`. The COFF corpora need Clang
   targets for `x86_64-w64-mingw32` and `i686-w64-mingw32`; when a target is
   unavailable the script warns and leaves that file unchanged.
3. Update the counts and SHA256 values in this file and in the corpus test.
4. Run `cargo test --test objc_corpus`; raise coverage by adding supported
   forms, never by accepting an unknown prefix.
