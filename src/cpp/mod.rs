//! C++ ABI symbol parsing.
//!
//! This module exposes an immutable, inspectable syntax tree shared by the
//! Itanium and Microsoft C++ mangling schemes. It is independent of the text
//! backends: [`crate::Demangler`] parses C++ into this AST and prints it
//! in-tree.
//!
//! `parse` dispatches on the mangling prefix: `_Z` (and its underscore
//! variants) selects the Itanium parser, a name carrying LLVM's
//! `__alloc_token_` decoration is parsed as the Itanium name it decorates, `?`
//! selects the Microsoft parser, and everything else is tried as a legacy C++
//! name, which is accepted only when a legacy parser consumes the whole symbol.
//! `parse_msvc` parses a Microsoft name directly, including the `.?` RTTI type
//! descriptor name form.
//!
//! The Itanium, Microsoft and legacy printers render this AST as demangled
//! text. [`crate::Demangler`] uses them directly; `print_msvc` is also exposed
//! for callers that want Microsoft text from a parsed tree.
//!
//! The tree is `Rc`-owned and therefore `!Send + !Sync`. Nodes are immutable
//! after construction, children are sealed, and substitution references reuse
//! the same node handle, so the graph may be a DAG.
//!
//! # Supported grammar
//!
//! The parser covers the core Itanium productions:
//!
//! - prefixes `_Z`, `__Z`, `___Z`, `____Z`, plus LLVM's `__alloc_token_`
//!   instrumentation decoration;
//! - encodings, function parameter lists and data names;
//! - unscoped, nested, template-id and well-known substituted names, including
//!   friend function names and module (vendor qualifier) chains;
//! - source names, operator names, constructors/destructors and ABI tags;
//! - local names, lambdas and unnamed types;
//! - builtin, pointer, reference, rvalue-reference, cv-qualified, array,
//!   function, member-pointer, `decltype`, pack-index and vendor-qualified
//!   types;
//! - template arguments, template parameters and expression literals;
//! - C++20 requires-clauses on encodings, template heads and lambda
//!   signatures, and requires-expressions with their requirements;
//! - expression productions: calls, casts, `sizeof`/`alignof`, conversions,
//!   member access, initializer lists, fold and pack expansions, pack
//!   subscripts and pointer-to-member conversions;
//! - special names: vtables, VTTs, typeinfo, thunks, guard variables,
//!   reference temporaries and TLS wrappers/initializers;
//! - substitutions and back-references.
//!
//! Builtins include the single-letter codes plus parametrized `_BitInt`,
//! `_Float`, `bfloat16` and fixed-point types.
//!
//! The corpus gate in `tests/cpp_text_corpus.rs` requires every pair of the
//! vendored libc++abi corpus to reproduce the expected text. The corpus also
//! lists a few bare type manglings, which are not symbol names: `parse`
//! requires an encoding, so those are excluded there by name.
//!
//! Legacy GNU v2 names are covered by the `legacy` module: `__`-separated
//! function names, `Q<n>`-qualified names, operators, constructors and
//! destructors, templates, and the pointer/reference/array/function type forms.
//! The same module implements the ARM, EDG and HP aCC dialects, which share the
//! declarator grammar but spell special names, type back-references and
//! template arguments differently. See that module for the supported subsets
//! and the spelling they print.
//!
//! # Budgets
//!
//! Untrusted input stays bounded by four parser dimensions — input bytes,
//! nesting depth, work units and allocated nodes — that a [`crate::Budget`]
//! may lower or raise within the scheme's hard ceiling. The public
//! [`parse`](crate::cpp::parse) and [`parse_msvc`](crate::cpp::parse_msvc)
//! entry points keep the compiled-in limits (`MAX_INPUT_LEN` and the
//! per-parser depth, work and node budgets); a [`crate::Demangler`] resolves
//! its configured budget for the C++ scheme it dispatches and passes the
//! clamped values to the Itanium, Microsoft and legacy parsers.
//!
//! The parsers derive their work allowance from the input length, so the work
//! dimension caps that allowance rather than replacing it. The depth ceilings
//! are the parsers' compiled-in depth budgets, because the fixed print
//! envelope already bounds the nesting a retained tree may have.
//!
//! Printing is not a budget: the printers keep fixed internal caps that no
//! [`crate::Budget`] field configures, and the [`crate::Demangler`] still
//! rejects a parsed tree outside that envelope.
//!
//! [`crate::Scheme::detect`] keeps the compiled-in limits. A
//! [`crate::Demangler`] passes the configured budget to the legacy C++ prefilter
//! and re-parses a retained legacy tree, so enforcement is exact.
//!
//! # Example
//!
//! ```rust
//! use demangler::cpp;
//!
//! assert_eq!(cpp::prefix_len("_Z1fv"), Some(2));
//! assert!(cpp::is_itanium("_ZN3Foo3barEv"));
//! ```

#[cfg(feature = "cpp-gnu-v2")]
pub(crate) mod legacy;
#[cfg(feature = "cpp-msvc")]
mod msvc;
#[cfg(feature = "cpp-msvc")]
mod msvc_printer;
mod node;
#[cfg(feature = "cpp-itanium")]
mod parser;
#[cfg(any(feature = "cpp-itanium", feature = "cpp-gnu-v2"))]
mod print_buffer;
#[cfg(feature = "cpp-itanium")]
mod printer;
mod text;

use alloc::rc::Rc;
use core::fmt;

pub(crate) use text::decode;
#[cfg(feature = "cpp-msvc")]
pub(crate) use text::owned_text;

pub use node::{Kind, Node, NodeRef};

#[cfg(feature = "cpp-msvc")]
pub use msvc::parse_msvc;
#[cfg(feature = "cpp-msvc")]
pub use msvc_printer::{PrintError, PrintErrorKind, print_msvc};

#[cfg(any(feature = "cpp-itanium", feature = "cpp-msvc", feature = "cpp-gnu-v2"))]
use crate::error::BudgetKind;
#[cfg(any(feature = "cpp-itanium", feature = "cpp-msvc"))]
use crate::parse::{Fail, FailKind};
#[cfg(feature = "cpp-itanium")]
use crate::parse::{Limits, State};
#[cfg(feature = "cpp-itanium")]
use parser::Parser;

use crate::budget::{EffectiveBudget, Limit};
use crate::scheme::Scheme;

/// Maximum input length accepted by the C++ text and AST paths.
pub(crate) const MAX_INPUT_LEN: usize = 64 * 1024;

/// Work allowance per input byte, shared by the three C++ parsers.
const WORK_PER_BYTE: usize = 32;

