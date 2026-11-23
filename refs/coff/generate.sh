#!/bin/sh
# Regenerate the PE/COFF symbol-table corpus in this directory.
#
# Requires clang-19 (Debian 1:19.1.7-3+b1), GNU objdump 2.44, llvm-nm 19.1.7
# and the LLVM 19.1.7 `opt`/`llc` pair (the ARM64 object is not readable by GNU
# objdump). The objects are temporary: only the sources and the normalized
# symbol-table text are vendored.
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

# Hot/cold-split fragments. LLVM's HotColdSplitting pass is an IR pass, so the
# probes are IR: `opt` outlines the cold block (it needs the branch weights and
# the function entry count recorded in the probe) and `llc` emits the COFF
# object. No executable is linked and no C runtime is involved.
for triple in i686-pc-windows-msvc x86_64-pc-windows-msvc aarch64-pc-windows-msvc; do
    case "$triple" in
        i686*) stem=probe-cold-x86-msvc ;;
        x86_64*) stem=probe-cold-x64-msvc ;;
        *) stem=probe-cold-arm64-msvc ;;
    esac
    opt-19 -passes=hotcoldsplit -S probe-cold.ll -o - \
        | llc-19 -mtriple="$triple" -filetype=obj -o "$stem.obj"
done
opt-19 -passes=hotcoldsplit -S probe-cold-stdcall.ll -o - \
    | llc-19 -mtriple=i686-pc-windows-msvc -filetype=obj -o probe-cold-stdcall-x86-msvc.obj

for stem in probe-cold-x86-msvc probe-cold-x64-msvc probe-cold-stdcall-x86-msvc; do
    objdump -t "$stem.obj" | sed -e 's/[[:space:]]*$//' -e '/^$/d' > "$stem.objdump.txt"
done
llvm-nm-19 probe-cold-arm64-msvc.obj | sed -e 's/[[:space:]]*$//' -e '/^$/d' \
    > probe-cold-arm64-msvc.llvm-nm.txt

rm -f ./*.obj
