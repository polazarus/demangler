use alloc::format;
use alloc::rc::Rc;
use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;

use super::*;
use crate::rust::{Encoding, parse};

/// Assert both renderings, and cross-check them with the reference decoder.
fn check(symbol: &str, display: &str, alternate: &str) {
    let ast = parse(symbol.as_bytes()).unwrap();
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
    assert_eq!(parse(b"_RMC0RL0_h").unwrap_err(), ParseError::Invalid);
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
    let ast = parse(b"_RXC0hC3foo").unwrap();
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
        assert_eq!(
            parse(symbol.as_bytes()).unwrap_err(),
            ParseError::Invalid,
            "{symbol}"
        );
    }
}

/// A string literal is a byte string: bytes that are not UTF-8 are kept
/// and escaped by the printer instead of rejecting the symbol.
#[test]
fn invalid_utf8_string_literals_are_escaped() {
    let ast = parse(b"_RIC0Kef09f_E").unwrap();
    assert_eq!(ast.to_string(), "::<{*\"\\xf0\\x9f\"}>");
    // Valid UTF-8 in a literal keeps printing verbatim.
    let ast = parse(b"_RIC0Ke61c3a9_E").unwrap();
    assert_eq!(ast.to_string(), "::<{*\"aé\"}>");
}

#[test]
fn backrefs_share_nodes() {
    let symbol = "_RMC0TTTTTTpB8_EB7_EB6_EB5_EB4_EB3_E";
    let ast = parse(symbol.as_bytes()).unwrap();
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
    assert_eq!(shared_links(&parse(b"_RMC0ThmE").unwrap()), 0);
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
    assert!(parse(symbol.as_bytes()).is_ok());
}

#[test]
fn instantiating_crate_is_kept_but_not_printed() {
    let ast = parse(b"_RNCNCNgCs6DXkGYLi8lr_2cc5spawn00B5_").unwrap();
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
    assert!(parse(accepted.as_bytes()).is_ok());
    let rejected = format!("_R{}C0{}", "I".repeat(500), "E".repeat(500));
    assert_eq!(
        parse(rejected.as_bytes()).unwrap_err(),
        ParseError::RecursedTooDeep
    );
    assert!(rustc_demangle::try_demangle(&accepted).is_ok());
    assert!(rustc_demangle::try_demangle(&rejected).is_err());
}

#[test]
fn node_budget_is_enforced() {
    let symbol = format!("_RMC0T{}E", "p".repeat(crate::rust::MAX_NODES + 1));
    assert_eq!(
        parse(symbol.as_bytes()).unwrap_err(),
        ParseError::BudgetExceeded
    );
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
    assert_eq!(
        parse(symbol.as_bytes()).unwrap_err(),
        ParseError::BudgetExceeded
    );
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
        assert_eq!(
            parse(symbol.as_bytes()).unwrap_err(),
            ParseError::Invalid,
            "{symbol}"
        );
    }
}

#[test]
fn forward_backrefs_are_rejected() {
    // A backref must point strictly before itself.
    assert_eq!(parse(b"_RTB3_E").unwrap_err(), ParseError::Invalid);
    assert_eq!(parse(b"_RB0_C3foo").unwrap_err(), ParseError::Invalid);
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
    assert_eq!(parse(b"_RCu2_a_").unwrap_err(), ParseError::Invalid);
}

#[test]
fn punycode_names_have_no_length_cap() {
    // 199 ASCII chars and one `é` decode in full; the shifts are charged
    // to the work budget instead of a fixed 128-char buffer.
    let symbol = alloc::format!("_RCu203_{}_ykr", "a".repeat(199));
    let expected = alloc::format!("{}é", "a".repeat(199));
    assert_eq!(parse(symbol.as_bytes()).unwrap().to_string(), expected);
    let tight = crate::Budget::new().with_work(64);
    assert!(
        crate::Demangler::with_budget(tight)
            .demangle(&symbol)
            .is_err()
    );
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
    assert_eq!(parse(b"_RMC0wwh").unwrap_err(), ParseError::Invalid);
    // The marker is only valid in type position, not before a root path.
    assert_eq!(parse(b"_RwC3foo").unwrap_err(), ParseError::Invalid);
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
        assert_eq!(
            parse(symbol.as_bytes()).unwrap_err(),
            ParseError::Invalid,
            "{symbol}"
        );
    }
}

