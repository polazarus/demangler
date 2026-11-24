//! Bounded parser for Kotlin/Native linker symbols.
//!
//! The grammar is the compiler mangler's own output, documented in
//! `refs/kotlin-native.md`: a `k*:` prefix, a qualified path, a `#` member
//! separator (or `.` for private symbols), a value-parameter list, a
//! type-parameter list, a return type and linkage markers. Unknown markers are
//! rejected rather than guessed.
//!
//! The grammar is matched on bytes through the shared [`Cursor`]: a symbol name
//! is a byte string, so the parser never requires the whole input to be UTF-8.
//! Text is interpreted only when it is printed (see [`super::text`]).

use alloc::boxed::Box;
use alloc::vec::Vec;

use super::ParseError;
use super::node::{
    Ast, Function, MAX_DEPTH, MAX_INPUT_LEN, MAX_PARAMETERS, MAX_PATH_SEGMENTS, MAX_TYPE_ARGUMENTS,
    MAX_TYPE_PARAMETERS, MAX_WORK, Symbol, Type, TypeArgument, TypeParameter, Variance,
};
use crate::parse::{Cursor, Fail, FailKind, Limits, State};

/// The UTF-8 encoding of `§`, which separates a type parameter's index from
/// its upper bounds.
const SECTION_SIGN: &[u8] = "§".as_bytes();

/// The known Kotlin/Native symbol prefixes.
const PREFIXES: [(&[u8], Prefix); 10] = [
    (b"kfun:", Prefix::Function),
    (b"ktype:", Prefix::Type),
    (b"kclass:", Prefix::Class),
    (b"kproperty:", Prefix::Property),
    (b"kobj:", Prefix::Object),
    (b"kfield:", Prefix::Field),
    (b"ktypealias:", Prefix::TypeAlias),
    (b"kpackage:", Prefix::Package),
    (b"knamespace:", Prefix::Namespace),
    (b"kvar:", Prefix::Variable),
];

/// A recognised prefix family.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Prefix {
    Function,
    Type,
    Class,
    Property,
    Object,
    Field,
    TypeAlias,
    Package,
    Namespace,
    Variable,
}

/// Parse a Kotlin/Native symbol name from raw bytes.
pub(crate) fn parse_bytes(symbol: &[u8]) -> Result<Ast<'_>, ParseError> {
    if symbol.is_empty() {
        return Err(ParseError::Invalid);
    }
    let symbol = strip_platform_underscores(symbol);
    let (prefix, body) = split_prefix(symbol).ok_or(ParseError::Invalid)?;

    let mut state = State::new(limits());
    let mut cursor = Cursor::new(symbol, 0, &mut state).map_err(map_fail)?;
    charge(&mut cursor, symbol.len())?;
    // Offset of the body inside the symbol the cursor scans.
    let body_offset = symbol.len() - body.len();

    let symbol = match prefix {
        Prefix::Function => Symbol::Function(parse_function(&mut cursor, body, body_offset)?),
        Prefix::Type => Symbol::Type(parse_type_body(&mut cursor, body, body_offset)?),
        Prefix::Class => Symbol::Class(parse_named(body, false)?),
        Prefix::Property => Symbol::Property(parse_named(body, true)?),
        Prefix::Object => Symbol::Object(parse_named(body, false)?),
        Prefix::Field => Symbol::Field(parse_named(body, false)?),
        Prefix::TypeAlias => Symbol::TypeAlias(parse_named(body, false)?),
        Prefix::Package => Symbol::Package(parse_named(body, false)?),
        Prefix::Namespace => Symbol::Namespace(parse_named(body, false)?),
        Prefix::Variable => Symbol::Variable(parse_named(body, false)?),
    };
    Ok(Ast { symbol })
}

/// The budgets the shared cursor enforces.
///
/// The parser has no node budget: it builds one symbol with its own bounded
/// lists.
fn limits() -> Limits {
    Limits {
        input: MAX_INPUT_LEN as u32,
        depth: MAX_DEPTH as u16,
        work: MAX_WORK as u32,
    }
}

/// Translate a shared-cursor failure into this backend's public error.
fn map_fail(fail: Fail) -> ParseError {
    match fail.kind() {
        FailKind::Depth => ParseError::RecursedTooDeep,
        FailKind::InputBudget | FailKind::Work  => {
            ParseError::BudgetExceeded
        }
        _ => ParseError::Invalid,
    }
}

/// Charge `units` of work against the shared budget without moving the cursor.
///
/// The shared cursor is the only work meter the combinator layer exposes, so a
/// scan that works on a slice charges by advancing over it and rewinding.
fn charge(cursor: &mut Cursor<'_, '_>, units: usize) -> Result<(), ParseError> {
    let checkpoint = cursor.checkpoint();
    let _ = cursor.advance(units).map_err(map_fail)?;
    cursor.reset(checkpoint);
    Ok(())
}

/// True when `name` starts with a known Kotlin/Native prefix.
///
/// This is the cheap prefilter used by [`crate::Scheme::detect`]. It accepts
/// the Mach-O leading-underscore variants (`_kfun:`, `__kfun:`) and claims a
/// known prefix even when the body is malformed, so
/// [`crate::kotlin_native::parse`] can reject it.
/// It never claims an arbitrary `k...` name.
#[must_use]
pub fn is_kotlin_native(name: &str) -> bool {
    is_kotlin_native_bytes(name.as_bytes())
}

