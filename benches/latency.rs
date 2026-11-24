//! Single-symbol latency benchmarks for the library API.
//!
//! The stages are deliberately separate:
//! - `detect` only;
//! - the backend's public `parse` entry point;
//! - `Demangler::demangle_bytes` (detection + parse + stored text);
//! - rendering into a null sink;
//! - rendering into an owned `String`.
//!
//! Rust and Swift `parse` already performs a printer-validation pass. C++ and D
//! store validated text in `Demangled`, so their full path includes rendering.
//! These boundaries are documented because "parse" is not the same operation
//! in every backend.

#![allow(clippy::significant_drop_tightening)]

use core::fmt::{self, Write};
use std::hint::black_box;

use criterion::{Criterion, criterion_group, criterion_main};
use demangler::{Demangler, Scheme};

struct NullSink;

impl Write for NullSink {
    fn write_str(&mut self, _text: &str) -> fmt::Result {
        Ok(())
    }
}

fn bench_detect(c: &mut Criterion) {
    let cases: &[(&str, &[u8])] = &[
        ("itanium", b"_ZN3Foo3barEv"),
        ("msvc", b"?x@@3HA"),
        ("legacy", b"foo__Fiv"),
        ("rust_legacy", b"_ZN3foo17h0123456789abcdefE"),
        ("rust_v0", b"_RNvCs1234_4main4main"),
        ("swift_stable", b"$s4main3fooyySiF"),
        ("swift_legacy", b"_TtSi"),
        ("go", b"fmt.Println"),
        ("d", b"_D3foo3barFZv"),
    ];
    let mut group = c.benchmark_group("detect");
    for (name, symbol) in cases {
        let _ = group.bench_function(*name, |b| {
            b.iter(|| black_box(Scheme::detect_bytes(black_box(*symbol))));
        });
    }
    group.finish();
}

fn bench_full(c: &mut Criterion) {
    let demangler = Demangler::new();
    let cases: &[(&str, &[u8])] = &[
        ("itanium", b"_ZN3Foo3barEv"),
        ("msvc", b"?x@@3HA"),
        ("legacy", b"foo__Fiv"),
        ("rust_legacy", b"_ZN3foo17h0123456789abcdefE"),
        ("rust_v0", b"_RNvCs1234_4main4main"),
        ("swift_stable", b"$s4main3fooyySiF"),
        ("swift_legacy", b"_TtSi"),
        ("go", b"fmt.Println"),
        ("d", b"_D3foo3barFZv"),
    ];
    let mut group = c.benchmark_group("full");
    for (name, symbol) in cases {
        let _ = group.bench_function(*name, |b| {
            b.iter(|| {
                let demangled = demangler
                    .demangle_bytes(black_box(*symbol))
                    .expect("benchmark symbol should demangle");
                black_box(demangled)
            });
        });
    }
    group.finish();

    let mut group = c.benchmark_group("full_to_string");
    for (name, symbol) in cases {
        let _ = group.bench_function(*name, |b| {
            b.iter(|| {
                let text = demangler
                    .demangle_bytes(black_box(*symbol))
                    .expect("benchmark symbol should demangle")
                    .to_string();
                black_box(text)
            });
        });
    }
    group.finish();

    let mut group = c.benchmark_group("render_sink");
    let mut sink = NullSink;
    for (name, symbol) in cases {
        let _ = group.bench_function(*name, |b| {
            b.iter(|| {
                let demangled = demangler
                    .demangle_bytes(black_box(*symbol))
                    .expect("benchmark symbol should demangle");
                write!(black_box(&mut sink), "{demangled}").unwrap();
            });
        });
    }
    group.finish();
}

#[cfg(feature = "cpp-itanium")]
fn bench_cpp_parse(c: &mut Criterion) {
    let mut group = c.benchmark_group("cpp_parse");
    for (name, symbol) in [
        ("itanium", "_ZN3Foo3barEv"),
        ("msvc", "?x@@3HA"),
        ("legacy", "foo__Fiv"),
    ] {
        let _ = group.bench_function(name, |b| {
            b.iter(|| black_box(demangler::cpp::parse(black_box(symbol)).is_ok()));
        });
    }
    group.finish();
}

#[cfg(feature = "rust")]
fn bench_rust_parse(c: &mut Criterion) {
    let mut group = c.benchmark_group("rust_parse");
    for (name, symbol) in [
        ("legacy", "_ZN3foo17h0123456789abcdefE"),
        ("v0", "_RNvCs1234_4main4main"),
    ] {
        let _ = group.bench_function(name, |b| {
            b.iter(|| black_box(demangler::rust::parse(black_box(symbol)).is_ok()));
        });
    }
    group.finish();
}

#[cfg(feature = "swift")]
fn bench_swift_parse(c: &mut Criterion) {
    let mut group = c.benchmark_group("swift_parse");
    for (name, symbol) in [
        ("stable", "$s4main3fooyySiF"),
        ("legacy", "_TtSi"),
    ] {
        let _ = group.bench_function(name, |b| {
            b.iter(|| black_box(demangler::swift::parse(black_box(symbol)).is_ok()));
        });
    }
    group.finish();
}

fn bench_parsers(c: &mut Criterion) {
    #[cfg(feature = "cpp-itanium")]
    bench_cpp_parse(c);
    #[cfg(feature = "rust")]
    bench_rust_parse(c);
    #[cfg(feature = "swift")]
    bench_swift_parse(c);
}

criterion_group!(benches, bench_detect, bench_full, bench_parsers);
criterion_main!(benches);
