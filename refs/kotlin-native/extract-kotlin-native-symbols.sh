#!/bin/sh
# Extract Kotlin/Native linker symbol names from a pinned compiler.
#
# The compiler archive is downloaded to a cache directory (never committed),
# verified against the release SHA256, and used to build the fixture under
# refs/kotlin-native/fixture/. Only symbol names are written to the corpus.
#
# Requirements: java (a JRE is not bundled in the prebuilt archive) and nm.
set -eu

KOTLIN_VERSION="2.4.21"
KOTLIN_SHA256="4935d8c8da90a2d2a8ff51da7d2c264c0d6026e1d35a8f169e218d9209965678"
KOTLIN_ARCHIVE="kotlin-native-prebuilt-linux-x86_64-${KOTLIN_VERSION}.tar.gz"
KOTLIN_URL="https://github.com/JetBrains/kotlin/releases/download/v${KOTLIN_VERSION}/${KOTLIN_ARCHIVE}"
CACHE_DIR="${KOTLIN_NATIVE_CACHE:-/tmp/kotlin-native-cache}"
EXTRACT_DIR="${CACHE_DIR}/kotlin-native-${KOTLIN_VERSION}"
CORPUS_DIR="$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)/corpus"
CORPUS="${CORPUS_DIR}/kotlin-${KOTLIN_VERSION}-linux-x86_64.txt"
FIXTURE="$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)/fixture/main.kt"

command -v java >/dev/null 2>&1 || {
    echo "error: java is required by konanc" >&2
    exit 1
}
command -v nm >/dev/null 2>&1 || {
    echo "error: nm is required to dump the symbol table" >&2
    exit 1
}

mkdir -p "${CACHE_DIR}"
if [ ! -f "${CACHE_DIR}/${KOTLIN_ARCHIVE}" ]; then
    echo "downloading ${KOTLIN_URL}" >&2
    curl -fL --retry 3 -o "${CACHE_DIR}/${KOTLIN_ARCHIVE}" "${KOTLIN_URL}"
fi
printf '%s  %s\n' "${KOTLIN_SHA256}" "${CACHE_DIR}/${KOTLIN_ARCHIVE}" | sha256sum -c -

if [ ! -x "${EXTRACT_DIR}/bin/konanc" ]; then
    mkdir -p "${EXTRACT_DIR}"
    tar -xzf "${CACHE_DIR}/${KOTLIN_ARCHIVE}" -C "${EXTRACT_DIR}" --strip-components=1
fi

WORK_DIR="$(mktemp -d)"
trap 'rm -rf "${WORK_DIR}"' EXIT

"${EXTRACT_DIR}/bin/konanc" \
    "${FIXTURE}" \
    -o "${WORK_DIR}/fixture" \
    -entry com.example.kdemo.main \
    -Xno-inline \
    -g

nm "${WORK_DIR}/fixture.kexe" |
    awk '{print $NF}' |
    grep -E '^_?k(fun|type|class|property|obj|field|typealias|package|namespace|var):' |
    LC_ALL=C sort -u > "${CORPUS}"

echo "wrote ${CORPUS}" >&2
wc -l "${CORPUS}" >&2
