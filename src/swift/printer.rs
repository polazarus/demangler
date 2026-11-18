//! Swift demangling printer.
//!
//! Walks the arena produced by [`crate::swift::parser`] and writes readable
//! text into a [`core::fmt::Write`] sink. No allocation takes place.

use core::fmt::{self, Write};

use crate::swift::node::{Accessor, Arena, Global, Kind, NIL, NodeId};

/// Maximum print recursion depth.
const DEPTH_CAP: u8 = 128;

/// Printer state.
pub(crate) struct Printer<'a, W: Write + ?Sized> {
    arena: &'a Arena,
    out: &'a mut W,
    depth: u8,
}

impl<'a, W: Write + ?Sized> Printer<'a, W> {
    /// Create a printer.
    pub(crate) const fn new(arena: &'a Arena, out: &'a mut W) -> Self {
        Self {
            arena,
            out,
            depth: 0,
        }
    }

    /// Print the node `id`.
    pub(crate) fn print(&mut self, id: NodeId) -> fmt::Result {
        if id == NIL {
            return Ok(());
        }
        self.depth = self.depth.wrapping_add(1);
        if self.depth > DEPTH_CAP {
            self.depth = self.depth.wrapping_sub(1);
            return Err(fmt::Error);
        }
        let result = self.print_node(id);
        self.depth = self.depth.wrapping_sub(1);
        result
    }

    fn write(&mut self, text: &str) -> fmt::Result {
        self.out.write_str(text)
    }

    fn children(&mut self, id: NodeId, separator: &str) -> fmt::Result {
        let mut first = true;
        let mut child = self.arena.node(id).first;
        let mut guard = 0_usize;
        while child != NIL {
            guard += 1;
            if guard > crate::swift::node::NODE_CAP {
                return Err(fmt::Error);
            }
            if !first {
                self.write(separator)?;
            }
            first = false;
            self.print(child)?;
            child = self.arena.node(child).next;
        }
        Ok(())
    }