/// Minimum work allowance for very short C++ symbols.
const MIN_WORK: usize = 1 << 12;

/// Work allowance the parsers derive at the default input cap.
///
/// Each parser derives its allowance from the symbol length (`32 × len +
/// 4096`) and caps it at the resolved work budget, so at [`MAX_INPUT_LEN`] the
/// derived allowance never exceeds this value. [`crate::Scheme::default_budget`]
/// reports it as the C++ work default.
pub(crate) const DEFAULT_WORK: usize = MAX_INPUT_LEN * WORK_PER_BYTE + MIN_WORK;

/// No budget override: every C++ parser keeps its compiled-in limits.
///
/// The public entry points parse with it, so they keep exactly the pre-budget
/// limits; a [`crate::Demangler`] resolves its configured [`crate::Budget`]
/// per scheme instead. A dimension left unset keeps the parser's compiled-in
/// limit. That includes the two expansion gates: this sentinel is shared by
/// every dialect, and the compiled-in per-dialect limits differ
/// ([`Scheme::CppItanium`](crate::Scheme::CppItanium) accepts 262,144 visits
/// while [`Scheme::CppGnuV2`](crate::Scheme::CppGnuV2) accepts 65,536), so
/// `None` keeps the dialect's own limit instead of naming one here.
pub(crate) const DEFAULT_BUDGET: EffectiveBudget = EffectiveBudget {
    input_bytes: None,
    parse_depth: None,
    work: None,
    nodes: None,
    expansion_visits: None,
    expansion_bytes: None,
};

/// Printer node visits charged per expanded tree visit in the print envelope.
///
/// The printers' declarator and template-resolution traversals re-enter some
/// nodes, so one tree visit can cost several printer visits. This converts the
/// tree's visit count into printer visits; it is a unit conversion, not a cap.
const PRINT_VISIT_FACTOR: usize = 4;

/// Deepest structural print depth a printer of `dialect` may recurse to.
///
/// This is the dialect's hard parse-depth ceiling
/// ([`crate::Scheme::hard_budget`]) times its declarator factor: the MSVC
/// printer walks a declarator left and right, so its stack can reach twice
/// the parser's. It follows the hard ceiling, not the caller's `parse_depth`,
/// because it guards a printer's stack, not the parse.
pub(crate) fn printer_depth_limit(dialect: Dialect) -> usize {
    let (scheme, factor) = match dialect {
        #[cfg(feature = "cpp-itanium")]
        Dialect::Itanium => (Scheme::CppItanium, 1),
        #[cfg(feature = "cpp-msvc")]
        Dialect::Msvc => (Scheme::CppMsvc, 2),
        #[cfg(feature = "cpp-gnu-v2")]
        Dialect::GnuV2 | Dialect::Arm | Dialect::Edg | Dialect::Hp => (Scheme::CppGnuV2, 1),
        #[allow(
            unreachable_patterns,
            reason = "reachable when a backend feature is disabled"
        )]
        _ => return 0,
    };
    match scheme.hard_budget().parse_depth {
        Some(Limit::Bounded(depth)) => depth.saturating_mul(factor),
        _ => 0,
    }
}

/// The parser limits one C++ parse enforces, resolved from a budget.
///
/// Every value is already clamped to the hard ceiling of the scheme whose
/// parser runs, so a recursive descent, a work charge and an allocation stay
/// bounded whatever the caller asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ParseLimits {
    input: usize,
    depth: u32,
    work: usize,
    nodes: u32,
    expansion_visits: usize,
    expansion_bytes: usize,
}

impl ParseLimits {
    /// Limits for the Itanium parser.
    #[cfg(feature = "cpp-itanium")]
    fn itanium(budget: EffectiveBudget) -> Self {
        Self::resolve(
            budget,
            Scheme::CppItanium,
            parser::MAX_DEPTH,
            parser::MAX_NODES,
        )
    }

    /// Limits for the Microsoft parser.
    #[cfg(feature = "cpp-msvc")]
    fn msvc(budget: EffectiveBudget) -> Self {
        Self::resolve(budget, Scheme::CppMsvc, msvc::MAX_DEPTH, msvc::MAX_NODES)
    }

    /// Limits for the legacy (GNU v2, ARM, EDG and HP aCC) parsers.
    ///
    /// The four legacy schemes share one default and one hard ceiling, so one
    /// resolution covers every dialect the legacy parser tries.
    #[cfg(feature = "cpp-gnu-v2")]
    fn legacy(budget: EffectiveBudget) -> Self {
        Self::resolve(
            budget,
            Scheme::CppGnuV2,
            legacy::MAX_DEPTH,
            legacy::MAX_NODES,
        )
    }

    /// Resolve one parser's compiled-in `depth` and `nodes` against `scheme`.
    fn resolve(budget: EffectiveBudget, scheme: Scheme, depth: u32, nodes: u32) -> Self {
        let hard = scheme.hard_budget();
        let defaults = scheme.default_budget();
        Self {
            input: clamp(budget.input_bytes, MAX_INPUT_LEN, hard.input_bytes),
            depth: clamp_u32(budget.parse_depth, depth, hard.parse_depth),
            work: clamp(budget.work, DEFAULT_WORK, hard.work),
            nodes: clamp_u32(budget.nodes, nodes, hard.nodes),
            expansion_visits: clamp(
                budget.expansion_visits,
                defaults.expansion_visits.unwrap_or(0),
                hard.expansion_visits,
            ),
            expansion_bytes: clamp(
                budget.expansion_bytes,
                defaults.expansion_bytes.unwrap_or(0),
                hard.expansion_bytes,
            ),
        }
    }

    /// Maximum accepted input length in bytes.
    const fn input(&self) -> usize {
        self.input
    }

    /// Maximum parser nesting depth.
    const fn depth(&self) -> u32 {
        self.depth
    }

    /// Maximum parser work units.
    const fn work(&self) -> usize {
        self.work
    }

    /// Maximum nodes one parse may allocate.
    const fn nodes(&self) -> u32 {
        self.nodes
    }

    /// Maximum expansion visits nested parse-time renders may reach.
    #[cfg(feature = "cpp-gnu-v2")]
    const fn expansion_visits(&self) -> usize {
        self.expansion_visits
    }

    /// Maximum rendered bytes nested parse-time renders may reach.
    #[cfg(feature = "cpp-gnu-v2")]
    const fn expansion_bytes(&self) -> usize {
        self.expansion_bytes
    }
}

