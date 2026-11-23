#!/bin/sh
# Extract the linker symbols of the Free Pascal fixture in refs/delphi/fixture.
#
# The fixture is compiled with the installed Free Pascal compiler into a
# temporary directory; nothing is written into refs/. Output is one defined
# global symbol per line, sorted with LC_ALL=C so the corpus stays stable.
#
# Usage: refs/delphi/extract-delphi-symbols.sh > /tmp/fpc-symbols.txt
set -eu

here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

cp "$here"/fixture/*.pas "$tmp"/
cd "$tmp"

for source in delphi_probe.pas delphi_probe.ns.pas delphi_probe_delphi.pas delphi_probe_prog.pas \
    delphi_probe_data.pas; do
    fpc -O- -gw "$source" >/dev/null
done

for object in ./*.o; do
    nm --defined-only "$object" | awk '$2 !~ /^[Nn]$/ && $3 !~ /^\.L/ { print $3 }'
done | grep -v -E '^(main|PASCALMAIN)$' | LC_ALL=C sort -u
