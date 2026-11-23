#!/bin/sh
# Dump the symbol names gfortran emits for the Fortran probe fixture.
#
# Usage: refs/fortran/extract-fortran-symbols.sh > /tmp/fortran-symbols.tsv
#
# The fixture is compiled into a temporary directory with three underscoring
# modes, because the external-name form depends on the mode:
#
#   default               `ext_sub`     -> `ext_sub_`
#   -fsecond-underscore   `my_ext_sub`  -> `my_ext_sub__`
#   -fno-underscoring     `ext_sub`     -> `ext_sub`
#
# Module symbols (`__<module>_MOD_<member>`) are independent of the mode.
# Output is `mode<TAB>object<TAB>type<TAB>symbol`, sorted, and never writes
# into `refs/`. The curated corpus is reviewed from this output; it is not
# generated automatically, because the display contract is project-defined.
set -eu

here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT INT TERM

for mode in default second-underscore no-underscoring; do
    case $mode in
    default) flags= ;;
    second-underscore) flags=-fsecond-underscore ;;
    no-underscoring) flags=-fno-underscoring ;;
    esac
    mkdir -p "$tmp/$mode"
    cp "$here"/fixture/*.f90 "$tmp/$mode"/
    # Compile in dependency order: a `.mod` file must exist before the file
    # that uses it is compiled.
    (cd "$tmp/$mode" && gfortran -c -O0 $flags \
        fprobe.f90 fprobe_child.f90 fprobe_generic.f90 \
        probe_ext.f90 probe_main.f90)
    for object in "$tmp/$mode"/*.o; do
        name=$(basename "$object" .o)
        nm --defined-only "$object" |
            awk -v mode="$mode" -v object="$name" 'NF == 3 { print mode "\t" object "\t" $2 "\t" $3 }'
    done
done | LC_ALL=C sort -u
