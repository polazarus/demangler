//! Fixed-capacity node arena for the Swift demangler.
//!
//! The demangler must not allocate, so nodes live in a fixed array and
//! identifier text lives in a fixed byte buffer. Running out of either is a
//! demangling error, not a panic.

/// Node index. `NIL` terminates child/sibling chains.
pub(crate) type NodeId = u16;

/// Terminator for child and sibling links.
pub(crate) const NIL: NodeId = u16::MAX;

/// Maximum number of nodes per demangling.
pub(crate) const NODE_CAP: usize = 1024;

/// Maximum number of bytes of reconstructed identifier text.
pub(crate) const TEXT_CAP: usize = 4096;

/// Maximum number of substitution entries.
pub(crate) const SUB_CAP: usize = 256;

/// Maximum number of words usable by word substitutions.
pub(crate) const WORD_CAP: usize = 26;

/// Node kinds produced by the parser and consumed by the printer.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
#[allow(dead_code)]
pub(crate) enum Kind {
    /// A module (also produced when an identifier is used as a context).
    Module,
    /// A local declaration name; children are the discriminator and the name.
    LocalDeclName,
    /// A private declaration name; children are the discriminator and name.
    PrivateDeclName,
    /// A related-entity declaration name (`related decl 'x' for name`).
    RelatedEntityDeclName,
    /// A numeric discriminator value (`value` holds it).
    Number,
    /// An opaque return type (`some`).
    OpaqueReturnType,
    /// The declaration an opaque return type belongs to.
    OpaqueReturnTypeOf,
    /// An opaque return type ordinal.
    OpaqueReturnTypeIndex,
    /// A freestanding macro expansion.
    FreestandingMacroExpansion,
    /// A unique-name macro expansion.
    MacroExpansionUniqueName,
    /// A macro expansion source location.
    MacroExpansionLoc,
    /// An infix operator declaration name.
    InfixOperator,
    /// A prefix operator declaration name.
    PrefixOperator,
    /// A postfix operator declaration name.
    PostfixOperator,
    /// A declaration name or identifier.
    Identifier,
    /// `class` nominal type.
    Class,
    /// `struct` nominal type.
    Structure,
    /// `enum` nominal type.
    Enum,
    /// `protocol` nominal type.
    Protocol,
    /// `typealias` nominal type.
    TypeAlias,
    /// Some other nominal type.
    OtherNominal,
    /// Extension context: module + extended entity (+ generic signature).
    Extension,
    /// Wrapper marking a node as a type.
    Type,
    /// Tuple type.
    Tuple,
    /// One tuple element.
    TupleElement,
    /// Name of a tuple element.
    TupleElementName,
    /// Parameter label list.
    LabelList,
    /// The `y` empty-list marker.
    EmptyList,
    /// The `_` first-element marker.
    FirstElementMarker,
    /// Variadic marker (`d`).
    VariadicMarker,
    /// Unmangled suffix (`.<n>` etc.).
    Suffix,
    /// Type mangling for the debugger (`D` suffix).
    TypeMangling,
    /// Function type.
    FunctionType,
    /// `@noescape` function type.
    NoEscapeFunctionType,
    /// `@convention(thin)` function type.
    ThinFunctionType,
    /// `@convention(c)` function type.
    CFunctionPointer,
    /// ObjC block function type.
    ObjCBlock,
    /// Escaping ObjC block function type.
    EscapingObjCBlock,
    /// `@once` function type.
    CalledOnceFunctionType,
    /// `@callable(once)` function type.
    CalledAtMostOnceFunctionType,
    /// `@autoclosure` function type.
    AutoClosureType,
    /// Escaping `@autoclosure` function type.
    EscapingAutoClosureType,
    /// Argument tuple of a function type.
    ArgumentTuple,
    /// Return type of a function type.
    ReturnType,
    /// A function entity.
    Function,
    /// A variable entity.
    Variable,
    /// A subscript entity.
    Subscript,
    /// An initializer.
    Initializer,
    /// A deinitializer.
    Deinitializer,
    /// An allocating initializer (`__allocating_init`).
    Allocator,
    /// A constructor (`init`).
    Constructor,
    /// A property wrapper backing initializer.
    PropertyWrapperBackingInitializer,
    /// A property-wrapped field init accessor.
    PropertyWrappedFieldInitAccessor,
    /// A property wrapper init from projected value.
    PropertyWrapperInitFromProjectedValue,
    /// A generic type parameter declaration.
    GenericTypeParamDecl,
    /// A macro declaration.
    Macro,
    /// An explicit closure.
    ExplicitClosure,
    /// An implicit closure.
    ImplicitClosure,
    /// A default-argument initializer.
    DefaultArgumentInitializer,
    /// An accessor entity.
    Accessor,
    /// The `static` marker.
    Static,
    /// A builtin type such as `Builtin.Int32`.
    BuiltinTypeName,
    /// `Builtin.Borrow<T>`.
    BuiltinBorrow,
    /// `Builtin.Vec<N>x<T>`.
    BuiltinVector,
    /// `Builtin.FixedArray<N, T>`.
    BuiltinFixedArray,
    /// A non-negative integer literal type.
    Integer,
    /// A negative integer literal type (`value` is the absolute value).
    NegativeInteger,
    /// A bound generic with nominal sugar (struct).
    BoundGenericStructure,
    /// A bound generic class.
    BoundGenericClass,
    /// A bound generic enum.
    BoundGenericEnum,
    /// A bound generic protocol.
    BoundGenericProtocol,
    /// A bound generic of another nominal kind.
    BoundGenericOtherNominal,
    /// A bound generic function (specialized declaration reference).
    BoundGenericFunction,
    /// A type list (bound generic arguments).
    TypeList,
    /// A generic parameter reference.
    DependentGenericParamType,
    /// An associated type rooted at a dependent member.
    DependentMemberType,
    /// A generic signature (`l`/`r`).
    DependentGenericSignature,
    /// Number of generic parameters at one depth; count in `value`.
    DependentGenericParamCount,
    /// `A: P` requirement; children are the param type and the protocol.
    DependentGenericConformanceRequirement,
    /// `A == B` requirement.
    DependentGenericSameTypeRequirement,
    /// `A: C` base class requirement.
    DependentGenericBaseClassRequirement,
    /// `A: Layout` requirement; layout kind code in `flags`.
    DependentGenericLayoutRequirement,
    /// `A: ~P` inverse requirement; index in the second child.
    DependentGenericInverseConformanceRequirement,
    /// A generic type with a signature (`u`).
    DependentGenericType,
    /// A metatype.
    Metatype,
    /// An existential metatype.
    ExistentialMetatype,
    /// `inout T`.
    InOut,
    /// `__shared T`.
    Shared,
    /// `__owned T`.
    Owned,
    /// `@noDerivative T`.
    NoDerivative,
    /// `sending T`.
    Sending,
    /// `isolated T`.
    Isolated,
    /// `_const T` (compile-time literal).
    CompileTimeLiteral,
    /// `@const T`.
    ConstValue,
    /// `async` annotation.
    AsyncAnnotation,
    /// `throws` annotation.
    ThrowsAnnotation,
    /// `throws(T)` annotation.
    TypedThrowsAnnotation,
    /// `@Sendable` annotation.
    ConcurrentFunctionType,
    /// `@differentiable(...)` annotation; `value` carries the kind.
    DifferentiableFunctionType,
    /// `@globalActor` annotation.
    GlobalActorFunctionType,
    /// `-> sending T` annotation.
    SendingResultFunctionType,
    /// `@isolated(any)` annotation.
    IsolatedAnyFunctionType,
    /// `nonisolated(nonsending)` annotation.
    NonIsolatedCallerFunctionType,
    /// Implementation (SIL) function type (`I...`).
    ImplFunctionType,
    /// One `ImplParameter`/`ImplResult`/`ImplYield`/`ImplErrorResult` entry.
    ImplParameter,
    /// One result entry of an impl function type.
    ImplResult,
    /// One yield result of an impl function type.
    ImplYield,
    /// The error result of an impl function type.
    ImplErrorResult,
    /// Parameter/result convention; `flags` holds the letter.
    ImplConvention,
    /// `@noDerivative` on a parameter or result.
    ImplDifferentiability,
    /// `sending` on a parameter.
    ImplSending,
    /// `isolated` on a parameter.
    ImplIsolated,
    /// `sil_implicit_leading_param` on a parameter.
    ImplImplicitLeading,
    /// `@escaping` function attribute.
    ImplEscaping,
    /// `@isolated(any)` isolation attribute.
    ImplErasedIsolation,
    /// `nonisolated(nonsending)` isolation attribute.
    ImplNonisolatedNonsending,
    /// `@once` / `@callable(once)` attribute.
    ImplCalledOnce,
    /// Differentiability attribute; `flags` holds the letter.
    ImplDifferentiabilityKind,
    /// Callee convention; `flags` holds the letter.
    ImplCalleeConvention,
    /// Function representation convention; `flags` holds the letter.
    ImplFunctionConvention,
    /// Coroutine kind; `flags` holds the letter.
    ImplCoroutineKind,
    /// `@Sendable` attribute.
    ImplSendable,
    /// `@async` attribute.
    ImplAsync,
    /// `sending` result attribute.
    ImplSendingResult,
    /// Top-level container of a demangled symbol.
    TopLevel,
    /// A function attribute marker (`@objc`, `@nonobjc`, ...).
    Attribute,
    /// A generic specialization prefix.
    GenericSpecialization,
    /// A function signature specialization prefix.
    FunctionSignatureSpecialization,
    /// One argument/return entry of a function signature specialization.
    FuncSpecParam,
    /// Spec kind tag inside a [`Kind::FuncSpecParam`]; raw kind in `value`.
    FuncSpecKind,
    /// Numeric payload inside a [`Kind::FuncSpecParam`].
    FuncSpecNumber,
    /// One generic argument of a specialization.
    GenericSpecializationParam,
    /// A dropped generic argument (`paramN-removed`).
    DroppedArgument,
    /// A vtable override thunk.
    VTableThunk,
    /// An autodiff function thunk.
    AutoDiffFunction,
    /// An autodiff derivative vtable thunk.
    AutoDiffDerivativeVTableThunk,
    /// An autodiff subset-parameters thunk.
    AutoDiffSubsetParametersThunk,
    /// An autodiff self-reordering reabstraction thunk.
    AutoDiffSelfReorderingReabstractionThunk,
    /// A differentiability witness.
    DifferentiabilityWitness,
    /// Autodiff function kind (value holds the kind char).
    AutoDiffFunctionKind,
    /// An autodiff index subset (text holds `S`/`U` markers).
    IndexSubset,
    /// A coroutine continuation prototype.
    CoroutineContinuationPrototype,
    /// An existential type (`any P`).
    ProtocolListType,
    /// A protocol list (inside an existential type).
    ProtocolList,
    /// A protocol conformance; children are type, protocol and module.
    ProtocolConformance,
    /// An associated type reference; children are name and protocol.
    DependentAssociatedTypeRef,
    /// A path of associated types.
    AssocTypePath,
    /// A global symbol wrapper (metadata, witness table, thunk, ...).
    Global,
    /// A value witness table entry; `flags` carries the value-witness kind.
    ValueWitness,
}

