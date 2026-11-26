//! Bounded parser for OCaml native linker names.
//!
//! The grammar is the one the OCaml 5 compiler's symbol printer produces, as
//! measured in `refs/ocaml.md`:
//!
//! ```text
//! symbol  := "_"? ( module | helper )
//! module  := "caml" path [ "." ( member | label ) ]
//! path    := segment ( "." segment )*
//! segment := upper ( alnum | "_" | escape )*
//! member  := marker | body "_" digits
//! body    := ( alnum | "_" | "." escape-start | escape )+
//! label   := digits
//! helper  := "caml_" name [ "." marker | "__" marker | fragment ]
//! name    := alnum+ ( "_" alnum+ )*
//! fragment := ".cold" | ".part." digits
//! escape  := "$" lower-hex lower-hex
//! ```
//!
//! The grammar is matched on bytes through the shared [`Cursor`]: a linker name
//! is a byte string, so the parser never requires the whole input to be UTF-8.
//! Every byte outside `[A-Za-z0-9_.]` is written by the compiler as a `$XX`
//! escape, so the accepted byte set is closed and a name that carries anything
//! else is rejected instead of guessed at.
//!
//! Two rules keep the path/member split unambiguous. A member may start with an
//! uppercase letter, because a functor is a module (`camlOcaml_probe.F_310`), so
//! the member is tried before the path grows. A member's only `.` is the
//! leading dot of an indexing operator, which the compiler always writes as `.`
//! followed by an escape (`.%()` is `.$25$28$29`), so any other dot ends the
//! member and starts another path segment or a static-data label.

use super::ParseError;
use super::node::{Ast, Form, MAX_PATH_SEGMENTS, MarkerJoin, ParseLimits};
use crate::parse::{Cursor, Fail, FailKind, State};

/// The section markers the compiler emits around every unit's data.
///
/// They are the only members without a numeric stamp: `asmcomp` emits one
/// `caml<unit>.code_begin`/`code_end`/`data_begin`/`data_end`/`frametable`/
/// `gc_roots` group plus `caml<unit>.entry`, the module initialiser.
const MARKERS: [&[u8]; 7] = [
    b"entry",
    b"code_begin",
    b"code_end",
    b"data_begin",
    b"data_end",
    b"frametable",
    b"gc_roots",
];

/// The runtime's C-level pseudo-units, whose markers are named `caml_<unit>`.
///
/// The runtime and the startup stub are C, not OCaml, so their unit names are
/// lowercase and carry the `caml_` prefix of the C runtime: `caml_startup`,
/// `caml_hot` and `caml_system` are the three the runtime emits.
const PSEUDO_UNITS: [&[u8]; 3] = [b"startup", b"hot", b"system"];

/// The assembler's doubled-underscore marker separator.
const DOUBLE_UNDERSCORE: &[u8] = b"__";

/// GCC's `-freorder-blocks-and-partition` cold-half suffix.
const COLD_FRAGMENT: &[u8] = b".cold";

/// The partial-inlining fragment prefix; a decimal index follows.
const PART_FRAGMENT: &[u8] = b".part.";

/// Parse an OCaml linker name from raw bytes within `limits`.
pub(crate) fn parse_bytes(symbol: &[u8], limits: ParseLimits) -> Result<Ast<'_>, ParseError> {
    let symbol = crate::scheme::strip_decorations_bytes(symbol);
    if symbol.is_empty() {
        return Err(ParseError::Invalid);
    }
    if symbol.len() > limits.input {
        return Err(ParseError::BudgetExceeded);
    }

    let mut state = State::new(limits.cursor());
    let mut cursor = Cursor::new(symbol, 0, &mut state).map_err(map_fail)?;

    // Darwin and PE prefix external symbols with `_`; the canonical spelling
    // the printer emits drops it.
    if cursor.peek() == Some(b'_') {
        advance(&mut cursor, 1)?;
    }
    let start = cursor.pos();
    if !cursor.starts_with(b"caml") {
        return Err(ParseError::Invalid);
    }
    advance(&mut cursor, 4)?;

    let form = if cursor.peek() == Some(b'_') {
        parse_helper(&mut cursor, start)?
    } else {
        parse_module(&mut cursor)?
    };
    if !cursor.remaining().is_empty() {
        return Err(ParseError::Invalid);
    }
    Ok(Ast { form })
}

/// Translate a shared-cursor failure into this backend's public error.
fn map_fail(fail: Fail) -> ParseError {
    match fail.kind {
        FailKind::Depth | FailKind::InputBudget | FailKind::Work => ParseError::BudgetExceeded,
        _ => ParseError::Invalid,
    }
}

/// Advance the cursor by `len` bytes, translating budget failures.
fn advance(cursor: &mut Cursor<'_, '_>, len: usize) -> Result<(), ParseError> {
    let _ = cursor.advance(len).map_err(map_fail)?;
    Ok(())
}

