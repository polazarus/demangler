//! Dotted rendering of a parsed gfortran name.
//!
//! The display form is documented in [`crate::fortran`]: a module symbol prints
//! as `<module>.<member>`, an external name without its appended underscore, the
//! main program as `main`, and a generated derived-type helper as
//! `<module>.<Type>'<helper>`. Parsing has already validated every byte, so
//! printing only writes text.

use core::fmt;

use super::node::Name;
use crate::parse::{Escape, write_escaped};

/// Write the dotted spelling of `name`.
pub fn print<W: fmt::Write + ?Sized>(name: &Name<'_>, out: &mut W) -> fmt::Result {
    match name {
        Name::Module { module, member } => {
            write_escaped(out, module, Escape::Identifier)?;
            out.write_str(".")?;
            write_escaped(out, member, Escape::Identifier)
        }
        Name::External { name } => write_escaped(out, name, Escape::Identifier),
        Name::Main => out.write_str("main"),
        Name::TypeHelper {
            module,
            tag,
            type_name,
        } => {
            write_escaped(out, module, Escape::Identifier)?;
            out.write_str(".")?;
            write_escaped(out, type_name, Escape::Identifier)?;
            out.write_str(tag.suffix())
        }
    }
}
