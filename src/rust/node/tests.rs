use alloc::rc::Rc;
use alloc::vec;
use alloc::vec::Vec;

use super::*;
use crate::rust::{DEFAULT_BUDGET, parse};

#[test]
fn print_envelope_counts_shared_references() {
    let mut builder = Builder::new(ParserLimits::from_effective(DEFAULT_BUDGET, true));
    let leaf = builder
        .node(Kind::Ident, None, None, NodeData::None, vec![])
        .unwrap();
    let root = builder
        .node(
            Kind::Tuple,
            None,
            None,
            NodeData::None,
            vec![leaf.clone(), leaf],
        )
        .unwrap();
    assert_eq!(root.print_visits(), 3);
    assert_eq!(root.print_depth(), 2);
    // Two shared leaves and the tuple: each leaf is charged its text and
    // syntax once per reference.
    assert_eq!(
        root.print_bytes(),
        2 * syntax_bytes(Kind::Ident) + syntax_bytes(Kind::Tuple)
    );
}

/// The byte counter is an upper bound: rendering a parser-built tree never
/// writes more than its root's charge.
#[test]
fn print_bytes_cover_rendered_bytes() {
    for symbol in [
        "_RCs_3foo",
        "_RIC0hKj4_E",
        "_RNCNCNgCs6DXkGYLi8lr_2cc5spawn00B5_",
        "_RMC0TTTTTTpB8_EB7_EB6_EB5_EB4_EB3_E",
        "_ZN3foo17h05af221e174051e9E",
        "_RIC0Kanb_E",
        "_RNvNtCs1234_4test3foo3bar",
    ] {
        let ast = parse(symbol.as_bytes()).unwrap_or_else(|error| {
            panic!("{symbol}: must parse at the default budget: {error:?}")
        });
        let mut out = alloc::string::String::new();
        core::fmt::Write::write_fmt(&mut out, format_args!("{ast}")).unwrap();
        assert!(
            out.len() <= ast.root().print_bytes(),
            "{symbol}: {} bytes rendered, envelope {}",
            out.len(),
            ast.root().print_bytes()
        );
    }
}

#[test]
fn accessors_read_the_payload() {
    let ast = parse(b"_RNvC3foo3bar").unwrap();
    let root = ast.root();
    assert_eq!(root.kind(), Kind::Nested);
    assert_eq!(root.namespace(), Some(Namespace::Internal));
    assert_eq!(root.text(), None);
    assert_eq!(root.disambiguator(), None);

    let ident = root.children().get(1).unwrap();
    assert_eq!(ident.kind(), Kind::Ident);
    assert_eq!(ident.text(), Some(&b"bar"[..]));
    assert_eq!(ident.disambiguator(), Some(0));
    assert!(ident.children().is_empty());
}

#[test]
fn span_helpers() {
    let span = Span { start: 3, end: 7 };
    assert_eq!(span.len(), 4);
    assert!(!span.is_empty());
    assert!(Span { start: 3, end: 3 }.is_empty());
}

#[test]
fn namespace_tags_round_trip() {
    assert_eq!(Namespace::Closure.tag(), 'C');
    assert_eq!(Namespace::Shim.tag(), 'S');
    assert_eq!(Namespace::Other('X').tag(), 'X');
    assert_eq!(Namespace::Internal.tag(), 'a');
}

#[test]
fn lifetime_index_is_exposed() {
    let ast = parse(b"_RIC0L_E").unwrap();
    let lifetime = ast.root().children().get(1).unwrap();
    assert_eq!(lifetime.kind(), Kind::Lifetime);
    assert_eq!(lifetime.lifetime_index(), Some(0));
}

/// Spans stay inside the input, and a depth-first walk terminates: a node
/// is never its own ancestor. Repeated nodes are DAG sharing, not a cycle.
#[test]
fn graph_invariants() {
    for symbol in [
        "_ZN3foo3barE",
        "_RC3foo",
        "_RMC0TTTTTTpB8_EB7_EB6_EB5_EB4_EB3_E",
        "_RNCNCNgCs6DXkGYLi8lr_2cc5spawn00B5_",
        "_RIC0hKj4_E",
    ] {
        let ast = parse(symbol.as_bytes()).unwrap();
        let mut on_path: Vec<*const Node<'_>> = Vec::new();
        let mut stack = vec![(ast.root().clone(), 0_usize)];
        let mut entered = 0_usize;
        while let Some(top) = stack.last_mut() {
            let node = top.0.clone();
            let index = top.1;
            if index == 0 {
                let pointer = Rc::as_ptr(&node);
                assert!(
                    !on_path.contains(&pointer),
                    "{symbol}: node is its own ancestor"
                );
                on_path.push(pointer);
                entered += 1;
                assert!(entered < 100_000, "{symbol}: walk did not terminate");
                if let Some(span) = node.span() {
                    assert!(span.start <= span.end, "{symbol}: inverted span");
                    assert!(
                        span.end as usize <= symbol.len(),
                        "{symbol}: span past the input"
                    );
                }
            }
            if let Some(child) = node.children().get(index).cloned() {
                top.1 += 1;
                stack.push((child, 0));
            } else {
                let _ = on_path.pop();
                let _ = stack.pop();
            }
        }
        assert!(entered > 0, "{symbol}: empty graph");
    }
}