/// True when `name` starts with a known Kotlin/Native prefix, as raw bytes.
///
/// Detection is a byte-prefix test, so a name is claimed regardless of whether
/// the rest of it decodes as UTF-8.
pub(crate) fn is_kotlin_native_bytes(name: &[u8]) -> bool {
    if name.is_empty() || name.len() > MAX_INPUT_LEN {
        return false;
    }
    split_prefix(strip_platform_underscores(name)).is_some()
}

/// Strip one or two leading platform underscores from a Mach-O symbol.
fn strip_platform_underscores(name: &[u8]) -> &[u8] {
    name.strip_prefix(b"__")
        .or_else(|| name.strip_prefix(b"_"))
        .unwrap_or(name)
}

/// Split a known prefix from the body.
fn split_prefix(name: &[u8]) -> Option<(Prefix, &[u8])> {
    PREFIXES
        .iter()
        .find_map(|(prefix, kind)| name.strip_prefix(*prefix).map(|body| (*kind, body)))
}

/// Parse a `kfun:` function.
fn parse_function<'a>(
    cursor: &mut Cursor<'a, '_>,
    body: &'a [u8],
    body_offset: usize,
) -> Result<Function<'a>, ParseError> {
    let (body, internal, is_impl) = strip_function_markers(body);
    if body.is_empty() {
        return Err(ParseError::Invalid);
    }
    let (path, name, rest) = split_function_head(body, internal)?;
    let path = validate_path(path, false)?;
    let name = validate_name(name)?;

    // The signature starts where the head ends; the cursor scans it in place,
    // bounded by the marker-stripped body.
    let signature_offset = body_offset + (body.len() - rest.len());
    let _ = cursor.advance(signature_offset).map_err(map_fail)?;
    let mut parser = Parser::new(cursor, body_offset + body.len());
    let mut is_static = false;
    let mut is_suspend = false;
    let mut companion = None;

    loop {
        if parser.eat_marker(b"#static")? {
            is_static = true;
        } else if parser.eat_marker(b"#suspend")? {
            is_suspend = true;
        } else if parser.eat_marker(b"#companion")? {
            if !parser.eat_at_marker()? {
                return Err(ParseError::Invalid);
            }
            companion = Some(parser.parse_path(false)?);
        } else if parser.eat_marker(b"#internal")? {
            // A marker in the middle of a signature is only produced by
            // private declarations; treat it like the trailing marker.
            return Err(ParseError::Invalid);
        } else if parser.peek_is(b'#') {
            return Err(ParseError::Invalid);
        } else {
            break;
        }
    }

    let mut receiver = if parser.eat_at_marker()? {
        Some(parser.parse_type()?)
    } else {
        None
    };

    let mut groups: Vec<Vec<Type<'a>>> = Vec::new();
    loop {
        if parser.eat_char(b'(')? {
            groups.push(parser.parse_parameters()?);
        } else if parser.eat_at_marker()? {
            if receiver.is_some() {
                return Err(ParseError::Invalid);
            }
            receiver = Some(parser.parse_type()?);
        } else {
            break;
        }
    }

    let saw_params = !groups.is_empty();
    let (context_params, params) = match groups.len() {
        0 => (Vec::new(), Vec::new()),
        1 => (Vec::new(), groups.pop().unwrap_or_default()),
        2 => {
            let params = groups.pop().unwrap_or_default();
            let context = groups.pop().unwrap_or_default();
            (context, params)
        }
        _ => return Err(ParseError::Invalid),
    };

    let mut signature = saw_params || receiver.is_some() || is_static || is_suspend;
    let type_params = if parser.eat_char(b'{')? {
        signature = true;
        parser.parse_type_parameters()?
    } else {
        Vec::new()
    };

    let ret = if signature && !parser.is_end() && !parser.peek_is(b'%') {
        Some(parser.parse_type()?)
    } else {
        None
    };

    // A compiler bridge's member name embeds the target's signature, and the
    // bridge's own signature follows it. Parse the outer signature so the
    // symbol is accepted; the canonical spelling shows the target signature.
    if !parser.is_end() && parser.peek_is(b'(') {
        parser.expect_char(b'(')?;
        let _ = parser.parse_parameters()?;
        if parser.eat_char(b'{')? {
            let _ = parser.parse_type_parameters()?;
        }
        if !parser.is_end() && !parser.peek_is(b'%') {
            let _ = parser.parse_type()?;
        }
    }

    let platform = if parser.eat_char(b'%')? {
        let suffix = parser.remaining();
        if suffix.is_empty() || suffix.iter().copied().any(is_platform_suffix_stop) {
            return Err(ParseError::Invalid);
        }
        parser.skip_to_end()?;
        Some(suffix)
    } else {
        None
    };

    if !parser.is_end() {
        return Err(ParseError::Invalid);
    }

    Ok(Function {
        path,
        name,
        receiver,
        context_params,
        params,
        type_params,
        ret,
        is_static,
        is_suspend,
        is_internal: internal,
        is_impl,
        companion,
        platform,
        signature,
    })
}

