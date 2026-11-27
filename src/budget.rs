use crate::scheme::Scheme;

/// Maximum accepted input length in bytes.
///
/// Every backend uses the same default, so one constant names it.
const DEFAULT_INPUT: usize = 64 * 1024;

/// Work allowance per input byte the C++ parsers charge.
const CPP_WORK_PER_BYTE: usize = 32;

/// Minimum work allowance for very short C++ symbols.
const CPP_MIN_WORK: usize = 1 << 12;

/// Work the C++ parsers derive for a symbol at the default input cap.
///
/// The Itanium, Microsoft and legacy parsers derive their allowance from the
/// input length (`32 × len + 4096`) rather than from a fixed constant, so the
/// reported default is that allowance at [`DEFAULT_INPUT`].
const CPP_DEFAULT_WORK: usize = DEFAULT_INPUT * CPP_WORK_PER_BYTE + CPP_MIN_WORK;

/// Default work allowance of the shared-cursor backends: eight units per byte.
const DEFAULT_WORK: usize = 8 * DEFAULT_INPUT;

/// Default work allowance of the D parser: sixteen units per byte.
const D_DEFAULT_WORK: usize = 16 * DEFAULT_INPUT;

/// Default node allowance of the Itanium and Microsoft parsers.
const CPP_DEFAULT_NODES: usize = 1 << 14;

/// Default node allowance of the legacy C++ parsers.
const LEGACY_DEFAULT_NODES: usize = 1 << 12;

/// Default node allowance of the Rust parsers.
const RUST_DEFAULT_NODES: usize = 1 << 16;

/// Default node allowance of the Swift parsers.
const SWIFT_DEFAULT_NODES: usize = 1 << 19;

/// Default node allowance of the D parser: four nodes per input byte.
const D_DEFAULT_NODES: usize = 4 * DEFAULT_INPUT;

/// Default expansion visit allowance of the Rust parsers.
const RUST_DEFAULT_EXPANSION_VISITS: usize = 4_000_000;

/// Default expansion visit allowance of the Swift parsers.
const SWIFT_DEFAULT_EXPANSION_VISITS: usize = 1 << 17;

/// Default expansion visit allowance of the Itanium and Microsoft parsers.
const CPP_DEFAULT_EXPANSION_VISITS: usize = 1 << 18;

/// Default expansion visit allowance of the legacy C++ parsers.
const LEGACY_DEFAULT_EXPANSION_VISITS: usize = 1 << 16;

/// Default expansion visit allowance of the D parser.
const D_DEFAULT_EXPANSION_VISITS: usize = 1 << 18;

/// Default rendered-byte estimate of the shared-DAG parsers: 1 MiB.
const DEFAULT_EXPANSION_BYTES: usize = 1 << 20;

/// Default rendered-byte estimate of the D parser.
const D_DEFAULT_EXPANSION_BYTES: usize = 1 << 18;

/// Generic hard input cap.
const HARD_INPUT: usize = 16 * 1024 * 1024;

/// Hard input cap for rustc v0, derived from its per-byte memory use.
const HARD_INPUT_RUST_V0: usize = 1024 * 1024;

/// Hard depth cap for the recursive parsers.
const HARD_DEPTH: usize = 1024;

/// Rust hard depth: the recursive printer descends about two frames per
/// level, so 512 keeps it within the generic 1024 stack budget; the print
/// depth gate is this same ceiling.
const HARD_DEPTH_RUST: usize = 512;

/// Hard work cap.
const HARD_WORK: usize = 64 * 1024 * 1024;

/// Hard node cap.
const HARD_NODES: usize = 1024 * 1024;

/// Hard expansion visit cap of the shared-DAG parsers.
const HARD_EXPANSION_VISITS: usize = 16_000_000;

/// Hard expansion visit cap of the D parser.
const D_HARD_EXPANSION_VISITS: usize = 1_000_000;

/// Hard rendered-byte estimate: 64 MiB.
const HARD_EXPANSION_BYTES: usize = 64 * 1024 * 1024;

