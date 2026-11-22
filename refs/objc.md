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
| GNUstep 2.0 | `_OBJC_CLASS_Class`, `_OBJC_METACLASS_Class`, `_OBJC_REF_CLASS_Class`, `_OBJC_INIT_CLASS_Class`, `_OBJC_PROTOCOL_Protocol` |
| GNU/GNUstep classic | `__objc_class_name_Class`, `__objc_class_ref_Class`, `__objc_ivar_offset_Class.ivar`, `__objc_ivar_offset_value_Class.ivar` |
| Old GNU/NeXT metadata | `_OBJC_Class_Class`, `_OBJC_ClassName_Class`, `_OBJC_MetaClass_Class`, `_OBJC_InstanceMethods_Class`, `_OBJC_ClassMethods_Class`, `_OBJC_InstanceIvars_Class`, `_OBJC_ClassIvars_Class`, `_OBJC_Module`, `_OBJC_SYMBOLS`, `_OBJC_SELECTOR_TABLE`, `_OBJC_METH_VAR_NAME_N`, `_OBJC_METH_VAR_TYPE_N` |
| Blocks | `__block_descriptor_…` (ELF), `___block_descriptor_…` (Mach-O) |

Apple's runtime separator is `_$_`; it is the only structural separator the
parser relies on. Class, category and protocol names are kept verbatim and are
never split on `_` or `$`. Metadata prefixes are case-sensitive.

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
| GNU runtime | Debian GCC 14.2.0-19, `libobjc.a` at `/usr/lib/gcc/x86_64-linux-gnu/14/libobjc.a` |
| GNUstep runtime | Clang `-fobjc-runtime=gnustep-2.0` and `-fobjc-runtime=gcc` |
| Symbol reader | `llvm-nm-19` for Mach-O, GNU `nm` for ELF |
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
| `refs/objc/corpus/apple-clang-19.txt` | 32 | `a110414e790959d2d6486940a17039defdbee78c2607da7ad6fa08d3dce7cf61` | `llvm-nm` on three generated Mach-O objects |
| `refs/objc/corpus/gnustep-clang-19.txt` | 8 | `d3f17af1ab0ebd94bd0f11cdb10b1addc1e3c9e3fe53923782bc4339465ffbc7` | `nm` on generated GNUstep 2.0 and GNU-runtime ELF objects |
| `refs/objc/corpus/gcc-libobjc-14.txt` | 51 | `8924df4d1d980f5126c8a489c724a6404f85372eb9b58e5e899439e0422995ed` | metadata subset of Debian GCC 14 `libobjc.a` |
| `refs/objc/corpus/methods-clang-19.txt` | 6 | `61969ebf35e46eda74826b25e33cbeb78b2a27904422c737a930a0745eb028c5` | `llvm-nm` method display names for the same Mach-O objects |
| `refs/objc/corpus/rejected.txt` | 17 | `2b2de21f1961268476a9dfdc7491b71fbd1539e4fe36dacc48ff98d965878154` | runtime helpers, local labels and truncated prefixes that must not render as Objective-C |

The Apple corpus covers classes, metaclasses, ivars, categories, instance and
class method lists, property lists, instance variables, protocols, protocol
references, protocol labels, protocol refs/method types, class and category
protocol lists, read-only class/metaclass data, selector references and a
block descriptor.

The GNUstep corpus covers `_OBJC_CLASS_Foo`, `_OBJC_METACLASS_Foo`,
`__objc_class_name_Foo`, `__objc_ivar_offset_Foo._x`,
`__objc_ivar_offset_value_Foo._x`, the GNUstep 2.0 `.i`-suffixed ivar-offset
form and both ELF block-descriptor spellings.

The libobjc corpus covers the old GNU/NeXT metadata records (`_OBJC_Class_*`,
`_OBJC_ClassName_*`, `_OBJC_MetaClass_*`, `_OBJC_InstanceMethods_*`,
`_OBJC_InstanceIvars_*`, `_OBJC_ClassIvars_*`, `_OBJC_METH_VAR_*`,
`_OBJC_Module`, `_OBJC_SYMBOLS`, `_OBJC_SELECTOR_TABLE`) and the classic
GNUstep `__objc_class_name_*` / `__objc_class_ref_*` records.

## Rejected forms

The backend rejects rather than guesses:

- generic GNUstep runtime helpers such as `__objc_exec_class`, `__objc_load`
  and `__objc_accessors_init`;
- `__objc_category_name_*`, whose class/category split uses `_` and is
  ambiguous because both names may contain underscores;
- local `.objc_*` symbols, `._OBJC_*` GNUstep 2.0 section-prefixed spellings,
  `l_OBJC_LABEL_*` local labels and block helper functions
  (`___copy_helper_block_*`, `___destroy_helper_block_*`);
- truncated known prefixes (`_OBJC_`, `_OBJC_CLASS_$_`, `__objc_class_name_`, …)
  are claimed and reported as invalid Objective-C symbols; unknown Apple
  metadata kinds (`_OBJC_NOPE_$_Foo`) and unknown `__objc_*` helpers are not
  claimed as Objective-C at all.

## Extraction

```sh
refs/objc/extract-objc-symbols.sh
```

The script writes the four hand-written Objective-C test programs to a
temporary directory, compiles them for the Apple Mach-O and GNUstep/GNU
runtimes, extracts symbol names with `llvm-nm`/`nm`, filters the metadata
families and prints the count and SHA256 of each corpus file. It never writes
the generated objects to `refs/`.

## Refresh

1. Install the new Clang/LLVM and GCC runtime, then update the snapshot table
   and the `LLVM_NM`/`CC` defaults in the script if needed.
2. Run `refs/objc/extract-objc-symbols.sh`.
3. Update the counts and SHA256 values in this file and in the corpus test.
4. Run `cargo test --test objc_corpus`; raise coverage by adding supported
   forms, never by accepting an unknown prefix.
