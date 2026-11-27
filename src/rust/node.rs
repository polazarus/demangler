//! AST node model, spans and parsing budgets.
//!
//! One immutable node graph serves both rustc encodings: the v0 front-end
//! fills the path/type/const vocabulary and the legacy front-end contributes
//! [`Kind::LegacyPath`]/[`Kind::LegacyElement`]. Children are sealed at
//! construction and nodes are `Rc`-shared, so v0 backrefs make the graph a DAG
//! rather than a tree.
//!
//! [`Node`] is only built by the parsers, which are also the only place where
//! the node and work budgets are enforced.
//!
//! Node text is raw bytes, not `str`: a symbol name is a byte string, so an
//! identifier may carry bytes that are not UTF-8. The printer decides how those
//! bytes are rendered (see [`crate::rust::printer`]).

use alloc::borrow::Cow;
use alloc::boxed::Box;
use alloc::rc::Rc;

use crate::budget::{EffectiveBudget, clamp};
use crate::parse::Limits;

use super::ParseError;

/// Default maximum nesting depth of the parser.
///
/// Matches `rustc-demangle`'s `MAX_DEPTH`, so at the default budget the
/// in-tree parser accepts exactly the nesting the reference decoder accepts.
/// Deeper input is rejected with [`ParseError::RecursedTooDeep`] instead of
/// overflowing the stack. A [`crate::Demangler`] budget may raise it up to
/// `HARD_DEPTH`.
pub const MAX_DEPTH: u32 = 500;

/// Default maximum number of nodes a single symbol may produce.
pub const MAX_NODES: usize = 0x0001_0000;

/// Default maximum number of parser work units (production entries, nodes
/// created and backrefs resolved).
pub const MAX_WORK: usize = 200_000;

/// AST printability limit: the deepest tree the recursive printer may descend
/// into.
///
/// It is the hard parse-depth ceiling ([`crate::Scheme::hard_budget`]), so it
/// is a parser-side stack bound, not a printer cap and not a separate
/// [`crate::Budget`] dimension: [`crate::rust::parse`] rejects a deeper tree
/// before it can reach [`core::fmt::Display`], so every parser-built AST
/// prints without overflowing the stack. It follows the hard ceiling, not the
/// caller's `parse_depth`, because back references can nest a tree deeper
/// than the parse recursed. The printer itself has no depth, visit or output
/// limit.
pub const MAX_PRINT_DEPTH: u32 = HARD_DEPTH;

/// Default expansion visit allowance of the Rust parsers.
///
/// One shared child counts once per reference, so a small DAG can expand into
/// many printed nodes. [`crate::rust::parse`] keeps this default; a
/// [`crate::Budget`] may lower or raise it within `HARD_EXPANSION_VISITS`.
pub const DEFAULT_EXPANSION_VISITS: usize = 4_000_000;

/// Default rendered-byte estimate of the Rust parsers: 1 MiB.
///
/// The estimate is a proven upper bound on the bytes the printer writes for an
/// accepted tree, not an exact size. A [`crate::Budget`] may lower or raise it
/// within `HARD_EXPANSION_BYTES`.
pub const DEFAULT_EXPANSION_BYTES: usize = 1 << 20;

/// Hard expansion visit ceiling.
const HARD_EXPANSION_VISITS: usize = 16_000_000;

/// Hard rendered-byte estimate ceiling: 64 MiB.
const HARD_EXPANSION_BYTES: usize = 64 * 1024 * 1024;

/// Default maximum accepted input length.
///
/// Every parser allocation is proportional to the input: the v0 offset cache,
/// the legacy element list and the node vectors. Capping the input at 64 KiB
/// keeps those allocations small even when the caller passes untrusted data;
/// real mangled names are far shorter. A [`crate::Demangler`] budget may raise
/// it up to `HARD_INPUT_LEN`, or `HARD_INPUT_LEN_V0` for v0.
pub const MAX_INPUT_LEN: usize = 64 * 1024;

