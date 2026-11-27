//! Immutable reference-counted node graph for the Swift demangler.
//!
//! A node is built once from a [`Builder`] and never mutated afterwards. Nodes
//! own their children as a sealed slice of [`NodeRef`] handles; a node reused
//! through a substitution is shared, not copied, so the finished graph may be a
//! DAG. Literal text borrows the input or static tables; reconstructed text is
//! owned by `Cow`. Identifier bytes that are not valid UTF-8 are escaped when
//! they become node text, so text is always valid `str`.

use alloc::{borrow::Cow, boxed::Box, rc::Rc, vec::Vec};

/// Node kinds produced by the parser and consumed by the printer.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
#[non_exhaustive]
pub enum Kind {
    /// A module (also produced when an identifier is used as a context).
    Module,
    /// A local declaration name; children are the discriminator and the name.
    LocalDeclName,
    /// A private declaration name; children are the discriminator and name.
    PrivateDeclName,
    /// A related-entity declaration name (`related decl 'x' for name`).
    RelatedEntityDeclName,
    /// A numeric discriminator value.
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
    /// An attached macro expansion.
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
    /// The placeholder an unparsable type prints as (`<ERROR TYPE>`).
    ErrorType,
    /// `Self` in a declaration's signature (legacy `_T` mangling).
    DynamicSelf,
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
    /// An Objective-C ivar initializer (legacy `_T` mangling).
    IVarInitializer,
    /// An Objective-C ivar destroyer (legacy `_T` mangling).
    IVarDestroyer,
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
    /// A negative integer literal type.
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
    /// Number of generic parameters at one depth.
    DependentGenericParamCount,
    /// `A: P` requirement; children are the param type and the protocol.
    DependentGenericConformanceRequirement,
    /// `A == B` requirement.
    DependentGenericSameTypeRequirement,
    /// `A: C` base class requirement.
    DependentGenericBaseClassRequirement,
    /// `A: Layout` requirement; the layout kind is a typed code.
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
    /// `unowned T`.
    Unowned,
    /// `unowned(unsafe) T`.
    Unmanaged,
    /// `weak T`.
    Weak,
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
    /// `@differentiable(...)` annotation.
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
    /// Parameter/result convention.
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
    /// Differentiability attribute.
    ImplDifferentiabilityKind,
    /// Callee convention.
    ImplCalleeConvention,
    /// Function representation convention.
    ImplFunctionConvention,
    /// Coroutine kind.
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
    /// Spec kind tag inside a [`Kind::FuncSpecParam`]; raw kind in the numeric payload.
    FuncSpecKind,
    /// Numeric payload inside a [`Kind::FuncSpecParam`].
    FuncSpecNumber,
    /// One generic argument of a specialization.
    GenericSpecializationParam,
    /// A dropped generic argument (`paramN-removed`).
    DroppedArgument,
    /// A vtable override thunk.
    VTableThunk,
    /// The variables a global one-time initializer covers.
    GlobalVariableOnceDeclList,
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
    /// Autodiff function kind; the kind char is the typed autodiff code.
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
    /// A value witness table entry (legacy `_T` mangling); the text is the
    /// witness name and the single child is the witnessed type.
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

/// Global symbol wrapper codes (stored in [`NodeData::Global`]).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
#[repr(u8)]
#[non_exhaustive]
pub enum Global {
    /// `TypeMetadata`.
    TypeMetadata,
    /// `FullTypeMetadata`.
    FullTypeMetadata,
    /// `TypeMetadataPattern`.
    TypeMetadataPattern,
    /// `TypeMetadataAccess`.
    TypeMetadataAccess,
    /// `TypeMetadataInstantiationCache`.
    TypeMetadataInstantiationCache,
    /// `TypeMetadataInstantiationFunction`.
    TypeMetadataInstantiationFunction,
    /// `TypeMetadataSingletonInitCache`.
    TypeMetadataSingletonInitCache,
    /// `TypeMetadataLazyCache`.
    TypeMetadataLazyCache,
    /// `TypeMetadataDemanglingCache`.
    TypeMetadataDemanglingCache,
    /// `TypeMetadataMangledNameRef`.
    TypeMetadataMangledNameRef,
    /// `NominalTypeDescriptor`.
    NominalTypeDescriptor,
    /// `ClassMetaclass`.
    ClassMetaclass,
    /// `ClassMethodLookupFunction`.
    ClassMethodLookupFunction,
    /// `ObjcMetadataUpdate`.
    ObjcMetadataUpdate,
    /// `ObjcResilientClassStub`.
    ObjcResilientClassStub,
    /// `FullObjcResilientClassStub`.
    FullObjcResilientClassStub,
    /// `ClassMetadataBaseOffset`.
    ClassMetadataBaseOffset,
    /// `GenericTypeCompletionFunction`.
    GenericTypeCompletionFunction,
    /// `ModuleDescriptor`.
    ModuleDescriptor,
    /// `ExtensionDescriptor`.
    ExtensionDescriptor,
    /// `AnonymousContextDescriptor`.
    AnonymousContextDescriptor,
    /// `ProtocolDescriptor`.
    ProtocolDescriptor,
    /// `ProtocolDescriptorRecord`.
    ProtocolDescriptorRecord,
    /// `NominalTypeDescriptorRecord`.
    NominalTypeDescriptorRecord,
    /// `ProtocolConformanceDescriptorRecord`.
    ProtocolConformanceDescriptorRecord,
    /// `AccessibleFunctionRecord`.
    AccessibleFunctionRecord,
    /// `ProtocolConformanceDescriptor`.
    ProtocolConformanceDescriptor,
    /// `ProtocolWitnessTable`.
    ProtocolWitnessTable,
    /// `ProtocolWitnessTableAccessor`.
    ProtocolWitnessTableAccessor,
    /// `ProtocolWitnessTablePattern`.
    ProtocolWitnessTablePattern,
    /// `GenericWitnessTableInstantiationFunction`.
    GenericWitnessTableInstantiationFunction,
    /// `GenericProtocolWitnessTable`.
    GenericProtocolWitnessTable,
    /// `ResilientProtocolWitnessTable`.
    ResilientProtocolWitnessTable,
    /// `LazyProtocolWitnessTableCache`.
    LazyProtocolWitnessTableCache,
    /// `LazyProtocolWitnessTableAccessor`.
    LazyProtocolWitnessTableAccessor,
    /// `AssociatedTypeMetadataAccessor`.
    AssociatedTypeMetadataAccessor,
    /// `AssociatedTypeWitnessTableAccessor`.
    AssociatedTypeWitnessTableAccessor,
    /// `BaseProtocolWitnessTableAccessor`.
    BaseProtocolWitnessTableAccessor,
    /// `ProtocolSelfConformanceWitnessTable`.
    ProtocolSelfConformanceWitnessTable,
    /// `ProtocolSelfConformanceDescriptor`.
    ProtocolSelfConformanceDescriptor,
    /// `ValueWitnessTable`.
    ValueWitnessTable,
    /// `FieldOffset`.
    FieldOffset,
    // Produced by no parser yet: the variant only names a printer arm, so the
    // shared vocabulary keeps it until the code is parsed.
    /// `EnumTagIndex`.
    EnumTagIndex,
    /// `OutlinedCopy`.
    OutlinedCopy,
    /// `OutlinedConsume`.
    OutlinedConsume,
    /// `OutlinedRetain`.
    OutlinedRetain,
    /// `OutlinedRelease`.
    OutlinedRelease,
    /// `OutlinedInitWithTake`.
    OutlinedInitWithTake,
    /// `OutlinedInitWithCopy`.
    OutlinedInitWithCopy,
    /// `OutlinedAssignWithTake`.
    OutlinedAssignWithTake,
    /// `OutlinedAssignWithCopy`.
    OutlinedAssignWithCopy,
    /// `OutlinedDestroy`.
    OutlinedDestroy,
    /// `OutlinedEnumTagStore`.
    OutlinedEnumTagStore,
    /// `OutlinedEnumProjectData`.
    OutlinedEnumProjectData,
    /// `OutlinedEnumGetTag`.
    OutlinedEnumGetTag,
    /// `OutlinedVariable`.
    OutlinedVariable,
    /// `OutlinedReadOnlyObject`.
    OutlinedReadOnlyObject,
    /// `OutlinedBridgedMethod`.
    OutlinedBridgedMethod,
    /// `PartialApplyForwarder`.
    PartialApplyForwarder,
    /// `PartialApplyObjcForwarder`.
    PartialApplyObjcForwarder,
    /// `ReabstractionThunkHelper`.
    ReabstractionThunkHelper,
    /// `ReabstractionThunk`.
    ReabstractionThunk,
    /// `DispatchThunk`.
    DispatchThunk,
    /// `MethodDescriptor`.
    MethodDescriptor,
    // Classified by the parser as a function attribute, but no parser builds it
    // yet, so the shared vocabulary keeps it.
    /// `DirectMethodReferenceThunk`.
    DirectMethodReferenceThunk,
    /// `AsyncFunctionPointer`.
    AsyncFunctionPointer,
    /// `MergedFunction`.
    MergedFunction,
    /// `ProtocolWitnessThunk`.
    ProtocolWitnessThunk,
    /// `ProtocolSelfConformanceWitnessThunk`.
    ProtocolSelfConformanceWitnessThunk,
    /// `BackDeploymentCopy`.
    BackDeploymentCopy,
    /// `BackDeploymentThunk`.
    BackDeploymentThunk,
    /// `BackDeploymentFallback`.
    BackDeploymentFallback,
    /// `HasSymbolQuery`.
    HasSymbolQuery,
    /// `ResumePartialFunction`.
    ResumePartialFunction,
    /// `GlobalVariableOnceFunction`.
    GlobalVariableOnceFunction,
    /// `GlobalVariableOnceToken`.
    GlobalVariableOnceToken,
    // Produced by no parser yet: the variant only names a printer arm.
    /// `TypeMetadataCompletionFunction`.
    TypeMetadataCompletionFunction,
    /// `DistributedThunk`.
    DistributedThunk,
    /// `DistributedAccessor`.
    DistributedAccessor,
    /// `DynamicallyReplaceableFunctionVar`.
    DynamicallyReplaceableFunctionVar,
    /// `DynamicallyReplaceableFunctionKey`.
    DynamicallyReplaceableFunctionKey,
    /// `DynamicallyReplaceableFunctionImpl`.
    DynamicallyReplaceableFunctionImpl,
    /// `CoroFunctionPointer`.
    CoroFunctionPointer,
    /// `DefaultOverride`.
    DefaultOverride,
    /// `ReflectionMetadataAssocTypeDescriptor`.
    ReflectionMetadataAssocTypeDescriptor,
    /// `OpaqueTypeDescriptorRecord`.
    OpaqueTypeDescriptorRecord,
    /// `AssociatedTypeDescriptor`.
    AssociatedTypeDescriptor,
    /// `ProtocolRequirementsBaseDescriptor`.
    ProtocolRequirementsBaseDescriptor,
    /// `AssociatedConformanceDescriptor`.
    AssociatedConformanceDescriptor,
    /// `DefaultAssociatedConformanceAccessor`.
    DefaultAssociatedConformanceAccessor,
    /// `BaseConformanceDescriptor`.
    BaseConformanceDescriptor,
    /// `DefaultAssociatedTypeMetadataAccessor`.
    DefaultAssociatedTypeMetadataAccessor,
    /// `PropertyDescriptor`.
    PropertyDescriptor,
    /// `OpaqueTypeDescriptor`.
    OpaqueTypeDescriptor,
}

/// Accessor codes (stored in [`NodeData::Accessor`]).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
#[repr(u8)]
#[non_exhaustive]
pub enum Accessor {
    /// Getter.
    Getter,
    /// `GlobalGetter`.
    GlobalGetter,
    /// Setter.
    Setter,
    /// `MaterializeForSet`.
    MaterializeForSet,
    /// Read.
    Read,
    /// Modify.
    Modify,
    /// `WillSet`.
    WillSet,
    /// `DidSet`.
    DidSet,
    /// Modify2.
    Modify2,
    /// `YieldingBorrow`.
    YieldingBorrow,
    /// Borrow.
    Borrow,
    /// `YieldingModify`.
    YieldingModify,
    /// `MutableAddressor`.
    MutableAddressor,
    /// `NonMutableAddressor`.
    NonMutableAddressor,
    /// Pseudo.
    Pseudo,
    // Accessors only the legacy `_T` grammar emits.
    /// `owningAddressor` (legacy `_T` mangling).
    /// `OwningAddressor`.
    OwningAddressor,
    /// `nativeOwningAddressor` (legacy `_T` mangling).
    /// `NativeOwningAddressor`.
    NativeOwningAddressor,
    /// `nativePinningAddressor` (legacy `_T` mangling).
    /// `NativePinningAddressor`.
    NativePinningAddressor,
    /// `owningMutableAddressor` (legacy `_T` mangling).
    /// `OwningMutableAddressor`.
    OwningMutableAddressor,
    /// `nativeOwningMutableAddressor` (legacy `_T` mangling).
    /// `NativeOwningMutableAddressor`.
    NativeOwningMutableAddressor,
    /// `nativePinningMutableAddressor` (legacy `_T` mangling).
    /// `NativePinningMutableAddressor`.
    NativePinningMutableAddressor,
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
            Accessor::OwningAddressor,
            Accessor::NativeOwningAddressor,
            Accessor::NativePinningAddressor,
            Accessor::OwningMutableAddressor,
            Accessor::NativeOwningMutableAddressor,
            Accessor::NativePinningMutableAddressor,
        ];
        if code == 0 {
            return None;
        }
        ALL.get(usize::from(code) - 1).copied()
    }
}

