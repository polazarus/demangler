use core::fmt::{self, Write};

/// How bytes are rendered as text.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Escape {
    /// Reversible identifier spelling: `\` becomes `\\`, invalid bytes `\xNN`.
    Identifier,
    /// Compiler payload spelling: valid UTF-8 is verbatim, invalid bytes `\xNN`.
    Verbatim,
}

/// Write identifier or payload bytes using the chosen policy.
pub fn write_escaped<W: Write + ?Sized>(out: &mut W, bytes: &[u8], policy: Escape) -> fmt::Result {
    for chunk in bytes.utf8_chunks() {
        if policy == Escape::Identifier {
            // Keep the spelling reversible and prevent C0 controls/DEL from
            // reaching a terminal or a line-oriented consumer.
            for character in chunk.valid().chars() {
                if character == '\\' {
                    out.write_str("\\\\")?;
                } else if character.is_control() {
                    let mut bytes = [0_u8; 4];
                    for byte in character.encode_utf8(&mut bytes).as_bytes() {
                        write!(out, "\\x{byte:02x}")?;
                    }
                } else {
                    out.write_char(character)?;
                }
            }
        } else {
            out.write_str(chunk.valid())?;
        }
        for byte in chunk.invalid() {
            write!(out, "\\x{byte:02x}")?;
        }
    }
    Ok(())
}