impl Kind {
    /// `true` for nominal type kinds that can be a contextual parent.
    pub(crate) const fn is_nominal(self) -> bool {
        matches!(
            self,
            Self::Class
                | Self::Structure
                | Self::Enum
                | Self::Protocol
                | Self::TypeAlias
                | Self::OtherNominal
                | Self::BoundGenericClass
                | Self::BoundGenericStructure
                | Self::BoundGenericEnum
                | Self::BoundGenericOtherNominal
        )
    }

    /// `true` for kinds accepted as declaration names.
    pub(crate) const fn is_decl_name(self) -> bool {
        matches!(
            self,
            Self::Identifier
                | Self::LocalDeclName
                | Self::PrivateDeclName
                | Self::RelatedEntityDeclName
                | Self::InfixOperator
                | Self::PrefixOperator
                | Self::PostfixOperator
        )
    }

    /// `true` for kinds that can be a contextual parent.
    pub(crate) const fn is_context(self) -> bool {
        matches!(
            self,
            Self::Module
                | Self::Class
                | Self::Structure
                | Self::Enum
                | Self::Protocol
                | Self::TypeAlias
                | Self::OtherNominal
                | Self::BoundGenericClass
                | Self::BoundGenericStructure
                | Self::BoundGenericEnum
                | Self::BoundGenericOtherNominal
                | Self::Extension
                | Self::Function
                | Self::Variable
                | Self::Subscript
                | Self::Initializer
                | Self::Deinitializer
                | Self::ExplicitClosure
                | Self::ImplicitClosure
                | Self::DefaultArgumentInitializer
                | Self::Accessor
                | Self::Static
                | Self::DependentGenericParamType
                | Self::TopLevel
                | Self::FreestandingMacroExpansion
                | Self::MacroExpansionUniqueName
        )
    }
}

