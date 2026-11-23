//! Printer for parsed Free Pascal linker names.
//!
//! The printed text is this crate's contract, not Pascal source: the unit,
//! scope chain and routine are joined with `.`, the signature is written as a
//! parameter list and an optional result type, and every type token is printed
//! as mangled.

use core::fmt;

use super::node::{Name, Type};
use crate::parse::{Escape, write_escaped};

/// Render one linker name.
pub(crate) fn print<W: fmt::Write + ?Sized>(name: &Name<'_>, out: &mut W) -> fmt::Result {
    if name.program {
        out.write_str("P$")?;
    }
    write_escaped(out, name.unit, Escape::Identifier)?;
    for scope in &name.scopes {
        out.write_str(".")?;
        write_escaped(out, scope, Escape::Identifier)?;
    }
    out.write_str(".")?;
    write_escaped(out, name.routine, Escape::Identifier)?;
    out.write_str("(")?;
    for (index, param) in name.params.iter().enumerate() {
        if index > 0 {
            out.write_str(", ")?;
        }
        print_type(out, param)?;
    }
    out.write_str(")")?;
    if let Some(result) = &name.ret {
        out.write_str(": ")?;
        print_type(out, result)?;
    }
    Ok(())
}

/// Render one parameter or result type token.
fn print_type<W: fmt::Write + ?Sized>(out: &mut W, ty: &Type<'_>) -> fmt::Result {
    match ty {
        Type::Named(name) => write_escaped(out, name, Escape::Identifier),
        Type::ArrayOf(element) => {
            out.write_str("array of ")?;
            write_escaped(out, element, Escape::Identifier)
        }
        Type::ArrayOfConst => out.write_str("array of const"),
    }
}