/// Optional node text.
pub type Text<'a> = Option<Cow<'a, str>>;

/// Default maximum nodes one parse may allocate.
///
/// A [`crate::Demangler`] budget may raise it up to the module's hard node
/// ceiling.
pub const MAX_NODES: usize = 0x8_0000;

/// Default expansion visit allowance of the Swift parsers.
///
/// One shared node counts once per reference, so a small DAG can expand into
/// many printed nodes. A [`crate::Budget`] may lower or raise this within
/// `HARD_EXPANSION_VISITS`.
pub const DEFAULT_EXPANSION_VISITS: usize = 1 << 17;

/// Default rendered-byte estimate of the Swift parsers: 1 MiB.
///
/// The estimate is an upper bound on the bytes the printer writes for an
/// accepted tree, not an exact size. A [`crate::Budget`] may lower or raise it
/// within `HARD_EXPANSION_BYTES`.
pub const DEFAULT_EXPANSION_BYTES: usize = 1 << 20;

/// Hard expansion visit ceiling.
pub const HARD_EXPANSION_VISITS: usize = 16_000_000;

/// Hard rendered-byte estimate ceiling: 64 MiB.
pub const HARD_EXPANSION_BYTES: usize = 64 * 1024 * 1024;

/// Conservative fixed output bytes charged for every node, covering printer
/// labels and separators. The longest literal in the Swift printer is 62 bytes.
const PRINT_NODE_OVERHEAD: usize = 128;