/// Global symbol wrapper codes (stored in [`Node::flags`] when
/// [`Kind::Global`]).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
#[allow(dead_code)]
pub(crate) enum Global {
    TypeMetadata,
    FullTypeMetadata,
    TypeMetadataPattern,
    TypeMetadataAccess,
    TypeMetadataInstantiationCache,
    TypeMetadataInstantiationFunction,
    TypeMetadataSingletonInitCache,
    TypeMetadataLazyCache,
    TypeMetadataDemanglingCache,
    TypeMetadataMangledNameRef,
    NominalTypeDescriptor,
    ClassMetaclass,
    ClassMethodLookupFunction,
    ObjcMetadataUpdate,
    ObjcResilientClassStub,
    FullObjcResilientClassStub,
    ClassMetadataBaseOffset,
    GenericTypeCompletionFunction,
    ModuleDescriptor,
    ExtensionDescriptor,
    AnonymousContextDescriptor,
    ProtocolDescriptor,
    ProtocolDescriptorRecord,
    NominalTypeDescriptorRecord,
    ProtocolConformanceDescriptorRecord,
    AccessibleFunctionRecord,
    ProtocolConformanceDescriptor,
    ProtocolWitnessTable,
    ProtocolWitnessTableAccessor,
    ProtocolWitnessTablePattern,
    GenericWitnessTableInstantiationFunction,
    GenericProtocolWitnessTable,
    ResilientProtocolWitnessTable,
    LazyProtocolWitnessTableCache,
    LazyProtocolWitnessTableAccessor,
    AssociatedTypeMetadataAccessor,
    AssociatedTypeWitnessTableAccessor,
    BaseProtocolWitnessTableAccessor,
    ProtocolSelfConformanceWitnessTable,
    ProtocolSelfConformanceDescriptor,
    ValueWitnessTable,
    FieldOffset,
    EnumTagIndex,
    OutlinedCopy,
    OutlinedConsume,
    OutlinedRetain,
    OutlinedRelease,
    OutlinedInitWithTake,
    OutlinedInitWithCopy,
    OutlinedAssignWithTake,
    OutlinedAssignWithCopy,
    OutlinedDestroy,
    OutlinedEnumTagStore,
    OutlinedEnumProjectData,
    OutlinedEnumGetTag,
    OutlinedVariable,
    OutlinedReadOnlyObject,
    OutlinedBridgedMethod,
    PartialApplyForwarder,
    PartialApplyObjcForwarder,
    ReabstractionThunkHelper,
    ReabstractionThunk,
    DispatchThunk,
    MethodDescriptor,
    ObjcToSwiftThunk,
    SwiftToObjcThunk,
    DynamicDispatchThunk,
    DirectMethodReferenceThunk,
    AsyncFunctionPointer,
    DynamicReplacementFunctionPointer,
    MergedFunction,
    ProtocolWitnessThunk,
    ProtocolSelfConformanceWitnessThunk,
    BackDeploymentCopy,
    BackDeploymentThunk,
    BackDeploymentFallback,
    HasSymbolQuery,
    ResumePartialFunction,
    GlobalVariableOnceFunction,
    GlobalVariableOnceToken,
    TypeMetadataCompletionFunction,
    DistributedThunk,
    DistributedAccessor,
    DynamicallyReplaceableFunctionVar,
    DynamicallyReplaceableFunctionKey,
    DynamicallyReplaceableFunctionImpl,
    CoroFunctionPointer,
    DefaultOverride,
    ReflectionMetadataAssocTypeDescriptor,
    OpaqueTypeDescriptorRecord,
    AssociatedTypeDescriptor,
    ProtocolRequirementsBaseDescriptor,
    AssociatedConformanceDescriptor,
    DefaultAssociatedConformanceAccessor,
    BaseConformanceDescriptor,
    DefaultAssociatedTypeMetadataAccessor,
}