/// A resolved cap for one budget dimension.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Limit {
    /// The dimension is capped at this many units.
    Bounded(usize),
    /// The scheme's backend does not cap this dimension, so no override can
    /// tighten it.
    Unbounded,
}

/// The limits the demangler resolves for one [`Scheme`].
///
/// [`Budget::effective`] and
/// [`Demangler::effective_budget`](crate::Demangler::effective_budget) return
/// the values after clamping. A dimension is `None` when the scheme has no such
/// dimension at all, [`Limit::Unbounded`] when the backend does not cap it, and
/// [`Limit::Bounded`] otherwise.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct EffectiveBudget {
    /// Maximum accepted input length in bytes.
    pub input_bytes: Option<Limit>,
    /// Maximum parser nesting depth.
    pub parse_depth: Option<Limit>,
    /// Maximum parser work units, in the backend's own unit.
    pub work: Option<Limit>,
    /// Maximum nodes one parse may allocate.
    pub nodes: Option<Limit>,
    /// Maximum expansion visits one accepted tree may reach, counted at parse
    /// time.
    pub expansion_visits: Option<Limit>,
    /// Maximum rendered-byte estimate one accepted tree may reach, checked at
    /// parse time.
    pub expansion_bytes: Option<Limit>,
}

/// Capacity policy for one [`crate::Demangler`].
///
/// Every field is an override: `None` keeps the backend default for the scheme
/// being demangled, so [`Budget::default`] is exactly the compiled-in
/// behaviour. A value is clamped to the scheme's [`Scheme::hard_budget`]
/// ceiling. `0` rejects at a bounded dimension; a dimension the scheme does not
/// have or does not cap ignores the override.
///
/// The values are denial-of-service bounds, not ABI contracts. `work` is an
/// opaque backend-defined unit and is not comparable between schemes. Printing
/// is not a budget, so there is no print field: the two expansion gates are
/// parse-time bounds on what an accepted tree may expand to, not print limits.
///
/// # Semantics
///
/// - `None` keeps the backend default, which is byte-identical to the
///   compiled-in behaviour.
/// - `Some(value)` is clamped to `min(value, hard cap)` for the scheme. `0`
///   rejects at a bounded dimension; a dimension the backend does not have or
///   does not cap ignores the override.
/// - A dimension the backend has but does not cap is reported as
///   [`Limit::Unbounded`]; an override cannot tighten it. A dimension the
///   scheme does not have at all (flat grammars have no depth/node dimension;
///   plain C and unknown names have no parser) is reported as `None`.
/// - Printing is not a budget. Shared-DAG printers keep a fixed parse-time
///   envelope and linear printers rely on their parser bounds; no [`Budget`]
///   field configures a printer.
///
/// # Expansion gates
///
/// `expansion_visits` and `expansion_bytes` are parse-time gates for the
/// schemes whose spelling can expand: the shared-DAG backends (Rust, Swift and
/// C++), where one node is printed once per reference, and D, whose back
/// references and modifiers re-expand earlier input. The gates resolve like the
/// parser dimensions and every one of those backends enforces them: a tree
/// whose expanded visits or rendered bytes exceed them is rejected at parse
/// time. Output a backend renders while parsing (Swift legacy specialization
/// payloads, HP aCC literal arguments) and output that grows with a count read
/// from the input (Swift generic parameter names) are charged to the same
/// gates. The linear backends print each node once, so they have no expansion
/// gate at all and report `None` for both dimensions.
///
/// # No other limits
///
/// A backend has no fixed limit outside these dimensions. List lengths
/// (segments, parameters, tuple elements, suffixes, split points) are bounded
/// by the input, work or node budget, because every element consumes input or
/// is a counted node; nested re-parses share the containing parse's budget;
/// repeat counts read from the input are charged as work. What remains fixed
/// is the grammar itself (MSVC's ten digit back-references, Rust v0's 64-bit
/// integers) and the printer stack bound, which every shared-DAG backend
/// derives from its hard parse-depth ceiling.
///
/// # Hard caps
///
/// The hard caps are provisional: they bound stack use, allocations and CPU
/// until per-backend stack and memory measurements justify tighter numbers.
/// Three constraints are already known:
///
/// - rustc v0 keeps a checkpoint and a cache entry per input byte, so its hard
///   input cap is memory-derived (1 MiB) instead of the generic 16 MiB.
/// - The C++ and Swift print envelopes bound the nesting a parser can usefully
///   accept, so their hard depth caps stay at the envelope's parser depth.
///
/// `work` is an opaque backend-defined unit; it is not comparable between
/// schemes.
///
/// # Detection
///
/// [`Scheme::detect`](crate::Scheme::detect) keeps the compiled-in budget.
/// [`Demangler`](crate::Demangler) uses a budget-aware detection pass: parse-based prefilters try
/// the configured limits first and fall back to the defaults only for
/// classification when that parse fails, so a too-small budget cannot silently
/// turn a known scheme into plain C. The dispatch then re-parses any retained
/// tree under the configured budget, so enforcement is exact.
///
/// # Example
///
/// ```rust
/// use demangler::{Budget, Demangler, Limit, Scheme};
///
/// let demangler = Demangler::with_budget(Budget::new().with_input_bytes(8 * 1024));
/// let effective = demangler.effective_budget(Scheme::CppItanium);
///
/// assert_eq!(effective.input_bytes, Some(Limit::Bounded(8 * 1024)));
/// assert_eq!(Scheme::CppItanium.default_budget().input_bytes, Some(64 * 1024));
/// ```
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct Budget {
    /// Maximum accepted input length in bytes.
    pub input_bytes: Option<usize>,
    /// Maximum parser nesting depth.
    pub parse_depth: Option<usize>,
    /// Maximum parser work units, in the backend's own unit.
    pub work: Option<usize>,
    /// Maximum nodes one parse may allocate.
    pub nodes: Option<usize>,
    /// Maximum expansion visits one accepted tree may reach, counted at parse
    /// time. Not a printer cap.
    pub expansion_visits: Option<usize>,
    /// Maximum rendered-byte estimate one accepted tree may reach, checked at
    /// parse time. Not a printer cap.
    pub expansion_bytes: Option<usize>,
}

