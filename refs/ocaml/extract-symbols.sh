#!/bin/sh
# Extract the OCaml linker symbols of the fixture in refs/ocaml/fixture.
#
# The fixture is compiled with the installed OCaml native compiler into a
# temporary directory; nothing is written into refs/. Output is one symbol per
# line, sorted with LC_ALL=C so the corpus stays stable, in five sections
# marked by a `#<name>` line:
#
#   #fixture   `caml` symbols of the fixture object files
#   #pack      `caml` symbols of the packed fixture object
#   #link      `caml` symbols of the linked program
#   #fragments `caml_` helper fragments of the linked program
#   #stdlib    `caml` symbols of the installed standard library archive
#
# Only defined symbols are listed: an undefined reference is not a name the
# compiler produced for this unit.
#
# Usage: refs/ocaml/extract-symbols.sh > /tmp/ocaml-symbols.txt
set -eu

here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

cp "$here"/fixture/*.ml "$tmp"/
cd "$tmp"

# One compilation unit per file. The fixture's `-for-pack` units are compiled
# and packed separately so the corpus covers a two-segment pack path.
ocamlopt -c ocaml_probe.ml ocaml_ops.ml ocaml_names.ml "ocaml_prime'.ml"
ocamlopt -for-pack Ocaml_pack.Ocaml_inner -c ocaml_pack_inner.ml
ocamlopt -for-pack Ocaml_pack -pack -o ocaml_inner.cmx ocaml_pack_inner.cmx
ocamlopt -for-pack Ocaml_pack -c ocaml_pack_base.ml
ocamlopt -pack -o ocaml_pack.cmx ocaml_pack_base.cmx ocaml_inner.cmx
ocamlopt -o ocaml_main ocaml_probe.cmx ocaml_ops.cmx ocaml_names.cmx \
    "ocaml_prime'.cmx" ocaml_pack.cmx ocaml_main.ml

# `caml` symbols of one object file, one per line.
symbols() {
    nm --defined-only "$@" |
        awk '$2 ~ /^[TDRC]$/ && $3 ~ /^caml/ { print $3 }' |
        LC_ALL=C sort -u
}

echo '#fixture'
symbols ocaml_probe.o ocaml_ops.o ocaml_names.o "ocaml_prime'.o"

echo '#pack'
symbols ocaml_pack.o

echo '#link'
symbols ocaml_main

# The host C compiler's cold-half (`.cold`) and partial-inlining (`.part.<N>`)
# fragments are local symbols of the linked program, so they are listed apart
# from the global `caml` names above.
echo '#fragments'
nm --defined-only ocaml_main |
    awk '$2 == "t" && $3 ~ /^caml/ && ($3 ~ /\.cold$/ || $3 ~ /\.part\.[0-9]+$/) { print $3 }' |
    LC_ALL=C sort -u

echo '#stdlib'
nm -g --defined-only "$(ocamlopt -where)/stdlib.a" |
    awk '$2 ~ /^[TDRC]$/ && $3 ~ /^caml/ { print $3 }' |
    LC_ALL=C sort -u
