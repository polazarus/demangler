//! # demangle CLI
//!
//! Basic command-line front end for the `demangle` library.
//!
//! Usage:
//!   demangle <symbol>...   demangle the given symbols
//!   demangle -             read symbols from stdin, one per line
//!   demangle               same as `demangle -`

use std::io::{self, BufRead, Write};

use demangle::Demangler;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let demangler = Demangler::new();
    let stdout = io::stdout();
    let mut out = stdout.lock();

    if args.is_empty() || args == ["-"] {
        let stdin = io::stdin();
        for line in stdin.lock().lines().map_while(Result::ok) {
            write_symbol(&demangler, &mut out, &line);
        }
    } else {
        for arg in &args {
            write_symbol(&demangler, &mut out, arg);
        }
    }
}

/// Demangle one symbol, falling back to the original text on failure.
fn write_symbol(demangler: &Demangler, out: &mut impl Write, symbol: &str) {
    match demangler.demangle(symbol) {
        Ok(demangled) => {
            let _ = writeln!(out, "{demangled}");
        }
        Err(_) => {
            let _ = writeln!(out, "{symbol}");
        }
    }
}