/// Bytes one `IndexSubset` text byte can render as.
///
/// The printer writes one index per `S` byte as `<number>, `, and the parser
/// keeps at most 64 text bytes, so an index is at most two digits.
const INDEX_SUBSET_BYTE_FACTOR: usize = 4;

/// Rendered bytes charged per generic parameter name a
/// `DependentGenericParamCount` prints: a base-26 name of up to seven letters
/// for a `u32` index, up to ten depth digits and a `, ` separator.
const GENERIC_PARAM_BYTES: usize = 19;

/// Typed node payload for the kinds that carry more than text and children.
///
/// One variant per payload family: `kind`, text and children live on [`Node`].
/// Public so callers can inspect payloads; `#[non_exhaustive]` lets the grammar
/// grow without a breaking change.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum NodeData {
    /// Structural nodes and text-bearing kinds.
    None,
    /// Kind-defined counters (`Number`, `Integer`, `FuncSpecKind`, ...).
    Number {
        /// The primary value.
        value: u32,
        /// The secondary value (`Number`'s length field).
        len: u32,
    },
    /// A numbered entity: closure or default-argument initializer index.
    Index(u32),
    /// Global symbol wrapper.
    Global {
        /// The global operator.
        code: Global,
        /// The operator's operand.
        operand: GlobalOperand,
    },
    /// Accessor-coded declaration (`Variable`, `Subscript`, `Accessor`).
    Accessor(Accessor),
    /// `Impl*` kind code; the byte's meaning is selected by `kind`.
    Impl {
        /// Convention/callee/callable code.
        code: u8,
    },
    /// `AttachedMacroExpansion` role.
    MacroRole(MacroRole),
    /// Autodiff code.
    AutoDiff(AutoDiffCode),
    /// Generic specialization (`Tg`/`TG`/`TB`/`Ts`/`Ti`).
    GenericSpecialization {
        /// The `T`-operator form.
        kind: GenericSpecializationKind,
        /// The `r` spec-attribute flag.
        representation_changed: bool,
    },
    /// Function-signature specialization (`Tf`).
    FunctionSignatureSpecialization {
        /// The `r` spec-attribute flag.
        representation_changed: bool,
    },
    /// Function-signature specialization parameter.
    FunctionSpec {
        /// True for the return-value entry.
        is_return: bool,
    },
    /// Key-path thunk flavour.
    KeyPathThunk(KeyPathKind),
    /// Generic layout requirement; size/alignment stay as children.
    Layout(LayoutCode),
    /// Deinitializer form.
    Deinitializer(DeinitializerKind),
    /// Function attribute.
    Attribute(AttributeKind),
}

