package com.example;

/**
 * JNI corpus fixture: every primitive descriptor byte, object and array types,
 * non-void returns, and the long form of a method whose parameters exercise
 * `_2` (`;`), `_3` (`[`) and `/` (a bare `_`) in the signature.
 *
 * Every native method descriptor is unique within the class, and no other class
 * in the fixture has the same descriptor multiset.
 */
public class Types {
    public native void baz(int i);

    public native void baz(boolean z, byte b, char c, short s, int i, long j, float f, double d);

    public native void arrs();

    public native void arrs(String[][] grid, int[][][] cube, Object[] items, byte[] raw);

    public native long get();

    public native long get(int index);

    public native Object obj(java.util.List<String> list, String[][] grid);
}
