//! Pure-Rust Kotlin/Native symbol demangling.
//!
//! Kotlin/Native is an LLVM backend with its own linker-symbol mangling,
//! unrelated to Kotlin/JVM descriptors and Kotlin/JS names. The compiler's
//! `KonanMangler` builds a declaration spelling from a qualified path, a `#`
//! member separator, value and type parameter lists, a return type and linkage
//! markers; the LLVM backend wraps it in a `k*:` prefix.
//!
//! No external demangler is used and there is no runtime dependency.
//!
//! # Supported forms
//!
//! - `kfun:` functions, constructors, property accessors, local functions,
//!   lambdas and compiler bridges;
//! - `ktype:` type symbols;
//! - `kclass:` classes, interfaces and objects;
//! - `kproperty:`, `kobj:`, `kfield:`, `ktypealias:`, `kvar:`;
//! - `kpackage:` and `knamespace:` package/namespace symbols;
//! - Mach-O leading-underscore variants (`_kfun:`, `__kfun:`);
//! - extension receivers (`@` in klib spellings, `__at__` in ELF symbols),
//!   `#static`, `#suspend`, `#companion`, `#internal`, `-impl`, `$` local and
//!   synthetic indexes, and `%` platform-specific suffixes;
//! - type parameters, bounds, type arguments, variance, nullable, star,
//!   vararg, flexible, `<dynamic>` and `<ERROR CLASS>` types.
//!
//! # Canonical output
//!
//! There is no standard Kotlin/Native demangler, so the printed text is this
//! crate's contract:
//!
//! | Input | Output |
//! |---|---|
//! | `kfun:com.example.foo#bar(kotlin.Int){}` | `com.example.foo.bar(kotlin.Int)` |
//! | `kfun:com.example.Box#<init>(kotlin.Int){}` | `com.example.Box.<init>(kotlin.Int)` |
//! | `kfun:com.example.Box#get(){}` | `com.example.Box.get()` |
//! | `kfun:com.example#topLevel(){}` | `com.example.topLevel()` |
//! | `ktype:com.example.Box<kotlin.Int>` | `com.example.Box<kotlin.Int>` |
//!
//! The spelling is a debugger name, not re-parsable Kotlin source:
//!
//! - `#` becomes `.`;
//! - `#static` and `#suspend` become `static ` and `suspend ` prefixes;
//! - `#internal` and `-impl` are linkage markers and are not printed;
//! - an extension receiver is printed as `name@receiver(...)`;
//! - type parameters are declared as `T<index>` and referenced as
//!   `T<container>:<index>`;
//! - variance is printed as `out ` / `in `, vararg as `vararg `, nullable as
//!   `?`, star as `*` and flexible types as `lower..upper`;
//! - a return type is appended as `: <type>`;
//! - a `%` platform suffix is appended as ` %<suffix>`.
//!
//! Unknown prefixes and unknown `#` markers are rejected, never guessed.
//!
//! # Raw bytes
//!
//! A symbol name is a byte string, so the parser matches the grammar on bytes
//! and never requires the whole name to be UTF-8. Text that is valid UTF-8
//! prints verbatim (so Unicode identifiers survive); bytes that are not valid
//! UTF-8 print escaped as `\xNN`. A backslash in a name prints as `\\`, and a
//! `%` platform suffix keeps its payload as written.
//!
//! # Detection
//!
//! [`crate::Scheme::detect`] claims only the known `k*:` prefixes (and their
//! leading-underscore variants) and tries them before Go and plain C. It does
//! not claim arbitrary `k...` names, and C++/Rust/Swift/Objective-C keep their
//! existing precedence.
//!
//! # Budgets
//!
//! Untrusted input stays bounded by [`MAX_INPUT_LEN`](crate::kotlin_native::MAX_INPUT_LEN), [`MAX_DEPTH`](crate::kotlin_native::MAX_DEPTH),
//! [`MAX_WORK`](crate::kotlin_native::MAX_WORK), [`MAX_PATH_SEGMENTS`](crate::kotlin_native::MAX_PATH_SEGMENTS), [`MAX_PARAMETERS`](crate::kotlin_native::MAX_PARAMETERS),
//! [`MAX_TYPE_ARGUMENTS`](crate::kotlin_native::MAX_TYPE_ARGUMENTS), [`MAX_TYPE_PARAMETERS`](crate::kotlin_native::MAX_TYPE_PARAMETERS) and [`MAX_OUTPUT_LEN`](crate::kotlin_native::MAX_OUTPUT_LEN).
//! Exceeding a budget is an error, never a partial result.
//!
//! # Gaps
//!
//! - Kotlin/JVM descriptors, Kotlin/JS names and Kotlin metadata blobs are out
//!   of scope.
//! - Kotlin/Native internal table symbols (`kintf:`, `kifacetable:`,
//!   `kifacevtable:`, `kexttype:`, `kextoff:`, `kextname:`, `krefs:`) are not
//!   declarations and are left unclaimed.
//! - The `%` platform suffix is preserved verbatim; its ObjC-export payload is
//!   not decoded.
//! - `ktype:`, `kproperty:`, `kobj:`, `ktypealias:`, `kpackage:` and
//!   `knamespace:` are implemented from the mangler's marker constants; Kotlin
//!   2.4.21 executables emit only `kfun:`, `kclass:` and `kvar:` symbols, so
//!   those prefixes are covered by hand-authored compatibility cases rather
//!   than compiler output.
mod node;
mod parser;
mod printer;
mod text;