/// Parse `caml<path>` and the optional `.`-separated member or static-data
/// label.
fn parse_module<'a>(cursor: &mut Cursor<'a, '_>) -> Result<Form<'a>, ParseError> {
    let start = cursor.pos();
    let mut segments = 1_usize;
    parse_segment(cursor)?;
    loop {
        if cursor.remaining().is_empty() {
            let path = cursor
                .bytes_between(start, cursor.pos())
                .ok_or(ParseError::Invalid)?;
            return Ok(Form::Module { path });
        }
        if cursor.peek() != Some(b'.') {
            return Err(ParseError::Invalid);
        }
        // A member may start with an uppercase letter: a functor is a module, so
        // `camlOcaml_probe.F_310` is the member `F` of unit `Ocaml_probe`, not the
        // path `Ocaml_probe.F`. The member is therefore tried first, and the
        // path only grows when the rest is not a complete member.
        let checkpoint = cursor.checkpoint();
        let separator = cursor.pos();
        advance(cursor, 1)?;
        if let Some(member) = parse_member(cursor)? {
            let path = cursor
                .bytes_between(start, separator)
                .ok_or(ParseError::Invalid)?;
            return Ok(Form::Member { path, member });
        }
        cursor.reset(checkpoint);
        // `caml<path>.<N>`: a compiler-emitted static-data label. The label is a
        // bare decimal number, so it can never be a member, a marker or a path
        // segment, and it must consume the rest of the name.
        if cursor
            .peek_at(1)
            .is_some_and(|byte| byte.is_ascii_digit())
        {
            advance(cursor, 1)?;
            let label_start = cursor.pos();
            while matches!(cursor.peek(), Some(byte) if byte.is_ascii_digit()) {
                advance(cursor, 1)?;
            }
            if !cursor.remaining().is_empty() {
                return Err(ParseError::Invalid);
            }
            let path = cursor
                .bytes_between(start, separator)
                .ok_or(ParseError::Invalid)?;
            let label = cursor
                .bytes_between(label_start, cursor.pos())
                .ok_or(ParseError::Invalid)?;
            return Ok(Form::Label { path, label });
        }
        if segments >= MAX_PATH_SEGMENTS {
            return Err(ParseError::BudgetExceeded);
        }
        if !cursor
            .peek_at(1)
            .is_some_and(|byte| byte.is_ascii_uppercase())
        {
            return Err(ParseError::Invalid);
        }
        advance(cursor, 1)?;
        parse_segment(cursor)?;
        segments += 1;
    }
}

/// Parse `caml_<name>`, then a pseudo-unit marker or a host-compiler fragment.
///
/// `start` is the offset of the `caml` prefix, which the helper form keeps.
fn parse_helper<'a>(cursor: &mut Cursor<'a, '_>, start: usize) -> Result<Form<'a>, ParseError> {
    advance(cursor, 1)?;
    let name_start = cursor.pos();
    loop {
        parse_word(cursor)?;
        if cursor.peek() != Some(b'_') {
            break;
        }
        // A doubled underscore is not a helper-name separator: it starts the
        // assembler's `caml_<unit>__<marker>` spelling.
        if cursor.peek_at(1) == Some(b'_') {
            break;
        }
        advance(cursor, 1)?;
    }
    let name = cursor
        .bytes_between(name_start, cursor.pos())
        .ok_or(ParseError::Invalid)?;
    if cursor.remaining().is_empty() {
        let symbol = cursor
            .bytes_between(start, cursor.pos())
            .ok_or(ParseError::Invalid)?;
        return Ok(Form::Helper { symbol });
    }
    // GCC's cold-half and partial-inlining fragments are linker suffixes on a C
    // runtime function, not part of its name: `caml_ba_compare.cold` is the cold
    // half of `caml_ba_compare`. They are peeled off, so the helper prints as it
    // stands without them.
    if let Some(len) = fragment_len(cursor.remaining()) {
        let symbol = cursor
            .bytes_between(start, cursor.pos())
            .ok_or(ParseError::Invalid)?;
        advance(cursor, len)?;
        return Ok(Form::Helper { symbol });
    }
    if !PSEUDO_UNITS.contains(&name) {
        return Err(ParseError::Invalid);
    }
    let join = if cursor.starts_with(DOUBLE_UNDERSCORE) {
        MarkerJoin::DoubleUnderscore
    } else if cursor.peek() == Some(b'.') {
        MarkerJoin::Dot
    } else {
        return Err(ParseError::Invalid);
    };
    advance(cursor, join.as_str().len())?;
    let marker = cursor.remaining();
    if !is_marker(marker) {
        return Err(ParseError::Invalid);
    }
    advance(cursor, marker.len())?;
    Ok(Form::Marker {
        unit: name,
        marker,
        join,
    })
}

