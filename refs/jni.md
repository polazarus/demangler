# Java JNI references

A Java native method is bound to a C function whose name the JVM derives from
the method's declaring class, its name and — for an overloaded method — its
argument types. That name is what native libraries export, so it is the string
a demangler sees. There is no canonical JNI demangler: the escaping rules are
specified, the display form is not, so the in-tree backend implements the rules
below and prints the project-defined spelling documented here.

## Mangling rules

Oracle JNI specification, "Resolving Native Method Names" (JDK 21):

- <https://docs.oracle.com/en/java/javase/21/docs/specs/jni/design.html#resolving-native-method-names>

A native method name is

```text
Java_ <mangled class> _ <mangled method> [ "__" <mangled argument list> ]
```

| Escape | Denotes |
|--------|---------|
| `_1` | `_` |
| `_2` | `;` |
| `_3` | `[` |
| `_0xxxx` | the UTF-16 code unit `xxxx` (four hexadecimal digits) |
| `_` | `.` in a class path, `/` in an argument descriptor |

Every character that is not an ASCII alphanumeric is escaped, so a literal `_`
is always `_1` and a bare `_` is always a separator. `/` (the package/class
separator of a binary name) and `.` (the source-level separator) both become a
bare `_`.

Three consequences drive the implementation:

1. **The class path is recoverable.** Unescaped `_` marks a component boundary,
   and a literal `_` inside a component is `_1`, so no information is lost.
2. **The class/method boundary is the last bare `_` before the argument list.**
   The method is the final component; every earlier component belongs to the
   class path. This is a rule, not a guess: a method name cannot contain a bare
   `_`.
3. **`__` is the argument-list delimiter.** Two adjacent bare `_` can only come
   from the delimiter, because a component is never empty. Escapes are consumed
   as units, so a separator followed by an escape that starts with `_`
   (`p__0d835_0dd38`) is *not* a delimiter.

The mangled argument list is the descriptor's argument types with the
parentheses dropped: `([Ljava/lang/String;I)` mangles to
`_3Ljava_lang_String_2I`, and `()` mangles to the empty string, so the
no-argument overload of an overloaded method ends in `__`. The return type is
**not** part of the symbol: the list exists only to disambiguate overloads, and
Java forbids return-type-only overloading, so nothing is lost. Every generated
symbol below was checked against `javap` to confirm this.

A supplementary character is a UTF-16 surrogate pair of escapes
(`_0d835_0dd38` for U+1D538); a lone surrogate is malformed.

## Display contract

There is no canonical demangler, so the printed text is this crate's contract:

| Input | Output |
|---|---|
| `Java_com_example_Foo_plain` | `com.example.Foo.plain` |
| `Java_com_example_Foo_bar___3Ljava_lang_String_2I` | `com.example.Foo.bar(java.lang.String[], int)` |
| `Java_com_example_Foo_under_1score__` | `com.example.Foo.under_score()` |
| `Java_com_example_Weird_11_m__` | `com.example.Weird_1.m()` |

- a short name prints `class.method`, with no parentheses: the method is not
  overloaded, and the symbol carries no argument list;
- a long name prints `class.method(args)`; an empty list prints `()`;
- types are Java-like: `int`, `byte[]`, `java.lang.String`, and `$` is kept for
  a nested class (`com.example.Foo$Inner`), matching `Class.getName()`;
- the return type is not printed (it is absent from the symbol);
- the spelling is a debugger name, not a Java source declaration.

## JDK snapshot

| Field | Value |
|-------|-------|
| JDK | OpenJDK 21.0.13.1, release date `2026-11-17` |
| `javac -version` | `javac 21.0.13.1` |
| Runtime | `OpenJDK Runtime Environment (build 21.0.13.1+2-1-deb13u1-Debian)` |
| Distribution | Debian `openjdk-21-jdk-headless` `21.0.13.1+2-1~deb13u1` |
| Upstream | <https://github.com/openjdk/jdk21u> |
| Retrieved | 2026-11-23 |
| License | GPL-2.0 with Classpath exception (OpenJDK); see `/usr/share/doc/openjdk-21-jre-headless/copyright` |

**Commit SHA:** the Debian build carries no VCS stamp, so the upstream commit
SHA is not available offline. The JDK version and the Debian package version
pin the toolchain; the fixture sources and the generated rows are pinned by the
SHA256 values below.

