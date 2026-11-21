#!/usr/bin/env python3
"""Extract mangled Rust symbols from a toolchain library.

This is local tooling, not upstream material. It deliberately keeps only
defined, non-local symbol names that start with a Rust mangling prefix. For
legacy `_ZN` names it also requires the trailing 16-hex-digit Rust crate hash,
which excludes Itanium C++ symbols that share the prefix. The corpus test
compares each symbol with rustc-demangle at test time, so this script does not
need a demangler of its own.

Usage:
    python3 refs/rust/extract-toolchain-symbols.py \
        --toolchain ~/.rustup/toolchains/1.75.0-x86_64-unknown-linux-gnu \
        --output refs/rust/corpus/toolchain-1.75.0-legacy.txt \
        --prefixes _ZN,ZN,__ZN
    python3 refs/rust/extract-toolchain-symbols.py \
        --toolchain ~/.rustup/toolchains/stable-x86_64-unknown-linux-gnu \
        --source rustc-driver --limit 2000 \
        --output refs/rust/corpus/toolchain-1.100.0-rustc-v0.txt \
        --prefixes _R,__R
"""

from __future__ import annotations

import argparse
import pathlib
import re
import subprocess
import sys

# Rust legacy symbols always end in the 16-hex-digit crate hash. This rejects
# Itanium C++ names (for example libstdc++'s `__cxxabiv1::__terminate`) that
# share the `_ZN` prefix.
LEGACY_HASH = re.compile(r"17h[0-9a-f]{16}E(?:\.llvm\.[0-9A-Fa-f]+)?$")


def find_library(toolchain: pathlib.Path, source: str) -> pathlib.Path:
    if source == "libstd":
        matches = sorted((toolchain / "lib" / "rustlib").glob("*/lib/libstd-*.rlib"))
        pattern = "libstd rlib"
    else:
        matches = sorted((toolchain / "lib").glob("librustc_driver*.so"))
        pattern = "rustc driver library"
    if len(matches) != 1:
        raise SystemExit(f"expected one {pattern} under {toolchain}, found {len(matches)}")
    return matches[0]


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--toolchain", required=True, type=pathlib.Path)
    parser.add_argument("--output", required=True, type=pathlib.Path)
    parser.add_argument(
        "--source",
        choices=("libstd", "rustc-driver"),
        default="libstd",
        help="library to scan (default: libstd)",
    )
    parser.add_argument(
        "--limit",
        type=int,
        default=0,
        help="keep an evenly spaced sample of at most N symbols (0: all)",
    )
    parser.add_argument(
        "--prefixes",
        default="_ZN,ZN,__ZN,_R,R,__R",
        help="comma-separated mangling prefixes to keep",
    )
    args = parser.parse_args()

    library = find_library(args.toolchain, args.source)
    prefixes = tuple(part for part in args.prefixes.split(",") if part)
    nm = subprocess.run(
        ["nm", "--defined-only", str(library)],
        check=True,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.DEVNULL,
    )
    symbols = set()
    for line in nm.stdout.splitlines():
        fields = line.split()
        if len(fields) < 2:
            continue
        name = fields[-1]
        if not name.startswith(prefixes):
            continue
        if name.startswith(("_ZN", "ZN", "__ZN")) and not LEGACY_HASH.search(name):
            continue
        symbols.add(name)

    symbols = sorted(symbols)
    if args.limit and len(symbols) > args.limit:
        # Evenly spaced sample; deterministic for a pinned library.
        symbols = [symbols[index * len(symbols) // args.limit] for index in range(args.limit)]

    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text("".join(f"{symbol}\n" for symbol in symbols))
    print(f"{args.output}: {len(symbols)} symbols from {library}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
