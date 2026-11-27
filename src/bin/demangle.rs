//! # demangle CLI
//!
//! Basic command-line front end for the `demangler` library.
//!
//! Usage:
//!   demangle SYMBOL...     demangle the given symbols
//!   demangle -             read symbols from stdin, one per line
//!   demangle               same as `demangle -`

use std::io::{self, BufRead, Write};

use demangler::Demangler;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let demangler = Demangler::new();
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut out = io::BufWriter::new(stdout.lock());
    if run(&args, &mut stdin.lock(), &mut out, &demangler).is_err() || out.flush().is_err() {
        std::process::exit(1);
    }
}

/// Run the CLI over arguments or standard input.
fn run(
    args: &[String],
    input: &mut impl BufRead,
    out: &mut impl Write,
    demangler: &Demangler,
) -> io::Result<()> {
    if args.is_empty() || (args.len() == 1 && args[0] == "-") {
        for line in input.lines().map_while(Result::ok) {
            write_symbol(demangler, out, &line)?;
        }
    } else {
        for arg in args {
            write_symbol(demangler, out, arg)?;
        }
    }
    Ok(())
}

/// Demangle one symbol, falling back to the original text on failure.
fn write_symbol(demangler: &Demangler, out: &mut impl Write, symbol: &str) -> io::Result<()> {
    match demangler.demangle(symbol) {
        Ok(demangled_symbol) => writeln!(out, "{demangled_symbol}"),
        Err(_) => writeln!(out, "{symbol}"),
    }
}

#[cfg(test)]
#[path = "demangle/tests.rs"]
mod tests;
