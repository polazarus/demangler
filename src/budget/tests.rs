use super::*;

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
#[test]
fn clamp_resolves_every_dimension_state() {
    assert_eq!(clamp(Some(Limit::Bounded(5)), 10, 20), 5);
    assert_eq!(clamp(Some(Limit::Bounded(50)), 10, 20), 20);
    assert_eq!(clamp(Some(Limit::Unbounded), 10, 20), 10);
    assert_eq!(clamp(None, 30, 20), 20);
}

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
fn expansion_builders_set_only_their_own_dimension() {
    let visits = Budget::new().with_expansion_visits(4096);
    assert_eq!(visits.expansion_visits, Some(4096));
    assert_eq!(visits.expansion_bytes, None);

    let bytes = Budget::new().with_expansion_bytes(1 << 16);
    assert_eq!(bytes.expansion_visits, None);
    assert_eq!(bytes.expansion_bytes, Some(1 << 16));

    assert_eq!(
        Budget::new()
            .with_expansion_visits(4096)
            .with_expansion_bytes(1 << 16),
        expansion(Budget::new(), 4096, 1 << 16)
    );
}

#[test]
fn every_scheme_reports_defaults_within_its_hard_caps() {
    for &scheme in Scheme::ALL {
        let default = scheme.default_budget();
        let hard = scheme.hard_budget();
        let dimensions = [
            ("input_bytes", default.input_bytes, hard.input_bytes),
            ("parse_depth", default.parse_depth, hard.parse_depth),
            ("work", default.work, hard.work),
            ("nodes", default.nodes, hard.nodes),
            (
                "expansion_visits",
                default.expansion_visits,
                hard.expansion_visits,
            ),
            (
                "expansion_bytes",
                default.expansion_bytes,
                hard.expansion_bytes,
            ),
        ];
        for (name, default, hard) in dimensions {
            match (default, hard) {
                (None, None | Some(Limit::Unbounded)) => {}
                (Some(value), Some(Limit::Bounded(ceiling))) => {
                    assert!(
                        value <= ceiling,
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
    for &scheme in Scheme::ALL {
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
        .with_nodes(64)
        .with_expansion_visits(512)
        .with_expansion_bytes(8192);
    let effective = budget.effective(Scheme::RustV0);
    assert_eq!(effective.input_bytes, Some(Limit::Bounded(1024)));
    assert_eq!(effective.parse_depth, Some(Limit::Bounded(8)));
    assert_eq!(effective.work, Some(Limit::Bounded(4096)));
    assert_eq!(effective.nodes, Some(Limit::Bounded(64)));
    assert_eq!(effective.expansion_visits, Some(Limit::Bounded(512)));
    assert_eq!(effective.expansion_bytes, Some(Limit::Bounded(8192)));
}

#[test]
fn raising_overrides_are_clamped_to_the_hard_caps() {
    let budget = Budget::new()
        .with_input_bytes(usize::MAX)
        .with_parse_depth(usize::MAX)
        .with_work(usize::MAX)
        .with_nodes(usize::MAX)
        .with_expansion_visits(usize::MAX)
        .with_expansion_bytes(usize::MAX);
    let effective = budget.effective(Scheme::CppItanium);
    assert_eq!(effective.input_bytes, Some(Limit::Bounded(HARD_INPUT)));
    assert_eq!(effective.parse_depth, Some(Limit::Bounded(256)));
    assert_eq!(effective.work, Some(Limit::Bounded(HARD_WORK)));
    assert_eq!(effective.nodes, Some(Limit::Bounded(HARD_NODES)));
    assert_eq!(
        effective.expansion_visits,
        Some(Limit::Bounded(HARD_EXPANSION_VISITS))
    );
    assert_eq!(
        effective.expansion_bytes,
        Some(Limit::Bounded(HARD_EXPANSION_BYTES))
    );
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
        .with_nodes(0)
        .with_expansion_visits(0)
        .with_expansion_bytes(0);
    let effective = budget.effective(Scheme::D);
    assert_eq!(effective.input_bytes, Some(Limit::Bounded(0)));
    assert_eq!(effective.parse_depth, Some(Limit::Bounded(0)));
    assert_eq!(effective.work, Some(Limit::Bounded(0)));
    assert_eq!(effective.nodes, Some(Limit::Bounded(0)));
    assert_eq!(effective.expansion_visits, Some(Limit::Bounded(0)));
    assert_eq!(effective.expansion_bytes, Some(Limit::Bounded(0)));
}

#[test]
fn expansion_defaults_are_per_scheme() {
    for (scheme, visits, bytes) in [
        (
            Scheme::RustLegacy,
            RUST_DEFAULT_EXPANSION_VISITS,
            DEFAULT_EXPANSION_BYTES,
        ),
        (
            Scheme::RustV0,
            RUST_DEFAULT_EXPANSION_VISITS,
            DEFAULT_EXPANSION_BYTES,
        ),
        (
            Scheme::SwiftStable,
            SWIFT_DEFAULT_EXPANSION_VISITS,
            DEFAULT_EXPANSION_BYTES,
        ),
        (
            Scheme::SwiftLegacy,
            SWIFT_DEFAULT_EXPANSION_VISITS,
            DEFAULT_EXPANSION_BYTES,
        ),
        (
            Scheme::CppItanium,
            CPP_DEFAULT_EXPANSION_VISITS,
            DEFAULT_EXPANSION_BYTES,
        ),
        (
            Scheme::CppMsvc,
            CPP_DEFAULT_EXPANSION_VISITS,
            DEFAULT_EXPANSION_BYTES,
        ),
        (
            Scheme::CppGnuV2,
            LEGACY_DEFAULT_EXPANSION_VISITS,
            DEFAULT_EXPANSION_BYTES,
        ),
        (
            Scheme::CppArm,
            LEGACY_DEFAULT_EXPANSION_VISITS,
            DEFAULT_EXPANSION_BYTES,
        ),
        (
            Scheme::CppEdg,
            LEGACY_DEFAULT_EXPANSION_VISITS,
            DEFAULT_EXPANSION_BYTES,
        ),
        (
            Scheme::CppHp,
            LEGACY_DEFAULT_EXPANSION_VISITS,
            DEFAULT_EXPANSION_BYTES,
        ),
        (
            Scheme::D,
            D_DEFAULT_EXPANSION_VISITS,
            D_DEFAULT_EXPANSION_BYTES,
        ),
    ] {
        let budget = scheme.default_budget();
        assert_eq!(budget.expansion_visits, Some(visits), "{scheme}");
        assert_eq!(budget.expansion_bytes, Some(bytes), "{scheme}");
        assert_eq!(
            budget.effective(scheme).expansion_visits,
            Some(Limit::Bounded(visits)),
            "{scheme}"
        );
    }
}

#[test]
fn linear_schemes_have_no_expansion_gate() {
    for scheme in [
        Scheme::C,
        Scheme::Unknown,
        Scheme::Go,
        Scheme::ObjC,
        Scheme::KotlinNative,
        Scheme::Jni,
        Scheme::Ada,
        Scheme::Fortran,
        Scheme::Delphi,
        Scheme::OCaml,
    ] {
        let effective = Budget::new()
            .with_expansion_visits(1)
            .with_expansion_bytes(1)
            .effective(scheme);
        assert_eq!(effective.expansion_visits, None, "{scheme}");
        assert_eq!(effective.expansion_bytes, None, "{scheme}");
        assert_eq!(scheme.default_budget().expansion_visits, None, "{scheme}");
        assert_eq!(scheme.hard_budget().expansion_visits, None, "{scheme}");
    }
}

#[test]
fn expansion_hard_caps_are_per_scheme() {
    let budget = Budget::new()
        .with_expansion_visits(usize::MAX)
        .with_expansion_bytes(usize::MAX);
    for scheme in [
        Scheme::RustLegacy,
        Scheme::RustV0,
        Scheme::SwiftStable,
        Scheme::SwiftLegacy,
        Scheme::CppItanium,
        Scheme::CppMsvc,
        Scheme::CppGnuV2,
        Scheme::CppArm,
        Scheme::CppEdg,
        Scheme::CppHp,
    ] {
        let effective = budget.effective(scheme);
        assert_eq!(
            effective.expansion_visits,
            Some(Limit::Bounded(HARD_EXPANSION_VISITS)),
            "{scheme}"
        );
        assert_eq!(
            effective.expansion_bytes,
            Some(Limit::Bounded(HARD_EXPANSION_BYTES)),
            "{scheme}"
        );
    }
    let d = budget.effective(Scheme::D);
    assert_eq!(
        d.expansion_visits,
        Some(Limit::Bounded(D_HARD_EXPANSION_VISITS))
    );
    assert_eq!(
        d.expansion_bytes,
        Some(Limit::Bounded(HARD_EXPANSION_BYTES))
    );
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
            .with_expansion_visits(1)
            .with_expansion_bytes(1)
            .effective(scheme);
        assert_eq!(effective.input_bytes, None, "{scheme}");
        assert_eq!(effective.parse_depth, None, "{scheme}");
        assert_eq!(effective.work, None, "{scheme}");
        assert_eq!(effective.nodes, None, "{scheme}");
        assert_eq!(effective.expansion_visits, None, "{scheme}");
        assert_eq!(effective.expansion_bytes, None, "{scheme}");
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
fn flat_grammars_have_no_depth_or_node_dimension() {
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
    assert_eq!(legacy.work, Some(crate::rust::MAX_WORK));
    assert_eq!(legacy.nodes, Some(crate::rust::MAX_NODES));
    assert_eq!(legacy.expansion_visits, Some(RUST_DEFAULT_EXPANSION_VISITS));
    assert_eq!(legacy.expansion_bytes, Some(DEFAULT_EXPANSION_BYTES));

    let v0 = Scheme::RustV0.default_budget();
    assert_eq!(v0.input_bytes, Some(crate::rust::MAX_INPUT_LEN));
    assert_eq!(v0.parse_depth, Some(crate::rust::MAX_DEPTH as usize));
    assert_eq!(v0.work, Some(crate::rust::MAX_WORK));
    assert_eq!(v0.nodes, Some(crate::rust::MAX_NODES));
    assert_eq!(v0.expansion_visits, Some(RUST_DEFAULT_EXPANSION_VISITS));
    assert_eq!(v0.expansion_bytes, Some(DEFAULT_EXPANSION_BYTES));
}

#[cfg(feature = "swift")]
#[test]
fn swift_defaults_mirror_the_backend_constants() {
    for scheme in [Scheme::SwiftStable, Scheme::SwiftLegacy] {
        let budget = scheme.default_budget();
        assert_eq!(budget.input_bytes, Some(crate::swift::MAX_INPUT_LEN));
        assert_eq!(budget.parse_depth, Some(crate::swift::MAX_DEPTH as usize));
        assert_eq!(budget.work, Some(crate::swift::WORK_BUDGET as usize));
        assert_eq!(budget.nodes, Some(crate::swift::MAX_NODES));
        assert_eq!(
            budget.expansion_visits,
            Some(SWIFT_DEFAULT_EXPANSION_VISITS)
        );
        assert_eq!(budget.expansion_bytes, Some(DEFAULT_EXPANSION_BYTES));
    }
}

#[cfg(any(feature = "cpp-itanium", feature = "cpp-msvc", feature = "cpp-gnu-v2"))]
#[test]
fn cpp_work_default_mirrors_the_backend_constant() {
    assert_eq!(CPP_DEFAULT_WORK, crate::cpp::DEFAULT_WORK);
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
        assert_eq!(
            budget.parse_depth,
            Some(crate::cpp::legacy::MAX_DEPTH as usize)
        );
        assert_eq!(budget.nodes, Some(crate::cpp::legacy::MAX_NODES as usize));
        assert_eq!(
            budget.expansion_visits,
            Some(LEGACY_DEFAULT_EXPANSION_VISITS)
        );
    }
}

#[cfg(feature = "go")]
#[test]
fn go_defaults_mirror_the_backend_constants() {
    let budget = Scheme::Go.default_budget();
    assert_eq!(budget.input_bytes, Some(crate::go::MAX_INPUT_LEN));
    assert_eq!(budget.parse_depth, Some(crate::go::MAX_DEPTH));
    assert_eq!(budget.work, Some(crate::go::MAX_WORK));
}

#[cfg(feature = "objc")]
#[test]
fn objc_defaults_mirror_the_backend_constants() {
    let budget = Scheme::ObjC.default_budget();
    assert_eq!(budget.input_bytes, Some(crate::objc::MAX_INPUT_LEN));
    assert_eq!(budget.parse_depth, Some(crate::objc::MAX_DEPTH));
    assert_eq!(budget.work, Some(crate::objc::MAX_WORK));
}

#[cfg(feature = "kotlin-native")]
#[test]
fn kotlin_native_defaults_mirror_the_backend_constants() {
    let budget = Scheme::KotlinNative.default_budget();
    assert_eq!(
        budget.input_bytes,
        Some(crate::kotlin_native::MAX_INPUT_LEN)
    );
    assert_eq!(budget.parse_depth, Some(crate::kotlin_native::MAX_DEPTH));
    assert_eq!(budget.work, Some(crate::kotlin_native::MAX_WORK));
}

#[cfg(feature = "d")]
#[test]
fn d_defaults_mirror_the_backend_constants() {
    let budget = Scheme::D.default_budget();
    assert_eq!(budget.input_bytes, Some(crate::d::MAX_INPUT_LEN));
    assert_eq!(budget.parse_depth, Some(crate::d::MAX_DEPTH));
    assert_eq!(budget.work, Some(crate::d::MAX_WORK));
    assert_eq!(budget.nodes, Some(crate::d::MAX_NODES));
    assert_eq!(budget.expansion_visits, Some(D_DEFAULT_EXPANSION_VISITS));
    assert_eq!(budget.expansion_bytes, Some(D_DEFAULT_EXPANSION_BYTES));

    // The D parser clamps every dimension to the ceilings the policy table
    // reports, so the two tables cannot drift apart.
    let hard = Scheme::D.hard_budget();
    assert_eq!(hard.input_bytes, Some(Limit::Bounded(HARD_INPUT)));
    assert_eq!(hard.parse_depth, Some(Limit::Bounded(HARD_DEPTH)));
    assert_eq!(hard.work, Some(Limit::Bounded(HARD_WORK)));
    assert_eq!(hard.nodes, Some(Limit::Bounded(HARD_NODES)));
    assert_eq!(
        hard.expansion_visits,
        Some(Limit::Bounded(D_HARD_EXPANSION_VISITS))
    );
    assert_eq!(
        hard.expansion_bytes,
        Some(Limit::Bounded(HARD_EXPANSION_BYTES))
    );
}

#[cfg(feature = "jni")]
#[test]
fn jni_defaults_mirror_the_backend_constants() {
    let budget = Scheme::Jni.default_budget();
    assert_eq!(budget.input_bytes, Some(crate::jni::MAX_INPUT_LEN));
    assert_eq!(budget.parse_depth, Some(crate::jni::MAX_DEPTH));
    assert_eq!(budget.work, Some(crate::jni::MAX_WORK));
}

#[cfg(feature = "ada")]
#[test]
fn ada_defaults_mirror_the_backend_constants() {
    let budget = Scheme::Ada.default_budget();
    assert_eq!(budget.input_bytes, Some(crate::ada::MAX_INPUT_LEN));
    assert_eq!(budget.work, Some(crate::ada::MAX_WORK));
    // The flat grammar has no depth dimension.
    assert_eq!(budget.parse_depth, None);
}

#[cfg(feature = "fortran")]
#[test]
fn fortran_defaults_mirror_the_backend_constants() {
    let budget = Scheme::Fortran.default_budget();
    assert_eq!(budget.input_bytes, Some(crate::fortran::MAX_INPUT_LEN));
    assert_eq!(budget.parse_depth, None);
    assert_eq!(budget.work, Some(crate::fortran::MAX_WORK));
}

#[cfg(feature = "delphi")]
#[test]
fn delphi_defaults_mirror_the_backend_constants() {
    let budget = Scheme::Delphi.default_budget();
    assert_eq!(budget.input_bytes, Some(crate::delphi::MAX_INPUT_LEN));
    assert_eq!(budget.work, Some(crate::delphi::MAX_WORK));
    // The flat grammar has no depth dimension.
    assert_eq!(budget.parse_depth, None);
}

#[cfg(feature = "ocaml")]
#[test]
fn ocaml_defaults_mirror_the_backend_constants() {
    let budget = Scheme::OCaml.default_budget();
    assert_eq!(budget.input_bytes, Some(crate::ocaml::MAX_INPUT_LEN));
    assert_eq!(budget.work, Some(crate::ocaml::MAX_WORK));
    // The flat grammar has no depth dimension.
    assert_eq!(budget.parse_depth, None);
}