// At the defaults the node budget fires before the work budget for
// node-only growth, and the fixed printability limit sits above the v0
// parse depth so the parser depth caps first.
const _: () = assert!(MAX_NODES < MAX_WORK);
const _: () = assert!(MAX_DEPTH <= MAX_PRINT_DEPTH);

#[test]
fn builder_charges_the_resolved_limits() {
    let limits = ParserLimits {
        input: MAX_INPUT_LEN,
        depth: MAX_DEPTH,
        work: 2,
        nodes: 1,
        expansion_visits: DEFAULT_EXPANSION_VISITS,
        expansion_bytes: DEFAULT_EXPANSION_BYTES,
    };
    let mut builder = Builder::new(limits);
    let _ = builder
        .node(Kind::Ident, None, None, NodeData::None, vec![])
        .expect("first node");
    assert_eq!(
        builder
            .node(Kind::Ident, None, None, NodeData::None, vec![])
            .unwrap_err(),
        ParseError::BudgetExceeded
    );
}

#[test]
fn hard_ceilings_mirror_the_budget_table() {
    use crate::budget::Limit;
    use crate::scheme::Scheme;

    let legacy = Scheme::RustLegacy.hard_budget();
    assert_eq!(legacy.input_bytes, Some(Limit::Bounded(HARD_INPUT_LEN)));
    assert_eq!(legacy.parse_depth, None);
    assert_eq!(legacy.work, Some(Limit::Bounded(HARD_WORK)));
    assert_eq!(legacy.nodes, Some(Limit::Bounded(HARD_NODES)));
    assert_eq!(
        legacy.expansion_visits,
        Some(Limit::Bounded(HARD_EXPANSION_VISITS))
    );
    assert_eq!(
        legacy.expansion_bytes,
        Some(Limit::Bounded(HARD_EXPANSION_BYTES))
    );

    let v0 = Scheme::RustV0.hard_budget();
    assert_eq!(v0.input_bytes, Some(Limit::Bounded(HARD_INPUT_LEN_V0)));
    assert_eq!(v0.parse_depth, Some(Limit::Bounded(HARD_DEPTH as usize)));
    assert_eq!(v0.work, Some(Limit::Bounded(HARD_WORK)));
    assert_eq!(v0.nodes, Some(Limit::Bounded(HARD_NODES)));
    assert_eq!(
        v0.expansion_visits,
        Some(Limit::Bounded(HARD_EXPANSION_VISITS))
    );
    assert_eq!(
        v0.expansion_bytes,
        Some(Limit::Bounded(HARD_EXPANSION_BYTES))
    );
}

/// The default expansion gates are the ones the policy table reports, so
/// the two tables cannot drift apart.
#[test]
fn default_expansion_gates_mirror_the_budget_table() {
    for scheme in [crate::Scheme::RustLegacy, crate::Scheme::RustV0] {
        let budget = scheme.default_overrides();
        assert_eq!(budget.expansion_visits, Some(DEFAULT_EXPANSION_VISITS));
        assert_eq!(budget.expansion_bytes, Some(DEFAULT_EXPANSION_BYTES));
    }
}

#[test]
fn over_hard_values_are_clamped_to_the_ceilings() {
    let budget = crate::Budget::new()
        .with_input_bytes(usize::MAX)
        .with_parse_depth(usize::MAX)
        .with_work(usize::MAX)
        .with_nodes(usize::MAX)
        .with_expansion_visits(usize::MAX)
        .with_expansion_bytes(usize::MAX);
    let limits = ParserLimits::from_effective(budget.effective(crate::Scheme::RustV0), true);
    assert_eq!(limits.input, HARD_INPUT_LEN_V0);
    assert_eq!(limits.depth, HARD_DEPTH);
    assert_eq!(limits.work, HARD_WORK);
    assert_eq!(limits.nodes, HARD_NODES);
    assert_eq!(limits.expansion_visits, HARD_EXPANSION_VISITS);
    assert_eq!(limits.expansion_bytes, HARD_EXPANSION_BYTES);
}

#[test]
fn absent_dimensions_keep_the_compiled_in_defaults() {
    for (scheme, v0) in [
        (crate::Scheme::RustLegacy, false),
        (crate::Scheme::RustV0, true),
    ] {
        let limits = ParserLimits::from_effective(crate::Budget::new().effective(scheme), v0);
        assert_eq!(
            limits,
            ParserLimits {
                input: MAX_INPUT_LEN,
                depth: MAX_DEPTH,
                work: MAX_WORK,
                nodes: MAX_NODES,
                expansion_visits: DEFAULT_EXPANSION_VISITS,
                expansion_bytes: DEFAULT_EXPANSION_BYTES,
            },
            "{scheme}"
        );
    }
}