/// Hard input ceiling of the legacy encoding.
///
/// Bounds the offset, element and node vectors a parse of untrusted bytes may
/// allocate, whatever the caller asks for.
const HARD_INPUT_LEN: usize = 16 * 1024 * 1024;

/// Hard input ceiling of the v0 encoding.
///
/// v0 keeps a checkpoint and a cache entry per input byte, so its ceiling is
/// memory-derived and much tighter than `HARD_INPUT_LEN`.
const HARD_INPUT_LEN_V0: usize = 1024 * 1024;

/// Hard parser depth ceiling: bounds the recursion stack of the v0 parser and,
/// as [`MAX_PRINT_DEPTH`], of the printer. The recursive printer descends
/// about two frames per level, which keeps it below the generic 1024.
const HARD_DEPTH: u32 = 512;

/// Hard parser work ceiling.
const HARD_WORK: usize = 64 * 1024 * 1024;

/// Hard node ceiling: bounds the nodes one parse may allocate.
const HARD_NODES: usize = 1024 * 1024;

/// The limits one parse enforces, resolved from a [`crate::Budget`].
///
/// Every value is already clamped to the hard ceiling of its dimension, so an
/// allocation or a recursive descent stays bounded whatever the caller asks
/// for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct ParserLimits {
    /// Maximum accepted input length in bytes.
    pub(super) input: usize,
    /// Maximum parser nesting depth.
    pub(super) depth: u32,
    /// Maximum parser work units.
    pub(super) work: usize,
    /// Maximum nodes one parse may allocate.
    pub(super) nodes: usize,
    /// Maximum expansion visits one accepted tree may reach.
    pub(super) expansion_visits: usize,
    /// Maximum rendered-byte estimate one accepted tree may reach.
    pub(super) expansion_bytes: usize,
}

impl ParserLimits {
    /// Resolve a scheme budget for a parse of the `v0` encoding.
    ///
    /// An absent dimension keeps its compiled-in default, and every dimension
    /// is clamped to this module's hard ceiling.
    pub(super) fn from_effective(budget: EffectiveBudget, v0: bool) -> Self {
        let input_ceiling = if v0 {
            HARD_INPUT_LEN_V0
        } else {
            HARD_INPUT_LEN
        };
        Self {
            input: clamp(budget.input_bytes, MAX_INPUT_LEN, input_ceiling),
            depth: u32::try_from(clamp(
                budget.parse_depth,
                MAX_DEPTH as usize,
                HARD_DEPTH as usize,
            ))
            .unwrap_or(HARD_DEPTH),
            work: clamp(budget.work, MAX_WORK, HARD_WORK),
            nodes: clamp(budget.nodes, MAX_NODES, HARD_NODES),
            expansion_visits: clamp(
                budget.expansion_visits,
                DEFAULT_EXPANSION_VISITS,
                HARD_EXPANSION_VISITS,
            ),
            expansion_bytes: clamp(
                budget.expansion_bytes,
                DEFAULT_EXPANSION_BYTES,
                HARD_EXPANSION_BYTES,
            ),
        }
    }

    /// Limits handed to the shared byte cursor.
    ///
    /// The cursor enforces the input length and is the only reader of the
    /// symbol bytes. The work, node and depth budgets stay with [`Builder`]
    /// and the parsers' own depth counters, which report them as
    /// [`ParseError`] variants, so the cursor's work and depth counters never
    /// bind.
    pub(super) fn cursor(&self) -> Limits {
        Limits {
            input: u32::try_from(self.input).unwrap_or(u32::MAX),
            depth: u16::try_from(self.depth).unwrap_or(u16::MAX),
            work: u32::MAX,
        }
    }
}

/// A byte range in the original symbol.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Span {
    /// First byte of the range.
    pub start: u32,
    /// One past the last byte of the range.
    pub end: u32,
}

