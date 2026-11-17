//! # demangle CLI
//!
//! Basic command-line front end for the `demangle` library.
//!
//! Usage:
//!   demangle <symbol>...           demangle the given symbol names
//!   demangle -                     read symbol names from stdin, one per line
//!   demangle                       same as `demangle -`

use std::io::{self, BufRead, Write};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();

    if args.is_empty() || args == ["-"] {
        let stdin = io::stdin();
        let mut stdout = io::stdout().lock();
        for line in stdin.lock().lines().map_while(Result::ok) {
            let result = demangle::Demangler::new().demangle(&line);
            let _ = writeln!(stdout, "{}", result.name);
        }
        return;
    }

    let stdout = io::stdout();
    let mut out = stdout.lock();
    for arg in args {
        let result = demangle::Demangler::new().demangle(&arg);
        let _ = writeln!(out, "{}", result.name);
    }
}
