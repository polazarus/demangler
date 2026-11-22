#!/bin/sh
# Extract the Go symbol-name corpus in refs/go/corpus/.
#
# The corpus is a list of real symbol names produced by the installed Go
# toolchain:
#
# - the ELF symbol table of a generated test binary (stdlib, methods, closures,
#   generics, shapes, type descriptors, linker specials, ABI markers);
# - the symbol table of a generated compiler object file (which also holds the
#   `go:string.`, `type:.namedata.`, `·f` and `gclocals·` names that the linker
#   hides from the final symbol table);
# - the symbol tables of the exported package archives of several standard
#   library packages plus the generated packages, which additionally expose
#   static compiler symbols (`..stmp_N`, `.jumpN`), equality/hash closures
#   (`type:.eqfunc*`, `type:.hashfunc*`), package-qualified method names and
#   the `<N>` object-file symbol version marker.
#
# Usage: refs/go/extract-go-symbols.sh [output-dir]
#
# Only symbol names are stored; no binaries or object files are vendored.

set -eu

GO_VERSION_EXPECTED="go1.24.4"
OUT_DIR="${1:-refs/go/corpus}"
ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
OUT_DIR="$ROOT/$OUT_DIR"

GO_VERSION=$(go version | awk '{print $3}')
if [ "$GO_VERSION" != "$GO_VERSION_EXPECTED" ]; then
    echo "warning: corpus pinned to $GO_VERSION_EXPECTED, found $GO_VERSION" >&2
fi

WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT

mkdir -p "$WORK/lib" "$WORK/pkg.v1" "$WORK/shape" "$WORK/tables" \
    "$WORK/emb" "$WORK/use" "$OUT_DIR"

cat >"$WORK/go.mod" <<'EOF'
module example.com/demangle/corpus

go 1.24
EOF

cat >"$WORK/lib/lib.go" <<'EOF'
package lib

type Number interface{ ~int | ~int64 | ~float64 }

type Box[T any] struct{ v T }

func New[T any](v T) *Box[T] { return &Box[T]{v: v} }

func (b *Box[T]) Get() T { return b.v }

func (b *Box[T]) Map(f func(T) T) *Box[T] { return &Box[T]{v: f(b.v)} }

func Sum[T Number](xs []T) T {
	var total T
	for _, x := range xs {
		total += x
	}
	return total
}

func Each[T any](xs []T, f func(int, T)) {
	for i, x := range xs {
		f(i, x)
	}
}

func Counter() func() int {
	n := 0
	return func() int { n++; return n }
}

type Err struct{ Code int }

func (e *Err) Error() string { return "err" }

var Registry = map[string]func() int{}

func init() {
	Registry["counter"] = Counter()
}
EOF

cat >"$WORK/pkg.v1/pkg.go" <<'EOF'
package pkgv1

type Thing struct{ N int }

func (t Thing) Value() int { return t.N }

func Make(n int) Thing { return Thing{N: n} }

func Apply(f func(int) int, v int) int { return f(v) }
EOF

cat >"$WORK/main.go" <<'EOF'
package main

import (
	"fmt"

	"example.com/demangle/corpus/lib"
	pkgv1 "example.com/demangle/corpus/pkg.v1"
)

type Shape interface{ Area() float64 }

type Circle struct{ R float64 }

func (c Circle) Area() float64   { return 3.14159 * c.R * c.R }
func (c *Circle) Scale(k float64) { c.R *= k }

type MyInt int

func (m MyInt) String() string { return fmt.Sprintf("MyInt(%d)", int(m)) }

func main() {
	c := &Circle{R: 1}
	c.Scale(2)
	var s Shape = c
	fmt.Println(s.Area())
	b := lib.New(3)
	fmt.Println(b.Get(), b.Map(func(x int) int { return x * 2 }).Get())
	fmt.Println(lib.Sum([]int{1, 2}), lib.Sum([]float64{1.5}))
	lib.Each([]string{"a"}, func(i int, s string) { fmt.Println(i, s) })
	fmt.Println(lib.Counter()())
	fmt.Println(MyInt(3), lib.Registry["counter"]())
	fmt.Println(pkgv1.Make(7).Value(), pkgv1.Apply(func(x int) int { return x + 1 }, 1))
	var e error = &lib.Err{Code: 1}
	fmt.Println(e)
}
EOF

