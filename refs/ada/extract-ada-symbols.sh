#!/bin/sh
# Rebuild the Ada/GNAT probe fixture and print its defined global symbols with
# the GNU `c++filt -s gnat` oracle output.
#
# Usage:
#   refs/ada/extract-ada-symbols.sh > /tmp/ada-oracle.tsv
#
# The curated corpus (`refs/ada/corpus/gnat-14.2-linux-x86_64.tsv`) is derived
# from this output and reviewed by hand; `c++filt` is only a generation-time
# oracle. See `refs/ada.md` for the pinned toolchain and classification rules.
set -eu

root=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
fixture="$root/refs/ada/fixture"
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

cp "$fixture"/*.ads "$fixture"/*.adb "$work"/
(
    cd "$work"
    gnatmake -c -q main.adb
)

nm -g --defined-only "$work"/*.o \
    | awk 'NF >= 3 { print $3 }' \
    | LC_ALL=C sort -u \
    | while IFS= read -r symbol; do
        printf '%s\t%s\n' "$symbol" "$(c++filt -s gnat "$symbol")"
    done