impl NodeData {
    /// The zero payload a kind carries before a typed builder overrides it.
    ///
    /// This is a direct port of the old kind-to-category mapping; keeping it
    /// makes a missed port visible in output rather than a compile error only
    /// where the kind requires a dedicated builder.
    #[expect(
        clippy::match_same_arms,
        reason = "the explicit kind list mirrors the reference category mapping"
    )]
    const fn default_for(kind: Kind) -> Self {
        match kind {
            Kind::Module
            | Kind::Identifier
            | Kind::BuiltinTypeName
            | Kind::BuiltinVector
            | Kind::TupleElementName
            | Kind::Suffix
            | Kind::InfixOperator
            | Kind::PrefixOperator
            | Kind::PostfixOperator
            | Kind::IndexSubset
            | Kind::ValueWitness => Self::None,
            Kind::Number
            | Kind::Integer
            | Kind::NegativeInteger
            | Kind::DependentGenericParamType
            | Kind::DependentGenericParamCount
            | Kind::FuncSpecNumber
            | Kind::FuncSpecKind
            | Kind::DroppedArgument
            | Kind::OpaqueReturnTypeIndex => Self::Number { value: 0, len: 0 },
            Kind::ImplFunctionType
            | Kind::ImplPatternSubstitutions
            | Kind::ImplInvocationSubstitutions
            | Kind::ImplParameter
            | Kind::ImplResult
            | Kind::ImplYield
            | Kind::ImplErrorResult
            | Kind::ImplConvention
            | Kind::ImplDifferentiability
            | Kind::ImplSending
            | Kind::ImplIsolated
            | Kind::ImplImplicitLeading
            | Kind::ImplEscaping
            | Kind::ImplErasedIsolation
            | Kind::ImplNonisolatedNonsending
            | Kind::ImplCalledOnce
            | Kind::ImplDifferentiabilityKind
            | Kind::ImplCalleeConvention
            | Kind::ImplFunctionConvention
            | Kind::ImplCoroutineKind
            | Kind::ImplSendable
            | Kind::ImplAsync
            | Kind::ImplSendingResult => Self::Impl { code: 0 },
            Kind::AttachedMacroExpansion => Self::MacroRole(MacroRole::Accessor),
            Kind::AutoDiffFunctionKind | Kind::DifferentiableFunctionType => {
                Self::AutoDiff(AutoDiffCode::Forward)
            }
            Kind::GenericSpecialization => Self::GenericSpecialization {
                kind: GenericSpecializationKind::Generic,
                representation_changed: false,
            },
            Kind::FunctionSignatureSpecialization => Self::FunctionSignatureSpecialization {
                representation_changed: false,
            },
            Kind::FuncSpecParam => Self::FunctionSpec { is_return: false },
            Kind::KeyPathThunkHelper => Self::KeyPathThunk(KeyPathKind::Getter),
            Kind::DependentGenericLayoutRequirement => Self::Layout(LayoutCode::UnknownLayout),
            Kind::Deinitializer => Self::Deinitializer(DeinitializerKind::Deinit),
            Kind::Attribute => Self::Attribute(AttributeKind::Objc),
            Kind::ExplicitClosure | Kind::ImplicitClosure | Kind::DefaultArgumentInitializer => {
                Self::Index(0)
            }
            _ => Self::None,
        }
    }
}

/// Operand carried by [`NodeData::Global`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum GlobalOperand {
    /// No operand.
    None,
    /// An index (`outlined variable #N`, `outlined read-only object #N`).
    Index(u32),
    /// `ResumePartialFunction`: index plus suspend/await style.
    IndexAndResume {
        /// The function index.
        index: u32,
        /// The `Y`/`Q` style.
        style: ResumeStyle,
    },
    /// `FieldOffset`: `true` prints `indirect`, `false` prints `direct`.
    Directness(bool),
}

/// Resume style of a partial function (`Y` suspend, `Q` await).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ResumeStyle {
    /// `Y`, printed `suspend`.
    Suspend,
    /// `Q`, printed `await`.
    Await,
}

/// Deinitializer form (`Deinitializer` code `0`, `1`, `2`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum DeinitializerKind {
    /// `0`: `.deinit`.
    Deinit,
    /// `1`: `.__deallocating_deinit`.
    Deallocating,
    /// `2`: `.__isolated_deallocating_deinit`.
    IsolatedDeallocating,
}

/// Function attribute code (`Attribute`, `0`..=`4`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum AttributeKind {
    /// `0`: `@objc`.
    Objc,
    /// `1`: `@nonobjc`.
    NonObjc,
    /// `2`: `dynamic`.
    Dynamic,
    /// `3`: `super`.
    Super,
    /// `4`: `override`.
    Override,
}

/// Role of an attached macro expansion (`a`, `r`, `m`, `p`, `c`, `e`, `q`, `b`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum MacroRole {
    /// `a`: `accessor`.
    Accessor,
    /// `r`: `memberAttribute`.
    MemberAttribute,
    /// `m`: `member`.
    Member,
    /// `p`: `peer`.
    Peer,
    /// `c`: `conformance`.
    Conformance,
    /// `e`: `extension`.
    Extension,
    /// `q`: `preamble`.
    Preamble,
    /// `b`: `body`.
    Body,
}

impl MacroRole {
    /// Decode the mangling byte.
    pub(crate) const fn from_byte(byte: u8) -> Option<Self> {
        Some(match byte {
            b'a' => Self::Accessor,
            b'r' => Self::MemberAttribute,
            b'm' => Self::Member,
            b'p' => Self::Peer,
            b'c' => Self::Conformance,
            b'e' => Self::Extension,
            b'q' => Self::Preamble,
            b'b' => Self::Body,
            _ => return None,
        })
    }

    /// The role name printed after the attached name.
    pub(crate) const fn name(self) -> &'static str {
        match self {
            Self::Accessor => "accessor",
            Self::MemberAttribute => "memberAttribute",
            Self::Member => "member",
            Self::Peer => "peer",
            Self::Conformance => "conformance",
            Self::Extension => "extension",
            Self::Preamble => "preamble",
            Self::Body => "body",
        }
    }
}

