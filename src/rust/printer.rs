//! Printer: renders an [`Ast`] into any [`core::fmt::Write`] sink.
//!
//! The output is byte-compatible with `rustc-demangle`, which — not RFC 2603 —
//! defines the demangled text: `{:#}` omits the crate-root hash, the legacy
//! trailing hash and integer const type suffixes, and nothing else.
//!
//! Printing is bounded by [`MAX_PRINT_DEPTH`], [`MAX_PRINT_VISITS`] and
//! [`MAX_OUTPUT_BYTES`]. Shared nodes are re-visited on every reference, so the
//! visit budget — not the node count — bounds DAG traversal.

use core::fmt;

use crate::parse::{Escape, write_escaped};

use super::node::{
    AdtConstKind, ConstValueKind, ImplKind, Kind, MAX_OUTPUT_BYTES, MAX_PRINT_DEPTH,
    MAX_PRINT_VISITS, Namespace, NodeData, NodeRef, PatternKind,
};
use super::{Ast, ParseError};

/// Written when the output budget is exhausted, like the reference adapter.
pub(super) const SIZE_LIMIT: &str = "{size limit reached}";

/// How a completed print run ended.
pub(super) enum Outcome {
    /// Everything was written.
    Printed,
    /// The output budget ran out; the caller writes [`SIZE_LIMIT`].
    SizeLimit,
}

/// Render `ast` into `out`.
///
/// Returns [`Outcome::SizeLimit`] after writing everything that fit in
/// [`MAX_OUTPUT_BYTES`]; every other failure is a `fmt::Error` from the sink or
/// an exceeded print budget.
pub(super) fn print<W: fmt::Write + ?Sized>(
    ast: &Ast<'_>,
    out: &mut W,
    alternate: bool,
) -> Result<Outcome, fmt::Error> {
    let mut printer = Printer {
        out,
        alternate,
        depth: 0,
        visits: 0,
        remaining: MAX_OUTPUT_BYTES,
        size_limit: false,
        bound_lifetime_depth: 0,
    };
    let result = printer
        .path(ast.root(), true)
        .and_then(|()| printer.write_text(ast.suffix().as_bytes(), Escape::Verbatim));
    match result {
        Ok(()) => Ok(Outcome::Printed),
        Err(_) if printer.size_limit => Ok(Outcome::SizeLimit),
        Err(error) => Err(error),
    }
}

/// Check that `ast` can be printed, so a retained [`Ast`] never fails later.
///
/// A tree that only trips the output budget is still printable, so it is
/// accepted here.
pub(super) fn validate(ast: &Ast<'_>) -> Result<(), ParseError> {
    match print(ast, &mut NullSink, false) {
        Ok(_) => Ok(()),
        Err(_) => Err(ParseError::Invalid),
    }
}

/// A sink that discards everything, used to validate a tree.
struct NullSink;

impl fmt::Write for NullSink {
    fn write_str(&mut self, _text: &str) -> fmt::Result {
        Ok(())
    }
}

/// A decimal or lowercase-hex rendering of a `u64`, without allocation.
struct NumBuf {
    buf: [u8; 20],
    start: usize,
}

impl NumBuf {
    /// Render `value` in base 10.
    fn decimal(mut value: u64) -> Self {
        let mut buf = [0_u8; 20];
        let mut index = buf.len();
        loop {
            index -= 1;
            // `value % 10` is always a valid ASCII digit.
            buf[index] = b'0' + u8::try_from(value % 10).unwrap_or(0);
            value /= 10;
            if value == 0 {
                break;
            }
        }
        Self { buf, start: index }
    }

    /// Render `value` in lowercase hexadecimal.
    fn hex(mut value: u64) -> Self {
        let mut buf = [0_u8; 20];
        let mut index = buf.len();
        loop {
            index -= 1;
            let nibble = u8::try_from(value & 0xf).unwrap_or(0);
            buf[index] = if nibble < 10 {
                b'0' + nibble
            } else {
                b'a' + nibble - 10
            };
            value >>= 4;
            if value == 0 {
                break;
            }
        }
        Self { buf, start: index }
    }

