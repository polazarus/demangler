# Adversarial symbol corpora

These files are deterministic stress inputs, not expected-output pairs.
`generate.py` writes them; `tests/adversarial_corpus.rs` runs every line through
`Demangler::demangle_bytes` in a bounded worker and requires termination,
no panic and bounded output.

The corpora target:

- recursive type/encoding bombs (pointer, generic, tuple, optional, function
  and template nesting);
- back-reference/expansion bombs and deep path/name nesting;
- oversized length prefixes and over-budget identifiers;
- GNU v2 split-search blowups;
- deeply nested Go generics, ObjC brackets and Kotlin type arguments;
- D length/type/template/back-reference bombs;
- JNI escape and descriptor bombs;
- Ada segment and elaboration bombs;
- COFF `__imp_` nesting and stack-byte overflow;
- plain C/unknown pass-through names.

A recursive bomb may be rejected; rejection is success. The test fails if a
line overflows the stack, panics, runs past the timeout, or emits more than the
adversarial output ceiling.

Regenerate with:

```sh
python3 refs/adversarial/generate.py
```
