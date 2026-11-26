//! Printer for parsed OCaml linker names.
//!
//! The printed text is this crate's contract, not OCaml source: the
//! compilation-unit path and the member are joined with `.`, the numeric stamp
//! is dropped and the compiler's `$XX` escapes are decoded to the byte they
//! stand for. A static-data label keeps its number, and a runtime helper is
//! printed as it stands, because its linker name already is the C identifier.

use core::fmt;

use super::node::Form;
use crate::parse::{Escape, write_escaped};

/// Render one parsed name.
pub fn print<W: fmt::Write + ?Sized>(form: &Form<'_>, out: &mut W) -> fmt::Result {
    match form {
        Form::Module { path } => write_name(out, path),
        Form::Member { path, member } => {
            write_name(out, path)?;
            out.write_str(".")?;
            write_name(out, member)
        }
        Form::Label { path, label } => {
            write_name(out, path)?;
            out.write_str(".")?;
            write_name(out, label)
        }
        Form::Helper { symbol } => write_name(out, symbol),
        Form::Marker { unit, marker, join } => {
            write_name(out, unit)?;
            out.write_str(join.as_str())?;
            write_name(out, marker)
        }
    }
}

/// Write mangled name bytes, decoding `$XX` escapes.
///
/// The compiler escapes every byte outside `[A-Za-z0-9_.]`, so decoding turns
/// `$2b$21` back into the operator `+!`. A `.` is never escaped (it is the path
/// separator), so a hand-written `$2e` is kept verbatim and the printed dotted
/// path stays unambiguous; an escape for a non-printable byte is kept too,
/// because there is no readable spelling for it.
fn write_name<W: fmt::Write + ?Sized>(out: &mut W, bytes: &[u8]) -> fmt::Result {
    let mut literal = 0;
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'$'
            && let (Some(high), Some(low)) =
                (hex_value(bytes, index + 1), hex_value(bytes, index + 2))
        {
            let byte = high * 16 + low;
            if byte.is_ascii_graphic() && byte != b'.' {
                write_escaped(out, &bytes[literal..index], Escape::Verbatim)?;
                if byte == b'\\' {
                    // Identifier bytes use the reversible escape policy; a
                    // decoded backslash must not become a raw `\`.
                    out.write_str("\\\\")?;
                } else {
                    out.write_char(char::from(byte))?;
                }
                index += 3;
                literal = index;
                continue;
            }
        }
        index += 1;
    }
    write_escaped(out, &bytes[literal..], Escape::Verbatim)
}

/// The value of the lowercase hexadecimal digit at `index`, if there is one.
fn hex_value(bytes: &[u8], index: usize) -> Option<u8> {
    match bytes.get(index)? {
        byte @ b'0'..=b'9' => Some(byte - b'0'),
        byte @ b'a'..=b'f' => Some(byte - b'a' + 10),
        _ => None,
    }
}