use core::fmt;

use crate::parse::LimitedWriter;
pub use node::{
    Ast, Function, MAX_DEPTH, MAX_INPUT_LEN, MAX_OUTPUT_LEN, MAX_PARAMETERS, MAX_PATH_SEGMENTS,
    MAX_TYPE_ARGUMENTS, MAX_TYPE_PARAMETERS, MAX_WORK, Symbol, Type, TypeArgument, TypeParameter,
    Variance,
};
pub use parser::is_kotlin_native;
pub(crate) use parser::is_kotlin_native_bytes;

/// Why a Kotlin/Native symbol could not be parsed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ParseError {
    /// The symbol does not match a known Kotlin/Native form.
    Invalid,
    /// A type nests deeper than [`MAX_DEPTH`].
    RecursedTooDeep,
    /// The input, work or list budget was exceeded.
    BudgetExceeded,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Invalid => "invalid syntax",
            Self::RecursedTooDeep => "recursion limit reached",
            Self::BudgetExceeded => "size limit reached",
        })
    }
}

impl core::error::Error for ParseError {}

impl fmt::Display for Ast<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.print(f)
    }
}

impl Ast<'_> {
    /// Render the parsed symbol into `out`, enforcing [`MAX_OUTPUT_LEN`](crate::kotlin_native::MAX_OUTPUT_LEN).
    pub(crate) fn print<W: fmt::Write + ?Sized>(&self, out: &mut W) -> fmt::Result {
        let mut limited = LimitedWriter::new(out, MAX_OUTPUT_LEN);
        printer::print(&self.symbol, &mut limited)
    }
}

/// Parse a Kotlin/Native symbol into a reusable [`Ast`].
///
/// A Mach-O leading underscore (`_kfun:`) and a double underscore
/// (`__kfun:`) are accepted, as are klib `@` and ELF `__at__` extension
/// markers.
///
/// # Errors
///
/// Returns [`ParseError::Invalid`] when the prefix is unknown or a marker,
/// separator, list or type is malformed, [`ParseError::RecursedTooDeep`] when
/// a type nests deeper than [`MAX_DEPTH`], and
/// [`ParseError::BudgetExceeded`] when the input, work or list budget is
/// exceeded.
pub fn parse(symbol: &str) -> Result<Ast<'_>, ParseError> {
    parse_bytes(symbol.as_bytes())
}

/// Parse a Kotlin/Native symbol from raw bytes.
///
/// This is the entry point the other backends and the demangler dispatch to
/// once symbol bytes, not text, are the input. See [`parse`] for the accepted
/// forms; the only difference is that the name does not have to be UTF-8.
pub(crate) fn parse_bytes(symbol: &[u8]) -> Result<Ast<'_>, ParseError> {
    parser::parse_bytes(symbol)
}

#[cfg(test)]
mod tests {
    use alloc::string::ToString;

    use super::*;

    #[test]
    fn parse_is_display() {
        assert_eq!(
            parse("kfun:com.example#main(){}").unwrap().to_string(),
            "com.example.main()"
        );
    }

    #[test]
    fn errors_are_displayed() {
        assert_eq!(ParseError::Invalid.to_string(), "invalid syntax");
        assert_eq!(
            ParseError::RecursedTooDeep.to_string(),
            "recursion limit reached"
        );
        assert_eq!(ParseError::BudgetExceeded.to_string(), "size limit reached");
    }

    #[test]
    fn ast_exposes_structure() {
        let ast = parse("kfun:com.example.Box#get(){}kotlin.Int").unwrap();
        let Symbol::Function(function) = ast.symbol() else {
            panic!("not a function");
        };
        assert_eq!(function.path(), "com.example.Box");
        assert_eq!(function.name(), "get");
        assert!(function.params().is_empty());
        let Some(Type::Class { name, .. }) = function.return_type() else {
            panic!("not a class type");
        };
        assert_eq!(name, b"kotlin.Int");
    }

    #[test]
    fn depth_budget_reports_recursion() {
        // The shared cursor's depth budget is the only path to
        // `RecursedTooDeep` in this backend; nesting past `MAX_DEPTH` must not
        // fall through to a generic budget error.
        let mut symbol = alloc::string::String::from("ktype:kotlin.Array<");
        symbol.push_str(&"kotlin.Array<".repeat(MAX_DEPTH + 1));
        symbol.push('*');
        symbol.push_str(&">".repeat(MAX_DEPTH + 1));
        assert_eq!(parse(&symbol).unwrap_err(), ParseError::RecursedTooDeep);
    }

    #[test]
    fn printer_is_bounded() {
        // Every accepted input renders; the ceiling is enforced separately.
        let ast = parse("kfun:com.example#foo(kotlin.Int){}").unwrap();
        let mut out = alloc::string::String::new();
        ast.print(&mut out).unwrap();
        assert!(out.len() <= MAX_OUTPUT_LEN);
    }
}
