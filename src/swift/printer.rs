//! Swift demangling printer.
//!
//! Walks the node graph produced by [`crate::swift::parser`] and writes
//! readable text into a [`core::fmt::Write`] sink. The printer borrows node
//! storage; only a few printers collect short-lived child-handle vectors, and
//! none allocates text.
//!
//! The printer has no limits: no recursion cap, no visit cap and no output cap.
//! Shared nodes make the graph a DAG, so the parser charges the expanded
//! visits and a conservative rendered-byte estimate while it builds each node
//! and rejects a symbol outside the resolved expansion gates; a tree that
//! passed that gate prints completely into any working sink. The counters this
//! module keeps are measurements for those tests, not limits.
#![expect(
    clippy::needless_pass_by_value,
    reason = "the printer passes `Rc` node handles by value through `print`'s \
              `Into<Option<NodeRef>>` entry point; borrowing them is a separate refactor"
)]

use alloc::rc::Rc;
use alloc::vec::Vec;
use core::fmt::{self, Write};
use core::marker::PhantomData;

use crate::swift::node::{
    Accessor, AttributeKind, AutoDiffCode, DeinitializerKind, GenericSpecializationKind, Global,
    GlobalOperand, KeyPathKind, Kind, MacroRole, NodeRef, ResumeStyle,
};

/// A child the printer requires; a missing child is a print error.
fn required(node: Option<NodeRef<'_>>) -> Result<NodeRef<'_>, fmt::Error> {
    node.ok_or(fmt::Error)
}

/// The index of a global operand that carries one (`0` otherwise).
const fn operand_index(operand: GlobalOperand) -> u32 {
    match operand {
        GlobalOperand::Index(index) | GlobalOperand::IndexAndResume { index, .. } => index,
        _ => 0,
    }
}

/// Printer state.
pub struct Printer<'a, 'o, W: Write + ?Sized> {
    out: &'o mut W,
    depth: u16,
    max_depth: u16,
    visits: usize,
    marker: PhantomData<&'a ()>,
}

/// A sink that counts the bytes written through it.
///
/// Only the differential tests need the count: the printer itself has no
/// output limit, so nothing in the production path measures its own output.
#[cfg(test)]
struct CountingSink<'a, W: Write + ?Sized> {
    out: &'a mut W,
    written: usize,
}

#[cfg(test)]
impl<W: Write + ?Sized> Write for CountingSink<'_, W> {
    fn write_str(&mut self, text: &str) -> fmt::Result {
        self.out.write_str(text)?;
        self.written = self.written.saturating_add(text.len());
        Ok(())
    }
}

/// Render a retained graph and return the printer's visit/depth/byte counts.
#[cfg(test)]
pub fn stats<W: Write + ?Sized>(
    ast: &crate::swift::Ast<'_>,
    out: &mut W,
) -> Result<(usize, u16, usize), fmt::Error> {
    let mut sink = CountingSink { out, written: 0 };
    let mut printer = Printer::new(&mut sink);
    printer.print(ast.root())?;
    Ok((printer.visits, printer.max_depth, sink.written))
}

impl<'a, 'o, W: Write + ?Sized> Printer<'a, 'o, W> {
    /// Create a printer.
    pub(crate) const fn new(out: &'o mut W) -> Self {
        Self {
            out,
            depth: 0,
            max_depth: 0,
            visits: 0,
            marker: PhantomData,
        }
    }

    /// Print `node`, if present.
    ///
    /// The counters are measurements, not limits: the parser's expansion gates
    /// and the sink are the only bounds.
    pub(crate) fn print(&mut self, node: impl Into<Option<NodeRef<'a>>>) -> fmt::Result {
        let Some(node) = node.into() else {
            return Ok(());
        };
        self.visits = self.visits.saturating_add(1);
        self.depth = self.depth.saturating_add(1);
        self.max_depth = self.max_depth.max(self.depth);
        let result = self.print_node(node);
        self.depth = self.depth.saturating_sub(1);
        result
    }

    fn write(&mut self, text: &str) -> fmt::Result {
        self.out.write_str(text)
    }

    fn write_text(&mut self, id: impl Into<Option<NodeRef<'a>>>) -> fmt::Result {
        if let Some(id) = id.into() {
            id.write_text(&mut self.out)?;
        }
        Ok(())
    }

    fn children(&mut self, id: impl Into<Option<NodeRef<'a>>>, separator: &str) -> fmt::Result {
        let Some(id) = id.into() else {
            return Ok(());
        };
        let children = id.children();
        for (index, child) in children.iter().enumerate() {
            if index > 0 {
                self.write(separator)?;
            }
            self.print(child.clone())?;
        }
        Ok(())
    }