/// One budget dimension after clamping to its compiled-in default and the
/// scheme's hard ceiling.
fn clamp(dimension: Option<Limit>, default: usize, ceiling: Option<Limit>) -> usize {
    let ceiling = match ceiling {
        Some(Limit::Bounded(value)) => value,
        // Every C++ scheme bounds every parser dimension; keep the compiled-in
        // default if the policy table ever leaves one open.
        _ => default,
    };
    match dimension {
        Some(Limit::Bounded(value)) => value.min(ceiling),
        Some(Limit::Unbounded) | None => default.min(ceiling),
    }
}

/// [`clamp`] for one of the `u32` budget dimensions.
fn clamp_u32(dimension: Option<Limit>, default: u32, ceiling: Option<Limit>) -> u32 {
    let default = to_usize(default);
    u32::try_from(clamp(dimension, default, ceiling)).unwrap_or(u32::MAX)
}

/// Widen a `u32` budget value to the `usize` used by [`Limit`].
fn to_usize(value: u32) -> usize {
    usize::try_from(value).unwrap_or(usize::MAX)
}

/// Length of a recognised Itanium mangling prefix.
///
/// The longest matching prefix wins: `____Z`, `___Z`, `__Z`, then `_Z`.
#[must_use]
pub fn prefix_len(symbol: &str) -> Option<usize> {
    prefix_len_bytes(symbol.as_bytes())
}

/// Length of a recognised Itanium mangling prefix in a byte string.
pub(crate) fn prefix_len_bytes(symbol: &[u8]) -> Option<usize> {
    if symbol.starts_with(b"____Z") {
        Some(5)
    } else if symbol.starts_with(b"___Z") {
        Some(4)
    } else if symbol.starts_with(b"__Z") {
        Some(3)
    } else if symbol.starts_with(b"_Z") {
        Some(2)
    } else {
        None
    }
}

/// True when `symbol` starts with an Itanium mangling prefix.
#[must_use]
pub fn is_itanium(symbol: &str) -> bool {
    prefix_len(symbol).is_some()
}

/// True when a byte string starts with an Itanium mangling prefix.
#[cfg(feature = "cpp-itanium")]
#[must_use]
pub(crate) fn is_itanium_bytes(symbol: &[u8]) -> bool {
    prefix_len_bytes(symbol).is_some()
}

/// True when `symbol` starts with a Microsoft C++ mangling prefix.
///
/// A Microsoft symbol starts with `?`. The RTTI type descriptor name form
/// starts with `.?` and is accepted by [`parse_msvc`] directly.
#[must_use]
pub fn is_msvc(symbol: &str) -> bool {
    symbol.starts_with('?') || symbol.starts_with(".?")
}

/// True when a byte string starts with a Microsoft C++ prefix.
#[cfg(feature = "cpp-msvc")]
#[must_use]
pub(crate) fn is_msvc_bytes(symbol: &[u8]) -> bool {
    symbol.starts_with(b"?") || symbol.starts_with(b".?")
}

/// The mangling scheme an [`Ast`] was parsed from.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Dialect {
    /// Itanium C++ ABI (`_Z...`).
    Itanium,
    /// Microsoft C++ (`?...`).
    Msvc,
    /// Legacy GNU v2 (`foo__Fiv`).
    GnuV2,
    /// ARM C++ (`__ct__3fooFv`, `__vtbl__3foo`, `__sti__foo`).
    Arm,
    /// EDG C++ (`__ct__3fooFv`, `foo__Fi_v`, `foo__tm__2_i`).
    Edg,
    /// HP aCC C++ (`foo__XSP5_Fv`, `foo__F3barXSP6`, `foo__Fi_1`).
    Hp,
}

/// A parsed C++ symbol with an immutable node graph.
///
/// The graph is `Rc`-owned, so `Ast` is `!Send + !Sync`. Clone the `Ast` to
/// share the graph without re-parsing.
#[derive(Clone)]
pub struct Ast<'a> {
    dialect: Dialect,
    root: NodeRef<'a>,
    /// True when the graph contains a `Kind::Lambda` node.
    ///
    /// The Itanium printer's lambda-parameter pre-pass is skipped when false;
    /// Microsoft and legacy names never emit lambdas.
    has_lambda: bool,
}

impl<'a> Ast<'a> {
    /// Wrap a parsed Itanium root node.
    #[cfg(feature = "cpp-itanium")]
    pub(crate) fn from_root(root: NodeRef<'a>, has_lambda: bool) -> Self {
        Self {
            dialect: Dialect::Itanium,
            root,
            has_lambda,
        }
    }

    /// Wrap a parsed Microsoft root node.
    #[cfg(feature = "cpp-msvc")]
    pub(crate) fn from_root_msvc(root: NodeRef<'a>) -> Self {
        Self {
            dialect: Dialect::Msvc,
            root,
            has_lambda: false,
        }
    }

    /// Wrap a parsed legacy root node for `dialect`.
    ///
    /// `dialect` is one of [`Dialect::GnuV2`], [`Dialect::Arm`],
    /// [`Dialect::Edg`] or [`Dialect::Hp`]; the legacy parser is the only
    /// caller.
    #[cfg(feature = "cpp-gnu-v2")]
    pub(crate) fn from_root_legacy(root: NodeRef<'a>, dialect: Dialect) -> Self {
        debug_assert!(matches!(
            dialect,
            Dialect::GnuV2 | Dialect::Arm | Dialect::Edg | Dialect::Hp
        ));
        Self {
            dialect,
            root,
            has_lambda: false,
        }
    }

    /// True when the graph contains a lambda node.
    #[cfg(feature = "cpp-itanium")]
    #[must_use]
    pub(crate) const fn has_lambda(&self) -> bool {
        self.has_lambda
    }

    /// True when the tree fits the conservative printer envelope.
    ///
    /// The envelope checks the resolved expansion budgets against the tree's
    /// expanded visits (times [`PRINT_VISIT_FACTOR`]) and rendered bytes, and
    /// bounds the printer's recursion by [`printer_depth_limit`].
    pub(crate) fn print_envelope_ok(&self, budget: EffectiveBudget) -> bool {
        let visits = match budget.expansion_visits {
            Some(Limit::Bounded(value)) => value,
            _ => usize::MAX,
        };
        let bytes = match budget.expansion_bytes {
            Some(Limit::Bounded(value)) => value,
            _ => usize::MAX,
        };
        self.printer_depth_ok()
            && self.expanded_visits() <= visits
            && self.root.print_bytes() <= bytes
    }

    /// Printer visits the envelope charges for this tree.
    pub(crate) fn expanded_visits(&self) -> usize {
        self.root.print_visits().saturating_mul(PRINT_VISIT_FACTOR)
    }

