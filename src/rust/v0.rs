//! v0 (`_R...`) front-end.
//!
//! Recursive descent over the RFC 2603 grammar, mirroring the shape of
//! `rustc-demangle`'s parser: the same recursion points, the same depth limit
//! and the same checked base-62/decimal arithmetic. Backrefs are resolved
//! through a per-offset cache, so every reference to one encoded position
//! shares a single [`NodeRef`] and the graph becomes a DAG.
//!
//! The symbol is read as bytes through [`crate::parse::Cursor`]: every length
//! and every backref is a byte offset, and an identifier may hold bytes that
//! are not UTF-8. Escaping is the printer's job.
//!
//! The unstable `w` splat marker is accepted as a [`Kind::Splat`] wrapper, and
//! printed with the pinned reference's `#[rustc_splat] ` spelling.

use alloc::borrow::Cow;
use alloc::vec;
use alloc::vec::Vec;

use crate::parse::{Checkpoint, Cursor, Fail, State};

use super::ParseError;
use super::node::{
    AdtConstKind, Builder, ConstValueKind, ImplKind, Kind, Namespace, NodeData, NodeRef,
    ParserLimits, PatternKind, Span,
};

/// The production a cached offset was parsed as.
///
/// A backref inherits the production of its context, and one byte offset can
/// be a valid path *and* a valid type, so the cache only reuses a node that was
/// parsed as the same production.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Production {
    /// A `path`.
    Path,
    /// A `type`.
    Type,
    /// A `const`.
    Const,
}

/// A v0 symbol split into its root path, instantiating crate and end offset.
pub(super) struct Parsed<'a> {
    /// The demangled root path.
    pub(super) root: NodeRef<'a>,
    /// The instantiating crate, when the symbol carries one.
    pub(super) instantiating_crate: Option<NodeRef<'a>>,
    /// Absolute offset in the original symbol, just past the last parsed byte.
    pub(super) end: usize,
}

/// The v0 mangling prefixes, longest first, with their text spelling.
const PREFIXES: [(&[u8], &str); 3] = [(b"__R", "__R"), (b"_R", "_R"), (b"R", "R")];

/// The v0 mangling prefix of `symbol`: its length and spelling.
pub(super) fn prefix(symbol: &[u8]) -> Option<(usize, &'static str)> {
    PREFIXES
        .iter()
        .find(|(prefix, _)| symbol.starts_with(prefix))
        .map(|&(prefix, text)| (prefix.len(), text))
}

/// Parse a v0 symbol, including its optional instantiating crate.
pub(super) fn parse(
    symbol: &[u8],
    prefix_len: usize,
    limits: ParserLimits,
) -> Result<Parsed<'_>, ParseError> {
    let mut state = State::new(limits.cursor());
    let input = Cursor::new(symbol, 0, &mut state).map_err(invalid)?;
    let mut parser = Parser::new(input, prefix_len, limits)?;
    let root = parser.path()?;
    // The instantiating crate is another path, parsed for validation only.
    let instantiating_crate = if parser.peek().is_some_and(|tag| tag.is_ascii_uppercase()) {
        let path = parser.path()?;
        Some(parser.node(
            Kind::InstantiatingCrate,
            None,
            None,
            NodeData::None,
            vec![path],
        )?)
    } else {
        None
    };
    Ok(Parsed {
        root,
        instantiating_crate,
        end: parser.pos(),
    })
}

/// Map a cursor failure to the public error.
///
/// The cursor only reports byte-level failures here: the input length is
/// checked before parsing, and the work, node and depth budgets stay with
/// [`Builder`] and [`Parser::push_depth`], which report them as
/// [`ParseError::BudgetExceeded`] and [`ParseError::RecursedTooDeep`].
const fn invalid(_fail: Fail) -> ParseError {
    ParseError::Invalid
}

/// The `basic-type` table, printed verbatim.
const fn basic_type(tag: u8) -> Option<&'static str> {
    Some(match tag {
        b'b' => "bool",
        b'c' => "char",
        b'e' => "str",
        b'u' => "()",
        b'a' => "i8",
        b's' => "i16",
        b'l' => "i32",
        b'x' => "i64",
        b'n' => "i128",
        b'i' => "isize",
        b'h' => "u8",
        b't' => "u16",
        b'm' => "u32",
        b'y' => "u64",
        b'o' => "u128",
        b'j' => "usize",
        b'f' => "f32",
        b'd' => "f64",
        b'z' => "!",
        b'p' => "_",
        b'v' => "...",
        _ => return None,
    })
}

/// True for the tags whose const literal is an integer with a type suffix.
const fn is_integer_type(tag: u8) -> bool {
    matches!(
        tag,
        b'h' | b't' | b'm' | b'y' | b'o' | b'j' | b'a' | b's' | b'l' | b'x' | b'n' | b'i'
    )
}

/// True for the integer tags that may carry the `n` (negative) marker.
const fn is_signed_integer_type(tag: u8) -> bool {
    matches!(tag, b'a' | b's' | b'l' | b'x' | b'n' | b'i')
}