impl Span {
    /// The number of bytes in the range.
    #[must_use]
    pub const fn len(&self) -> u32 {
        self.end.saturating_sub(self.start)
    }

    /// True when the range covers no bytes.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.start == self.end
    }
}

/// The namespace tag of a v0 nested path (`N`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Namespace {
    /// `C`: a closure.
    Closure,
    /// `S`: a shim.
    Shim,
    /// Any other uppercase tag, printed verbatim inside `{...}`.
    Other(char),
    /// A lowercase tag: implementation-internal, hidden when the name is empty.
    Internal,
}

impl Namespace {
    /// The tag character as encoded.
    #[must_use]
    pub const fn tag(&self) -> char {
        match self {
            Self::Closure => 'C',
            Self::Shim => 'S',
            Self::Other(tag) => *tag,
            Self::Internal => 'a',
        }
    }
}

/// Which v0 impl production a [`Kind::Impl`] node came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ImplKind {
    /// `M`: an inherent impl (`<Self>`).
    Inherent,
    /// `X`: a trait impl (`<Self as Trait>`).
    TraitImpl,
    /// `Y`: a trait definition (`<Self as Trait>`).
    TraitDef,
}

/// The value shape of a [`Kind::ConstValue`] node.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ConstValueKind {
    /// An integer literal, `["n"] {hex-digit} "_"`; the node text is the
    /// nibble string and the const's type is its sibling type node.
    Integer {
        /// True when the encoding carried the `n` (negative) marker.
        negative: bool,
    },
    /// The placeholder const `p`, printed `_`.
    Placeholder,
    /// A `b` boolean literal.
    Bool(bool),
    /// A `c` char literal; the node text is the character.
    Char,
    /// An `e` `str` literal, printed `*"..."`; the node text holds the
    /// characters.
    Str,
    /// An `Re` `&str` literal, printed `"..."`; the node text holds the
    /// characters.
    StrRef,
    /// An `R`/`Q` const reference; children: `[value const]`.
    Ref {
        /// True for `Q` (`&mut`), false for `R` (`&`).
        mutability: bool,
    },
    /// An `A` const array; children: the element values.
    Array,
    /// A `T` const tuple; children: the element values.
    Tuple,
    /// A `V` ADT const; see [`AdtConstKind`] for its children.
    Adt(AdtConstKind),
}

/// The shape of a `V` ADT const, which decides the value's children.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AdtConstKind {
    /// `U`: a unit const, printed as the path alone; children: `[path]`.
    Unit,
    /// `T`: a tuple const, printed with `(...)`; children:
    /// `[path, value...]`.
    Tuple,
    /// `S`: a struct const, printed with `{ name: value, ... }`; children:
    /// `[path, field...]`, where every field is a [`Kind::AdtField`].
    Struct,
}

/// The shape of a [`Kind::Pattern`] node.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PatternKind {
    /// `R`: an inclusive range; children: `[low const, high const]`.
    Range,
    /// `O`: an or-pattern; children: the alternative patterns.
    Or,
    /// `N`: `!null`, with no children.
    NeverNull,
}