impl Global {
    /// Decode a stored code.
    pub(crate) fn from_code(code: u8) -> Option<Self> {
        const ALL: &[Global] = &[
            Global::TypeMetadata,
            Global::FullTypeMetadata,
            Global::TypeMetadataPattern,
            Global::TypeMetadataAccess,
            Global::TypeMetadataInstantiationCache,
            Global::TypeMetadataInstantiationFunction,
            Global::TypeMetadataSingletonInitCache,
            Global::TypeMetadataLazyCache,
            Global::TypeMetadataDemanglingCache,
            Global::TypeMetadataMangledNameRef,
            Global::NominalTypeDescriptor,
            Global::ClassMetaclass,
            Global::ClassMethodLookupFunction,
            Global::ObjcMetadataUpdate,
            Global::ObjcResilientClassStub,
            Global::FullObjcResilientClassStub,
            Global::ClassMetadataBaseOffset,
            Global::GenericTypeCompletionFunction,
            Global::ModuleDescriptor,
            Global::ExtensionDescriptor,
            Global::AnonymousContextDescriptor,
            Global::ProtocolDescriptor,
            Global::ProtocolDescriptorRecord,
            Global::NominalTypeDescriptorRecord,
            Global::ProtocolConformanceDescriptorRecord,
            Global::AccessibleFunctionRecord,
            Global::ProtocolConformanceDescriptor,
            Global::ProtocolWitnessTable,
            Global::ProtocolWitnessTableAccessor,
            Global::ProtocolWitnessTablePattern,
            Global::GenericWitnessTableInstantiationFunction,
            Global::GenericProtocolWitnessTable,
            Global::ResilientProtocolWitnessTable,
            Global::LazyProtocolWitnessTableCache,
            Global::LazyProtocolWitnessTableAccessor,
            Global::AssociatedTypeMetadataAccessor,
            Global::AssociatedTypeWitnessTableAccessor,
            Global::BaseProtocolWitnessTableAccessor,
            Global::ProtocolSelfConformanceWitnessTable,
            Global::ProtocolSelfConformanceDescriptor,
            Global::ValueWitnessTable,
            Global::FieldOffset,
            Global::EnumTagIndex,
            Global::OutlinedCopy,
            Global::OutlinedConsume,
            Global::OutlinedRetain,
            Global::OutlinedRelease,
            Global::OutlinedInitWithTake,
            Global::OutlinedInitWithCopy,
            Global::OutlinedAssignWithTake,
            Global::OutlinedAssignWithCopy,
            Global::OutlinedDestroy,
            Global::OutlinedEnumTagStore,
            Global::OutlinedEnumProjectData,
            Global::OutlinedEnumGetTag,
            Global::OutlinedVariable,
            Global::OutlinedReadOnlyObject,
            Global::OutlinedBridgedMethod,
            Global::PartialApplyForwarder,
            Global::PartialApplyObjcForwarder,
            Global::ReabstractionThunkHelper,
            Global::ReabstractionThunk,
            Global::DispatchThunk,
            Global::MethodDescriptor,
            Global::ObjcToSwiftThunk,
            Global::SwiftToObjcThunk,
            Global::DynamicDispatchThunk,
            Global::DirectMethodReferenceThunk,
            Global::AsyncFunctionPointer,
            Global::DynamicReplacementFunctionPointer,
            Global::MergedFunction,
            Global::ProtocolWitnessThunk,
            Global::ProtocolSelfConformanceWitnessThunk,
            Global::BackDeploymentCopy,
            Global::BackDeploymentThunk,
            Global::BackDeploymentFallback,
            Global::HasSymbolQuery,
            Global::ResumePartialFunction,
            Global::GlobalVariableOnceFunction,
            Global::GlobalVariableOnceToken,
            Global::TypeMetadataCompletionFunction,
            Global::DistributedThunk,
            Global::DistributedAccessor,
            Global::DynamicallyReplaceableFunctionVar,
            Global::DynamicallyReplaceableFunctionKey,
            Global::DynamicallyReplaceableFunctionImpl,
            Global::CoroFunctionPointer,
            Global::DefaultOverride,
            Global::ReflectionMetadataAssocTypeDescriptor,
            Global::OpaqueTypeDescriptorRecord,
            Global::AssociatedTypeDescriptor,
            Global::ProtocolRequirementsBaseDescriptor,
            Global::AssociatedConformanceDescriptor,
            Global::DefaultAssociatedConformanceAccessor,
            Global::BaseConformanceDescriptor,
            Global::DefaultAssociatedTypeMetadataAccessor,
        ];
        ALL.iter().copied().find(|global| *global as u8 == code)
    }
}

