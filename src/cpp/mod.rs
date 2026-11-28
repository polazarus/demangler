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
//! and the `.?` RTTI type descriptor name form select the Microsoft parser,
//! and everything else is tried as a legacy C++ name, which is accepted only
//! when a legacy parser consumes the whole symbol. To force one dialect, use
//! [`Demangler::demangle_with_scheme`](crate::Demangler::demangle_with_scheme).
//!
//! The Itanium, Microsoft and legacy printers render this AST as demangled
//! text through [`Ast`](crate::cpp::Ast)'s `Display` implementation.
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
//! [`parse`](crate::cpp::parse) entry point keeps the compiled-in limits (`MAX_INPUT_LEN` and the
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
//! assert!(cpp::is_itanium(b"_ZN3Foo3barEv"));
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
#[cfg(feature = "cpp-msvc")]
use msvc_printer::print_msvc;

#[cfg(any(feature = "cpp-itanium", feature = "cpp-msvc"))]
#[cfg(feature = "cpp-itanium")]
use crate::parse::{Limits, State};
#[cfg(any(feature = "cpp-itanium", feature = "cpp-msvc", feature = "cpp-gnu-v2"))]
use crate::{BudgetKind, ParseError, ParseErrorKind};
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
/// 4096`) and caps it at the resolved work budget, so at `MAX_INPUT_LEN` the
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
#[allow(
    dead_code,
    reason = "only the Itanium and legacy unit tests and wrappers use the default"
)]
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

/// Printed tree levels above the deepest parsed production: the encoding,
/// function type and parameter list that wrap a type the parser counted.
///
/// Without it a symbol at the parser's depth limit would pass the parse and
/// then fail the printer cap, so the backend and the demangler would disagree
/// on where the limit is.
const PRINT_WRAPPER_DEPTH: usize = 16;

/// Deepest structural print depth a printer of `dialect` may recurse to.
///
/// This is the dialect's hard parse-depth ceiling
/// ([`crate::Scheme::hard_budget`]) times its declarator factor, plus
/// [`PRINT_WRAPPER_DEPTH`]: the MSVC printer walks a declarator left and
/// right, so its stack can reach twice the parser's. It follows the hard
/// ceiling, not the caller's `parse_depth`, because it guards a printer's
/// stack, not the parse.
pub(crate) const fn printer_depth_limit(dialect: Dialect) -> usize {
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
        Some(Limit::Bounded(depth)) => depth
            .saturating_mul(factor)
            .saturating_add(PRINT_WRAPPER_DEPTH),
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
        let defaults = scheme.default_overrides();
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

/// True when `symbol` starts with an Itanium mangling prefix (`_Z`, `__Z`,
/// `___Z` or `____Z`).
#[must_use]
pub fn is_itanium(symbol: &[u8]) -> bool {
    prefix_len_bytes(symbol).is_some()
}

/// True when `symbol` starts with a Microsoft C++ mangling prefix: `?`, or
/// `.?` for an RTTI type descriptor name. [`parse`] accepts both.
#[must_use]
pub fn is_msvc(symbol: &[u8]) -> bool {
    symbol.starts_with(b"?") || symbol.starts_with(b".?")
}

/// The mangling scheme an [`Ast`] was parsed from.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[allow(
    dead_code,
    reason = "a single-dialect build never constructs the other variants"
)]
pub(crate) enum Dialect {
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

/// A [`fmt::Write`] sink that drops its input, for [`Ast::renders`].
struct Discard;

impl fmt::Write for Discard {
    fn write_str(&mut self, _: &str) -> fmt::Result {
        Ok(())
    }
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
    /// True when the graph contains an Itanium `Kind::TemplateParam` node,
    /// which the printer resolves against the template arguments in scope.
    has_template_param: bool,
}

impl<'a> Ast<'a> {
    /// Wrap a parsed Itanium root node.
    #[cfg(feature = "cpp-itanium")]
    pub(crate) const fn from_root(
        root: NodeRef<'a>,
        has_lambda: bool,
        has_template_param: bool,
    ) -> Self {
        Self {
            dialect: Dialect::Itanium,
            root,
            has_lambda,
            has_template_param,
        }
    }

    /// Wrap a parsed Microsoft root node.
    #[cfg(feature = "cpp-msvc")]
    pub(crate) const fn from_root_msvc(root: NodeRef<'a>) -> Self {
        Self {
            dialect: Dialect::Msvc,
            root,
            has_lambda: false,
            has_template_param: false,
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
            has_template_param: false,
        }
    }

    /// True when the graph contains a lambda node.
    #[cfg(feature = "cpp-itanium")]
    #[must_use]
    pub(crate) const fn has_lambda(&self) -> bool {
        self.has_lambda
    }

    /// True when the printer renders this tree.
    ///
    /// A [`crate::Demangler`] only returns trees that print. Every parser
    /// resolves its references while parsing except for Itanium template
    /// parameters, which the printer resolves against the arguments in scope;
    /// one with no argument to name is found only when printing. A tree that
    /// holds such a reference is therefore rendered once into a discarding
    /// sink; the envelope check bounds that work. The corpus mutation sweep
    /// checks that every other accepted tree prints.
    pub(crate) fn renders(&self) -> bool {
        !self.has_template_param || fmt::write(&mut Discard, format_args!("{self}")).is_ok()
    }

