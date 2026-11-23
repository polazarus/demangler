package q_w;

/**
 * JNI corpus fixture: a package and class whose names contain `_`, so the
 * mangled class part carries `_1` escapes and the signature carries them inside
 * an object type.
 *
 * Every native method descriptor is unique within the class, and no other class
 * in the fixture has the same descriptor multiset.
 */
public class x {
    public native void m(q_w.x value);

    public native void m(int value);
}
