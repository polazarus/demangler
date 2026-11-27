use alloc::format;
use alloc::string::String;

use super::Printer;
use crate::swift::node::{Context, Global, GlobalOperand, Kind, MAX_NODES};

/// Print `node` and return the printer counters.
fn stats_for(out: &mut String, node: crate::swift::node::NodeRef<'_>) -> (usize, u16, usize) {
    let mut sink = super::CountingSink { out, written: 0 };
    let mut printer = Printer::new(&mut sink);
    printer.print(node).unwrap();
    (printer.visits, printer.max_depth, sink.written)
}

fn render_global(code: Global) -> String {
    let mut context = Context::new(MAX_NODES);
    let child = context.builder_text(Kind::Identifier, "Foo").build();
    let mut global = context.builder_global(code, GlobalOperand::None);
    global.push(child);
    let node = global.build();

    let mut out = String::new();
    Printer::new(&mut out).print(node).unwrap();
    out
}

#[test]
fn global_symbol_families_render() {
    for (code, prefix) in [
        (Global::TypeMetadata, "type metadata for "),
        (Global::FullTypeMetadata, "full type metadata for "),
        (
            Global::TypeMetadataPattern,
            "generic type metadata pattern for ",
        ),
        (Global::TypeMetadataAccess, "type metadata accessor for "),
        (
            Global::TypeMetadataLazyCache,
            "lazy cache variable for type metadata for ",
        ),
        (
            Global::TypeMetadataDemanglingCache,
            "demangling cache variable for type metadata for ",
        ),
        (
            Global::NominalTypeDescriptor,
            "nominal type descriptor for ",
        ),
        (Global::PropertyDescriptor, "property descriptor for "),
        (Global::OpaqueTypeDescriptor, "opaque type descriptor for "),
        (Global::ClassMetaclass, "metaclass for "),
        (
            Global::ClassMethodLookupFunction,
            "method lookup function for ",
        ),
        (
            Global::ObjcMetadataUpdate,
            "ObjC metadata update function for ",
        ),
        (
            Global::ObjcResilientClassStub,
            "ObjC resilient class stub for ",
        ),
        (
            Global::FullObjcResilientClassStub,
            "full ObjC resilient class stub for ",
        ),
        (
            Global::ClassMetadataBaseOffset,
            "class metadata base offset for ",
        ),
        (
            Global::GenericTypeCompletionFunction,
            "type metadata completion function for ",
        ),
        (Global::ModuleDescriptor, "module descriptor "),
        (Global::ExtensionDescriptor, "extension descriptor "),
        (Global::AnonymousContextDescriptor, "anonymous descriptor "),
        (Global::ProtocolDescriptor, "protocol descriptor for "),
        (
            Global::ProtocolDescriptorRecord,
            "protocol descriptor runtime record for ",
        ),
        (Global::ProtocolWitnessTable, "protocol witness table for "),
        (Global::ValueWitnessTable, "value witness table for "),
        (Global::EnumTagIndex, "resilient enum tag index for "),
        (Global::OutlinedCopy, "outlined copy of "),
        (Global::OutlinedConsume, "outlined consume of "),
        (Global::OutlinedRetain, "outlined retain of "),
        (Global::OutlinedRelease, "outlined release of "),
        (Global::OutlinedInitWithTake, "outlined init with take of "),
        (Global::OutlinedInitWithCopy, "outlined init with copy of "),
        (Global::OutlinedDestroy, "outlined destroy of "),
    ] {
        assert_eq!(render_global(code), format!("{prefix}Foo"), "{code:?}");
    }
}

#[test]
fn shared_graph_prints_completely() {
    // Nineteen doublings expand to 2^20 - 1 visits, far past the old visit
    // cap. The printer has no visit limit, so the whole graph prints.
    let mut context = Context::new(MAX_NODES);
    let mut current = context.builder_text(Kind::Identifier, "a").build();
    for _ in 0..19 {
        let mut node = context.builder(Kind::Tuple);
        node.push(current.clone());
        node.push(current.clone());
        current = node.build();
    }
    let visits = current.print_visits();

    let mut out = String::new();
    let (printed, _, bytes) = stats_for(&mut out, current);
    assert_eq!(printed, visits, "every expanded visit must print");
    assert_eq!(out.len(), bytes);
    assert!(out.len() > 1 << 20, "{} bytes rendered", out.len());
}

struct FailingSink;

impl core::fmt::Write for FailingSink {
    fn write_str(&mut self, _text: &str) -> core::fmt::Result {
        Err(core::fmt::Error)
    }
}

#[test]
fn failing_sink_returns_error() {
    let mut context = Context::new(MAX_NODES);
    let node = context.builder_text(Kind::Identifier, "main").build();

    let mut sink = FailingSink;
    assert!(Printer::new(&mut sink).print(node).is_err());
}

#[test]
fn long_text_prints_completely() {
    // One byte past the old output cap: the printer has no output limit, so
    // the text is written in full instead of being truncated or refused.
    let text = "a".repeat((1 << 20) + 1);
    let mut context = Context::new(MAX_NODES);
    let node = context.builder_owned(Kind::Identifier, &text).build();

    let mut out = String::new();
    Printer::new(&mut out).print(node).expect("no output cap");
    assert_eq!(out.len(), text.len());
}