/// Strip the trailing `-impl`, `#internal` and `#internal.<n>` markers from a
/// function body.
fn strip_function_markers(mut body: &[u8]) -> (&[u8], bool, bool) {
    let mut internal = false;
    let mut is_impl = false;
    loop {
        // Older releases append a linker trampoline marker that is not part
        // of the mangled declaration.
        if let Some(rest) = body.strip_suffix(b"-trampoline") {
            body = rest;
        } else if let Some(rest) = strip_local_type_suffix(body) {
            body = rest;
        } else if let Some(rest) = body.strip_suffix(b"-impl") {
            is_impl = true;
            body = rest;
        } else {
            let (rest, found) = strip_internal_suffix(body);
            if found {
                internal = true;
                body = rest;
            } else {
                break;
            }
        }
    }
    (body, internal, is_impl)
}

/// Strip a trailing local declaration marker such as `-1:0`.
fn strip_local_type_suffix(body: &[u8]) -> Option<&[u8]> {
    let (head, tail) = rsplit_once(body, b"-")?;
    let (left, right) = split_once(tail, b":")?;
    if left.is_empty()
        || right.is_empty()
        || !left.iter().all(u8::is_ascii_digit)
        || !right.iter().all(u8::is_ascii_digit)
    {
        return None;
    }
    Some(head)
}

/// Strip one `#internal` or `#internal.<digits>` suffix.
fn strip_internal_suffix(body: &[u8]) -> (&[u8], bool) {
    if let Some(rest) = body.strip_suffix(b"#internal") {
        return (rest, true);
    }
    if let Some((rest, digits)) = rsplit_once(body, b"#internal.")
        && !digits.is_empty()
        && digits.iter().all(u8::is_ascii_digit)
    {
        return (rest, true);
    }
    (body, false)
}

/// A function body split into container path, member name and signature.
type FunctionHead<'a> = (&'a [u8], &'a [u8], &'a [u8]);

/// Split a function body into container path, member name and the rest.
///
/// Exported functions use `path#name`; private symbols use
/// `container.name` (`computePrivateSymbolName` prepends the container FQN).
/// The rest begins at the first marker or signature character.
fn split_function_head(body: &[u8], internal: bool) -> Result<FunctionHead<'_>, ParseError> {
    if let Some(hash) = find(body, b"#") {
        let after = &body[hash + 1..];
        if is_marker(after) {
            // Private symbols use `container.name#marker`. A marker without
            // the private marker is not a form the compiler emits.
            if !internal {
                return Err(ParseError::Invalid);
            }
            let head = &body[..hash];
            let (path, name) = split_last_dot(head)?;
            return Ok((path, name, &body[hash..]));
        }
        let name_end = find_name_end(after);
        let name = &after[..name_end];
        return Ok((&body[..hash], name, &after[name_end..]));
    }

    if !internal {
        return Err(ParseError::Invalid);
    }
    let name_end = find_name_end(body);
    let head = &body[..name_end];
    let (path, name) = split_last_dot(head)?;
    Ok((path, name, &body[name_end..]))
}

/// Find where a member name ends: at a marker, signature or extension marker.
fn find_name_end(text: &[u8]) -> usize {
    for (index, byte) in text.iter().enumerate() {
        if text[index..].starts_with(b"__at__") {
            return index;
        }
        if matches!(byte, b'#' | b'(' | b'{' | b'%' | b'@') {
            return index;
        }
    }
    text.len()
}

/// Split `container.member` at the last dot.
fn split_last_dot(text: &[u8]) -> Result<(&[u8], &[u8]), ParseError> {
    let Some(dot) = rfind(text, b".") else {
        return Err(ParseError::Invalid);
    };
    let (path, name) = (&text[..dot], &text[dot + 1..]);
    if path.is_empty() || name.is_empty() {
        return Err(ParseError::Invalid);
    }
    Ok((path, name))
}

/// Linkage and modifier markers that may follow a `#`.
const MARKERS: [&[u8]; 4] = [b"static", b"suspend", b"companion", b"internal"];

/// True when `text` begins with a known linkage or modifier marker.
fn is_marker(text: &[u8]) -> bool {
    MARKERS.iter().any(|marker| {
            text.strip_prefix(*marker).is_some_and(|rest| {
                rest.is_empty()
                    || rest.starts_with(b"(")
                    || rest.starts_with(b"{")
                    || rest.starts_with(b"%")
                    || rest.starts_with(b"@")
                    || rest.starts_with(b"#")
                    || rest.starts_with(b"-")
                    || rest.starts_with(b"__at__")
            })
        })
}

/// Parse a `ktype:` body.
fn parse_type_body<'a>(
    cursor: &mut Cursor<'a, '_>,
    body: &'a [u8],
    body_offset: usize,
) -> Result<Type<'a>, ParseError> {
    let (body, _) = strip_internal_marker(body);
    if body.is_empty() {
        return Err(ParseError::Invalid);
    }
    let _ = cursor.advance(body_offset).map_err(map_fail)?;
    let mut parser = Parser::new(cursor, body_offset + body.len());
    let ty = parser.parse_type()?;
    if !parser.is_end() {
        return Err(ParseError::Invalid);
    }
    Ok(ty)
}

/// Parse a path-only body (`kclass:`, `kfield:`, `kvar:`, …).
fn parse_named(body: &[u8], allow_member: bool) -> Result<&[u8], ParseError> {
    let (body, _) = strip_internal_marker(body);
    if body.is_empty() {
        return Err(ParseError::Invalid);
    }
    validate_path(body, allow_member)
}

