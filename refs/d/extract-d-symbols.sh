#!/bin/sh
# Regenerate the raw D symbol/expected pairs for refs/d/.
#
# This is a provenance script, not a build or test dependency. It needs the
# pinned LDC 1.40, libphobos 2.110.0 and binutils 2.44 installations described
# in refs/d.md. It writes only symbol names and generated demangled text.
set -eu

out=${1:-refs-d-raw}
mkdir -p "$out"

libphobos=/usr/lib/x86_64-linux-gnu/libphobos2-ldc-shared.so.110.0

nm -D --defined-only "$libphobos" \
  | awk '{print $3}' | grep '^_D' | sort -u > "$out/libphobos.syms"
c++filt -s dlang < "$out/libphobos.syms" > "$out/libphobos.expected"

cat > "$out/app.d" <<'D'
module app;

int globalInt = 42;
const double globalConst = 3.5;
immutable string globalStr = "hi";
int[5] staticArr;
int[] dynArr;
int[string] assocArr;

struct Point { int x, y; }
class Widget {
    int v;
    this(int v) { this.v = v; }
    ~this() {}
    int get() { return v; }
    static int make() { return 1; }
}

enum Color { red, green, blue }

alias IntAlias = int;

int add(int a, int b) { return a + b; }
T identity(T)(T x) { return x; }
int useIdentity() { return identity!int(3); }
T[] mapArray(T)(T[] xs) { return xs; }

void takesDelegate(int delegate(int) dg) {}
void takesFunction(int function(int) fn) {}
void takesArrays(int[] a, int[5] b, int[string] c) {}

int outer(int x) {
    int inner(int y) { return y + x; }
    return inner(2);
}

class Box(T) { T value; this(T v) { value = v; } T get() { return value; } }

void main() {
    auto w = new Widget(1);
    auto b = new Box!int(2);
    int local = add(1, 2);
    int[] arr = [1,2,3];
    foreach (i; arr) { local += i; }
}
D

ldc2 -c -g -O0 "$out/app.d" -of="$out/app.o"
nm "$out/app.o" | awk '{print $NF}' | grep '^_D' | sort -u > "$out/app.syms"
c++filt -s dlang < "$out/app.syms" > "$out/app.expected"

echo "wrote $out/{libphobos,app}.{syms,expected}"
