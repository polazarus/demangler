//! Growable node arena for the Swift demangler.
//!
//! The demangler uses the `alloc` crate for nodes, reconstructed identifier
//! text, substitutions and word substitutions. There are no fixed capacities.

use alloc::vec::Vec;

/// Node index. `NIL` terminates child/sibling chains.
pub(crate) type NodeId = usize;

/// Terminator for child and sibling links.
pub(crate) const NIL: NodeId = usize::MAX;

/// Maximum depth of a substitution clone.
///
/// Substitutions may reference the same already-linked subtree repeatedly.
/// Cloning it is necessary for a tree-shaped printer, but an adversarial input
/// could otherwise force exponential work and stack growth.
const MAX_CLONE_DEPTH: usize = 256;

/// Maximum number of nodes copied by one substitution clone.
const MAX_CLONE_NODES: usize = 0x4000;

/// Maximum total number of nodes copied by all substitutions in one parse.
const MAX_TOTAL_CLONE_NODES: usize = 0x1_0000;

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
    /// A variadic pack type.
    Pack,
    /// A constrained existential (`any P<...>`).
    ConstrainedExistential,
    /// Requirements of a constrained existential.
    ConstrainedExistentialRequirementList,
    /// An opaque type declaration (`name.ordinal`).
    OpaqueType,
    /// An opaque return type (`some`).
    OpaqueReturnType,
    /// The declaration an opaque return type belongs to.
    OpaqueReturnTypeOf,
    /// An opaque return type ordinal.
    OpaqueReturnTypeIndex,
    /// A freestanding macro expansion.
    FreestandingMacroExpansion,
    /// An attached macro expansion; `flags` holds the role letter.
    AttachedMacroExpansion,
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
    /// `Self` inside a constrained existential.
    ConstrainedExistentialSelf,
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
    /// Pattern substitutions of an impl function type.
    ImplPatternSubstitutions,
    /// Invocation substitutions of an impl function type.
    ImplInvocationSubstitutions,
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
    /// A key-path thunk helper (getter/setter/unapplied/applied).
    KeyPathThunkHelper,
    /// A SIL box type with an explicit layout.
    SILBoxTypeWithLayout,
    /// The layout of a SIL box.
    SILBoxLayout,
    /// A mutable SIL-box field.
    SILBoxMutableField,
    /// An immutable SIL-box field.
    SILBoxImmutableField,
    /// The `serialized` marker on a key-path thunk.
    IsSerialized,
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
    /// `any P & AnyObject`.
    ProtocolListWithAnyObject,
    /// An anonymous runtime context; children are name, parent and types.
    AnonymousContext,
    /// A protocol conformance; children are type, protocol and module.
    ProtocolConformance,
    /// A protocol conformance reference in the type's module.
    ProtocolConformanceRefInTypeModule,
    /// A protocol conformance reference in the protocol's module.
    ProtocolConformanceRefInProtocolModule,
    /// A retroactive protocol conformance reference.
    ProtocolConformanceRefInOtherModule,
    /// A concrete protocol conformance.
    ConcreteProtocolConformance,
    /// A list of conditional protocol conformances.
    AnyProtocolConformanceList,
    /// A pack protocol conformance.
    PackProtocolConformance,
    /// A retroactive protocol conformance wrapper.
    RetroactiveConformance,
    /// A dependent conformance rooted at a type.
    DependentProtocolConformanceRoot,
    /// A dependent inherited conformance.
    DependentProtocolConformanceInherited,
    /// A dependent associated conformance.
    DependentProtocolConformanceAssociated,
    /// A dependent opaque conformance.
    DependentProtocolConformanceOpaque,
    /// An associated conformance of a dependent type.
    DependentAssociatedConformance,
    /// An associated type reference; children are name and protocol.
    DependentAssociatedTypeRef,
    /// A path of associated types.
    AssocTypePath,
    /// A global symbol wrapper (metadata, witness table, thunk, ...).
    Global,
    /// A value witness table entry; `flags` carries the value-witness kind.
    ValueWitness,
    /// An enum case declaration.
    EnumCase,
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

    /// `true` for unbound generic nominal kinds (`G` operands).
    pub(crate) const fn is_any_generic(self) -> bool {
        matches!(
            self,
            Self::Class
                | Self::Structure
                | Self::Enum
                | Self::Protocol
                | Self::TypeAlias
                | Self::OtherNominal
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
                | Self::AnonymousContext
                | Self::Function
                | Self::Variable
                | Self::Subscript
                | Self::Initializer
                | Self::Allocator
                | Self::Constructor
                | Self::Deinitializer
                | Self::ExplicitClosure
                | Self::ImplicitClosure
                | Self::DefaultArgumentInitializer
                | Self::Accessor
                | Self::Static
                | Self::DependentGenericParamType
                | Self::TopLevel
                | Self::FreestandingMacroExpansion
                | Self::AttachedMacroExpansion
                | Self::MacroExpansionLoc
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
    PropertyDescriptor,
    OpaqueTypeDescriptor,
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
            Global::PropertyDescriptor,
            Global::OpaqueTypeDescriptor,
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
    /// Text length, or a numeric value.
    pub len: u32,
    /// First child.
    pub first: NodeId,
    /// Next sibling.
    pub next: NodeId,
    /// Whether this node is already part of a sibling list.
    pub linked: bool,
}

impl Node {
    const EMPTY: Self = Self {
        kind: Kind::Module,
        flags: 0,
        value: 0,
        len: 0,
        first: NIL,
        next: NIL,
        linked: false,
    };
}

/// Growable node and text arena.
pub(crate) struct Arena {
    nodes: Vec<Node>,
    text: Vec<u8>,
    subs: Vec<NodeId>,
    words: Vec<(u32, u32)>,
    /// Nodes copied by substitution expansion so far.
    cloned: usize,
    /// Set when a safety limit rejected part of an operation.
    failed: bool,
}

#[allow(dead_code)]
impl Arena {
    /// Create an empty arena.
    pub(crate) const fn new() -> Self {
        Self {
            nodes: Vec::new(),
            text: Vec::new(),
            subs: Vec::new(),
            words: Vec::new(),
            cloned: 0,
            failed: false,
        }
    }

    /// Whether an operation exceeded a safety limit.
    pub(crate) const fn failed(&self) -> bool {
        self.failed
    }

    /// Allocate a node with no children.
    pub(crate) fn alloc(&mut self, kind: Kind) -> NodeId {
        let id = self.nodes.len();
        self.nodes.push(Node {
            kind,
            ..Node::EMPTY
        });
        id
    }

    /// Allocate a node with text copied from `text`.
    pub(crate) fn alloc_text(&mut self, kind: Kind, text: &str) -> NodeId {
        let start = self.text.len();
        self.text.extend_from_slice(text.as_bytes());
        let id = self.alloc(kind);
        let node = &mut self.nodes[id];
        node.value = start as u32;
        node.len = text.len() as u32;
        id
    }

    /// Allocate a node with a numeric value.
    pub(crate) fn alloc_value(&mut self, kind: Kind, value: u32) -> NodeId {
        let id = self.alloc(kind);
        self.nodes[id].value = value;
        id
    }

    /// Node accessor.
    pub(crate) fn node(&self, id: NodeId) -> &Node {
        &self.nodes[id]
    }

    /// Mutable node accessor.
    pub(crate) fn node_mut(&mut self, id: NodeId) -> &mut Node {
        &mut self.nodes[id]
    }

    /// Node kind.
    pub(crate) fn kind(&self, id: NodeId) -> Kind {
        self.nodes[id].kind
    }

    /// Text of a text-bearing node.
    pub(crate) fn text(&self, id: NodeId) -> &str {
        let node = &self.nodes[id];
        let start = node.value as usize;
        let Some(end) = start.checked_add(node.len as usize) else {
            return "";
        };
        let Some(bytes) = self.text.get(start..end) else {
            return "";
        };
        core::str::from_utf8(bytes).unwrap_or("")
    }

    /// Append `child` to `parent`'s child list.
    ///
    /// A node can only appear once in a linked list, but substitutions can
    /// legitimately reference the same node twice. In that case the subtree is
    /// cloned so both positions print correctly.
    pub(crate) fn add_child(&mut self, parent: NodeId, child: NodeId) {
        if child == NIL || child == parent {
            return;
        }
        // A node can only be in one sibling list. Substitutions may reference a
        // linked node again; clone it so the original list stays intact.
        if self.nodes[child].linked {
            let mut budget = MAX_CLONE_NODES;
            if let Some(clone) = self.clone_tree(child, 0, &mut budget) {
                self.append(parent, clone);
            } else {
                self.failed = true;
            }
            return;
        }
        self.append(parent, child);
    }

    fn append(&mut self, parent: NodeId, child: NodeId) {
        {
            let node = &mut self.nodes[child];
            node.next = NIL;
            node.linked = true;
        }
        if self.nodes[parent].first == NIL {
            self.nodes[parent].first = child;
            return;
        }
        let mut current = self.nodes[parent].first;
        loop {
            let next = self.nodes[current].next;
            if next == NIL {
                self.nodes[current].next = child;
                return;
            }
            current = next;
        }
    }

    fn clone_tree(&mut self, src: NodeId, depth: usize, budget: &mut usize) -> Option<NodeId> {
        if depth > MAX_CLONE_DEPTH || *budget == 0 || self.cloned >= MAX_TOTAL_CLONE_NODES {
            return None;
        }
        *budget -= 1;
        self.cloned += 1;
        let node = self.nodes[src];
        let new = self.alloc(node.kind);
        {
            let copy = self.node_mut(new);
            copy.flags = node.flags;
            copy.value = node.value;
            copy.len = node.len;
        }
        let mut child = node.first;
        while child != NIL {
            let next = self.nodes[child].next;
            let copy = self.clone_tree(child, depth + 1, budget)?;
            self.append(new, copy);
            child = next;
        }
        Some(new)
    }

    /// Create a copy of `id` with a different kind.
    ///
    /// Nodes can be shared through substitutions, so changing a node's kind
    /// must not mutate it in place: the same node may still be referenced by
    /// another subtree (and its sibling link may already point elsewhere).
    pub(crate) fn change_kind(&mut self, id: NodeId, kind: Kind) -> NodeId {
        let node = self.nodes[id];
        let new = self.alloc(kind);
        {
            let copy = self.node_mut(new);
            copy.flags = node.flags;
            copy.value = node.value;
            copy.len = node.len;
        }
        let mut child = node.first;
        while child != NIL {
            let next = self.nodes[child].next;
            self.nodes[child].next = NIL;
            self.nodes[child].linked = false;
            self.add_child(new, child);
            child = next;
        }
        new
    }

    /// Add all children of `other` to `parent`.
    pub(crate) fn add_children_of(&mut self, parent: NodeId, other: NodeId) {
        let mut child = self.nodes[other].first;
        while child != NIL {
            let next = self.nodes[child].next;
            self.nodes[child].next = NIL;
            self.nodes[child].linked = false;
            self.add_child(parent, child);
            child = next;
        }
    }

    /// Detach and return the first child of `parent`.
    pub(crate) fn take_first_child(&mut self, parent: NodeId) -> NodeId {
        let first = self.nodes[parent].first;
        if first != NIL {
            self.nodes[parent].first = self.nodes[first].next;
            self.nodes[first].next = NIL;
            self.nodes[first].linked = false;
        }
        first
    }

    /// Number of children of `parent`.
    pub(crate) fn child_count(&self, parent: NodeId) -> usize {
        self.children(parent).count()
    }

    /// Nth child (0-based).
    pub(crate) fn child(&self, parent: NodeId, index: usize) -> NodeId {
        self.children(parent).nth(index).unwrap_or(NIL)
    }

    /// Iterate child ids of `parent`.
    pub(crate) fn children(&self, parent: NodeId) -> ChildIter<'_> {
        ChildIter {
            arena: self,
            current: self.nodes[parent].first,
        }
    }

    /// Push a substitution entry.
    pub(crate) fn add_sub(&mut self, id: NodeId) {
        self.subs.push(id);
    }

    /// Fetch substitution by index.
    pub(crate) fn sub(&self, index: usize) -> Option<NodeId> {
        self.subs.get(index).copied()
    }

    /// Remember a word for word substitutions.
    pub(crate) fn add_word(&mut self, offset: u32, len: u32) {
        self.words.push((offset, len));
    }

    /// Word text by index.
    pub(crate) fn word(&self, index: usize) -> &str {
        let Some(&(offset, len)) = self.words.get(index) else {
            return "";
        };
        let start = offset as usize;
        let Some(end) = start.checked_add(len as usize) else {
            return "";
        };
        self.text
            .get(start..end)
            .and_then(|bytes| core::str::from_utf8(bytes).ok())
            .unwrap_or("")
    }

    /// Number of remembered words.
    pub(crate) fn word_count(&self) -> usize {
        self.words.len()
    }

    /// Append a raw byte run to the text buffer, returning (offset, len).
    pub(crate) fn push_text(&mut self, bytes: &[u8]) -> (u32, u32) {
        let start = self.text.len();
        self.text.extend_from_slice(bytes);
        (start as u32, bytes.len() as u32)
    }
}

impl core::fmt::Debug for Arena {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Arena")
            .field("nodes", &self.nodes.len())
            .field("text_len", &self.text.len())
            .field("sub_len", &self.subs.len())
            .field("word_len", &self.words.len())
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

#[cfg(test)]
mod tests {
    use super::{Arena, MAX_CLONE_DEPTH};

    #[test]
    fn clone_depth_is_bounded() {
        let mut arena = Arena::new();
        let mut child = arena.alloc(super::Kind::Identifier);
        for _ in 0..=MAX_CLONE_DEPTH {
            let parent = arena.alloc(super::Kind::Type);
            arena.add_child(parent, child);
            child = parent;
        }
        let holder = arena.alloc(super::Kind::Tuple);
        arena.add_child(holder, child);
        assert!(!arena.failed());

        let other = arena.alloc(super::Kind::Tuple);
        arena.add_child(other, child);
        assert!(arena.failed());
    }
}