/// Strip trailing `#internal` markers from a non-function body.
fn strip_internal_marker(mut body: &[u8]) -> (&[u8], bool) {
    let mut internal = false;
    loop {
        let (rest, found) = strip_internal_suffix(body);
        if found {
            internal = true;
            body = rest;
        } else {
            break;
        }
    }
    (body, internal)
}

/// Validate a qualified path.
fn validate_path(path: &[u8], allow_member: bool) -> Result<&[u8], ParseError> {
    if path.is_empty() {
        return Err(ParseError::Invalid);
    }
    let mut segment_len = 0_usize;
    let mut segments = 1_usize;
    let mut members = 0_usize;
    for byte in path {
        if *byte == b'.' || (allow_member && *byte == b'#') {
            if segment_len == 0 {
                return Err(ParseError::Invalid);
            }
            if *byte == b'#' {
                members += 1;
                if members > 1 {
                    return Err(ParseError::Invalid);
                }
            }
            segment_len = 0;
            segments += 1;
            if segments > MAX_PATH_SEGMENTS {
                return Err(ParseError::BudgetExceeded);
            }
        } else if is_path_byte(*byte) {
            segment_len += 1;
        } else {
            return Err(ParseError::Invalid);
        }
    }
    if segment_len == 0 {
        return Err(ParseError::Invalid);
    }
    Ok(path)
}

/// Validate a member name.
fn validate_name(name: &[u8]) -> Result<&[u8], ParseError> {
    if name.is_empty()
        || name.iter().any(|byte| {
            byte.is_ascii_control()
                || matches!(byte, b'#' | b'(' | b')' | b'{' | b'}' | b'%' | b'@' | b' ')
        })
    {
        return Err(ParseError::Invalid);
    }
    Ok(name)
}

/// Bytes allowed in a qualified path segment.
///
/// Bytes from `0x80` up are part of a UTF-8 identifier (or a broken one, which
/// prints escaped).
const fn is_path_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'$' | b'-' | b'<' | b'>') || byte >= 0x80
}

/// A bounded window over the shared cursor: one function signature or type
/// body.
///
/// The window can end before the symbol does, because trailing linkage markers
/// (`#internal`, `-impl`) are stripped before the signature is parsed.
struct Parser<'i, 's, 'c> {
    cursor: &'c mut Cursor<'i, 's>,
    end: usize,
}

impl<'i, 's, 'c> Parser<'i, 's, 'c> {
    const fn new(cursor: &'c mut Cursor<'i, 's>, end: usize) -> Self {
        Self { cursor, end }
    }