fn demangle(symbol: &str) -> String {
    let ast =
        crate::swift::parse_bytes_with_budget(symbol.as_bytes(), crate::swift::DEFAULT_BUDGET)
            .expect("symbol demangles");
    format!("{ast}")
}

#[test]
fn reference_storage_types_print_their_ownership() {
    assert_eq!(demangle("$sSiSgXwD"), "weak Swift.Int?");
    assert_eq!(demangle("$sSiXoD"), "unowned Swift.Int");
    assert_eq!(demangle("$sSiXuD"), "unowned(unsafe) Swift.Int");
    assert_eq!(
        demangle("$s4main3fooyySiXwF"),
        "main.foo(weak Swift.Int) -> ()"
    );
}

#[test]
fn escaping_autoclosures_print_like_autoclosures() {
    assert_eq!(demangle("$sSiyXAD"), "@autoclosure () -> Swift.Int");
    assert_eq!(demangle("$sSiyXKD"), "@autoclosure () -> Swift.Int");
}

#[test]
fn global_variable_once_names_its_variables() {
    assert_eq!(
        demangle("$s4main1x_WZ"),
        "one-time initialization function for x"
    );
    assert_eq!(
        demangle("$s4main1x_Wz"),
        "one-time initialization token for x"
    );
    assert_eq!(
        demangle("$s4main1x_1y_WZ"),
        "one-time initialization function for (x, y)"
    );
    // The grammar lists at least one variable.
    assert_eq!(
        crate::swift::parse_bytes_with_budget(b"$s4mainWZ", crate::swift::DEFAULT_BUDGET)
            .unwrap_err(),
        crate::swift::Failure::Invalid
    );
}

#[test]
fn vtable_thunks_name_the_base_method_first() {
    assert_eq!(
        demangle("$s4main3FooC3fooyyFAA3BarCADyyFTV"),
        "vtable thunk for main.Bar.foo() -> () dispatching to main.Foo.foo() -> ()"
    );
}

#[test]
fn metatypes_of_existentials_print_protocol() {
    assert_eq!(demangle("$sypmD"), "Any.Protocol");
    assert_eq!(demangle("$s4main1P_pmD"), "main.P.Protocol");
    assert_eq!(demangle("_TtMP_"), "Any.Protocol");
    // A bare protocol type is not an existential.
    assert_eq!(demangle("$s4main1PPmD"), "main.P.Type");
    // An existential metatype is always `.Type`.
    assert_eq!(demangle("$s4main1P_pXpD"), "main.P.Type");
    assert_eq!(demangle("$sypXpD"), "Any.Type");
    // A compound instance type is parenthesized.
    assert_eq!(
        demangle("$s4main1P_4main1QPpmD"),
        "(main.P & main.Q).Protocol"
    );
    assert_eq!(
        demangle("$s4main1P_XlmD"),
        "(main.P & Swift.AnyObject).Protocol"
    );
    assert_eq!(demangle("$syycmD"), "(() -> ()).Type");
    assert_eq!(demangle("$s4main1P_4main1QPpSgD"), "(main.P & main.Q)?");
}

#[test]
fn old_style_parameters_label_unnamed_elements() {
    assert_eq!(
        demangle("_T08mangling14varargsVsArrayySaySiG3arr_S1ntF"),
        "mangling.varargsVsArray(arr: [Swift.Int], _: Swift.Range) -> ()"
    );
    assert_eq!(
        demangle("_T04main3FooVACSi1a_SitcfC"),
        "main.Foo.init(a: Swift.Int, _: Swift.Int) -> main.Foo"
    );
    assert_eq!(
        demangle("_T04main3fooySi_SitF"),
        "main.foo(Swift.Int, Swift.Int) -> ()"
    );
    // Legacy constructors keep the labels of their parameter tuple.
    assert_eq!(
        demangle("_TFC3red11BaseClassEHcfzT1aSi_S0_"),
        "red.BaseClassEH.init(a: Swift.Int) throws -> red.BaseClassEH"
    );
}

#[test]
fn entities_of_non_function_type_print_like_swift() {
    // A C function pointer follows the name after a space.
    assert_eq!(
        demangle("_TFSqccT_GSqx_"),
        "Swift.Optional.init @convention(c) () -> A?"
    );
    assert_eq!(
        demangle("_TF4main3foocT_T_"),
        "main.foo @convention(c) () -> ()"
    );
    // Any other non-function type follows ` : `, even after a local name.
    assert_eq!(demangle("_TF4main3fooT_"), "main.foo : ()");
    assert_eq!(
        demangle(
            "_TFFV23interface_type_mangling18GenericTypeContextg31closureInGenericPropertyContextxL_3ooFT_x"
        ),
        "ooF #1 : () in interface_type_mangling.GenericTypeContext.\
         closureInGenericPropertyContext.getter : A with unmangled suffix \"x\""
    );
    assert_eq!(
        demangle("_TFC3foo3barCT_S0_"),
        "foo.bar.__allocating_init : () with unmangled suffix \"S0_\""
    );
}

#[test]
fn observers_print_in_lower_case() {
    assert_eq!(demangle("$s4main3fooSivw"), "main.foo.willset : Swift.Int");
    assert_eq!(demangle("$s4main3fooSivW"), "main.foo.didset : Swift.Int");
}