/// Autodiff code (`f`, `r`, `d`, `l`, `p`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum AutoDiffCode {
    /// `f`: forward-mode derivative.
    Forward,
    /// `r`: reverse-mode derivative.
    Reverse,
    /// `d`: differential / `@differentiable`.
    Differential,
    /// `l`: `@differentiable(_linear)`.
    Linear,
    /// `p`: pullback (`AutoDiffFunctionKind` only).
    Pullback,
}

impl AutoDiffCode {
    /// Decode the mangling byte; the caller enforces the per-kind domain.
    pub(crate) const fn from_byte(byte: u8) -> Option<Self> {
        Some(match byte {
            b'f' => Self::Forward,
            b'r' => Self::Reverse,
            b'd' => Self::Differential,
            b'l' => Self::Linear,
            b'p' => Self::Pullback,
            _ => return None,
        })
    }
}

/// `GenericSpecialization` kind (`g`, `G`, `B`, `s`, `i`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum GenericSpecializationKind {
    /// `g`: generic specialization.
    Generic,
    /// `G`: generic not re-abstracted specialization.
    NotReabstracted,
    /// `B`: generic specialization in the resilience domain.
    ResilienceDomain,
    /// `s`: generic pre-specialization.
    Prespecialized,
    /// `i`: inlined generic function.
    Inlined,
}

/// `KeyPathThunkHelper` flavour.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum KeyPathKind {
    /// `0`: key path getter.
    Getter,
    /// `1`: key path setter.
    Setter,
    /// `2`: key path unapplied method.
    UnappliedMethod,
    /// `4`: key path applied method.
    AppliedMethod,
}

/// `LAYOUT-CONSTRAINT` letter (`refs/swift/Mangling.rst:1149-1160`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum LayoutCode {
    /// `U`: `_UnknownLayout`.
    UnknownLayout,
    /// `R`: `_RefCountedObject`.
    RefCountedObject,
    /// `N`: `_NativeRefCountedObject`.
    NativeRefCountedObject,
    /// `C`: `AnyObject`.
    AnyObject,
    /// `D`: `_NativeClass`.
    NativeClass,
    /// `T`: `_Trivial`.
    Trivial,
    /// `B`: accepts, prints nothing (upstream has no name).
    BridgeObject,
    /// `E`: `_Trivial` with size and alignment.
    TrivialExactSizeAndAlignment,
    /// `e`: `_Trivial` with size.
    TrivialExactSize,
    /// `M`: `_TrivialAtMost` with size and alignment.
    TrivialAtMostSizeAndAlignment,
    /// `m`: `_TrivialAtMost` with size.
    TrivialAtMostSize,
    /// `S`: accepts, consumes a size, prints nothing.
    TrivialStride,
}

impl LayoutCode {
    /// Decode the layout letter; unknown bytes are rejected.
    pub(crate) const fn from_byte(byte: u8) -> Option<Self> {
        Some(match byte {
            b'U' => Self::UnknownLayout,
            b'R' => Self::RefCountedObject,
            b'N' => Self::NativeRefCountedObject,
            b'C' => Self::AnyObject,
            b'D' => Self::NativeClass,
            b'T' => Self::Trivial,
            b'B' => Self::BridgeObject,
            b'E' => Self::TrivialExactSizeAndAlignment,
            b'e' => Self::TrivialExactSize,
            b'M' => Self::TrivialAtMostSizeAndAlignment,
            b'm' => Self::TrivialAtMostSize,
            b'S' => Self::TrivialStride,
            _ => return None,
        })
    }

    /// True when a size index follows.
    pub(crate) const fn takes_size(self) -> bool {
        matches!(
            self,
            Self::TrivialExactSizeAndAlignment
                | Self::TrivialExactSize
                | Self::TrivialAtMostSizeAndAlignment
                | Self::TrivialAtMostSize
                | Self::TrivialStride
        )
    }

    /// True when an alignment index follows the size.
    pub(crate) const fn takes_alignment(self) -> bool {
        matches!(
            self,
            Self::TrivialExactSizeAndAlignment | Self::TrivialAtMostSizeAndAlignment
        )
    }

    /// The printed name, or `None` for the upstream-unhandled `B`/`S`.
    pub(crate) const fn name(self) -> Option<&'static str> {
        Some(match self {
            Self::UnknownLayout => "_UnknownLayout",
            Self::RefCountedObject => "_RefCountedObject",
            Self::NativeRefCountedObject => "_NativeRefCountedObject",
            Self::AnyObject => "AnyObject",
            Self::NativeClass => "_NativeClass",
            Self::Trivial | Self::TrivialExactSizeAndAlignment | Self::TrivialExactSize => {
                "_Trivial"
            }
            Self::TrivialAtMostSizeAndAlignment | Self::TrivialAtMostSize => "_TrivialAtMost",
            Self::BridgeObject | Self::TrivialStride => return None,
        })
    }
}

/// One immutable AST node.
pub struct Node<'a> {
    kind: Kind,
    text: Text<'a>,
    data: NodeData,
    children: Box<[NodeRef<'a>]>,
    /// Expanded printer visits for this node, counting a shared child once per
    /// reference (saturating).
    print_visits: usize,
    /// Structural printer depth for this node (saturating).
    print_depth: u16,
    /// Conservative expanded output bytes for this node (saturating).
    print_bytes: usize,
}

impl<'a> Node<'a> {
    /// The node kind.
    #[must_use]
    pub const fn kind(&self) -> Kind {
        self.kind
    }

    /// The node's text, if it has any.
    #[must_use]
    pub fn text(&self) -> Option<&str> {
        self.text.as_deref()
    }

