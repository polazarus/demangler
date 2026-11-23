#!/usr/bin/env python3
"""Classify raw D symbol/expected pairs into refs/d/ TSVs.

Usage: classify-corpus.py RAW_DIR OUT_DIR

RAW_DIR contains libphobos.syms/libphobos.expected and
app.syms/app.expected as produced by extract-d-symbols.sh. Rows where the
oracle output equals the input are oracle gaps; all other rows are exact.
"""

import pathlib
import re
import sys


def labels(symbol, expected):
    categories = set()
    if "TypeInfo_" in symbol:
        categories.add("typeinfo")
    if (
        symbol == "_Dmain"
        or expected.startswith(
            (
                "initializer for ",
                "vtable for ",
                "ClassInfo for ",
                "Interface for ",
                "ModuleInfo for ",
            )
        )
        or expected.endswith(".__moduleRef")
        or any(marker in symbol for marker in ("__ctor", "__dtor", "__postblit"))
    ):
        categories.add("runtime-special")
    if "__T" in symbol or "__U" in symbol:
        categories.add("template")
    if "delegate" in expected:
        categories.add("delegate")
    if "[" in expected:
        categories.add("array")
    if "__S" in symbol or (")." in expected and "template" not in categories):
        categories.add("local")
    if any(
        text in expected
        for text in ("ClassInfo for ", "vtable for ", ".this(", ".~this", "this(this)")
    ):
        categories.add("class")
    if re.search(r'!\([^)]*(?:null|\d|")', expected):
        categories.add("constant")
    if "(" in expected:
        categories.add("function")
    if "." in expected:
        categories.add("qualified")
    if not categories:
        categories.add("name")
    return ",".join(sorted(categories))


def pairs(raw, stem):
    symbols = (raw / f"{stem}.syms").read_text().splitlines()
    expected = (raw / f"{stem}.expected").read_text().splitlines()
    if len(symbols) != len(expected):
        raise SystemExit(f"{stem}: symbol/expected count mismatch")
    return list(zip(symbols, expected))


def split(raw, stem):
    exact = []
    gaps = []
    for symbol, expected in pairs(raw, stem):
        if expected == symbol:
            gaps.append(symbol)
        else:
            exact.append((symbol, expected, labels(symbol, expected)))
    return exact, gaps


def write_exact(path, rows):
    path.write_text("".join(f"{s}\t{e}\t{c}\n" for s, e, c in rows))


def main():
    if len(sys.argv) != 3:
        raise SystemExit(__doc__)
    raw = pathlib.Path(sys.argv[1])
    out = pathlib.Path(sys.argv[2])
    out.mkdir(parents=True, exist_ok=True)

    phobos, phobos_gaps = split(raw, "libphobos")
    app, app_gaps = split(raw, "app")
    write_exact(out / "libphobos-2.110.0.tsv", phobos)
    write_exact(out / "ldc-1.40-app.tsv", app)
    (out / "oracle-gap.tsv").write_text(
        "".join(f"{s}\tc++filt echoes the input\n" for s in phobos_gaps + app_gaps)
    )
    print(
        f"libphobos exact={len(phobos)} gaps={len(phobos_gaps)}; "
        f"app exact={len(app)} gaps={len(app_gaps)}"
    )


if __name__ == "__main__":
    main()
