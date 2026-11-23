#!/usr/bin/env python3
"""Generate deterministic adversarial symbol-name corpora.

These are stress inputs, not expected-output corpora. The test harness checks
that every line terminates, does not panic, and stays within its backend's
output budget (or is rejected).
"""
from pathlib import Path

OUT = Path(__file__).resolve().parent


def write(name, lines):
    (OUT / name).write_text("\n".join(lines) + "\n")
    print(f"{name}: {len(lines)} lines")


write("cpp-itanium.txt", [
    "_Z1f" + "P" * 2000 + "i",
    "_Z" + "N" * 1200 + "1A" + "E",
    "_Z1f" + "I" * 1200 + "1A" + "E" * 1200,
    "_Z1f" + "F" * 1200 + "v" + "E" * 1200,
    "_Z" + "A" * 1200 + "i" + "E" * 1200,
    "_Z1f" + "P" * 600 + "K" * 600 + "i",
])
write("cpp-msvc.txt", [
    "?x@@3" + "PEA" * 2000 + "HA",
    "?f@@YAX" + "PEA" * 2000 + "XZ",
    "?x@@3" + "PEA" * 1000 + "P" * 1000 + "HA",
    "?f@@YA" + "PEA" * 2000 + "XZ",
])
write("cpp-legacy.txt", [
    "foo__F" + "P" * 2000 + "i",
    "a__" * 4000 + "b__Fv",
    "foo__F" + "A5_" * 1000 + "i",
    "foo__F" + "T0" * 2000 + "i",
])
write("rust.txt", [
    "_RIC3foo" + "P" * 600 + "iE",
    "_R" + "I" * 600 + "C0" + "E" * 600,
    "_RMC0" + "T" * 600 + "p" + "E",
    "_ZN" + "1a" * 60000 + "E",
    "_ZN" + "9" * 60 + "fooE",
])
write("swift.txt", [
    "$s" + "Sg" * 2000 + "Si",
    "$s" + "G" * 1500 + "Si",
    "$s" + "R" * 1500 + "Si",
    "$s4main3foo" + "y" * 3000,
])
write("go.txt", [
    "pkg.F" + "[" * 2000 + "]" * 2000,
    "type:.eq.pkg.F" + "[" * 2000 + "]" * 2000,
    "go:string." + "A" * 60000,
    "pkg." + "A" * 60000,
])
write("objc.txt", [
    "-[Foo bar" + "[" * 2000 + "]" * 2000 + "]",
    "_OBJC_CLASS_$_" + "A" * 60000,
    "__block_descriptor_" + "A" * 60000,
    "__OBJC_$_CLASS_METHODS_" + "A" * 60000,
])
write("kotlin-native.txt", [
    "ktype:kotlin.Array<" + "kotlin.Array<" * 1200 + "*" + ">" * 1200,
    "kfun:com.example#" + "f" * 60000 + "(){}",
    "kclass:" + "com.example.F" * 10000,
])
write("c-plain.txt", [
    "a" * 60000,
    "A_" * 30000,
])