/// A typed node payload that is not a child node.
///
/// Every payload the implemented grammar produces lives here; accessors on
/// [`Node`] are convenience reads, not a parallel set of fields. The lifetime
/// is the symbol's: function ABI names borrow the input.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum NodeData<'a> {
    /// No payload.
    None,
    /// A disambiguator (`s` + base-62 + 1) on [`Kind::Ident`] or
    /// [`Kind::CrateRoot`].
    Disambiguator(u64),
    /// The namespace tag of a [`Kind::Nested`] path.
    Namespace(Namespace),
    /// Which of `M`, `X`, `Y` a [`Kind::Impl`] came from.
    Impl(ImplKind),
    /// `true` for `&mut`/`*mut`, `false` for shared references and `*const`.
    Mutability(bool),
    /// The base-62 index of a [`Kind::Lifetime`].
    Lifetime(u64),
    /// `true` when a [`Kind::LegacyElement`] is the trailing rustc hash.
    Hash(bool),
    /// The value shape of a [`Kind::ConstValue`].
    Const(ConstValueKind),
    /// The late-bound lifetime count of the `G` binder on a [`Kind::Dyn`]
    /// node; zero means the encoding carried no binder.
    Binder(u64),
    /// A [`Kind::Fn`] signature.
    Function {
        /// The late-bound lifetime count of the `G` binder; zero means none.
        binder: u64,
        /// True for `U`, printed `unsafe `.
        is_unsafe: bool,
        /// The ABI name of `K`, printed inside `extern "..."`; `None` when
        /// the encoding carried no `K`. Raw bytes, like every other name.
        abi: Option<Cow<'a, [u8]>>,
        /// True when the return type was the bare `u` tag, which the
        /// reference prints as nothing instead of ` -> ()`.
        ret_elided: bool,
    },
    /// The shape of a [`Kind::Pattern`].
    Pattern(PatternKind),
}

/// The kind of a Rust AST node.
///
/// The vocabulary is shared by both encodings and grows with the grammar the
/// parser implements. Child order is part of the contract and documented per
/// variant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Kind {
    /// A v0 crate root (`C`).
    ///
    /// No children; `text` is the crate name and [`NodeData::Disambiguator`]
    /// holds its disambiguator.
    CrateRoot,
    /// A v0 nested path (`N`).
    ///
    /// Children: `[parent path, ident]`; [`NodeData::Namespace`] holds the
    /// namespace tag.
    Nested,
    /// A v0 impl (`M`, `X`, `Y`); [`NodeData::Impl`] says which.
    ///
    /// Children: `[impl path, self type]` for `M`,
    /// `[impl path, self type, trait path]` for `X` and
    /// `[self type, trait path]` for `Y`.
    Impl,
    /// A v0 generic argument list (`I`).
    ///
    /// Children: `[base path, argument...]`.
    GenericArgs,
    /// The v0 instantiating crate: parsed for validation, never printed.
    ///
    /// Children: `[path]`.
    InstantiatingCrate,
    /// A v0 identifier.
    ///
    /// No children; `text` is the name and [`NodeData::Disambiguator`] holds
    /// the disambiguator that the enclosing path may print.
    Ident,
    /// A v0 basic type leaf; `text` is the printed type name.
    BasicType,
    /// A v0 array type (`A`).
    ///
    /// Children: `[element type, length const]`.
    Array,
    /// A v0 slice type (`S`).
    ///
    /// Children: `[element type]`.
    Slice,
    /// A v0 tuple type (`T`).
    ///
    /// Children: the element types, in order.
    Tuple,
    /// A v0 reference type (`R`, `Q`); [`NodeData::Mutability`] says which.
    ///
    /// Children: an optional [`Kind::Lifetime`] followed by the pointee type.
    Ref,
    /// A v0 raw pointer type (`P`, `O`); [`NodeData::Mutability`] says which.
    ///
    /// Children: `[pointee type]`.
    RawPtr,
    /// A v0 lifetime (`L`); [`NodeData::Lifetime`] holds the base-62 index.
    Lifetime,
    /// A v0 fn pointer type (`F`).
    ///
    /// Children: the parameter types followed by the return type;
    /// [`NodeData::Function`] holds the binder, `unsafe`, ABI and return
    /// elision.
    Fn,
    /// A v0 `dyn` trait object type (`D`).
    ///
    /// Children: the [`Kind::DynTrait`]s followed by the trailing lifetime;
    /// [`NodeData::Binder`] holds the binder's lifetime count.
    Dyn,
    /// One trait of a `dyn` trait object.
    ///
    /// Children: `[path, binding...]`. When the path is a
    /// [`Kind::GenericArgs`] node, the bindings extend its argument list, so
    /// they print inside the same `<...>`.
    DynTrait,
    /// One associated binding of a `dyn` trait (`p`).
    ///
    /// Children: `[name ident, value]`, where the value is a [`Kind::Const`]
    /// for a `K` binding and a type otherwise.
    DynAssocBinding,
    /// A v0 pattern type (`W`), printed `<type> is <pattern>`.
    ///
    /// Children: `[type, pattern]`.
    PatternType,
    /// The unstable splat marker (`w`) before a type.
    ///
    /// Children: `[type]`; printed `#[rustc_splat] ` followed by the type.
    /// The marker is not in RFC 2603 but is emitted by the unstable `splat`
    /// feature, and the pinned reference decoder accepts it.
    Splat,
    /// A v0 pattern; [`NodeData::Pattern`] says which shape it is.
    ///
    /// Children: `[low, high]` consts for a range, the alternative patterns
    /// for an or-pattern and none for `!null`.
    Pattern,
    /// One field of an `S` (struct) ADT const.
    ///
    /// Children: `[name ident, value const]`.
    AdtField,
    /// A v0 const: a `K` generic argument, an array length, a `dyn` binding
    /// value or a pattern bound.
    ///
    /// Children: `[value]` for every shape except an integer literal, which
    /// has `[type, value]`.
    Const,
    /// A v0 const value.
    ///
    /// [`NodeData::Const`] says which shape it is, `text` holds the decoded
    /// characters of a char or string literal and the hex nibbles of an
    /// integer literal, and compound shapes keep their operands as children.
    ConstValue,
    /// A legacy path (`_ZN...E`).
    ///
    /// Children: the [`Kind::LegacyElement`]s, in order.
    LegacyPath,
    /// One legacy path element.
    ///
    /// No children; `text` is the unescaped element and [`NodeData::Hash`]
    /// marks the trailing rustc hash.
    LegacyElement,
}