    /// Rendered bytes the envelope charges for this tree.
    #[cfg(feature = "cpp-gnu-v2")]
    pub(crate) fn expanded_bytes(&self) -> usize {
        self.root.print_bytes()
    }

    /// True when the printer's recursion stays within the dialect's hard
    /// parse-depth ceiling times its declarator factor.
    pub(crate) fn printer_depth_ok(&self) -> bool {
        usize::try_from(self.root.print_depth())
            .is_ok_and(|printed| printed <= printer_depth_limit(self.dialect))
    }

    /// The mangling scheme this tree was parsed from.
    #[must_use]
    pub const fn dialect(&self) -> Dialect {
        self.dialect
    }

    /// The root [`Kind::Encoding`] node.
    #[must_use]
    pub fn root(&self) -> NodeRef<'a> {
        Rc::clone(&self.root)
    }
}

impl fmt::Debug for Ast<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Ast")
            .field("dialect", &self.dialect)
            .field("has_lambda", &self.has_lambda)
            .field("root", &self.root)
            .finish()
    }
}

impl fmt::Display for Ast<'_> {
    /// Render the parsed symbol as demangled C++ text.
    ///
    /// The text follows the LLVM demangler's conventions. Printing is bounded
    /// by depth, node-visit and output-byte budgets, so a symbol that parses
    /// but cannot be rendered is reported as an error. Each call re-renders the
    /// retained tree; cache the text if it is printed repeatedly.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.dialect {
            #[cfg(feature = "cpp-itanium")]
            Dialect::Itanium => printer::print(self, f),
            #[cfg(feature = "cpp-msvc")]
            Dialect::Msvc => print_msvc(self, f).map_err(|_| fmt::Error),
            #[cfg(feature = "cpp-gnu-v2")]
            Dialect::GnuV2 | Dialect::Arm | Dialect::Edg | Dialect::Hp => legacy::print(self, f),
            #[allow(
                unreachable_patterns,
                reason = "reachable when a backend feature is disabled"
            )]
            _ => Err(fmt::Error),
        }
    }
}

/// Why a C++ symbol could not be parsed.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[non_exhaustive]
pub enum ParseErrorKind {
    /// The symbol does not start with a supported Itanium prefix.
    NotItanium,
    /// The symbol does not start with a supported Microsoft prefix.
    NotMsvc,
    /// The symbol is not a legacy C++ name.
    NotLegacy,
    /// The input ended in the middle of a production.
    UnexpectedEnd,
    /// A byte was not valid at this position.
    UnexpectedByte,
    /// A source-name length or sequence id was malformed or out of range.
    InvalidLength,
    /// A substitution referenced an entry that does not exist yet.
    UnknownSubstitution,
    /// The production is valid Itanium but not implemented yet.
    Unsupported,
    /// The nesting budget was exceeded.
    TooDeep,
    /// The node budget was exceeded.
    TooManyNodes,
    /// The work budget was exceeded.
    TooMuchWork,
    /// A nested symbol rendered at parse time exceeded the expansion visit
    /// budget.
    TooManyExpansionVisits,
    /// A nested symbol rendered at parse time exceeded the expansion byte
    /// budget.
    TooManyExpansionBytes,
    /// The symbol is too long for `u32` spans.
    TooLong,
}

impl ParseError {
    /// The parser budget dimension this failure belongs to, when it is a
    /// capacity failure rather than a malformed symbol.
    #[must_use]
    pub const fn budget_kind(&self) -> Option<BudgetKind> {
        match self.kind() {
            ParseErrorKind::TooLong => Some(BudgetKind::Input),
            ParseErrorKind::TooDeep => Some(BudgetKind::ParseDepth),
            ParseErrorKind::TooMuchWork => Some(BudgetKind::Work),
            ParseErrorKind::TooManyExpansionVisits => Some(BudgetKind::ExpansionVisits),
            ParseErrorKind::TooManyExpansionBytes => Some(BudgetKind::ExpansionBytes),
            ParseErrorKind::TooManyNodes => Some(BudgetKind::Nodes),
            _ => None,
        }
    }
}

impl fmt::Display for ParseErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::NotItanium => "not an Itanium mangled name",
            Self::NotMsvc => "not a Microsoft mangled name",
            Self::NotLegacy => "not a legacy C++ mangled name",
            Self::UnexpectedEnd => "unexpected end of input",
            Self::UnexpectedByte => "unexpected byte",
            Self::InvalidLength => "invalid length",
            Self::UnknownSubstitution => "unknown substitution",
            Self::Unsupported => "unsupported production",
            Self::TooDeep => "nesting budget exceeded",
            Self::TooManyNodes => "node budget exceeded",
            Self::TooMuchWork => "work budget exceeded",
            Self::TooManyExpansionVisits => "expansion visit budget exceeded",
            Self::TooManyExpansionBytes => "expansion byte budget exceeded",
            Self::TooLong => "symbol too long",
        };
        f.write_str(message)
    }
}

/// A parse failure with the byte offset where it was detected.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ParseError {
    position: usize,
    kind: ParseErrorKind,
}

impl ParseError {
    /// Create an error at `position`.
    pub(crate) const fn new(position: usize, kind: ParseErrorKind) -> Self {
        Self { position, kind }
    }

    /// Byte offset in the input where parsing failed.
    #[must_use]
    pub const fn position(&self) -> usize {
        self.position
    }

    /// What went wrong.
    #[must_use]
    pub const fn kind(&self) -> ParseErrorKind {
        self.kind
    }
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} at byte {}", self.kind, self.position)
    }
}

impl core::error::Error for ParseError {}

/// Map a cursor failure back onto the public C++ parse error.
///
/// The cursor-based parsers (Itanium and Microsoft) keep their own depth,
/// node and work budgets, so the cursor's failures are byte-level: end of
/// input, a length overflow, or the input cap.
#[cfg(any(feature = "cpp-itanium", feature = "cpp-msvc"))]
pub(crate) fn cursor_error(fail: Fail) -> ParseError {
    let kind = match fail.kind {
        FailKind::EndOfInput => ParseErrorKind::UnexpectedEnd,
        FailKind::InputBudget => ParseErrorKind::TooLong,
        FailKind::LengthOverflow => ParseErrorKind::InvalidLength,
        FailKind::Work => ParseErrorKind::TooMuchWork,
        FailKind::Depth => ParseErrorKind::TooDeep,
        FailKind::Invalid => ParseErrorKind::UnexpectedByte,
    };
    ParseError::new(fail.offset as usize, kind)
}

/// True when `symbol` carries LLVM's `__alloc_token_` decoration.
#[cfg(feature = "cpp-itanium")]
#[must_use]
pub(crate) fn is_alloc_token_bytes(symbol: &[u8]) -> bool {
    alloc_token_name_offset_bytes(symbol).is_some()
}