impl Budget {
    /// A budget with every dimension left at its backend default.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            input_bytes: None,
            parse_depth: None,
            work: None,
            nodes: None,
            expansion_visits: None,
            expansion_bytes: None,
        }
    }

    /// Override the maximum accepted input length in bytes.
    #[must_use]
    pub const fn with_input_bytes(mut self, bytes: usize) -> Self {
        self.input_bytes = Some(bytes);
        self
    }

    /// Override the maximum parser nesting depth.
    #[must_use]
    pub const fn with_parse_depth(mut self, depth: usize) -> Self {
        self.parse_depth = Some(depth);
        self
    }

    /// Override the maximum parser work units.
    #[must_use]
    pub const fn with_work(mut self, work: usize) -> Self {
        self.work = Some(work);
        self
    }

    /// Override the maximum number of nodes one parse may allocate.
    #[must_use]
    pub const fn with_nodes(mut self, nodes: usize) -> Self {
        self.nodes = Some(nodes);
        self
    }

    /// Override the maximum expansion visits one accepted tree may reach.
    ///
    /// Only the shared-DAG backends (Rust, Swift and C++) and D have this
    /// parse-time gate; the linear backends ignore the override. It is not a
    /// printer cap.
    #[must_use]
    pub const fn with_expansion_visits(mut self, visits: usize) -> Self {
        self.expansion_visits = Some(visits);
        self
    }

    /// Override the maximum rendered-byte estimate one accepted tree may
    /// reach.
    ///
    /// Only the shared-DAG backends (Rust, Swift and C++) and D have this
    /// parse-time gate; the linear backends ignore the override. It is not a
    /// printer cap.
    #[must_use]
    pub const fn with_expansion_bytes(mut self, bytes: usize) -> Self {
        self.expansion_bytes = Some(bytes);
        self
    }

    /// The limits `scheme` enforces under this budget.
    ///
    /// Each override is clamped to the scheme's [`Scheme::hard_budget`]; an
    /// unset dimension keeps [`Scheme::default_budget`]. A dimension the scheme
    /// does not have stays `None`, and one its backend does not cap stays
    /// [`Limit::Unbounded`].
    #[must_use]
    pub fn effective(self, scheme: Scheme) -> EffectiveBudget {
        let default = scheme.default_budget();
        let hard = scheme.hard_budget();
        EffectiveBudget {
            input_bytes: clamp_limit(self.input_bytes, default.input_bytes, hard.input_bytes),
            parse_depth: clamp_limit(self.parse_depth, default.parse_depth, hard.parse_depth),
            work: clamp_limit(self.work, default.work, hard.work),
            nodes: clamp_limit(self.nodes, default.nodes, hard.nodes),
            expansion_visits: clamp_limit(
                self.expansion_visits,
                default.expansion_visits,
                hard.expansion_visits,
            ),
            expansion_bytes: clamp_limit(
                self.expansion_bytes,
                default.expansion_bytes,
                hard.expansion_bytes,
            ),
        }
    }
}

