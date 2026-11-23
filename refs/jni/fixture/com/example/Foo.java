package com.example;

/**
 * JNI corpus fixture: a non-overloaded method, overloads, an object parameter
 * and nested classes.
 *
 * Every class in the fixture keeps native method descriptors unique so that a
 * header symbol can be paired with a `javap` row by (class, descriptor) alone,
 * and every class keeps a descriptor multiset that no other class shares.
 */
public class Foo {
    public native void plain();

    public native int bar(String[] args, int x);

    public native void bar(int x);

    public native void bar(long x);

    public native void inner(Foo.Inner value);

    public native void inner(short x);

    public static class Inner {
        public native void innerNative();

        public native void innerNative(int x);
    }

    public class NonStatic {
        public native void nonStatic();
    }
}