cat >"$WORK/shape/shape.go" <<'EOF'
package shape

type Number interface{ ~int | ~int64 | ~float64 }

type Pair[A, B any] struct {
	First  A
	Second B
}

func NewPair[A, B any](a A, b B) Pair[A, B] { return Pair[A, B]{a, b} }

func (p Pair[A, B]) Swap() Pair[B, A] { return Pair[B, A]{p.Second, p.First} }

func (p Pair[A, B]) Values() (A, B) { return p.First, p.Second }

type Set[T comparable] map[T]struct{}

func (s Set[T]) Add(v T) { s[v] = struct{}{} }

func (s Set[T]) Has(v T) bool {
	_, ok := s[v]
	return ok
}

func Map[T, U any](xs []T, f func(T) U) []U {
	out := make([]U, 0, len(xs))
	for _, x := range xs {
		out = append(out, f(x))
	}
	return out
}

func Filter[T any](xs []T, keep func(T) bool) []T {
	var out []T
	for _, x := range xs {
		if keep(x) {
			out = append(out, x)
		}
	}
	return out
}

func Reduce[T, A any](xs []T, init A, f func(A, T) A) A {
	acc := init
	for _, x := range xs {
		acc = f(acc, x)
	}
	return acc
}

func Sum[T Number](xs []T) T {
	var total T
	for _, x := range xs {
		total += x
	}
	return total
}

func Max[T Number](a, b T) T {
	if a > b {
		return a
	}
	return b
}

func Instances() (Pair[int, string], Set[string], int, int64, float64) {
	p := NewPair(1, "one")
	swapped := p.Swap()
	s := Set[string]{}
	s.Add(swapped.First)
	_ = s.Has("one")
	total := Sum([]int{1, 2, 3}) + int(Sum([]float64{1.5}))
	wide := Max(int64(1), int64(2))
	return p, s, total, wide, Sum([]float64{2.5})
}
EOF

cat >"$WORK/tables/tables.go" <<'EOF'
package tables

type key struct {
	A int
	B string
	C [2]int64
}

var lookup = map[key]int{}

var (
	Alpha = "alpha"
	Beta  = "beta"
	Words = []string{"gamma", "delta", "epsilon"}
)

var Handler = func(n int) int { return n + 1 }

type Shape interface{ Area() float64 }

type Square struct{ Side float64 }

func (s Square) Area() float64 { return s.Side * s.Side }

type Circle struct{ R float64 }

func (c Circle) Area() float64 { return 3 * c.R * c.R }

func Lookup(k key) int { return lookup[k] }

func Store(k key, v int) { lookup[k] = v }

func Kind(n int) string {
	switch n {
	case 0:
		return "zero"
	case 3:
		return "three"
	case 7:
		return "seven"
	case 11:
		return "eleven"
	case 19:
		return "nineteen"
	case 23:
		return "twenty-three"
	case 42:
		return "answer"
	case 97:
		return "ninety-seven"
	case 101:
		return "hundred and one"
	case 255:
		return "byte"
	case 1024:
		return "kilo"
	case 4096:
		return "page"
	default:
		return "other"
	}
}

func Describe(v any) string {
	switch v.(type) {
	case int:
		return "int"
	case string:
		return "string"
	case []byte:
		return "bytes"
	case Shape:
		return "shape"
	default:
		return "unknown"
	}
}

func AsInt(v any) (int, bool) {
	n, ok := v.(int)
	return n, ok
}

func Defer(f func()) { defer f() }

func Go(f func()) { go f() }

func Range(seq func(func(int) bool)) int {
	total := 0
	for i := range seq {
		total += i
	}
	return total
}