/// Parse a C++ mangled name into an inspectable AST.
///
/// Itanium names (a `_Z` prefix, including the underscored variants) use the
/// Itanium parser; a name carrying LLVM's `__alloc_token_` decoration is parsed
/// as the Itanium name it decorates; names starting with `?` use the Microsoft
/// parser; anything else is parsed as a legacy C++ name and accepted only when
/// one of the legacy parsers consumes the whole symbol. The legacy dialects are
/// tried in priority order — GNU v2 first, then ARM, EDG and HP aCC — except
/// for the reserved cfront special-name markers, which select those dialects
/// first; the `cpp::legacy` module documents the selection rules in full.
///
/// # Errors
///
/// Returns [`ParseError`] when the symbol is not a recognised C++ mangling,
/// uses an unsupported production, is malformed, or exceeds the parser's
/// depth, node or work budgets.
pub fn parse(symbol: &str) -> Result<Ast<'_>, ParseError> {
    parse_bytes(symbol.as_bytes())
}

/// Parse a C++ mangled name from raw bytes.
///
/// This is the byte-native entry point: it performs the same dialect dispatch
/// as [`parse`] but never requires the symbol to be UTF-8. Identifier bytes
/// that are not valid UTF-8 are escaped at node construction (see
/// [`text`]), so every byte of the input is represented in the tree. The
/// compiled-in limits are used; a [`crate::Demangler`] resolves its configured
/// budget instead.
pub(crate) fn parse_bytes(symbol: &[u8]) -> Result<Ast<'_>, ParseError> {
    parse_bytes_with_budget(symbol, DEFAULT_BUDGET)
}

/// Parse a C++ mangled name from raw bytes with one scheme's resolved budget.
///
/// [`parse_bytes`] wraps this with [`DEFAULT_BUDGET`]; a [`crate::Demangler`]
/// passes [`Demangler::effective_budget`] for the C++ scheme it
/// dispatches, so a caller's overrides reach the parser the symbol selects.
/// Each parser resolves the budget again for its own scheme, so a legacy name
/// keeps the legacy ceilings even when the requested scheme is Itanium.
///
/// Printing itself has no cap. The [`crate::Demangler`] checks the resolved
/// expansion budgets before returning a retained tree; public `parse` keeps
/// its documented fallible-AST contract.
pub(crate) fn parse_bytes_with_budget(
    symbol: &[u8],
    budget: EffectiveBudget,
) -> Result<Ast<'_>, ParseError> {
    #[cfg(feature = "cpp-itanium")]
    if let Some(prefix) = prefix_len_bytes(symbol) {
        let limits = ParseLimits::itanium(budget);
        let mut state = itanium_state(limits.input());
        return Parser::new(symbol, prefix, &mut state, limits)?.parse();
    }
    #[cfg(feature = "cpp-itanium")]
    if let Some(offset) = alloc_token_name_offset_bytes(symbol) {
        // LLVM's allocation-token instrumentation prefixes the mangled name.
        let prefix = symbol
            .get(offset..)
            .and_then(prefix_len_bytes)
            .ok_or(ParseError::new(0, ParseErrorKind::NotItanium))?;
        let limits = ParseLimits::itanium(budget);
        let mut state = itanium_state(limits.input());
        return Parser::new_alloc_token(symbol, offset + prefix, &mut state, limits)?.parse();
    }
    #[cfg(feature = "cpp-msvc")]
    if symbol.starts_with(b"?") {
        return msvc::parse_msvc_bytes_with_budget(symbol, budget);
    }
    #[cfg(feature = "cpp-gnu-v2")]
    if legacy::looks_like_legacy_bytes(symbol) {
        return legacy::parse_bytes_with_budget(symbol, budget);
    }
    Err(ParseError::new(0, ParseErrorKind::NotItanium))
}

/// Parser budget for the Itanium parser.
///
/// The parser keeps its own depth, node and work accounting, so the cursor's
/// counters are only used to bound the input length.
#[cfg(feature = "cpp-itanium")]
fn itanium_state(input: usize) -> State {
    State::new(Limits {
        input: u32::try_from(input).unwrap_or(u32::MAX),
        depth: u16::MAX,
        work: u32::MAX,
    })
}

/// Byte offset of the mangled name in a symbol that carries LLVM's
/// allocation-token decoration.
///
/// The instrumentation pass spells the decoration `__alloc_token_`, optionally
/// followed by the token number and a `_` separator. It is not part of the
/// mangling.
#[cfg(feature = "cpp-itanium")]
fn alloc_token_name_offset_bytes(symbol: &[u8]) -> Option<usize> {
    const PREFIX: &[u8] = b"__alloc_token_";
    let rest = symbol.strip_prefix(PREFIX)?;
    let digits = rest.iter().take_while(|byte| byte.is_ascii_digit()).count();
    let mut offset = PREFIX.len() + digits;
    if digits > 0 {
        if symbol.get(offset) != Some(&b'_') {
            return None;
        }
        offset += 1;
    }
    Some(offset)
}

#[cfg(test)]
mod tests {
    use alloc::format;

    use super::*;
    use crate::{Budget, Demangler};
    #[test]
    fn corpus_print_envelope_holds() {
        let base = env!("CARGO_MANIFEST_DIR");
        let mut checked = 0_usize;

        // Itanium text corpus: { "mangled", "expected" } entries.
        if let Ok(data) =
            std::fs::read_to_string(format!("{base}/refs/cpp/itanium/DemangleTestCases.inc"))
        {
            for line in data.lines() {
                let Some(rest) = line.strip_prefix("{\"") else {
                    continue;
                };
                let Some((symbol, _)) = rest.split_once("\",") else {
                    continue;
                };
                checked += check_cpp_envelope(symbol);
            }
        }

        // Legacy corpora: {@code <mangled><TAB><expected>}.
        for name in [
            "gnu-v2.txt",
            "gnu-v2-large.txt",
            "arm.txt",
            "edg.txt",
            "hp.txt",
        ] {
            let path = format!("{base}/refs/cpp/legacy/{name}");
            let Ok(data) = std::fs::read_to_string(path) else {
                continue;
            };
            for line in data.lines() {
                if line.starts_with('#') || line.is_empty() {
                    continue;
                }
                let symbol = line.split('\t').next().unwrap_or_default();
                checked += check_cpp_envelope(symbol);
            }
        }

        // MSVC real-symbol corpora and .test fixtures.
        let msvc = format!("{base}/refs/cpp/msvc");
        if let Ok(entries) = std::fs::read_dir(&msvc) {
            for entry in entries.flatten() {
                let path = entry.path();
                let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                    continue;
                };
                let extension = path.extension().and_then(|extension| extension.to_str());
                if !(name.starts_with("real-") && extension == Some("txt")
                    || extension == Some("test"))
                {
                    continue;
                }
                let Ok(data) = std::fs::read_to_string(&path) else {
                    continue;
                };
                for line in data.lines() {
                    let symbol = line.trim();
                    if symbol.starts_with('?') {
                        checked += check_cpp_envelope(symbol);
                    }
                }
            }
        }

