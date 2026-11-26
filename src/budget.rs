//! Runtime-configurable demangling budgets.
//!
//! Every backend bounds untrusted input with compiled-in limits: input bytes,
//! nesting depth, work units and allocated nodes. [`Budget`] carries caller
//! overrides for one [`crate::Demangler`], [`Scheme::default_budget`] and
//! [`Scheme::hard_budget`] expose the compiled-in defaults and ceilings, and
//! [`Budget::effective`] resolves one override set for one scheme.
//!
//! # Staged migration (WIP)
//!
//! The policy types, the per-scheme tables and the [`Demangler`] storage are in
//! place. The **Rust** (`Scheme::RustLegacy`, `Scheme::RustV0`), **Swift**
//! (`Scheme::SwiftStable`, `Scheme::SwiftLegacy`), **C++** (`Scheme::Cpp*`) and
//! **flat-grammar** (`Scheme::Ada`, `Scheme::Fortran`, `Scheme::Delphi`,
//! `Scheme::OCaml`) backends read the overrides; every other backend still
//! enforces its compiled-in defaults, so for them a [`Budget`] records and
//! clamps what the migrated backends will use without changing today's output
//! or rejections. The remaining backends are migrated dimension by dimension in
//! later stages.
//!
//! Detection is still default-only: [`Scheme::detect`] and the parse-based
//! prefilters it runs use the compiled-in budget. A [`Demangler`] resolves the
//! configured budget for the scheme it dispatches and re-parses a tree a
//! prefilter retained with the defaults when a budget is configured.
//!
//! # Semantics
//!
//! - `None` keeps the backend default, which is byte-identical to the
//!   compiled-in behaviour.
//! - `Some(value)` is clamped to `min(value, hard cap)` for the scheme. `0`
//!   asks a migrated backend to reject every input at that dimension.
//! - A dimension the backend does not cap is reported as [`Limit::Unbounded`];
//!   an override cannot tighten it. A dimension the scheme does not have at all
//!   (plain C and unknown names have no parser) is reported as `None`.
//! - Printing is not a budget. Shared-DAG printers keep a fixed parse-time
//!   envelope and linear printers rely on their parser bounds; no [`Budget`]
//!   field configures a printer.
//!
//! # Hard caps
//!
//! The hard caps are provisional: they bound stack use, allocations and CPU
//! until per-backend stack and memory measurements justify tighter numbers.
//! Two constraints are already known:
//!
//! - rustc v0 keeps a checkpoint and a cache entry per input byte, so its hard
//!   input cap is memory-derived (1 MiB) instead of the generic 16 MiB.
//! - The C++ and Swift print envelopes bound the nesting a parser can usefully
//!   accept, so their hard depth caps stay at the envelope's parser depth.
//!
//! `work` is an opaque backend-defined unit; it is not comparable between
//! schemes.

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
const CPP_DEFAULT_WORK: u32 = (DEFAULT_INPUT * CPP_WORK_PER_BYTE + CPP_MIN_WORK) as u32;

/// Default work allowance of the shared-cursor backends: eight units per byte.
const DEFAULT_WORK: u32 = 8 * DEFAULT_INPUT as u32;

/// Default work allowance of the D parser: sixteen units per byte.
const D_DEFAULT_WORK: u32 = 16 * DEFAULT_INPUT as u32;

/// Default node allowance of the Itanium and Microsoft parsers.
const CPP_DEFAULT_NODES: u32 = 1 << 14;

/// Default node allowance of the legacy C++ parsers.
const LEGACY_DEFAULT_NODES: u32 = 1 << 12;

/// Default node allowance of the Rust parsers.
const RUST_DEFAULT_NODES: u32 = 1 << 16;

/// Default node allowance of the Swift parsers.
const SWIFT_DEFAULT_NODES: u32 = 1 << 19;

/// Default node allowance of the D parser: four nodes per input byte.
const D_DEFAULT_NODES: u32 = 4 * DEFAULT_INPUT as u32;