    fn remaining(&self) -> &'i [u8] {
        self.cursor
            .bytes_between(self.cursor.pos(), self.end)
            .unwrap_or(&[])
    }

    fn is_end(&self) -> bool {
        self.cursor.pos() >= self.end
    }

    fn peek(&self) -> Option<u8> {
        self.remaining().first().copied()
    }

    fn peek_is(&self, byte: u8) -> bool {
        self.peek() == Some(byte)
    }

    fn bump(&mut self) -> Result<(), ParseError> {
        let _ = self.cursor.advance(1).map_err(map_fail)?;
        Ok(())
    }

    fn skip_to_end(&mut self) -> Result<(), ParseError> {
        let remaining = self.end.saturating_sub(self.cursor.pos());
        let _ = self.cursor.advance(remaining).map_err(map_fail)?;
        Ok(())
    }

    fn eat_char(&mut self, byte: u8) -> Result<bool, ParseError> {
        if self.peek_is(byte) {
            self.bump()?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    fn expect_char(&mut self, byte: u8) -> Result<(), ParseError> {
        if self.eat_char(byte)? {
            Ok(())
        } else {
            Err(ParseError::Invalid)
        }
    }

    fn eat_str(&mut self, text: &[u8]) -> Result<bool, ParseError> {
        if self.remaining().starts_with(text) {
            let _ = self.cursor.advance(text.len()).map_err(map_fail)?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    fn expect_str(&mut self, text: &[u8]) -> Result<(), ParseError> {
        if self.eat_str(text)? {
            Ok(())
        } else {
            Err(ParseError::Invalid)
        }
    }

    /// Eat a `#` marker only when it is followed by a marker or signature
    /// delimiter, so `#staticx` is rejected rather than read as `#static`.
    fn eat_marker(&mut self, marker: &[u8]) -> Result<bool, ParseError> {
        if let Some(after) = self.remaining().strip_prefix(marker)
            && (after.is_empty()
                || after.starts_with(b"(")
                || after.starts_with(b"{")
                || after.starts_with(b"%")
                || after.starts_with(b"@")
                || after.starts_with(b"#")
                || after.starts_with(b"-")
                || after.starts_with(b"__at__"))
        {
            let _ = self.cursor.advance(marker.len()).map_err(map_fail)?;
            return Ok(true);
        }
        Ok(false)
    }

    fn eat_at_marker(&mut self) -> Result<bool, ParseError> {
        if self.eat_char(b'@')? {
            Ok(true)
        } else {
            self.eat_str(b"__at__")
        }
    }

    /// Parse a qualified path (used for companion-extension class names).
    fn parse_path(&mut self, allow_member: bool) -> Result<&'i [u8], ParseError> {
        let start = self.cursor.pos();
        let mut segment_len = 0_usize;
        let mut segments = 1_usize;
        let mut members = 0_usize;
        while let Some(byte) = self.peek() {
            if byte == b'.' || (allow_member && byte == b'#') {
                if segment_len == 0 {
                    return Err(ParseError::Invalid);
                }
                if byte == b'#' {
                    members += 1;
                    if members > 1 {
                        return Err(ParseError::Invalid);
                    }
                }
                segment_len = 0;
                segments += 1;
                if segments > MAX_PATH_SEGMENTS {
                    return Err(ParseError::BudgetExceeded);
                }
                self.bump()?;
            } else if is_path_byte(byte) {
                segment_len += 1;
                self.bump()?;
            } else {
                break;
            }
        }
        if segment_len == 0 {
            return Err(ParseError::Invalid);
        }
        let path = self
            .cursor
            .bytes_between(start, self.cursor.pos())
            .ok_or(ParseError::Invalid)?;
        Ok(path)
    }

    /// Parse one value-parameter list, after the opening `(`.
    fn parse_parameters(&mut self) -> Result<Vec<Type<'i>>, ParseError> {
        let mut params = Vec::new();
        if self.eat_char(b')')? {
            return Ok(params);
        }
        loop {
            if params.len() >= MAX_PARAMETERS {
                return Err(ParseError::BudgetExceeded);
            }
            params.push(self.parse_type()?);
            if self.eat_char(b';')? {
                if self.peek_is(b')') {
                    return Err(ParseError::Invalid);
                }
            } else if self.eat_char(b')')? {
                break;
            } else {
                return Err(ParseError::Invalid);
            }
        }
        Ok(params)
    }

    /// Parse a type-parameter list, after the opening `{`.
    fn parse_type_parameters(&mut self) -> Result<Vec<TypeParameter<'i>>, ParseError> {
        let mut params = Vec::new();
        if self.eat_char(b'}')? {
            return Ok(params);
        }
        loop {
            if params.len() >= MAX_TYPE_PARAMETERS {
                return Err(ParseError::BudgetExceeded);
            }
            let index = self.parse_u32()?;
            self.expect_str(SECTION_SIGN)?;
            self.expect_char(b'<')?;
            let mut bounds = Vec::new();
            if !self.eat_char(b'>')? {
                loop {
                    bounds.push(self.parse_type()?);
                    if self.eat_char(b'&')? {
                        continue;
                    }
                    self.expect_char(b'>')?;
                    break;
                }
            }
            params.push(TypeParameter { index, bounds });
            if self.eat_char(b';')? {
                continue;
            }
            self.expect_char(b'}')?;
            break;
        }
        Ok(params)
    }

    /// Parse one type.
    fn parse_type(&mut self) -> Result<Type<'i>, ParseError> {
        self.cursor.enter().map_err(map_fail)?;
        let result = self.parse_type_inner();
        self.cursor.leave();
        result
    }

    /// Parse a type without changing the recursion counter.
    fn parse_type_inner(&mut self) -> Result<Type<'i>, ParseError> {
        if self.eat_str(b"<dynamic>")? {
            return Ok(Type::Dynamic);
        }
        if self.eat_str(b"<ERROR CLASS>")? {
            return Ok(Type::Error);
        }
        if self.eat_char(b'[')? {
            let lower = self.parse_type()?;
            self.expect_char(b'~')?;
            let upper = self.parse_type()?;
            self.expect_char(b']')?;
            return Ok(Type::Flexible {
                lower: Box::new(lower),
                upper: Box::new(upper),
            });
        }
        if let Some(reference) = self.try_type_parameter_reference()? {
            return self.finish_type(reference);
        }

        let name = self.parse_type_name()?;
        let arguments = if self.eat_char(b'<')? {
            self.parse_type_arguments()?
        } else {
            Vec::new()
        };
        let nullable = self.eat_char(b'?')?;
        self.finish_type(Type::Class {
            name,
            arguments,
            nullable,
        })
    }

    /// Attach a trailing `...` vararg marker, if present.
    fn finish_type(&mut self, ty: Type<'i>) -> Result<Type<'i>, ParseError> {
        if self.eat_str(b"...")? {
            Ok(Type::Vararg(Box::new(ty)))
        } else {
            Ok(ty)
        }
    }

    /// Parse a type-parameter reference (`0:0`) when the input has that shape.
    fn try_type_parameter_reference(&mut self) -> Result<Option<Type<'i>>, ParseError> {
        let rest = self.remaining();
        let digits = rest
            .iter()
            .take_while(|byte| byte.is_ascii_digit())
            .count();
        if digits == 0 || rest.get(digits) != Some(&b':') {
            return Ok(None);
        }
        let container = parse_u32_digits(&rest[..digits])?;
        let _ = self.cursor.advance(digits + 1).map_err(map_fail)?;
        let index = self.parse_u32()?;
        let nullable = self.eat_char(b'?')?;
        Ok(Some(Type::TypeParameter {
            container,
            index,
            nullable,
        }))
    }

    /// Parse a qualified type name.
    fn parse_type_name(&mut self) -> Result<&'i [u8], ParseError> {
        let start = self.cursor.pos();
        while let Some(byte) = self.peek() {
            if self.remaining().starts_with(b"...") {
                break;
            }
            if is_type_name_stop(byte) {
                break;
            }
            self.bump()?;
        }
        let name = self
            .cursor
            .bytes_between(start, self.cursor.pos())
            .ok_or(ParseError::Invalid)?;
        validate_path(name, false)
    }

    /// Parse a type-argument list, after the opening `<`.
    fn parse_type_arguments(&mut self) -> Result<Vec<TypeArgument<'i>>, ParseError> {
        let mut arguments = Vec::new();
        if self.eat_char(b'>')? {
            return Err(ParseError::Invalid);
        }
        loop {
            if arguments.len() >= MAX_TYPE_ARGUMENTS {
                return Err(ParseError::BudgetExceeded);
            }
            if self.eat_char(b'*')? {
                arguments.push(TypeArgument::Star);
            } else {
                let variance = if self.eat_str(b"out|")? {
                    Variance::Out
                } else if self.eat_str(b"in|")? {
                    Variance::In
                } else {
                    if is_bare_variance(self.remaining()) {
                        return Err(ParseError::Invalid);
                    }
                    Variance::Invariant
                };
                arguments.push(TypeArgument::Type {
                    variance,
                    ty: self.parse_type()?,
                });
            }
            if self.eat_char(b',')? {
                continue;
            }
            self.expect_char(b'>')?;
            break;
        }
        Ok(arguments)
    }

    /// Parse an unsigned decimal integer.
    fn parse_u32(&mut self) -> Result<u32, ParseError> {
        let start = self.cursor.pos();
        while self.peek().is_some_and(|byte| byte.is_ascii_digit()) {
            self.bump()?;
        }
        if self.cursor.pos() == start {
            return Err(ParseError::Invalid);
        }
        let digits = self
            .cursor
            .bytes_between(start, self.cursor.pos())
            .ok_or(ParseError::Invalid)?;
        parse_u32_digits(digits)
    }
}

