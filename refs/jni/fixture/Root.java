/**
 * JNI corpus fixture: a class in the default package, so the mangled name has
 * no package separator at all, and the empty-signature long form.
 */
public class Root {
    public native void ping();

    public native void ping(byte value);
}