/// Accessor codes (stored in [`Node::flags`] when [`Kind::Accessor`]).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
#[allow(dead_code)]
pub(crate) enum Accessor {
    Getter,
    GlobalGetter,
    Setter,
    MaterializeForSet,
    Read,
    Modify,
    WillSet,
    DidSet,
    Modify2,
    YieldingBorrow,
    Borrow,
    YieldingModify,
    MutableAddressor,
    NonMutableAddressor,
    Pseudo,
}

impl Accessor {
    /// Decode a stored accessor code (1-based; 0 means "no accessor").
    pub(crate) fn from_code(code: u8) -> Option<Self> {
        const ALL: &[Accessor] = &[
            Accessor::Getter,
            Accessor::GlobalGetter,
            Accessor::Setter,
            Accessor::MaterializeForSet,
            Accessor::Read,
            Accessor::Modify,
            Accessor::WillSet,
            Accessor::DidSet,
            Accessor::Modify2,
            Accessor::YieldingBorrow,
            Accessor::Borrow,
            Accessor::YieldingModify,
            Accessor::MutableAddressor,
            Accessor::NonMutableAddressor,
            Accessor::Pseudo,
        ];
        if code == 0 {
            return None;
        }
        ALL.get(usize::from(code) - 1).copied()
    }
}