/// Bytes that end a qualified type name.
const fn is_type_name_stop(byte: u8) -> bool {
    byte.is_ascii_control()
        || matches!(
            byte,
            b'<' | b'>'
                | b'?'
                | b','
                | b';'
                | b')'
                | b'}'
                | b']'
                | b'['
                | b'#'
                | b'('
                | b'{'
                | b'%'
                | b'|'
                | b'&'
                | b'@'
                | b'*'
                | b':'
                | b'~'
                | b' '
        )
}

/// True when a type argument is a bare `out`/`in` keyword without `|`.
fn is_bare_variance(text: &[u8]) -> bool {
    [b"out".as_slice(), b"in"].iter().any(|variance| {
        text.strip_prefix(*variance).is_some_and(|rest| {
            rest.is_empty()
                || rest.starts_with(b">")
                || rest.starts_with(b",")
                || rest.starts_with(b";")
                || rest.starts_with(b")")
                || rest.starts_with(b"}")
                || rest.starts_with(b"&")
        })
    })
}

/// Bytes that are structural markers, not part of a `%` platform suffix.
const fn is_platform_suffix_stop(byte: u8) -> bool {
    byte.is_ascii_control()
        || matches!(
            byte,
            b'#' | b'('
                | b')'
                | b'{'
                | b'}'
                | b'%'
                | b';'
                | b','
                | b'<'
                | b'>'
                | b'|'
                | b'*'
                | b'?'
                | b'['
                | b']'
                | b'~'
                | b' '
        )
}

/// Parse a decimal `u32`, rejecting overflow.
fn parse_u32_digits(digits: &[u8]) -> Result<u32, ParseError> {
    if digits.is_empty() {
        return Err(ParseError::Invalid);
    }
    let mut value = 0_u32;
    for byte in digits {
        if !byte.is_ascii_digit() {
            return Err(ParseError::Invalid);
        }
        value = value
            .checked_mul(10)
            .and_then(|value| value.checked_add(u32::from(byte - b'0')))
            .ok_or(ParseError::BudgetExceeded)?;
    }
    Ok(value)
}

/// Byte offset of the first occurrence of `needle` in `haystack`.
fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || needle.len() > haystack.len() {
        return None;
    }
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

/// Byte offset of the last occurrence of `needle` in `haystack`.
fn rfind(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || needle.len() > haystack.len() {
        return None;
    }
    haystack
        .windows(needle.len())
        .rposition(|window| window == needle)
}

/// Split `haystack` at the first occurrence of `needle`.
fn split_once<'a>(haystack: &'a [u8], needle: &[u8]) -> Option<(&'a [u8], &'a [u8])> {
    let index = find(haystack, needle)?;
    Some((&haystack[..index], &haystack[index + needle.len()..]))
}

/// Split `haystack` at the last occurrence of `needle`.
fn rsplit_once<'a>(haystack: &'a [u8], needle: &[u8]) -> Option<(&'a [u8], &'a [u8])> {
    let index = rfind(haystack, needle)?;
    Some((&haystack[..index], &haystack[index + needle.len()..]))
}

#[cfg(test)]
mod tests {
    use alloc::string::ToString;

    use super::*;
    use crate::kotlin_native::parse;

    #[test]
    fn parses_function_examples() {
        for (symbol, expected) in [
            (
                "kfun:com.example.foo#bar(kotlin.Int){}",
                "com.example.foo.bar(kotlin.Int)",
            ),
            (
                "kfun:com.example.Box#<init>(kotlin.Int){}",
                "com.example.Box.<init>(kotlin.Int)",
            ),
            ("kfun:com.example.Box#get(){}", "com.example.Box.get()"),
            ("kfun:com.example#topLevel(){}", "com.example.topLevel()"),
        ] {
            assert_eq!(parse(symbol).unwrap().to_string(), expected, "{symbol}");
        }
    }