    /// Clone the text handle.
    pub(crate) fn text_cow(&self) -> Text<'a> {
        self.text.clone()
    }

    /// The node's typed payload.
    #[must_use]
    pub const fn data(&self) -> &NodeData {
        &self.data
    }

    /// The node's children in order.
    #[must_use]
    pub fn children(&self) -> &[NodeRef<'a>] {
        &self.children
    }

    /// The child at `index`, if present.
    #[must_use]
    pub fn child(&self, index: usize) -> Option<NodeRef<'a>> {
        self.children.get(index).cloned()
    }

    /// The number of children.
    #[must_use]
    pub fn child_count(&self) -> usize {
        self.children.len()
    }

    /// True when the node has no children.
    #[must_use]
    pub fn is_leaf(&self) -> bool {
        self.children.is_empty()
    }

    /// Expanded printer visits, counting a shared child once per reference.
    #[must_use]
    pub(crate) const fn print_visits(&self) -> usize {
        self.print_visits
    }

    /// Structural printer depth.
    #[must_use]
    pub(crate) const fn print_depth(&self) -> u16 {
        self.print_depth
    }

    /// Conservative expanded output bytes.
    #[must_use]
    pub(crate) const fn print_bytes(&self) -> usize {
        self.print_bytes
    }

    /// The primary number value, or zero.
    pub(crate) const fn number_value(&self) -> u32 {
        match self.data {
            NodeData::Number { value, .. } => value,
            _ => 0,
        }
    }

    /// The secondary number value, or zero.
    pub(crate) const fn number_len(&self) -> u32 {
        match self.data {
            NodeData::Number { len, .. } => len,
            _ => 0,
        }
    }

    /// The closure/default-argument index, or zero.
    pub(crate) const fn entity_index(&self) -> u32 {
        match self.data {
            NodeData::Index(index) => index,
            _ => 0,
        }
    }

    /// The global operator, if this is a global wrapper.
    pub(crate) const fn global_code(&self) -> Option<Global> {
        match self.data {
            NodeData::Global { code, .. } => Some(code),
            _ => None,
        }
    }

    /// The global operand.
    pub(crate) const fn global_operand(&self) -> GlobalOperand {
        match self.data {
            NodeData::Global { operand, .. } => operand,
            _ => GlobalOperand::None,
        }
    }

    /// The accessor code, if any.
    pub(crate) const fn accessor_code(&self) -> Option<Accessor> {
        match self.data {
            NodeData::Accessor(code) => Some(code),
            _ => None,
        }
    }

    /// The kind-scoped impl code, or zero.
    pub(crate) const fn impl_flag(&self) -> u8 {
        match self.data {
            NodeData::Impl { code } => code,
            _ => 0,
        }
    }

    /// The macro role, if any.
    pub(crate) const fn macro_role(&self) -> Option<MacroRole> {
        match self.data {
            NodeData::MacroRole(role) => Some(role),
            _ => None,
        }
    }

    /// The autodiff code.
    pub(crate) const fn autodiff_code(&self) -> AutoDiffCode {
        match self.data {
            NodeData::AutoDiff(code) => code,
            _ => AutoDiffCode::Forward,
        }
    }

    /// The generic-specialization kind and representation-changed flag.
    pub(crate) const fn generic_specialization(&self) -> Option<(GenericSpecializationKind, bool)> {
        match self.data {
            NodeData::GenericSpecialization {
                kind,
                representation_changed,
            } => Some((kind, representation_changed)),
            _ => None,
        }
    }

    /// The function-signature specialization representation-changed flag.
    pub(crate) const fn function_signature_representation_changed(&self) -> bool {
        match self.data {
            NodeData::FunctionSignatureSpecialization {
                representation_changed,
            } => representation_changed,
            _ => false,
        }
    }

    /// True for the return-value entry of a function specialization.
    pub(crate) const fn function_spec_is_return(&self) -> bool {
        match self.data {
            NodeData::FunctionSpec { is_return } => is_return,
            _ => false,
        }
    }

    /// The key-path thunk flavour.
    pub(crate) const fn keypath_kind(&self) -> KeyPathKind {
        match self.data {
            NodeData::KeyPathThunk(kind) => kind,
            _ => KeyPathKind::Getter,
        }
    }

    /// The layout constraint.
    pub(crate) const fn layout_code(&self) -> LayoutCode {
        match self.data {
            NodeData::Layout(code) => code,
            _ => LayoutCode::UnknownLayout,
        }
    }

    /// The deinitializer form.
    pub(crate) const fn deinitializer_kind(&self) -> DeinitializerKind {
        match self.data {
            NodeData::Deinitializer(kind) => kind,
            _ => DeinitializerKind::Deinit,
        }
    }

    /// The function attribute.
    pub(crate) const fn attribute_kind(&self) -> AttributeKind {
        match self.data {
            NodeData::Attribute(kind) => kind,
            _ => AttributeKind::Objc,
        }
    }

    /// Compare this node's text with `expected`.
    pub(crate) fn text_eq(&self, expected: &str) -> bool {
        self.text() == Some(expected)
    }

    /// Write this node's text into `out`.
    pub(crate) fn write_text<W: core::fmt::Write + ?Sized>(
        &self,
        out: &mut W,
    ) -> core::fmt::Result {
        if let Some(text) = self.text() {
            out.write_str(text)?;
        }
        Ok(())
    }
}

impl core::fmt::Debug for Node<'_> {
    /// Print the node without walking the graph, so a shared DAG cannot make
    /// `Debug` unbounded.
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Node")
            .field("kind", &self.kind)
            .field("text", &self.text)
            .field("child_count", &self.children.len())
            .finish()
    }
}

/// Shared, reference-counted node handle.
pub type NodeRef<'a> = Rc<Node<'a>>;

/// One node under construction.
pub struct Builder<'a> {
    kind: Kind,
    text: Text<'a>,
    data: NodeData,
    children: Vec<NodeRef<'a>>,
}

impl<'a> Builder<'a> {
    /// Append a child.
    pub(crate) fn push(&mut self, child: NodeRef<'a>) {
        self.children.push(child);
    }

    /// Append a child when present.
    pub(crate) fn push_opt(&mut self, child: Option<NodeRef<'a>>) {
        if let Some(child) = child {
            self.children.push(child);
        }
    }

    /// Replace the payload.
    pub(crate) const fn set_data(&mut self, data: NodeData) {
        self.data = data;
    }

    /// Replace the text.
    pub(crate) fn set_text(&mut self, text: Text<'a>) {
        self.text = text;
    }