/// Generic hard input cap.
const HARD_INPUT: usize = 16 * 1024 * 1024;

/// Hard input cap for rustc v0, derived from its per-byte memory use.
const HARD_INPUT_RUST_V0: usize = 1024 * 1024;

/// Hard depth cap for the recursive parsers.
const HARD_DEPTH: usize = 1024;

/// Rust v0 hard depth: the print envelope is 1024 and uses a 2x depth bound,
/// so 512 is the deepest tree the Rust backend can actually accept.
const HARD_DEPTH_RUST: usize = 512;

/// Hard work cap.
const HARD_WORK: usize = 64 * 1024 * 1024;

/// Hard node cap.
const HARD_NODES: usize = 1024 * 1024;

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
/// [`Limit::Bounded`] otherwise. A backend that is still on its compiled-in
/// defaults ignores overrides until it is migrated; the module docs name the
/// migrated schemes.
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
}

/// Capacity policy for one [`crate::Demangler`].
///
/// Every field is an override: `None` keeps the backend default for the scheme
/// being demangled, so [`Budget::default`] is exactly the compiled-in
/// behaviour. A value is clamped to the scheme's [`Scheme::hard_budget`]
/// ceiling, and `0` asks a migrated backend to reject every input at that
/// dimension.
///
/// The values are denial-of-service bounds, not ABI contracts. `work` is an
/// opaque backend-defined unit and is not comparable between schemes. Printing
/// is not a budget, so there is no print field.
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
    pub parse_depth: Option<u32>,
    /// Maximum parser work units, in the backend's own unit.
    pub work: Option<u32>,
    /// Maximum nodes one parse may allocate.
    pub nodes: Option<u32>,
}

impl Budget {
    /// A budget with every dimension left at its backend default.
    pub const fn new() -> Self {
        Self {
            input_bytes: None,
            parse_depth: None,
            work: None,
            nodes: None,
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
    pub const fn with_parse_depth(mut self, depth: u32) -> Self {
        self.parse_depth = Some(depth);
        self
    }

    /// Override the maximum parser work units.
    #[must_use]
    pub const fn with_work(mut self, work: u32) -> Self {
        self.work = Some(work);
        self
    }

    /// Override the maximum number of nodes one parse may allocate.
    #[must_use]
    pub const fn with_nodes(mut self, nodes: u32) -> Self {
        self.nodes = Some(nodes);
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
            parse_depth: clamp_limit(
                self.parse_depth.map(to_usize),
                default.parse_depth.map(to_usize),
                hard.parse_depth,
            ),
            work: clamp_limit(
                self.work.map(to_usize),
                default.work.map(to_usize),
                hard.work,
            ),
            nodes: clamp_limit(
                self.nodes.map(to_usize),
                default.nodes.map(to_usize),
                hard.nodes,
            ),
        }
    }
}

/// Widen a `u32` budget value to the `usize` used by [`Limit`].
fn to_usize(value: u32) -> usize {
    usize::try_from(value).unwrap_or(usize::MAX)
}

/// Clamp one dimension against the scheme's default and hard cap.
fn clamp_limit(over: Option<usize>, default: Option<usize>, hard: Option<Limit>) -> Option<Limit> {
    match hard {
        None => None,
        Some(Limit::Unbounded) => Some(Limit::Unbounded),
        Some(Limit::Bounded(ceiling)) => {
            let Some(requested) = over.or(default) else {
                debug_assert!(false, "bounded hard cap without a default");
                return None;
            };
            Some(Limit::Bounded(requested.min(ceiling)))
        }
    }
}

/// Build a [`Budget`] from its four dimensions.
const fn policy(
    input_bytes: Option<usize>,
    parse_depth: Option<u32>,
    work: Option<u32>,
    nodes: Option<u32>,
) -> Budget {
    Budget {
        input_bytes,
        parse_depth,
        work,
        nodes,
    }
}

/// Build an [`EffectiveBudget`] from its four dimensions.
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
    }
}