    #[test]
    fn parses_private_dot_separator() {
        assert_eq!(
            parse("kfun:com.example#outer$local#internal")
                .unwrap()
                .to_string(),
            "com.example.outer$local"
        );
        assert_eq!(
            parse("kfun:com.example.useAll$lambda$1.invoke#internal")
                .unwrap()
                .to_string(),
            "com.example.useAll$lambda$1.invoke"
        );
        assert_eq!(
            parse("kfun:com.example.useAll$lambda$1.$<bridge-UNU>invoke(kotlin.Int){}kotlin.Int#internal")
                .unwrap()
                .to_string(),
            "com.example.useAll$lambda$1.$<bridge-UNU>invoke(kotlin.Int): kotlin.Int"
        );
    }

    #[test]
    fn parses_extension_receiver() {
        let ast =
            parse("kfun:com.example.kdemo#shout__at__kotlin.String(){}kotlin.String").unwrap();
        assert_eq!(
            ast.to_string(),
            "com.example.kdemo.shout@kotlin.String(): kotlin.String"
        );
    }

    #[test]
    fn parses_static_and_suspend_markers() {
        let ast =
            parse("kfun:com.example.Singleton#<get-$instance>#static(){}com.example.Singleton")
                .unwrap();
        assert_eq!(
            ast.to_string(),
            "static com.example.Singleton.<get-$instance>(): com.example.Singleton"
        );
        let ast = parse(
            "kfun:com.example#suspendWork#suspend(kotlin.String;kotlin.coroutines.Continuation<kotlin.String>){}kotlin.Any",
        )
        .unwrap();
        assert_eq!(
            ast.to_string(),
            "suspend com.example.suspendWork(kotlin.String, kotlin.coroutines.Continuation<kotlin.String>): kotlin.Any"
        );
    }

    #[test]
    fn parses_type_parameters_and_bounds() {
        let ast = parse("kfun:com.example#maxOf(0:0;0:0){0§<kotlin.Comparable<0:0>>}0:0").unwrap();
        assert_eq!(
            ast.to_string(),
            "com.example.maxOf(T0:0, T0:0)<T0: kotlin.Comparable<T0:0>>: T0:0"
        );
    }

    #[test]
    fn parses_variance_nullable_star_vararg() {
        let ast = parse(
            "kfun:com.example#projections(kotlin.collections.List<out|kotlin.Number>;kotlin.collections.MutableList<in|kotlin.Number>){}kotlin.Int",
        )
        .unwrap();
        assert_eq!(
            ast.to_string(),
            "com.example.projections(kotlin.collections.List<out kotlin.Number>, kotlin.collections.MutableList<in kotlin.Number>): kotlin.Int"
        );
        assert_eq!(
            parse("kfun:com.example#stars(kotlin.collections.List<*>){}kotlin.Int")
                .unwrap()
                .to_string(),
            "com.example.stars(kotlin.collections.List<*>): kotlin.Int"
        );
        assert_eq!(
            parse("kfun:com.example#varargs(kotlin.IntArray...){}kotlin.Int")
                .unwrap()
                .to_string(),
            "com.example.varargs(vararg kotlin.IntArray): kotlin.Int"
        );
        assert_eq!(
            parse("kfun:com.example#nullable(kotlin.String?){}kotlin.String?")
                .unwrap()
                .to_string(),
            "com.example.nullable(kotlin.String?): kotlin.String?"
        );
        // A class name starting with `in` is not a bare `in` variance marker.
        assert_eq!(
            parse("ktype:kotlin.Array<input>").unwrap().to_string(),
            "kotlin.Array<input>"
        );
    }

    #[test]
    fn parses_named_prefixes() {
        for (symbol, expected) in [
            (
                "ktype:com.example.Box<kotlin.Int>",
                "com.example.Box<kotlin.Int>",
            ),
            ("kclass:com.example.Box", "com.example.Box"),
            ("kclass:com.example.Box#internal", "com.example.Box"),
            ("kproperty:com.example.Box#value", "com.example.Box.value"),
            ("kobj:com.example.Singleton", "com.example.Singleton"),
            ("kfield:com.example.Box.value", "com.example.Box.value"),
            ("ktypealias:com.example.IntList", "com.example.IntList"),
            ("kpackage:com.example", "com.example"),
            ("knamespace:com.example", "com.example"),
            (
                "kvar:com.example.Singleton.$instance",
                "com.example.Singleton.$instance",
            ),
        ] {
            assert_eq!(parse(symbol).unwrap().to_string(), expected, "{symbol}");
        }
    }

    #[test]
    fn accepts_platform_underscores() {
        for symbol in [
            "_kfun:com.example#main(){}",
            "__kfun:com.example#main(){}",
            "_kclass:com.example.Box",
        ] {
            assert!(is_kotlin_native(symbol), "{symbol}");
            assert!(parse(symbol).is_ok(), "{symbol}");
        }
    }

    #[test]
    fn accepts_platform_suffix() {
        let ast = parse("kfun:com.example#foo(){}%objc").unwrap();
        assert_eq!(ast.to_string(), "com.example.foo() %objc");
    }

