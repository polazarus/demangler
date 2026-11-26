//! Printer for parsed Free Pascal linker names.
//!
//! The printed text is this crate's contract, not Pascal source: the unit,
//! scope chain and member are joined with `.`, the signature is written as a
//! parameter list and an optional result type, and every token is printed as
//! mangled. A compiler data symbol keeps its prefix (`U_$`, `VMT_$`, `RTTI_$`,
//! `INIT_$`, `INIT$_$`, `FINALIZE$_$`) and its `$indirect` marker verbatim.

use core::fmt;

use super::node::{Kind, Name, Type};
use crate::parse::{Escape, write_escaped};

/// Render one linker name.
pub fn print<W: fmt::Write + ?Sized>(name: &Name<'_>, out: &mut W) -> fmt::Result {
    match name.kind {
        Kind::Routine => {}
        Kind::UnitVariable => out.write_str("U_$")?,
        Kind::VirtualMethodTable => out.write_str("VMT_$")?,
        Kind::RuntimeTypeInfo => out.write_str("RTTI_$")?,
        Kind::TypeInit => out.write_str("INIT_$")?,
        Kind::UnitInit => out.write_str("INIT$_$")?,
        Kind::UnitFinalize => out.write_str("FINALIZE$_$")?,
    }
    if name.program {
        out.write_str("P$")?;
    }
    write_escaped(out, name.unit, Escape::Identifier)?;
    for scope in &name.scopes {
        out.write_str(".")?;
        write_escaped(out, scope, Escape::Identifier)?;
    }
    if let Some(member) = name.member {
        out.write_str(".")?;
        write_escaped(out, member, Escape::Identifier)?;
        if name.kind == Kind::Routine {
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
        }
        if name.indirect {
            out.write_str("$indirect")?;
        }
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