    /// The rendered digits.
    fn as_str(&self) -> &str {
        core::str::from_utf8(&self.buf[self.start..]).unwrap_or("")
    }
}

/// One pass over an [`Ast`], writing text into the sink.
struct Printer<'w, W: fmt::Write + ?Sized> {
    out: &'w mut W,
    alternate: bool,
    depth: u32,
    visits: usize,
    remaining: usize,
    size_limit: bool,
    /// Late-bound lifetimes introduced by the `for<...>` binders around the
    /// node being printed; lifetime indices resolve against it.
    bound_lifetime_depth: u32,
}

impl<W: fmt::Write + ?Sized> Printer<'_, W> {
    /// Write `text`, respecting the output budget.
    ///
    /// Exhausting the budget aborts the traversal; [`print`] turns that into
    /// [`Outcome::SizeLimit`] so the caller can append [`SIZE_LIMIT`], exactly
    /// like the reference's size-limited adapter.
    fn write(&mut self, text: &str) -> fmt::Result {
        let Some(remaining) = self.remaining.checked_sub(text.len()) else {
            self.size_limit = true;
            return Err(fmt::Error);
        };
        self.remaining = remaining;
        self.out.write_str(text)
    }

    /// Write a number, respecting the output budget.
    fn write_num(&mut self, number: NumBuf) -> fmt::Result {
        self.write(number.as_str())
    }

    /// Write node text bytes, respecting the output budget.
    ///
    /// The bytes are rendered by the shared escaper, which charges the budget
    /// through [`BudgetSink`]: identifier text escapes `\` and non-UTF-8 bytes
    /// as `\xNN`, compiler payloads escape only non-UTF-8 bytes.
    fn write_text(&mut self, bytes: &[u8], policy: Escape) -> fmt::Result {
        let mut sink = BudgetSink { printer: self };
        write_escaped(&mut sink, bytes, policy)
    }

    /// Charge one node visit and enter a nesting level.
    fn enter(&mut self) -> fmt::Result {
        self.visits = self.visits.saturating_add(1);
        if self.visits > MAX_PRINT_VISITS {
            return Err(fmt::Error);
        }
        self.depth = self.depth.saturating_add(1);
        if self.depth > MAX_PRINT_DEPTH {
            return Err(fmt::Error);
        }
        Ok(())
    }

    /// Leave a nesting level.
    fn exit(&mut self) {
        self.depth = self.depth.saturating_sub(1);
    }

    /// Print a `path`. `in_value` adds the extra `::` of an `I` path in value
    /// position, exactly as the reference does.
    fn path(&mut self, node: &NodeRef<'_>, in_value: bool) -> fmt::Result {
        self.enter()?;
        let result = self.path_body(node, in_value);
        self.exit();
        result
    }

    /// The body of [`Printer::path`].
    fn path_body(&mut self, node: &NodeRef<'_>, in_value: bool) -> fmt::Result {
        match node.kind() {
            Kind::CrateRoot => {
                if let Some(text) = node.text() {
                    self.write_text(text, Escape::Identifier)?;
                }
                // The disambiguator is only printed in `Display` mode.
                if let Some(disambiguator) = node.disambiguator()
                    && disambiguator != 0
                    && !self.alternate
                {
                    self.write("[")?;
                    self.write_num(NumBuf::hex(disambiguator))?;
                    self.write("]")?;
                }
                Ok(())
            }
            Kind::Nested => {
                let children = node.children();
                let (Some(parent), Some(ident)) = (children.first(), children.get(1)) else {
                    return Err(fmt::Error);
                };
                self.path(parent, in_value)?;
                let Some(namespace) = node.namespace() else {
                    return Err(fmt::Error);
                };
                let name = ident.text().filter(|text| !text.is_empty());
                if namespace == Namespace::Internal {
                    if let Some(name) = name {
                        self.write("::")?;
                        self.write_text(name, Escape::Identifier)?;
                    }
                    return Ok(());
                }
                self.write("::{")?;
                match namespace {
                    Namespace::Closure => self.write("closure")?,
                    Namespace::Shim => self.write("shim")?,
                    Namespace::Other(tag) => {
                        let mut buf = [0_u8; 4];
                        self.write(tag.encode_utf8(&mut buf))?;
                    }
                    Namespace::Internal => return Err(fmt::Error),
                }
                if let Some(name) = name {
                    self.write(":")?;
                    self.write_text(name, Escape::Identifier)?;
                }
                self.write("#")?;
                let disambiguator = ident.disambiguator().unwrap_or(0);
                self.write_num(NumBuf::decimal(disambiguator))?;
                self.write("}")
            }
            Kind::Impl => self.impl_(node),
            Kind::GenericArgs => {
                let children = node.children();
                let Some(base) = children.first() else {
                    return Err(fmt::Error);
                };
                self.path(base, in_value)?;
                if in_value {
                    self.write("::")?;
                }
                self.write("<")?;
                let args = children.get(1..).unwrap_or(&[]);
                self.sep_list(args, ", ", Self::generic_arg)?;
                self.write(">")
            }
            // Parsed for validation only; never printed.
            Kind::InstantiatingCrate => Ok(()),
            Kind::LegacyPath => {
                let children = node.children();
                let count = children.len();
                for (index, element) in children.iter().enumerate() {
                    // `{:#}` omits a trailing rustc hash.
                    if self.alternate
                        && index + 1 == count
                        && matches!(element.data(), NodeData::Hash(true))
                    {
                        break;
                    }
                    if index != 0 {
                        self.write("::")?;
                    }
                    self.write_text(element.text().unwrap_or_default(), Escape::Identifier)?;
                }
                Ok(())
            }
            _ => Err(fmt::Error),
        }
    }

    /// Print an `M`/`X`/`Y` impl, dropping the impl's own path.
    fn impl_(&mut self, node: &NodeRef<'_>) -> fmt::Result {
        let children = node.children();
        let NodeData::Impl(kind) = node.data() else {
            return Err(fmt::Error);
        };
        let (ty, trait_path) = match kind {
            ImplKind::Inherent => (children.get(1), None),
            ImplKind::TraitImpl => (children.get(1), children.get(2)),
            ImplKind::TraitDef => (children.first(), children.get(1)),
        };
        let Some(ty) = ty else {
            return Err(fmt::Error);
        };
        self.write("<")?;
        self.ty(ty)?;
        if *kind != ImplKind::Inherent {
            let Some(trait_path) = trait_path else {
                return Err(fmt::Error);
            };
            self.write(" as ")?;
            self.path(trait_path, false)?;
        }
        self.write(">")
    }

    /// Print a `generic-arg`.
    fn generic_arg(&mut self, node: &NodeRef<'_>) -> fmt::Result {
        match node.kind() {
            Kind::Lifetime => self.lifetime(node),
            Kind::Const => self.const_(node, false),
            _ => self.ty(node),
        }
    }

    /// Print a `type`.
    fn ty(&mut self, node: &NodeRef<'_>) -> fmt::Result {
        self.enter()?;
        let result = self.ty_body(node);
        self.exit();
        result
    }

    /// The body of [`Printer::ty`].
    fn ty_body(&mut self, node: &NodeRef<'_>) -> fmt::Result {
        let children = node.children();
        match node.kind() {
            Kind::BasicType => self.write_text(node.text().unwrap_or_default(), Escape::Identifier),
            Kind::Ref => {
                self.write("&")?;
                let pointee = match children.split_first() {
                    Some((first, rest)) if first.kind() == Kind::Lifetime => {
                        // Index 0 is `'_`, which the reference omits entirely.
                        if first.lifetime_index().unwrap_or(0) != 0 {
                            self.lifetime(first)?;
                            self.write(" ")?;
                        }
                        rest
                    }
                    _ => children,
                };
                if matches!(node.data(), NodeData::Mutability(true)) {
                    self.write("mut ")?;
                }
                let Some(pointee) = pointee.first() else {
                    return Err(fmt::Error);
                };
                self.ty(pointee)
            }
            Kind::RawPtr => {
                let Some(pointee) = children.first() else {
                    return Err(fmt::Error);
                };
                self.write("*")?;
                if matches!(node.data(), NodeData::Mutability(true)) {
                    self.write("mut ")?;
                } else {
                    self.write("const ")?;
                }
                self.ty(pointee)
            }
            Kind::Array => {
                let (Some(element), Some(len)) = (children.first(), children.get(1)) else {
                    return Err(fmt::Error);
                };
                self.write("[")?;
                self.ty(element)?;
                self.write("; ")?;
                self.const_(len, true)?;
                self.write("]")
            }
            Kind::Slice => {
                let Some(element) = children.first() else {
                    return Err(fmt::Error);
                };
                self.write("[")?;
                self.ty(element)?;
                self.write("]")
            }
            Kind::Tuple => {
                self.write("(")?;
                self.sep_list(children, ", ", Self::ty)?;
                if children.len() == 1 {
                    self.write(",")?;
                }
                self.write(")")
            }
            Kind::Fn => self.fn_(node),
            Kind::Dyn => self.dyn_(node),
            Kind::PatternType => {
                let (Some(ty), Some(pattern)) = (children.first(), children.get(1)) else {
                    return Err(fmt::Error);
                };
                self.ty(ty)?;
                self.write(" is ")?;
                self.pattern(pattern)
            }
            Kind::Splat => {
                let Some(ty) = children.first() else {
                    return Err(fmt::Error);
                };
                self.write("#[rustc_splat] ")?;
                self.ty(ty)
            }
            // A path in type position is never in value position.
            Kind::CrateRoot | Kind::Nested | Kind::Impl | Kind::GenericArgs => {
                self.path(node, false)
            }
            _ => Err(fmt::Error),
        }
    }

    /// Print a `const`.
    ///
    /// `in_value` marks a const nested inside another const expression, where
    /// compound values print without the braces they need in generic argument
    /// position.
    fn const_(&mut self, node: &NodeRef<'_>, in_value: bool) -> fmt::Result {
        self.enter()?;
        let result = self.const_body(node, in_value);
        self.exit();
        result
    }

    /// The body of [`Printer::const_`].
    fn const_body(&mut self, node: &NodeRef<'_>, in_value: bool) -> fmt::Result {
        let children = node.children();
        let Some(value) = children.last() else {
            return Err(fmt::Error);
        };
        let NodeData::Const(kind) = value.data() else {
            return Err(fmt::Error);
        };
        let braced = !in_value
            && matches!(
                kind,
                ConstValueKind::Str
                    | ConstValueKind::Ref { .. }
                    | ConstValueKind::Array
                    | ConstValueKind::Tuple
                    | ConstValueKind::Adt(_)
            );
        if braced {
            self.write("{")?;
        }
        match kind {
            ConstValueKind::Placeholder => self.write("_")?,
            ConstValueKind::Integer { negative } => {
                let Some(ty) = children.first().filter(|_| children.len() == 2) else {
                    return Err(fmt::Error);
                };
                if *negative {
                    self.write("-")?;
                }
                let nibbles = value.text().unwrap_or_default();
                let trimmed = match nibbles.iter().position(|&byte| byte != b'0') {
                    Some(index) => &nibbles[index..],
                    None => &[],
                };
                // Values above `u64` are printed verbatim in hex.
                if trimmed.len() > 16 {
                    self.write("0x")?;
                    self.write_text(nibbles, Escape::Identifier)?;
                } else {
                    let number = trimmed.iter().fold(0_u64, |acc, &byte| {
                        let digit = hex_digit(byte).unwrap_or(0);
                        (acc << 4) | u64::from(digit)
                    });
                    self.write_num(NumBuf::decimal(number))?;
                }
                if !self.alternate {
                    self.write_text(ty.text().unwrap_or_default(), Escape::Identifier)?;
                }
            }
            ConstValueKind::Bool(flag) => self.write(if *flag { "true" } else { "false" })?,
            ConstValueKind::Char => self.quoted('\'', value.text().unwrap_or_default())?,
            ConstValueKind::Str => {
                self.write("*")?;
                self.quoted('"', value.text().unwrap_or_default())?;
            }
            ConstValueKind::StrRef => self.quoted('"', value.text().unwrap_or_default())?,
            ConstValueKind::Ref { mutability } => {
                self.write("&")?;
                if *mutability {
                    self.write("mut ")?;
                }
                let Some(inner) = value.children().first() else {
                    return Err(fmt::Error);
                };
                self.const_(inner, true)?;
            }
            ConstValueKind::Array => {
                self.write("[")?;
                self.sep_list(value.children(), ", ", |this, node| this.const_(node, true))?;
                self.write("]")?;
            }
            ConstValueKind::Tuple => {
                self.write("(")?;
                self.sep_list(value.children(), ", ", |this, node| this.const_(node, true))?;
                if value.children().len() == 1 {
                    self.write(",")?;
                }
                self.write(")")?;
            }
            ConstValueKind::Adt(kind) => self.adt(value, *kind)?,
        }
        if braced {
            self.write("}")?;
        }
        Ok(())
    }

    /// Print a `V` ADT const: its path, then the fields of a tuple or struct.
    fn adt(&mut self, value: &NodeRef<'_>, kind: AdtConstKind) -> fmt::Result {
        let Some((path, fields)) = value.children().split_first() else {
            return Err(fmt::Error);
        };
        // The ADT's path is a value, so an `I` path prints `::<...>`.
        self.path(path, true)?;
        match kind {
            AdtConstKind::Unit => Ok(()),
            AdtConstKind::Tuple => {
                self.write("(")?;
                self.sep_list(fields, ", ", |this, node| this.const_(node, true))?;
                self.write(")")
            }
            AdtConstKind::Struct => {
                self.write(" { ")?;
                self.sep_list(fields, ", ", Self::adt_field)?;
                self.write(" }")
            }
        }
    }

    /// Print one struct field of an ADT const, `name: value`.
    fn adt_field(&mut self, node: &NodeRef<'_>) -> fmt::Result {
        let children = node.children();
        let (Some(name), Some(value)) = (children.first(), children.get(1)) else {
            return Err(fmt::Error);
        };
        self.write_text(name.text().unwrap_or_default(), Escape::Identifier)?;
        self.write(": ")?;
        self.const_(value, true)
    }

    /// Print `bytes` inside `quote`, escaped like the reference does.
    ///
    /// Valid UTF-8 is escaped character by character exactly as the reference
    /// does; bytes that are not UTF-8 are written as `\xNN`.
    fn quoted(&mut self, quote: char, bytes: &[u8]) -> fmt::Result {
        let mut buf = [0_u8; 4];
        self.write(quote.encode_utf8(&mut buf))?;
        for chunk in bytes.utf8_chunks() {
            for c in chunk.valid().chars() {
                // A quote is not escaped inside the other kind of quote.
                if matches!((quote, c), ('\'', '"') | ('"', '\'')) {
                    self.write(c.encode_utf8(&mut buf))?;
                    continue;
                }
                for escaped in c.escape_debug() {
                    self.write(escaped.encode_utf8(&mut buf))?;
                }
            }
            self.write_text(chunk.invalid(), Escape::Verbatim)?;
        }
        self.write(quote.encode_utf8(&mut buf))
    }

    /// Print a `fn` pointer, eliding the ` -> ()` of a unit return type.
    fn fn_(&mut self, node: &NodeRef<'_>) -> fmt::Result {
        let NodeData::Function {
            binder,
            is_unsafe,
            abi,
            ret_elided,
        } = node.data()
        else {
            return Err(fmt::Error);
        };
        let Some((ret, params)) = node.children().split_last() else {
            return Err(fmt::Error);
        };
        self.with_binder(*binder, |this| {
            if *is_unsafe {
                this.write("unsafe ")?;
            }
            if let Some(abi) = abi {
                this.write("extern \"")?;
                // The encoding replaced `-` with `_`, so re-join the parts.
                let mut parts = abi.split(|&byte| byte == b'_');
                this.write_text(parts.next().unwrap_or_default(), Escape::Identifier)?;
                for part in parts {
                    this.write("-")?;
                    this.write_text(part, Escape::Identifier)?;
                }
                this.write("\" ")?;
            }
            this.write("fn(")?;
            this.sep_list(params, ", ", Self::ty)?;
            this.write(")")?;
            if !*ret_elided {
                this.write(" -> ")?;
                this.ty(ret)?;
            }
            Ok(())
        })
    }

    /// Print a `dyn` trait object, `dyn <trait> + ... + 'lt`.
    fn dyn_(&mut self, node: &NodeRef<'_>) -> fmt::Result {
        let NodeData::Binder(binder) = node.data() else {
            return Err(fmt::Error);
        };
        let Some((trailing, traits)) = node.children().split_last() else {
            return Err(fmt::Error);
        };
        self.write("dyn ")?;
        self.with_binder(*binder, |this| {
            this.sep_list(traits, " + ", Self::dyn_trait)
        })?;
        // The trailing lifetime is outside the binder, and `'_` is omitted.
        if trailing.lifetime_index().unwrap_or(0) != 0 {
            self.write(" + ")?;
            self.lifetime(trailing)?;
        }
        Ok(())
    }

    /// Print one trait of a `dyn` trait object, with its associated bindings
    /// inside the trait's `<...>`.
    fn dyn_trait(&mut self, node: &NodeRef<'_>) -> fmt::Result {
        self.enter()?;
        let result = self.dyn_trait_body(node);
        self.exit();
        result
    }

    /// The body of [`Printer::dyn_trait`].
    fn dyn_trait_body(&mut self, node: &NodeRef<'_>) -> fmt::Result {
        let Some((path, bindings)) = node.children().split_first() else {
            return Err(fmt::Error);
        };
        // An `I` path keeps its argument list open for the bindings.
        let mut open = path.kind() == Kind::GenericArgs;
        if open {
            let Some((base, args)) = path.children().split_first() else {
                return Err(fmt::Error);
            };
            self.path(base, false)?;
            self.write("<")?;
            self.sep_list(args, ", ", Self::generic_arg)?;
        } else {
            self.path(path, false)?;
        }
        for binding in bindings {
            if open {
                self.write(", ")?;
            } else {
                self.write("<")?;
                open = true;
            }
            self.binding(binding)?;
        }
        if open {
            self.write(">")?;
        }
        Ok(())
    }

    /// Print one `dyn` associated binding, `name = value`.
    fn binding(&mut self, node: &NodeRef<'_>) -> fmt::Result {
        let children = node.children();
        let (Some(name), Some(value)) = (children.first(), children.get(1)) else {
            return Err(fmt::Error);
        };
        self.write_text(name.text().unwrap_or_default(), Escape::Identifier)?;
        self.write(" = ")?;
        if value.kind() == Kind::Const {
            self.const_(value, false)
        } else {
            self.ty(value)
        }
    }

    /// Print a `pattern`.
    fn pattern(&mut self, node: &NodeRef<'_>) -> fmt::Result {
        self.enter()?;
        let result = self.pattern_body(node);
        self.exit();
        result
    }

    /// The body of [`Printer::pattern`].
    fn pattern_body(&mut self, node: &NodeRef<'_>) -> fmt::Result {
        let NodeData::Pattern(kind) = node.data() else {
            return Err(fmt::Error);
        };
        match kind {
            PatternKind::Range => {
                let children = node.children();
                let (Some(low), Some(high)) = (children.first(), children.get(1)) else {
                    return Err(fmt::Error);
                };
                self.const_(low, false)?;
                self.write("..=")?;
                self.const_(high, false)
            }
            PatternKind::Or => self.sep_list(node.children(), " | ", Self::pattern),
            PatternKind::NeverNull => self.write("!null"),
        }
    }

    /// Print a `lifetime`, resolving its index against the binder depth.
    fn lifetime(&mut self, node: &NodeRef<'_>) -> fmt::Result {
        let Some(index) = node.lifetime_index() else {
            return Err(fmt::Error);
        };
        self.print_lifetime_from_index(index)
    }

    /// Print the lifetime an index refers to: `'_` for zero, then `'a`..`'z`
    /// and `'_123` counting back through the enclosing binders.
    fn print_lifetime_from_index(&mut self, index: u64) -> fmt::Result {
        self.write("'")?;
        if index == 0 {
            return self.write("_");
        }
        let Some(depth) = u64::from(self.bound_lifetime_depth).checked_sub(index) else {
            return Err(fmt::Error);
        };
        if depth < 26 {
            let letter = char::from(b'a' + u8::try_from(depth).unwrap_or(0));
            let mut buf = [0_u8; 4];
            self.write(letter.encode_utf8(&mut buf))
        } else {
            self.write("_")?;
            self.write_num(NumBuf::decimal(depth))
        }
    }

    /// Print an optional `for<'a, 'b> ` binder, then `body` with the bound
    /// lifetimes in scope.
    fn with_binder(
        &mut self,
        bound: u64,
        body: impl FnOnce(&mut Self) -> fmt::Result,
    ) -> fmt::Result {
        let outer = self.bound_lifetime_depth;
        if bound > 0 {
            self.write("for<")?;
            for index in 0..bound {
                if index > 0 {
                    self.write(", ")?;
                }
                self.bound_lifetime_depth = self.bound_lifetime_depth.saturating_add(1);
                self.print_lifetime_from_index(1)?;
            }
            self.write("> ")?;
        }
        let result = body(self);
        self.bound_lifetime_depth = outer;
        result
    }

    /// Print `nodes` separated by `sep`.
    fn sep_list(
        &mut self,
        nodes: &[NodeRef<'_>],
        sep: &str,
        mut print: impl FnMut(&mut Self, &NodeRef<'_>) -> fmt::Result,
    ) -> fmt::Result {
        for (index, node) in nodes.iter().enumerate() {
            if index > 0 {
                self.write(sep)?;
            }
            print(self, node)?;
        }
        Ok(())
    }
}

/// The value of one lowercase hex nibble.
const fn hex_digit(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        _ => None,
    }
}