/// A parsed identifier.
struct Ident<'a> {
    /// The `s` disambiguator, zero when the encoding carried none.
    disambiguator: u64,
    /// The printed name: borrowed bytes, or an owned punycode decoding.
    text: Cow<'a, [u8]>,
    /// True when the encoding carried the `u` (punycode) marker.
    punycode: bool,
    /// The span of the whole identifier, disambiguator included.
    span: Span,
}

/// Decode a punycode identifier (RFC 3492).
///
/// `ascii` is the part before the last `_` and `punycode` the rest. A
/// malformed encoding produces the `punycode{ascii-punycode}` fallback text;
/// an ASCII part that is not UTF-8 takes the same fallback, which keeps its
/// bytes verbatim. The decoded name has no length cap: every insertion shifts
/// the chars after it, and that shift is charged to the work budget through
/// `builder`, so a long name costs its quadratic decoding in work units
/// instead of being refused at a fixed length.
fn decode_punycode<'a>(
    ascii: &'a [u8],
    punycode: &'a [u8],
    builder: &mut Builder,
) -> Result<Cow<'a, [u8]>, ParseError> {
    let Ok(ascii_text) = core::str::from_utf8(ascii) else {
        return Ok(punycode_fallback(ascii, punycode));
    };
    let mut out: Vec<char> = Vec::new();
    let mut budget = Ok(());
    let decoded = punycode_decode(ascii_text, punycode, |index, c| {
        if index > out.len() {
            return Err(());
        }
        if let Err(error) = builder.charge(out.len() - index + 1) {
            budget = Err(error);
            return Err(());
        }
        out.insert(index, c);
        Ok(())
    });
    budget?;
    if decoded.is_err() {
        return Ok(punycode_fallback(ascii, punycode));
    }
    let mut bytes = Vec::with_capacity(out.len());
    let mut buf = [0_u8; 4];
    for c in out {
        bytes.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
    }
    Ok(Cow::Owned(bytes))
}

/// The reference's `punycode{ascii-punycode}` fallback text, as raw bytes.
fn punycode_fallback<'a>(ascii: &'a [u8], punycode: &'a [u8]) -> Cow<'a, [u8]> {
    let mut text = Vec::from(&b"punycode{"[..]);
    if !ascii.is_empty() {
        text.extend_from_slice(ascii);
        text.push(b'-');
    }
    text.extend_from_slice(punycode);
    text.push(b'}');
    Cow::Owned(text)
}

/// Decode RFC 3492 punycode, reporting every insertion through `insert`.
///
/// `insert` returns `Err` to abort the decoding. Ported from the reference
/// decoder, including its overflow checks.
#[expect(
    clippy::many_single_char_names,
    reason = "the variable names follow RFC 3492's decoding procedure"
)]
fn punycode_decode<F>(ascii: &str, punycode: &[u8], mut insert: F) -> Result<(), ()>
where
    F: FnMut(usize, char) -> Result<(), ()>,
{
    let mut punycode_bytes = punycode.iter().copied().peekable();
    if punycode_bytes.peek().is_none() {
        return Err(());
    }

    let mut len = 0_usize;

    // The ASCII part comes first, unencoded.
    for c in ascii.chars() {
        insert(len, c)?;
        len += 1;
    }

    // Punycode parameters and initial state.
    let base = 36_usize;
    let t_min = 1_usize;
    let t_max = 26_usize;
    let skew = 38_usize;
    let mut damp = 700_usize;
    let mut bias = 72_usize;
    let mut i = 0_usize;
    let mut n = 0x80_usize;

    loop {
        // Read one delta value.
        let mut delta = 0_usize;
        let mut w = 1_usize;
        let mut k = 0_usize;
        loop {
            k += base;
            let t = k.saturating_sub(bias).clamp(t_min, t_max);

            let digit = match punycode_bytes.next() {
                Some(digit @ b'a'..=b'z') => digit - b'a',
                Some(digit @ b'0'..=b'9') => 26 + (digit - b'0'),
                _ => return Err(()),
            };
            let digit = usize::from(digit);
            delta = delta
                .checked_add(digit.checked_mul(w).ok_or(())?)
                .ok_or(())?;
            if digit < t {
                break;
            }
            w = w.checked_mul(base - t).ok_or(())?;
        }

        // Compute the new insert position and character.
        len += 1;
        i = i.checked_add(delta).ok_or(())?;
        n = n.checked_add(i / len).ok_or(())?;
        i %= len;

        let scalar = u32::try_from(n).ok().and_then(char::from_u32).ok_or(())?;
        insert(i, scalar)?;
        i += 1;

        if punycode_bytes.peek().is_none() {
            return Ok(());
        }

        // Perform bias adaptation.
        delta /= damp;
        damp = 2;
        delta += delta / len;
        let mut k = 0_usize;
        while delta > ((base - t_min) * t_max) / 2 {
            delta /= base - t_min;
            k += base;
        }
        bias = k + ((base - t_min + 1) * delta) / (delta + skew);
    }
}

