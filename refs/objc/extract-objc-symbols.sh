#!/bin/sh
# Regenerate the Objective-C symbol-name corpora in refs/objc/corpus/.
#
# The Apple and GNUstep sources are hand-written test programs compiled by
# Clang; the corpora contain only the resulting symbol names. The libobjc
# corpus is the metadata subset of the Debian GCC runtime archive. No binary,
# object file or source is stored under refs/.
set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
OUT="$ROOT/refs/objc/corpus"
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT

CLANG=${CLANG:-clang}
CC=${CC:-gcc}

if [ -n "${LLVM_NM:-}" ]; then
    :
elif command -v llvm-nm-19 >/dev/null 2>&1; then
    LLVM_NM=llvm-nm-19
elif command -v llvm-nm >/dev/null 2>&1; then
    LLVM_NM=llvm-nm
else
    echo "error: llvm-nm is required to read Mach-O objects" >&2
    exit 1
fi

GCC_INCLUDE=$("$CC" -print-file-name=include)

cat > "$TMP/apple.m" <<'EOF'
__attribute__((objc_root_class))
@interface Root { id isa; int _ivar; }
@property int prop;
+ (void)classThing;
- (void)bar:(int)a baz:(int)b;
@end
@implementation Root
@synthesize prop = _ivar;
+ (void)classThing {}
- (void)bar:(int)a baz:(int)b {}
@end
@protocol P
- (void)protoMethod;
@optional
+ (int)optClassMethod;
- (void)optInstanceMethod;
@end
@protocol Q <P>
- (void)qMethod;
@end
@interface Root (Cat) <P>
- (void)catMethod;
+ (void)catClassMethod;
@end
@implementation Root (Cat)
- (void)catMethod {}
+ (void)catClassMethod {}
@end
void use(void) { __block int x = 1; ^{ x++; }; }
EOF

cat > "$TMP/apple2.m" <<'EOF'
__attribute__((objc_root_class))
@interface Root
@property int prop;
+ (void)classThing;
- (void)bar:(int)a baz:(int)b;
@end
@implementation Root
@synthesize prop;
+ (void)classThing {}
- (void)bar:(int)a baz:(int)b {}
@end
@protocol P
- (void)protoMethod;
@optional
+ (int)optClassMethod;
- (void)optInstanceMethod;
@end
@protocol Q <P>
- (void)qMethod;
@end
@interface Root (Cat) <P>
@property (nonatomic) int catProp;
- (void)catMethod;
+ (void)catClassMethod;
@end
@implementation Root (Cat)
- (void)catMethod {}
+ (void)catClassMethod {}
@end
EOF

cat > "$TMP/proto.m" <<'EOF'
@protocol P
@property int pprop;
- (void)protoMethod;
@optional
+ (int)optClassMethod;
@end
@protocol Q <P>
- (void)qMethod;
@end
__attribute__((objc_root_class))
@interface Root <Q> @end
@implementation Root @end
void use(id x) { (void)[x conformsToProtocol:@protocol(P)]; }
EOF

cat > "$TMP/apple3.m" <<'EOF'
__attribute__((objc_root_class))
@interface Base3 { id isa; int _a; double _b; }
@property int a;
@property double b;
+ (void)baseClassThing;
- (void)baseThing:(int)x other:(double)y;
@end
@implementation Base3
@synthesize a = _a;
@synthesize b = _b;
+ (void)baseClassThing {}
- (void)baseThing:(int)x other:(double)y {}
@end

@protocol P3
@property int pprop;
- (void)protoThing;
@optional
+ (int)optClassThing;
- (void)optThing;
@end
@protocol Q3 <P3>
- (void)qThing;
@end

@interface Alpha3 : Base3 <Q3> { int _alpha; }
@property int alpha;
- (void)alphaThing:(id)x;
+ (id)makeAlpha;
@end
@implementation Alpha3
@synthesize alpha = _alpha;
- (void)alphaThing:(id)x { (void)x; }
+ (id)makeAlpha { return 0; }
- (void)qThing {}
@end

@interface Beta3 : Base3 { id _beta; }
@property id beta;
- (void)betaThing:(int)x;
@end
@implementation Beta3
@synthesize beta = _beta;
- (void)betaThing:(int)x { (void)x; }
@end

@interface Gamma3 : Base3 { char _gamma; }
@property char gamma;
- (void)gammaThing;
@end
@implementation Gamma3
@synthesize gamma = _gamma;
- (void)gammaThing {}
@end

@interface Base3 (Cat3) <P3>
@property int catProp3;
- (void)catThing:(id)x;
+ (void)catClassThing;
@end
@implementation Base3 (Cat3)
- (void)catThing:(id)x { (void)x; }
+ (void)catClassThing {}
@end

@interface Base3 (Other3)
- (void)otherThing;
@end
@implementation Base3 (Other3)
- (void)otherThing {}
@end

void use3(void) {
    __block int x = 1;
    ^{ x++; };
    ^{ (void)x; };
}
EOF