/// A single demangler node.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Node {
    /// Node kind.
    pub kind: Kind,
    /// Kind-specific flag byte (global/accessor codes, builtin kinds).
    pub flags: u8,
    /// Text offset in the arena buffer, or a numeric value (index).
    pub value: u32,
    /// Text length, or the low part of a numeric value.
    pub len: u16,
    /// First child.
    pub first: NodeId,
    /// Next sibling.
    pub next: NodeId,
}

impl Node {
    const EMPTY: Self = Self {
        kind: Kind::Module,
        flags: 0,
        value: 0,
        len: 0,
        first: NIL,
        next: NIL,
    };
}

/// Fixed-capacity node and text arena.
pub(crate) struct Arena {
    nodes: [Node; NODE_CAP],
    len: usize,
    text: [u8; TEXT_CAP],
    text_len: usize,
    subs: [NodeId; SUB_CAP],
    sub_len: usize,
    words: [(u32, u16); WORD_CAP],
    word_len: usize,
}

#[allow(dead_code)]
impl Arena {
    /// Create an empty arena.
    pub(crate) const fn new() -> Self {
        Self {
            nodes: [Node::EMPTY; NODE_CAP],
            len: 0,
            text: [0; TEXT_CAP],
            text_len: 0,
            subs: [NIL; SUB_CAP],
            sub_len: 0,
            words: [(0, 0); WORD_CAP],
            word_len: 0,
        }
    }

    /// Allocate a node with no children.
    pub(crate) fn alloc(&mut self, kind: Kind) -> Option<NodeId> {
        let id = u16::try_from(self.len).ok()?;
        if self.len >= NODE_CAP {
            return None;
        }
        self.nodes[self.len] = Node {
            kind,
            ..Node::EMPTY
        };
        self.len += 1;
        Some(id)
    }

    /// Allocate a node with text copied from `text`.
    pub(crate) fn alloc_text(&mut self, kind: Kind, text: &str) -> Option<NodeId> {
        let start = self.text_len;
        let end = start.checked_add(text.len())?;
        if end > TEXT_CAP {
            return None;
        }
        self.text[start..end].copy_from_slice(text.as_bytes());
        let id = self.alloc(kind)?;
        let node = &mut self.nodes[usize::from(id)];
        node.value = u32::try_from(start).ok()?;
        node.len = u16::try_from(text.len()).ok()?;
        self.text_len = end;
        Some(id)
    }

    /// Allocate a node with a numeric value.
    pub(crate) fn alloc_value(&mut self, kind: Kind, value: u32) -> Option<NodeId> {
        let id = self.alloc(kind)?;
        self.nodes[usize::from(id)].value = value;
        Some(id)
    }