/// The value of one lowercase hex nibble.
const fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        _ => None,
    }
}

/// Decode the integer value of a hex-nibble string, like the reference.
fn parse_uint(nibbles: &[u8]) -> Option<u64> {
    let trimmed = match nibbles.iter().position(|&byte| byte != b'0') {
        Some(index) => &nibbles[index..],
        None => &[],
    };
    if trimmed.len() > 16 {
        return None;
    }
    trimmed.iter().try_fold(0_u64, |value, &byte| {
        value
            .checked_mul(16)?
            .checked_add(u64::from(hex_value(byte)?))
    })
}

/// Decode a hex-nibble string into the bytes it spells, like the reference.
///
/// The nibbles are arbitrary bytes: a literal that is not UTF-8 is kept as
/// bytes and escaped by the printer, never replaced.
fn parse_str(nibbles: &[u8]) -> Result<Vec<u8>, ParseError> {
    if !nibbles.len().is_multiple_of(2) {
        return Err(ParseError::Invalid);
    }
    let mut decoded = Vec::with_capacity(nibbles.len() / 2);
    for pair in nibbles.as_chunks::<2>().0 {
        let high = hex_value(pair[0]).ok_or(ParseError::Invalid)?;
        let low = hex_value(pair[1]).ok_or(ParseError::Invalid)?;
        decoded.push(high << 4 | low);
    }
    Ok(decoded)
}

/// A byte cursor over one v0 symbol.
struct Parser<'a, 's> {
    /// The symbol bytes, positioned just past the mangling prefix.
    input: Cursor<'a, 's>,
    /// Length of the mangling prefix: backref indices are offsets into the
    /// encoding, which starts just past it.
    base: usize,
    /// A checkpoint at every byte offset of the symbol, so a backref can seek
    /// to an earlier offset without re-reading the bytes before it.
    offsets: Vec<Checkpoint>,
    /// Current recursion depth.
    depth: u32,
    /// Maximum recursion depth this parse accepts.
    max_depth: u32,
    /// Node and work budget accounting.
    builder: Builder,
    /// Nodes already parsed, indexed by absolute offset in the symbol, with the
    /// offset just past the production. A cache hit must advance the cursor,
    /// otherwise a caller parsing at an already-seen offset would stop making
    /// progress.
    cache: Vec<Option<(Production, NodeRef<'a>, usize)>>,
    /// Late-bound lifetimes in scope for the current production.
    binder_depth: u64,
}

impl<'a, 's> Parser<'a, 's> {
    /// Start parsing `symbol` just past its v0 prefix.
    fn new(
        mut input: Cursor<'a, 's>,
        prefix_len: usize,
        limits: ParserLimits,
    ) -> Result<Self, ParseError> {
        let len = input.remaining().len();
        let mut offsets = Vec::with_capacity(len);
        for _ in 0..len {
            offsets.push(input.checkpoint());
            let _ = input.advance(1).map_err(invalid)?;
        }
        // The walk left the cursor at the end of the symbol.
        let begin = offsets.first().copied().ok_or(ParseError::Invalid)?;
        input.reset(begin);
        let _ = input.advance(prefix_len).map_err(invalid)?;
        // Paths always start with an uppercase tag.
        if !input.peek().is_some_and(|tag| tag.is_ascii_uppercase()) {
            return Err(ParseError::Invalid);
        }
        Ok(Self {
            input,
            base: prefix_len,
            offsets,
            depth: 0,
            max_depth: limits.depth,
            builder: Builder::new(limits),
            cache: vec![None; len],
            binder_depth: 0,
        })
    }

    /// Build a node, charging the budgets.
    fn node(
        &mut self,
        kind: Kind,
        text: Option<Cow<'a, [u8]>>,
        span: Option<Span>,
        data: NodeData<'a>,
        children: Vec<NodeRef<'a>>,
    ) -> Result<NodeRef<'a>, ParseError> {
        self.builder.node(kind, text, span, data, children)
    }

