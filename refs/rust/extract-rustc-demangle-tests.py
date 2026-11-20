#!/usr/bin/env python3
"""Extract the test cases from rustc-demangle's inline test modules.

Local tooling, not upstream material. It reads the vendored
`refs/rust/rustc-demangle/src/{lib,v0,legacy}.rs` and writes normalized
`mangled ---> expected` corpora to `refs/rust/corpus/`:

  rustc-demangle-tests.txt        `Display` output (hashes and disambiguators)
  rustc-demangle-tests-nohash.txt `{:#}` output (hashes omitted)
  rustc-demangle-tests-errors.txt inputs that `try_demangle` must reject

The `t!`/`t_nohash!`/`t_err!` macros are expanded, including the
`t_nohash_type!`, `t_const!` and `t_const_suffixed!` wrappers, so the
extracted pairs match what the upstream tests assert.

Usage: python3 refs/rust/extract-rustc-demangle-tests.py
"""

import pathlib
import re
import sys

ROOT = pathlib.Path(__file__).resolve().parent / "rustc-demangle" / "src"
SOURCES = ["lib.rs", "v0.rs", "legacy.rs"]

ESCAPES = {"n": "\n", "r": "\r", "t": "\t", "0": "\0", "\\": "\\", '"': '"', "'": "'"}


def decode_rust_string(lit: str) -> str:
    assert lit.startswith('"') and lit.endswith('"'), lit
    body = lit[1:-1]
    out = []
    i = 0
    while i < len(body):
        c = body[i]
        if c != "\\":
            out.append(c)
            i += 1
            continue
        nxt = body[i + 1]
        if nxt in ESCAPES:
            out.append(ESCAPES[nxt])
            i += 2
        elif nxt == "u":
            end = body.index("}", i)
            out.append(chr(int(body[i + 3 : end], 16)))
            i = end + 1
        elif nxt == "\n":
            i += 2
            while i < len(body) and body[i] in " \t\n":
                i += 1
        else:
            raise ValueError(f"unsupported escape \\{nxt} in {lit}")
    return "".join(out)


def _raw_string_end(text: str, start: int) -> int | None:
    """Return the index just past a Rust raw string starting at `start`."""
    if text[start] != "r":
        return None
    i = start + 1
    hashes = 0
    while i < len(text) and text[i] == "#":
        hashes += 1
        i += 1
    if i >= len(text) or text[i] != '"':
        return None
    close = '"' + "#" * hashes
    end = text.find(close, i + 1)
    return None if end < 0 else end + len(close)


def split_top_level(text: str) -> list[str]:
    parts, depth, start, in_str = [], 0, 0, False
    i = 0
    while i < len(text):
        c = text[i]
        if in_str:
            if c == "\\":
                i += 2
                continue
            if c == '"':
                in_str = False
        else:
            raw_end = _raw_string_end(text, i)
            if raw_end is not None:
                i = raw_end
                continue
            if c == '"':
                in_str = True
            elif c in "([{":
                depth += 1
            elif c in ")]}":
                depth -= 1
            elif c == "," and depth == 0:
                parts.append(text[start:i].strip())
                start = i + 1
        i += 1
    tail = text[start:].strip()
    if tail:
        parts.append(tail)
    return parts


def macro_calls(source: str, name: str):
    for m in re.finditer(rf"\b{name}!\s*\(", source):
        depth, i = 0, m.end() - 1
        in_str = False
        while i < len(source):
            c = source[i]
            if in_str:
                if c == "\\":
                    i += 2
                    continue
                if c == '"':
                    in_str = False
            else:
                raw_end = _raw_string_end(source, i)
                if raw_end is not None:
                    i = raw_end
                    continue
                if c == '"':
                    in_str = True
                elif c == "(":
                    depth += 1
                elif c == ")":
                    depth -= 1
                    if depth == 0:
                        break
            i += 1
        yield source[m.end() : i]


RAW_STRING = re.compile(r'r(#*)"(.*)"\1', re.S)