/// A shared handle to an AST node.
///
/// Cloning a `NodeRef` only bumps a reference count; v0 backrefs hand out the
/// same handle for every reference to one encoded position.
pub type NodeRef<'a> = Rc<Node<'a>>;

/// One immutable AST node.
///
/// Built only by the parsers (which also enforce the budgets), so the graph is
/// acyclic by construction and every child slice is sealed.
#[derive(Debug)]
pub struct Node<'a> {
    kind: Kind,
    text: Option<Cow<'a, [u8]>>,
    span: Option<Span>,
    children: Box<[NodeRef<'a>]>,
    data: NodeData<'a>,
    /// Expanded printer visits for this node, counting a shared child once per
    /// reference (saturating).
    print_visits: usize,
    /// Structural printer depth for this node (saturating).
    print_depth: u32,
    /// Conservative upper bound on the bytes printing this node writes
    /// (saturating).
    print_bytes: usize,
}

impl<'a> Node<'a> {
    /// The node kind.
    #[must_use]
    pub const fn kind(&self) -> Kind {
        self.kind
    }

    /// The node's text bytes, if it has any.
    ///
    /// Identifiers, basic types and legacy elements borrow the input; text
    /// that had to be reconstructed is owned. The bytes are not guaranteed to
    /// be UTF-8: an identifier copied from a symbol may hold arbitrary bytes,
    /// and the printer escapes them.
    #[must_use]
    pub fn text(&self) -> Option<&[u8]> {
        self.text.as_deref()
    }

    /// The byte range this node was parsed from, if it has one.
    ///
    /// Backref targets keep the span of the bytes they were first parsed from.
    #[must_use]
    pub const fn span(&self) -> Option<Span> {
        self.span
    }

    /// The node's children, in the order documented on its [`Kind`].
    #[must_use]
    pub fn children(&self) -> &[NodeRef<'a>] {
        &self.children
    }

