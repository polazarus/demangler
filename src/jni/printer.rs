//! Java-like rendering of a parsed JNI name.
//!
//! The display form is documented in [`crate::jni`]: the dotted class path, the
//! method name and, for a long-form name, the argument list in parentheses.
//! Parsing has already decoded every escape, so printing only writes text.

use core::fmt::{self, Write};

use super::Ast;

/// Write the Java-like spelling of `ast`.
pub(crate) fn print<W: Write + ?Sized>(ast: &Ast<'_>, out: &mut W) -> fmt::Result {
    out.write_str(ast.class())?;
    out.write_char('.')?;
    out.write_str(ast.method())?;
    let Some(descriptor) = ast.descriptor() else {
        return Ok(());
    };
    out.write_char('(')?;
    for (index, argument) in descriptor.args().iter().enumerate() {
        if index > 0 {
            out.write_str(", ")?;
        }
        write!(out, "{argument}")?;
    }
    out.write_char(')')
}