    fn print_node(&mut self, id: NodeId) -> fmt::Result {
        let kind = self.arena.kind(id);
        match kind {
            Kind::Module | Kind::Identifier | Kind::BuiltinTypeName => {
                self.write(self.arena.text(id))
            }
            Kind::LocalDeclName => {
                let name = self.arena.child(id, 1);
                self.print(name)?;
                self.write(" #")?;
                let number = self.arena.child(id, 0);
                write_number(self.out, self.arena.node(number).value.saturating_add(1))
            }
            Kind::PrivateDeclName => {
                let discriminator = self.arena.child(id, 0);
                let count = self.arena.child_count(id);
                self.write("(")?;
                if count > 1 {
                    let name = self.arena.child(id, 1);
                    self.print(name)?;
                    self.write(" in ")?;
                } else {
                    self.write("in ")?;
                }
                self.write(self.arena.text(discriminator))?;
                self.write(")")
            }
            Kind::RelatedEntityDeclName => {
                let kind = self.arena.child(id, 0);
                let name = self.arena.child(id, 1);
                self.write("related decl '")?;
                self.write(self.arena.text(kind))?;
                self.write("' for ")?;
                self.print(name)
            }
            Kind::Number | Kind::Integer => write_number(self.out, self.arena.node(id).value),
            Kind::ConstrainedExistential => {
                self.write("any ")?;
                self.print(self.arena.child(id, 0))?;
                self.write("<")?;
                self.print(self.arena.child(id, 1))?;
                self.write(">")
            }
            Kind::ConstrainedExistentialRequirementList => self.children(id, ", "),
            Kind::OpaqueType => {
                self.print(self.arena.child(id, 0))?;
                self.write(".")?;
                let index = self.arena.child(id, 1);
                write_number(self.out, self.arena.node(index).value)
            }
            Kind::OpaqueReturnType => self.write("some"),
            Kind::OpaqueReturnTypeOf => {
                self.write("<<opaque return type of ")?;
                self.children(id, "")?;
                self.write(">>")
            }
            Kind::OpaqueReturnTypeIndex => Ok(()),
            Kind::FreestandingMacroExpansion => {
                let context = self.arena.child(id, 0);
                let name = self.arena.child(id, 1);
                let discriminator = self.arena.child(id, 2);
                self.write("freestanding macro expansion #")?;
                write_number(
                    self.out,
                    self.arena.node(discriminator).value.saturating_add(1),
                )?;
                self.write(" of ")?;
                self.print(name)?;
                self.write(" in ")?;
                self.print(context)
            }
            Kind::MacroExpansionUniqueName => {
                let context = self.arena.child(id, 0);
                let name = self.arena.child(id, 1);
                let discriminator = self.arena.child(id, 2);
                self.write("unique name #")?;
                write_number(
                    self.out,
                    self.arena.node(discriminator).value.saturating_add(1),
                )?;
                self.write(" of ")?;
                self.print(name)?;
                self.write(" in ")?;
                self.print(context)
            }
            Kind::MacroExpansionLoc => {
                let module = self.arena.child(id, 0);
                let buffer = self.arena.child(id, 1);
                let line = self.arena.child(id, 2);
                let column = self.arena.child(id, 3);
                self.write("module ")?;
                self.print(module)?;
                self.write(" file ")?;
                self.print(buffer)?;
                self.write(" line ")?;
                write_number(self.out, self.arena.node(line).value)?;
                self.write(" column ")?;
                write_number(self.out, self.arena.node(column).value)
            }
            Kind::NegativeInteger => {
                self.write("-")?;
                write_number(self.out, self.arena.node(id).value)
            }
            Kind::BuiltinFixedArray => {
                self.write("Builtin.FixedArray<")?;
                self.print(self.arena.child(id, 0))?;
                self.write(", ")?;
                self.print(self.arena.child(id, 1))?;
                self.write(">")
            }
            Kind::BuiltinBorrow => {
                self.write("Builtin.Borrow<")?;
                self.print(self.arena.child(id, 0))?;
                self.write(">")
            }
            Kind::ProtocolConformance => {
                let ty = self.arena.child(id, 0);
                let protocol = self.arena.child(id, 1);
                let module = self.arena.child(id, 2);
                self.print(ty)?;
                self.write(" : ")?;
                self.print(protocol)?;
                self.write(" in ")?;
                self.print(module)
            }
            Kind::DependentAssociatedTypeRef => {
                if self.arena.child_count(id) > 1 {
                    let protocol = self.arena.child(id, 1);
                    self.print(protocol)?;
                    self.write(".")?;
                }
                let name = self.arena.child(id, 0);
                self.print(name)
            }
            Kind::AssocTypePath => self.children(id, "."),
            Kind::ProtocolConformanceRefInTypeModule => {
                self.write("protocol conformance ref (type's module) ")?;
                self.children(id, "")
            }
            Kind::ProtocolConformanceRefInProtocolModule => {
                self.write("protocol conformance ref (protocol's module) ")?;
                self.children(id, "")
            }
            Kind::ProtocolConformanceRefInOtherModule => {
                self.write("protocol conformance ref (retroactive) ")?;
                self.children(id, "")
            }
            Kind::ConcreteProtocolConformance => {
                self.write("concrete protocol conformance ")?;
                self.print(self.arena.child(id, 0))?;
                self.write(" to ")?;
                self.print(self.arena.child(id, 1))?;
                let conditionals = self.arena.child(id, 2);
                if conditionals != NIL && self.arena.child_count(conditionals) > 0 {
                    self.write(" with conditional requirements: ")?;
                    self.print(conditionals)?;
                }
                Ok(())
            }
            Kind::AnyProtocolConformanceList => {
                if self.arena.child_count(id) == 0 {
                    return Ok(());
                }
                self.write("(")?;
                self.children(id, ", ")?;
                self.write(")")
            }
            Kind::PackProtocolConformance => {
                self.write("pack protocol conformance ")?;
                self.children(id, "")
            }
            Kind::RetroactiveConformance => self.print(self.arena.child(id, 1)),
            Kind::DependentProtocolConformanceRoot => {
                self.write("dependent root protocol conformance ")?;
                self.print_dependent_index(id)?;
                self.print(self.arena.child(id, 0))?;
                self.write(" to ")?;
                self.print(self.arena.child(id, 1))
            }
            Kind::DependentProtocolConformanceInherited => {
                self.write("dependent inherited protocol conformance ")?;
                self.print_dependent_index(id)?;
                self.print(self.arena.child(id, 0))?;
                self.write(" to ")?;
                self.print(self.arena.child(id, 1))
            }
            Kind::DependentProtocolConformanceAssociated => {
                self.write("dependent associated protocol conformance ")?;
                self.print_dependent_index(id)?;
                self.print(self.arena.child(id, 0))?;
                self.write(" to ")?;
                self.print(self.arena.child(id, 1))
            }
            Kind::DependentProtocolConformanceOpaque => {
                self.write("opaque result conformance ")?;
                self.print(self.arena.child(id, 0))?;
                self.write(" of ")?;
                self.print(self.arena.child(id, 1))
            }
            Kind::DependentAssociatedConformance => {
                self.write("dependent associated conformance ")?;
                self.children(id, "")
            }
            Kind::InfixOperator | Kind::PrefixOperator | Kind::PostfixOperator => {
                self.write(self.arena.text(id))?;
                self.write(match kind {
                    Kind::InfixOperator => " infix",
                    Kind::PrefixOperator => " prefix",
                    _ => " postfix",
                })
            }
            Kind::Type | Kind::TypeMangling => {
                let child = self.arena.child(id, 0);
                self.print(child)
            }
            Kind::Class
            | Kind::Structure
            | Kind::Enum
            | Kind::Protocol
            | Kind::TypeAlias
            | Kind::OtherNominal => self.print_nominal(id),
            Kind::Extension => self.print_extension(id),
            Kind::Tuple => self.print_tuple(id),
            Kind::TupleElement => self.print_tuple_element(id),
            Kind::TupleElementName => self.write(self.arena.text(id)),
            Kind::FunctionType
            | Kind::NoEscapeFunctionType
            | Kind::ThinFunctionType
            | Kind::CFunctionPointer
            | Kind::ObjCBlock
            | Kind::EscapingObjCBlock
            | Kind::CalledOnceFunctionType
            | Kind::CalledAtMostOnceFunctionType
            | Kind::AutoClosureType
            | Kind::EscapingAutoClosureType => self.print_function_type(id),
            Kind::ArgumentTuple => self.print_argument_tuple(id, NIL),
            Kind::ReturnType => {
                let child = self.arena.child(id, 0);
                self.print(child)
            }
            Kind::Function => self.print_function(id),
            Kind::Variable => self.print_variable(id),
            Kind::Subscript => self.print_subscript(id),
            Kind::Initializer => {
                self.write("variable initialization expression of ")?;
                self.print(self.arena.child(id, 0))
            }
            Kind::Deinitializer => self.print_deinitializer(id),
            Kind::Allocator | Kind::Constructor => self.print_constructor(id, kind),
            Kind::PropertyWrapperBackingInitializer => {
                self.write("property wrapper backing initializer of ")?;
                self.print(self.arena.child(id, 0))
            }
            Kind::PropertyWrappedFieldInitAccessor => {
                self.write("property wrapped field init accessor of ")?;
                self.print(self.arena.child(id, 0))
            }
            Kind::PropertyWrapperInitFromProjectedValue => {
                self.write("property wrapper init from projected value of ")?;
                self.print(self.arena.child(id, 0))
            }
            Kind::GenericTypeParamDecl => {
                let context = self.arena.child(id, 0);
                let name = self.arena.child(id, 1);
                self.print(context)?;
                self.write(".")?;
                self.print(name)
            }
            Kind::Macro => self.print_macro(id),
            Kind::ExplicitClosure | Kind::ImplicitClosure => self.print_closure(id, kind),
            Kind::DefaultArgumentInitializer => self.print_default_argument(id),
            Kind::Accessor => self.print_accessor(id),
            Kind::Static => {
                self.write("static ")?;
                let child = self.arena.child(id, 0);
                self.print(child)
            }
            Kind::BoundGenericClass
            | Kind::BoundGenericStructure
            | Kind::BoundGenericEnum
            | Kind::BoundGenericProtocol
            | Kind::BoundGenericOtherNominal
            | Kind::BoundGenericFunction => self.print_bound_generic(id),
            Kind::TypeList => self.children(id, ", "),
            Kind::DependentGenericParamType => self.print_generic_param(id),
            Kind::ConstrainedExistentialSelf => self.write("Self"),
            Kind::DependentMemberType => {
                let base = self.arena.child(id, 0);
                let name = self.arena.child(id, 1);
                if base != NIL {
                    self.print(base)?;
                    self.write(".")?;
                }
                self.print(name)
            }
            Kind::DependentGenericType => {
                let signature = self.arena.child(id, 0);
                let ty = self.arena.child(id, 1);
                self.print(signature)?;
                self.write(" ")?;
                self.print(ty)
            }
            Kind::DependentGenericSignature => self.print_generic_signature(id),
            Kind::DependentGenericConformanceRequirement
            | Kind::DependentGenericSameTypeRequirement
            | Kind::DependentGenericBaseClassRequirement
            | Kind::DependentGenericLayoutRequirement
            | Kind::DependentGenericInverseConformanceRequirement => self.print_requirement(id),
            Kind::DependentGenericParamCount => Ok(()),
            Kind::Metatype | Kind::ExistentialMetatype => {
                let child = self.arena.child(id, 0);
                self.print(child)?;
                self.write(".Type")
            }
            Kind::InOut => self.print_prefixed(id, "inout "),
            Kind::Shared => self.print_prefixed(id, "__shared "),
            Kind::Owned => self.print_prefixed(id, "__owned "),
            Kind::NoDerivative => self.print_prefixed(id, "@noDerivative "),
            Kind::Sending => self.print_prefixed(id, "sending "),
            Kind::Isolated => self.print_prefixed(id, "isolated "),
            Kind::CompileTimeLiteral => self.print_prefixed(id, "_const "),
            Kind::ConstValue => self.print_prefixed(id, "@const "),
            Kind::ProtocolListType => {
                let list = self.arena.child(id, 0);
                if self.arena.child_count(list) == 0 {
                    self.write("Any")
                } else {
                    self.children(list, " & ")
                }
            }
            Kind::ProtocolList => self.children(id, " & "),
            Kind::Suffix => {
                self.write(" with unmangled suffix \"")?;
                self.write(self.arena.text(id))?;
                self.write("\"")
            }
            Kind::BuiltinVector => self.print_builtin_vector(id),
            Kind::ImplFunctionType => self.print_impl_function_type(id),
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
                if self.arena.node(id).flags == 1 {
                    self.write("@callable(once)")
                } else {
                    self.write("@once")
                }
            }
            Kind::ImplDifferentiabilityKind => {
                let text = match self.arena.node(id).flags {
                    b'f' => "@differentiable(_forward)",
                    b'r' => "@differentiable(reverse)",
                    b'l' => "@differentiable(_linear)",
                    _ => "@differentiable",
                };
                self.write(text)
            }
            Kind::ImplCalleeConvention => {
                let text = match self.arena.node(id).flags {
                    b'g' => "@callee_guaranteed",
                    b'x' => "@callee_owned",
                    b't' => "@convention(thin)",
                    _ => "@callee_unowned",
                };
                self.write(text)
            }
            Kind::ImplFunctionConvention => {
                let text = match self.arena.node(id).flags {
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
                let text = match self.arena.node(id).flags {
                    b'A' => "@yield_once",
                    b'I' => "@yield_once_2",
                    _ => "@yield_many",
                };
                self.write(text)
            }
            Kind::ImplSendable => self.write("@Sendable"),
            Kind::ImplAsync => self.write("@async"),
            Kind::ImplSendingResult => self.write("sending"),
            Kind::TopLevel => self.children(id, ""),
            Kind::Attribute => {
                let text = match self.arena.node(id).flags {
                    0 => "@objc ",
                    1 => "@nonobjc ",
                    2 => "dynamic ",
                    3 => "super ",
                    _ => "override ",
                };
                self.write(text)
            }
            Kind::GenericSpecialization => self.print_generic_specialization(id),
            Kind::FunctionSignatureSpecialization => self.print_function_specialization(id),
            Kind::FuncSpecParam => self.print_func_spec_param(id),
            Kind::FuncSpecKind | Kind::FuncSpecNumber => Ok(()),
            Kind::GenericSpecializationParam => {
                let child = self.arena.child(id, 0);
                self.print(child)
            }
            Kind::DroppedArgument => {
                self.write("param")?;
                write_number(self.out, self.arena.node(id).value)?;
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
                self.children(self.arena.child(id, 1), ", ")?;
                self.write(">")?;
                Ok(())
            }
            Kind::ImplInvocationSubstitutions => {
                self.write(" for <")?;
                self.children(self.arena.child(id, 0), ", ")?;
                self.write(">")?;
                Ok(())
            }
            Kind::SILBoxTypeWithLayout => {
                let layout = self.arena.child(id, 0);
                let signature = self.arena.child(id, 1);
                let args = self.arena.child(id, 2);
                if signature != NIL {
                    self.print(signature)?;
                    self.write(" ")?;
                }
                self.print(layout)?;
                if args != NIL {
                    self.write(" <")?;
                    self.children(args, ", ")?;
                    self.write(">")?;
                }
                Ok(())
            }
            Kind::SILBoxLayout => {
                self.write("{")?;
                let mut child = self.arena.node(id).first;
                let mut first = true;
                while child != NIL {
                    if !first {
                        self.write(",")?;
                    }
                    first = false;
                    self.write(" ")?;
                    self.print(child)?;
                    child = self.arena.node(child).next;
                }
                self.write(" }")
            }
            Kind::SILBoxMutableField | Kind::SILBoxImmutableField => {
                self.write(if kind == Kind::SILBoxMutableField {
                    "var "
                } else {
                    "let "
                })?;
                self.print(self.arena.child(id, 0))
            }
            Kind::IsSerialized => self.write("serialized"),
            Kind::AutoDiffFunctionKind => self.write(match self.arena.node(id).flags {
                b'f' => "forward-mode derivative",
                b'r' => "reverse-mode derivative",
                b'd' => "differential",
                _ => "pullback",
            }),
            Kind::IndexSubset => {
                self.write("{")?;
                let mut first = true;
                let text = self.arena.text(id);
                for (index, byte) in text.bytes().enumerate() {
                    if byte != b'S' {
                        continue;
                    }
                    if !first {
                        self.write(", ")?;
                    }
                    first = false;
                    write_number(self.out, u32::try_from(index).unwrap_or(0))?;
                }
                self.write("}")
            }
            Kind::VTableThunk => {
                self.write("vtable thunk for ")?;
                let derived = self.arena.child(id, 0);
                let base = self.arena.child(id, 1);
                self.print(derived)?;
                self.write(" dispatching to ")?;
                self.print(base)
            }
            Kind::CoroutineContinuationPrototype => {
                self.write("coroutine continuation prototype for ")?;
                let ty = self.arena.child(id, 0);
                self.print(ty)
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
            | Kind::LabelList
            | Kind::ValueWitness => Ok(()),
        }
    }

    fn print_builtin_vector(&mut self, id: NodeId) -> fmt::Result {
        self.write(self.arena.text(id))?;
        self.write("x")?;
        let element = self.arena.child(id, 0);
        if element == NIL {
            return Ok(());
        }
        let mut inner = element;
        if self.arena.kind(inner) == Kind::Type {
            inner = self.arena.child(inner, 0);
        }
        let text = self.arena.text(inner);
        let name = text.strip_prefix("Builtin.").unwrap_or(text);
        self.write(name)
    }

    fn print_prefixed(&mut self, id: NodeId, prefix: &str) -> fmt::Result {
        let child = self.arena.child(id, 0);
        self.write(prefix)?;
        self.print(child)
    }

    fn print_nominal(&mut self, id: NodeId) -> fmt::Result {
        let context = self.arena.child(id, 0);
        let name = self.arena.child(id, 1);
        if name == NIL {
            return Ok(());
        }
        let name_kind = self.arena.kind(name);
        let multi_word = name_kind == Kind::LocalDeclName || self.context_prints_type(context);
        if context != NIL && !multi_word {
            self.print(context)?;
            self.write(".")?;
        }
        self.print(name)?;
        if context != NIL && multi_word {
            self.write(" in ")?;
            self.print(context)?;
        }
        Ok(())
    }

    fn context_prints_type(&self, context: NodeId) -> bool {
        if context == NIL {
            return false;
        }
        matches!(
            self.arena.kind(context),
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

    fn print_extension(&mut self, id: NodeId) -> fmt::Result {
        let module = self.arena.child(id, 0);
        let ty = self.arena.child(id, 1);
        self.write("(extension in ")?;
        self.print(module)?;
        self.write("):")?;
        self.print(ty)?;
        let signature = self.arena.child(id, 2);
        if signature != NIL {
            self.print(signature)?;
        }
        Ok(())
    }

    fn print_generic_signature(&mut self, id: NodeId) -> fmt::Result {
        let mut counts = [(0_u32, 0_u32); 8];
        let mut depths = 0_usize;
        let mut requirements = [NIL; 16];
        let mut requirement_count = 0_usize;
        let mut child = self.arena.node(id).first;
        while child != NIL {
            match self.arena.kind(child) {
                Kind::DependentGenericParamCount => {
                    if depths < counts.len() {
                        counts[depths] = (
                            u32::try_from(depths).unwrap_or(0),
                            self.arena.node(child).value,
                        );
                        depths += 1;
                    }
                }
                _ => {
                    if requirement_count < requirements.len() {
                        requirements[requirement_count] = child;
                        requirement_count += 1;
                    }
                }
            }
            child = self.arena.node(child).next;
        }
        self.write("<")?;
        let mut first = true;
        for (position, &(depth, count)) in counts.iter().take(depths).enumerate() {
            if position > 0 {
                self.write("><")?;
                first = true;
            }
            for index in 0..count {
                if !first {
                    self.write(", ")?;
                }
                first = false;
                generic_param_name(self.out, depth, index)?;
            }
        }
        if requirement_count > 0 {
            self.write(" where ")?;
            let mut first = true;
            for &requirement in requirements.iter().take(requirement_count) {
                if !first {
                    self.write(", ")?;
                }
                first = false;
                self.print_requirement(requirement)?;
            }
        }
        self.write(">")
    }

    fn print_requirement(&mut self, id: NodeId) -> fmt::Result {
        let kind = self.arena.kind(id);
        let subject = self.arena.child(id, 0);
        let other = self.arena.child(id, 1);
        self.print(subject)?;
        match kind {
            Kind::DependentGenericSameTypeRequirement => {
                self.write(" == ")?;
                self.print(other)
            }
            Kind::DependentGenericInverseConformanceRequirement => {
                let index = self.arena.child(id, 1);
                self.write(": ~")?;
                match self.arena.node(index).value {
                    0 => self.write("Swift.Copyable"),
                    1 => self.write("Swift.Escapable"),
                    2 => self.write("Swift.Deinitable"),
                    value => {
                        self.write("Swift.<bit ")?;
                        write_number(self.out, value)?;
                        self.write(">")
                    }
                }
            }
            Kind::DependentGenericLayoutRequirement => {
                let layout = self.arena.node(id).flags;
                let name = match layout {
                    b'N' | b'R' => "AnyObject",
                    b'C' | b'D' => "AnyObject",
                    b'T' => "Trivial",
                    b'U' => "UnknownLayout",
                    b'B' => "BridgeObject",
                    b'S' => "TrivialStride",
                    _ => "AnyObject",
                };
                self.write(": ")?;
                self.write(name)
            }
            _ => {
                self.write(": ")?;
                if other == NIL {
                    self.write("AnyObject")
                } else {
                    self.print(other)
                }
            }
        }
    }

    fn print_generic_param(&mut self, id: NodeId) -> fmt::Result {
        let node = self.arena.node(id);
        let index = node.len as u32;
        let depth = node.value;
        generic_param_name(self.out, depth, index)
    }

    fn print_tuple(&mut self, id: NodeId) -> fmt::Result {
        self.write("(")?;
        self.children(id, ", ")?;
        self.write(")")
    }

    fn print_tuple_element(&mut self, id: NodeId) -> fmt::Result {
        let mut child = self.arena.node(id).first;
        while child != NIL {
            let kind = self.arena.kind(child);
            match kind {
                Kind::VariadicMarker => {}
                Kind::TupleElementName => {
                    self.print(child)?;
                    self.write(": ")?;
                }
                _ => {
                    self.print(child)?;
                }
            }
            child = self.arena.node(child).next;
        }
        // A variadic marker may be the first child.
        let first = self.arena.child(id, 0);
        if first != NIL && self.arena.kind(first) == Kind::VariadicMarker {
            self.write("...")?;
        }
        Ok(())
    }

    fn find_child(&self, parent: NodeId, kind: Kind) -> Option<NodeId> {
        let mut child = self.arena.node(parent).first;
        let mut guard = 0_usize;
        while child != NIL {
            guard += 1;
            if guard > crate::swift::node::NODE_CAP {
                return None;
            }
            if self.arena.kind(child) == kind {
                return Some(child);
            }
            child = self.arena.node(child).next;
        }
        None
    }

    fn find_child_of(&self, parent: NodeId, kinds: &[Kind]) -> Option<NodeId> {
        let mut child = self.arena.node(parent).first;
        let mut guard = 0_usize;
        while child != NIL {
            guard += 1;
            if guard > crate::swift::node::NODE_CAP {
                return None;
            }
            if kinds.contains(&self.arena.kind(child)) {
                return Some(child);
            }
            child = self.arena.node(child).next;
        }
        None
    }

    fn print_argument_tuple(&mut self, id: NodeId, labels: NodeId) -> fmt::Result {
        let child = self.arena.child(id, 0);
        let mut inner = child;
        if inner != NIL && self.arena.kind(inner) == Kind::Type {
            inner = self.arena.child(inner, 0);
        }
        if inner != NIL && self.arena.kind(inner) == Kind::Tuple {
            self.write("(")?;
            let mut index = 0_usize;
            let mut element = self.arena.node(inner).first;
            let has_labels = labels != NIL && self.arena.child_count(labels) > 0;
            while element != NIL {
                if index > 0 {
                    self.write(", ")?;
                }
                if has_labels {
                    let label = self.arena.child(labels, index);
                    match self.arena.kind(label) {
                        Kind::Identifier => {
                            self.write(self.arena.text(label))?;
                            self.write(": ")?;
                        }
                        Kind::FirstElementMarker => {
                            self.write("_: ")?;
                        }
                        _ => {}
                    }
                }
                self.print(element)?;
                index += 1;
                element = self.arena.node(element).next;
            }
            return self.write(")");
        }
        self.write("(")?;
        self.print(child)?;
        self.write(")")
    }

    fn print_function_type(&mut self, id: NodeId) -> fmt::Result {
        let kind = self.arena.kind(id);
        match kind {
            Kind::ThinFunctionType => self.write("@convention(thin) ")?,
            Kind::CFunctionPointer => self.write("@convention(c) ")?,
            Kind::ObjCBlock => self.write("@convention(block) ")?,
            Kind::EscapingObjCBlock => self.write("@escaping @convention(block) ")?,
            Kind::CalledOnceFunctionType => self.write("@once ")?,
            Kind::CalledAtMostOnceFunctionType => self.write("@callable(once) ")?,
            Kind::AutoClosureType => self.write("@autoclosure ")?,
            Kind::EscapingAutoClosureType => self.write("@autoclosure @escaping ")?,
            _ => {}
        }
        self.print_signature(id, NIL)
    }

    /// Print the signature part of a function type: parameters, effects, result.
    fn print_signature(&mut self, func: NodeId, labels: NodeId) -> fmt::Result {
        let isolated = self.find_child_of(
            func,
            &[
                Kind::IsolatedAnyFunctionType,
                Kind::NonIsolatedCallerFunctionType,
                Kind::GlobalActorFunctionType,
            ],
        );
        let differentiable = self.find_child(func, Kind::DifferentiableFunctionType);
        let sendable = self.find_child(func, Kind::ConcurrentFunctionType);
        let sending_result = self.find_child(func, Kind::SendingResultFunctionType);
        let args = self.find_child(func, Kind::ArgumentTuple);
        let ret = self.find_child(func, Kind::ReturnType);
        let throws =
            self.find_child_of(func, &[Kind::ThrowsAnnotation, Kind::TypedThrowsAnnotation]);
        let is_async = self.find_child(func, Kind::AsyncAnnotation);

        if let Some(node) = isolated {
            match self.arena.kind(node) {
                Kind::IsolatedAnyFunctionType => self.write("@isolated(any) ")?,
                Kind::NonIsolatedCallerFunctionType => self.write("nonisolated(nonsending) ")?,
                Kind::GlobalActorFunctionType => {
                    let actor = self.arena.child(node, 0);
                    self.write("@")?;
                    self.print(actor)?;
                    self.write(" ")?;
                }
                _ => {}
            }
        }
        if let Some(node) = differentiable {
            let flag = self.arena.node(node).flags;
            match flag {
                b'f' => self.write("@differentiable(_forward) ")?,
                b'r' => self.write("@differentiable(reverse) ")?,
                b'l' => self.write("@differentiable(_linear) ")?,
                _ => self.write("@differentiable ")?,
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
            if self.arena.kind(node) == Kind::TypedThrowsAnnotation {
                let ty = self.arena.child(node, 0);
                self.write(" throws(")?;
                self.print(ty)?;
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
            Some(ret) => self.print(ret),
            None => self.write("()"),
        }
    }

    fn function_node(&self, id: NodeId) -> Option<NodeId> {
        let mut ty = NIL;
        let mut child = self.arena.node(id).first;
        while child != NIL {
            if self.arena.kind(child) == Kind::Type {
                ty = child;
            }
            child = self.arena.node(child).next;
        }
        if ty == NIL {
            return None;
        }
        let mut func = self.arena.child(ty, 0);
        while func != NIL && self.arena.kind(func) != Kind::FunctionType {
            let next = if self.arena.kind(func) == Kind::DependentGenericType {
                self.arena.child(func, 1)
            } else {
                self.arena.child(func, 0)
            };
            if next == func || next == NIL {
                break;
            }
            func = next;
        }
        (func != NIL && self.arena.kind(func) == Kind::FunctionType).then_some(func)
    }

    fn print_function(&mut self, id: NodeId) -> fmt::Result {
        let context = self.arena.child(id, 0);
        let name = self.arena.child(id, 1);
        if context != NIL {
            self.print(context)?;
            self.write(".")?;
        }
        self.print(name)?;
        if let Some(signature) = self.generic_signature_of(id) {
            self.print_generic_signature(signature)?;
        }
        let labels = self.find_child(id, Kind::LabelList).unwrap_or(NIL);
        match self.function_node(id) {
            Some(func) => self.print_signature(func, labels),
            None => self.write("() -> ()"),
        }
    }

    /// Generic signature attached to an entity's type, if any.
    fn generic_signature_of(&self, id: NodeId) -> Option<NodeId> {
        let mut child = self.arena.node(id).first;
        while child != NIL {
            if self.arena.kind(child) == Kind::Type {
                let inner = self.arena.child(child, 0);
                if inner != NIL && self.arena.kind(inner) == Kind::DependentGenericType {
                    return Some(self.arena.child(inner, 0));
                }
            }
            child = self.arena.node(child).next;
        }
        None
    }

    fn print_variable(&mut self, id: NodeId) -> fmt::Result {
        let context = self.arena.child(id, 0);
        let name = self.arena.child(id, 1);
        let multi_word = matches!(
            self.arena.kind(name),
            Kind::LocalDeclName | Kind::PrivateDeclName | Kind::RelatedEntityDeclName
        );
        if context != NIL && !multi_word {
            self.print(context)?;
            self.write(".")?;
        }
        self.print(name)?;
        let flags = self.arena.node(id).flags;
        if let Some(code) = Accessor::from_code(flags) {
            self.write(accessor_name(code))?;
        }
        let ty = self.arena.child(id, 2);
        if ty != NIL {
            self.write(" : ")?;
            self.print(ty)?;
        }
        if context != NIL && multi_word {
            self.write(" in ")?;
            self.print(context)?;
        }
        Ok(())
    }

    fn print_subscript(&mut self, id: NodeId) -> fmt::Result {
        let context = self.arena.child(id, 0);
        if context != NIL {
            self.print(context)?;
            self.write(".")?;
        }
        self.write("subscript")?;
        if let Some(code) = Accessor::from_code(self.arena.node(id).flags) {
            self.write(accessor_name(code))?;
        }
        let ty = self.arena.child(id, 1);
        if ty != NIL {
            self.write(" : ")?;
            self.print(ty)?;
        }
        Ok(())
    }

    fn print_deinitializer(&mut self, id: NodeId) -> fmt::Result {
        let context = self.arena.child(id, 0);
        self.print(context)?;
        if self.arena.node(id).flags == 1 {
            self.write(".__deallocating_deinit")
        } else {
            self.write(".deinit")
        }
    }

    fn print_constructor(&mut self, id: NodeId, kind: Kind) -> fmt::Result {
        let context = self.arena.child(id, 0);
        let labels = self.arena.child(id, 1);
        let private = self.find_child(id, Kind::PrivateDeclName).unwrap_or(NIL);
        self.print(context)?;
        self.write(".")?;
        if private != NIL {
            self.print(private)?;
            self.write(".")?;
        }
        let is_class = matches!(
            self.arena.kind(context),
            Kind::Class | Kind::BoundGenericClass
        );
        if kind == Kind::Allocator && is_class {
            self.write("__allocating_init")?;
        } else {
            self.write("init")?;
        }
        if let Some(signature) = self.generic_signature_of(id) {
            self.print_generic_signature(signature)?;
        }
        match self.function_node(id) {
            Some(func) => self.print_signature(func, labels),
            None => self.write("() -> ()"),
        }
    }

    fn print_macro(&mut self, id: NodeId) -> fmt::Result {
        let context = self.arena.child(id, 0);
        let name = self.arena.child(id, 1);
        let labels = self.arena.child(id, 2);
        self.print(context)?;
        self.write(".")?;
        self.print(name)?;
        if let Some(func) = self.function_node(id) {
            return self.print_signature(func, labels);
        }
        let mut ty = NIL;
        let mut child = self.arena.node(id).first;
        while child != NIL {
            if self.arena.kind(child) == Kind::Type {
                ty = child;
            }
            child = self.arena.node(child).next;
        }
        if ty != NIL {
            self.write(" : ")?;
            self.print(ty)?;
        }
        Ok(())
    }

    fn print_closure(&mut self, id: NodeId, kind: Kind) -> fmt::Result {
        let context = self.arena.child(id, 0);
        let ty = self.arena.child(id, 1);
        let index = self.arena.node(id).value;
        if kind == Kind::ImplicitClosure {
            self.write("implicit closure #")?;
        } else {
            self.write("closure #")?;
        }
        write_number(self.out, index.saturating_add(1))?;
        self.write(" ")?;
        if ty != NIL {
            self.print(ty)?;
        }
        self.write(" in ")?;
        if context != NIL {
            self.print(context)?;
        }
        Ok(())
    }

    fn print_default_argument(&mut self, id: NodeId) -> fmt::Result {
        let context = self.arena.child(id, 0);
        let index = self.arena.node(id).value;
        self.write("default argument #")?;
        write_number(self.out, index)?;
        self.write(" of ")?;
        self.print(context)
    }

    fn print_accessor(&mut self, id: NodeId) -> fmt::Result {
        let code = Accessor::from_code(self.arena.node(id).flags);
        let inner = self.arena.child(id, 0);
        if let Some(code) = code {
            self.write(accessor_name(code))?;
        }
        self.print(inner)
    }

    fn print_bound_generic(&mut self, id: NodeId) -> fmt::Result {
        let nominal = self.arena.child(id, 0);
        let args = self.arena.child(id, 1);
        if self.arena.child_count(args) == 0 {
            return self.print(nominal);
        }
        if self.arena.kind(id) == Kind::BoundGenericProtocol {
            self.children(args, ", ")?;
            self.write(" as ")?;
            return self.print(nominal);
        }
        if self.is_sugar(nominal, "Optional") && self.arena.child_count(args) == 1 {
            let arg = self.arena.child(args, 0);
            self.print_with_parens(arg)?;
            return self.write("?");
        }
        if self.is_sugar(nominal, "Array") && self.arena.child_count(args) == 1 {
            let arg = self.arena.child(args, 0);
            self.write("[")?;
            self.print(arg)?;
            return self.write("]");
        }
        if self.is_sugar(nominal, "Dictionary") && self.arena.child_count(args) == 2 {
            let key = self.arena.child(args, 0);
            let value = self.arena.child(args, 1);
            self.write("[")?;
            self.print(key)?;
            self.write(" : ")?;
            self.print(value)?;
            return self.write("]");
        }
        self.print(nominal)?;
        self.write("<")?;
        self.children(args, ", ")?;
        self.write(">")
    }

    fn print_with_parens(&mut self, id: NodeId) -> fmt::Result {
        if self.is_simple_type(id) {
            return self.print(id);
        }
        self.write("(")?;
        self.print(id)?;
        self.write(")")
    }

    fn is_simple_type(&self, id: NodeId) -> bool {
        let mut node = id;
        if self.arena.kind(node) == Kind::Type {
            node = self.arena.child(node, 0);
        }
        matches!(
            self.arena.kind(node),
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
                | Kind::ProtocolListType
        )
    }

    fn is_sugar(&self, nominal: NodeId, name: &str) -> bool {
        if nominal == NIL {
            return false;
        }
        let module = self.arena.child(nominal, 0);
        let ident = self.arena.child(nominal, 1);
        if module == NIL || ident == NIL {
            return false;
        }
        if self.arena.kind(module) != Kind::Module || self.arena.kind(ident) != Kind::Identifier {
            return false;
        }
        self.arena.text(module) == "Swift" && self.arena.text(ident) == name
    }

    fn print_function_specialization(&mut self, id: NodeId) -> fmt::Result {
        if self.arena.node(id).flags == 1 {
            return self.write("representation changed of ");
        }
        self.write("function signature specialization <")?;
        let mut first = true;
        let mut argument = 0_u32;
        let mut child = self.arena.node(id).first;
        while child != NIL {
            if self.arena.kind(child) == Kind::FuncSpecParam && self.arena.node(child).first != NIL
            {
                if !first {
                    self.write(", ")?;
                }
                first = false;
                if self.arena.node(child).flags == 1 {
                    self.write("Return = ")?;
                } else {
                    self.write("Arg[")?;
                    write_number(self.out, argument)?;
                    self.write("] = ")?;
                }
                self.print_func_spec_param(child)?;
            }
            argument = argument.saturating_add(1);
            child = self.arena.node(child).next;
        }
        self.write("> of ")
    }

    fn print_func_spec_param(&mut self, id: NodeId) -> fmt::Result {
        let mut child = self.arena.node(id).first;
        while child != NIL {
            if self.arena.kind(child) != Kind::FuncSpecKind {
                child = self.arena.node(child).next;
                continue;
            }
            let raw = self.arena.node(child).value;
            if raw >= 1 << 6 {
                self.write(option_set_text(raw))?;
                child = self.arena.node(child).next;
                continue;
            }
            self.write("[")?;
            self.write(func_spec_kind_text(raw))?;
            let next = self.arena.node(child).next;
            let mut close = true;
            match raw {
                0 | 1 | 2 | 3 | 10 => {
                    if next != NIL {
                        self.write(" : ")?;
                        self.print(next)?;
                    }
                    child = if next == NIL {
                        next
                    } else {
                        self.arena.node(next).next
                    };
                }
                4 => {
                    if next != NIL {
                        self.write(" : ")?;
                        self.write(self.arena.text(next))?;
                        let string = self.arena.node(next).next;
                        if string != NIL {
                            self.write("'")?;
                            let text = self.arena.text(string);
                            self.write(text.strip_prefix('_').unwrap_or(text))?;
                            self.write("'")?;
                        }
                    }
                    child = NIL;
                }
                5 | 12 => {
                    if next != NIL {
                        self.write(" : ")?;
                        self.print(next)?;
                    }
                    self.write(", Argument Types : [")?;
                    let mut ty = if next == NIL {
                        NIL
                    } else {
                        self.arena.node(next).next
                    };
                    let mut first = true;
                    while ty != NIL {
                        if !first {
                            self.write(", ")?;
                        }
                        first = false;
                        self.print(ty)?;
                        ty = self.arena.node(ty).next;
                    }
                    self.write("]")?;
                    close = false;
                    child = ty;
                }
                9 => {
                    self.write(" : ")?;
                    let mut node = next;
                    for label in ["", "<", ",", ">"] {
                        if !label.is_empty() {
                            self.write(label)?;
                        }
                        if node == NIL {
                            break;
                        }
                        self.print(node)?;
                        node = self.arena.node(node).next;
                    }
                    child = node;
                }
                13 => {
                    self.write(", Components : [")?;
                    let mut node = next;
                    for pair in 0..3 {
                        if pair > 0 {
                            self.write(", ")?;
                        }
                        if node == NIL {
                            break;
                        }
                        self.print(node)?;
                        self.write(" : ")?;
                        node = self.arena.node(node).next;
                        if node != NIL {
                            self.print(node)?;
                            node = self.arena.node(node).next;
                        }
                    }
                    self.write("]]")?;
                    close = false;
                    child = node;
                }
                11 => {
                    self.write(" ")?;
                    if next != NIL {
                        if self.arena.kind(next) == Kind::FuncSpecNumber {
                            write_number(self.out, self.arena.node(next).value)?;
                        } else {
                            self.print(next)?;
                        }
                    }
                    child = if next == NIL {
                        next
                    } else {
                        self.arena.node(next).next
                    };
                }
                _ => {
                    child = next;
                }
            }
            if close {
                self.write("]")?;
            }
        }
        Ok(())
    }

    fn print_generic_specialization(&mut self, id: NodeId) -> fmt::Result {
        let description = match self.arena.node(id).flags {
            b'G' => "generic not re-abstracted specialization",
            b'B' => "generic specialization",
            b's' => "generic pre-specialization",
            b'i' => "inlined generic function",
            b't' => "generic specialization",
            _ => "generic specialization",
        };
        self.write(description)?;
        self.write(" <")?;
        let mut first = true;
        let mut child = self.arena.node(id).first;
        while child != NIL {
            if self.arena.kind(child) == Kind::DroppedArgument {
                child = self.arena.node(child).next;
                continue;
            }
            if !first {
                self.write(", ")?;
            }
            first = false;
            self.print(child)?;
            child = self.arena.node(child).next;
        }
        self.write("> of ")
    }

    fn print_impl_function_type(&mut self, id: NodeId) -> fmt::Result {
        let mut first_attr = true;
        let mut child = self.arena.node(id).first;
        let mut sending_result = false;
        // Attributes come first and are space separated.
        while child != NIL {
            let kind = self.arena.kind(child);
            if matches!(
                kind,
                Kind::ImplParameter | Kind::ImplResult | Kind::ImplYield | Kind::ImplErrorResult
            ) || kind == Kind::DependentGenericSignature
            {
                break;
            }
            if kind == Kind::ImplSendingResult {
                sending_result = true;
                child = self.arena.node(child).next;
                continue;
            }
            if !first_attr {
                self.write(" ")?;
            }
            first_attr = false;
            self.print(child)?;
            child = self.arena.node(child).next;
        }
        if !first_attr {
            self.write(" ")?;
        }
        self.write("(")?;
        let mut first = true;
        while child != NIL {
            if self.arena.kind(child) == Kind::ImplParameter {
                if !first {
                    self.write(", ")?;
                }
                first = false;
                self.print(child)?;
            }
            child = self.arena.node(child).next;
        }
        self.write(") -> ")?;
        if sending_result {
            self.write("sending ")?;
        }
        self.write("(")?;
        child = self.arena.node(id).first;
        let mut first = true;
        while child != NIL {
            if matches!(
                self.arena.kind(child),
                Kind::ImplResult | Kind::ImplYield | Kind::ImplErrorResult
            ) {
                if !first {
                    self.write(", ")?;
                }
                first = false;
                self.print(child)?;
            }
            child = self.arena.node(child).next;
        }
        self.write(")")?;
        let mut child = self.arena.node(id).first;
        while child != NIL {
            let next = self.arena.node(child).next;
            if matches!(
                self.arena.kind(child),
                Kind::ImplPatternSubstitutions | Kind::ImplInvocationSubstitutions
            ) {
                self.print(child)?;
            }
            child = next;
        }
        Ok(())
    }

    fn print_impl_entry(&mut self, id: NodeId) -> fmt::Result {
        if self.arena.kind(id) == Kind::ImplErrorResult {
            self.write("@error ")?;
        } else if self.arena.kind(id) == Kind::ImplYield {
            self.write("@yields ")?;
        }
        let convention = self.arena.child(id, 0);
        self.print(convention)?;
        self.write(" ")?;
        let mut child = self.arena.node(id).first;
        while child != NIL {
            let next = self.arena.node(child).next;
            if child != convention
                && !matches!(
                    self.arena.kind(child),
                    Kind::ImplIsolated | Kind::ImplImplicitLeading
                )
            {
                self.print(child)?;
            }
            child = next;
        }
        Ok(())
    }

    fn print_impl_convention(&mut self, id: NodeId) -> fmt::Result {
        let text = match self.arena.node(id).flags {
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

    fn print_keypath_thunk(&mut self, id: NodeId) -> fmt::Result {
        let flags = self.arena.node(id).flags;
        if flags & 2 != 0 {
            self.write("key path unapplied method ")?;
        } else if flags & 4 != 0 {
            self.write("key path applied method ")?;
        } else if flags & 1 != 0 {
            self.write("key path setter for ")?;
        } else {
            self.write("key path getter for ")?;
        }
        self.print(self.arena.child(id, 0))?;
        self.write(" : ")?;
        let count = self.arena.child_count(id);
        for index in 1..count {
            let child = self.arena.child(id, index);
            if self.arena.kind(child) == Kind::IsSerialized {
                self.write(", ")?;
            }
            self.print(child)?;
        }
        Ok(())
    }

    fn print_dependent_index(&mut self, id: NodeId) -> fmt::Result {
        let index = self.arena.child(id, 2);
        if index != NIL {
            self.write("#")?;
            write_number(self.out, self.arena.node(index).value)?;
            self.write(" ")?;
        }
        Ok(())
    }

    fn print_auto_diff_function(&mut self, id: NodeId, kind: Kind) -> fmt::Result {
        let mut prefix_end = 0_usize;
        while prefix_end < self.arena.child_count(id)
            && self.arena.kind(self.arena.child(id, prefix_end)) != Kind::AutoDiffFunctionKind
        {
            prefix_end += 1;
        }
        if prefix_end >= self.arena.child_count(id) {
            return Err(fmt::Error);
        }
        if kind == Kind::AutoDiffDerivativeVTableThunk {
            self.write("vtable thunk for ")?;
        }
        self.print(self.arena.child(id, prefix_end))?;
        self.write(" of ")?;
        let mut signature = NIL;
        for index in 0..prefix_end {
            let child = self.arena.child(id, index);
            if index + 1 == prefix_end && self.arena.kind(child) == Kind::DependentGenericSignature
            {
                signature = child;
                break;
            }
            self.print(child)?;
        }
        self.write(" with respect to parameters ")?;
        self.print(self.arena.child(id, prefix_end + 1))?;
        self.write(" and results ")?;
        self.print(self.arena.child(id, prefix_end + 2))?;
        if signature != NIL {
            self.write(" with ")?;
            self.print(signature)?;
        }
        Ok(())
    }

    fn print_auto_diff_self_reordering(&mut self, id: NodeId) -> fmt::Result {
        let count = self.arena.child_count(id);
        if count < 3 {
            return Err(fmt::Error);
        }
        let from = self.arena.child(id, 0);
        let to = self.arena.child(id, 1);
        let mut index = 2;
        let mut signature = NIL;
        if self.arena.kind(self.arena.child(id, index)) == Kind::DependentGenericSignature {
            signature = self.arena.child(id, index);
            index += 1;
        }
        self.write("autodiff self-reordering reabstraction thunk for ")?;
        self.print(self.arena.child(id, index))?;
        if signature != NIL {
            self.print(signature)?;
            self.write(" ")?;
        }
        self.write(" from ")?;
        self.print(from)?;
        self.write(" to ")?;
        self.print(to)
    }

    fn print_auto_diff_subset(&mut self, id: NodeId) -> fmt::Result {
        let count = self.arena.child_count(id);
        if count < 5 {
            return Err(fmt::Error);
        }
        let to_params = self.arena.child(id, count - 1);
        let results = self.arena.child(id, count - 2);
        let params = self.arena.child(id, count - 3);
        let kind = self.arena.child(id, count - 4);
        let prefix_end = count - 4;
        self.write("autodiff subset parameters thunk for ")?;
        self.print(kind)?;
        self.write(" from ")?;
        if prefix_end == 0 {
            return Err(fmt::Error);
        }
        if prefix_end == 1 {
            self.print(self.arena.child(id, 0))?;
        } else {
            for index in 0..prefix_end {
                self.print(self.arena.child(id, index))?;
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
            self.print(self.arena.child(id, prefix_end - 1))?;
        }
        Ok(())
    }

    fn print_differentiability_witness(&mut self, id: NodeId) -> fmt::Result {
        let count = self.arena.child_count(id);
        let has_signature = count > 0
            && self.arena.kind(self.arena.child(id, count - 1)) == Kind::DependentGenericSignature;
        let kind_index = count.saturating_sub(if has_signature { 4 } else { 3 });
        let kind_node = self.arena.child(id, kind_index);
        self.write(match self.arena.node(kind_node).value {
            value if value == u32::from(b'f') => "forward-mode",
            value if value == u32::from(b'r') => "reverse-mode",
            value if value == u32::from(b'd') => "normal",
            _ => "linear",
        })?;
        self.write(" differentiability witness for ")?;
        for index in 0..kind_index {
            self.print(self.arena.child(id, index))?;
        }
        self.write(" with respect to parameters ")?;
        self.print(self.arena.child(id, kind_index + 1))?;
        self.write(" and results ")?;
        self.print(self.arena.child(id, kind_index + 2))?;
        if has_signature {
            self.write(" with ")?;
            self.print(self.arena.child(id, count - 1))?;
        }
        Ok(())
    }

    fn print_global(&mut self, id: NodeId) -> fmt::Result {
        let code = Global::from_code(self.arena.node(id).flags);
        let Some(code) = code else {
            return Err(fmt::Error);
        };
        let child = self.arena.child(id, 0);
        let index = self.arena.node(id).value;
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
            Global::TypeMetadataCompletionFunction => {
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
            Global::GenericTypeCompletionFunction => {
                self.with_child("generic type completion function for ", child)
            }
            Global::ModuleDescriptor => self.with_child("module descriptor for ", child),
            Global::ExtensionDescriptor => self.with_child("extension descriptor for ", child),
            Global::AnonymousContextDescriptor => {
                self.with_child("anonymous context descriptor for ", child)
            }
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
            Global::FieldOffset => self.with_child("field offset for ", child),
            Global::EnumTagIndex => self.with_child("resilient enum tag index for ", child),
            Global::LazyProtocolWitnessTableCache => {
                self.write("lazy protocol witness table cache variable for type ")?;
                self.print(child)?;
                self.write(" and conformance ")?;
                let conformance = self.arena.child(id, 1);
                self.print(conformance)
            }
            Global::LazyProtocolWitnessTableAccessor => {
                self.write("lazy protocol witness table accessor for type ")?;
                self.print(child)?;
                self.write(" and conformance ")?;
                let conformance = self.arena.child(id, 1);
                self.print(conformance)
            }
            Global::AssociatedTypeMetadataAccessor => {
                let name = self.arena.child(id, 1);
                self.write("associated type metadata accessor for ")?;
                self.print(name)?;
                self.write(" in ")?;
                self.print(child)
            }
            Global::AssociatedTypeWitnessTableAccessor => {
                let associated = self.arena.child(id, 1);
                let protocol = self.arena.child(id, 2);
                self.write("associated type witness table accessor for ")?;
                self.print(associated)?;
                self.write(" : ")?;
                self.print(protocol)?;
                self.write(" in ")?;
                self.print(child)
            }
            Global::BaseProtocolWitnessTableAccessor => {
                let protocol = self.arena.child(id, 1);
                self.write("base witness table accessor for ")?;
                self.print(protocol)?;
                self.write(" in ")?;
                self.print(child)
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
                self.write(self.arena.text(id))?;
                self.write(") of ")?;
                self.print(child)
            }
            Global::OutlinedVariable => {
                self.write("outlined variable #")?;
                write_number(self.out, index)?;
                self.write(" of ")?;
                self.print(child)
            }
            Global::OutlinedReadOnlyObject => {
                self.write("outlined read-only object #")?;
                write_number(self.out, index)?;
                self.write(" of ")?;
                self.print(child)
            }
            Global::PartialApplyForwarder => {
                self.write("partial apply forwarder")?;
                if self.arena.child_count(id) > 0 {
                    self.write(" for ")?;
                    self.children(id, "")?;
                }
                Ok(())
            }
            Global::PartialApplyObjcForwarder => {
                self.write("partial apply ObjC forwarder")?;
                if self.arena.child_count(id) > 0 {
                    self.write(" for ")?;
                    self.children(id, "")?;
                }
                Ok(())
            }
            Global::ReabstractionThunk | Global::ReabstractionThunkHelper => {
                self.write("reabstraction thunk ")?;
                if code == Global::ReabstractionThunkHelper {
                    self.write("helper ")?;
                }
                let count = self.arena.child_count(id);
                let index = if count == 3 {
                    let signature = self.arena.child(id, 0);
                    self.print(signature)?;
                    self.write(" ")?;
                    1
                } else {
                    0
                };
                self.write("from ")?;
                self.print(self.arena.child(id, index + 1))?;
                self.write(" to ")?;
                self.print(self.arena.child(id, index))
            }
            Global::DistributedThunk => self.write("distributed thunk "),
            Global::DistributedAccessor => self.write("distributed accessor for "),
            Global::DynamicallyReplaceableFunctionVar => {
                self.write("dynamically replaceable function variable for ")
            }
            Global::DynamicallyReplaceableFunctionKey => {
                self.write("dynamically replaceable function key for ")
            }
            Global::DynamicallyReplaceableFunctionImpl => {
                self.write("dynamically replaceable function impl for ")
            }
            Global::CoroFunctionPointer => self.write("coro function pointer to "),
            Global::ReflectionMetadataAssocTypeDescriptor => {
                self.write("reflection metadata associated type descriptor ")?;
                self.print(self.arena.child(id, 0))
            }
            Global::OpaqueTypeDescriptorRecord => {
                self.write("opaque type descriptor runtime record for ")?;
                self.print(self.arena.child(id, 0))
            }
            Global::ProtocolSelfConformanceDescriptor => {
                self.write("protocol self-conformance descriptor for ")?;
                self.print(self.arena.child(id, 0))
            }
            Global::AssociatedTypeDescriptor => {
                self.write("associated type descriptor for ")?;
                self.print(self.arena.child(id, 0))
            }
            Global::ProtocolRequirementsBaseDescriptor => {
                self.write("protocol requirements base descriptor for ")?;
                self.print(self.arena.child(id, 0))
            }
            Global::AssociatedConformanceDescriptor
            | Global::DefaultAssociatedConformanceAccessor => {
                if code == Global::AssociatedConformanceDescriptor {
                    self.write("associated conformance descriptor for ")?;
                } else {
                    self.write("default associated conformance accessor for ")?;
                }
                self.print(self.arena.child(id, 0))?;
                self.write(".")?;
                self.print(self.arena.child(id, 1))?;
                self.write(": ")?;
                self.print(self.arena.child(id, 2))
            }
            Global::BaseConformanceDescriptor => {
                self.write("base conformance descriptor for ")?;
                self.print(self.arena.child(id, 0))?;
                self.write(": ")?;
                self.print(self.arena.child(id, 1))
            }
            Global::DefaultAssociatedTypeMetadataAccessor => {
                self.write("default associated type metadata accessor for ")?;
                self.print(self.arena.child(id, 0))
            }
            Global::ProtocolWitnessThunk => {
                self.write("protocol witness for ")?;
                self.print(self.arena.child(id, 1))?;
                self.write(" in conformance ")?;
                self.print(self.arena.child(id, 0))
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
                let flag = u8::try_from(self.arena.node(id).len).unwrap_or(0);
                self.write("(")?;
                write_number(self.out, index)?;
                self.write(") ")?;
                if flag == b'Y' {
                    self.write("suspend")?;
                } else {
                    self.write("await")?;
                }
                self.write(" resume partial function for ")?;
                self.print(child)
            }
            Global::GlobalVariableOnceFunction => self.with_child(
                "global variable one-time initialization function for ",
                child,
            ),
            Global::GlobalVariableOnceToken => {
                self.with_child("global variable one-time initialization token for ", child)
            }
            _ => Err(fmt::Error),
        }
    }

    fn with_child(&mut self, prefix: &str, child: NodeId) -> fmt::Result {
        self.write(prefix)?;
        if child == NIL {
            return Ok(());
        }
        self.print(child)
    }
}

fn accessor_name(code: Accessor) -> &'static str {
    match code {
        Accessor::Getter => ".getter",
        Accessor::GlobalGetter => ".getter",
        Accessor::Setter => ".setter",
        Accessor::MaterializeForSet => ".materializeForSet",
        Accessor::Read => ".read",
        Accessor::Modify => ".modify",
        Accessor::WillSet => ".willSet",
        Accessor::DidSet => ".didSet",
        Accessor::Modify2 => ".yielding_mutate",
        Accessor::YieldingBorrow => ".yielding_borrow",
        Accessor::Borrow => ".borrow",
        Accessor::YieldingModify => ".modify",
        Accessor::MutableAddressor => ".unsafeMutableAddressor",
        Accessor::NonMutableAddressor => ".unsafeAddressor",
        Accessor::Pseudo => "",
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
    let letter_index = index % 26;
    let group = index / 26;
    let letter = char::from(b'A' + u8::try_from(letter_index).map_err(|_| fmt::Error)?);
    let mut buf = [0_u8; 4];
    out.write_str(letter.encode_utf8(&mut buf))?;
    if group > 0 {
        write_number(out, group)?;
    }
    if depth > 0 {
        write_number(out, depth)?;
    }
    Ok(())
}

fn func_spec_kind_text(raw: u32) -> &'static str {
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

fn option_set_text(raw: u32) -> &'static str {
    if raw & (1 << 8) != 0 {
        "Exploded"
    } else if raw & (1 << 10) != 0 {
        "Existential To Protocol Constrained Generic"
    } else if raw & (1 << 6) != 0 {
        "Dead"
    } else if raw & (1 << 7) != 0 {
        "Owned To Guaranteed"
    } else {
        "Guaranteed To Owned"
    }
}
