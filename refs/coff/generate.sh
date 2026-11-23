#!/bin/sh
# Regenerate the PE/COFF symbol-table corpus in this directory.
#
# Requires clang-19 (Debian 1:19.1.7-3+b1), GNU objdump 2.44 and llvm-nm 19.1.7
# (the ARM64 object is not readable by GNU objdump). The objects are temporary:
# only the sources and the normalized symbol-table text are vendored.
set -eu
export LC_ALL=C
cd "$(dirname "$0")"

clang-19 --target=i686-pc-windows-msvc -c probe.c -o probe-c-msvc.obj
clang-19 --target=i686-pc-windows-msvc -c probe.cpp -o probe-cpp-msvc.obj
clang-19 --target=i686-pc-windows-msvc -c probe-import.cpp -o probe-import-msvc.obj
clang-19 --target=i686-pc-windows-msvc -c probe-vectorcall.c -o probe-vectorcall-msvc.obj
clang-19 --target=i686-pc-windows-msvc -c probe-weak.c -o probe-weak-msvc.obj
clang-19 --target=x86_64-pc-windows-msvc -c probe-vectorcall.c -o probe-vectorcall-x64-msvc.obj
clang-19 --target=x86_64-pc-windows-msvc -c probe-weak.c -o probe-weak-x64-msvc.obj
clang-19 --target=i686-pc-windows-gnu -c probe.cpp -o probe-cpp-gnu.obj
clang-19 --target=i686-pc-windows-gnu -c probe-import.cpp -o probe-import-gnu.obj
clang-19 --target=aarch64-pc-windows-msvc -c probe-arm64.c -o probe-arm64-msvc.obj

for object in ./*.obj; do
    case "$object" in
        *arm64*) continue ;;
    esac
    objdump -t "${object#./}" | sed -e 's/[[:space:]]*$//' -e '/^$/d' > "${object%.obj}.objdump.txt"
done

# GNU objdump 2.44 does not recognise ARM64 COFF, so the ARM64 object is
# recorded with llvm-nm from the same LLVM release.
llvm-nm-19 probe-arm64-msvc.obj | sed -e 's/[[:space:]]*$//' -e '/^$/d' \
    > probe-arm64-msvc.llvm-nm.txt

rm -f ./*.obj