/// Adapts the printer's budgeted writer to [`core::fmt::Write`], so the shared
/// escaper renders node text through the print budget.
struct BudgetSink<'p, 'w, W: fmt::Write + ?Sized> {
    printer: &'p mut Printer<'w, W>,
}

impl<W: fmt::Write + ?Sized> fmt::Write for BudgetSink<'_, '_, W> {
    fn write_str(&mut self, text: &str) -> fmt::Result {
        self.printer.write(text)
    }
}

#[cfg(test)]
mod tests {
    use alloc::format;
    use alloc::string::{String, ToString};
    use alloc::vec;
    use alloc::vec::Vec;

    use super::*;
    use crate::rust::parse;

    #[test]
    fn alternate_mode_changes_three_things() {
        // Crate-root hash.
        assert_eq!(parse("_RCs_3foo").unwrap().to_string(), "foo[1]");
        assert_eq!(format!("{:#}", parse("_RCs_3foo").unwrap()), "foo");
        // Legacy trailing hash.
        let legacy = parse("_ZN3foo17h05af221e174051e9E").unwrap();
        assert_eq!(legacy.to_string(), "foo::h05af221e174051e9");
        assert_eq!(format!("{legacy:#}"), "foo");
        // Integer const type suffix.
        let konst = parse("_RIC0Kanb_E").unwrap();
        assert_eq!(konst.to_string(), "::<-11i8>");
        assert_eq!(format!("{konst:#}"), "::<-11>");
    }