/// Resolve one effective dimension to a concrete backend limit.
///
/// A bounded value is clamped to `ceiling`; an unbounded or absent dimension
/// keeps the backend's compiled-in `default`, which is itself clamped. The
/// C++ backends clamp against their own per-dialect tables instead, so this is
/// compiled only with another backend.
#[cfg(any(
    feature = "rust",
    feature = "swift",
    feature = "go",
    feature = "objc",
    feature = "kotlin-native",
    feature = "d",
    feature = "jni",
    feature = "ada",
    feature = "fortran",
    feature = "delphi",
    feature = "ocaml"
))]
pub fn clamp(dimension: Option<Limit>, default: usize, ceiling: usize) -> usize {
    match dimension {
        Some(Limit::Bounded(value)) => value.min(ceiling),
        Some(Limit::Unbounded) | None => default.min(ceiling),
    }
}

/// Clamp one dimension against the scheme's default and hard cap.
fn clamp_limit(over: Option<usize>, default: Option<usize>, hard: Option<Limit>) -> Option<Limit> {
    match hard {
        None => None,
        Some(Limit::Unbounded) => Some(Limit::Unbounded),
        // Every bounded hard cap has a default (checked by the policy-table
        // test), so `over.or(default)` is always set here.
        Some(Limit::Bounded(ceiling)) => over
            .or(default)
            .map(|requested| Limit::Bounded(requested.min(ceiling))),
    }
}

/// Build a [`Budget`] from its four parser dimensions.
///
/// The two expansion gates stay unset; [`expansion`] adds them for the schemes
/// that have them.
const fn policy(
    input_bytes: Option<usize>,
    parse_depth: Option<usize>,
    work: Option<usize>,
    nodes: Option<usize>,
) -> Budget {
    Budget {
        input_bytes,
        parse_depth,
        work,
        nodes,
        expansion_visits: None,
        expansion_bytes: None,
    }
}

/// Attach the two expansion gates to one [`Budget`] policy row.
const fn expansion(budget: Budget, visits: usize, bytes: usize) -> Budget {
    Budget {
        expansion_visits: Some(visits),
        expansion_bytes: Some(bytes),
        ..budget
    }
}

/// Build an [`EffectiveBudget`] from its four parser dimensions.
///
/// The two expansion ceilings stay unset; [`hard_expansion`] adds them for the
/// schemes that have them.
const fn hard_policy(
    input_bytes: Option<Limit>,
    parse_depth: Option<Limit>,
    work: Option<Limit>,
    nodes: Option<Limit>,
) -> EffectiveBudget {
    EffectiveBudget {
        input_bytes,
        parse_depth,
        work,
        nodes,
        expansion_visits: None,
        expansion_bytes: None,
    }
}

/// Attach the two expansion ceilings to one [`EffectiveBudget`] policy row.
const fn hard_expansion(hard: EffectiveBudget, visits: usize, bytes: usize) -> EffectiveBudget {
    EffectiveBudget {
        expansion_visits: Some(Limit::Bounded(visits)),
        expansion_bytes: Some(Limit::Bounded(bytes)),
        ..hard
    }
}