cat > "$TMP/gnustep.m" <<'EOF'
#include <objc/objc.h>
@interface Foo { int _x; }
@property int x;
+ (void)classThing;
- (void)bar:(int)a baz:(int)b;
@end
@implementation Foo
@synthesize x = _x;
+ (void)classThing {}
- (void)bar:(int)a baz:(int)b {}
@end
@protocol P
- (void)protoMethod;
@optional
+ (int)optClassMethod;
@end
@interface Foo (Cat) <P>
- (void)catMethod;
@end
@implementation Foo (Cat)
- (void)catMethod {}
@end
void use(void) { __block int x = 1; ^{ x++; }; }

@interface GBase { int _g; }
@property int g;
+ (void)gClassThing;
- (void)gThing:(id)x;
@end
@implementation GBase
@synthesize g = _g;
+ (void)gClassThing {}
- (void)gThing:(id)x { (void)x; }
@end

@protocol GP
- (void)gpThing;
@optional
+ (int)gpOpt;
@end

@interface GAlpha : GBase <GP> { id _ga; }
@property id ga;
- (void)gAlphaThing;
@end
@implementation GAlpha
@synthesize ga = _ga;
- (void)gAlphaThing {}
- (void)gpThing {}
@end

@interface GBase (GCat)
@property int gCat;
- (void)gCatThing;
+ (void)gCatClassThing;
@end
@implementation GBase (GCat)
- (void)gCatThing {}
+ (void)gCatClassThing {}
@end
EOF

for source in apple apple2 proto apple3; do
    "$CLANG" -target x86_64-apple-macosx10.15 -fblocks -c "$TMP/$source.m" -o "$TMP/$source.o"
done
"$CLANG" -c -fblocks -fobjc-runtime=gnustep-2.0 -I"$GCC_INCLUDE" \
    "$TMP/gnustep.m" -o "$TMP/gnustep-2.0.o"
"$CLANG" -c -fblocks -fobjc-runtime=gcc -I"$GCC_INCLUDE" \
    "$TMP/gnustep.m" -o "$TMP/gcc-runtime.o"

"$LLVM_NM" "$TMP/apple.o" "$TMP/apple2.o" "$TMP/proto.o" "$TMP/apple3.o" \
    | awk 'NF >= 2 { print $NF }' | sort -u > "$TMP/apple.raw"
"$LLVM_NM" "$TMP/apple.o" "$TMP/apple2.o" "$TMP/proto.o" "$TMP/apple3.o" \
    | sed -nE 's/^[0-9a-f]+ [tT] ([+-]\[.*\])$/\1/p' | sort -u > "$TMP/methods.raw"
nm "$TMP/gnustep-2.0.o" "$TMP/gcc-runtime.o" \
    | awk 'NF >= 2 { print $NF }' | sort -u > "$TMP/gnustep.raw"

LIBOBJC=${LIBOBJC:-}
if [ -z "$LIBOBJC" ]; then
    LIBOBJC=$(find /usr/lib/gcc -name libobjc.a -print -quit 2>/dev/null || true)
fi
if [ -n "$LIBOBJC" ] && [ -f "$LIBOBJC" ]; then
    nm "$LIBOBJC" | awk 'NF >= 2 { print $NF }' | sort -u > "$TMP/libobjc.raw"
else
    echo "warning: libobjc.a not found; leaving gcc-libobjc-14.txt unchanged" >&2
fi

grep -E '^(_|__)OBJC_|^___block_descriptor_' "$TMP/apple.raw" \
    | sort -u > "$OUT/apple-clang-19.txt"
cp "$TMP/methods.raw" "$OUT/methods-clang-19.txt"
grep -E '^_OBJC_|^__objc_(class_name_|class_ref_|ivar_offset_|ivar_offset_value_)|^__block_descriptor_' \
    "$TMP/gnustep.raw" | sort -u > "$OUT/gnustep-clang-19.txt"
if [ -f "$TMP/libobjc.raw" ]; then
    grep -E '^_OBJC_(Class|ClassName_|Instance|MetaClass_|METH_VAR_|Module|SELECTOR_TABLE|SYMBOLS)|^__objc_(class_name_|class_ref_)' \
        "$TMP/libobjc.raw" \
        | grep -v '^_OBJC_ClassName_$' \
        | sort -u > "$OUT/gcc-libobjc-14.txt"
fi

cat > "$OUT/rejected.txt" <<'EOF'
__objc_exec_class
__objc_load
__objc_accessors_init
__objc_class_links_resolved
__objc_category_name_Foo_Cat
_OBJC_ClassName_
_OBJC_METH_VAR_NAME_
_OBJC_NOPE_$_Foo
_OBJC_CLASS_$_
._OBJC_CLASS_Foo
.objc_category_FooCat
l_OBJC_LABEL_CLASS_$
l_OBJC_LABEL_CATEGORY_$
___copy_helper_block_e8_32r
___destroy_helper_block_e8_32r
___use_block_invoke
__NSConcreteStackBlock
EOF

for file in "$OUT"/*.txt; do
    printf '%s: %s names  %s\n' \
        "$(basename "$file")" \
        "$(wc -l < "$file" | tr -d ' ')" \
        "$(sha256sum "$file" | cut -d' ' -f1)"
done
