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
pub(super) fn parse<'a>(
    symbol: &'a [u8],
    prefix_len: usize,
    limits: ParserLimits,
) -> Result<Parsed<'a>, ParseError> {
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
fn invalid(_fail: Fail) -> ParseError {
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

/// The maximum number of chars a punycode identifier may decode into.
///
/// The reference decodes into a fixed stack buffer of this size and prints the
/// `punycode{...}` fallback for anything longer, so matching the length keeps
/// the demangled text byte-identical.
const SMALL_PUNYCODE_LEN: usize = 128;

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

/// Decode a punycode identifier exactly like the reference printer.
///
/// `ascii` is the part before the last `_` and `punycode` the rest. Failure,
/// including a name longer than [`SMALL_PUNYCODE_LEN`], produces the
/// `punycode{ascii-punycode}` fallback text the reference prints. The
/// reference decodes the ASCII part character by character, so an ASCII part
/// that is not UTF-8 takes the same fallback, which keeps its bytes verbatim.
fn decode_punycode<'a>(ascii: &'a [u8], punycode: &'a [u8]) -> Cow<'a, [u8]> {
    let mut out = ['\0'; SMALL_PUNYCODE_LEN];
    let mut len = 0_usize;
    let Ok(ascii_text) = core::str::from_utf8(ascii) else {
        return punycode_fallback(ascii, punycode);
    };
    let decoded = punycode_decode(ascii_text, punycode, |index, c| {
        // `out` grows by one char at `index`, so it needs room for it.
        if index > len || len >= SMALL_PUNYCODE_LEN {
            return Err(());
        }
        let mut j = len;
        len += 1;
        while j > index {
            out[j] = out[j - 1];
            j -= 1;
        }
        out[index] = c;
        Ok(())
    });
    if decoded.is_err() {
        return punycode_fallback(ascii, punycode);
    }
    let mut bytes = Vec::with_capacity(len);
    let mut buf = [0_u8; 4];
    for c in out.get(..len).unwrap_or(&[]).iter().copied() {
        bytes.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
    }
    Cow::Owned(bytes)
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
    fn pos(&self) -> usize {
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
    fn span_of(&self, start: usize, end: usize) -> Span {
        Span {
            start: offset(start),
            end: offset(end),
        }
    }

    /// Enter one recursion level.
    fn push_depth(&mut self) -> Result<(), ParseError> {
        self.depth = self.depth.saturating_add(1);
        if self.depth > self.max_depth {
            return Err(ParseError::RecursedTooDeep);
        }
        Ok(())
    }

    /// Leave one recursion level.
    fn pop_depth(&mut self) {
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
            decode_punycode(ascii, punycode)
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
        let span = self.span_of(start, self.pos());
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
        let span = self.span_of(start, self.pos().saturating_sub(1));
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
mod tests {
    use alloc::format;
    use alloc::rc::Rc;
    use alloc::string::{String, ToString};
    use alloc::vec;
    use alloc::vec::Vec;

    use super::*;
    use crate::rust::{Encoding, parse};

    /// Assert both renderings, and cross-check them with the reference decoder.
    fn check(symbol: &str, display: &str, alternate: &str) {
        let ast = parse(symbol).unwrap();
        assert_eq!(ast.encoding(), Encoding::V0);
        assert_eq!(ast.to_string(), display, "{symbol}");
        assert_eq!(format!("{ast:#}"), alternate, "{symbol}");
        check_oracle(symbol, display, alternate);
    }

    /// Compare with `rustc-demangle`, the executable oracle.
    fn check_oracle(symbol: &str, display: &str, alternate: &str) {
        let oracle = rustc_demangle::try_demangle(symbol).unwrap();
        // The locked 0.1.28 release predates the pinned `#[rustc_splat]`
        // spelling, so normalize that one documented divergence.
        let normalize = |text: String| {
            if display.contains("#[rustc_splat] ") || alternate.contains("#[rustc_splat] ") {
                text.replace("#[splat] ", "#[rustc_splat] ")
            } else {
                text
            }
        };
        assert_eq!(normalize(format!("{oracle}")), display, "{symbol}");
        assert_eq!(normalize(format!("{oracle:#}")), alternate, "{symbol}");
    }

    #[test]
    fn crate_root_and_nested_paths() {
        check("_RC3foo", "foo", "foo");
        check("_RNvC3foo3bar", "foo::bar", "foo::bar");
        check("_RNvC6_123foo3bar", "123foo::bar", "123foo::bar");
        check(
            "_RNvNtNtNtC4rand4rngs7adapter9reseeding4fork",
            "rand::rngs::adapter::reseeding::fork",
            "rand::rngs::adapter::reseeding::fork",
        );
    }

    #[test]
    fn crate_root_disambiguator_is_display_only() {
        // `s_` is the smallest disambiguator: base-62 zero plus the two
        // "+ 1" adjustments in `disambiguator` and `integer_62`.
        check("_RCs_3foo", "foo[1]", "foo");
        check("_RCs0_3foo", "foo[2]", "foo");
    }

    #[test]
    fn special_namespaces() {
        // The corpus only records the `{:#}` expectation; `Display` also
        // prints the crate root's disambiguator.
        check(
            "_RNCNCNgCs6DXkGYLi8lr_2cc5spawn00B5_",
            "cc[4d6468d6c9fd4bb3]::spawn::{closure#0}::{closure#0}",
            "cc::spawn::{closure#0}::{closure#0}",
        );
        check("_RNSC3foo3bar", "foo::{shim:bar#0}", "foo::{shim:bar#0}");
    }

    #[test]
    fn basic_types() {
        for (tag, name) in [
            ('b', "bool"),
            ('c', "char"),
            ('e', "str"),
            ('u', "()"),
            ('a', "i8"),
            ('s', "i16"),
            ('l', "i32"),
            ('x', "i64"),
            ('n', "i128"),
            ('i', "isize"),
            ('h', "u8"),
            ('t', "u16"),
            ('m', "u32"),
            ('y', "u64"),
            ('o', "u128"),
            ('j', "usize"),
            ('f', "f32"),
            ('d', "f64"),
            ('z', "!"),
            ('p', "_"),
            ('v', "..."),
        ] {
            let symbol = format!("_RMC0{tag}");
            check(&symbol, &format!("<{name}>"), &format!("<{name}>"));
        }
    }

    #[test]
    fn references_and_pointers() {
        check("_RMC0Rh", "<&u8>", "<&u8>");
        check("_RMC0Qh", "<&mut u8>", "<&mut u8>");
        check("_RMC0Ph", "<*const u8>", "<*const u8>");
        check("_RMC0Oh", "<*mut u8>", "<*mut u8>");
        // A lifetime index of 0 is `'_`, which the reference omits entirely.
        check("_RMC0RL_h", "<&u8>", "<&u8>");
        // Index 1 needs a binder, and binders are not implemented yet.
        assert_eq!(parse("_RMC0RL0_h").unwrap_err(), ParseError::Invalid);
    }

    #[test]
    fn tuples_arrays_and_slices() {
        check("_RMC0ThmE", "<(u8, u32)>", "<(u8, u32)>");
        check("_RMC0TpE", "<(_,)>", "<(_,)>");
        check("_RMC0TE", "<()>", "<()>");
        check("_RMC0Sh", "<[u8]>", "<[u8]>");
        check("_RMC0Ahj4_", "<[u8; 4usize]>", "<[u8; 4]>");
        check(
            "_RMC0AAhj4_j0_",
            "<[[u8; 4usize]; 0usize]>",
            "<[[u8; 4]; 0]>",
        );
    }

    #[test]
    fn generic_args() {
        check("_RIC0hE", "::<u8>", "::<u8>");
        check("_RIC0hKj4_E", "::<u8, 4usize>", "::<u8, 4>");
        check("_RIC0L_E", "::<'_>", "::<'_>");
        check("_RIC0KpE", "::<_>", "::<_>");
        check(
            "_RMC0INtC8arrayvec8ArrayVechKj7b_E",
            "<arrayvec::ArrayVec<u8, 123usize>>",
            "<arrayvec::ArrayVec<u8, 123>>",
        );
    }

    #[test]
    fn impls() {
        check("_RMC0h", "<u8>", "<u8>");
        check("_RXC0hC3foo", "<u8 as foo>", "<u8 as foo>");
        check("_RYhC3foo", "<u8 as foo>", "<u8 as foo>");
        // `X` keeps the impl path in the tree but never prints it.
        let ast = parse("_RXC0hC3foo").unwrap();
        let impl_node = ast.root();
        assert_eq!(impl_node.kind(), Kind::Impl);
        assert_eq!(impl_node.children().len(), 3);
    }

    #[test]
    fn const_values() {
        check("_RIC0Kanb_E", "::<-11i8>", "::<-11>");
        check("_RIC0Khb_E", "::<11u8>", "::<11>");
        check("_RIC0Kj7b_E", "::<123usize>", "::<123>");
        check(
            "_RIC0Koff00ff00ff00ff00ff_E",
            "::<0xff00ff00ff00ff00ffu128>",
            "::<0xff00ff00ff00ff00ff>",
        );
        // Leading zeros are trimmed before the `u64` check.
        check("_RIC0Kj0007b_E", "::<123usize>", "::<123>");
        check("_RIC0Kb1_E", "::<true>", "::<true>");
        check("_RIC0Kb0_E", "::<false>", "::<false>");
        check("_RIC0Kc76_E", "::<'v'>", "::<'v'>");
        check("_RIC0Kca_E", "::<'\\n'>", "::<'\\n'>");
        check("_RIC0Kc22_E", "::<'\"'>", "::<'\"'>");
        // `e` is a `str` literal and `Re` a `&str` literal.
        check("_RIC0Ke616263_E", "::<{*\"abc\"}>", "::<{*\"abc\"}>");
        check("_RIC0Ke090a_E", "::<{*\"\\t\\n\"}>", "::<{*\"\\t\\n\"}>");
        check("_RIC0KRe616263_E", "::<\"abc\">", "::<\"abc\">");
        check("_RIC0KRe27_E", "::<\"'\">", "::<\"'\">");
        // Compound values need braces outside a const expression, and keep
        // the integer type suffixes of their leaves in `Display` mode.
        check("_RIC0KRh7b_E", "::<{&123u8}>", "::<{&123}>");
        check("_RIC0KQpE", "::<{&mut _}>", "::<{&mut _}>");
        check("_RIC0KRRRe_E", "::<{&&\"\"}>", "::<{&&\"\"}>");
        check("_RIC0KQAEE", "::<{&mut []}>", "::<{&mut []}>");
        check(
            "_RIC0KAAh1_h2_EAh3_h4_EEE",
            "::<{[[1u8, 2u8], [3u8, 4u8]]}>",
            "::<{[[1, 2], [3, 4]]}>",
        );
        check("_RIC0KTj0_EE", "::<{(0usize,)}>", "::<{(0,)}>");
        check("_RIC0KTh1_b0_EE", "::<{(1u8, false)}>", "::<{(1, false)}>");
        check(
            "_RIC0KVNvINtNtC4core6option6OptionjE4NoneUE",
            "::<{core::option::Option::<usize>::None}>",
            "::<{core::option::Option::<usize>::None}>",
        );
        check(
            "_RIC0KVNvINtNtC4core6option6OptionjE4SomeTj0_EE",
            "::<{core::option::Option::<usize>::Some(0usize)}>",
            "::<{core::option::Option::<usize>::Some(0)}>",
        );
        check(
            "_RIC0KVNtC3foo3BarS1sRe616263_2chc78_5sliceRAh1_h2_h3_EEE",
            "::<{foo::Bar { s: \"abc\", ch: 'x', slice: &[1u8, 2u8, 3u8] }}>",
            "::<{foo::Bar { s: \"abc\", ch: 'x', slice: &[1, 2, 3] }}>",
        );
    }

    #[test]
    fn malformed_consts_are_rejected() {
        for symbol in [
            // `b` is only `false` or `true`.
            "_RIC0Kb2_E",
            // A char literal needs a valid scalar value.
            "_RIC0Kcd800_E",
            // A string literal needs whole byte pairs.
            "_RIC0Ke61626_E",
        ] {
            assert_eq!(parse(symbol).unwrap_err(), ParseError::Invalid, "{symbol}");
        }
    }

    /// A string literal is a byte string: bytes that are not UTF-8 are kept
    /// and escaped by the printer instead of rejecting the symbol.
    #[test]
    fn invalid_utf8_string_literals_are_escaped() {
        let ast = crate::rust::parse_bytes(b"_RIC0Kef09f_E").unwrap();
        assert_eq!(ast.to_string(), "::<{*\"\\xf0\\x9f\"}>");
        // Valid UTF-8 in a literal keeps printing verbatim.
        let ast = crate::rust::parse_bytes(b"_RIC0Ke61c3a9_E").unwrap();
        assert_eq!(ast.to_string(), "::<{*\"aé\"}>");
    }

    #[test]
    fn backrefs_share_nodes() {
        let symbol = "_RMC0TTTTTTpB8_EB7_EB6_EB5_EB4_EB3_E";
        let ast = parse(symbol).unwrap();
        check(
            symbol,
            "<((((((_, _), (_, _)), ((_, _), (_, _))), (((_, _), (_, _)), ((_, _), (_, _)))), \
             ((((_, _), (_, _)), ((_, _), (_, _))), (((_, _), (_, _)), ((_, _), (_, _))))), \
             (((((_, _), (_, _)), ((_, _), (_, _))), (((_, _), (_, _)), ((_, _), (_, _)))), \
             ((((_, _), (_, _)), ((_, _), (_, _))), (((_, _), (_, _)), ((_, _), (_, _))))))>",
            "<((((((_, _), (_, _)), ((_, _), (_, _))), (((_, _), (_, _)), ((_, _), (_, _)))), \
             ((((_, _), (_, _)), ((_, _), (_, _))), (((_, _), (_, _)), ((_, _), (_, _))))), \
             (((((_, _), (_, _)), ((_, _), (_, _))), (((_, _), (_, _)), ((_, _), (_, _)))), \
             ((((_, _), (_, _)), ((_, _), (_, _))), (((_, _), (_, _)), ((_, _), (_, _))))))>",
        );
        // The graph is a DAG: at least one node is reachable by two links.
        assert!(shared_links(&ast) > 0, "backrefs did not share nodes");
        // A symbol without backrefs shares nothing.
        assert_eq!(shared_links(&parse("_RMC0ThmE").unwrap()), 0);
    }

    /// Count child links that point at an already-visited node.
    fn shared_links(ast: &crate::rust::Ast<'_>) -> usize {
        let mut seen: Vec<*const crate::rust::Node<'_>> = Vec::new();
        let mut stack = vec![ast.root().clone()];
        let mut shared = 0;
        while let Some(node) = stack.pop() {
            let pointer = Rc::as_ptr(&node);
            if seen.contains(&pointer) {
                shared += 1;
                continue;
            }
            seen.push(pointer);
            stack.extend(node.children().iter().cloned());
        }
        shared
    }

    #[test]
    fn cached_path_reused_as_type_advances_cursor() {
        // Regression: a path cached at an offset could be re-entered as a type
        // without consuming input, stalling the generic-argument loop. This
        // real rustc 1.75.0 symbol exposed the missing end offset.
        let symbol = "_RNCINvNvXsg_NtNtNtCs2TW8dYrBCkv_4core4iter8adapters7flattenINtBa_13FlattenCompatppENtNtNtBe_6traits8iterator8Iterator8try_fold7flattenINtNtBg_6option8IntoIterINtNtBg_6result6ResultNtNtNtCs5FgMjSxpWcW_21rustc_trait_selection6traits6select18EvaluatedCandidateNtNtCseOYjLujkZGR_12rustc_middle6traits14SelectionErrorEEuINtNtNtBg_3ops12control_flow11ControlFlowIB54_B2S_EENCINvXBc_INtBc_12GenericShuntINtBa_7FlatMapINtNtBc_3map3MapINtNtNtCsgCZNw7TX6xD_5alloc3vec9into_iter8IntoIterNtNtB49_6select18SelectionCandidateENCNvMs0_B2U_NtB2U_16SelectionContext34candidate_from_obligation_no_cache0EINtB2b_6OptionB2w_ENvMs1_B2z_IB2x_IB9r_B2S_EB47_E9transposeEIB2x_NtNtBg_7convert10InfallibleB47_EEB1j_8try_folduNCINvNvB1j_12try_for_each4callB2S_B5I_NcNtB5I_5Break0E0B5I_E0E0B2Y_";
        assert!(parse(symbol).is_ok());
    }

    #[test]
    fn instantiating_crate_is_kept_but_not_printed() {
        let ast = parse("_RNCNCNgCs6DXkGYLi8lr_2cc5spawn00B5_").unwrap();
        let instantiating = ast.instantiating_crate().expect("instantiating crate");
        assert_eq!(instantiating.kind(), Kind::InstantiatingCrate);
        assert_eq!(instantiating.children().len(), 1);
        assert!(!ast.to_string().contains("cc::cc"));
    }

    #[test]
    fn depth_budget_matches_the_reference() {
        // Each `I` adds one level, and the innermost `C` path adds one more,
        // so 499 nested `I`s reach exactly `MAX_DEPTH`.
        let accepted = format!("_R{}C0{}", "I".repeat(499), "E".repeat(499));
        assert!(parse(&accepted).is_ok());
        let rejected = format!("_R{}C0{}", "I".repeat(500), "E".repeat(500));
        assert_eq!(parse(&rejected).unwrap_err(), ParseError::RecursedTooDeep);
        assert!(rustc_demangle::try_demangle(&accepted).is_ok());
        assert!(rustc_demangle::try_demangle(&rejected).is_err());
    }

    #[test]
    fn node_budget_is_enforced() {
        let symbol = format!("_RMC0T{}E", "p".repeat(crate::rust::MAX_NODES + 1));
        assert_eq!(parse(&symbol).unwrap_err(), ParseError::BudgetExceeded);
    }

    #[test]
    fn work_budget_is_enforced() {
        // `T TpE` completes a tuple at offset 1, then every `B0_` resolves to
        // it through the cache: each costs one work unit and no node.
        let mut symbol = String::from("_RMC0TTpE");
        for _ in 0..crate::rust::MAX_WORK {
            symbol.push_str("B3_");
        }
        symbol.push('E');
        assert_eq!(parse(&symbol).unwrap_err(), ParseError::BudgetExceeded);
    }

    #[test]
    fn malformed_symbols_are_rejected() {
        for symbol in [
            "_R",
            "_RZ",
            "_RB_",
            "_RC3fo",
            "_RC99foo",
            "_RC99999999999999999999foo",
            "_RMC0",
            "_RIC0h",
            "_RTp",
            "_RC3foo\u{e9}",
            "_RC3fo\u{0}o",
        ] {
            assert_eq!(parse(symbol).unwrap_err(), ParseError::Invalid, "{symbol}");
        }
    }

    #[test]
    fn forward_backrefs_are_rejected() {
        // A backref must point strictly before itself.
        assert_eq!(parse("_RTB3_E").unwrap_err(), ParseError::Invalid);
        assert_eq!(parse("_RB0_C3foo").unwrap_err(), ParseError::Invalid);
    }

    #[test]
    fn fn_pointers() {
        check("_RMC0FhEu", "<fn(u8)>", "<fn(u8)>");
        check("_RMC0FhEm", "<fn(u8) -> u32>", "<fn(u8) -> u32>");
        check("_RMC0FThmEEu", "<fn((u8, u32))>", "<fn((u8, u32))>");
        check("_RMC0FUhEu", "<unsafe fn(u8)>", "<unsafe fn(u8)>");
        check(
            "_RMC0FKChEu",
            "<extern \"C\" fn(u8)>",
            "<extern \"C\" fn(u8)>",
        );
        // ABI names re-join the `_` the encoding substituted for `-`.
        check(
            "_RMC0FK7foo_barhEu",
            "<extern \"foo-bar\" fn(u8)>",
            "<extern \"foo-bar\" fn(u8)>",
        );
        // A binder covers the whole signature, parameters and return type.
        check(
            "_RMC0FG_RL0_hEu",
            "<for<'a> fn(&'a u8)>",
            "<for<'a> fn(&'a u8)>",
        );
    }

    #[test]
    fn dyn_traits() {
        check("_RMC0DC3fooEL_", "<dyn foo>", "<dyn foo>");
        // The binder covers the traits, not the trailing lifetime.
        check("_RMC0DG_C3fooEL_", "<dyn for<'a> foo>", "<dyn for<'a> foo>");
        // An `I` path keeps its argument list open for the bindings.
        check(
            "_RMC0DNtC5krate5Traitp1NKj0_EL_",
            "<dyn krate::Trait<N = 0usize>>",
            "<dyn krate::Trait<N = 0>>",
        );
        check(
            "_RINbNbCskIICzLVDPPb_5alloc5alloc8box_freeDINbNiB4_5boxed5FnBoxuEp6OutputuEL_\
             ECs1iopQbuBiw2_3std",
            "alloc[f15a878b47eb696b]::alloc::box_free::<dyn alloc[f15a878b47eb696b]::boxed::\
             FnBox<(), Output = ()>>",
            "alloc::alloc::box_free::<dyn alloc::boxed::FnBox<(), Output = ()>>",
        );
    }

    #[test]
    fn patterns() {
        check("_RMC0WmN", "<u32 is !null>", "<u32 is !null>");
        check("_RMC0WmRm1_m9_", "<u32 is 1u32..=9u32>", "<u32 is 1..=9>");
        check(
            "_RMC0WmORm1_m2_Rm5_m6_E",
            "<u32 is 1u32..=2u32 | 5u32..=6u32>",
            "<u32 is 1..=2 | 5..=6>",
        );
    }

    #[test]
    fn punycode_identifiers() {
        check(
            "_RNqCs4fqI2P2rA04_11utf8_identsu30____7hkackfecea1cbdathfdh9hlq6y",
            "utf8_idents[317d481089b8c8fe]::საჭმელად_გემრიელი_სადილი",
            "utf8_idents::საჭმელად_გემრიელი_სადილი",
        );
        // An undecodable punycode part keeps the reference's fallback text.
        check("_RCu3_a_Z", "punycode{a-Z}", "punycode{a-Z}");
        // An empty punycode part is not an identifier.
        assert_eq!(parse("_RCu2_a_").unwrap_err(), ParseError::Invalid);
    }

    #[test]
    fn splat_marker() {
        // The unstable `w` marker prints the pinned reference's spelling; the
        // locked 0.1.28 backend prints `#[splat] `, which `check_oracle`
        // normalizes as the documented divergence.
        check(
            "_RMC0wThmE",
            "<#[rustc_splat] (u8, u32)>",
            "<#[rustc_splat] (u8, u32)>",
        );
        check(
            "_RMC0wPFhEu",
            "<#[rustc_splat] *const fn(u8)>",
            "<#[rustc_splat] *const fn(u8)>",
        );
        // Only one marker may prefix a type.
        assert_eq!(parse("_RMC0wwh").unwrap_err(), ParseError::Invalid);
        // The marker is only valid in type position, not before a root path.
        assert_eq!(parse("_RwC3foo").unwrap_err(), ParseError::Invalid);
    }

    #[test]
    fn malformed_fn_and_dyn_symbols_are_rejected() {
        for symbol in [
            // The return type is mandatory.
            "_RMC0FhE",
            // An ABI name must be a plain ASCII identifier.
            "_RMC0FKu3_a_ZhEu",
            "_RMC0FK0hEu",
            // A `dyn` bound list needs its `E` and its trailing lifetime.
            "_RMC0DC3foo",
            "_RMC0DC3fooE",
            // A lifetime index above the binder depth is not printable.
            "_RMC0FG_RL1_hEu",
        ] {
            assert_eq!(parse(symbol).unwrap_err(), ParseError::Invalid, "{symbol}");
        }
    }

    #[test]
    fn spans_point_into_the_symbol() {
        let symbol = "_RMC0ThmE";
        let ast = parse(symbol).unwrap();
        let tuple = ast.root().children().get(1).unwrap();
        let span = tuple.span().unwrap();
        assert_eq!(
            symbol.get(span.start as usize..span.end as usize),
            Some("ThmE")
        );
        let first = tuple.children().first().unwrap();
        let span = first.span().unwrap();
        assert_eq!(
            symbol.get(span.start as usize..span.end as usize),
            Some("h")
        );
        assert_eq!(first.text(), Some(&b"u8"[..]));
    }

    #[test]
    fn text_borrows_the_input() {
        let symbol = "_RC3foo";
        let ast = parse(symbol).unwrap();
        assert!(matches!(
            ast.root().text(),
            Some(text) if text == b"foo"
        ));
    }

    /// Identifier bytes are byte offsets, so a length may cut a UTF-8 sequence
    /// in half and the printer escapes the bytes instead of rejecting them.
    #[test]
    fn invalid_utf8_identifiers_are_escaped() {
        let ast = crate::rust::parse_bytes(b"_RC3f\xffo").unwrap();
        assert_eq!(ast.root().text(), Some(&b"f\xffo"[..]));
        assert_eq!(ast.to_string(), "f\\xffo");

        // A length that splits a two-byte character is legal at this layer.
        let ast = crate::rust::parse_bytes(b"_RC1\xc3").unwrap();
        assert_eq!(ast.root().text(), Some(&b"\xc3"[..]));
        assert_eq!(ast.to_string(), "\\xc3");

        // Valid UTF-8 keeps printing verbatim.
        let ast = crate::rust::parse_bytes("_RC3fé".as_bytes()).unwrap();
        assert_eq!(ast.to_string(), "fé");
    }

    /// A backref is a byte offset: a one-byte shift changes the target.
    #[test]
    fn backrefs_are_byte_offsets() {
        // `B4_` resolves to the `h` at encoding offset 5 and shares its node.
        let ast = parse("_RMC0TThB4_EE").unwrap();
        assert_eq!(ast.to_string(), "<((u8, u8),)>");
        // Offsetting the same reference by one byte no longer reaches it.
        assert_eq!(parse("_RMC0TThB5_EE").unwrap_err(), ParseError::Invalid);
        // A backref must point strictly before itself.
        assert_eq!(parse("_RB1_C3foo").unwrap_err(), ParseError::Invalid);
    }

    /// Truncating a valid v0 symbol never panics, and a truncated length or
    /// backref is rejected rather than read past the end.
    #[test]
    fn truncations_are_rejected() {
        for symbol in [
            &b"_RMC0TTTTTTpB8_EB7_EB6_EB5_EB4_EB3_E"[..],
            b"_RNvCs1234_4test4main",
            b"_RIC0KRe616263_E",
        ] {
            for end in 0..symbol.len() {
                let _ = crate::rust::parse_bytes(&symbol[..end]);
            }
            assert_eq!(
                crate::rust::parse_bytes(&symbol[..symbol.len() - 1]).unwrap_err(),
                ParseError::Invalid,
                "{symbol:?}"
            );
        }
    }

    #[test]
    fn fn_dyn_and_pattern_nodes_are_inspectable() {
        let ast = parse("_RMC0FG_RL0_hEm").unwrap();
        let fn_node = ast.root().children().get(1).expect("fn type");
        assert_eq!(fn_node.kind(), Kind::Fn);
        let NodeData::Function {
            binder,
            is_unsafe,
            abi,
            ret_elided,
        } = fn_node.data()
        else {
            panic!("fn node without a function payload");
        };
        assert_eq!(*binder, 1);
        assert!(!*is_unsafe);
        assert!(abi.is_none());
        assert!(!*ret_elided);
        let children = fn_node.children();
        assert_eq!(children.len(), 2, "one parameter and the return type");
        assert_eq!(children[0].kind(), Kind::Ref);
        assert_eq!(children[1].kind(), Kind::BasicType);
        // A bare `u` return keeps its node but is flagged as elided.
        let ast = parse("_RMC0FhEu").unwrap();
        let fn_node = ast.root().children().get(1).expect("fn type");
        assert!(matches!(
            fn_node.data(),
            NodeData::Function {
                ret_elided: true,
                ..
            }
        ));
        assert_eq!(fn_node.children().len(), 2);

        let ast = parse("_RMC0DG_C3fooEL_").unwrap();
        let dyn_node = ast.root().children().get(1).expect("dyn type");
        assert_eq!(dyn_node.kind(), Kind::Dyn);
        assert_eq!(dyn_node.data(), &NodeData::Binder(1));
        let children = dyn_node.children();
        assert_eq!(children.len(), 2, "one trait and the trailing lifetime");
        assert_eq!(children[0].kind(), Kind::DynTrait);
        assert_eq!(children[1].kind(), Kind::Lifetime);

        let ast = parse("_RMC0WmRm1_m9_").unwrap();
        let pattern_type = ast.root().children().get(1).expect("pattern type");
        assert_eq!(pattern_type.kind(), Kind::PatternType);
        let pattern = pattern_type.children().get(1).expect("pattern");
        assert_eq!(pattern.data(), &NodeData::Pattern(PatternKind::Range));
        assert_eq!(pattern.children().len(), 2);
    }

    #[test]
    fn ast_is_inspectable() {
        let ast = parse("_RIC0hKj4_E").unwrap();
        let root = ast.root();
        assert_eq!(root.kind(), Kind::GenericArgs);
        assert_eq!(root.children().len(), 3);
        assert_eq!(root.children()[0].kind(), Kind::CrateRoot);
        assert_eq!(root.children()[1].kind(), Kind::BasicType);
        assert_eq!(root.children()[2].kind(), Kind::Const);
        let value = root.children()[2].children()[1].data().clone();
        assert!(matches!(
            value,
            NodeData::Const(ConstValueKind::Integer { negative: false })
        ));
    }
}