    #[test]
    fn suffix_is_appended_in_both_modes() {
        let ast = parse("_RNvNtC4rand4fork23FORK_HANDLER_REGISTERED.0.0").unwrap();
        assert_eq!(ast.suffix(), ".0.0");
        assert_eq!(ast.to_string(), "rand::fork::FORK_HANDLER_REGISTERED.0.0");
        assert_eq!(
            format!("{ast:#}"),
            "rand::fork::FORK_HANDLER_REGISTERED.0.0"
        );
    }

    #[test]
    fn num_buf_renders_decimal_and_hex() {
        assert_eq!(NumBuf::decimal(0).as_str(), "0");
        assert_eq!(NumBuf::decimal(1235).as_str(), "1235");
        assert_eq!(NumBuf::decimal(u64::MAX).as_str(), "18446744073709551615");
        assert_eq!(NumBuf::hex(0).as_str(), "0");
        assert_eq!(NumBuf::hex(0x4d3).as_str(), "4d3");
        assert_eq!(NumBuf::hex(u64::MAX).as_str(), "ffffffffffffffff");
    }

    #[test]
    fn hex_digits_are_lowercase_only() {
        assert_eq!(hex_digit(b'0'), Some(0));
        assert_eq!(hex_digit(b'f'), Some(15));
        assert_eq!(hex_digit(b'A'), None);
        assert_eq!(hex_digit(b'g'), None);
    }

