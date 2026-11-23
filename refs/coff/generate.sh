#!/bin/sh
# Regenerate the PE/COFF symbol-table corpus in this directory.
#
# Requires clang-19 (Debian 1:19.1.7-3+b1) and GNU objdump 2.44. The objects
# are temporary: only the source and the normalized `objdump -t` text are
# vendored.
set -eu
export LC_ALL=C
cd "$(dirname "$0")"

clang-19 --target=i686-pc-windows-msvc -c probe.c -o probe-c-msvc.obj
clang-19 --target=i686-pc-windows-msvc -c probe.cpp -o probe-cpp-msvc.obj
clang-19 --target=i686-pc-windows-msvc -c probe-import.cpp -o probe-import-msvc.obj
clang-19 --target=i686-pc-windows-msvc -c probe-vectorcall.c -o probe-vectorcall-msvc.obj
clang-19 --target=i686-pc-windows-gnu -c probe.cpp -o probe-cpp-gnu.obj
clang-19 --target=i686-pc-windows-gnu -c probe-import.cpp -o probe-import-gnu.obj

for object in ./*.obj; do
    objdump -t "${object#./}" | sed -e 's/[[:space:]]*$//' -e '/^$/d' > "${object%.obj}.objdump.txt"
done

rm -f ./*.obj
