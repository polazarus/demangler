package üni.codé;

/**
 * JNI corpus fixture: a non-ASCII package, class and method names. The mangling
 * escapes every non-alphanumeric character as `_0xxxx`, so a supplementary
 * character (here U+1D538, MATHEMATICAL DOUBLE-STRUCK CAPITAL A) is emitted as
 * a UTF-16 surrogate pair of escapes.
 *
 * Every native method descriptor is unique within the class so that a header
 * symbol can be paired with a `javap` row by (class, descriptor) alone.
 */
public class Ünïcode {
    public native void café();

    public native void café(int x);

    public native void 𝔸bc(char c);

    public native void 𝔸bc(boolean z);

    public native void mix𝔸_1(String text);
}