    /// Set the primary number value.
    pub(crate) const fn set_value(&mut self, value: u32) {
        if let NodeData::Number { value: current, .. } = &mut self.data {
            *current = value;
        }
    }

    /// Set the secondary number value.
    pub(crate) const fn set_len(&mut self, len: u32) {
        if let NodeData::Number { len: current, .. } = &mut self.data {
            *current = len;
        }
    }

    /// Seal the child list and wrap the node.
    pub(crate) fn build(self) -> NodeRef<'a> {
        let text_len = self.text.as_deref().map_or(0, str::len);
        let (mut print_visits, print_depth, mut print_bytes) =
            envelope(self.kind, text_len, &self.children);
        // A parameter count prints one generic parameter name per counted
        // parameter, so its output is charged here rather than capped in the
        // printer.
        if self.kind == Kind::DependentGenericParamCount
            && let NodeData::Number { value, .. } = self.data
        {
            let names = usize::try_from(value).unwrap_or(usize::MAX);
            print_visits = print_visits.saturating_add(names);
            print_bytes = print_bytes.saturating_add(names.saturating_mul(GENERIC_PARAM_BYTES));
        }
        Rc::new(Node {
            kind: self.kind,
            text: self.text,
            data: self.data,
            children: self.children.into_boxed_slice(),
            print_visits,
            print_depth,
            print_bytes,
        })
    }
}

/// Conservative printer envelope: expanded visits, structural depth and bytes.
///
/// Visits count a shared child once per reference and depth is the structural
/// nesting; both are exact. Bytes are an upper bound: the node's own text is
/// charged at the factor its printer expands it by, every kind contributes a
/// fixed [`PRINT_NODE_OVERHEAD`] charge for its labels and separators, and the
/// children add their own charges.
fn envelope(kind: Kind, text_len: usize, children: &[NodeRef<'_>]) -> (usize, u16, usize) {
    let mut visits = 1_usize;
    let mut depth = 1_u16;
    let mut bytes = text_bytes(kind, text_len).saturating_add(PRINT_NODE_OVERHEAD);
    for child in children {
        visits = visits.saturating_add(child.print_visits());
        depth = depth.max(child.print_depth().saturating_add(1));
        bytes = bytes.saturating_add(child.print_bytes());
    }
    (visits, depth, bytes)
}

/// Rendered bytes one node's text can expand to.
///
/// Text is escaped when the parser stores it, so it normally prints verbatim.
/// An `IndexSubset` is the one kind whose printer writes more than its text:
/// one index per `S` byte.
const fn text_bytes(kind: Kind, text_len: usize) -> usize {
    let factor = match kind {
        Kind::IndexSubset => INDEX_SUBSET_BYTE_FACTOR,
        _ => 1,
    };
    text_len.saturating_mul(factor)
}

/// Parsing context: substitutions, word substitutions and the node budget.
pub struct Context<'a> {
    subs: Vec<NodeRef<'a>>,
    words: Vec<&'a [u8]>,
    nodes: usize,
    node_budget: usize,
    failed: bool,
}

impl<'a> Context<'a> {
    /// Create an empty context with no nodes charged and a node budget.
    pub(crate) const fn new(node_budget: usize) -> Self {
        Self {
            subs: Vec::new(),
            words: Vec::new(),
            nodes: 0,
            node_budget,
            failed: false,
        }
    }

    /// Whether the node budget was exceeded.
    pub(crate) const fn failed(&self) -> bool {
        self.failed
    }

    const fn charge(&mut self) {
        self.charge_many(1);
    }

    /// Charge `count` nodes built elsewhere, such as by a nested payload parse.
    pub(crate) const fn charge_many(&mut self, count: usize) {
        self.nodes = self.nodes.saturating_add(count);
        if self.nodes > self.node_budget {
            self.failed = true;
        }
    }

    /// Nodes charged so far.
    pub(crate) const fn used(&self) -> usize {
        self.nodes
    }

    /// Nodes this parse may still allocate.
    pub(crate) const fn remaining(&self) -> usize {
        self.node_budget.saturating_sub(self.nodes)
    }

    const fn builder_with(&mut self, kind: Kind, text: Text<'a>, data: NodeData) -> Builder<'a> {
        self.charge();
        Builder {
            kind,
            text,
            data,
            children: Vec::new(),
        }
    }

    /// A structural builder with the kind's default payload.
    pub(crate) const fn builder(&mut self, kind: Kind) -> Builder<'a> {
        self.builder_with(kind, None, NodeData::default_for(kind))
    }

    /// A text-bearing builder.
    pub(crate) const fn builder_text(&mut self, kind: Kind, text: &'a str) -> Builder<'a> {
        self.builder_with(kind, Some(Cow::Borrowed(text)), NodeData::default_for(kind))
    }

    /// A text-bearing builder from either borrowed or escaped text.
    pub(crate) const fn builder_text_cow(&mut self, kind: Kind, text: Cow<'a, str>) -> Builder<'a> {
        self.builder_with(kind, Some(text), NodeData::default_for(kind))
    }

    /// A text-bearing builder with reconstructed text.
    pub(crate) fn builder_owned(&mut self, kind: Kind, text: &str) -> Builder<'a> {
        self.builder_with(
            kind,
            Some(Cow::Owned(text.into())),
            NodeData::default_for(kind),
        )
    }

    /// A numeric builder.
    pub(crate) const fn builder_value(&mut self, kind: Kind, value: u32) -> Builder<'a> {
        self.builder_with(kind, None, NodeData::Number { value, len: 0 })
    }