    /// Identifier text is escaped reversibly: `\` doubles, a non-UTF-8 byte
    /// becomes `\xNN` and ASCII is unchanged.
    #[test]
    fn identifier_text_is_escaped_reversibly() {
        let ast = crate::rust::parse_bytes(b"_ZN5a\\b\xffcE").unwrap();
        assert_eq!(ast.to_string(), "a\\\\b\\xffc");
        // The three spellings that must stay distinguishable: an escaped
        // backslash, an escaped byte and the raw ASCII digit `A`.
        let ast = crate::rust::parse_bytes(b"_ZN3A\\AE").unwrap();
        assert_eq!(ast.to_string(), "A\\\\A");
        assert_ne!(ast.to_string(), "A\\x41A");
    }

    /// Escaping multiplies the output by at most four, so a symbol inside the
    /// input budget always stays inside the print budget.
    #[test]
    fn escaped_output_stays_within_the_print_budget() {
        let bytes = vec![0xff_u8; 60 * 1024];
        let mut symbol = Vec::from(&b"_ZN"[..]);
        symbol.extend_from_slice(bytes.len().to_string().as_bytes());
        symbol.extend_from_slice(&bytes);
        symbol.push(b'E');
        let ast = crate::rust::parse_bytes(&symbol).expect("a long escaped name is valid");
        let rendered = ast.to_string();
        assert_eq!(rendered.len(), 4 * bytes.len());
        assert!(rendered.len() <= MAX_OUTPUT_BYTES);
    }

