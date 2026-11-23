#!/bin/sh
# Extract the JNI corpus in refs/jni/corpus/.
#
# The corpus is a list of `symbol<TAB>display` rows for real native-method
# symbols produced by `javac -h` on the fixture under refs/jni/fixture/:
#
# - the symbol column is javac's own mangling (authoritative);
# - the display column is built from `javap -p -s` output, which names the
#   class, the method and the parameter types, so the expected text comes from
#   the JVM toolchain rather than from this crate's rules.
#
# refs/jni/javac-header-corpus.awk pairs a header symbol with its javap row by
# (class, descriptor) and renders the documented display form. It aborts when a
# row is missing, a descriptor is ambiguous or an escape is malformed.
#
# Usage: refs/jni/extract-jni-symbols.sh
#
# Only symbol names and display text are stored; no class files or headers are
# vendored.

set -eu

JDK_VERSION_EXPECTED="21.0.13.1"
ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
FIXTURE="$ROOT/refs/jni/fixture"

JAVAC_VERSION=$(javac -version 2>&1 | awk '{print $2}')
case "$JAVAC_VERSION" in
    21.*) ;;
    *) echo "warning: corpus pinned to JDK $JDK_VERSION_EXPECTED, found $JAVAC_VERSION" >&2 ;;
esac

WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT
mkdir -p "$WORK/classes" "$WORK/headers"

(
    cd "$FIXTURE"
    find . -name '*.java' -print | LC_ALL=C sort >"$WORK/sources.txt"
    javac -d "$WORK/classes" -h "$WORK/headers" @"$WORK/sources.txt"
)

(
    cd "$WORK/classes"
    find . -name '*.class' -print | LC_ALL=C sed -e 's|^\./||' -e 's|\.class$||' -e 's|/|.|g' |
        LC_ALL=C sort >"$WORK/classes.txt"
    javap -p -s $(cat "$WORK/classes.txt") >"$WORK/javap.txt"
)

# shellcheck disable=SC2046
LC_ALL=C awk -f "$ROOT/refs/jni/javac-header-corpus.awk" \
    "$WORK/javap.txt" $(find "$WORK/headers" -name '*.h' | LC_ALL=C sort) |
    LC_ALL=C sort -u >"$ROOT/refs/jni/corpus/jdk21-javac-h.txt"

# The JDK's own native libraries export real JNI names. They carry no display
# text (there is no demangler to ask), so they are a coverage corpus: every name
# must parse and re-encode to itself. Only the names are stored.
JAVA_HOME=$(dirname "$(dirname "$(readlink -f "$(command -v javac)")")")
nm -D --defined-only "$JAVA_HOME"/lib/*.so 2>/dev/null | awk '{print $NF}' |
    grep '^Java_' | LC_ALL=C sort -u >"$ROOT/refs/jni/corpus/jdk21-libraries.txt"

printf 'jni corpus: %s rows from %s headers, %s library names (javac %s)\n' \
    "$(wc -l <"$ROOT/refs/jni/corpus/jdk21-javac-h.txt" | tr -d ' ')" \
    "$(find "$WORK/headers" -name '*.h' | wc -l | tr -d ' ')" \
    "$(wc -l <"$ROOT/refs/jni/corpus/jdk21-libraries.txt" | tr -d ' ')" \
    "$JAVAC_VERSION" >&2