    /// Check that this tree fits the conservative printer envelope for
    /// `budget` and renders, naming the gate that rejects it.
    ///
    /// The envelope checks the resolved expansion budgets against the tree's
    /// expanded visits (times [`PRINT_VISIT_FACTOR`]) and rendered bytes, and
    /// bounds the printer's recursion by [`printer_depth_limit`].
    pub(crate) fn printable(&self, budget: EffectiveBudget) -> Result<(), ParseError> {
        use crate::ParseErrorKind as Kind;
        let reject = |kind| Err(ParseError::new(kind));
        let bound = |limit| match limit {
            Some(Limit::Bounded(value)) => value,
            _ => usize::MAX,
        };
        if !self.printer_depth_ok() {
            return reject(Kind::Budget(BudgetKind::ParseDepth));
        }
        if self.expanded_visits() > bound(budget.expansion_visits) {
            return reject(Kind::Budget(BudgetKind::ExpansionVisits));
        }
        if self.root.print_bytes() > bound(budget.expansion_bytes) {
            return reject(Kind::Budget(BudgetKind::ExpansionBytes));
        }
        if !self.renders() {
            return reject(Kind::Invalid);
        }
        Ok(())
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

    /// The C++ scheme this tree was parsed from.
    #[must_use]
    pub const fn scheme(&self) -> Scheme {
        crate::scheme::scheme_for_cpp_dialect(self.dialect)
    }

    /// The parser dialect behind [`Self::scheme`].
    #[must_use]
    pub(crate) const fn dialect(&self) -> Dialect {
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
            .field("has_template_param", &self.has_template_param)
            .field("root", &self.root)
            .finish()
    }
}

impl fmt::Display for Ast<'_> {
    /// Render the parsed symbol as demangled C++ text.
    ///
    /// The text follows the LLVM demangler's conventions. A tree from
    /// [`parse`] can still fail to render — for example when a template
    /// parameter has no argument to name — and that is reported as an error;
    /// a [`crate::Demangler`] returns only trees that render. Each call
    /// re-renders the retained tree; cache the text if it is printed
    /// repeatedly.
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
/// `budget` overrides the compiled-in limits per dimension, exactly as on a
/// [`Demangler`](crate::Demangler): [`Budget::default`](crate::Budget::default)
/// keeps them, and every value is clamped to
/// [`Scheme::hard_budget`](crate::Scheme::hard_budget).
///
/// # Errors
///
/// Returns [`ParseError`] when the symbol is not a recognised C++ mangling,
/// uses an unsupported production, is malformed, or exceeds the parser's
/// depth, node or work budgets. A tree outside the print envelope (expansion
/// visits or bytes, printer depth) is a budget error, and one that names a
/// template parameter with no argument is
/// [`Invalid`](crate::ParseErrorKind::Invalid): every returned tree prints.
pub fn parse(symbol: &[u8], budget: crate::Budget) -> Result<Ast<'_>, ParseError> {
    // A GNU `@`/`@@` symbol version is a linker suffix outside Microsoft
    // names, where `@` is grammar. A ThinLTO `.llvm.<hex>` ending is kept: the
    // Itanium printer renders it as a clone suffix, as LLVM does.
    let symbol = if symbol.starts_with(b"?") || symbol.starts_with(b".?") {
        symbol
    } else {
        crate::scheme::strip_version_suffix_bytes(symbol)
    };
    let effective = budget.effective(dispatch_scheme(symbol));
    let ast = parse_bytes_with_budget(symbol, effective)?;
    ast.printable(effective)?;
    Ok(ast)
}

/// The C++ scheme whose budget applies to `symbol`, chosen by the same prefix
/// dispatch as [`parse_bytes_with_budget`].
#[allow(
    clippy::missing_const_for_fn,
    reason = "const only when the Itanium and Microsoft checks are compiled out"
)]
fn dispatch_scheme(symbol: &[u8]) -> Scheme {
    #[cfg(feature = "cpp-itanium")]
    if prefix_len_bytes(symbol).is_some() || is_alloc_token_bytes(symbol) {
        return Scheme::CppItanium;
    }
    #[cfg(feature = "cpp-msvc")]
    if is_msvc(symbol) {
        return Scheme::CppMsvc;
    }
    let _ = symbol;
    Scheme::CppGnuV2
}

/// Parse a C++ mangled name from raw bytes with one scheme's resolved budget.
///
/// [`parse`] wraps this with [`DEFAULT_BUDGET`]; a [`crate::Demangler`]
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
            .ok_or(ParseError::new(ParseErrorKind::NotThisScheme))?;
        let limits = ParseLimits::itanium(budget);
        let mut state = itanium_state(limits.input());
        return Parser::new_alloc_token(symbol, offset + prefix, &mut state, limits)?.parse();
    }
    #[cfg(feature = "cpp-msvc")]
    if is_msvc(symbol) {
        return msvc::parse_msvc_bytes_with_budget(symbol, budget);
    }
    #[cfg(feature = "cpp-gnu-v2")]
    if legacy::looks_like_legacy_bytes(symbol) {
        return legacy::parse_bytes_with_budget(symbol, budget);
    }
    Err(ParseError::new(ParseErrorKind::NotThisScheme))
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
mod tests;