    /// The typed payload.
    #[must_use]
    pub const fn data(&self) -> &NodeData<'a> {
        &self.data
    }

    /// The disambiguator of an [`Kind::Ident`] or [`Kind::CrateRoot`].
    #[must_use]
    pub const fn disambiguator(&self) -> Option<u64> {
        match self.data {
            NodeData::Disambiguator(value) => Some(value),
            _ => None,
        }
    }

    /// The namespace tag of a [`Kind::Nested`] path.
    #[must_use]
    pub const fn namespace(&self) -> Option<Namespace> {
        match self.data {
            NodeData::Namespace(namespace) => Some(namespace),
            _ => None,
        }
    }

    /// The base-62 index of a [`Kind::Lifetime`].
    #[must_use]
    pub const fn lifetime_index(&self) -> Option<u64> {
        match self.data {
            NodeData::Lifetime(index) => Some(index),
            _ => None,
        }
    }

    /// Expanded printer visits, counting a shared child once per reference.
    #[must_use]
    pub(crate) const fn print_visits(&self) -> usize {
        self.print_visits
    }

    /// Structural printer depth.
    #[must_use]
    pub(crate) const fn print_depth(&self) -> u32 {
        self.print_depth
    }

    /// Conservative upper bound on the rendered bytes of this subtree.
    ///
    /// The bound counts node text expansion, a fixed per-kind syntax charge
    /// and any `for<...>` binder or ABI name, and sums over every reference, so
    /// it is an upper bound on what printing this node writes, not an estimate
    /// of the exact size. The vendor suffix is printed outside the tree and is
    /// bounded by the input budget.
    #[must_use]
    pub(crate) const fn print_bytes(&self) -> usize {
        self.print_bytes
    }
}

/// Node factory that charges the node and work budgets.
pub(super) struct Builder {
    nodes: usize,
    work: usize,
    max_nodes: usize,
    max_work: usize,
}

impl<'a> Builder {
    /// A builder with no nodes charged yet and the parse's work and node
    /// ceilings.
    pub(super) const fn new(limits: ParserLimits) -> Self {
        Self {
            nodes: 0,
            work: 0,
            max_nodes: limits.nodes,
            max_work: limits.work,
        }
    }

    /// Charge `work` units, failing when the work budget is exhausted.
    pub(super) const fn charge(&mut self, work: usize) -> Result<(), ParseError> {
        self.work = self.work.saturating_add(work);
        if self.work > self.max_work {
            return Err(ParseError::BudgetExceeded);
        }
        Ok(())
    }

    /// Create a node, charging one node and one work unit.
    pub(super) fn node(
        &mut self,
        kind: Kind,
        text: Option<Cow<'a, [u8]>>,
        span: Option<Span>,
        data: NodeData<'a>,
        children: alloc::vec::Vec<NodeRef<'a>>,
    ) -> Result<NodeRef<'a>, ParseError> {
        self.nodes = self.nodes.saturating_add(1);
        if self.nodes > self.max_nodes {
            return Err(ParseError::BudgetExceeded);
        }
        self.charge(1)?;
        let (print_visits, print_depth, print_bytes) =
            envelope(kind, text.as_deref(), &data, &children);
        Ok(Rc::new(Node {
            kind,
            text,
            span,
            children: children.into_boxed_slice(),
            data,
            print_visits,
            print_depth,
            print_bytes,
        }))
    }
}

/// Conservative printer envelope for one node.
///
/// Visits count a shared child once per reference and depth is the structural
/// nesting; both are exact. Bytes are an upper bound: node text can expand to
/// [`ESCAPED_BYTE_FACTOR`] bytes per input byte, each kind contributes a fixed
/// [`syntax_bytes`] charge and a `for<...>` binder or ABI name adds its own.
fn envelope<'a>(
    kind: Kind,
    text: Option<&[u8]>,
    data: &NodeData<'a>,
    children: &[NodeRef<'a>],
) -> (usize, u32, usize) {
    let mut visits = 1_usize;
    let mut depth = 1_u32;
    let mut bytes = text
        .map_or(0, |text| text.len().saturating_mul(ESCAPED_BYTE_FACTOR))
        .saturating_add(syntax_bytes(kind))
        .saturating_add(data_bytes(data));
    for child in children {
        visits = visits.saturating_add(child.print_visits());
        depth = depth.max(child.print_depth().saturating_add(1));
        bytes = bytes.saturating_add(child.print_bytes());
    }
    (visits, depth, bytes)
}