/// The generic hard input and work caps with a scheme-specific depth and node
/// dimension.
///
/// The two expansion ceilings stay unset; [`hard_expansion`] adds them for the
/// schemes that have them.
const fn hard_caps(parse_depth: usize, nodes: Option<Limit>) -> EffectiveBudget {
    hard_policy(
        Some(Limit::Bounded(HARD_INPUT)),
        Some(Limit::Bounded(parse_depth)),
        Some(Limit::Bounded(HARD_WORK)),
        nodes,
    )
}

impl Scheme {
    /// The compiled-in budget this scheme enforces today.
    ///
    /// Every dimension names the constant its backend uses, so the values do
    /// not depend on the enabled features. Plain C and unknown names are
    /// pass-through text with no parser, so every dimension is `None`.
    /// This returns the override-free [`Budget`], so an unset dimension is
    /// `None` whether the scheme lacks it or leaves it uncapped.
    /// [`Budget::effective`] distinguishes the two: it reports `None` for a
    /// dimension the scheme does not have and [`Limit::Unbounded`] for one the
    /// backend has but does not cap.
    ///
    /// The C++ parsers derive their work allowance from the input length; the
    /// reported `work` is that allowance at the default 64 KiB input cap. A
    /// work override caps the derived allowance, so it can only tighten it:
    /// raising the input budget needs a raised work budget as well.
    ///
    /// The expansion gates are `Some` for the shared-DAG backends (Rust, Swift
    /// and C++) and D, whose spelling can expand one parsed node or one back
    /// reference into many rendered bytes, and `None` for the linear backends
    /// and the pass-through schemes.
    #[must_use]
    #[expect(clippy::match_same_arms, reason = "one policy row per scheme")]
    pub const fn default_budget(self) -> Budget {
        match self {
            Self::C | Self::Unknown => Budget::new(),
            Self::CppItanium | Self::CppMsvc => expansion(
                policy(
                    Some(DEFAULT_INPUT),
                    Some(256),
                    Some(CPP_DEFAULT_WORK),
                    Some(CPP_DEFAULT_NODES),
                ),
                CPP_DEFAULT_EXPANSION_VISITS,
                DEFAULT_EXPANSION_BYTES,
            ),
            Self::CppGnuV2 | Self::CppArm | Self::CppEdg | Self::CppHp => expansion(
                policy(
                    Some(DEFAULT_INPUT),
                    Some(64),
                    Some(CPP_DEFAULT_WORK),
                    Some(LEGACY_DEFAULT_NODES),
                ),
                LEGACY_DEFAULT_EXPANSION_VISITS,
                DEFAULT_EXPANSION_BYTES,
            ),
            Self::RustLegacy => expansion(
                policy(
                    Some(DEFAULT_INPUT),
                    None,
                    Some(200_000),
                    Some(RUST_DEFAULT_NODES),
                ),
                RUST_DEFAULT_EXPANSION_VISITS,
                DEFAULT_EXPANSION_BYTES,
            ),
            Self::RustV0 => expansion(
                policy(
                    Some(DEFAULT_INPUT),
                    Some(500),
                    Some(200_000),
                    Some(RUST_DEFAULT_NODES),
                ),
                RUST_DEFAULT_EXPANSION_VISITS,
                DEFAULT_EXPANSION_BYTES,
            ),
            Self::SwiftStable | Self::SwiftLegacy => expansion(
                policy(
                    Some(DEFAULT_INPUT),
                    Some(96),
                    Some(200_000),
                    Some(SWIFT_DEFAULT_NODES),
                ),
                SWIFT_DEFAULT_EXPANSION_VISITS,
                DEFAULT_EXPANSION_BYTES,
            ),
            Self::Go => policy(Some(DEFAULT_INPUT), Some(32), Some(DEFAULT_WORK), None),
            Self::ObjC => policy(Some(DEFAULT_INPUT), Some(1), Some(DEFAULT_WORK), None),
            Self::KotlinNative => policy(Some(DEFAULT_INPUT), Some(32), Some(DEFAULT_WORK), None),
            Self::D => expansion(
                policy(
                    Some(DEFAULT_INPUT),
                    Some(64),
                    Some(D_DEFAULT_WORK),
                    Some(D_DEFAULT_NODES),
                ),
                D_DEFAULT_EXPANSION_VISITS,
                D_DEFAULT_EXPANSION_BYTES,
            ),
            Self::Jni => policy(Some(DEFAULT_INPUT), Some(32), Some(DEFAULT_WORK), None),
            Self::Ada | Self::Fortran | Self::Delphi | Self::OCaml => {
                policy(Some(DEFAULT_INPUT), None, Some(DEFAULT_WORK), None)
            }
        }
    }