The fixture sources under `refs/jni/fixture/` are original work for this crate
(MIT OR Apache-2.0, the crate's own license), not OpenJDK material. The corpus
files hold symbol names and display text produced by running the toolchain; no
JDK source, class file or header is vendored.

## Fixture

| File | SHA256 |
|------|--------|
| `fixture/Root.java` | `7a0df0868abc22ccd0901845486d7ae914bdeb0d1882746d0eb638c153499a48` |
| `fixture/com/example/Dollar$Cls.java` | `10eb5c70b8241dd6a717fb1beecedfb1ede034c03b5b8de1a40d94ef57651568` |
| `fixture/com/example/Foo.java` | `385bc66cb835d785528732ce3955c1c88300797e30280c40ac26710966c061f6` |
| `fixture/com/example/Names.java` | `ee8129e755704abdd5d1c63ff26156c095e008083badb4b09317ed7374b185a6` |
| `fixture/com/example/Types.java` | `c6047aaddc4cd73ee56e8a6c2db5a581166374b1f852522939d7e02351a09017` |
| `fixture/com/example/Weird_1.java` | `cff4c45ee22b7f715af55abdb40481b29b53d67347cacecda50487fe275ac113` |
| `fixture/com/example/deep/pkg/Deep.java` | `4203d1c98adaae445cafc15c94894477ce2103bb9c9b3035cd6f4601ba8c0d60` |
| `fixture/q_w/x.java` | `58b6f4347be14a5075590f163a0be1a92f0e1d348b506447cb6bdf6e50761e05` |
| `fixture/üni/codé/Ünïcode.java` | `a57ef1fb2eba4d91bf329594b997efa445867cf43dcd958fd14036c24c5bfe39` |

The fixture covers: a default-package class, multi-segment packages, a package
and class containing `_`, a class containing `$`, a nested static and a nested
inner class, a class name ending in `_`, every primitive parameter type, object
and array parameters, one-, two- and three-dimensional arrays, non-void
returns, short and long forms, the empty long-form argument list, leading,
trailing, doubled and literal `_1` method names, `$` in a method name, and
non-ASCII (BMP and supplementary) characters in class and method names.

Each class keeps its native method descriptors unique and its descriptor
multiset distinct from every other class, which is what makes the pairing below
unambiguous.

## Corpora

| File | Rows | SHA256 | Source |
|------|------|--------|--------|
| `corpus/jdk21-javac-h.txt` | 42 | `d413769f4cd603b00ddea919039b97edc9a16bde805ce8732709d7ee099884bc` | `javac -h` headers of the fixture, display text from `javap -p -s` |
| `corpus/hand.txt` | 16 | `923070f6fe6044632a5387776763b9e504b3ebea15a1861518c2d6762a01fe59` | hand-authored spec shapes and real JDK symbols |
| `corpus/rejected.txt` | 35 | `c4d15a8030135556024edc1ae6911e9b9310525d35a0bbb817886ee5d92a0b44` | hand-authored malformed names |
| `corpus/jdk21-libraries.txt` | 1,037 | `7079c197a664b442cfc1cbd3a6688ef8dc3fa42281ab30141f6fb57469f69f29` | every `Java_` symbol exported by the JDK's own native libraries (`nm -D`) |

`corpus/jdk21-javac-h.txt` is `symbol<TAB>display`, sorted with `LC_ALL=C sort
-u`. The symbol column is javac's own mangling, so it is authoritative. The
display column is built from `javap -p -s`, which names the class, the method
and the parameter types, so the expected text never comes from this crate's
rules.

`corpus/hand.txt` covers shapes the fixture cannot produce — a single-component
class path, an escaped `_` at a component boundary (`Java_a_1_m`), an escaped
`$` as the whole method name, a class path with more components than the class
really has (`Java_com_example_Foo_bar_baz`, documenting the boundary rule),
uppercase hexadecimal escape digits, a surrogate pair in the class part and a
long-form name with several array dimensions — plus four real JDK symbols whose
display text was verified by hand.

`corpus/jdk21-libraries.txt` is one name per line, extracted with

```sh
nm -D --defined-only "$JAVA_HOME"/lib/*.so | awk '{print $NF}' | grep '^Java_'
```

There is no demangler to ask for their display text, so the corpus is a coverage
gate instead: every name must parse, be claimed as JNI and **re-encode to
itself**. The test implements the mangling rules independently (the inverse of
the parser's decoding) and compares, which proves the decoding loses nothing.
Five of the 1,037 names are long-form (for example
`Java_sun_awt_DebugSettings_setCTracingOn__Z`, which prints
`sun.awt.DebugSettings.setCTracingOn(boolean)`), and they confirm on real
symbols that the argument list carries no return type.

Only symbol names are stored: no JDK source, class file, header or library is
vendored. The names are facts exported by the OpenJDK binaries (GPL-2.0 with
Classpath exception); they are reproduced here only as test data for a
demangler, the same way `refs/kotlin-native/` stores linker symbols.

`corpus/rejected.txt` covers, one class each:

- a missing or empty prefix (`Java_`), a name with no separator (`Java_foo`), an
  empty class part (`Java__bar`) and an empty method (`Java_foo_`);
- truncated (`_0`), non-hexadecimal (`_0zzzz`) and unpaired-surrogate escapes
  (`_0d835`, `_0dd38`, `_0d835_0000`, `_0d835_z`);
- `_2` and `_3` outside an argument list, and `_4`/`_9` (a separator is never
  followed by a digit, because no Java identifier starts with one);
- malformed argument lists: an unknown descriptor byte, `V` (a return marker,
  which the symbol never carries), a literal `(` or `[`, literal `/` and `;`,
  an unterminated object type, an empty object name, and trailing bytes;
- arrays nested deeper than the depth budget;
- a `(` in the class/method part (`Java_com_example_Foo_plain()`);
- `JNI_OnLoad` and `JNI_CreateJavaVM`, which are JNI helpers but plain C.

## Extraction

```sh
refs/jni/extract-jni-symbols.sh
```

The script compiles `refs/jni/fixture/` with `javac -d <tmp>/classes -h
<tmp>/headers`, runs `javap -p -s` on every compiled class, and pipes both
through `refs/jni/javac-header-corpus.awk`, which pairs a header block with its
`javap` row by (class, descriptor) and prints `symbol<TAB>display`. It then
extracts the JDK library names with `nm`. It warns when the installed `javac` is
not 21.x and aborts on an ambiguous or missing row.

### `javac -h` header comments are not the mangled name

The generated headers carry `Class:`, `Method:` and `Signature:` comments. Only
the `Signature:` comment is used, and only for pairing:

- `Class:` and `Method:` are written with a different, lossy mangling: a `$`
  becomes `__` (`com_example_Dollar__Cls`) while a literal `_` is left
  unescaped (`withUnderscore_1` for a method whose symbol part is
  `withUnderscore_11`). They cannot be decoded back to the Java name.
- `Signature:` is the plain JVM descriptor, except that a nested class is
  written with `/` instead of `$` (`(Lcom/example/Foo/Inner;)V` for
  `(Lcom/example/Foo$Inner;)V`). The pairing key therefore flattens `$` to `/`
  on the `javap` side as well; the display text keeps `javap`'s `$`.

Both facts were measured on JDK 21.0.13.1 and are the reason the corpus is
paired by descriptor rather than by comment.

## Refresh

1. Install the new JDK and update the version pin in
   `refs/jni/extract-jni-symbols.sh`.
2. Run `refs/jni/extract-jni-symbols.sh`.
3. Update the counts and SHA256 values in this file and the floors in
   `tests/jni_corpus.rs`; raise the floors when coverage improves, never lower
   them. The library corpus changes with the JDK's own native libraries, so its
   floor tracks the installed build.
4. Run `cargo test --no-default-features --features jni --test jni_corpus`.

## Gaps

- Registration names (`RegisterNatives`) and JNI helpers are not JNI symbols
  and are never claimed.
- The class/method boundary is the last bare `_`, so a class path with trailing
  components and a method name are formally indistinguishable. The documented
  rule resolves it; a symbol never carries enough information to do better.
- Decoded characters are not validated as Java identifier characters; the
  mangling is defined on escape sequences, not on identifier syntax. A `_0xxxx`
  escape that decodes to `;` or `[` is therefore accepted inside a class name.
- Only the argument types are printed. The return type is absent from the
  symbol, so nothing is dropped.
- Kotlin/JVM, Scala and other JVM languages use the same `Java_` scheme, so
  their native names parse as JNI; the display form is still the JVM binary
  name, not the source declaration.
