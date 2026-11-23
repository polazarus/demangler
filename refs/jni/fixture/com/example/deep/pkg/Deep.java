package com.example.deep.pkg;

/** JNI corpus fixture: a multi-segment package with primitive array parameters. */
public class Deep {
    public native void method();

    public native void method(int[] a);

    public native void method(int[][] a);

    public native void method(String a);
}