    /// The absolute maximum an override may reach for this scheme.
    ///
    /// The caps are provisional and bound stack use, allocations and CPU.
    /// Plain C and unknown names have no parser, so every dimension is `None`.
    /// A dimension the backend does not cap is [`Limit::Unbounded`]. Backends
    /// keep no fixed limit outside these dimensions.
    ///
    /// Some ceilings are tighter than the generic ones on purpose: rustc v0
    /// allocates a checkpoint and a cache entry per input byte, the C++ and
    /// Swift print envelopes bound the nesting their parsers can usefully
    /// accept, and Objective-C has a single bracket level.
    /// The D parser expands a back reference into at most a bounded node count,
    /// so its expansion visit ceiling is tighter than the shared-DAG one.
    #[must_use]
    pub const fn hard_budget(self) -> EffectiveBudget {
        match self {
            Self::C | Self::Unknown => hard_policy(None, None, None, None),
            Self::CppItanium | Self::CppMsvc => hard_expansion(
                hard_caps(256, Some(Limit::Bounded(HARD_NODES))),
                HARD_EXPANSION_VISITS,
                HARD_EXPANSION_BYTES,
            ),
            Self::CppGnuV2 | Self::CppArm | Self::CppEdg | Self::CppHp => hard_expansion(
                hard_caps(64, Some(Limit::Bounded(HARD_NODES))),
                HARD_EXPANSION_VISITS,
                HARD_EXPANSION_BYTES,
            ),
            Self::RustLegacy => hard_expansion(
                hard_policy(
                    Some(Limit::Bounded(HARD_INPUT)),
                    None,
                    Some(Limit::Bounded(HARD_WORK)),
                    Some(Limit::Bounded(HARD_NODES)),
                ),
                HARD_EXPANSION_VISITS,
                HARD_EXPANSION_BYTES,
            ),
            Self::RustV0 => hard_expansion(
                hard_policy(
                    Some(Limit::Bounded(HARD_INPUT_RUST_V0)),
                    Some(Limit::Bounded(HARD_DEPTH_RUST)),
                    Some(Limit::Bounded(HARD_WORK)),
                    Some(Limit::Bounded(HARD_NODES)),
                ),
                HARD_EXPANSION_VISITS,
                HARD_EXPANSION_BYTES,
            ),
            Self::SwiftStable | Self::SwiftLegacy => hard_expansion(
                hard_caps(128, Some(Limit::Bounded(HARD_NODES))),
                HARD_EXPANSION_VISITS,
                HARD_EXPANSION_BYTES,
            ),
            Self::Go | Self::KotlinNative | Self::Jni => {
                hard_caps(HARD_DEPTH, Some(Limit::Unbounded))
            }
            Self::ObjC => hard_caps(1, Some(Limit::Unbounded)),
            Self::D => hard_expansion(
                hard_caps(HARD_DEPTH, Some(Limit::Bounded(HARD_NODES))),
                D_HARD_EXPANSION_VISITS,
                HARD_EXPANSION_BYTES,
            ),
            Self::Ada | Self::Fortran | Self::Delphi | Self::OCaml => hard_policy(
                Some(Limit::Bounded(HARD_INPUT)),
                None,
                Some(Limit::Bounded(HARD_WORK)),
                None,
            ),
        }
    }
}

#[cfg(test)]
mod tests;