func Build() []int { return []int{1, 2, 3, 5, 8, 13, 21, 34} }

func Areas() []float64 {
	shapes := []Shape{Square{Side: 2}, &Circle{R: 1}}
	out := make([]float64, 0, len(shapes))
	for _, s := range shapes {
		out = append(out, s.Area())
	}
	return out
}

func Values() (int, string, []string, func(int) int, func()) {
	s := Square{Side: 3}
	area := s.Area
	counter := 0
	inc := func() { counter++ }
	return int(area()), Alpha, Words, Handler, inc
}
EOF

cat >"$WORK/emb/emb.go" <<'EOF'
package emb

type Inner struct{ N int }

func (i *Inner) bump() { i.N++ }

func (i *Inner) Value() int { return i.N }

type Embedded interface {
	Value() int
}
EOF

cat >"$WORK/use/use.go" <<'EOF'
package use

import "example.com/demangle/corpus/emb"

// Outer promotes emb.Inner's methods, including the unexported bump, so the
// compiler emits package-qualified method wrappers
// (`(*Outer).example.com/demangle/corpus/emb.bump`).
type Outer struct{ *emb.Inner }

type Alias = Outer

func Make() *Outer { return &Outer{Inner: &emb.Inner{}} }

func Value(o *Outer) int { return o.Value() }
EOF

cd "$WORK"

# `go tool nm` prints `<addr> <type> <name>`; names may contain spaces, and
# undefined entries have no address. Archives with more than one object file
# prefix every line with `<archive>(<member>):`. Keep the name column only.
normalize() {
    awk '
        { sub(/^.*\([^)]*\):\t/, "") }
        /^[[:space:]]*$/ { next }
        $1 ~ /^[0-9a-f]+$/ && NF >= 3 { $1 = ""; $2 = ""; sub(/^[[:space:]]+/, ""); print; next }
        ($1 == "U" || $1 == "u") && NF >= 2 { $1 = ""; sub(/^[[:space:]]+/, ""); print; next }
    ' \
        | grep -v '^$' \
        | grep -v '^<autogenerated>$' \
        | grep -v '^/' \
        | grep -v '^\$GOROOT/' \
        | LC_ALL=C sort -u
}

# Inlined and non-inlined builds together cover inlined closure names
# (`init.0.Counter.func1`) and the full set of outlined functions.
go build -o corpus-inline .
go build -gcflags=all=-l -o corpus-noinline .

{
    go tool nm corpus-inline
    go tool nm corpus-noinline
} | normalize >"$OUT_DIR/go1.24.4-binary.txt"

# The compiler object file keeps the names the linker hides.
go tool compile -p example.com/demangle/corpus/lib -o lib.o lib/lib.go
go tool nm lib.o | normalize >"$OUT_DIR/go1.24.4-object.txt"

# Exported package archives. The standard library packages add real-world
# names (methods, closures, generics, shapes, type descriptors, jump tables,
# package-qualified methods); the generated packages add the compiler symbols
# this module's code shapes produce.
STDLIB_PACKAGES="fmt errors sort slices sync internal/sync io bufio regexp
    text/template encoding/binary container/list crypto/sha256 path/filepath"
GENERATED_PACKAGES="./lib ./pkg.v1 ./shape ./tables ./emb ./use ."

{
    for pkg in $STDLIB_PACKAGES $GENERATED_PACKAGES; do
        archive=$(go list -export -f '{{.Export}}' "$pkg")
        go tool nm "$archive"
    done
} | normalize >"$OUT_DIR/go1.24.4-archive.txt"

echo "wrote $OUT_DIR/go1.24.4-binary.txt ($(wc -l <"$OUT_DIR/go1.24.4-binary.txt") names)"
echo "wrote $OUT_DIR/go1.24.4-object.txt ($(wc -l <"$OUT_DIR/go1.24.4-object.txt") names)"
echo "wrote $OUT_DIR/go1.24.4-archive.txt ($(wc -l <"$OUT_DIR/go1.24.4-archive.txt") names)"