    /// An indexed-entity builder.
    pub(crate) const fn builder_index(&mut self, kind: Kind, index: u32) -> Builder<'a> {
        self.builder_with(kind, None, NodeData::Index(index))
    }

    /// A global wrapper builder.
    pub(crate) const fn builder_global(
        &mut self,
        code: Global,
        operand: GlobalOperand,
    ) -> Builder<'a> {
        self.builder_with(Kind::Global, None, NodeData::Global { code, operand })
    }

    /// An accessor-coded builder.
    pub(crate) const fn builder_accessor(&mut self, kind: Kind, code: Accessor) -> Builder<'a> {
        self.builder_with(kind, None, NodeData::Accessor(code))
    }

    /// An impl-code builder.
    pub(crate) const fn builder_impl(&mut self, kind: Kind, code: u8) -> Builder<'a> {
        self.builder_with(kind, None, NodeData::Impl { code })
    }

    /// A macro-role builder.
    pub(crate) const fn builder_macro_role(&mut self, kind: Kind, role: MacroRole) -> Builder<'a> {
        self.builder_with(kind, None, NodeData::MacroRole(role))
    }

    /// An autodiff builder.
    pub(crate) const fn builder_autodiff(&mut self, kind: Kind, code: AutoDiffCode) -> Builder<'a> {
        self.builder_with(kind, None, NodeData::AutoDiff(code))
    }

    /// A generic-specialization builder.
    pub(crate) const fn builder_generic_specialization(
        &mut self,
        kind: GenericSpecializationKind,
        representation_changed: bool,
    ) -> Builder<'a> {
        self.builder_with(
            Kind::GenericSpecialization,
            None,
            NodeData::GenericSpecialization {
                kind,
                representation_changed,
            },
        )
    }

    /// A function-signature specialization builder.
    pub(crate) const fn builder_function_signature_specialization(
        &mut self,
        representation_changed: bool,
    ) -> Builder<'a> {
        self.builder_with(
            Kind::FunctionSignatureSpecialization,
            None,
            NodeData::FunctionSignatureSpecialization {
                representation_changed,
            },
        )
    }

    /// A function-spec parameter builder.
    pub(crate) const fn builder_function_spec(&mut self, is_return: bool) -> Builder<'a> {
        self.builder_with(
            Kind::FuncSpecParam,
            None,
            NodeData::FunctionSpec { is_return },
        )
    }

    /// A key-path thunk builder.
    pub(crate) const fn builder_keypath(&mut self, kind: KeyPathKind) -> Builder<'a> {
        self.builder_with(Kind::KeyPathThunkHelper, None, NodeData::KeyPathThunk(kind))
    }

    /// A layout requirement builder.
    pub(crate) fn builder_layout(&mut self, code: u8) -> Option<Builder<'a>> {
        let code = LayoutCode::from_byte(code)?;
        Some(self.builder_with(
            Kind::DependentGenericLayoutRequirement,
            None,
            NodeData::Layout(code),
        ))
    }

    /// A deinitializer builder.
    pub(crate) const fn builder_deinit(&mut self, kind: DeinitializerKind) -> Builder<'a> {
        self.builder_with(Kind::Deinitializer, None, NodeData::Deinitializer(kind))
    }

    /// A function-attribute builder.
    pub(crate) const fn builder_attribute(&mut self, kind: AttributeKind) -> Builder<'a> {
        self.builder_with(Kind::Attribute, None, NodeData::Attribute(kind))
    }

    /// Copy `id` with a different kind, preserving payload and children.
    ///
    /// The copy is a new node, so it is charged to the node budget.
    pub(crate) fn relabel(&mut self, id: &NodeRef<'a>, kind: Kind) -> NodeRef<'a> {
        self.charge();
        let text_len = id.text().map_or(0, str::len);
        let children = id.children().to_vec();
        let (print_visits, print_depth, print_bytes) = envelope(kind, text_len, &children);
        Rc::new(Node {
            kind,
            text: id.text_cow(),
            data: *id.data(),
            children: children.into_boxed_slice(),
            print_visits,
            print_depth,
            print_bytes,
        })
    }

    /// Copy `id` with a different kind and a replacement child list.
    pub(crate) fn rebuild_with_children(
        &mut self,
        id: &NodeRef<'a>,
        kind: Kind,
        children: Vec<NodeRef<'a>>,
    ) -> NodeRef<'a> {
        self.charge();
        let text_len = id.text().map_or(0, str::len);
        let (print_visits, print_depth, print_bytes) = envelope(kind, text_len, &children);
        Rc::new(Node {
            kind,
            text: id.text_cow(),
            data: *id.data(),
            children: children.into_boxed_slice(),
            print_visits,
            print_depth,
            print_bytes,
        })
    }

    /// Copy `id` with one more child; used only where the reference mutates a
    /// node that may already be shared.
    ///
    /// The copy is a new node, so it is charged to the node budget.
    pub(crate) fn append(&mut self, id: &NodeRef<'a>, child: NodeRef<'a>) -> NodeRef<'a> {
        self.charge();
        let mut children = Vec::with_capacity(id.child_count() + 1);
        children.extend(id.children().iter().cloned());
        children.push(child);
        let text_len = id.text().map_or(0, str::len);
        let (print_visits, print_depth, print_bytes) = envelope(id.kind(), text_len, &children);
        Rc::new(Node {
            kind: id.kind(),
            text: id.text_cow(),
            data: *id.data(),
            children: children.into_boxed_slice(),
            print_visits,
            print_depth,
            print_bytes,
        })
    }

    /// Push a substitution.
    pub(crate) fn add_sub(&mut self, id: NodeRef<'a>) {
        self.subs.push(id);
    }

    /// Fetch a substitution.
    pub(crate) fn sub(&self, index: usize) -> Option<NodeRef<'a>> {
        self.subs.get(index).cloned()
    }

    /// Remember a word.
    pub(crate) fn add_word(&mut self, word: &'a [u8]) {
        self.words.push(word);
    }

    /// Word by index.
    pub(crate) fn word(&self, index: usize) -> &'a [u8] {
        self.words.get(index).copied().unwrap_or(&[])
    }

    /// Number of words.
    pub(crate) const fn word_count(&self) -> usize {
        self.words.len()
    }
}

impl core::fmt::Debug for Context<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Context")
            .field("subs", &self.subs.len())
            .field("words", &self.words.len())
            .finish()
    }
}

/// Map an accessor letter to its [`Accessor`] code.
pub const fn accessor_code(byte: u8) -> u8 {
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
mod tests;