    #[expect(
        clippy::too_many_lines,
        reason = "one match arm per node kind the printer renders"
    )]
    #[expect(
        clippy::match_same_arms,
        reason = "one arm per node kind, in reference printer order"
    )]
    fn print_node(&mut self, id: NodeRef<'a>) -> fmt::Result {
        let kind = id.kind();
        match kind {
            Kind::Module | Kind::Identifier | Kind::BuiltinTypeName => self.write_text(id),
            Kind::LocalDeclName => {
                let name = id.child(1);
                self.print(name.clone())?;
                self.write(" #")?;
                let number = required(id.child(0))?;
                write_number(&mut self.out, number.number_value().saturating_add(1))
            }
            Kind::PrivateDeclName => {
                let discriminator = id.child(0);
                let count = id.child_count();
                self.write("(")?;
                if count > 1 {
                    let name = id.child(1);
                    self.print(name.clone())?;
                    self.write(" in ")?;
                } else {
                    self.write("in ")?;
                }
                self.write_text(discriminator)?;
                self.write(")")
            }
            Kind::RelatedEntityDeclName => {
                let kind = id.child(0);
                let name = id.child(1);
                self.write("related decl '")?;
                self.write_text(kind)?;
                self.write("' for ")?;
                self.print(name.clone())
            }
            Kind::Number | Kind::Integer => write_number(&mut self.out, id.number_value()),
            Kind::Pack => {
                self.write("Pack{")?;
                self.children(id.clone(), ", ")?;
                self.write("}")
            }
            Kind::ConstrainedExistential => {
                self.write("any ")?;
                self.print(id.child(0))?;
                self.write("<")?;
                self.print(id.child(1))?;
                self.write(">")
            }
            Kind::ConstrainedExistentialRequirementList => self.children(id.clone(), ", "),
            Kind::OpaqueType => {
                self.print(id.child(0))?;
                self.write(".")?;
                let index = required(id.child(1))?;
                write_number(&mut self.out, index.number_value())
            }
            Kind::OpaqueReturnType => self.write("some"),
            Kind::OpaqueReturnTypeOf => {
                self.write("<<opaque return type of ")?;
                self.children(id.clone(), "")?;
                self.write(">>")
            }
            Kind::OpaqueReturnTypeIndex => Ok(()),
            Kind::FreestandingMacroExpansion => {
                let context = id.child(0);
                let name = id.child(1);
                let discriminator = required(id.child(2))?;
                self.write("freestanding macro expansion #")?;
                write_number(
                    &mut self.out,
                    discriminator.number_value().saturating_add(1),
                )?;
                self.write(" of ")?;
                self.print(name.clone())?;
                self.write(" in ")?;
                self.print(context.clone())
            }
            Kind::AttachedMacroExpansion => {
                let context = id.child(0);
                let attached = id.child(1);
                let name = id.child(2);
                let discriminator = required(id.child(3))?;
                self.write(id.macro_role().map_or("attached", MacroRole::name))?;
                self.write(" macro @")?;
                self.print(name.clone())?;
                self.write(" expansion #")?;
                write_number(
                    &mut self.out,
                    discriminator.number_value().saturating_add(1),
                )?;
                self.write(" of ")?;
                self.print(attached.clone())?;
                self.write(" in ")?;
                self.print(context.clone())
            }
            Kind::MacroExpansionUniqueName => {
                let context = id.child(0);
                let name = id.child(1);
                let discriminator = required(id.child(2))?;
                self.write("unique name #")?;
                write_number(
                    &mut self.out,
                    discriminator.number_value().saturating_add(1),
                )?;
                self.write(" of ")?;
                self.print(name.clone())?;
                self.write(" in ")?;
                self.print(context.clone())
            }
            Kind::MacroExpansionLoc => {
                let module = id.child(0);
                let buffer = id.child(1);
                let line = required(id.child(2))?;
                let column = required(id.child(3))?;
                self.write("module ")?;
                self.print(module.clone())?;
                self.write(" file ")?;
                self.print(buffer.clone())?;
                self.write(" line ")?;
                write_number(&mut self.out, line.number_value())?;
                self.write(" column ")?;
                write_number(&mut self.out, column.number_value())
            }
            Kind::NegativeInteger => {
                self.write("-")?;
                write_number(&mut self.out, id.number_value())
            }
            Kind::BuiltinFixedArray => {
                self.write("Builtin.FixedArray<")?;
                self.print(id.child(0))?;
                self.write(", ")?;
                self.print(id.child(1))?;
                self.write(">")
            }
            Kind::BuiltinBorrow => {
                self.write("Builtin.Borrow<")?;
                self.print(id.child(0))?;
                self.write(">")
            }
            Kind::AnonymousContext => {
                let name = id.child(0);
                let parent = id.child(1);
                let types = id.child(2);
                self.print(parent.clone())?;
                self.write(".(unknown context at ")?;
                self.print(name.clone())?;
                self.write(")")?;
                if types.as_ref().is_some_and(|types| types.child_count() > 0) {
                    self.write("<")?;
                    self.print(types.clone())?;
                    self.write(">")?;
                }
                Ok(())
            }
            Kind::ProtocolConformance => {
                let ty = id.child(0);
                let protocol = id.child(1);
                let module = id.child(2);
                self.print(ty.clone())?;
                self.write(" : ")?;
                self.print(protocol.clone())?;
                self.write(" in ")?;
                self.print(module.clone())
            }
            Kind::DependentAssociatedTypeRef => {
                if id.child_count() > 1 {
                    let protocol = id.child(1);
                    self.print(protocol.clone())?;
                    self.write(".")?;
                }
                let name = id.child(0);
                self.print(name.clone())
            }
            Kind::AssocTypePath => self.children(id.clone(), "."),
            Kind::ProtocolConformanceRefInTypeModule => {
                self.write("protocol conformance ref (type's module) ")?;
                self.children(id.clone(), "")
            }
            Kind::ProtocolConformanceRefInProtocolModule => {
                self.write("protocol conformance ref (protocol's module) ")?;
                self.children(id.clone(), "")
            }
            Kind::ProtocolConformanceRefInOtherModule => {
                self.write("protocol conformance ref (retroactive) ")?;
                self.children(id.clone(), "")
            }
            Kind::ConcreteProtocolConformance => {
                self.write("concrete protocol conformance ")?;
                self.print(id.child(0))?;
                self.write(" to ")?;
                self.print(id.child(1))?;
                let conditionals = id.child(2);
                if conditionals
                    .as_ref()
                    .is_some_and(|node| node.child_count() > 0)
                {
                    self.write(" with conditional requirements: ")?;
                    self.print(conditionals.clone())?;
                }
                Ok(())
            }
            Kind::AnyProtocolConformanceList => {
                if id.child_count() == 0 {
                    return Ok(());
                }
                self.write("(")?;
                self.children(id.clone(), ", ")?;
                self.write(")")
            }
            Kind::PackProtocolConformance => {
                self.write("pack protocol conformance ")?;
                self.children(id.clone(), "")
            }
            Kind::RetroactiveConformance => self.print(id.child(1)),
            Kind::DependentProtocolConformanceRoot => {
                self.write("dependent root protocol conformance ")?;
                self.print_dependent_index(id.clone())?;
                self.print(id.child(0))?;
                self.write(" to ")?;
                self.print(id.child(1))
            }
            Kind::DependentProtocolConformanceInherited => {
                self.write("dependent inherited protocol conformance ")?;
                self.print_dependent_index(id.clone())?;
                self.print(id.child(0))?;
                self.write(" to ")?;
                self.print(id.child(1))
            }
            Kind::DependentProtocolConformanceAssociated => {
                self.write("dependent associated protocol conformance ")?;
                self.print_dependent_index(id.clone())?;
                self.print(id.child(0))?;
                self.write(" to ")?;
                self.print(id.child(1))
            }
            Kind::DependentProtocolConformanceOpaque => {
                self.write("opaque result conformance ")?;
                self.print(id.child(0))?;
                self.write(" of ")?;
                self.print(id.child(1))
            }
            Kind::DependentAssociatedConformance => {
                self.write("dependent associated conformance ")?;
                self.children(id.clone(), "")
            }
            Kind::InfixOperator | Kind::PrefixOperator | Kind::PostfixOperator => {
                self.write_text(id)?;
                self.write(match kind {
                    Kind::InfixOperator => " infix",
                    Kind::PrefixOperator => " prefix",
                    _ => " postfix",
                })
            }
            Kind::Type | Kind::TypeMangling => {
                let child = id.child(0);
                self.print(child.clone())
            }
            Kind::Class
            | Kind::Structure
            | Kind::Enum
            | Kind::Protocol
            | Kind::TypeAlias
            | Kind::OtherNominal => self.print_nominal(id.clone()),
            Kind::Extension => self.print_extension(id),
            Kind::Tuple => self.print_tuple(id.clone()),
            Kind::TupleElement => self.print_tuple_element(id.clone()),
            Kind::TupleElementName => self.write_text(id),
            Kind::FunctionType
            | Kind::NoEscapeFunctionType
            | Kind::ThinFunctionType
            | Kind::CFunctionPointer
            | Kind::ObjCBlock
            | Kind::EscapingObjCBlock
            | Kind::CalledOnceFunctionType
            | Kind::CalledAtMostOnceFunctionType
            | Kind::AutoClosureType
            | Kind::EscapingAutoClosureType => self.print_function_type(id.clone()),
            Kind::ArgumentTuple => self.print_argument_tuple(id.clone(), None),
            Kind::ReturnType => {
                let child = id.child(0);
                self.print(child.clone())
            }
            Kind::Function => self.print_function(id.clone()),
            Kind::Variable => self.print_variable(id.clone()),
            Kind::Subscript => self.print_subscript(id.clone()),
            Kind::Initializer => {
                self.write("variable initialization expression of ")?;
                self.print(id.child(0))
            }
            Kind::Deinitializer => self.print_deinitializer(id),
            Kind::Allocator | Kind::Constructor => self.print_constructor(id.clone(), kind),
            Kind::PropertyWrapperBackingInitializer => {
                self.write("property wrapper backing initializer of ")?;
                self.print(id.child(0))
            }
            Kind::PropertyWrappedFieldInitAccessor => {
                self.write("property wrapped field init accessor of ")?;
                self.print(id.child(0))
            }
            Kind::PropertyWrapperInitFromProjectedValue => {
                self.write("property wrapper init from projected value of ")?;
                self.print(id.child(0))
            }
            Kind::GenericTypeParamDecl => {
                let context = id.child(0);
                let name = id.child(1);
                self.print(context.clone())?;
                self.write(".")?;
                self.print(name.clone())
            }
            Kind::Macro => self.print_macro(id.clone()),
            Kind::ExplicitClosure | Kind::ImplicitClosure => self.print_closure(id, kind),
            Kind::DefaultArgumentInitializer => self.print_default_argument(id),
            Kind::Accessor => self.print_accessor(id),
            Kind::Static => {
                self.write("static ")?;
                let child = id.child(0);
                self.print(child.clone())
            }
            Kind::BoundGenericClass
            | Kind::BoundGenericStructure
            | Kind::BoundGenericEnum
            | Kind::BoundGenericProtocol
            | Kind::BoundGenericOtherNominal => self.print_bound_generic(id),
            Kind::BoundGenericFunction => self.print_bound_generic_function(id),
            Kind::TypeList => self.children(id.clone(), ", "),
            Kind::DependentGenericParamType => self.print_generic_param(id),
            Kind::ConstrainedExistentialSelf => self.write("Self"),
            Kind::DependentMemberType => {
                let base = id.child(0);
                let name = id.child(1);
                if base.is_some() {
                    self.print(base.clone())?;
                    self.write(".")?;
                }
                self.print(name.clone())
            }
            Kind::DependentGenericType => {
                let signature = id.child(0);
                let ty = id.child(1);
                self.print(signature.clone())?;
                if Self::need_space_before_type(ty.clone()) {
                    self.write(" ")?;
                }
                self.print(ty.clone())
            }
            Kind::DependentGenericSignature => self.print_generic_signature(id.clone()),
            Kind::DependentGenericConformanceRequirement
            | Kind::DependentGenericSameTypeRequirement
            | Kind::DependentGenericBaseClassRequirement
            | Kind::DependentGenericLayoutRequirement
            | Kind::DependentGenericInverseConformanceRequirement => {
                self.print_requirement(id.clone())
            }
            Kind::DependentGenericParamCount => Ok(()),
            Kind::Metatype => {
                let child = id.child(0);
                self.print_with_parens(child.clone())?;
                self.write(Self::metatype_suffix(child))
            }
            Kind::ExistentialMetatype => {
                self.print(id.child(0))?;
                self.write(".Type")
            }
            Kind::InOut => self.print_prefixed(id.clone(), "inout "),
            Kind::Shared => self.print_prefixed(id.clone(), "__shared "),
            Kind::Owned => self.print_prefixed(id.clone(), "__owned "),
            Kind::Unowned => self.print_prefixed(id.clone(), "unowned "),
            Kind::Unmanaged => self.print_prefixed(id.clone(), "unowned(unsafe) "),
            Kind::Weak => self.print_prefixed(id.clone(), "weak "),
            Kind::NoDerivative => self.print_prefixed(id.clone(), "@noDerivative "),
            Kind::Sending => self.print_prefixed(id.clone(), "sending "),
            Kind::Isolated => self.print_prefixed(id.clone(), "isolated "),
            Kind::CompileTimeLiteral => self.print_prefixed(id.clone(), "_const "),
            Kind::ConstValue => self.print_prefixed(id.clone(), "@const "),
            Kind::ProtocolListWithAnyObject => {
                let list = required(id.child(0))?;
                if list.child_count() > 0 {
                    self.children(list.clone(), " & ")?;
                    self.write(" & ")?;
                }
                self.write("Swift.AnyObject")
            }
            Kind::ProtocolListType => {
                let list = required(id.child(0))?;
                if list.child_count() == 0 {
                    self.write("Any")
                } else {
                    self.children(list.clone(), " & ")
                }
            }
            Kind::ProtocolList => self.children(id.clone(), " & "),
            Kind::Suffix => {
                self.write(" with unmangled suffix \"")?;
                self.write_text(id)?;
                self.write("\"")
            }
            Kind::BuiltinVector => self.print_builtin_vector(id),
            Kind::ImplFunctionType => self.print_impl_function_type(id.clone()),
            Kind::ImplParameter | Kind::ImplResult | Kind::ImplYield | Kind::ImplErrorResult => {
                self.print_impl_entry(id)
            }
            Kind::ImplConvention => self.print_impl_convention(id),
            Kind::ImplDifferentiability => self.write("@noDerivative "),
            Kind::ImplSending => self.write("sending "),
            Kind::ImplIsolated | Kind::ImplImplicitLeading => Ok(()),
            Kind::ImplEscaping => self.write("@escaping"),
            Kind::ImplErasedIsolation => self.write("@isolated(any)"),
            Kind::ImplNonisolatedNonsending => self.write("@caller_isolated"),
            Kind::ImplCalledOnce => {
                if id.impl_flag() == 1 {
                    self.write("@callable(once)")
                } else {
                    self.write("@once")
                }
            }
            Kind::ImplDifferentiabilityKind => {
                let text = match id.impl_flag() {
                    b'f' => "@differentiable(_forward)",
                    b'r' => "@differentiable(reverse)",
                    b'l' => "@differentiable(_linear)",
                    _ => "@differentiable",
                };
                self.write(text)
            }
            Kind::ImplCalleeConvention => {
                let text = match id.impl_flag() {
                    b'g' => "@callee_guaranteed",
                    b'x' => "@callee_owned",
                    b't' => "@convention(thin)",
                    _ => "@callee_unowned",
                };
                self.write(text)
            }
            Kind::ImplFunctionConvention => {
                let text = match id.impl_flag() {
                    b'B' => "@convention(block)",
                    b'C' => "@convention(c)",
                    b'M' => "@convention(method)",
                    b'O' => "@convention(objc_method)",
                    b'K' => "@convention(closure)",
                    b'W' => "@convention(witness_method)",
                    b'V' => "@convention(com_method)",
                    _ => "@convention(thin)",
                };
                self.write(text)
            }
            Kind::ImplCoroutineKind => {
                let text = match id.impl_flag() {
                    b'A' => "@yield_once",
                    b'I' => "@yield_once_2",
                    _ => "@yield_many",
                };
                self.write(text)
            }
            Kind::ImplSendable => self.write("@Sendable"),
            Kind::ImplAsync => self.write("@async"),
            Kind::ImplSendingResult => self.write("sending"),
            Kind::TopLevel => self.children(id.clone(), ""),
            Kind::Attribute => {
                let text = match id.attribute_kind() {
                    AttributeKind::Objc => "@objc ",
                    AttributeKind::NonObjc => "@nonobjc ",
                    AttributeKind::Dynamic => "dynamic ",
                    AttributeKind::Super => "super ",
                    AttributeKind::Override => "override ",
                };
                self.write(text)
            }
            Kind::GenericSpecialization => self.print_generic_specialization(id),
            Kind::FunctionSignatureSpecialization => self.print_function_specialization(id.clone()),
            Kind::FuncSpecParam => self.print_func_spec_param(id.clone()),
            Kind::FuncSpecKind | Kind::FuncSpecNumber => Ok(()),
            Kind::GenericSpecializationParam => {
                let child = id.child(0);
                self.print(child.clone())?;
                // A legacy specialization parameter carries the conformances
                // it was bound with; the modern grammar does not.
                for index in 1..id.child_count() {
                    if index == 1 {
                        self.write(" with ")?;
                    } else {
                        self.write(" and ")?;
                    }
                    self.print(id.child(index))?;
                }
                Ok(())
            }
            Kind::DroppedArgument => {
                self.write("param")?;
                write_number(&mut self.out, id.number_value())?;
                self.write("-removed")
            }
            Kind::AutoDiffFunction | Kind::AutoDiffDerivativeVTableThunk => {
                self.print_auto_diff_function(id, kind)
            }
            Kind::AutoDiffSelfReorderingReabstractionThunk => {
                self.print_auto_diff_self_reordering(id)
            }
            Kind::AutoDiffSubsetParametersThunk => self.print_auto_diff_subset(id),
            Kind::DifferentiabilityWitness => self.print_differentiability_witness(id),
            Kind::KeyPathThunkHelper => self.print_keypath_thunk(id),
            Kind::ImplPatternSubstitutions => {
                self.write(" for <")?;
                self.children(id.child(1), ", ")?;
                self.write(">")?;
                Ok(())
            }
            Kind::ImplInvocationSubstitutions => {
                self.write(" for <")?;
                self.children(id.child(0), ", ")?;
                self.write(">")?;
                Ok(())
            }
            Kind::SILBoxTypeWithLayout => {
                let layout = id.child(0);
                let signature = id.child(1);
                let args = id.child(2);
                if signature.is_some() {
                    self.print(signature.clone())?;
                    self.write(" ")?;
                }
                self.print(layout.clone())?;
                if args.is_some() {
                    self.write(" <")?;
                    self.children(args.clone(), ", ")?;
                    self.write(">")?;
                }
                Ok(())
            }
            Kind::SILBoxLayout => {
                self.write("{")?;
                let children = id.children();
                for (index, child) in children.iter().enumerate() {
                    if index > 0 {
                        self.write(",")?;
                    }
                    self.write(" ")?;
                    self.print(child.clone())?;
                }
                self.write(" }")
            }
            Kind::SILBoxMutableField | Kind::SILBoxImmutableField => {
                self.write(if kind == Kind::SILBoxMutableField {
                    "var "
                } else {
                    "let "
                })?;
                self.print(id.child(0))
            }
            Kind::IsSerialized => self.write("serialized"),
            Kind::AutoDiffFunctionKind => self.write(match id.autodiff_code() {
                AutoDiffCode::Forward => "forward-mode derivative",
                AutoDiffCode::Reverse => "reverse-mode derivative",
                AutoDiffCode::Differential => "differential",
                AutoDiffCode::Linear | AutoDiffCode::Pullback => "pullback",
            }),
            Kind::IndexSubset => {
                self.write("{")?;
                let mut first = true;
                let text = id.text().unwrap_or("");
                for (index, byte) in text.bytes().enumerate() {
                    if byte != b'S' {
                        continue;
                    }
                    if !first {
                        self.write(", ")?;
                    }
                    first = false;
                    write_number(&mut self.out, u32::try_from(index).unwrap_or(0))?;
                }
                self.write("}")
            }
            Kind::VTableThunk => {
                // The thunk is named for the base method it overrides and
                // dispatches to the derived implementation.
                self.write("vtable thunk for ")?;
                self.print(id.child(1))?;
                self.write(" dispatching to ")?;
                self.print(id.child(0))
            }
            Kind::GlobalVariableOnceDeclList => {
                if id.child_count() == 1 {
                    return self.print(id.child(0));
                }
                self.write("(")?;
                self.children(id.clone(), ", ")?;
                self.write(")")
            }
            Kind::CoroutineContinuationPrototype => {
                self.write("coroutine continuation prototype for ")?;
                let ty = id.child(0);
                self.print(ty.clone())
            }
            Kind::EnumCase => {
                self.write("enum case for ")?;
                self.print(id.child(0))
            }
            Kind::ValueWitness => {
                self.write_text(id.clone())?;
                self.write(" value witness for ")?;
                self.print(id.child(0))
            }
            Kind::ErrorType => self.write("<ERROR TYPE>"),
            Kind::DynamicSelf => self.write("Self"),
            Kind::IVarInitializer => {
                self.print(id.child(0))?;
                self.write(".__ivar_initializer")
            }
            Kind::IVarDestroyer => {
                self.print(id.child(0))?;
                self.write(".__ivar_destroyer")
            }
            Kind::Global => self.print_global(id),
            Kind::EmptyList | Kind::FirstElementMarker | Kind::VariadicMarker => Ok(()),
            Kind::AsyncAnnotation
            | Kind::ThrowsAnnotation
            | Kind::TypedThrowsAnnotation
            | Kind::ConcurrentFunctionType
            | Kind::DifferentiableFunctionType
            | Kind::GlobalActorFunctionType
            | Kind::SendingResultFunctionType
            | Kind::IsolatedAnyFunctionType
            | Kind::NonIsolatedCallerFunctionType
            | Kind::LabelList => Ok(()),
        }
    }

    fn print_builtin_vector(&mut self, id: NodeRef<'a>) -> fmt::Result {
        self.write_text(id.clone())?;
        self.write("x")?;
        let element = id.child(0);
        if element.is_none() {
            return Ok(());
        }
        let Some(mut inner) = element else {
            return Ok(());
        };
        if inner.kind() == Kind::Type {
            let Some(child) = inner.child(0) else {
                return Ok(());
            };
            inner = child;
        }
        if inner.kind() == Kind::BuiltinTypeName {
            let text = inner.text().unwrap_or("");
            let name = text.strip_prefix("Builtin.").unwrap_or(text);
            self.write(name)?;
        }
        Ok(())
    }

    fn print_prefixed(&mut self, id: NodeRef<'a>, prefix: &str) -> fmt::Result {
        let child = id.child(0);
        self.write(prefix)?;
        self.print(child.clone())
    }

    fn print_nominal(&mut self, id: NodeRef<'a>) -> fmt::Result {
        let context = id.child(0);
        let Some(name) = id.child(1) else {
            return Ok(());
        };
        let name_kind = name.kind();
        let multi_word =
            name_kind == Kind::LocalDeclName || Self::context_prints_type(context.clone());
        if context.is_some() && !multi_word {
            self.print(context.clone())?;
            self.write(".")?;
        }
        self.print(name.clone())?;
        if context.is_some() && multi_word {
            self.write(" in ")?;
            self.print(context.clone())?;
        }
        Ok(())
    }

    /// The suffix of a metatype: `.Protocol` when the instance type is an
    /// existential (a protocol list, possibly with `AnyObject`, or an
    /// existential metatype), `.Type` otherwise. A bare protocol type is
    /// not an existential.
    fn metatype_suffix(ty: Option<NodeRef<'a>>) -> &'static str {
        let mut node = ty;
        while let Some(current) = node.clone().filter(|node| node.kind() == Kind::Type) {
            node = current.child(0);
        }
        let existential = node.is_some_and(|node| {
            matches!(
                node.kind(),
                Kind::ProtocolListType
                    | Kind::ProtocolListWithAnyObject
                    | Kind::ExistentialMetatype
            )
        });
        if existential { ".Protocol" } else { ".Type" }
    }

    fn need_space_before_type(ty: Option<NodeRef<'a>>) -> bool {
        let Some(mut node) = ty else {
            return false;
        };
        while node.kind() == Kind::Type {
            let Some(child) = node.child(0) else {
                return false;
            };
            node = child;
        }
        !matches!(
            node.kind(),
            Kind::FunctionType
                | Kind::NoEscapeFunctionType
                | Kind::ThinFunctionType
                | Kind::CFunctionPointer
                | Kind::ObjCBlock
                | Kind::EscapingObjCBlock
                | Kind::CalledOnceFunctionType
                | Kind::CalledAtMostOnceFunctionType
                | Kind::DependentGenericType
        )
    }

    fn context_prints_type(context: Option<NodeRef<'a>>) -> bool {
        let Some(context) = context else {
            return false;
        };
        matches!(
            context.kind(),
            Kind::Subscript
                | Kind::Variable
                | Kind::Function
                | Kind::Initializer
                | Kind::Allocator
                | Kind::Constructor
                | Kind::Deinitializer
                | Kind::ExplicitClosure
                | Kind::ImplicitClosure
                | Kind::DefaultArgumentInitializer
                | Kind::Accessor
                | Kind::Macro
                | Kind::PropertyWrapperBackingInitializer
                | Kind::PropertyWrappedFieldInitAccessor
                | Kind::PropertyWrapperInitFromProjectedValue
        )
    }

    /// True when `node` prints as more than one word, so a parent entity has
    /// to use the `<name> in <context>` form instead of `<context>.<name>`.
    ///
    /// Mirrors the reference printer: an entity that prints a type, and a
    /// nominal type with a local (multi-word) name, force the suffix form.
    fn is_multi_word_context(node: Option<NodeRef<'a>>) -> bool {
        let Some(node) = node else {
            return false;
        };
        match node.kind() {
            Kind::Class
            | Kind::Structure
            | Kind::Enum
            | Kind::Protocol
            | Kind::TypeAlias
            | Kind::OtherNominal => {
                node.child(1)
                    .is_some_and(|name| name.kind() == Kind::LocalDeclName)
                    || Self::is_multi_word_context(node.child(0))
            }
            Kind::BoundGenericClass
            | Kind::BoundGenericStructure
            | Kind::BoundGenericEnum
            | Kind::BoundGenericProtocol
            | Kind::BoundGenericOtherNominal
            | Kind::BoundGenericFunction
            | Kind::Static => Self::is_multi_word_context(node.child(0)),
            _ => Self::context_prints_type(Some(node)),
        }
    }

    fn print_extension(&mut self, id: NodeRef<'a>) -> fmt::Result {
        let module = id.child(0);
        let ty = id.child(1);
        self.write("(extension in ")?;
        self.print(module.clone())?;
        self.write("):")?;
        self.print(ty.clone())?;
        let signature = id.child(2);
        if signature.is_some() {
            self.print(signature.clone())?;
        }
        Ok(())
    }

    fn print_generic_signature(&mut self, id: NodeRef<'a>) -> fmt::Result {
        let mut counts = Vec::new();
        let mut requirements = Vec::new();
        for child in id.children() {
            match child.kind() {
                Kind::DependentGenericParamCount => {
                    let depth = u32::try_from(counts.len()).map_err(|_| fmt::Error)?;
                    counts.push((depth, child.number_value()));
                }
                _ => requirements.push(child.clone()),
            }
        }
        self.write("<")?;
        let mut first = true;
        for (position, &(depth, count)) in counts.iter().enumerate() {
            if position > 0 {
                self.write("><")?;
                first = true;
            }
            for index in 0..count {
                if !first {
                    self.write(", ")?;
                }
                first = false;
                generic_param_name(&mut self.out, depth, index)?;
            }
        }
        if !requirements.is_empty() {
            self.write(" where ")?;
            let mut first = true;
            for requirement in requirements {
                if !first {
                    self.write(", ")?;
                }
                first = false;
                self.print_requirement(requirement)?;
            }
        }
        self.write(">")
    }

    fn print_requirement(&mut self, id: NodeRef<'a>) -> fmt::Result {
        let kind = id.kind();
        let subject = id.child(0);
        let other = id.child(1);
        self.print(subject.clone())?;
        match kind {
            Kind::DependentGenericSameTypeRequirement => {
                self.write(" == ")?;
                self.print(other.clone())
            }
            Kind::DependentGenericInverseConformanceRequirement => {
                let index = required(id.child(1))?;
                self.write(": ~")?;
                match index.number_value() {
                    0 => self.write("Swift.Copyable"),
                    1 => self.write("Swift.Escapable"),
                    2 => self.write("Swift.Deinitable"),
                    value => {
                        self.write("Swift.<bit ")?;
                        write_number(&mut self.out, value)?;
                        self.write(">")
                    }
                }
            }
            Kind::DependentGenericLayoutRequirement => {
                // Upstream has no printed name for the `B`/`S` constraints, so
                // they print nothing at all.
                let Some(name) = id.layout_code().name() else {
                    return Ok(());
                };
                self.write(": ")?;
                self.write(name)?;
                // The legacy grammar records the size and alignment of a
                // trivial layout; the modern grammar has no such payload.
                if let Some(size) = id.child(1) {
                    self.write("(")?;
                    write_number(&mut self.out, size.number_value())?;
                    if let Some(alignment) = id.child(2) {
                        self.write(", ")?;
                        write_number(&mut self.out, alignment.number_value())?;
                    }
                    self.write(")")?;
                }
                Ok(())
            }
            _ => {
                self.write(": ")?;
                if other.is_none() {
                    self.write("AnyObject")
                } else {
                    self.print(other.clone())
                }
            }
        }
    }

    fn print_generic_param(&mut self, id: NodeRef<'a>) -> fmt::Result {
        generic_param_name(&mut self.out, id.number_value(), id.number_len())
    }

    fn print_tuple(&mut self, id: NodeRef<'a>) -> fmt::Result {
        self.write("(")?;
        self.children(id.clone(), ", ")?;
        self.write(")")
    }

    /// Print a parameter tuple element whose label comes from a label list.
    fn print_tuple_element_unnamed(&mut self, id: NodeRef<'a>) -> fmt::Result {
        if id.kind() != Kind::TupleElement {
            return self.print(id);
        }
        for child in id.children() {
            if !matches!(child.kind(), Kind::VariadicMarker | Kind::TupleElementName) {
                self.print(child.clone())?;
            }
        }
        if id
            .child(0)
            .is_some_and(|first| first.kind() == Kind::VariadicMarker)
        {
            self.write("...")?;
        }
        Ok(())
    }

    fn print_tuple_element(&mut self, id: NodeRef<'a>) -> fmt::Result {
        for child in id.children() {
            let kind = child.kind();
            match kind {
                Kind::VariadicMarker => {}
                Kind::TupleElementName => {
                    self.print(child.clone())?;
                    self.write(": ")?;
                }
                _ => {
                    self.print(child.clone())?;
                }
            }
        }
        // A variadic marker may be the first child.
        let first = id.child(0);
        if first.is_some_and(|first| first.kind() == Kind::VariadicMarker) {
            self.write("...")?;
        }
        Ok(())
    }

    fn find_child(parent: NodeRef<'a>, kind: Kind) -> Option<NodeRef<'a>> {
        parent
            .children()
            .iter()
            .find(|child| child.kind() == kind)
            .cloned()
    }

    fn find_child_of(parent: NodeRef<'a>, kinds: &[Kind]) -> Option<NodeRef<'a>> {
        parent
            .children()
            .iter()
            .find(|child| kinds.contains(&child.kind()))
            .cloned()
    }

    fn print_argument_tuple(
        &mut self,
        id: NodeRef<'a>,
        labels: Option<NodeRef<'a>>,
    ) -> fmt::Result {
        let child = id.child(0);
        let inner = match child.clone() {
            Some(inner) if inner.kind() == Kind::Type => inner.child(0),
            other => other,
        };
        if inner
            .as_ref()
            .is_some_and(|inner| inner.kind() == Kind::Tuple)
        {
            self.write("(")?;
            let has_labels = labels
                .as_ref()
                .is_some_and(|labels| labels.child_count() > 0);
            let elements = inner
                .map(|inner| inner.children().to_vec())
                .unwrap_or_default();
            for (index, element) in elements.into_iter().enumerate() {
                if index > 0 {
                    self.write(", ")?;
                }
                if has_labels {
                    let label = labels.as_ref().and_then(|labels| labels.child(index));
                    match label.as_ref().map(|label| label.kind()) {
                        Some(Kind::Identifier) => {
                            self.write_text(label)?;
                            self.write(": ")?;
                        }
                        Some(Kind::FirstElementMarker) => {
                            self.write("_: ")?;
                        }
                        _ => {}
                    }
                    // The label list replaces an old-style element name.
                    self.print_tuple_element_unnamed(element)?;
                    continue;
                }
                self.print(element)?;
            }
            return self.write(")");
        }
        self.write("(")?;
        self.print(child)?;
        self.write(")")
    }

    fn print_function_type(&mut self, id: NodeRef<'a>) -> fmt::Result {
        let kind = id.kind();
        match kind {
            Kind::ThinFunctionType => self.write("@convention(thin) ")?,
            Kind::CFunctionPointer => self.write("@convention(c) ")?,
            Kind::ObjCBlock => self.write("@convention(block) ")?,
            Kind::EscapingObjCBlock => self.write("@escaping @convention(block) ")?,
            Kind::CalledOnceFunctionType => self.write("@once ")?,
            Kind::CalledAtMostOnceFunctionType => self.write("@callable(once) ")?,
            // Swift prints both autoclosure kinds alike; the escaping one
            // does not add `@escaping`.
            Kind::AutoClosureType | Kind::EscapingAutoClosureType => {
                self.write("@autoclosure ")?;
            }
            _ => {}
        }
        self.print_signature(id.clone(), None)
    }

    /// Print the signature part of a function type: parameters, effects, result.
    fn print_signature(&mut self, func: NodeRef<'a>, labels: Option<NodeRef<'a>>) -> fmt::Result {
        let isolated = Self::find_child_of(
            func.clone(),
            &[
                Kind::IsolatedAnyFunctionType,
                Kind::NonIsolatedCallerFunctionType,
                Kind::GlobalActorFunctionType,
            ],
        );
        let differentiable = Self::find_child(func.clone(), Kind::DifferentiableFunctionType);
        let sendable = Self::find_child(func.clone(), Kind::ConcurrentFunctionType);
        let sending_result = Self::find_child(func.clone(), Kind::SendingResultFunctionType);
        let args = Self::find_child(func.clone(), Kind::ArgumentTuple);
        let ret = Self::find_child(func.clone(), Kind::ReturnType);
        let throws = Self::find_child_of(
            func.clone(),
            &[Kind::ThrowsAnnotation, Kind::TypedThrowsAnnotation],
        );
        let is_async = Self::find_child(func.clone(), Kind::AsyncAnnotation);

        if let Some(node) = isolated {
            match node.kind() {
                Kind::IsolatedAnyFunctionType => self.write("@isolated(any) ")?,
                Kind::NonIsolatedCallerFunctionType => self.write("nonisolated(nonsending) ")?,
                Kind::GlobalActorFunctionType => {
                    let actor = node.child(0);
                    self.write("@")?;
                    self.print(actor.clone())?;
                    self.write(" ")?;
                }
                _ => {}
            }
        }
        if let Some(node) = differentiable {
            match node.autodiff_code() {
                AutoDiffCode::Forward => self.write("@differentiable(_forward) ")?,
                AutoDiffCode::Reverse => self.write("@differentiable(reverse) ")?,
                AutoDiffCode::Linear => self.write("@differentiable(_linear) ")?,
                AutoDiffCode::Differential | AutoDiffCode::Pullback => {
                    self.write("@differentiable ")?;
                }
            }
        }
        if sendable.is_some() {
            self.write("@Sendable ")?;
        }
        if let Some(args) = args {
            self.print_argument_tuple(args, labels)?;
        } else {
            self.write("()")?;
        }
        if is_async.is_some() {
            self.write(" async")?;
        }
        if let Some(node) = throws {
            if node.kind() == Kind::TypedThrowsAnnotation {
                let ty = node.child(0);
                self.write(" throws(")?;
                self.print(ty.clone())?;
                self.write(")")?;
            } else {
                self.write(" throws")?;
            }
        }
        self.write(" -> ")?;
        if sending_result.is_some() {
            self.write("sending ")?;
        }
        match ret {
            Some(ret) => self.print(ret.clone()),
            None => self.write("()"),
        }
    }

    fn function_node(id: NodeRef<'a>) -> Option<NodeRef<'a>> {
        let ty = id
            .children()
            .iter()
            .rev()
            .find(|child| child.kind() == Kind::Type)?
            .clone();
        let mut func = ty.child(0);
        while let Some(node) = func.clone() {
            if node.kind() == Kind::FunctionType {
                return Some(node);
            }
            let next = if node.kind() == Kind::DependentGenericType {
                node.child(1)
            } else {
                node.child(0)
            };
            if next.as_ref().is_some_and(|next| Rc::ptr_eq(next, &node)) {
                break;
            }
            func = next;
        }
        None
    }

    fn print_function(&mut self, id: NodeRef<'a>) -> fmt::Result {
        let context = id.child(0);
        let name = id.child(1);
        let local_name = name
            .as_ref()
            .is_some_and(|name| name.kind() == Kind::LocalDeclName);
        let suffix = local_name || Self::is_multi_word_context(context.clone());
        if context.is_some() && !suffix {
            self.print(context.clone())?;
            self.write(".")?;
        }
        self.print(name.clone())?;
        self.print_entity_signature(id.clone(), local_name)?;
        if context.is_some() && suffix {
            self.write(" in ")?;
            self.print(context.clone())?;
        }
        Ok(())
    }

    /// Print a function-like entity's type after its name.
    ///
    /// As in the reference, a function type prints in function style, after
    /// a space when `spaced` (a local name) is set; a C or thin function
    /// pointer prints after a space; any other type prints as ` : <type>`.
    fn print_entity_signature(&mut self, id: NodeRef<'a>, spaced: bool) -> fmt::Result {
        let labels = Self::find_child(id.clone(), Kind::LabelList);
        if let Some(func) = Self::function_node(id.clone()) {
            if spaced {
                self.write(" ")?;
            }
            if let Some(signature) = Self::generic_signature_of(id) {
                self.print_generic_signature(signature)?;
            }
            return self.print_signature(func, labels);
        }
        let ty = id
            .children()
            .iter()
            .rev()
            .find(|child| child.kind() == Kind::Type)
            .cloned();
        let pointer = ty.as_ref().and_then(|ty| ty.child(0)).is_some_and(|inner| {
            matches!(
                inner.kind(),
                Kind::CFunctionPointer | Kind::ThinFunctionType
            )
        });
        if ty.is_some() {
            self.write(if pointer { " " } else { " : " })?;
            self.print(ty)?;
        }
        Ok(())
    }

    /// Generic signature attached to an entity's type, if any.
    fn generic_signature_of(id: NodeRef<'a>) -> Option<NodeRef<'a>> {
        for child in id.children() {
            if child.kind() == Kind::Type {
                let inner = child.child(0);
                if inner
                    .as_ref()
                    .is_some_and(|inner| inner.kind() == Kind::DependentGenericType)
                {
                    return inner.and_then(|inner| inner.child(0));
                }
            }
        }
        None
    }

    fn print_variable(&mut self, id: NodeRef<'a>) -> fmt::Result {
        let context = id.child(0);
        let name = id.child(1);
        let multi_word = name
            .as_ref()
            .is_some_and(|name| name.kind() == Kind::LocalDeclName)
            || Self::is_multi_word_context(context.clone());
        if context.is_some() && !multi_word {
            self.print(context.clone())?;
            self.write(".")?;
        }
        self.print(name.clone())?;
        if let Some(code) = id.accessor_code() {
            self.write(accessor_name(code))?;
        }
        let ty = id.child(2);
        if ty.is_some() {
            self.write(" : ")?;
            self.print(ty.clone())?;
        }
        if context.is_some() && multi_word {
            self.write(" in ")?;
            self.print(context.clone())?;
        }
        Ok(())
    }

    fn print_subscript(&mut self, id: NodeRef<'a>) -> fmt::Result {
        let context = id.child(0);
        if context.is_some() {
            self.print(context.clone())?;
            self.write(".")?;
        }
        self.write("subscript")?;
        let code = id.accessor_code();
        if let Some(code) = code {
            self.write(accessor_name(code))?;
        }
        let labels = Self::find_child(id.clone(), Kind::LabelList);
        let Some(ty) = id
            .children()
            .iter()
            .rev()
            .find(|child| child.kind() == Kind::Type)
            .cloned()
        else {
            return Ok(());
        };
        let mut outer = ty.clone();
        if outer.kind() == Kind::Type
            && let Some(child) = outer.child(0)
        {
            outer = child;
        }
        let mut inner = outer;
        let mut signature = None;
        if inner.kind() == Kind::DependentGenericType {
            signature = inner.child(0);
            if let Some(nested) = inner.child(1) {
                inner = match nested.kind() {
                    Kind::Type => nested.child(0).unwrap_or(nested),
                    _ => nested,
                };
            }
        }
        if inner.kind() == Kind::FunctionType {
            let function_style = code.is_none_or(|code| code == Accessor::Pseudo);
            if !function_style {
                self.write(" : ")?;
            }
            if signature.is_some() {
                self.print(signature.clone())?;
            }
            self.print_signature(inner.clone(), labels)?;
            Ok(())
        } else {
            self.write(" : ")?;
            self.print(ty.clone())
        }
    }

    fn print_deinitializer(&mut self, id: NodeRef<'a>) -> fmt::Result {
        let context = id.child(0);
        self.print(context.clone())?;
        let is_class = matches!(
            context.as_ref().map(|context| context.kind()),
            Some(Kind::Class | Kind::BoundGenericClass)
        );
        match (id.deinitializer_kind(), is_class) {
            (DeinitializerKind::Deallocating, true) => self.write(".__deallocating_deinit"),
            (DeinitializerKind::IsolatedDeallocating, true) => {
                self.write(".__isolated_deallocating_deinit")
            }
            _ => self.write(".deinit"),
        }
    }

    fn print_constructor(&mut self, id: NodeRef<'a>, kind: Kind) -> fmt::Result {
        let context = id.child(0);
        let private = Self::find_child(id.clone(), Kind::PrivateDeclName);
        let suffix = Self::is_multi_word_context(context.clone());
        if !suffix {
            self.print(context.clone())?;
            self.write(".")?;
        }
        if private.is_some() {
            self.print(private.clone())?;
            self.write(".")?;
        }
        let is_class = matches!(
            context.as_ref().map(|context| context.kind()),
            Some(Kind::Class | Kind::BoundGenericClass)
        );
        if kind == Kind::Allocator && is_class {
            self.write("__allocating_init")?;
        } else {
            self.write("init")?;
        }
        self.print_entity_signature(id.clone(), false)?;
        if suffix {
            self.write(" in ")?;
            self.print(context.clone())?;
        }
        Ok(())
    }

    fn print_macro(&mut self, id: NodeRef<'a>) -> fmt::Result {
        let context = id.child(0);
        let name = id.child(1);
        let labels = id.child(2);
        self.print(context.clone())?;
        self.write(".")?;
        self.print(name.clone())?;
        if let Some(func) = Self::function_node(id.clone()) {
            return self.print_signature(func.clone(), labels);
        }
        let ty = id
            .children()
            .iter()
            .rev()
            .find(|child| child.kind() == Kind::Type)
            .cloned();
        if ty.is_some() {
            self.write(" : ")?;
            self.print(ty.clone())?;
        }
        Ok(())
    }

    fn print_closure(&mut self, id: NodeRef<'a>, kind: Kind) -> fmt::Result {
        let context = id.child(0);
        let ty = id.child(1);
        let index = id.entity_index();
        if kind == Kind::ImplicitClosure {
            self.write("implicit closure #")?;
        } else {
            self.write("closure #")?;
        }
        write_number(&mut self.out, index.saturating_add(1))?;
        if ty.is_some() {
            // A closure's type prints in function style only when it is a
            // function type; other types use the entity colon form.
            if Self::is_function_style(ty.clone()) {
                self.write(" ")?;
            } else {
                self.write(" : ")?;
            }
            self.print(ty.clone())?;
        }
        self.write(" in ")?;
        if context.is_some() {
            self.print(context.clone())?;
        }
        Ok(())
    }

    /// True when `ty` prints as a function signature (no colon separator).
    fn is_function_style(ty: Option<NodeRef<'a>>) -> bool {
        let Some(mut node) = ty else {
            return false;
        };
        while node.kind() == Kind::Type {
            let Some(child) = node.child(0) else {
                return false;
            };
            node = child;
        }
        while node.kind() == Kind::DependentGenericType {
            let Some(inner) = node.child(1) else {
                return false;
            };
            node = inner;
            while node.kind() == Kind::Type {
                let Some(child) = node.child(0) else {
                    return false;
                };
                node = child;
            }
        }
        matches!(
            node.kind(),
            Kind::FunctionType
                | Kind::NoEscapeFunctionType
                | Kind::ThinFunctionType
                | Kind::CFunctionPointer
                | Kind::CalledOnceFunctionType
                | Kind::CalledAtMostOnceFunctionType
        )
    }

    fn print_default_argument(&mut self, id: NodeRef<'a>) -> fmt::Result {
        let context = id.child(0);
        let index = id.entity_index();
        self.write("default argument ")?;
        write_number(&mut self.out, index)?;
        self.write(" of ")?;
        self.print(context.clone())
    }

    fn print_accessor(&mut self, id: NodeRef<'a>) -> fmt::Result {
        let code = id.accessor_code();
        let inner = id.child(0);
        if let Some(code) = code {
            self.write(accessor_name(code))?;
        }
        self.print(inner.clone())
    }

    fn print_bound_generic_function(&mut self, id: NodeRef<'a>) -> fmt::Result {
        let nominal = required(id.child(0))?;
        let args = id.child(1);
        match nominal.kind() {
            Kind::Constructor | Kind::Allocator => {
                let context = nominal.child(0);
                let labels = nominal.child(1);
                let private = Self::find_child(nominal.clone(), Kind::PrivateDeclName);
                self.print(context.clone())?;
                self.write(".")?;
                if private.is_some() {
                    self.print(private.clone())?;
                    self.write(".")?;
                }
                let is_class = matches!(
                    context.as_ref().map(|context| context.kind()),
                    Some(Kind::Class | Kind::BoundGenericClass)
                );
                if nominal.kind() == Kind::Allocator {
                    if is_class {
                        self.write("__allocating_init")?;
                    } else {
                        self.write("init")?;
                    }
                }
                self.write("<")?;
                self.children(args.clone(), ", ")?;
                self.write(">")?;
                match Self::function_node(nominal.clone()) {
                    Some(func) => self.print_signature(func.clone(), labels),
                    None => self.write("() -> ()"),
                }
            }
            _ => {
                let context = nominal.child(0);
                let name = nominal.child(1);
                if context.is_some() {
                    self.print(context.clone())?;
                    self.write(".")?;
                }
                self.print(name.clone())?;
                self.write("<")?;
                self.children(args.clone(), ", ")?;
                self.write(">")?;
                let labels = Self::find_child(nominal.clone(), Kind::LabelList);
                Self::function_node(nominal.clone())
                    .map_or(Ok(()), |func| self.print_signature(func.clone(), labels))
            }
        }
    }

    fn print_bound_generic(&mut self, id: NodeRef<'a>) -> fmt::Result {
        let nominal = required(id.child(0))?;
        let args = required(id.child(1))?;
        if args.child_count() == 0 {
            return self.print(nominal.clone());
        }
        if id.kind() == Kind::BoundGenericProtocol {
            self.children(args.clone(), ", ")?;
            self.write(" as ")?;
            return self.print(nominal.clone());
        }
        if Self::is_sugar(nominal.clone(), "Optional") && args.child_count() == 1 {
            let arg = args.child(0);
            self.print_with_parens(arg)?;
            return self.write("?");
        }
        if Self::is_sugar(nominal.clone(), "Array") && args.child_count() == 1 {
            let arg = args.child(0);
            self.write("[")?;
            self.print(arg.clone())?;
            return self.write("]");
        }
        if Self::is_sugar(nominal.clone(), "Dictionary") && args.child_count() == 2 {
            let key = args.child(0);
            let value = args.child(1);
            self.write("[")?;
            self.print(key.clone())?;
            self.write(" : ")?;
            self.print(value.clone())?;
            return self.write("]");
        }
        self.print(nominal.clone())?;
        self.write("<")?;
        self.children(args.clone(), ", ")?;
        self.write(">")
    }

    fn print_with_parens(&mut self, id: Option<NodeRef<'a>>) -> fmt::Result {
        if Self::is_simple_type(id.clone()) {
            return self.print(id.clone());
        }
        self.write("(")?;
        self.print(id.clone())?;
        self.write(")")
    }

    fn is_simple_type(id: Option<NodeRef<'a>>) -> bool {
        let Some(mut node) = id else {
            return false;
        };
        while node.kind() == Kind::Type {
            let Some(child) = node.child(0) else {
                return false;
            };
            node = child;
        }
        // A protocol list is simple with at most one protocol; with
        // `AnyObject`, only when it has none.
        let protocols = |list: Option<NodeRef<'a>>| list.map_or(0, |list| list.child_count());
        match node.kind() {
            Kind::ProtocolListType => return protocols(node.child(0)) <= 1,
            Kind::ProtocolListWithAnyObject => return protocols(node.child(0)) == 0,
            _ => {}
        }
        matches!(
            node.kind(),
            Kind::Class
                | Kind::Structure
                | Kind::Enum
                | Kind::Protocol
                | Kind::TypeAlias
                | Kind::OtherNominal
                | Kind::BoundGenericClass
                | Kind::BoundGenericStructure
                | Kind::BoundGenericEnum
                | Kind::BoundGenericOtherNominal
                | Kind::BuiltinTypeName
                | Kind::BuiltinVector
                | Kind::DependentGenericParamType
                | Kind::DependentMemberType
                | Kind::Metatype
                | Kind::ExistentialMetatype
                | Kind::Tuple
        )
    }

    fn is_sugar(nominal: NodeRef<'a>, name: &str) -> bool {
        let Some(module) = nominal.child(0) else {
            return false;
        };
        let Some(ident) = nominal.child(1) else {
            return false;
        };
        if module.kind() != Kind::Module || ident.kind() != Kind::Identifier {
            return false;
        }
        module.text_eq("Swift") && ident.text_eq(name)
    }

    fn print_function_specialization(&mut self, id: NodeRef<'a>) -> fmt::Result {
        if id.function_signature_representation_changed() {
            return self.write("representation changed of ");
        }
        self.write("function signature specialization <")?;
        let mut first = true;
        let mut argument = 0_u32;
        for child in id.children() {
            // The serialized marker is not an argument slot.
            if child.kind() == Kind::IsSerialized {
                if !first {
                    self.write(", ")?;
                }
                first = false;
                self.print(child.clone())?;
                continue;
            }
            if child.kind() != Kind::FuncSpecParam {
                continue;
            }
            if child.child_count() > 0 {
                if !first {
                    self.write(", ")?;
                }
                first = false;
                if child.function_spec_is_return() {
                    self.write("Return = ")?;
                } else {
                    self.write("Arg[")?;
                    write_number(&mut self.out, argument)?;
                    self.write("] = ")?;
                }
                self.print_func_spec_param(child.clone())?;
            }
            // An empty parameter still occupies its argument slot.
            argument = argument.saturating_add(1);
        }
        self.write("> of ")
    }

    #[expect(
        clippy::too_many_lines,
        reason = "one match arm per node kind the printer renders"
    )]
    fn print_func_spec_param(&mut self, id: NodeRef<'a>) -> fmt::Result {
        let children = id.children().to_vec();
        let mut index = 0_usize;
        while index < children.len() {
            let child = children[index].clone();
            if child.kind() != Kind::FuncSpecKind {
                index += 1;
                continue;
            }
            let raw = child.number_value();
            if raw >= 1 << 6 {
                self.print_option_set(raw)?;
                index += 1;
                continue;
            }
            // Box/inout conversions print without a bracketed payload.
            if matches!(raw, 6..=8) {
                self.write(func_spec_kind_text(raw))?;
                index += 1;
                continue;
            }
            self.write("[")?;
            self.write(func_spec_kind_text(raw))?;
            let next = children.get(index + 1).cloned();
            let mut close = true;
            match raw {
                0 | 1 | 2 | 3 | 10 => {
                    if let Some(next) = &next {
                        self.write(" : ")?;
                        self.print(next.clone())?;
                    }
                    index += if next.is_some() { 2 } else { 1 };
                }
                4 => {
                    if let Some(next) = &next {
                        self.write(" : ")?;
                        self.write_text(next.clone())?;
                        if let Some(string) = children.get(index + 2) {
                            self.write("'")?;
                            let text = string.text().unwrap_or("");
                            self.write(text.strip_prefix('_').unwrap_or(text))?;
                            self.write("'")?;
                        }
                    }
                    index = children.len();
                }
                5 | 12 => {
                    if let Some(next) = &next {
                        self.write(" : ")?;
                        self.print(next.clone())?;
                    }
                    self.write(", Argument Types : [")?;
                    let mut position = index + 1;
                    if next.is_some() {
                        position += 1;
                    }
                    let mut first = true;
                    while let Some(ty) = children.get(position) {
                        if !first {
                            self.write(", ")?;
                        }
                        first = false;
                        self.print(ty.clone())?;
                        position += 1;
                    }
                    self.write("]")?;
                    close = false;
                    index = children.len();
                }
                9 => {
                    self.write(" : ")?;
                    let mut position = index + 1;
                    for label in ["", "<", ",", ">"] {
                        if !label.is_empty() {
                            self.write(label)?;
                        }
                        let Some(node) = children.get(position) else {
                            break;
                        };
                        self.print(node.clone())?;
                        position += 1;
                    }
                    index = position;
                }
                13 => {
                    self.write(", Components : [")?;
                    let mut position = index + 1;
                    for pair in 0..3 {
                        if pair > 0 {
                            self.write(", ")?;
                        }
                        let Some(node) = children.get(position) else {
                            break;
                        };
                        self.print(node.clone())?;
                        self.write(" : ")?;
                        position += 1;
                        if let Some(node) = children.get(position) {
                            self.print(node.clone())?;
                            position += 1;
                        }
                    }
                    self.write("]]")?;
                    close = false;
                    index = position;
                }
                11 => {
                    self.write(" ")?;
                    if let Some(next) = &next {
                        if next.kind() == Kind::FuncSpecNumber {
                            write_number(&mut self.out, next.number_value())?;
                        } else {
                            self.print(next.clone())?;
                        }
                    }
                    index += if next.is_some() { 2 } else { 1 };
                }
                _ => {
                    index += 1;
                }
            }
            if close {
                self.write("]")?;
            }
        }
        Ok(())
    }

    /// Print a function-specialization option set, in the reference order.
    fn print_option_set(&mut self, raw: u32) -> fmt::Result {
        let mut printed = false;
        for (bit, text) in [
            (1 << 10, "Existential To Protocol Constrained Generic"),
            (1 << 6, "Dead"),
            (1 << 7, "Owned To Guaranteed"),
            (1 << 9, "Guaranteed To Owned"),
            (1 << 8, "Exploded"),
        ] {
            if raw & bit == 0 {
                continue;
            }
            if printed {
                self.write(" and ")?;
            }
            printed = true;
            self.write(text)?;
        }
        Ok(())
    }

    fn print_generic_specialization(&mut self, id: NodeRef<'a>) -> fmt::Result {
        let description = match id.generic_specialization() {
            Some((GenericSpecializationKind::NotReabstracted, _)) => {
                "generic not re-abstracted specialization"
            }
            Some((GenericSpecializationKind::Prespecialized, _)) => "generic pre-specialization",
            Some((GenericSpecializationKind::Inlined, _)) => "inlined generic function",
            Some((
                GenericSpecializationKind::Generic | GenericSpecializationKind::ResilienceDomain,
                _,
            ))
            | None => "generic specialization",
        };
        self.write(description)?;
        self.write(" <")?;
        let mut first = true;
        for child in id.children() {
            if child.kind() == Kind::DroppedArgument {
                continue;
            }
            if !first {
                self.write(", ")?;
            }
            first = false;
            self.print(child.clone())?;
        }
        self.write("> of ")
    }

    fn print_impl_function_type(&mut self, id: NodeRef<'a>) -> fmt::Result {
        let children = id.children().to_vec();
        let pattern_subs = children
            .iter()
            .find(|child| child.kind() == Kind::ImplPatternSubstitutions)
            .cloned();
        let invocation_subs = children
            .iter()
            .find(|child| child.kind() == Kind::ImplInvocationSubstitutions)
            .cloned();
        let mut first_attr = true;
        let mut sending_result = false;
        let mut index = 0_usize;
        // Attributes come first and are space separated.
        while let Some(child) = children.get(index) {
            let kind = child.kind();
            if matches!(
                kind,
                Kind::ImplParameter | Kind::ImplResult | Kind::ImplYield | Kind::ImplErrorResult
            ) {
                break;
            }
            if kind == Kind::ImplSendingResult {
                sending_result = true;
                index += 1;
                continue;
            }
            if matches!(
                kind,
                Kind::ImplPatternSubstitutions | Kind::ImplInvocationSubstitutions
            ) {
                index += 1;
                continue;
            }
            if !first_attr {
                self.write(" ")?;
            }
            first_attr = false;
            self.print(child.clone())?;
            index += 1;
        }
        if !first_attr {
            self.write(" ")?;
        }
        if let Some(pattern_subs) = &pattern_subs {
            self.write("@substituted ")?;
            self.print(pattern_subs.child(0))?;
            self.write(" ")?;
        }
        self.write("(")?;
        let mut first = true;
        while let Some(child) = children.get(index) {
            if child.kind() == Kind::ImplParameter {
                if !first {
                    self.write(", ")?;
                }
                first = false;
                self.print(child.clone())?;
            }
            index += 1;
        }
        self.write(") -> ")?;
        if sending_result {
            self.write("sending ")?;
        }
        self.write("(")?;
        let mut first = true;
        for child in &children {
            if matches!(
                child.kind(),
                Kind::ImplResult | Kind::ImplYield | Kind::ImplErrorResult
            ) {
                if !first {
                    self.write(", ")?;
                }
                first = false;
                self.print(child.clone())?;
            }
        }
        self.write(")")?;
        if let Some(pattern_subs) = &pattern_subs {
            self.write(" for <")?;
            self.children(pattern_subs.child(1), "")?;
            self.write(">")?;
        } else if let Some(invocation_subs) = &invocation_subs {
            self.write(" for <")?;
            self.print(invocation_subs.child(0))?;
            self.write(">")?;
        }
        Ok(())
    }

    fn print_impl_entry(&mut self, id: NodeRef<'a>) -> fmt::Result {
        if id.kind() == Kind::ImplErrorResult {
            self.write("@error ")?;
        } else if id.kind() == Kind::ImplYield {
            self.write("@yields ")?;
        }
        let convention = id.child(0);
        self.print(convention.clone())?;
        self.write(" ")?;
        for child in id.children() {
            let is_convention = convention
                .as_ref()
                .is_some_and(|convention| Rc::ptr_eq(convention, child));
            if !is_convention
                && !matches!(child.kind(), Kind::ImplIsolated | Kind::ImplImplicitLeading)
            {
                self.print(child.clone())?;
            }
        }
        Ok(())
    }

    #[expect(clippy::match_same_arms, reason = "one arm per convention code")]
    fn print_impl_convention(&mut self, id: NodeRef<'a>) -> fmt::Result {
        let text = match id.impl_flag() {
            b'i' => "@in",
            b'c' => "@in_constant",
            b'l' => "@inout",
            b'b' => "@inout_aliasable",
            b'n' => "@in_guaranteed",
            b'X' => "@in_cxx",
            b'x' => "@owned",
            b'g' => "@guaranteed",
            b'e' => "@deallocating",
            b'y' => "@unowned",
            b'v' => "@pack_owned",
            b'p' => "@pack_guaranteed",
            b'm' => "@pack_inout",
            b'r' => "@out",
            b'o' => "@owned",
            b'd' => "@unowned",
            b'u' => "@unowned_inner_pointer",
            b'a' => "@autoreleased",
            b'k' => "@pack_out",
            _ => "",
        };
        self.write(text)
    }

    fn print_keypath_thunk(&mut self, id: NodeRef<'a>) -> fmt::Result {
        match id.keypath_kind() {
            KeyPathKind::UnappliedMethod => self.write("key path unapplied method ")?,
            KeyPathKind::AppliedMethod => self.write("key path applied method ")?,
            KeyPathKind::Setter => self.write("key path setter for ")?,
            KeyPathKind::Getter => self.write("key path getter for ")?,
        }
        self.print(id.child(0))?;
        self.write(" : ")?;
        for child in id.children().iter().skip(1) {
            if child.kind() == Kind::IsSerialized {
                self.write(", ")?;
            }
            self.print(child.clone())?;
        }
        Ok(())
    }

    fn print_dependent_index(&mut self, id: NodeRef<'a>) -> fmt::Result {
        if let Some(index) = id.child(2) {
            self.write("#")?;
            write_number(&mut self.out, index.number_value())?;
            self.write(" ")?;
        }
        Ok(())
    }

    fn print_auto_diff_function(&mut self, id: NodeRef<'a>, kind: Kind) -> fmt::Result {
        let children = id.children().to_vec();
        let Some(prefix_end) = children
            .iter()
            .position(|child| child.kind() == Kind::AutoDiffFunctionKind)
        else {
            return Err(fmt::Error);
        };
        if kind == Kind::AutoDiffDerivativeVTableThunk {
            self.write("vtable thunk for ")?;
        }
        self.print(children[prefix_end].clone())?;
        self.write(" of ")?;
        let mut signature = None;
        for (index, child) in children.iter().take(prefix_end).enumerate() {
            if index + 1 == prefix_end && child.kind() == Kind::DependentGenericSignature {
                signature = Some(child.clone());
                break;
            }
            self.print(child.clone())?;
        }
        self.write(" with respect to parameters ")?;
        self.print(children.get(prefix_end + 1).cloned())?;
        self.write(" and results ")?;
        self.print(children.get(prefix_end + 2).cloned())?;
        if signature.is_some() {
            self.write(" with ")?;
            self.print(signature)?;
        }
        Ok(())
    }

    fn print_auto_diff_self_reordering(&mut self, id: NodeRef<'a>) -> fmt::Result {
        let count = id.child_count();
        if count < 3 {
            return Err(fmt::Error);
        }
        let from = id.child(0);
        let to = id.child(1);
        let mut index = 2;
        let mut signature = None;
        if id
            .child(index)
            .is_some_and(|node| node.kind() == Kind::DependentGenericSignature)
        {
            signature = id.child(index);
            index += 1;
        }
        self.write("autodiff self-reordering reabstraction thunk for ")?;
        self.print(id.child(index))?;
        if signature.is_some() {
            self.print(signature)?;
            self.write(" ")?;
        }
        self.write(" from ")?;
        self.print(from)?;
        self.write(" to ")?;
        self.print(to)
    }

    fn print_auto_diff_subset(&mut self, id: NodeRef<'a>) -> fmt::Result {
        let count = id.child_count();
        if count < 5 {
            return Err(fmt::Error);
        }
        let to_params = id.child(count - 1);
        let results = id.child(count - 2);
        let params = id.child(count - 3);
        let kind = id.child(count - 4);
        let prefix_end = count - 4;
        self.write("autodiff subset parameters thunk for ")?;
        self.print(kind)?;
        self.write(" from ")?;
        if prefix_end == 0 {
            return Err(fmt::Error);
        }
        if prefix_end == 1 {
            self.print(id.child(0))?;
        } else {
            for index in 0..prefix_end - 1 {
                self.print(id.child(index))?;
            }
        }
        self.write(" with respect to parameters ")?;
        self.print(params)?;
        self.write(" and results ")?;
        self.print(results)?;
        self.write(" to parameters ")?;
        self.print(to_params)?;
        if prefix_end > 1 {
            self.write(" of type ")?;
            self.print(id.child(prefix_end - 1))?;
        }
        Ok(())
    }

    fn print_differentiability_witness(&mut self, id: NodeRef<'a>) -> fmt::Result {
        let count = id.child_count();
        let has_signature = count > 0
            && id
                .child(count - 1)
                .is_some_and(|node| node.kind() == Kind::DependentGenericSignature);
        let kind_index = count.saturating_sub(if has_signature { 4 } else { 3 });
        let kind_node = required(id.child(kind_index))?;
        self.write(match kind_node.number_value() {
            value if value == u32::from(b'f') => "forward-mode",
            value if value == u32::from(b'r') => "reverse-mode",
            value if value == u32::from(b'd') => "normal",
            _ => "linear",
        })?;
        self.write(" differentiability witness for ")?;
        for index in 0..kind_index {
            self.print(id.child(index))?;
        }
        self.write(" with respect to parameters ")?;
        self.print(id.child(kind_index + 1))?;
        self.write(" and results ")?;
        self.print(id.child(kind_index + 2))?;
        if has_signature {
            self.write(" with ")?;
            self.print(id.child(count - 1))?;
        }
        Ok(())
    }

    #[expect(
        clippy::too_many_lines,
        reason = "one match arm per node kind the printer renders"
    )]
    fn print_global(&mut self, id: NodeRef<'a>) -> fmt::Result {
        let Some(code) = id.global_code() else {
            return Err(fmt::Error);
        };
        let child = id.child(0);
        let operand = id.global_operand();
        let index = operand_index(operand);
        match code {
            Global::TypeMetadata => self.with_child("type metadata for ", child),
            Global::FullTypeMetadata => self.with_child("full type metadata for ", child),
            Global::TypeMetadataPattern => {
                self.with_child("generic type metadata pattern for ", child)
            }
            Global::TypeMetadataAccess => self.with_child("type metadata accessor for ", child),
            Global::TypeMetadataInstantiationCache => {
                self.with_child("type metadata instantiation cache for ", child)
            }
            Global::TypeMetadataInstantiationFunction => {
                self.with_child("type metadata instantiation function for ", child)
            }
            Global::TypeMetadataSingletonInitCache => {
                self.with_child("type metadata singleton initialization cache for ", child)
            }
            // Swift has one completion-function kind; both codes print it.
            Global::TypeMetadataCompletionFunction | Global::GenericTypeCompletionFunction => {
                self.with_child("type metadata completion function for ", child)
            }
            Global::TypeMetadataLazyCache => {
                self.with_child("lazy cache variable for type metadata for ", child)
            }
            Global::TypeMetadataDemanglingCache => {
                self.with_child("demangling cache variable for type metadata for ", child)
            }
            Global::TypeMetadataMangledNameRef => {
                self.with_child("mangled name ref for type metadata for ", child)
            }
            Global::NominalTypeDescriptor => self.with_child("nominal type descriptor for ", child),
            Global::PropertyDescriptor => self.with_child("property descriptor for ", child),
            Global::OpaqueTypeDescriptor => self.with_child("opaque type descriptor for ", child),
            Global::ClassMetaclass => self.with_child("metaclass for ", child),
            Global::ClassMethodLookupFunction => {
                self.with_child("method lookup function for ", child)
            }
            Global::ObjcMetadataUpdate => {
                self.with_child("ObjC metadata update function for ", child)
            }
            Global::ObjcResilientClassStub => {
                self.with_child("ObjC resilient class stub for ", child)
            }
            Global::FullObjcResilientClassStub => {
                self.with_child("full ObjC resilient class stub for ", child)
            }
            Global::ClassMetadataBaseOffset => {
                self.with_child("class metadata base offset for ", child)
            }
            Global::ModuleDescriptor => self.with_child("module descriptor ", child),
            Global::ExtensionDescriptor => self.with_child("extension descriptor ", child),
            Global::AnonymousContextDescriptor => self.with_child("anonymous descriptor ", child),
            Global::ProtocolDescriptor => self.with_child("protocol descriptor for ", child),
            Global::ProtocolDescriptorRecord => {
                self.with_child("protocol descriptor runtime record for ", child)
            }
            Global::NominalTypeDescriptorRecord => {
                self.with_child("nominal type descriptor runtime record for ", child)
            }
            Global::ProtocolConformanceDescriptorRecord => {
                self.with_child("protocol conformance descriptor runtime record for ", child)
            }
            Global::AccessibleFunctionRecord => {
                self.with_child("accessible function runtime record for ", child)
            }
            Global::ProtocolConformanceDescriptor => {
                self.with_child("protocol conformance descriptor for ", child)
            }
            Global::ProtocolWitnessTable => self.with_child("protocol witness table for ", child),
            Global::ProtocolWitnessTableAccessor => {
                self.with_child("protocol witness table accessor for ", child)
            }
            Global::ProtocolWitnessTablePattern => {
                self.with_child("protocol witness table pattern for ", child)
            }
            Global::GenericWitnessTableInstantiationFunction => self.with_child(
                "instantiation function for generic protocol witness table for ",
                child,
            ),
            Global::GenericProtocolWitnessTable => {
                self.with_child("generic protocol witness table for ", child)
            }
            Global::ResilientProtocolWitnessTable => {
                self.with_child("resilient protocol witness table for ", child)
            }
            Global::ProtocolSelfConformanceWitnessTable => {
                self.with_child("protocol self-conformance witness table for ", child)
            }
            Global::ValueWitnessTable => self.with_child("value witness table for ", child),
            Global::FieldOffset => {
                self.write(if matches!(operand, GlobalOperand::Directness(true)) {
                    "indirect "
                } else {
                    "direct "
                })?;
                self.with_child("field offset for ", child)
            }
            Global::EnumTagIndex => self.with_child("resilient enum tag index for ", child),
            Global::LazyProtocolWitnessTableCache => {
                self.write("lazy protocol witness table cache variable for type ")?;
                self.print(child.clone())?;
                self.write(" and conformance ")?;
                let conformance = id.child(1);
                self.print(conformance.clone())
            }
            Global::LazyProtocolWitnessTableAccessor => {
                self.write("lazy protocol witness table accessor for type ")?;
                self.print(child.clone())?;
                self.write(" and conformance ")?;
                let conformance = id.child(1);
                self.print(conformance.clone())
            }
            Global::AssociatedTypeMetadataAccessor => {
                let name = id.child(1);
                self.write("associated type metadata accessor for ")?;
                self.print(name.clone())?;
                self.write(" in ")?;
                self.print(child.clone())
            }
            Global::AssociatedTypeWitnessTableAccessor => {
                let associated = id.child(1);
                let protocol = id.child(2);
                self.write("associated type witness table accessor for ")?;
                self.print(associated.clone())?;
                self.write(" : ")?;
                self.print(protocol.clone())?;
                self.write(" in ")?;
                self.print(child.clone())
            }
            Global::BaseProtocolWitnessTableAccessor => {
                let protocol = id.child(1);
                self.write("base witness table accessor for ")?;
                self.print(protocol.clone())?;
                self.write(" in ")?;
                self.print(child.clone())
            }
            Global::OutlinedCopy => self.with_child("outlined copy of ", child),
            Global::OutlinedConsume => self.with_child("outlined consume of ", child),
            Global::OutlinedRetain => self.with_child("outlined retain of ", child),
            Global::OutlinedRelease => self.with_child("outlined release of ", child),
            Global::OutlinedInitWithTake => self.with_child("outlined init with take of ", child),
            Global::OutlinedInitWithCopy => self.with_child("outlined init with copy of ", child),
            Global::OutlinedAssignWithTake => {
                self.with_child("outlined assign with take of ", child)
            }
            Global::OutlinedAssignWithCopy => {
                self.with_child("outlined assign with copy of ", child)
            }
            Global::OutlinedDestroy => self.with_child("outlined destroy of ", child),
            Global::OutlinedEnumTagStore => self.with_child("outlined enum tag store of ", child),
            Global::OutlinedEnumProjectData => {
                self.with_child("outlined enum project data for load of ", child)
            }
            Global::OutlinedEnumGetTag => self.with_child("outlined enum get tag of ", child),
            Global::OutlinedBridgedMethod => {
                self.write("outlined bridged method (")?;
                self.write_text(id)?;
                self.write(") of ")?;
                self.print(child.clone())
            }
            Global::OutlinedVariable => {
                self.write("outlined variable #")?;
                write_number(&mut self.out, index)?;
                self.write(" of ")?;
                self.print(child.clone())
            }
            Global::OutlinedReadOnlyObject => {
                self.write("outlined read-only object #")?;
                write_number(&mut self.out, index)?;
                self.write(" of ")?;
                self.print(child.clone())
            }
            Global::PartialApplyForwarder => {
                self.write("partial apply forwarder")?;
                if id.child_count() > 0 {
                    self.write(" for ")?;
                    self.children(id.clone(), "")?;
                }
                Ok(())
            }
            Global::PartialApplyObjcForwarder => {
                self.write("partial apply ObjC forwarder")?;
                if id.child_count() > 0 {
                    self.write(" for ")?;
                    self.children(id.clone(), "")?;
                }
                Ok(())
            }
            Global::ReabstractionThunk | Global::ReabstractionThunkHelper => {
                self.write("reabstraction thunk ")?;
                if code == Global::ReabstractionThunkHelper {
                    self.write("helper ")?;
                }
                let count = id.child_count();
                let index = if count == 3 {
                    let signature = id.child(0);
                    self.print(signature.clone())?;
                    self.write(" ")?;
                    1
                } else {
                    0
                };
                self.write("from ")?;
                self.print(id.child(index + 1))?;
                self.write(" to ")?;
                self.print(id.child(index))
            }
            Global::DistributedThunk => self.write("distributed thunk "),
            Global::DistributedAccessor => self.write("distributed accessor for "),
            Global::DynamicallyReplaceableFunctionVar => {
                self.write("dynamically replaceable variable for ")
            }
            Global::DynamicallyReplaceableFunctionKey => {
                self.write("dynamically replaceable key for ")
            }
            Global::DynamicallyReplaceableFunctionImpl => {
                self.write("dynamically replaceable thunk for ")
            }
            Global::CoroFunctionPointer => self.write("coro function pointer to "),
            Global::ReflectionMetadataAssocTypeDescriptor => {
                self.write("reflection metadata associated type descriptor ")?;
                self.print(id.child(0))
            }
            Global::OpaqueTypeDescriptorRecord => {
                self.write("opaque type descriptor runtime record for ")?;
                self.print(id.child(0))
            }
            Global::ProtocolSelfConformanceDescriptor => {
                self.write("protocol self-conformance descriptor for ")?;
                self.print(id.child(0))
            }
            Global::AssociatedTypeDescriptor => {
                self.write("associated type descriptor for ")?;
                self.print(id.child(0))
            }
            Global::ProtocolRequirementsBaseDescriptor => {
                self.write("protocol requirements base descriptor for ")?;
                self.print(id.child(0))
            }
            Global::AssociatedConformanceDescriptor
            | Global::DefaultAssociatedConformanceAccessor => {
                if code == Global::AssociatedConformanceDescriptor {
                    self.write("associated conformance descriptor for ")?;
                } else {
                    self.write("default associated conformance accessor for ")?;
                }
                self.print(id.child(0))?;
                self.write(".")?;
                self.print(id.child(1))?;
                self.write(": ")?;
                self.print(id.child(2))
            }
            Global::BaseConformanceDescriptor => {
                self.write("base conformance descriptor for ")?;
                self.print(id.child(0))?;
                self.write(": ")?;
                self.print(id.child(1))
            }
            Global::DefaultAssociatedTypeMetadataAccessor => {
                self.write("default associated type metadata accessor for ")?;
                self.print(id.child(0))
            }
            Global::ProtocolWitnessThunk => {
                self.write("protocol witness for ")?;
                self.print(id.child(1))?;
                self.write(" in conformance ")?;
                self.print(id.child(0))
            }
            Global::ProtocolSelfConformanceWitnessThunk => {
                self.with_child("protocol self-conformance witness for ", child)
            }
            Global::DefaultOverride => self.write("default override of "),
            Global::DispatchThunk => self.with_child("dispatch thunk of ", child),
            Global::MethodDescriptor => self.with_child("method descriptor for ", child),
            Global::MergedFunction => self.with_child("merged ", child),
            Global::BackDeploymentThunk => self.with_child("back deployment thunk for ", child),
            Global::BackDeploymentFallback => {
                self.with_child("back deployment fallback for ", child)
            }
            Global::BackDeploymentCopy => self.with_child("back deployment copy of ", child),
            Global::HasSymbolQuery => self.with_child("#_hasSymbol query for ", child),
            Global::AsyncFunctionPointer => self.with_child("async function pointer to ", child),
            Global::ResumePartialFunction => {
                let style = match operand {
                    GlobalOperand::IndexAndResume { style, .. } => style,
                    _ => ResumeStyle::Await,
                };
                self.write("(")?;
                write_number(&mut self.out, index)?;
                self.write(") ")?;
                if style == ResumeStyle::Suspend {
                    self.write("suspend")?;
                } else {
                    self.write("await")?;
                }
                self.write(" resume partial function for ")?;
                self.print(child.clone())
            }
            // The context is not printed: Swift names only the variables.
            Global::GlobalVariableOnceFunction => {
                self.with_child("one-time initialization function for ", id.child(1))
            }
            Global::GlobalVariableOnceToken => {
                self.with_child("one-time initialization token for ", id.child(1))
            }
            _ => Err(fmt::Error),
        }
    }

    fn with_child(&mut self, prefix: &str, child: impl Into<Option<NodeRef<'a>>>) -> fmt::Result {
        self.write(prefix)?;
        self.print(child)
    }
}