        assert!(checked > 0, "no C++ corpus symbols were checked");
    }

    fn check_cpp_envelope(symbol: &str) -> usize {
        let Ok(ast) = parse(symbol) else {
            return 0;
        };
        let mut out = alloc::string::String::new();
        let stats = match ast.dialect() {
            #[cfg(feature = "cpp-itanium")]
            Dialect::Itanium => printer::stats(&ast, &mut out),
            #[cfg(feature = "cpp-msvc")]
            Dialect::Msvc => msvc_printer::stats(&ast, &mut out).map_err(|_| fmt::Error),
            #[cfg(feature = "cpp-gnu-v2")]
            Dialect::GnuV2 | Dialect::Arm | Dialect::Edg | Dialect::Hp => {
                legacy::stats(&ast, &mut out)
            }
            #[allow(
                unreachable_patterns,
                reason = "reachable when a backend feature is disabled"
            )]
            _ => Err(fmt::Error),
        };
        let (visits, depth, bytes) = stats.expect("accepted C++ tree must render");
        let root = ast.root();
        let depth_factor = if matches!(ast.dialect(), Dialect::Msvc) {
            1
        } else {
            2
        };
        assert!(
            visits <= root.print_visits().saturating_mul(4),
            "{symbol}: {visits} visits > {}",
            root.print_visits().saturating_mul(4)
        );
        assert!(
            depth <= root.print_depth().saturating_mul(depth_factor),
            "{symbol}: depth {depth} > {}",
            root.print_depth().saturating_mul(depth_factor)
        );
        assert!(
            bytes <= root.print_bytes(),
            "{symbol}: {bytes} bytes > {}",
            root.print_bytes()
        );
        1
    }

    #[test]
    fn recognises_prefixes() {
        assert_eq!(prefix_len("_Z1fv"), Some(2));
        assert_eq!(prefix_len("__Z1fv"), Some(3));
        assert_eq!(prefix_len("___Z1fv"), Some(4));
        assert_eq!(prefix_len("____Z1fv"), Some(5));
        assert_eq!(prefix_len("_Z"), Some(2));
        assert_eq!(prefix_len("main"), None);
        assert!(is_itanium("_ZN3Foo3barEv"));
        assert!(!is_itanium("$s4main3foo"));
    }

    #[test]
    fn rejects_non_itanium() {
        let error = parse("main").unwrap_err();
        assert_eq!(error.kind(), ParseErrorKind::NotItanium);
        assert_eq!(error.position(), 0);
    }

    #[cfg(feature = "cpp-itanium")]
    #[test]
    fn root_is_an_encoding() {
        let ast = parse("_Z1A").unwrap();
        assert_eq!(ast.root().kind(), Kind::Encoding);
        assert_eq!(ast.root().child_count(), 1);
        assert_eq!(ast.root().child(0).unwrap().kind(), Kind::Data);
    }

    #[cfg(feature = "cpp-itanium")]
    #[test]
    fn lambda_flag_tracks_the_graph() {
        assert!(parse("_ZNK10inline_varMUlvE_clEv").unwrap().has_lambda());
        assert!(!parse("_Z3foov").unwrap().has_lambda());
    }

    #[cfg(feature = "cpp-itanium")]
    #[test]
    fn ast_clone_shares_the_root() {
        let ast = parse("_Z1fv").unwrap();
        let cloned = ast.clone();
        assert!(Rc::ptr_eq(&ast.root(), &cloned.root()));
    }

    #[cfg(feature = "cpp-itanium")]
    #[test]
    fn ast_debug_is_bounded() {
        let ast = parse("_Z1fPPPPi").unwrap();
        let rendered = alloc::format!("{ast:?}");
        assert!(rendered.contains("Ast"));
        assert!(rendered.contains("Encoding"));
        assert!(!rendered.contains("BuiltinType"));
    }

    /// Collect the text of every node carrying `kind`, bounded by the node
    /// budget so a shared DAG cannot loop.
    #[cfg(any(feature = "cpp-itanium", feature = "cpp-msvc", feature = "cpp-gnu-v2"))]
    fn texts_of(ast: &Ast<'_>, kind: Kind) -> alloc::vec::Vec<alloc::string::String> {
        let mut out = alloc::vec::Vec::new();
        let mut stack = alloc::vec![ast.root()];
        let mut visits = 0_usize;
        while let Some(node) = stack.pop() {
            visits += 1;
            assert!(visits < 1 << 16, "node walk is bounded");
            if node.kind() == kind
                && let Some(text) = node.text()
            {
                out.push(alloc::string::String::from(text));
            }
            stack.extend(node.children().iter().cloned());
        }
        out
    }

    #[cfg(feature = "cpp-itanium")]
    #[test]
    fn byte_path_accepts_invalid_utf8_in_source_names() {
        // The source name is length-prefixed by bytes, so the `\xff` byte is
        // part of the identifier even though it is not UTF-8.
        let ast = parse_bytes(b"_ZN4F\xffoo3barEv").unwrap();
        let texts = texts_of(&ast, Kind::Identifier);
        assert!(texts.iter().any(|text| text == "F\\xffoo"), "{texts:?}");
        assert!(texts.iter().any(|text| text == "bar"), "{texts:?}");

        // Spans address the supplied bytes.
        let mut stack = alloc::vec![ast.root()];
        let mut found = None;
        while let Some(node) = stack.pop() {
            if node.text() == Some("F\\xffoo") {
                found = node.span();
                break;
            }
            stack.extend(node.children().iter().cloned());
        }
        assert_eq!(found, Some(3..8));
    }

    #[cfg(feature = "cpp-gnu-v2")]
    #[test]
    fn byte_path_accepts_invalid_utf8_in_legacy_names() {
        let ast = parse_bytes(b"f\xffoo__Fiv").unwrap();
        assert_eq!(ast.dialect(), Dialect::GnuV2);
        let texts = texts_of(&ast, Kind::Identifier);
        assert!(texts.iter().any(|text| text == "f\\xffoo"), "{texts:?}");
    }

    #[cfg(feature = "cpp-msvc")]
    #[test]
    fn byte_path_accepts_invalid_utf8_in_msvc_names() {
        let ast = parse_bytes(b"?F\xffoo@@YAXXZ").unwrap();
        assert_eq!(ast.dialect(), Dialect::Msvc);
        let texts = texts_of(&ast, Kind::Identifier);
        assert!(texts.iter().any(|text| text == "F\\xffoo"), "{texts:?}");
    }

    #[cfg(feature = "cpp-itanium")]
    #[test]
    fn byte_path_keeps_valid_text_unchanged() {
        for ast in [
            parse_bytes(b"_ZN3Foo3barEv").unwrap(),
            parse("_ZN3Foo3barEv").unwrap(),
        ] {
            let texts = texts_of(&ast, Kind::Identifier);
            assert!(texts.iter().any(|text| text == "Foo"), "{texts:?}");
            assert!(texts.iter().any(|text| text == "bar"), "{texts:?}");
        }
    }

    /// The C++ symbols the budget tests parse, with the scheme each reports.
    fn budget_symbols() -> alloc::vec::Vec<(&'static str, Scheme)> {
        alloc::vec![
            #[cfg(feature = "cpp-itanium")]
            ("_Z1fPi", Scheme::CppItanium),
            #[cfg(feature = "cpp-msvc")]
            ("?bar@Foo@@QEAAXXZ", Scheme::CppMsvc),
            #[cfg(feature = "cpp-gnu-v2")]
            ("foo__Fiv", Scheme::CppGnuV2),
            #[cfg(feature = "cpp-gnu-v2")]
            ("__ct__3fooFv", Scheme::CppArm),
        ]
    }

    /// One dimension's ceiling from the policy table.
    fn ceiling(dimension: Option<Limit>) -> usize {
        match dimension {
            Some(Limit::Bounded(value)) => value,
            _ => panic!("the scheme must bound this dimension"),
        }
    }

    /// A dimension left unset keeps the parser's compiled-in limit, so the
    /// public entry points are exactly the pre-budget behaviour.
    #[test]
    fn the_default_budget_keeps_the_compiled_in_limits() {
        #[cfg(feature = "cpp-itanium")]
        {
            let limits = ParseLimits::itanium(DEFAULT_BUDGET);
            assert_eq!(limits.input(), MAX_INPUT_LEN);
            assert_eq!(limits.depth(), parser::MAX_DEPTH);
            assert_eq!(limits.work(), DEFAULT_WORK);
            assert_eq!(limits.nodes(), parser::MAX_NODES);
            assert_eq!(
                limits,
                ParseLimits::itanium(Budget::default().effective(Scheme::CppItanium))
            );
        }
        #[cfg(feature = "cpp-msvc")]
        {
            let limits = ParseLimits::msvc(DEFAULT_BUDGET);
            assert_eq!(limits.input(), MAX_INPUT_LEN);
            assert_eq!(limits.depth(), msvc::MAX_DEPTH);
            assert_eq!(limits.work(), DEFAULT_WORK);
            assert_eq!(limits.nodes(), msvc::MAX_NODES);
            assert_eq!(
                limits,
                ParseLimits::msvc(Budget::default().effective(Scheme::CppMsvc))
            );
        }
        #[cfg(feature = "cpp-gnu-v2")]
        {
            let limits = ParseLimits::legacy(DEFAULT_BUDGET);
            assert_eq!(limits.input(), MAX_INPUT_LEN);
            assert_eq!(limits.depth(), legacy::MAX_DEPTH);
            assert_eq!(limits.work(), DEFAULT_WORK);
            assert_eq!(limits.nodes(), legacy::MAX_NODES);
            for scheme in [
                Scheme::CppGnuV2,
                Scheme::CppArm,
                Scheme::CppEdg,
                Scheme::CppHp,
            ] {
                assert_eq!(
                    limits,
                    ParseLimits::legacy(Budget::default().effective(scheme)),
                    "{scheme}"
                );
            }
        }
    }

    /// The public entry point parses exactly what the default budget parses.
    #[test]
    fn public_parse_matches_the_default_budget() {
        for (symbol, scheme) in budget_symbols() {
            let public = parse(symbol).unwrap_or_else(|error| panic!("{symbol}: {error}"));
            let budgeted =
                parse_bytes_with_budget(symbol.as_bytes(), Budget::new().effective(scheme))
                    .unwrap_or_else(|error| panic!("{symbol}: {error}"));
            assert_eq!(public.dialect(), budgeted.dialect(), "{symbol}");
            assert_eq!(format!("{public}"), format!("{budgeted}"), "{symbol}");
            let effective = Budget::new().effective(scheme);
            assert_eq!(
                public.print_envelope_ok(effective),
                budgeted.print_envelope_ok(effective),
                "{symbol}"
            );
        }
    }

    /// Every parser dimension rejects one unit below the value a symbol needs
    /// and accepts at that value.
    #[test]
    fn lowering_each_dimension_rejects_at_the_boundary() {
        fn assert_boundary(
            symbol: &str,
            scheme: Scheme,
            name: &str,
            expected: ParseErrorKind,
            limit: usize,
            budget: &dyn Fn(usize) -> Budget,
        ) {
            let needed = (0..=limit)
                .find(|&value| {
                    parse_bytes_with_budget(symbol.as_bytes(), budget(value).effective(scheme))
                        .is_ok()
                })
                .unwrap_or_else(|| panic!("{symbol}: no {name} within {limit} parses"));
            assert!(needed > 0, "{symbol}: {name} is not bounded");
            let error =
                parse_bytes_with_budget(symbol.as_bytes(), budget(needed - 1).effective(scheme))
                    .unwrap_err();
            assert_eq!(
                error.kind(),
                expected,
                "{symbol}: {name} accepted at {} (needs {needed})",
                needed - 1
            );
        }

        for (symbol, scheme) in budget_symbols() {
            let hard = scheme.hard_budget();
            assert_boundary(
                symbol,
                scheme,
                "input",
                ParseErrorKind::TooLong,
                ceiling(hard.input_bytes),
                &|value| Budget::new().with_input_bytes(value),
            );
            assert_boundary(
                symbol,
                scheme,
                "depth",
                ParseErrorKind::TooDeep,
                ceiling(hard.parse_depth),
                &|value| Budget::new().with_parse_depth(value),
            );
            assert_boundary(
                symbol,
                scheme,
                "work",
                ParseErrorKind::TooMuchWork,
                ceiling(hard.work),
                &|value| Budget::new().with_work(value),
            );
            assert_boundary(
                symbol,
                scheme,
                "nodes",
                ParseErrorKind::TooManyNodes,
                ceiling(hard.nodes),
                &|value| Budget::new().with_nodes(value),
            );
        }
    }

    /// A symbol the default budget rejects for its length is accepted once the
    /// caller raises the input budget within the hard cap.
    #[test]
    #[cfg(feature = "cpp-itanium")]
    fn raising_the_input_budget_accepts_longer_symbols() {
        // One long source name: few nodes, so only the input budget binds.
        let symbol = long_identifier_symbol();
        assert_eq!(parse(&symbol).unwrap_err().kind(), ParseErrorKind::TooLong);

        let budget = Budget::new().with_input_bytes(2 * MAX_INPUT_LEN);
        let ast = parse_bytes_with_budget(symbol.as_bytes(), budget.effective(Scheme::CppItanium))
            .expect("raised input budget");
        assert_eq!(ast.dialect(), Dialect::Itanium);
    }

    /// An Itanium symbol whose only large dimension is one source name.
    #[cfg(feature = "cpp-itanium")]
    fn long_identifier_symbol() -> alloc::string::String {
        let length = MAX_INPUT_LEN + 4 * 1024;
        let mut symbol = alloc::string::String::from("_ZN");
        symbol.push_str(&alloc::string::ToString::to_string(&length));
        symbol.push_str(&"a".repeat(length));
        symbol.push_str("3barEv");
        symbol
    }

    /// Raising the node budget accepts a wider tree; raising the input budget
    /// alone does not, because the two dimensions are independent.
    #[test]
    #[cfg(feature = "cpp-itanium")]
    fn raising_the_node_budget_accepts_wider_trees() {
        let symbol = wide_parameter_symbol();
        assert_eq!(
            parse(&symbol).unwrap_err().kind(),
            ParseErrorKind::TooManyNodes
        );

        let raised = Budget::new().with_nodes(0x8000);
        let ast = parse_bytes_with_budget(symbol.as_bytes(), raised.effective(Scheme::CppItanium))
            .expect("raised node budget");
        assert_eq!(ast.dialect(), Dialect::Itanium);
    }

    /// An Itanium function with more parameters than the default node budget
    /// allows.
    #[cfg(feature = "cpp-itanium")]
    fn wide_parameter_symbol() -> alloc::string::String {
        let mut symbol = alloc::string::String::from("_Z1f");
        symbol.push_str(&"i".repeat(10_000));
        symbol
    }

    /// Over-hard values are clamped, visibly and in the parser limits.
    #[test]
    fn over_hard_budgets_are_clamped_to_the_hard_caps() {
        let budget = Budget::new()
            .with_input_bytes(usize::MAX)
            .with_parse_depth(usize::MAX)
            .with_work(usize::MAX)
            .with_nodes(usize::MAX);
        let itanium = budget.effective(Scheme::CppItanium);
        assert_eq!(itanium.input_bytes, Some(Limit::Bounded(16 * 1024 * 1024)));
        assert_eq!(itanium.parse_depth, Some(Limit::Bounded(256)));
        assert_eq!(itanium.work, Some(Limit::Bounded(64 * 1024 * 1024)));
        assert_eq!(itanium.nodes, Some(Limit::Bounded(1024 * 1024)));
        let legacy = budget.effective(Scheme::CppGnuV2);
        assert_eq!(legacy.parse_depth, Some(Limit::Bounded(64)));

        #[cfg(feature = "cpp-itanium")]
        {
            let limits = ParseLimits::itanium(itanium);
            assert_eq!(limits.input(), 16 * 1024 * 1024);
            // The depth ceiling is the compiled-in depth budget; the demangler
            // applies the separate structural printability gate.
            assert_eq!(limits.depth(), parser::MAX_DEPTH);
            assert_eq!(limits.work(), 64 * 1024 * 1024);
            assert_eq!(limits.nodes(), 1024 * 1024);
        }
        #[cfg(feature = "cpp-gnu-v2")]
        {
            let limits = ParseLimits::legacy(legacy);
            assert_eq!(limits.depth(), legacy::MAX_DEPTH);
            assert_eq!(limits.nodes(), 1024 * 1024);
        }
    }

    /// The print envelope stays fixed policy: raising the node budget parses a
    /// wider tree, but the demangler still rejects one the printers cannot
    /// render.
    #[test]
    #[cfg(feature = "cpp-itanium")]
    fn the_print_envelope_is_not_a_budget() {
        let symbol = wide_parameter_symbol();
        let budget = Budget::new().with_nodes(0x8000).with_expansion_visits(1);
        let effective = budget.effective(Scheme::CppItanium);
        let ast =
            parse_bytes_with_budget(symbol.as_bytes(), effective).expect("raised node budget");
        assert!(!ast.print_envelope_ok(effective));
        assert!(
            Demangler::with_budget(budget)
                .demangle_with_scheme(Scheme::CppItanium, &symbol)
                .is_err()
        );
    }

    /// Zero at a migrated dimension rejects through the forced-scheme and
    /// `Selection` paths.
    #[test]
    fn a_zero_input_budget_rejects_through_demangler_and_selection() {
        let demangler = Demangler::with_budget(Budget::new().with_input_bytes(0));
        for (symbol, scheme) in budget_symbols() {
            assert!(
                demangler.demangle_with_scheme(scheme, symbol).is_err(),
                "{symbol}"
            );
            assert!(
                demangler
                    .selection()
                    .allow(scheme)
                    .demangle(symbol)
                    .is_err(),
                "{symbol}"
            );
        }
    }

    /// A configured budget reaches the parser through the detected path too.
    ///
    /// Detection parses a legacy name and retains
    /// the tree, so the demangler re-parses it under the configured budget
    /// instead of reusing a tree that may exceed it.
    #[test]
    fn a_configured_budget_reaches_the_detected_path() {
        for (symbol, scheme) in budget_symbols() {
            assert!(Demangler::new().demangle(symbol).is_ok(), "{symbol}");
            let tightened = Demangler::with_budget(Budget::new().with_parse_depth(0));
            assert!(tightened.demangle(symbol).is_err(), "{symbol} {scheme}");
        }
    }

    /// Raising the input budget through the demangler, not only through the
    /// backend parse helper.
    #[test]
    #[cfg(feature = "cpp-itanium")]
    fn demangler_raising_the_input_budget_accepts_longer_symbols() {
        let symbol = long_identifier_symbol();
        assert!(Demangler::new().demangle(&symbol).is_err());
        let raised = Demangler::with_budget(Budget::new().with_input_bytes(2 * MAX_INPUT_LEN));
        assert!(raised.demangle(&symbol).is_ok());
    }
}