    /// Build a childless node spanning `start..self.pos()`.
    fn leaf(
        &mut self,
        kind: Kind,
        text: Option<Cow<'a, [u8]>>,
        start: Checkpoint,
        data: NodeData<'a>,
    ) -> Result<NodeRef<'a>, ParseError> {
        let span = self.span(start);
        self.node(kind, text, Some(span), data, Vec::new())
    }

    /// The offset of the next unread byte in the symbol.
    const fn pos(&self) -> usize {
        self.input.pos()
    }

    /// The span of `start..self.pos()`.
    fn span(&self, start: Checkpoint) -> Span {
        let span = self.input.span(start);
        Span {
            start: span.start,
            end: span.end,
        }
    }

    /// The span of the byte range `start..end`.
    fn span_of(start: usize, end: usize) -> Span {
        Span {
            start: offset(start),
            end: offset(end),
        }
    }

    /// Enter one recursion level.
    const fn push_depth(&mut self) -> Result<(), ParseError> {
        self.depth = self.depth.saturating_add(1);
        if self.depth > self.max_depth {
            return Err(ParseError::RecursedTooDeep);
        }
        Ok(())
    }

    /// Leave one recursion level.
    const fn pop_depth(&mut self) {
        self.depth = self.depth.saturating_sub(1);
    }

    /// The next byte, without consuming it.
    fn peek(&self) -> Option<u8> {
        self.input.peek()
    }

    /// Consume `byte` if it is next.
    fn eat(&mut self, byte: u8) -> bool {
        if self.peek() == Some(byte) {
            let _ = self.input.advance(1);
            true
        } else {
            false
        }
    }

    /// Consume and return the next byte.
    fn bump(&mut self) -> Result<u8, ParseError> {
        let byte = self.peek().ok_or(ParseError::Invalid)?;
        let _ = self.input.advance(1).map_err(invalid)?;
        Ok(byte)
    }

    /// Consume a decimal digit.
    fn digit_10(&mut self) -> Result<u8, ParseError> {
        match self.peek() {
            Some(digit @ b'0'..=b'9') => {
                let _ = self.input.advance(1).map_err(invalid)?;
                Ok(digit - b'0')
            }
            _ => Err(ParseError::Invalid),
        }
    }

    /// Consume a base-62 digit.
    fn digit_62(&mut self) -> Result<u8, ParseError> {
        let digit = match self.peek() {
            Some(digit @ b'0'..=b'9') => digit - b'0',
            Some(digit @ b'a'..=b'z') => 10 + (digit - b'a'),
            Some(digit @ b'A'..=b'Z') => 10 + 26 + (digit - b'A'),
            _ => return Err(ParseError::Invalid),
        };
        let _ = self.input.advance(1).map_err(invalid)?;
        Ok(digit)
    }

    /// A base-62 number terminated by `_`; `_` alone is zero.
    fn integer_62(&mut self) -> Result<u64, ParseError> {
        if self.eat(b'_') {
            return Ok(0);
        }
        let mut value: u64 = 0;
        while !self.eat(b'_') {
            let digit = u64::from(self.digit_62()?);
            value = value.checked_mul(62).ok_or(ParseError::Invalid)?;
            value = value.checked_add(digit).ok_or(ParseError::Invalid)?;
        }
        value.checked_add(1).ok_or(ParseError::Invalid)
    }

    /// `tag` + base-62 + 1, or zero when the tag is absent.
    fn opt_integer_62(&mut self, tag: u8) -> Result<u64, ParseError> {
        if !self.eat(tag) {
            return Ok(0);
        }
        self.integer_62()?.checked_add(1).ok_or(ParseError::Invalid)
    }

    /// A `disambiguator`: `s` + base-62 + 1.
    fn disambiguator(&mut self) -> Result<u64, ParseError> {
        self.opt_integer_62(b's')
    }

    /// A `namespace` tag.
    fn namespace(&mut self) -> Result<Namespace, ParseError> {
        Ok(match self.bump()? {
            b'C' => Namespace::Closure,
            b'S' => Namespace::Shim,
            tag @ b'A'..=b'Z' => Namespace::Other(char::from(tag)),
            b'a'..=b'z' => Namespace::Internal,
            _ => return Err(ParseError::Invalid),
        })
    }

    /// An `identifier`: optional disambiguator, then the name.
    fn ident(&mut self) -> Result<Ident<'a>, ParseError> {
        let start = self.input.checkpoint();
        let disambiguator = self.disambiguator()?;
        let is_punycode = self.eat(b'u');
        // The reference stops reading the length after a leading zero, so the
        // loop is written out rather than shared with the legacy front-end.
        let mut len = usize::from(self.digit_10()?);
        if len != 0 {
            while let Ok(digit) = self.digit_10() {
                len = len.checked_mul(10).ok_or(ParseError::Invalid)?;
                len = len
                    .checked_add(usize::from(digit))
                    .ok_or(ParseError::Invalid)?;
            }
        }
        // Skip the optional `_` separator between the length and the bytes.
        let _ = self.eat(b'_');
        let raw = self.input.advance(len).map_err(invalid)?;
        let text = if is_punycode {
            // The last `_` separates the ASCII part from the punycode part.
            let (ascii, punycode) = raw.iter().rposition(|&byte| byte == b'_').map_or_else(
                || (&[][..], raw),
                |index| (&raw[..index], &raw[index + 1..]),
            );
            if punycode.is_empty() {
                return Err(ParseError::Invalid);
            }
            decode_punycode(ascii, punycode, &mut self.builder)?
        } else {
            Cow::Borrowed(raw)
        };
        Ok(Ident {
            disambiguator,
            text,
            punycode: is_punycode,
            span: self.span(start),
        })
    }

    /// A `path`.
    fn path(&mut self) -> Result<NodeRef<'a>, ParseError> {
        self.enter(Production::Path)
    }

    /// A `type`.
    fn ty(&mut self) -> Result<NodeRef<'a>, ParseError> {
        self.enter(Production::Type)
    }

    /// A `const`.
    fn const_value(&mut self) -> Result<NodeRef<'a>, ParseError> {
        self.enter(Production::Const)
    }

    /// Parse the production at the cursor, reusing a cached node when the same
    /// production was already parsed at this offset.
    fn enter(&mut self, production: Production) -> Result<NodeRef<'a>, ParseError> {
        // Charge every production entry, including cache hits, so total parser
        // work is bounded even when a caller re-enters cached spans.
        self.builder.charge(1)?;
        let start = self.pos();
        let checkpoint = self.input.checkpoint();
        if let Some(Some((cached, node, end))) = self.cache.get(start)
            && *cached == production
        {
            let skip = end.checked_sub(start).ok_or(ParseError::Invalid)?;
            let _ = self.input.advance(skip).map_err(invalid)?;
            return Ok(node.clone());
        }
        let node = match production {
            Production::Path => self.path_inner(checkpoint)?,
            Production::Type => self.type_inner(checkpoint)?,
            Production::Const => self.const_inner(checkpoint)?,
        };
        let end = self.pos();
        if let Some(slot) = self.cache.get_mut(start) {
            *slot = Some((production, node.clone(), end));
        }
        Ok(node)
    }

    /// Parse a `path` whose first tag is at `start`.
    fn path_inner(&mut self, start: Checkpoint) -> Result<NodeRef<'a>, ParseError> {
        self.push_depth()?;
        let node = self.path_body(start);
        self.pop_depth();
        node
    }

    /// The body of [`Parser::path_inner`].
    fn path_body(&mut self, start: Checkpoint) -> Result<NodeRef<'a>, ParseError> {
        match self.bump()? {
            b'C' => {
                let disambiguator = self.disambiguator()?;
                let ident = self.ident()?;
                self.leaf(
                    Kind::CrateRoot,
                    Some(ident.text),
                    start,
                    NodeData::Disambiguator(disambiguator),
                )
            }
            b'N' => {
                let namespace = self.namespace()?;
                let parent = self.path()?;
                let ident = self.ident()?;
                let ident = self.node(
                    Kind::Ident,
                    Some(ident.text),
                    Some(ident.span),
                    NodeData::Disambiguator(ident.disambiguator),
                    Vec::new(),
                )?;
                self.node(
                    Kind::Nested,
                    None,
                    Some(self.span(start)),
                    NodeData::Namespace(namespace),
                    vec![parent, ident],
                )
            }
            tag @ (b'M' | b'X' | b'Y') => {
                let kind = match tag {
                    b'M' => ImplKind::Inherent,
                    b'X' => ImplKind::TraitImpl,
                    _ => ImplKind::TraitDef,
                };
                let mut children = Vec::new();
                if kind != ImplKind::TraitDef {
                    // The impl path's own disambiguator is never printed; the
                    // path itself is kept for inspection.
                    let _ = self.disambiguator()?;
                    children.push(self.path()?);
                }
                children.push(self.ty()?);
                if kind != ImplKind::Inherent {
                    children.push(self.path()?);
                }
                self.node(
                    Kind::Impl,
                    None,
                    Some(self.span(start)),
                    NodeData::Impl(kind),
                    children,
                )
            }
            b'I' => {
                let base = self.path()?;
                let mut children = vec![base];
                while !self.eat(b'E') {
                    children.push(self.generic_arg()?);
                }
                self.node(
                    Kind::GenericArgs,
                    None,
                    Some(self.span(start)),
                    NodeData::None,
                    children,
                )
            }
            b'B' => {
                let target = self.backref()?;
                self.enter_at(target, Production::Path)
            }
            _ => Err(ParseError::Invalid),
        }
    }

    /// A `generic-arg`: lifetime, const or type.
    fn generic_arg(&mut self) -> Result<NodeRef<'a>, ParseError> {
        if self.eat(b'L') {
            self.lifetime()
        } else if self.eat(b'K') {
            self.const_value()
        } else {
            self.ty()
        }
    }

    /// A `lifetime`: `L` + base-62. The caller consumes the `L`.
    fn lifetime(&mut self) -> Result<NodeRef<'a>, ParseError> {
        let start = self.pos().saturating_sub(1);
        let index = self.integer_62()?;
        if index > self.binder_depth {
            return Err(ParseError::Invalid);
        }
        let span = Self::span_of(start, self.pos());
        self.node(
            Kind::Lifetime,
            None,
            Some(span),
            NodeData::Lifetime(index),
            Vec::new(),
        )
    }

    /// Resolve a `backref` whose `B` tag was just consumed.
    ///
    /// Returns the offset of the target production.
    fn backref(&mut self) -> Result<usize, ParseError> {
        self.builder.charge(1)?;
        // Backref indices count from the start of the encoding.
        let backref_offset = self.pos().saturating_sub(self.base).saturating_sub(1);
        let target = usize::try_from(self.integer_62()?).map_err(|_| ParseError::Invalid)?;
        // Backrefs must point strictly before themselves, which makes cycles
        // impossible by construction.
        if target >= backref_offset {
            return Err(ParseError::Invalid);
        }
        // The reference re-parses the target at the backref site's depth, so a
        // chain of backrefs is depth-limited.
        self.push_depth()?;
        Ok(target)
    }

    /// Parse `production` at `offset`, then leave the backref's depth level.
    fn enter_at(
        &mut self,
        offset: usize,
        production: Production,
    ) -> Result<NodeRef<'a>, ParseError> {
        let saved = self.input.checkpoint();
        let target = *self
            .offsets
            .get(self.base.saturating_add(offset))
            .ok_or(ParseError::Invalid)?;
        self.input.reset(target);
        let node = self.enter(production);
        self.input.reset(saved);
        self.pop_depth();
        node
    }

    /// Parse a `type` whose first tag is at `start`.
    fn type_inner(&mut self, start: Checkpoint) -> Result<NodeRef<'a>, ParseError> {
        // The unstable splat marker prefixes a type. The reference consumes at
        // most one marker per type, so `ww...` is invalid rather than nested.
        if self.eat(b'w') {
            if self.peek() == Some(b'w') {
                return Err(ParseError::Invalid);
            }
            let ty = self.ty()?;
            return self.node(
                Kind::Splat,
                None,
                Some(self.span(start)),
                NodeData::None,
                vec![ty],
            );
        }
        let tag = self.bump()?;
        if let Some(name) = basic_type(tag) {
            return self.leaf(
                Kind::BasicType,
                Some(Cow::Borrowed(name.as_bytes())),
                start,
                NodeData::None,
            );
        }
        self.push_depth()?;
        let node = self.type_body(tag, start);
        self.pop_depth();
        node
    }

    /// The body of [`Parser::type_inner`], after its tag.
    fn type_body(&mut self, tag: u8, start: Checkpoint) -> Result<NodeRef<'a>, ParseError> {
        match tag {
            b'R' | b'Q' => {
                let mut children = Vec::new();
                if self.eat(b'L') {
                    children.push(self.lifetime()?);
                }
                children.push(self.ty()?);
                self.node(
                    Kind::Ref,
                    None,
                    Some(self.span(start)),
                    NodeData::Mutability(tag == b'Q'),
                    children,
                )
            }
            b'P' | b'O' => {
                let pointee = self.ty()?;
                self.node(
                    Kind::RawPtr,
                    None,
                    Some(self.span(start)),
                    NodeData::Mutability(tag == b'O'),
                    vec![pointee],
                )
            }
            b'A' => {
                let element = self.ty()?;
                let len = self.const_value()?;
                self.node(
                    Kind::Array,
                    None,
                    Some(self.span(start)),
                    NodeData::None,
                    vec![element, len],
                )
            }
            b'S' => {
                let element = self.ty()?;
                self.node(
                    Kind::Slice,
                    None,
                    Some(self.span(start)),
                    NodeData::None,
                    vec![element],
                )
            }
            b'T' => {
                let mut children = Vec::new();
                while !self.eat(b'E') {
                    children.push(self.ty()?);
                }
                self.node(
                    Kind::Tuple,
                    None,
                    Some(self.span(start)),
                    NodeData::None,
                    children,
                )
            }
            b'B' => {
                let target = self.backref()?;
                self.enter_at(target, Production::Type)
            }
            b'F' => self.fn_sig(start),
            b'D' => self.dyn_bounds(start),
            b'W' => {
                let ty = self.ty()?;
                let pattern = self.pattern()?;
                self.node(
                    Kind::PatternType,
                    None,
                    Some(self.span(start)),
                    NodeData::None,
                    vec![ty, pattern],
                )
            }
            // Every other tag starts a path, which `type_inner` already
            // consumed the first byte of.
            _ => {
                self.input.reset(start);
                self.path()
            }
        }
    }

    /// A `fn-sig`: `[binder] ["U"] ["K" abi] {type} "E" type`.
    fn fn_sig(&mut self, start: Checkpoint) -> Result<NodeRef<'a>, ParseError> {
        let binder = self.opt_integer_62(b'G')?;
        let is_unsafe = self.eat(b'U');
        let abi = if self.eat(b'K') {
            if self.eat(b'C') {
                Some(Cow::Borrowed(b"C".as_slice()))
            } else {
                // An ABI name is a plain identifier: the reference rejects an
                // empty name and a punycode one.
                let Ident { text, punycode, .. } = self.ident()?;
                if text.is_empty() || punycode {
                    return Err(ParseError::Invalid);
                }
                Some(text)
            }
        } else {
            None
        };
        let outer_binder = self.binder_depth;
        self.binder_depth = self.binder_depth.saturating_add(binder);
        let mut children = Vec::new();
        while !self.eat(b'E') {
            children.push(self.ty()?);
        }
        // A bare `u` tag is `()`; the reference prints no `-> ()` for it.
        let ret_elided = self.peek() == Some(b'u');
        children.push(self.ty()?);
        self.binder_depth = outer_binder;
        self.node(
            Kind::Fn,
            None,
            Some(self.span(start)),
            NodeData::Function {
                binder,
                is_unsafe,
                abi,
                ret_elided,
            },
            children,
        )
    }

    /// A `dyn-bounds`: `[binder] {dyn-trait} "E" "L" <base-62>`.
    fn dyn_bounds(&mut self, start: Checkpoint) -> Result<NodeRef<'a>, ParseError> {
        let binder = self.opt_integer_62(b'G')?;
        let outer_binder = self.binder_depth;
        self.binder_depth = self.binder_depth.saturating_add(binder);
        let mut children = Vec::new();
        while !self.eat(b'E') {
            children.push(self.dyn_trait()?);
        }
        self.binder_depth = outer_binder;
        if !self.eat(b'L') {
            return Err(ParseError::Invalid);
        }
        children.push(self.lifetime()?);
        self.node(
            Kind::Dyn,
            None,
            Some(self.span(start)),
            NodeData::Binder(binder),
            children,
        )
    }

    /// A `dyn-trait`: a path followed by its `p` associated bindings.
    fn dyn_trait(&mut self) -> Result<NodeRef<'a>, ParseError> {
        let start = self.input.checkpoint();
        let path = self.path()?;
        let mut children = vec![path];
        loop {
            let binding_start = self.input.checkpoint();
            if !self.eat(b'p') {
                break;
            }
            let ident = self.ident()?;
            let name = self.node(
                Kind::Ident,
                Some(ident.text),
                Some(ident.span),
                NodeData::Disambiguator(ident.disambiguator),
                Vec::new(),
            )?;
            let value = if self.eat(b'K') {
                self.const_value()?
            } else {
                self.ty()?
            };
            children.push(self.node(
                Kind::DynAssocBinding,
                None,
                Some(self.span(binding_start)),
                NodeData::None,
                vec![name, value],
            )?);
        }
        self.node(
            Kind::DynTrait,
            None,
            Some(self.span(start)),
            NodeData::None,
            children,
        )
    }

    /// A `pattern`: a range, an or-pattern or `!null`.
    fn pattern(&mut self) -> Result<NodeRef<'a>, ParseError> {
        let start = self.input.checkpoint();
        let (kind, children) = match self.bump()? {
            b'R' => {
                let low = self.const_value()?;
                let high = self.const_value()?;
                (PatternKind::Range, vec![low, high])
            }
            b'O' => {
                self.push_depth()?;
                let mut children = Vec::new();
                while !self.eat(b'E') {
                    children.push(self.pattern()?);
                }
                self.pop_depth();
                (PatternKind::Or, children)
            }
            b'N' => (PatternKind::NeverNull, Vec::new()),
            _ => return Err(ParseError::Invalid),
        };
        self.node(
            Kind::Pattern,
            None,
            Some(self.span(start)),
            NodeData::Pattern(kind),
            children,
        )
    }

    /// Parse a `const` whose first tag is at `start`.
    fn const_inner(&mut self, start: Checkpoint) -> Result<NodeRef<'a>, ParseError> {
        let tag = self.bump()?;
        self.push_depth()?;
        let node = self.const_body(tag, start);
        self.pop_depth();
        node
    }

    /// The body of [`Parser::const_inner`], after its tag.
    #[expect(
        clippy::too_many_lines,
        reason = "one match arm per grammar production"
    )]
    fn const_body(&mut self, tag: u8, start: Checkpoint) -> Result<NodeRef<'a>, ParseError> {
        match tag {
            b'p' => {
                let value = self.leaf(
                    Kind::ConstValue,
                    None,
                    start,
                    NodeData::Const(ConstValueKind::Placeholder),
                )?;
                self.const_node(start, vec![value])
            }
            _ if is_integer_type(tag) => {
                let name = basic_type(tag).ok_or(ParseError::Invalid)?;
                let ty = self.leaf(
                    Kind::BasicType,
                    Some(Cow::Borrowed(name.as_bytes())),
                    start,
                    NodeData::None,
                )?;
                let negative = is_signed_integer_type(tag) && self.eat(b'n');
                let (nibbles, span) = self.nibbles()?;
                let value = self.node(
                    Kind::ConstValue,
                    Some(nibbles),
                    Some(span),
                    NodeData::Const(ConstValueKind::Integer { negative }),
                    Vec::new(),
                )?;
                self.const_node(start, vec![ty, value])
            }
            b'b' => {
                let (nibbles, span) = self.nibbles()?;
                let flag = match parse_uint(&nibbles) {
                    Some(0) => false,
                    Some(1) => true,
                    _ => return Err(ParseError::Invalid),
                };
                let value = self.node(
                    Kind::ConstValue,
                    None,
                    Some(span),
                    NodeData::Const(ConstValueKind::Bool(flag)),
                    Vec::new(),
                )?;
                self.const_node(start, vec![value])
            }
            b'c' => {
                let (nibbles, span) = self.nibbles()?;
                let c = parse_uint(&nibbles)
                    .and_then(|value| u32::try_from(value).ok())
                    .and_then(char::from_u32)
                    .ok_or(ParseError::Invalid)?;
                let mut buf = [0_u8; 4];
                let text = Vec::from(c.encode_utf8(&mut buf).as_bytes());
                let value = self.node(
                    Kind::ConstValue,
                    Some(Cow::Owned(text)),
                    Some(span),
                    NodeData::Const(ConstValueKind::Char),
                    Vec::new(),
                )?;
                self.const_node(start, vec![value])
            }
            b'e' => {
                let (nibbles, span) = self.nibbles()?;
                let text = parse_str(&nibbles)?;
                let value = self.node(
                    Kind::ConstValue,
                    Some(Cow::Owned(text)),
                    Some(span),
                    NodeData::Const(ConstValueKind::Str),
                    Vec::new(),
                )?;
                self.const_node(start, vec![value])
            }
            // `Re` is a `&str` literal; every other `R`/`Q` wraps a const.
            b'R' | b'Q' => {
                if tag == b'R' && self.eat(b'e') {
                    let (nibbles, span) = self.nibbles()?;
                    let text = parse_str(&nibbles)?;
                    let value = self.node(
                        Kind::ConstValue,
                        Some(Cow::Owned(text)),
                        Some(span),
                        NodeData::Const(ConstValueKind::StrRef),
                        Vec::new(),
                    )?;
                    return self.const_node(start, vec![value]);
                }
                let inner = self.const_value()?;
                let value = self.node(
                    Kind::ConstValue,
                    None,
                    Some(self.span(start)),
                    NodeData::Const(ConstValueKind::Ref {
                        mutability: tag == b'Q',
                    }),
                    vec![inner],
                )?;
                self.const_node(start, vec![value])
            }
            b'A' | b'T' => {
                let mut children = Vec::new();
                while !self.eat(b'E') {
                    children.push(self.const_value()?);
                }
                let kind = if tag == b'A' {
                    ConstValueKind::Array
                } else {
                    ConstValueKind::Tuple
                };
                let value = self.node(
                    Kind::ConstValue,
                    None,
                    Some(self.span(start)),
                    NodeData::Const(kind),
                    children,
                )?;
                self.const_node(start, vec![value])
            }
            b'V' => {
                let path = self.path()?;
                let mut children = vec![path];
                let kind = match self.bump()? {
                    b'U' => AdtConstKind::Unit,
                    b'T' => {
                        while !self.eat(b'E') {
                            children.push(self.const_value()?);
                        }
                        AdtConstKind::Tuple
                    }
                    b'S' => {
                        loop {
                            let field_start = self.input.checkpoint();
                            if self.eat(b'E') {
                                break;
                            }
                            let ident = self.ident()?;
                            let name = self.node(
                                Kind::Ident,
                                Some(ident.text),
                                Some(ident.span),
                                NodeData::Disambiguator(ident.disambiguator),
                                Vec::new(),
                            )?;
                            let value = self.const_value()?;
                            children.push(self.node(
                                Kind::AdtField,
                                None,
                                Some(self.span(field_start)),
                                NodeData::None,
                                vec![name, value],
                            )?);
                        }
                        AdtConstKind::Struct
                    }
                    _ => return Err(ParseError::Invalid),
                };
                let value = self.node(
                    Kind::ConstValue,
                    None,
                    Some(self.span(start)),
                    NodeData::Const(ConstValueKind::Adt(kind)),
                    children,
                )?;
                self.const_node(start, vec![value])
            }
            b'B' => {
                let target = self.backref()?;
                self.enter_at(target, Production::Const)
            }
            _ => Err(ParseError::Invalid),
        }
    }

    /// A `Const` node wrapping `children`, spanning `start..self.pos()`.
    fn const_node(
        &mut self,
        start: Checkpoint,
        children: Vec<NodeRef<'a>>,
    ) -> Result<NodeRef<'a>, ParseError> {
        self.node(
            Kind::Const,
            None,
            Some(self.span(start)),
            NodeData::None,
            children,
        )
    }

    /// A run of lowercase hex nibbles terminated by `_`, with the span of the
    /// digits alone.
    fn nibbles(&mut self) -> Result<(Cow<'a, [u8]>, Span), ParseError> {
        let start = self.pos();
        let nibbles = self.hex_nibbles()?;
        let span = Self::span_of(start, self.pos().saturating_sub(1));
        Ok((nibbles, span))
    }

    /// A run of lowercase hex nibbles terminated by `_`.
    fn hex_nibbles(&mut self) -> Result<Cow<'a, [u8]>, ParseError> {
        let start = self.pos();
        loop {
            match self.bump()? {
                b'0'..=b'9' | b'a'..=b'f' => {}
                b'_' => break,
                _ => return Err(ParseError::Invalid),
            }
        }
        let nibbles = self
            .input
            .bytes_between(start, self.pos().saturating_sub(1))
            .ok_or(ParseError::Invalid)?;
        Ok(Cow::Borrowed(nibbles))
    }
}

/// `offset` as a span bound, saturating at `u32::MAX`.
fn offset(value: usize) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}

#[cfg(test)]
mod tests;