#[expect(clippy::match_same_arms, reason = "one arm per accessor kind")]
const fn accessor_name(code: Accessor) -> &'static str {
    match code {
        Accessor::Getter => ".getter",
        Accessor::GlobalGetter => ".getter",
        Accessor::Setter => ".setter",
        Accessor::MaterializeForSet => ".materializeForSet",
        Accessor::Read => ".read",
        Accessor::Modify => ".modify",
        Accessor::WillSet => ".willset",
        Accessor::DidSet => ".didset",
        Accessor::Modify2 => ".yielding_mutate",
        Accessor::YieldingBorrow => ".yielding_borrow",
        Accessor::Borrow => ".borrow",
        Accessor::YieldingModify => ".modify",
        Accessor::MutableAddressor => ".unsafeMutableAddressor",
        Accessor::NonMutableAddressor => ".unsafeAddressor",
        Accessor::Pseudo => "",
        Accessor::OwningAddressor => ".owningAddressor",
        Accessor::NativeOwningAddressor => ".nativeOwningAddressor",
        Accessor::NativePinningAddressor => ".nativePinningAddressor",
        Accessor::OwningMutableAddressor => ".owningMutableAddressor",
        Accessor::NativeOwningMutableAddressor => ".nativeOwningMutableAddressor",
        Accessor::NativePinningMutableAddressor => ".nativePinningMutableAddressor",
    }
}