#[test]
fn spans_point_into_the_symbol() {
    let symbol = "_RMC0ThmE";
    let ast = parse(symbol.as_bytes()).unwrap();
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
    let ast = parse(symbol.as_bytes()).unwrap();
    assert!(matches!(
        ast.root().text(),
        Some(text) if text == b"foo"
    ));
}

/// Identifier bytes are byte offsets, so a length may cut a UTF-8 sequence
/// in half and the printer escapes the bytes instead of rejecting them.
#[test]
fn invalid_utf8_identifiers_are_escaped() {
    let ast = parse(b"_RC3f\xffo").unwrap();
    assert_eq!(ast.root().text(), Some(&b"f\xffo"[..]));
    assert_eq!(ast.to_string(), "f\\xffo");

    // A length that splits a two-byte character is legal at this layer.
    let ast = parse(b"_RC1\xc3").unwrap();
    assert_eq!(ast.root().text(), Some(&b"\xc3"[..]));
    assert_eq!(ast.to_string(), "\\xc3");

    // Valid UTF-8 keeps printing verbatim.
    let ast = parse("_RC3fé".as_bytes()).unwrap();
    assert_eq!(ast.to_string(), "fé");
}

/// A backref is a byte offset: a one-byte shift changes the target.
#[test]
fn backrefs_are_byte_offsets() {
    // `B4_` resolves to the `h` at encoding offset 5 and shares its node.
    let ast = parse(b"_RMC0TThB4_EE").unwrap();
    assert_eq!(ast.to_string(), "<((u8, u8),)>");
    // Offsetting the same reference by one byte no longer reaches it.
    assert_eq!(parse(b"_RMC0TThB5_EE").unwrap_err(), ParseError::Invalid);
    // A backref must point strictly before itself.
    assert_eq!(parse(b"_RB1_C3foo").unwrap_err(), ParseError::Invalid);
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
            let _ = parse(&symbol[..end]);
        }
        assert_eq!(
            parse(&symbol[..symbol.len() - 1]).unwrap_err(),
            ParseError::Invalid,
            "{symbol:?}"
        );
    }
}

#[test]
fn fn_dyn_and_pattern_nodes_are_inspectable() {
    let ast = parse(b"_RMC0FG_RL0_hEm").unwrap();
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
    let ast = parse(b"_RMC0FhEu").unwrap();
    let fn_node = ast.root().children().get(1).expect("fn type");
    assert!(matches!(
        fn_node.data(),
        NodeData::Function {
            ret_elided: true,
            ..
        }
    ));
    assert_eq!(fn_node.children().len(), 2);

    let ast = parse(b"_RMC0DG_C3fooEL_").unwrap();
    let dyn_node = ast.root().children().get(1).expect("dyn type");
    assert_eq!(dyn_node.kind(), Kind::Dyn);
    assert_eq!(dyn_node.data(), &NodeData::Binder(1));
    let children = dyn_node.children();
    assert_eq!(children.len(), 2, "one trait and the trailing lifetime");
    assert_eq!(children[0].kind(), Kind::DynTrait);
    assert_eq!(children[1].kind(), Kind::Lifetime);

    let ast = parse(b"_RMC0WmRm1_m9_").unwrap();
    let pattern_type = ast.root().children().get(1).expect("pattern type");
    assert_eq!(pattern_type.kind(), Kind::PatternType);
    let pattern = pattern_type.children().get(1).expect("pattern");
    assert_eq!(pattern.data(), &NodeData::Pattern(PatternKind::Range));
    assert_eq!(pattern.children().len(), 2);
}

#[test]
fn ast_is_inspectable() {
    let ast = parse(b"_RIC0hKj4_E").unwrap();
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
