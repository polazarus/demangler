#!/bin/sh
# Refresh the pinned Swift demangler references.
#
# Project tooling, not upstream material; excluded from the published crate
# together with the rest of refs/.
#
# Policy: coverage follows *released* toolchains. A grammar snapshot from main
# may be vendored as a forward reference, but only released productions are
# implemented. See refs/swift.md and refs/swift/coverage.md.
#
# Usage:
#   ./refresh.sh grammar <commit-sha>
#   ./refresh.sh toolchain <version> <full-image> <oracle-image>
#   ./refresh.sh check
#
# `full-image` is the official toolchain image (has `nm` and `swift-demangle`);
# `oracle-image` is the slim image built from refs/swift/Containerfile. After a
# refresh: update SHA/date/hashes in refs/swift.md, update
# refs/swift/coverage.md, run the corpus tests, and raise floors only.
set -eu

dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)

usage() {
    sed -n '2,24p' "$0"
    exit 2
}

fetch() {
    curl -fsSL --retry 3 --retry-delay 5 "$1" -o "$2"
}

[ $# -ge 1 ] || usage

case $1 in
grammar)
    [ $# -eq 2 ] || usage
    sha=$2
    base="https://raw.githubusercontent.com/swiftlang/swift/$sha"
    fetch "$base/docs/ABI/Mangling.rst" "$dir/Mangling.rst"
    fetch "$base/test/Demangle/Inputs/manglings.txt" "$dir/manglings.txt"
    fetch "$base/LICENSE.txt" "$dir/LICENSE.txt"
    echo "vendored grammar at $sha"
    sha256sum "$dir/Mangling.rst" "$dir/manglings.txt" "$dir/LICENSE.txt"
    echo "update the pinned SHA/date in refs/swift.md, then run the corpus tests"
    ;;
toolchain)
    [ $# -eq 4 ] || usage
    version=$2
    full=$3
    oracle=$4
    if ! command -v podman >/dev/null 2>&1; then
        echo "podman is required for the container oracle" >&2
        exit 1
    fi
    podman image exists "$full" || podman pull "$full"
    podman image exists "$oracle" || podman build -t "$oracle" -f "$dir/Containerfile" "$dir"
    tmp=$(mktemp -d)
    trap 'rm -rf "$tmp"' EXIT
    podman run --rm -i "$full" sh -s >"$tmp/symbols" <<'EOS'
nm -D --defined-only /usr/lib/swift/linux/libswift*.so | awk '{print $3}' | grep '^\$s' | sort -u
EOS
    if [ ! -s "$tmp/symbols" ]; then
        echo "no \$s symbols found in $full" >&2
        exit 1
    fi
    podman run --rm -i "$oracle" -compact <"$tmp/symbols" >"$tmp/demangled"
    out=$dir/toolchain-$version-manglings.txt
    python3 - "$tmp/symbols" "$tmp/demangled" "$out" <<'PY'
import sys

symbols = open(sys.argv[1]).read().splitlines()
demangled = open(sys.argv[2]).read().splitlines()
with open(sys.argv[3], "w") as out:
    for mangled, expected in zip(symbols, demangled):
        if mangled != expected:
            out.write(f"{mangled} ---> {expected}\n")
PY
    echo "wrote $out"
    sha256sum "$out"
    echo "update the toolchain version/tag/hash/date in refs/swift.md"
    ;;
check)
    for f in "$dir/Mangling.rst" "$dir/manglings.txt" "$dir/LICENSE.txt"; do
        sha256sum "$f"
    done
    for f in "$dir"/toolchain-*-manglings.txt; do
        [ -f "$f" ] && sha256sum "$f"
    done
    if command -v podman >/dev/null 2>&1; then
        podman images --format '{{.Repository}}:{{.Tag}} {{.Digest}}' \
            | grep -E '^docker\.io/library/swift:|^localhost/swift-oracle:' || true
    fi
    echo "compare these hashes and image digests with the pins in refs/swift.md"
    ;;
*)
    usage
    ;;
esac