/// Bytes one input byte can render as: `\xNN` for a byte that is not UTF-8.
const ESCAPED_BYTE_FACTOR: usize = 4;

/// Rendered bytes a `for<...>` binder writes per bound lifetime, including its
/// `, ` separator; the binder's `for<` and `> ` are part of the node charge.
const BINDER_LIFETIME_BYTES: usize = 4;

/// Fixed rendered bytes charged for one node's own syntax.
///
/// Every value is an upper bound on what the printer writes for that kind
/// beyond the node's text and children. The longest single-node literal is
/// `::{closure:#<disambiguator>}` (35 bytes with a 20-digit disambiguator) and
/// the longest separator is ` + ` (3 bytes); every kind is charged at least
/// that separator, so a node's separators are covered by its children's
/// charges.
#[expect(clippy::match_same_arms, reason = "one charge per node kind")]
const fn syntax_bytes(kind: Kind) -> usize {
    match kind {
        // `[` + 16 hex digits + `]`.
        Kind::CrateRoot => 18,
        // `::` + `::{closure:` + `:` + `#` + 20 digits + `}`.
        Kind::Nested => 35,
        // `<` + ` as ` + `>`.
        Kind::Impl => 6,
        // `::` + `<` + `>`.
        Kind::GenericArgs => 4,
        // `[` + `; ` + `]`.
        Kind::Array => 4,
        // `*` + `mut `/`const `.
        Kind::RawPtr => 7,
        // `&` + `mut ` + ` `.
        Kind::Ref => 6,
        // `'` + `_` + 20 digits.
        Kind::Lifetime => 22,
        // `unsafe ` + `extern "` + `" ` + `fn(` + `)` + ` -> `.
        Kind::Fn => 25,
        // `dyn `.
        Kind::Dyn => 4,
        // `<` + `>` + `, `.
        Kind::DynTrait => 4,
        // `#[rustc_splat] `.
        Kind::Splat => 15,
        // `..=` / `!null` / ` | `.
        Kind::Pattern => 5,
        // `{` + `}` plus `false`, `*`, quotes and ` { ` + ` } `.
        Kind::ConstValue => 6,
        // `(` + `,` + `)`, `[` + `]` and `::` separators.
        Kind::Tuple | Kind::Slice | Kind::LegacyPath => 3,
        // `::` + `::{closure:` above; every remaining kind only separates its
        // children or prints nothing.
        _ => 3,
    }
}

/// Rendered bytes charged for the payload shapes the printer expands.
///
/// A `for<...>` binder prints two bytes per bound lifetime plus a separator and
/// an ABI name is escaped like identifier text.
fn data_bytes(data: &NodeData<'_>) -> usize {
    match data {
        NodeData::Function { binder, abi, .. } => binder_bytes(*binder).saturating_add(
            abi.as_ref()
                .map_or(0, |abi| abi.len().saturating_mul(ESCAPED_BYTE_FACTOR)),
        ),
        NodeData::Binder(binder) => binder_bytes(*binder),
        _ => 0,
    }
}

/// Rendered bytes a `for<...>` binder of `bound` lifetimes writes.
fn binder_bytes(bound: u64) -> usize {
    usize::try_from(bound)
        .unwrap_or(usize::MAX)
        .saturating_mul(BINDER_LIFETIME_BYTES)
}

#[cfg(test)]
mod tests;
