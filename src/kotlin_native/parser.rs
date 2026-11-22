//! Bounded parser for Kotlin/Native linker symbols.
//!
//! The grammar is the compiler mangler's own output, documented in
//! `refs/kotlin-native.md`: a `k*:` prefix, a qualified path, a `#` member
//! separator (or `.` for private symbols), a value-parameter list, a
//! type-parameter list, a return type and linkage markers. Unknown markers are
//! rejected rather than guessed.

use alloc::boxed::Box;
use alloc::vec::Vec;

use super::ParseError;
use super::node::{
    Ast, Function, MAX_DEPTH, MAX_INPUT_LEN, MAX_PARAMETERS, MAX_PATH_SEGMENTS, MAX_TYPE_ARGUMENTS,
    MAX_TYPE_PARAMETERS, MAX_WORK, Symbol, Type, TypeArgument, TypeParameter, Variance,
};

/// The known Kotlin/Native symbol prefixes.
const PREFIXES: [(&str, Prefix); 10] = [
    ("kfun:", Prefix::Function),
    ("ktype:", Prefix::Type),
    ("kclass:", Prefix::Class),
    ("kproperty:", Prefix::Property),
    ("kobj:", Prefix::Object),
    ("kfield:", Prefix::Field),
    ("ktypealias:", Prefix::TypeAlias),
    ("kpackage:", Prefix::Package),
    ("knamespace:", Prefix::Namespace),
    ("kvar:", Prefix::Variable),
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

/// Parse a Kotlin/Native symbol name.
pub(crate) fn parse(symbol: &str) -> Result<Ast<'_>, ParseError> {
    if symbol.is_empty() {
        return Err(ParseError::Invalid);
    }
    if symbol.len() > MAX_INPUT_LEN {
        return Err(ParseError::BudgetExceeded);
    }
    let symbol = strip_platform_underscores(symbol);
    let (prefix, body) = split_prefix(symbol).ok_or(ParseError::Invalid)?;
    let mut budget = Budget::new();
    budget.spend(symbol.len())?;

    let symbol = match prefix {
        Prefix::Function => Symbol::Function(parse_function(body, &mut budget)?),
        Prefix::Type => Symbol::Type(parse_type_body(body, &mut budget)?),
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

/// True when `name` starts with a known Kotlin/Native prefix.
///
/// This is the cheap prefilter used by [`crate::Scheme::detect`]. It accepts
/// the Mach-O leading-underscore variants (`_kfun:`, `__kfun:`) and claims a
/// known prefix even when the body is malformed, so
/// [`crate::kotlin_native::parse`] can reject it.
/// It never claims an arbitrary `k...` name.
#[must_use]
pub fn is_kotlin_native(name: &str) -> bool {
    if name.is_empty() || name.len() > MAX_INPUT_LEN {
        return false;
    }
    split_prefix(strip_platform_underscores(name)).is_some()
}

/// Strip one or two leading platform underscores from a Mach-O symbol.
fn strip_platform_underscores(name: &str) -> &str {
    name.strip_prefix("__")
        .or_else(|| name.strip_prefix('_'))
        .unwrap_or(name)
}

/// Split a known prefix from the body.
fn split_prefix(name: &str) -> Option<(Prefix, &str)> {
    PREFIXES
        .iter()
        .find_map(|(prefix, kind)| name.strip_prefix(prefix).map(|body| (*kind, body)))
}

/// A bounded work counter: one unit per byte scanned.
struct Budget {
    remaining: usize,
}

impl Budget {
    const fn new() -> Self {
        Self {
            remaining: MAX_WORK,
        }
    }

    fn spend(&mut self, units: usize) -> Result<(), ParseError> {
        self.remaining = self
            .remaining
            .checked_sub(units)
            .ok_or(ParseError::BudgetExceeded)?;
        Ok(())
    }
}

/// Parse a `kfun:` function.
fn parse_function<'a>(body: &'a str, budget: &mut Budget) -> Result<Function<'a>, ParseError> {
    let (body, internal, is_impl) = strip_function_markers(body);
    if body.is_empty() {
        return Err(ParseError::Invalid);
    }
    let (path, name, rest) = split_function_head(body, internal)?;
    validate_path(path, false)?;
    validate_name(name)?;

    let mut parser = Parser::new(rest, budget);
    let mut is_static = false;
    let mut is_suspend = false;
    let mut companion = None;

    loop {
        if parser.eat_marker("#static")? {
            is_static = true;
        } else if parser.eat_marker("#suspend")? {
            is_suspend = true;
        } else if parser.eat_marker("#companion")? {
            if !parser.eat_at_marker()? {
                return Err(ParseError::Invalid);
            }
            companion = Some(parser.parse_path(false)?);
        } else if parser.eat_marker("#internal")? {
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
        if suffix.is_empty() || suffix.bytes().any(is_platform_suffix_stop) {
            return Err(ParseError::Invalid);
        }
        parser.skip_to_end();
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
fn strip_function_markers(mut body: &str) -> (&str, bool, bool) {
    let mut internal = false;
    let mut is_impl = false;
    loop {
        if let Some(rest) = body.strip_suffix("-impl") {
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

/// Strip one `#internal` or `#internal.<digits>` suffix.
fn strip_internal_suffix(body: &str) -> (&str, bool) {
    if let Some(rest) = body.strip_suffix("#internal") {
        return (rest, true);
    }
    if let Some((rest, digits)) = body.rsplit_once("#internal.")
        && !digits.is_empty()
        && digits.bytes().all(|byte| byte.is_ascii_digit())
    {
        return (rest, true);
    }
    (body, false)
}

/// Split a function body into container path, member name and the rest.
///
/// Exported functions use `path#name`; private symbols use
/// `container.name` (`computePrivateSymbolName` prepends the container FQN).
/// The rest begins at the first marker or signature character.
fn split_function_head(body: &str, internal: bool) -> Result<(&str, &str, &str), ParseError> {
    if let Some(hash) = body.find('#') {
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
fn find_name_end(text: &str) -> usize {
    for (index, ch) in text.char_indices() {
        if text[index..].starts_with("__at__") {
            return index;
        }
        if matches!(ch, '#' | '(' | '{' | '%' | '@') {
            return index;
        }
    }
    text.len()
}

/// Split `container.member` at the last dot.
fn split_last_dot(text: &str) -> Result<(&str, &str), ParseError> {
    let Some(dot) = text.rfind('.') else {
        return Err(ParseError::Invalid);
    };
    let (path, name) = (&text[..dot], &text[dot + 1..]);
    if path.is_empty() || name.is_empty() {
        return Err(ParseError::Invalid);
    }
    Ok((path, name))
}

/// True when `text` begins with a known linkage or modifier marker.
fn is_marker(text: &str) -> bool {
    ["static", "suspend", "companion", "internal"]
        .iter()
        .any(|marker| {
            text.strip_prefix(marker).is_some_and(|rest| {
                rest.is_empty()
                    || rest.starts_with('(')
                    || rest.starts_with('{')
                    || rest.starts_with('%')
                    || rest.starts_with('@')
                    || rest.starts_with('#')
                    || rest.starts_with('-')
                    || rest.starts_with("__at__")
            })
        })
}

/// Parse a `ktype:` body.
fn parse_type_body<'a>(body: &'a str, budget: &mut Budget) -> Result<Type<'a>, ParseError> {
    let (body, _) = strip_internal_marker(body);
    if body.is_empty() {
        return Err(ParseError::Invalid);
    }
    let mut parser = Parser::new(body, budget);
    let ty = parser.parse_type()?;
    if !parser.is_end() {
        return Err(ParseError::Invalid);
    }
    Ok(ty)
}

/// Parse a path-only body (`kclass:`, `kfield:`, `kvar:`, …).
fn parse_named(body: &str, allow_member: bool) -> Result<&str, ParseError> {
    let (body, _) = strip_internal_marker(body);
    if body.is_empty() {
        return Err(ParseError::Invalid);
    }
    validate_path(body, allow_member)?;
    Ok(body)
}

/// Strip trailing `#internal` markers from a non-function body.
fn strip_internal_marker(mut body: &str) -> (&str, bool) {
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
fn validate_path(path: &str, allow_member: bool) -> Result<(), ParseError> {
    if path.is_empty() {
        return Err(ParseError::Invalid);
    }
    let mut segment_len = 0_usize;
    let mut segments = 1_usize;
    let mut members = 0_usize;
    for byte in path.bytes() {
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
        } else if is_path_byte(byte) {
            segment_len += 1;
        } else {
            return Err(ParseError::Invalid);
        }
    }
    if segment_len == 0 {
        return Err(ParseError::Invalid);
    }
    Ok(())
}

/// Validate a member name.
fn validate_name(name: &str) -> Result<(), ParseError> {
    if name.is_empty()
        || name.bytes().any(|byte| {
            byte.is_ascii_control()
                || matches!(byte, b'#' | b'(' | b')' | b'{' | b'}' | b'%' | b'@' | b' ')
        })
    {
        return Err(ParseError::Invalid);
    }
    Ok(())
}

/// Bytes allowed in a qualified path segment.
const fn is_path_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'$' | b'-' | b'<' | b'>') || byte >= 0x80
}

/// A bounded cursor over a function signature.
struct Parser<'a, 'b> {
    input: &'a str,
    pos: usize,
    budget: &'b mut Budget,
    depth: usize,
}

impl<'a, 'b> Parser<'a, 'b> {
    const fn new(input: &'a str, budget: &'b mut Budget) -> Self {
        Self {
            input,
            pos: 0,
            budget,
            depth: 0,
        }
    }

    fn remaining(&self) -> &'a str {
        &self.input[self.pos..]
    }

    const fn is_end(&self) -> bool {
        self.pos >= self.input.len()
    }

    fn peek(&self) -> Option<u8> {
        self.input.as_bytes().get(self.pos).copied()
    }

    fn peek_is(&self, byte: u8) -> bool {
        matches!(self.input.as_bytes().get(self.pos), Some(found) if *found == byte)
    }

    fn bump(&mut self) -> Result<(), ParseError> {
        let Some(ch) = self.input[self.pos..].chars().next() else {
            return Err(ParseError::Invalid);
        };
        self.pos += ch.len_utf8();
        self.budget.spend(ch.len_utf8())
    }

    fn skip_to_end(&mut self) {
        self.pos = self.input.len();
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

    fn eat_str(&mut self, text: &str) -> Result<bool, ParseError> {
        if self.remaining().starts_with(text) {
            self.pos += text.len();
            self.budget.spend(text.len())?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    fn expect_str(&mut self, text: &str) -> Result<(), ParseError> {
        if self.eat_str(text)? {
            Ok(())
        } else {
            Err(ParseError::Invalid)
        }
    }

    /// Eat a `#` marker only when it is followed by a marker or signature
    /// delimiter, so `#staticx` is rejected rather than read as `#static`.
    fn eat_marker(&mut self, marker: &str) -> Result<bool, ParseError> {
        if let Some(after) = self.remaining().strip_prefix(marker)
            && (after.is_empty()
                || after.starts_with('(')
                || after.starts_with('{')
                || after.starts_with('%')
                || after.starts_with('@')
                || after.starts_with('#')
                || after.starts_with('-')
                || after.starts_with("__at__"))
        {
            self.pos += marker.len();
            self.budget.spend(marker.len())?;
            return Ok(true);
        }
        Ok(false)
    }

    fn eat_at_marker(&mut self) -> Result<bool, ParseError> {
        if self.eat_char(b'@')? {
            Ok(true)
        } else {
            self.eat_str("__at__")
        }
    }

    /// Parse a qualified path (used for companion-extension class names).
    fn parse_path(&mut self, allow_member: bool) -> Result<&'a str, ParseError> {
        let start = self.pos;
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
        Ok(&self.input[start..self.pos])
    }

    /// Parse one value-parameter list, after the opening `(`.
    fn parse_parameters(&mut self) -> Result<Vec<Type<'a>>, ParseError> {
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
    fn parse_type_parameters(&mut self) -> Result<Vec<TypeParameter<'a>>, ParseError> {
        let mut params = Vec::new();
        if self.eat_char(b'}')? {
            return Ok(params);
        }
        loop {
            if params.len() >= MAX_TYPE_PARAMETERS {
                return Err(ParseError::BudgetExceeded);
            }
            let index = self.parse_u32()?;
            self.expect_str("§")?;
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
    fn parse_type(&mut self) -> Result<Type<'a>, ParseError> {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            self.depth -= 1;
            return Err(ParseError::RecursedTooDeep);
        }
        let result = self.parse_type_inner();
        self.depth -= 1;
        result
    }

    /// Parse a type without changing the recursion counter.
    fn parse_type_inner(&mut self) -> Result<Type<'a>, ParseError> {
        if self.eat_str("<dynamic>")? {
            return Ok(Type::Dynamic);
        }
        if self.eat_str("<ERROR CLASS>")? {
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
    fn finish_type(&mut self, ty: Type<'a>) -> Result<Type<'a>, ParseError> {
        if self.eat_str("...")? {
            Ok(Type::Vararg(Box::new(ty)))
        } else {
            Ok(ty)
        }
    }

    /// Parse a type-parameter reference (`0:0`) when the input has that shape.
    fn try_type_parameter_reference(&mut self) -> Result<Option<Type<'a>>, ParseError> {
        let rest = self.remaining();
        let digits = rest
            .bytes()
            .take_while(|byte| byte.is_ascii_digit())
            .count();
        if digits == 0 || rest.as_bytes().get(digits) != Some(&b':') {
            return Ok(None);
        }
        let container = parse_u32_digits(&rest[..digits])?;
        self.pos += digits + 1;
        self.budget.spend(digits + 1)?;
        let index = self.parse_u32()?;
        let nullable = self.eat_char(b'?')?;
        Ok(Some(Type::TypeParameter {
            container,
            index,
            nullable,
        }))
    }

    /// Parse a qualified type name.
    fn parse_type_name(&mut self) -> Result<&'a str, ParseError> {
        let start = self.pos;
        while let Some(byte) = self.peek() {
            if self.remaining().starts_with("...") {
                break;
            }
            if is_type_name_stop(byte) {
                break;
            }
            self.bump()?;
        }
        let name = &self.input[start..self.pos];
        validate_path(name, false)?;
        Ok(name)
    }

    /// Parse a type-argument list, after the opening `<`.
    fn parse_type_arguments(&mut self) -> Result<Vec<TypeArgument<'a>>, ParseError> {
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
                let variance = if self.eat_str("out|")? {
                    Variance::Out
                } else if self.eat_str("in|")? {
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
        let start = self.pos;
        while self.peek().is_some_and(|byte| byte.is_ascii_digit()) {
            self.bump()?;
        }
        if self.pos == start {
            return Err(ParseError::Invalid);
        }
        parse_u32_digits(&self.input[start..self.pos])
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
fn is_bare_variance(text: &str) -> bool {
    ["out", "in"].iter().any(|variance| {
        text.strip_prefix(variance).is_some_and(|rest| {
            rest.is_empty()
                || rest.starts_with('>')
                || rest.starts_with(',')
                || rest.starts_with(';')
                || rest.starts_with(')')
                || rest.starts_with('}')
                || rest.starts_with('&')
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
fn parse_u32_digits(digits: &str) -> Result<u32, ParseError> {
    if digits.is_empty() {
        return Err(ParseError::Invalid);
    }
    let mut value = 0_u32;
    for byte in digits.bytes() {
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

#[cfg(test)]
mod tests {
    use alloc::string::ToString;

    use super::*;

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
