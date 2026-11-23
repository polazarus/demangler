# Turn `javac -h` JNI headers into corpus rows of the form
# `symbol<TAB>java-like display`.
#
# Input, in order:
#
# 1. `javap -p -s` output for the compiled fixture classes (the JVM's own
#    spelling of every class and native method);
# 2. the `javac -h` headers (one per class).
#
# The symbol column is javac's own mangling. The display column is built from
# `javap`: it names the class, the method and the parameter types, so the
# expected text never depends on this crate's escaping rules.
#
# Pairing: the `Signature:` comment of a header block is the *plain* JVM
# descriptor, the same string `javap` reports, so a header file is matched to
# its class by the multiset of its native method descriptors, and a block to its
# method by (class, descriptor). Both matches must be unique, which the fixture
# guarantees and this script verifies. The `Class:` and `Method:` comments are
# deliberately not used: javac writes them with a different, lossy mangling
# (`$` becomes `__`, `_` stays unescaped), so they cannot be decoded.
#
# A method whose name occurs more than once in its class is overloaded, which is
# exactly when javac appends `__<signature>` to the symbol; only then is the
# parameter list printed.
#
# Input is read with LC_ALL=C so that `substr` works on bytes. Any malformed
# row, ambiguous class or missing member aborts with a non-zero exit status.

function fail(message) {
    printf "jni corpus: %s (line %d of %s)\n", message, FNR, FILENAME > "/dev/stderr"
    failed = 1
}

# A header `Signature:` comment is the plain JVM descriptor, except that javac
# flattens a nested-class `$` to `/` (`(Lcom/example/Foo/Inner;)V` for
# `(Lcom/example/Foo$Inner;)V`). The javap side is flattened the same way for
# the pairing key only; `parameters_of` keeps javap's own spelling, which is
# what the display text uses. A flattened collision makes the descriptor
# ambiguous and aborts the run.
function flatten(descriptor) {
    gsub(/\$/, "/", descriptor)
    return descriptor
}

# Sort the space-separated tokens of `text` and join them with `|`.
function sorted_key(text,   parts, count, i, j, tmp, out) {
    count = split(text, parts, " ")
    for (i = 2; i <= count; i++) {
        tmp = parts[i]
        for (j = i - 1; j >= 1 && parts[j] > tmp; j--) parts[j + 1] = parts[j]
        parts[j + 1] = tmp
    }
    out = ""
    for (i = 1; i <= count; i++) out = out "|" parts[i]
    return out
}

# ---- pass 1: javap ---------------------------------------------------------

NR == FNR {
    if ($0 ~ /^ *descriptor: /) {
        if (member != "") {
            key = class SUBSEP flatten($2)
            if (key in method_of) {
                fail("duplicate descriptor " $2 " in " class)
            } else {
                method_of[key] = member
                parameters_of[key] = parameters
                overloads[class SUBSEP member]++
                descriptors[class] = descriptors[class] " " flatten($2)
            }
        }
        member = ""
        next
    }
    if ($0 ~ /\(/ && $0 ~ /;$/) {
        member = ""
        if ($0 ~ / native /) {
            line = $0
            sub(/;.*$/, "", line)
            open = index(line, "(")
            member = substr(line, 1, open - 1)
            sub(/.* /, "", member)
            parameters = substr(line, open + 1, length(line) - open - 1)
        }
        next
    }
    if ($0 ~ /\{$/) {
        line = $0
        sub(/ *\{$/, "", line)
        class = line
        sub(/.* /, "", class)
        next
    }
    next
}

# ---- pass 2: javac -h headers ---------------------------------------------

/^\/\*$/ { in_comment = 1; next }
/^ \*\/$/ { in_comment = 0; next }
in_comment && $2 == "Signature:" { signature = $3; next }

$3 == "JNICALL" {
    if (signature == "") {
        fail("symbol without a Signature comment")
        next
    }
    rows++
    symbol[rows] = $4
    row_signature[rows] = signature
    row_file[rows] = FILENAME
    file_signatures[FILENAME] = file_signatures[FILENAME] " " signature
    signature = ""
}

END {
    if (failed) exit 1

    for (file in file_signatures) {
        wanted = sorted_key(file_signatures[file])
        matches = 0
        for (class in descriptors) {
            if (sorted_key(descriptors[class]) == wanted) {
                matches++
                matched_class = class
            }
        }
        if (matches != 1) {
            printf "jni corpus: %d javap classes match %s\n", matches, file > "/dev/stderr"
            exit 1
        }
        class_of_file[file] = matched_class
    }

    for (i = 1; i <= rows; i++) {
        class = class_of_file[row_file[i]]
        key = class SUBSEP row_signature[i]
        if (!(key in method_of)) {
            fail("no javap member for " class " " row_signature[i])
            continue
        }
        name = class "." method_of[key]
        if (overloads[class SUBSEP method_of[key]] > 1) name = name "(" parameters_of[key] ")"
        print symbol[i] "\t" name
    }

    if (failed) exit 1
    if (rows == 0) {
        printf "jni corpus: no rows\n" > "/dev/stderr"
        exit 1
    }
}