/// The length of a host-compiler fragment suffix, if `rest` is exactly one.
///
/// The suffix set is the one measured on the linked runtime: GCC writes
/// `<name>.cold` for the outlined cold half of a function and `<name>.part.<N>`
/// for a partially inlined body. Anything else, including a bare `.cold.<N>`
/// LLVM index that the runtime was not observed to produce, is rejected instead
/// of being guessed at.
fn fragment_len(rest: &[u8]) -> Option<usize> {
    if rest == COLD_FRAGMENT {
        return Some(rest.len());
    }
    let index = rest.strip_prefix(PART_FRAGMENT)?;
    if index.is_empty() || !index.iter().all(u8::is_ascii_digit) {
        return None;
    }
    Some(rest.len())
}

/// Parse one module-path segment: an uppercase letter, then identifier bytes.
fn parse_segment(cursor: &mut Cursor<'_, '_>) -> Result<(), ParseError> {
    match cursor.peek() {
        Some(byte) if byte.is_ascii_uppercase() => advance(cursor, 1)?,
        _ => return Err(ParseError::Invalid),
    }
    loop {
        match cursor.peek() {
            Some(byte) if byte.is_ascii_alphanumeric() || byte == b'_' => advance(cursor, 1)?,
            Some(b'$') => parse_escape(cursor)?,
            _ => break,
        }
    }
    Ok(())
}

/// Parse a complete member at the cursor.
///
/// A member is a compiler marker or `<body>_<digits>`, and it must consume the
/// rest of the input. `Ok(None)` means the rest is not a member; the cursor has
/// moved and the caller restores its checkpoint.
fn parse_member<'a>(cursor: &mut Cursor<'a, '_>) -> Result<Option<&'a [u8]>, ParseError> {
    let start = cursor.pos();
    loop {
        match cursor.peek() {
            Some(byte) if byte.is_ascii_alphanumeric() || byte == b'_' => advance(cursor, 1)?,
            Some(b'$') => parse_escape(cursor)?,
            // The only `.` a member carries is the leading dot of an indexing
            // operator, which the compiler always writes as `.` + `$XX`
            // (`.%()` is `.$25$28$29`). Any other dot ends the member.
            Some(b'.') if cursor.pos() == start && cursor.peek_at(1) == Some(b'$') => {
                advance(cursor, 1)?;
            }
            _ => break,
        }
    }
    if cursor.pos() == start || !cursor.remaining().is_empty() {
        return Ok(None);
    }
    let member = cursor
        .bytes_between(start, cursor.pos())
        .ok_or(ParseError::Invalid)?;
    if is_marker(member) {
        return Ok(Some(member));
    }
    Ok(stamped_body(member))
}

/// Parse a `caml_` helper name component: one alphanumeric run.
fn parse_word(cursor: &mut Cursor<'_, '_>) -> Result<(), ParseError> {
    let start = cursor.pos();
    while matches!(cursor.peek(), Some(byte) if byte.is_ascii_alphanumeric()) {
        advance(cursor, 1)?;
    }
    if cursor.pos() == start {
        return Err(ParseError::Invalid);
    }
    Ok(())
}

/// Consume one `$XX` escape. The compiler writes the two digits in lowercase.
fn parse_escape(cursor: &mut Cursor<'_, '_>) -> Result<(), ParseError> {
    let (Some(high), Some(low)) = (cursor.peek_at(1), cursor.peek_at(2)) else {
        return Err(ParseError::Invalid);
    };
    if !is_lower_hex(high) || !is_lower_hex(low) {
        return Err(ParseError::Invalid);
    }
    advance(cursor, 3)
}

/// True when `member` is one of the compiler's section markers.
fn is_marker(member: &[u8]) -> bool {
    MARKERS.contains(&member)
}

/// The body of a stamped member, `<body>_<digits>`, without its stamp.
fn stamped_body(member: &[u8]) -> Option<&[u8]> {
    let index = member.iter().rposition(|byte| *byte == b'_')?;
    let body = member.get(..index)?;
    let stamp = member.get(index.checked_add(1)?..)?;
    if body.is_empty() || stamp.is_empty() || !stamp.iter().all(u8::is_ascii_digit) {
        return None;
    }
    Some(body)
}

/// True for a lowercase hexadecimal digit.
const fn is_lower_hex(byte: u8) -> bool {
    byte.is_ascii_digit() || matches!(byte, b'a'..=b'f')
}

/// True when a symbol is a fully valid OCaml linker name.
///
/// Detection is a full parse: the `caml` prefix alone is not enough, because
/// `caml_`-prefixed C names and identifiers such as `camlist` share it.
/// Detection runs with the compiled-in budget; a [`crate::Demangler`] applies
/// its configured budget to the dispatch parse.
#[must_use]
pub fn is_ocaml(symbol: &[u8]) -> bool {
    super::parse_bytes(symbol).is_ok()
}
