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
        while child != NIL {
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
            Kind::ArgumentTuple => self.print_argument_tuple(id),
            Kind::ReturnType => {
                let child = self.arena.child(id, 0);
                self.print(child)
            }
            Kind::Function => self.print_function(id),
            Kind::Variable => self.print_variable(id),
            Kind::Subscript => self.print_subscript(id),
            Kind::Initializer => self.print_initializer(id),
            Kind::Deinitializer => self.print_deinitializer(id),
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
            | Kind::BoundGenericOtherNominal => self.print_bound_generic(id),
            Kind::TypeList => self.children(id, ", "),
            Kind::DependentGenericParamType => self.print_generic_param(id),
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
                let child = self.arena.child(id, 1);
                self.print(child)
            }
            Kind::DependentGenericSignature => self.print_generic_signature(id),
            Kind::DependentGenericParamCount
            | Kind::DependentGenericConformanceRequirement
            | Kind::DependentGenericSameTypeRequirement
            | Kind::DependentGenericBaseClassRequirement
            | Kind::DependentGenericLayoutRequirement => Ok(()),
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
            Kind::ProtocolListType => {
                self.write("any ")?;
                let list = self.arena.child(id, 0);
                self.children(list, " & ")
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
            Kind::ImplIsolated => self.write("isolated "),
            Kind::ImplImplicitLeading => self.write("sil_implicit_leading_param "),
            Kind::ImplEscaping => self.write("@escaping"),
            Kind::ImplErasedIsolation => self.write("@isolated(any)"),
            Kind::ImplNonisolatedNonsending => self.write("nonisolated(nonsending)"),
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
            | Kind::ValueWitness
            | Kind::BuiltinBorrow => Ok(()),
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
        if context != NIL {
            self.print(context)?;
            self.write(".")?;
        }
        self.print(name)
    }

    fn print_extension(&mut self, id: NodeId) -> fmt::Result {
        let module = self.arena.child(id, 0);
        let ty = self.arena.child(id, 1);
        self.write("(extension in ")?;
        self.print(module)?;
        self.write("):")?;
        self.print(ty)
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
        for &(depth, count) in counts.iter().take(depths) {
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
        while child != NIL {
            if self.arena.kind(child) == kind {
                return Some(child);
            }
            child = self.arena.node(child).next;
        }
        None
    }

    fn find_child_of(&self, parent: NodeId, kinds: &[Kind]) -> Option<NodeId> {
        let mut child = self.arena.node(parent).first;
        while child != NIL {
            if kinds.contains(&self.arena.kind(child)) {
                return Some(child);
            }
            child = self.arena.node(child).next;
        }
        None
    }

    fn print_argument_tuple(&mut self, id: NodeId) -> fmt::Result {
        let child = self.arena.child(id, 0);
        let mut inner = child;
        if inner != NIL && self.arena.kind(inner) == Kind::Type {
            inner = self.arena.child(inner, 0);
        }
        if inner != NIL && self.arena.kind(inner) == Kind::Tuple {
            return self.print(inner);
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
        self.print_signature(id)
    }

    /// Print the signature part of a function type: parameters, effects, result.
    fn print_signature(&mut self, func: NodeId) -> fmt::Result {
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
            self.print(args)?;
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
        (func != NIL).then_some(func)
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
        match self.function_node(id) {
            Some(func) => self.print_signature(func),
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
        if context != NIL {
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
        Ok(())
    }

    fn print_subscript(&mut self, id: NodeId) -> fmt::Result {
        let context = self.arena.child(id, 0);
        if context != NIL {
            self.print(context)?;
            self.write(".")?;
        }
        self.write("subscript")
    }

    fn print_initializer(&mut self, id: NodeId) -> fmt::Result {
        let context = self.arena.child(id, 0);
        let name = self.arena.child(id, 1);
        if context != NIL {
            self.print(context)?;
            self.write(".")?;
        }
        if name != NIL {
            self.print(name)?;
        }
        let ty = self.arena.child(id, 3);
        let ty = if ty == NIL {
            self.arena.child(id, 2)
        } else {
            ty
        };
        let mut func = ty;
        if func != NIL && self.arena.kind(func) == Kind::Type {
            func = self.arena.child(func, 0);
        }
        if func == NIL {
            return Ok(());
        }
        let args = self.arena.child(func, 6);
        if args != NIL {
            self.print(args)?;
        }
        self.write(" -> ")?;
        if context != NIL {
            self.print(context)?;
            self.write(".")?;
        }
        if name != NIL {
            self.print(name)
        } else {
            Ok(())
        }
    }

    fn print_deinitializer(&mut self, id: NodeId) -> fmt::Result {
        let context = self.arena.child(id, 0);
        let name = self.arena.child(id, 1);
        if context != NIL {
            self.print(context)?;
            self.write(".")?;
        }
        if name != NIL {
            self.print(name)?;
        }
        self.write(".deinit")
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
        write_number(self.out, index)?;
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

    fn print_impl_function_type(&mut self, id: NodeId) -> fmt::Result {
        let mut first_attr = true;
        let mut child = self.arena.node(id).first;
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
        self.write(") -> (")?;
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
        self.write(")")
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
            if child != convention {
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
                    self.children(id, ", ")?;
                }
                Ok(())
            }
            Global::PartialApplyObjcForwarder => {
                self.write("partial apply ObjC forwarder")?;
                if self.arena.child_count(id) > 0 {
                    self.write(" for ")?;
                    self.children(id, ", ")?;
                }
                Ok(())
            }
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
