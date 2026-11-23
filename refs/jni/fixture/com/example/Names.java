package com.example;

/**
 * JNI corpus fixture: escaped characters in method names — `$` as `_0xxxx`, `_`
 * as `_1`, and a leading, trailing and doubled underscore. `under_score` and
 * `a__b` are overloaded, so their long-form symbols carry `_1` escapes in the
 * method part.
 *
 * Every native method descriptor is unique within the class, and no other class
 * in the fixture has the same descriptor multiset.
 */
public class Names {
    public native void dollar$name();

    public native void withUnderscore_1(char c);

    public native void under_score(int a);

    public native void under_score(long a);

    public native void trail_(short value);

    public native void _lead(byte value);

    public native void a_1b(boolean value);

    public native void a__b(float value);

    public native void a__b(double value);
}