/// The generic hard input and work caps with a scheme-specific depth and node
/// dimension.
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
    /// `nodes: None` means the backend has no node counter; [`Budget::effective`]
    /// reports that as [`Limit::Unbounded`].
    ///
    /// The C++ parsers derive their work allowance from the input length; the
    /// reported `work` is that allowance at the default 64 KiB input cap. A
    /// work override caps the derived allowance, so it can only tighten it:
    /// raising the input budget needs a raised work budget as well.
    #[must_use]
    pub const fn default_budget(self) -> Budget {
        match self {
            Self::C | Self::Unknown => Budget::new(),
            Self::CppItanium | Self::CppMsvc => policy(
                Some(DEFAULT_INPUT),
                Some(256),
                Some(CPP_DEFAULT_WORK),
                Some(CPP_DEFAULT_NODES),
            ),
            Self::CppGnuV2 | Self::CppArm | Self::CppEdg | Self::CppHp => policy(
                Some(DEFAULT_INPUT),
                Some(64),
                Some(CPP_DEFAULT_WORK),
                Some(LEGACY_DEFAULT_NODES),
            ),
            Self::RustLegacy => policy(
                Some(DEFAULT_INPUT),
                None,
                Some(200_000),
                Some(RUST_DEFAULT_NODES),
            ),
            Self::RustV0 => policy(
                Some(DEFAULT_INPUT),
                Some(500),
                Some(200_000),
                Some(RUST_DEFAULT_NODES),
            ),
            Self::SwiftStable | Self::SwiftLegacy => policy(
                Some(DEFAULT_INPUT),
                Some(96),
                Some(200_000),
                Some(SWIFT_DEFAULT_NODES),
            ),
            Self::Go => policy(Some(DEFAULT_INPUT), Some(32), Some(DEFAULT_WORK), None),
            Self::ObjC => policy(Some(DEFAULT_INPUT), Some(1), Some(DEFAULT_WORK), None),
            Self::KotlinNative => policy(Some(DEFAULT_INPUT), Some(32), Some(DEFAULT_WORK), None),
            Self::D => policy(
                Some(DEFAULT_INPUT),
                Some(64),
                Some(D_DEFAULT_WORK),
                Some(D_DEFAULT_NODES),
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
    /// A dimension the backend does not cap is [`Limit::Unbounded`]; the fixed
    /// scope, segment, parameter and output caps of the flat-grammar backends
    /// are backend extras and are not part of the budget.
    ///
    /// Two ceilings are tighter than the generic ones on purpose: rustc v0
    /// allocates a checkpoint and a cache entry per input byte, and the C++ and
    /// Swift print envelopes bound the nesting their parsers can usefully
    /// accept.
    #[must_use]
    pub const fn hard_budget(self) -> EffectiveBudget {
        match self {
            Self::C | Self::Unknown => hard_policy(None, None, None, None),
            Self::CppItanium | Self::CppMsvc => hard_caps(256, Some(Limit::Bounded(HARD_NODES))),
            Self::CppGnuV2 | Self::CppArm | Self::CppEdg | Self::CppHp => {
                hard_caps(64, Some(Limit::Bounded(HARD_NODES)))
            }
            Self::RustLegacy => hard_policy(
                Some(Limit::Bounded(HARD_INPUT)),
                None,
                Some(Limit::Bounded(HARD_WORK)),
                Some(Limit::Bounded(HARD_NODES)),
            ),
            Self::RustV0 => hard_policy(
                Some(Limit::Bounded(HARD_INPUT_RUST_V0)),
                Some(Limit::Bounded(HARD_DEPTH_RUST)),
                Some(Limit::Bounded(HARD_WORK)),
                Some(Limit::Bounded(HARD_NODES)),
            ),
            Self::SwiftStable | Self::SwiftLegacy => {
                hard_caps(128, Some(Limit::Bounded(HARD_NODES)))
            }
            Self::Go | Self::KotlinNative | Self::Jni => {
                hard_caps(HARD_DEPTH, Some(Limit::Unbounded))
            }
            Self::ObjC => hard_caps(1, Some(Limit::Unbounded)),
            Self::D => hard_caps(HARD_DEPTH, Some(Limit::Bounded(HARD_NODES))),
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
mod tests {
    use super::*;

    #[test]
    fn a_new_budget_leaves_every_dimension_at_its_default() {
        let budget = Budget::new();
        assert_eq!(budget, Budget::default());
        assert_eq!(budget.input_bytes, None);
        assert_eq!(budget.parse_depth, None);
        assert_eq!(budget.work, None);
        assert_eq!(budget.nodes, None);
    }

    #[test]
    fn builders_set_one_dimension_each() {
        let budget = Budget::new()
            .with_input_bytes(1024)
            .with_parse_depth(16)
            .with_work(4096)
            .with_nodes(32);
        assert_eq!(budget, policy(Some(1024), Some(16), Some(4096), Some(32)));
    }

    #[test]
    fn every_scheme_reports_defaults_within_its_hard_caps() {
        for scheme in Scheme::ALL {
            let default = scheme.default_budget();
            let hard = scheme.hard_budget();
            let dimensions = [
                (
                    "input_bytes",
                    default.input_bytes.map(|value| value as u64),
                    hard.input_bytes,
                ),
                (
                    "parse_depth",
                    default.parse_depth.map(u64::from),
                    hard.parse_depth,
                ),
                ("work", default.work.map(u64::from), hard.work),
                ("nodes", default.nodes.map(u64::from), hard.nodes),
            ];
            for (name, default, hard) in dimensions {
                match (default, hard) {
                    (None, None) | (None, Some(Limit::Unbounded)) => {}
                    (Some(value), Some(Limit::Bounded(ceiling))) => {
                        assert!(
                            value <= ceiling as u64,
                            "{scheme}: {name} default {value} exceeds its hard cap {ceiling}"
                        );
                    }
                    (default, hard) => panic!(
                        "{scheme}: {name} has an inconsistent default/hard pair: {default:?}/{hard:?}"
                    ),
                }
            }
        }
    }

    #[test]
    fn the_no_override_budget_reports_the_defaults() {
        for scheme in Scheme::ALL {
            assert_eq!(
                Budget::default().effective(scheme),
                scheme.default_budget().effective(scheme),
                "{scheme}"
            );
        }
    }

    #[test]
    fn lowering_overrides_are_kept() {
        let budget = Budget::new()
            .with_input_bytes(1024)
            .with_parse_depth(8)
            .with_work(4096)
            .with_nodes(64);
        let effective = budget.effective(Scheme::RustV0);
        assert_eq!(effective.input_bytes, Some(Limit::Bounded(1024)));
        assert_eq!(effective.parse_depth, Some(Limit::Bounded(8)));
        assert_eq!(effective.work, Some(Limit::Bounded(4096)));
        assert_eq!(effective.nodes, Some(Limit::Bounded(64)));
    }

    #[test]
    fn raising_overrides_are_clamped_to_the_hard_caps() {
        let budget = Budget::new()
            .with_input_bytes(usize::MAX)
            .with_parse_depth(u32::MAX)
            .with_work(u32::MAX)
            .with_nodes(u32::MAX);
        let effective = budget.effective(Scheme::CppItanium);
        assert_eq!(effective.input_bytes, Some(Limit::Bounded(HARD_INPUT)));
        assert_eq!(effective.parse_depth, Some(Limit::Bounded(256)));
        assert_eq!(effective.work, Some(Limit::Bounded(HARD_WORK)));
        assert_eq!(effective.nodes, Some(Limit::Bounded(HARD_NODES)));
    }

    #[test]
    fn rust_v0_clamps_its_memory_derived_input_cap() {
        let budget = Budget::new().with_input_bytes(usize::MAX);
        assert_eq!(
            budget.effective(Scheme::RustV0).input_bytes,
            Some(Limit::Bounded(HARD_INPUT_RUST_V0))
        );
        assert_eq!(
            budget.effective(Scheme::RustLegacy).input_bytes,
            Some(Limit::Bounded(HARD_INPUT))
        );
    }

    #[test]
    fn a_zero_override_asks_for_rejection_at_that_dimension() {
        let budget = Budget::new()
            .with_input_bytes(0)
            .with_parse_depth(0)
            .with_work(0)
            .with_nodes(0);
        let effective = budget.effective(Scheme::D);
        assert_eq!(effective.input_bytes, Some(Limit::Bounded(0)));
        assert_eq!(effective.parse_depth, Some(Limit::Bounded(0)));
        assert_eq!(effective.work, Some(Limit::Bounded(0)));
        assert_eq!(effective.nodes, Some(Limit::Bounded(0)));
    }

    #[test]
    fn one_budget_resolves_per_scheme() {
        let budget = Budget::new().with_parse_depth(100);
        assert_eq!(
            budget.effective(Scheme::CppItanium).parse_depth,
            Some(Limit::Bounded(100))
        );
        assert_eq!(
            budget.effective(Scheme::CppGnuV2).parse_depth,
            Some(Limit::Bounded(64))
        );
        assert_eq!(
            budget.effective(Scheme::ObjC).parse_depth,
            Some(Limit::Bounded(1))
        );
        assert_eq!(
            budget.effective(Scheme::SwiftStable).parse_depth,
            Some(Limit::Bounded(100))
        );
    }

    #[test]
    fn pass_through_schemes_have_no_dimensions() {
        for scheme in [Scheme::C, Scheme::Unknown] {
            let effective = Budget::new()
                .with_input_bytes(1)
                .with_parse_depth(1)
                .with_work(1)
                .with_nodes(1)
                .effective(scheme);
            assert_eq!(effective.input_bytes, None, "{scheme}");
            assert_eq!(effective.parse_depth, None, "{scheme}");
            assert_eq!(effective.work, None, "{scheme}");
            assert_eq!(effective.nodes, None, "{scheme}");
        }
    }

    #[test]
    fn uncapped_node_dimensions_stay_unbounded() {
        for scheme in [Scheme::Go, Scheme::ObjC, Scheme::KotlinNative, Scheme::Jni] {
            assert_eq!(
                Budget::new().effective(scheme).nodes,
                Some(Limit::Unbounded),
                "{scheme}"
            );
            assert_eq!(
                Budget::new().with_nodes(1).effective(scheme).nodes,
                Some(Limit::Unbounded),
                "{scheme}"
            );
        }
    }

    #[test]
    fn fixed_extra_caps_are_not_budget_dimensions() {
        for scheme in [Scheme::Ada, Scheme::Fortran, Scheme::Delphi, Scheme::OCaml] {
            let effective = Budget::new()
                .with_parse_depth(1)
                .with_nodes(1)
                .effective(scheme);
            assert_eq!(effective.parse_depth, None, "{scheme}");
            assert_eq!(effective.nodes, None, "{scheme}");
        }
    }

    #[test]
    fn cpp_work_default_is_the_derived_ceiling() {
        assert_eq!(CPP_DEFAULT_WORK, 2_101_248);
        for scheme in [
            Scheme::CppItanium,
            Scheme::CppMsvc,
            Scheme::CppGnuV2,
            Scheme::CppArm,
            Scheme::CppEdg,
            Scheme::CppHp,
        ] {
            assert_eq!(
                scheme.default_budget().work,
                Some(CPP_DEFAULT_WORK),
                "{scheme}"
            );
        }
    }

    #[cfg(feature = "rust")]
    #[test]
    fn rust_defaults_mirror_the_backend_constants() {
        let legacy = Scheme::RustLegacy.default_budget();
        assert_eq!(legacy.input_bytes, Some(crate::rust::MAX_INPUT_LEN));
        assert_eq!(legacy.parse_depth, None);
        assert_eq!(
            legacy.work,
            Some(u32::try_from(crate::rust::MAX_WORK).unwrap())
        );
        assert_eq!(
            legacy.nodes,
            Some(u32::try_from(crate::rust::MAX_NODES).unwrap())
        );

        let v0 = Scheme::RustV0.default_budget();
        assert_eq!(v0.input_bytes, Some(crate::rust::MAX_INPUT_LEN));
        assert_eq!(v0.parse_depth, Some(crate::rust::MAX_DEPTH));
        assert_eq!(v0.work, Some(u32::try_from(crate::rust::MAX_WORK).unwrap()));
        assert_eq!(
            v0.nodes,
            Some(u32::try_from(crate::rust::MAX_NODES).unwrap())
        );
    }

    #[cfg(feature = "swift")]
    #[test]
    fn swift_defaults_mirror_the_backend_constants() {
        for scheme in [Scheme::SwiftStable, Scheme::SwiftLegacy] {
            let budget = scheme.default_budget();
            assert_eq!(budget.input_bytes, Some(crate::swift::MAX_INPUT_LEN));
            assert_eq!(budget.parse_depth, Some(u32::from(crate::swift::MAX_DEPTH)));
            assert_eq!(budget.work, Some(crate::swift::WORK_BUDGET));
            assert_eq!(
                budget.nodes,
                Some(u32::try_from(crate::swift::MAX_NODES).unwrap())
            );
        }
    }

    #[cfg(any(feature = "cpp-itanium", feature = "cpp-msvc", feature = "cpp-gnu-v2"))]
    #[test]
    fn cpp_work_default_mirrors_the_backend_constant() {
        assert_eq!(
            CPP_DEFAULT_WORK,
            u32::try_from(crate::cpp::DEFAULT_WORK).unwrap()
        );
    }

    #[cfg(any(feature = "cpp-itanium", feature = "cpp-msvc", feature = "cpp-gnu-v2"))]
    #[test]
    fn cpp_input_default_mirrors_the_backend_constant() {
        for scheme in [
            Scheme::CppItanium,
            Scheme::CppMsvc,
            Scheme::CppGnuV2,
            Scheme::CppArm,
            Scheme::CppEdg,
            Scheme::CppHp,
        ] {
            assert_eq!(
                scheme.default_budget().input_bytes,
                Some(crate::cpp::MAX_INPUT_LEN),
                "{scheme}"
            );
        }
    }

    #[cfg(feature = "cpp-gnu-v2")]
    #[test]
    fn legacy_cpp_defaults_mirror_the_backend_constants() {
        for scheme in [
            Scheme::CppGnuV2,
            Scheme::CppArm,
            Scheme::CppEdg,
            Scheme::CppHp,
        ] {
            let budget = scheme.default_budget();
            assert_eq!(budget.parse_depth, Some(crate::cpp::legacy::MAX_DEPTH));
            assert_eq!(budget.nodes, Some(crate::cpp::legacy::MAX_NODES));
        }
    }

    #[cfg(feature = "go")]
    #[test]
    fn go_defaults_mirror_the_backend_constants() {
        let budget = Scheme::Go.default_budget();
        assert_eq!(budget.input_bytes, Some(crate::go::MAX_INPUT_LEN));
        assert_eq!(
            budget.parse_depth,
            Some(u32::try_from(crate::go::MAX_DEPTH).unwrap())
        );
        assert_eq!(
            budget.work,
            Some(u32::try_from(crate::go::MAX_WORK).unwrap())
        );
    }

    #[cfg(feature = "objc")]
    #[test]
    fn objc_defaults_mirror_the_backend_constants() {
        let budget = Scheme::ObjC.default_budget();
        assert_eq!(budget.input_bytes, Some(crate::objc::MAX_INPUT_LEN));
        assert_eq!(
            budget.parse_depth,
            Some(u32::try_from(crate::objc::MAX_DEPTH).unwrap())
        );
        assert_eq!(
            budget.work,
            Some(u32::try_from(crate::objc::MAX_WORK).unwrap())
        );
    }

    #[cfg(feature = "kotlin-native")]
    #[test]
    fn kotlin_native_defaults_mirror_the_backend_constants() {
        let budget = Scheme::KotlinNative.default_budget();
        assert_eq!(
            budget.input_bytes,
            Some(crate::kotlin_native::MAX_INPUT_LEN)
        );
        assert_eq!(
            budget.parse_depth,
            Some(u32::try_from(crate::kotlin_native::MAX_DEPTH).unwrap())
        );
        assert_eq!(
            budget.work,
            Some(u32::try_from(crate::kotlin_native::MAX_WORK).unwrap())
        );
    }

    #[cfg(feature = "d")]
    #[test]
    fn d_defaults_mirror_the_backend_constants() {
        let budget = Scheme::D.default_budget();
        assert_eq!(budget.input_bytes, Some(crate::d::MAX_INPUT_LEN));
        assert_eq!(
            budget.parse_depth,
            Some(u32::try_from(crate::d::MAX_DEPTH).unwrap())
        );
        assert_eq!(
            budget.work,
            Some(u32::try_from(crate::d::MAX_WORK).unwrap())
        );
        assert_eq!(
            budget.nodes,
            Some(u32::try_from(crate::d::MAX_NODES).unwrap())
        );
    }

    #[cfg(feature = "jni")]
    #[test]
    fn jni_defaults_mirror_the_backend_constants() {
        let budget = Scheme::Jni.default_budget();
        assert_eq!(budget.input_bytes, Some(crate::jni::MAX_INPUT_LEN));
        assert_eq!(
            budget.parse_depth,
            Some(u32::try_from(crate::jni::MAX_DEPTH).unwrap())
        );
        assert_eq!(
            budget.work,
            Some(u32::try_from(crate::jni::MAX_WORK).unwrap())
        );
    }

    #[cfg(feature = "ada")]
    #[test]
    fn ada_defaults_mirror_the_backend_constants() {
        let budget = Scheme::Ada.default_budget();
        assert_eq!(budget.input_bytes, Some(crate::ada::MAX_INPUT_LEN));
        assert_eq!(
            budget.work,
            Some(u32::try_from(crate::ada::MAX_WORK).unwrap())
        );
        // The flat grammar has no depth dimension.
        assert_eq!(budget.parse_depth, None);
    }

    #[cfg(feature = "fortran")]
    #[test]
    fn fortran_defaults_mirror_the_backend_constants() {
        let budget = Scheme::Fortran.default_budget();
        assert_eq!(budget.input_bytes, Some(crate::fortran::MAX_INPUT_LEN));
        assert_eq!(budget.parse_depth, None);
        assert_eq!(
            budget.work,
            Some(u32::try_from(crate::fortran::MAX_WORK).unwrap())
        );
    }

    #[cfg(feature = "delphi")]
    #[test]
    fn delphi_defaults_mirror_the_backend_constants() {
        let budget = Scheme::Delphi.default_budget();
        assert_eq!(budget.input_bytes, Some(crate::delphi::MAX_INPUT_LEN));
        assert_eq!(
            budget.work,
            Some(u32::try_from(crate::delphi::MAX_WORK).unwrap())
        );
        // The flat grammar has no depth dimension.
        assert_eq!(budget.parse_depth, None);
    }

    #[cfg(feature = "ocaml")]
    #[test]
    fn ocaml_defaults_mirror_the_backend_constants() {
        let budget = Scheme::OCaml.default_budget();
        assert_eq!(budget.input_bytes, Some(crate::ocaml::MAX_INPUT_LEN));
        assert_eq!(
            budget.work,
            Some(u32::try_from(crate::ocaml::MAX_WORK).unwrap())
        );
        // The flat grammar has no depth dimension.
        assert_eq!(budget.parse_depth, None);
    }
}