    /// Node accessor.
    pub(crate) fn node(&self, id: NodeId) -> &Node {
        &self.nodes[usize::from(id)]
    }

    /// Mutable node accessor.
    pub(crate) fn node_mut(&mut self, id: NodeId) -> &mut Node {
        &mut self.nodes[usize::from(id)]
    }

    /// Node kind.
    pub(crate) fn kind(&self, id: NodeId) -> Kind {
        self.nodes[usize::from(id)].kind
    }

    /// Text of a text-bearing node.
    pub(crate) fn text(&self, id: NodeId) -> &str {
        let node = &self.nodes[usize::from(id)];
        let start = node.value as usize;
        let end = start + usize::from(node.len);
        core::str::from_utf8(&self.text[start..end]).unwrap_or("")
    }

    /// Append `child` to `parent`'s child list.
    ///
    /// A node can only appear once in a linked list, but substitutions can
    /// legitimately reference the same node twice. In that case the subtree is
    /// cloned so both positions print correctly.
    pub(crate) fn add_child(&mut self, parent: NodeId, child: NodeId) {
        if child == NIL {
            return;
        }
        let mut current = self.nodes[usize::from(parent)].first;
        let mut guard = 0_usize;
        while current != NIL {
            guard += 1;
            if guard > NODE_CAP {
                return;
            }
            if current == child {
                let mut budget = NODE_CAP;
                if let Some(clone) = self.clone_tree(child, &mut budget) {
                    self.append(parent, clone);
                }
                return;
            }
            current = self.nodes[usize::from(current)].next;
        }
        self.append(parent, child);
    }

    fn append(&mut self, parent: NodeId, child: NodeId) {
        self.nodes[usize::from(child)].next = NIL;
        let parent_idx = usize::from(parent);
        let first = self.nodes[parent_idx].first;
        if first == NIL {
            self.nodes[parent_idx].first = child;
            return;
        }
        let mut current = first;
        let mut guard = 0_usize;
        loop {
            guard += 1;
            if guard > NODE_CAP {
                return;
            }
            let next = self.nodes[usize::from(current)].next;
            if next == NIL {
                self.nodes[usize::from(current)].next = child;
                return;
            }
            current = next;
        }
    }

    fn clone_tree(&mut self, src: NodeId, budget: &mut usize) -> Option<NodeId> {
        if *budget == 0 {
            return None;
        }
        *budget -= 1;
        let node = self.nodes[usize::from(src)];
        let new = self.alloc(node.kind)?;
        {
            let copy = self.node_mut(new);
            copy.flags = node.flags;
            copy.value = node.value;
            copy.len = node.len;
        }
        let mut child = node.first;
        while child != NIL {
            let copy = self.clone_tree(child, budget)?;
            self.append(new, copy);
            child = self.nodes[usize::from(child)].next;
        }
        Some(new)
    }

    /// Create a copy of `id` with a different kind.
    ///
    /// Nodes can be shared through substitutions, so changing a node's kind
    /// must not mutate it in place: the same node may still be referenced by
    /// another subtree (and its sibling link may already point elsewhere).
    pub(crate) fn change_kind(&mut self, id: NodeId, kind: Kind) -> Option<NodeId> {
        let node = self.nodes[usize::from(id)];
        let new = self.alloc(kind)?;
        {
            let copy = self.node_mut(new);
            copy.flags = node.flags;
            copy.value = node.value;
            copy.len = node.len;
        }
        let mut child = node.first;
        while child != NIL {
            let next = self.nodes[usize::from(child)].next;
            self.nodes[usize::from(child)].next = NIL;
            self.add_child(new, child);
            child = next;
        }
        Some(new)
    }

    /// Add all children of `other` to `parent`.
    pub(crate) fn add_children_of(&mut self, parent: NodeId, other: NodeId) {
        let mut child = self.nodes[usize::from(other)].first;
        while child != NIL {
            let next = self.nodes[usize::from(child)].next;
            self.nodes[usize::from(child)].next = NIL;
            self.add_child(parent, child);
            child = next;
        }
    }