    #[test]
    fn rejects_unknown_markers() {
        for symbol in [
            "kfun:com.example#foo#nope(){}",
            "kfun:com.example#foo(kotlin.Int){}#nope",
            "kfun:com.example#foo#staticx(){}",
            "kfun:com.example",
            "kfun:com.example.foo",
            "kfun:com.example.foo#static(){}",
        ] {
            assert!(parse(symbol).is_err(), "{symbol}");
        }
    }

    #[test]
    fn rejects_malformed_bodies() {
        for symbol in [
            "kfun:",
            "kfun:#foo(){}",
            "kfun:com.example#",
            "kfun:com.example#foo(",
            "kfun:com.example#foo(kotlin.Int",
            "kfun:com.example#foo(){}kotlin.Int<",
            "kfun:com.example#foo(kotlin.Int;",
            "kfun:com.example#foo(kotlin.Int;){}",
            "kfun:com.example#foo{0§<kotlin.Int>",
            "kfun:com.example#foo{0§<kotlin.Int}",
            "kfun:com.example#foo(){}kotlin.Array<",
            "kfun:com.example#foo(){}kotlin.Array<>",
            "kfun:com.example#foo(){}kotlin.Array<out|>",
            "kfun:com.example#foo(){}kotlin.Array<out>",
            "kfun:com.example#foo(){}kotlin.Array<in>",
            "ktype:",
            "kclass:",
            "kclass:com.example.Box#",
            "kproperty:com.example#a#b",
            "kfield:com.example.",
            "kpackage:.com",
        ] {
            assert!(parse(symbol).is_err(), "{symbol}");
        }
    }

    #[test]
    fn rejects_unknown_prefixes() {
        for symbol in ["kfoo:bar", "kintf:com.example", "kifacetable:x", "kclass"] {
            assert!(!is_kotlin_native(symbol), "{symbol}");
        }
    }

    #[test]
    fn input_is_bounded() {
        let mut symbol = alloc::string::String::from("kfun:");
        symbol.push_str(&"a".repeat(MAX_INPUT_LEN));
        assert!(matches!(parse(&symbol), Err(ParseError::BudgetExceeded)));
    }

    #[test]
    fn depth_is_bounded() {
        let mut symbol = alloc::string::String::from("ktype:kotlin.Array<");
        symbol.push_str(&"kotlin.Array<".repeat(MAX_DEPTH + 1));
        symbol.push('*');
        symbol.push_str(&">".repeat(MAX_DEPTH + 1));
        assert!(matches!(
            parse(&symbol),
            Err(ParseError::RecursedTooDeep | ParseError::BudgetExceeded)
        ));
    }

    #[test]
    fn byte_path_matches_the_text_path() {
        for symbol in [
            "kfun:com.example.foo#bar(kotlin.Int){}",
            "kfun:com.example#shout__at__kotlin.String(){}kotlin.String",
            "ktype:kotlin.collections.Map<kotlin.String,kotlin.Array<out|kotlin.Number?>>",
            "kclass:com.example.Box#internal",
            "kproperty:com.example.Box#value",
            "kfun:com.example#foo(){}%objc",
            "_kfun:com.example#main(){}",
        ] {
            assert_eq!(
                parse_bytes(symbol.as_bytes()).unwrap().to_string(),
                parse(symbol).unwrap().to_string(),
                "{symbol}"
            );
        }
    }

    #[test]
    fn byte_path_replays_the_corpus() {
        for path in [
            "refs/kotlin-native/corpus/kotlin-2.4.21-linux-x86_64.txt",
            "refs/kotlin-native/corpus/hand-authored.txt",
        ] {
            let path = std::format!("{}/{path}", std::env!("CARGO_MANIFEST_DIR"));
            let Ok(data) = std::fs::read_to_string(&path) else {
                continue;
            };
            for line in data.lines() {
                let symbol = line.trim_end();
                if symbol.is_empty() {
                    continue;
                }
                let Ok(text) = parse(symbol) else {
                    continue;
                };
                let bytes = parse_bytes(symbol.as_bytes()).unwrap_or_else(|error| {
                    panic!("{symbol}: byte path failed: {error}");
                });
                assert_eq!(bytes.to_string(), text.to_string(), "{symbol}");
            }
        }
    }

    #[test]
    fn invalid_utf8_text_is_escaped() {
        assert_eq!(
            parse_bytes(b"kclass:com.example.F\xffoo").unwrap().to_string(),
            "com.example.F\\xffoo"
        );
        assert_eq!(
            parse_bytes(b"kfun:com.example#f\xffoo(){}").unwrap().to_string(),
            "com.example.f\\xffoo()"
        );
    }

    #[test]
    fn valid_unicode_text_stays_verbatim() {
        assert_eq!(
            parse_bytes("kclass:com.example.Föö".as_bytes())
                .unwrap()
                .to_string(),
            "com.example.Föö"
        );
    }

    #[test]
    fn truncated_names_never_panic() {
        for symbol in [
            "kfun:com.example#foo(kotlin.Int;kotlin.String){0§<kotlin.Any>}kotlin.Int",
            "kfun:com.example#shout__at__kotlin.String(){}kotlin.String",
            "ktype:kotlin.collections.Map<kotlin.String,kotlin.Array<out|kotlin.Number?>>",
            "kclass:com.example.Box#internal",
            "kproperty:com.example.Box#value",
        ] {
            for end in 0..=symbol.len() {
                if let Some(prefix) = symbol.get(..end) {
                    let _ = parse(prefix);
                    let _ = is_kotlin_native(prefix);
                }
            }
        }
    }
}