fn write_number<W: Write + ?Sized>(out: &mut W, value: u32) -> fmt::Result {
    let mut digits = [0_u8; 10];
    let mut count = 0_usize;
    let mut value = value;
    if value == 0 {
        return out.write_str("0");
    }
    while value > 0 {
        digits[count] = b'0' + u8::try_from(value % 10).map_err(|_| fmt::Error)?;
        value /= 10;
        count += 1;
    }
    for index in (0..count).rev() {
        out.write_str(core::str::from_utf8(&digits[index..=index]).map_err(|_| fmt::Error)?)?;
    }
    Ok(())
}

fn generic_param_name<W: Write + ?Sized>(out: &mut W, depth: u32, index: u32) -> fmt::Result {
    // Index 0..25 is `A`..`Z`, then each further base-26 digit is appended:
    // 26 is `AB`, 27 `BB`, and so on, matching the reference printer.
    let mut buf = [0_u8; 4];
    let mut value = index;
    loop {
        let letter = char::from(b'A' + u8::try_from(value % 26).map_err(|_| fmt::Error)?);
        out.write_str(letter.encode_utf8(&mut buf))?;
        value /= 26;
        if value == 0 {
            break;
        }
    }
    if depth > 0 {
        write_number(out, depth)?;
    }
    Ok(())
}

const fn func_spec_kind_text(raw: u32) -> &'static str {
    match raw {
        0 => "Constant Propagated Function",
        1 => "Constant Propagated Global",
        2 => "Constant Propagated Integer",
        3 => "Constant Propagated Float",
        4 => "Constant Propagated String",
        5 => "Closure Propagated",
        6 => "Value Promoted from Box",
        7 => "Stack Promoted from Box",
        8 => "InOut Converted to Out",
        9 => "Constant Propagated KeyPath",
        10 => "Constant Propagated Struct",
        11 => "Same As Argument",
        12 => "Escaping Closure Propagated",
        13 => "Differentiable Function Propagated",
        _ => "Specialization",
    }
}

#[cfg(test)]
mod tests {
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
        let mut out = String::new();
        crate::swift::print(symbol, &mut out).expect("symbol demangles");
        out
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
        let mut out = String::new();
        assert!(crate::swift::print("$s4mainWZ", &mut out).is_err());
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
}