def eval_arg(arg: str, bindings: dict[str, str]) -> str | None:
    arg = arg.strip()
    if arg.startswith('"'):
        return decode_rust_string(arg)
    raw = RAW_STRING.fullmatch(arg)
    if raw:
        return raw.group(2)
    if arg in bindings:
        return bindings[arg]
    concat = re.fullmatch(r"concat!\((.*)\)", arg, re.S)
    if concat:
        pieces = [eval_arg(p, bindings) for p in split_top_level(concat.group(1))]
        if any(p is None for p in pieces):
            return None
        return "".join(pieces)  # type: ignore[arg-type]
    return None


def main() -> int:
    exact: set[tuple[str, str]] = set()
    nohash: set[tuple[str, str]] = set()
    errors: set[str] = set()

    for name in SOURCES:
        source = (ROOT / name).read_text()
        bindings = {
            m.group(1): decode_rust_string(m.group(2))
            for m in re.finditer(r'let\s+(\w+)\s*=\s*("(?:[^"\\]|\\.)*");', source)
        }

        for body in macro_calls(source, "t"):
            args = [eval_arg(a, bindings) for a in split_top_level(body)]
            if len(args) == 2 and all(a is not None for a in args):
                exact.add((args[0], args[1]))
            else:
                print(f"skip t! {body.strip()[:60]!r}", file=sys.stderr)

        for body in macro_calls(source, "t_nohash"):
            args = [eval_arg(a, bindings) for a in split_top_level(body)]
            if len(args) == 2 and all(a is not None for a in args):
                nohash.add((args[0], args[1]))
            else:
                print(f"skip t_nohash! {body.strip()[:60]!r}", file=sys.stderr)

        for body in macro_calls(source, "t_err"):
            args = [eval_arg(a, bindings) for a in split_top_level(body)]
            if len(args) == 1 and args[0] is not None:
                errors.add(args[0])
            else:
                print(f"skip t_err! {body.strip()[:60]!r}", file=sys.stderr)

        for body in macro_calls(source, "t_nohash_type"):
            args = [eval_arg(a, bindings) for a in split_top_level(body)]
            if len(args) == 2 and all(a is not None for a in args):
                nohash.add(("_RMC0" + args[0], "<" + args[1] + ">"))
            else:
                print(f"skip t_nohash_type! {body.strip()[:60]!r}", file=sys.stderr)

        for body in macro_calls(source, "t_const"):
            args = [eval_arg(a, bindings) for a in split_top_level(body)]
            if len(args) == 2 and all(a is not None for a in args):
                nohash.add(("_RIC0K" + args[0] + "E", "::<" + args[1] + ">"))
            else:
                print(f"skip t_const! {body.strip()[:60]!r}", file=sys.stderr)

        for body in macro_calls(source, "t_const_suffixed"):
            args = [eval_arg(a, bindings) for a in split_top_level(body)]
            if len(args) == 3 and all(a is not None for a in args):
                mangled = "_RIC0K" + args[0] + "E"
                nohash.add((mangled, "::<" + args[1] + ">"))
                exact.add((mangled, "::<" + args[1] + args[2] + ">"))
            else:
                print(f"skip t_const_suffixed! {body.strip()[:60]!r}", file=sys.stderr)

    def write(path: pathlib.Path, pairs) -> None:
        with path.open("w") as out:
            for left, right in sorted(pairs):
                out.write(f"{left} ---> {right}\n")

    corpus = ROOT.parent.parent / "corpus"
    corpus.mkdir(exist_ok=True)
    write(corpus / "rustc-demangle-tests.txt", exact)
    write(corpus / "rustc-demangle-tests-nohash.txt", nohash)
    with (corpus / "rustc-demangle-tests-errors.txt").open("w") as out:
        for sym in sorted(errors):
            out.write(f"{sym}\n")

    print(
        f"exact={len(exact)} nohash={len(nohash)} errors={len(errors)}",
        file=sys.stderr,
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
