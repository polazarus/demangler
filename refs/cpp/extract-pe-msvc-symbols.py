#!/usr/bin/env python3
"""Extract decorated C++ names from a PE/COFF export table with objdump.

This is local tooling, not vendored upstream material. It is intentionally
small so the corpus can be regenerated from a pinned binary:

    python3 refs/cpp/extract-pe-msvc-symbols.py \
        --binary /tmp/torch/lib/torch_cpu.dll \
        --output refs/cpp/msvc/real-torch-2.5.1-cpu-win64-torch_cpu.txt

Only exported names are visible in a stripped DLL. That is still useful: the
export table is exactly what a caller sees at runtime. `objdump` 2.43 prints
one `[ordinal] +base[...] ... name` line per export. The last field is the
decorated name.
"""

from __future__ import annotations

import argparse
import pathlib
import subprocess
import sys


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--binary", required=True, type=pathlib.Path)
    parser.add_argument("--output", required=True, type=pathlib.Path)
    args = parser.parse_args()

    dump = subprocess.run(
        ["objdump", "-p", str(args.binary)],
        check=True,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.DEVNULL,
    )
    symbols = set()
    for line in dump.stdout.splitlines():
        if "+base[" not in line:
            continue
        name = line.split()[-1]
        if name.startswith("?"):
            symbols.add(name)

    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text("".join(f"{symbol}\n" for symbol in sorted(symbols)))
    print(f"{args.output}: {len(symbols)} symbols from {args.binary}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