    /// The output budget stops printing and appends the reference's message,
    /// instead of failing or truncating mid-symbol.
    #[test]
    fn output_budget_is_enforced() {
        let symbol = doubling_symbol(20);
        let ast = parse(&symbol).expect("a long demangling is still a valid tree");
        let rendered = ast.to_string();
        assert!(
            rendered.ends_with(SIZE_LIMIT),
            "expected the size-limit marker, got {} bytes",
            rendered.len()
        );
        assert!(
            rendered.len() <= MAX_OUTPUT_BYTES + SIZE_LIMIT.len(),
            "output budget exceeded: {} bytes",
            rendered.len()
        );
        let oracle = rustc_demangle::try_demangle(&symbol).unwrap();
        assert_eq!(rendered, format!("{oracle}"));
    }

    /// Build a v0 tuple whose printed form doubles at every level, using the
    /// same backref chain as the corpus' exponential-tuple case.
    fn doubling_symbol(levels: usize) -> String {
        let mut symbol = String::from("_RMC0");
        symbol.push_str(&"T".repeat(levels));
        symbol.push('p');
        // The innermost element is the placeholder at `3 + levels`; every
        // enclosing tuple is then referenced once, so printing doubles.
        let mut targets: Vec<usize> = Vec::new();
        targets.push(3 + levels);
        for target in (4..3 + levels).rev() {
            targets.push(target);
        }
        for (index, target) in targets.iter().enumerate() {
            if index > 0 {
                symbol.push('E');
            }
            symbol.push('B');
            symbol.push_str(&base62(target - 1));
            symbol.push('_');
        }
        symbol.push('E');
        symbol
    }

    /// The base-62 digit string of `value`, as the mangling writes it.
    fn base62(mut value: usize) -> String {
        const DIGITS: &[u8] = b"0123456789abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ";
        let mut digits = Vec::new();
        loop {
            digits.push(DIGITS[value % 62]);
            value /= 62;
            if value == 0 {
                break;
            }
        }
        digits.reverse();
        String::from_utf8(digits).unwrap_or_default()
    }

    #[test]
    fn base62_matches_the_mangling() {
        assert_eq!(base62(0), "0");
        assert_eq!(base62(8), "8");
        assert_eq!(base62(61), "Z");
        assert_eq!(base62(62), "10");
    }
}
