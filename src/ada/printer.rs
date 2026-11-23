//! Printer for parsed Ada/GNAT names.

use core::fmt;

use super::node::{ElabSuffix, Name};
use crate::parse::{Escape, write_escaped};

/// Render a qualified Ada name and its elaboration suffix.
pub(crate) fn print<W: fmt::Write + ?Sized>(name: &Name<'_>, out: &mut W) -> fmt::Result {
    for (index, segment) in name.segments.iter().enumerate() {
        if index > 0 {
            out.write_str(".")?;
        }
        write_escaped(out, segment, Escape::Identifier)?;
    }
    match name.suffix {
        Some(ElabSuffix::Body) => out.write_str("'Elab_Body"),
        Some(ElabSuffix::Spec) => out.write_str("'Elab_Spec"),
        None => Ok(()),
    }
}
