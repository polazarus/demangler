//! Printer for parsed Ada/GNAT names.

use core::fmt;

use super::node::{Name, Suffix};
use crate::parse::{Escape, write_escaped};

/// Render a qualified Ada name and its compiler suffix.
///
/// The elaboration suffixes print as Ada attribute-style text. The operation,
/// entry and task suffixes name a generated subprogram of the entity, so they
/// print nothing: `pkg__prot__setN` and `pkg__prot__setP` both render as
/// `pkg.prot.set`, which is what `c++filt -s gnat` prints too.
pub(crate) fn print<W: fmt::Write + ?Sized>(name: &Name<'_>, out: &mut W) -> fmt::Result {
    for (index, segment) in name.segments.iter().enumerate() {
        if index > 0 {
            out.write_str(".")?;
        }
        write_escaped(out, segment, Escape::Identifier)?;
    }
    match name.suffix {
        Some(Suffix::ElabBody) => out.write_str("'Elab_Body"),
        Some(Suffix::ElabSpec) => out.write_str("'Elab_Spec"),
        Some(Suffix::ProtectedOp | Suffix::EntryBarrier | Suffix::EntryBody | Suffix::TaskBody)
        | None => Ok(()),
    }
}
