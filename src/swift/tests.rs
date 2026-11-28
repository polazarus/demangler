use alloc::borrow::{Cow, ToOwned};
use alloc::format;
use alloc::rc::Rc;
use alloc::string::String;
use std::vec::Vec;

use super::*;

#[test]
fn ast_is_inspectable() {
    let ast = parse_bytes_with_budget(b"$s4main3fooyySiF", DEFAULT_BUDGET).unwrap();
    assert_eq!(ast.root().kind(), Kind::TopLevel);

    let mut stack = std::vec![ast.root()];
    let mut texts = Vec::new();
    while let Some(node) = stack.pop() {
        assert!(node.child_count() <= node.children().len());
        if let Some(text) = node.text() {
            texts.push(text.to_owned());
        }
        stack.extend(node.children().iter().cloned());
        assert_eq!(node.is_leaf(), node.child_count() == 0);
    }

    assert!(texts.iter().any(|text| text == "main"));
    assert!(texts.iter().any(|text| text == "foo"));
}

#[test]
fn envelope_bounds_rendered_bytes() {
    // `IndexSubset` text is the one kind whose printer writes more than its
    // own text: each `S` byte becomes an index. A maximal subset keeps the
    // byte estimate an upper bound.
    let base = "$s13TangentVector16_Differentiation14DifferentiablePQzAaDQy_SdAFIegnnnr_TJSd";
    let long_subset = format!("{base}{}pSrSUSP", "S".repeat(64));
    for symbol in [
        "$s4main3fooyySiF",
        "_TtSi",
        "_TtGSqGSaC5sugar7MyClass__",
        long_subset.as_str(),
    ] {
        let ast = parse_bytes_with_budget(symbol.as_bytes(), DEFAULT_BUDGET).unwrap();
        let mut out = String::new();
        ast.print(&mut out).unwrap();
        assert!(
            out.len() <= ast.root().print_bytes(),
            "{symbol}: {} bytes rendered, envelope {}",
            out.len(),
            ast.root().print_bytes()
        );
    }
}

#[test]
fn corpus_print_envelope_holds() {
    let base = env!("CARGO_MANIFEST_DIR");
    if !std::path::Path::new(base).join("refs/swift").is_dir() {
        std::eprintln!("refs/swift is absent; skipping the Swift envelope corpus check");
        return;
    }
    let mut compared = 0_usize;
    for file in [
        "refs/swift/manglings.txt",
        "refs/swift/toolchain-6.4.2-manglings.txt",
    ] {
        let Ok(data) = std::fs::read_to_string(format!("{base}/{file}")) else {
            continue;
        };
        for line in data.lines() {
            let symbol = line.split(" ---> ").next().unwrap_or_default();
            if symbol.is_empty() || symbol.starts_with('#') {
                continue;
            }
            let Ok(ast) = parse_bytes_with_budget(symbol.as_bytes(), DEFAULT_BUDGET) else {
                continue;
            };
            let mut out = String::new();
            let (visits, depth, bytes) = match printer::stats(&ast, &mut out) {
                Ok(stats) => stats,
                Err(error) => panic!("{symbol}: print failed after the gates: {error:?}"),
            };
            let root = ast.root();
            assert!(
                visits <= root.print_visits(),
                "{symbol}: {visits} visits > {}",
                root.print_visits()
            );
            assert!(
                depth <= root.print_depth(),
                "{symbol}: depth {depth} > {}",
                root.print_depth()
            );
            assert!(
                bytes <= root.print_bytes(),
                "{symbol}: {bytes} bytes > {}",
                root.print_bytes()
            );
            // The accepted tree must also fit the default expansion gates,
            // so the corpus never relies on a raised budget.
            assert!(
                root.print_visits() <= node::DEFAULT_EXPANSION_VISITS,
                "{symbol}: {} visits > the default gate",
                root.print_visits()
            );
            assert!(
                root.print_bytes() <= node::DEFAULT_EXPANSION_BYTES,
                "{symbol}: {} bytes > the default gate",
                root.print_bytes()
            );
            assert!(
                root.print_depth() <= MAX_PRINT_DEPTH,
                "{symbol}: depth {} > the printability ceiling",
                root.print_depth()
            );
            compared += 1;
        }
    }
    assert!(compared > 0, "the corpus compared no symbols");
}

#[test]
fn ast_reports_the_mangling_family() {
    assert_eq!(
        parse_bytes_with_budget(b"$s4main3fooyySiF", DEFAULT_BUDGET)
            .unwrap()
            .encoding(),
        Encoding::Stable
    );
    assert_eq!(
        parse_bytes_with_budget(b"_T03foo3barC3basyAA3zimCAE_tFTo", DEFAULT_BUDGET)
            .unwrap()
            .encoding(),
        Encoding::Stable
    );
    assert_eq!(
        parse_bytes_with_budget(b"_TtSi", DEFAULT_BUDGET)
            .unwrap()
            .encoding(),
        Encoding::Legacy
    );
    assert_eq!(
        parse_bytes_with_budget(b"_TF3foog3barSi", DEFAULT_BUDGET)
            .unwrap()
            .encoding(),
        Encoding::Legacy
    );
}

#[test]
fn ast_clone_shares_the_tree() {
    let ast = parse_bytes_with_budget(b"$s4main3fooyySiF", DEFAULT_BUDGET).unwrap();
    let cloned = ast.clone();
    assert!(Rc::ptr_eq(&ast.root(), &cloned.root()));
    assert_eq!(ast.root().kind(), cloned.root().kind());
    assert_eq!(ast.root().child_count(), cloned.root().child_count());
}

#[test]
fn deep_iterative_wrappers_are_rejected_without_stack_overflow() {
    let symbol = std::format!("$sSi{}", "h".repeat(0xFFFE));
    let parsed = std::thread::Builder::new()
        .stack_size(64 * 1024)
        .spawn(move || parse_bytes_with_budget(symbol.as_bytes(), DEFAULT_BUDGET).is_err())
        .expect("spawn")
        .join()
        .expect("join");
    assert!(parsed);
}

#[test]
fn ast_text_can_be_borrowed() {
    let ast = parse_bytes_with_budget(b"$s4main3fooyySiF", DEFAULT_BUDGET).unwrap();
    let mut stack = std::vec![ast.root()];
    let mut node = None;
    while let Some(candidate) = stack.pop() {
        if candidate.kind() == Kind::Identifier && candidate.text() == Some("foo") {
            node = Some(candidate);
            break;
        }
        stack.extend(candidate.children().iter().cloned());
    }
    let node = node.unwrap();
    assert_eq!(node.text(), Some("foo"));
    assert!(matches!(node.text_cow(), Some(Cow::Borrowed(_))));
}

#[test]
fn node_handles_and_payloads_are_inspectable() {
    let ast = parse_bytes_with_budget(b"$s4main3fooyySiFyyXEfU_TA.1", DEFAULT_BUDGET).unwrap();
    let root = ast.root();
    assert_eq!(root.kind(), Kind::TopLevel);
    assert!(matches!(root.data(), NodeData::None));
    let first = root.child(0).expect("top-level child");
    assert!(Rc::ptr_eq(&first, &root.children()[0]));
    assert_eq!(first.child_count(), first.children().len());

    let mut stack = std::vec![root];
    let mut saw_global = false;
    while let Some(node) = stack.pop() {
        if let NodeData::Global { code, operand } = node.data() {
            assert_eq!(node.global_code(), Some(*code));
            assert_eq!(node.global_operand(), *operand);
            saw_global = true;
        }
        stack.extend(node.children().iter().cloned());
    }
    assert!(saw_global, "expected a global wrapper");
}