    /// Detach and return the first child of `parent`.
    pub(crate) fn take_first_child(&mut self, parent: NodeId) -> NodeId {
        let idx = usize::from(parent);
        let first = self.nodes[idx].first;
        if first != NIL {
            self.nodes[idx].first = self.nodes[usize::from(first)].next;
            self.nodes[usize::from(first)].next = NIL;
        }
        first
    }

    /// Number of children of `parent`.
    pub(crate) fn child_count(&self, parent: NodeId) -> usize {
        let mut count = 0;
        let mut child = self.nodes[usize::from(parent)].first;
        while child != NIL && count < NODE_CAP {
            count += 1;
            child = self.nodes[usize::from(child)].next;
        }
        count
    }

    /// Nth child (0-based).
    pub(crate) fn child(&self, parent: NodeId, index: usize) -> NodeId {
        let mut child = self.nodes[usize::from(parent)].first;
        let mut i = 0;
        while child != NIL && i < NODE_CAP {
            if i == index {
                return child;
            }
            i += 1;
            child = self.nodes[usize::from(child)].next;
        }
        NIL
    }

    /// Iterate child ids of `parent`.
    pub(crate) fn children(&self, parent: NodeId) -> ChildIter<'_> {
        ChildIter {
            arena: self,
            current: self.nodes[usize::from(parent)].first,
        }
    }

    /// Push a substitution entry.
    pub(crate) fn add_sub(&mut self, id: NodeId) {
        if self.sub_len < SUB_CAP {
            self.subs[self.sub_len] = id;
            self.sub_len += 1;
        }
    }

    /// Fetch substitution by index.
    pub(crate) fn sub(&self, index: usize) -> Option<NodeId> {
        if index < self.sub_len {
            Some(self.subs[index])
        } else {
            None
        }
    }

    /// Remember a word for word substitutions.
    pub(crate) fn add_word(&mut self, offset: u32, len: u16) {
        if self.word_len < WORD_CAP {
            self.words[self.word_len] = (offset, len);
            self.word_len += 1;
        }
    }

    /// Word text by index.
    pub(crate) fn word(&self, index: usize) -> &str {
        if index >= self.word_len {
            return "";
        }
        let (offset, len) = self.words[index];
        let start = offset as usize;
        let end = start + usize::from(len);
        core::str::from_utf8(&self.text[start..end]).unwrap_or("")
    }

    /// Number of remembered words.
    pub(crate) fn word_count(&self) -> usize {
        self.word_len
    }

    /// Residual text capacity.
    pub(crate) fn text_room(&self) -> usize {
        TEXT_CAP - self.text_len
    }

    /// Append a raw byte run to the text buffer, returning (offset, len).
    pub(crate) fn push_text(&mut self, bytes: &[u8]) -> Option<(u32, u16)> {
        let start = self.text_len;
        let end = start.checked_add(bytes.len())?;
        if end > TEXT_CAP {
            return None;
        }
        self.text[start..end].copy_from_slice(bytes);
        self.text_len = end;
        Some((u32::try_from(start).ok()?, u16::try_from(bytes.len()).ok()?))
    }
}

impl core::fmt::Debug for Arena {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Arena")
            .field("len", &self.len)
            .field("text_len", &self.text_len)
            .field("sub_len", &self.sub_len)
            .field("word_len", &self.word_len)
            .finish()
    }
}

/// Iterator over the children of a node.
#[derive(Clone, Copy)]
#[allow(dead_code)]
pub(crate) struct ChildIter<'a> {
    arena: &'a Arena,
    current: NodeId,
}

impl<'a> Iterator for ChildIter<'a> {
    type Item = NodeId;

    fn next(&mut self) -> Option<Self::Item> {
        if self.current == NIL {
            return None;
        }
        let id = self.current;
        self.current = self.arena.node(id).next;
        Some(id)
    }
}

/// Map an accessor letter to its [`Accessor`] code.
pub(crate) const fn accessor_code(byte: u8) -> u8 {
    // Codes are shifted by one so that 0 means "no accessor".
    match byte {
        b'g' => 1,
        b'G' => 2,
        b's' => 3,
        b'm' => 4,
        b'r' => 5,
        b'M' => 6,
        b'w' => 7,
        b'W' => 8,
        b'x' => 9,
        b'y' => 10,
        b'b' => 11,
        b'z' => 12,
        b'a' => 13,
        b'l' => 14,
        b'p' => 15,
        _ => 0,
    }
}
