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
    /// `unowned T` (legacy `_T` mangling).
    Unowned,
    /// `weak T` (legacy `_T` mangling).
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
    /// TypeMetadata.
    TypeMetadata,
    /// FullTypeMetadata.
    FullTypeMetadata,
    /// TypeMetadataPattern.
    TypeMetadataPattern,
    /// TypeMetadataAccess.
    TypeMetadataAccess,
    /// TypeMetadataInstantiationCache.
    TypeMetadataInstantiationCache,
    /// TypeMetadataInstantiationFunction.
    TypeMetadataInstantiationFunction,
    /// TypeMetadataSingletonInitCache.
    TypeMetadataSingletonInitCache,
    /// TypeMetadataLazyCache.
    TypeMetadataLazyCache,
    /// TypeMetadataDemanglingCache.
    TypeMetadataDemanglingCache,
    /// TypeMetadataMangledNameRef.
    TypeMetadataMangledNameRef,
    /// NominalTypeDescriptor.
    NominalTypeDescriptor,
    /// ClassMetaclass.
    ClassMetaclass,
    /// ClassMethodLookupFunction.
    ClassMethodLookupFunction,
    /// ObjcMetadataUpdate.
    ObjcMetadataUpdate,
    /// ObjcResilientClassStub.
    ObjcResilientClassStub,
    /// FullObjcResilientClassStub.
    FullObjcResilientClassStub,
    /// ClassMetadataBaseOffset.
    ClassMetadataBaseOffset,
    /// GenericTypeCompletionFunction.
    GenericTypeCompletionFunction,
    /// ModuleDescriptor.
    ModuleDescriptor,
    /// ExtensionDescriptor.
    ExtensionDescriptor,
    /// AnonymousContextDescriptor.
    AnonymousContextDescriptor,
    /// ProtocolDescriptor.
    ProtocolDescriptor,
    /// ProtocolDescriptorRecord.
    ProtocolDescriptorRecord,
    /// NominalTypeDescriptorRecord.
    NominalTypeDescriptorRecord,
    /// ProtocolConformanceDescriptorRecord.
    ProtocolConformanceDescriptorRecord,
    /// AccessibleFunctionRecord.
    AccessibleFunctionRecord,
    /// ProtocolConformanceDescriptor.
    ProtocolConformanceDescriptor,
    /// ProtocolWitnessTable.
    ProtocolWitnessTable,
    /// ProtocolWitnessTableAccessor.
    ProtocolWitnessTableAccessor,
    /// ProtocolWitnessTablePattern.
    ProtocolWitnessTablePattern,
    /// GenericWitnessTableInstantiationFunction.
    GenericWitnessTableInstantiationFunction,
    /// GenericProtocolWitnessTable.
    GenericProtocolWitnessTable,
    /// ResilientProtocolWitnessTable.
    ResilientProtocolWitnessTable,
    /// LazyProtocolWitnessTableCache.
    LazyProtocolWitnessTableCache,
    /// LazyProtocolWitnessTableAccessor.
    LazyProtocolWitnessTableAccessor,
    /// AssociatedTypeMetadataAccessor.
    AssociatedTypeMetadataAccessor,
    /// AssociatedTypeWitnessTableAccessor.
    AssociatedTypeWitnessTableAccessor,
    /// BaseProtocolWitnessTableAccessor.
    BaseProtocolWitnessTableAccessor,
    /// ProtocolSelfConformanceWitnessTable.
    ProtocolSelfConformanceWitnessTable,
    /// ProtocolSelfConformanceDescriptor.
    ProtocolSelfConformanceDescriptor,
    /// ValueWitnessTable.
    ValueWitnessTable,
    /// FieldOffset.
    FieldOffset,
    // Produced by no parser yet: the variant only names a printer arm, so the
    // shared vocabulary keeps it until the code is parsed.
    #[allow(dead_code)]
    /// EnumTagIndex.
    EnumTagIndex,
    /// OutlinedCopy.
    OutlinedCopy,
    /// OutlinedConsume.
    OutlinedConsume,
    /// OutlinedRetain.
    OutlinedRetain,
    /// OutlinedRelease.
    OutlinedRelease,
    /// OutlinedInitWithTake.
    OutlinedInitWithTake,
    /// OutlinedInitWithCopy.
    OutlinedInitWithCopy,
    /// OutlinedAssignWithTake.
    OutlinedAssignWithTake,
    /// OutlinedAssignWithCopy.
    OutlinedAssignWithCopy,
    /// OutlinedDestroy.
    OutlinedDestroy,
    /// OutlinedEnumTagStore.
    OutlinedEnumTagStore,
    /// OutlinedEnumProjectData.
    OutlinedEnumProjectData,
    /// OutlinedEnumGetTag.
    OutlinedEnumGetTag,
    /// OutlinedVariable.
    OutlinedVariable,
    /// OutlinedReadOnlyObject.
    OutlinedReadOnlyObject,
    /// OutlinedBridgedMethod.
    OutlinedBridgedMethod,
    /// PartialApplyForwarder.
    PartialApplyForwarder,
    /// PartialApplyObjcForwarder.
    PartialApplyObjcForwarder,
    /// ReabstractionThunkHelper.
    ReabstractionThunkHelper,
    /// ReabstractionThunk.
    ReabstractionThunk,
    /// DispatchThunk.
    DispatchThunk,
    /// MethodDescriptor.
    MethodDescriptor,
    // Classified by the parser as a function attribute, but no parser builds it
    // yet, so the shared vocabulary keeps it.
    #[allow(dead_code)]
    /// DirectMethodReferenceThunk.
    DirectMethodReferenceThunk,
    /// AsyncFunctionPointer.
    AsyncFunctionPointer,
    /// MergedFunction.
    MergedFunction,
    /// ProtocolWitnessThunk.
    ProtocolWitnessThunk,
    /// ProtocolSelfConformanceWitnessThunk.
    ProtocolSelfConformanceWitnessThunk,
    /// BackDeploymentCopy.
    BackDeploymentCopy,
    /// BackDeploymentThunk.
    BackDeploymentThunk,
    /// BackDeploymentFallback.
    BackDeploymentFallback,
    /// HasSymbolQuery.
    HasSymbolQuery,
    /// ResumePartialFunction.
    ResumePartialFunction,
    /// GlobalVariableOnceFunction.
    GlobalVariableOnceFunction,
    /// GlobalVariableOnceToken.
    GlobalVariableOnceToken,
    // Produced by no parser yet: the variant only names a printer arm.
    #[allow(dead_code)]
    /// TypeMetadataCompletionFunction.
    TypeMetadataCompletionFunction,
    /// DistributedThunk.
    DistributedThunk,
    /// DistributedAccessor.
    DistributedAccessor,
    /// DynamicallyReplaceableFunctionVar.
    DynamicallyReplaceableFunctionVar,
    /// DynamicallyReplaceableFunctionKey.
    DynamicallyReplaceableFunctionKey,
    /// DynamicallyReplaceableFunctionImpl.
    DynamicallyReplaceableFunctionImpl,
    /// CoroFunctionPointer.
    CoroFunctionPointer,
    /// DefaultOverride.
    DefaultOverride,
    /// ReflectionMetadataAssocTypeDescriptor.
    ReflectionMetadataAssocTypeDescriptor,
    /// OpaqueTypeDescriptorRecord.
    OpaqueTypeDescriptorRecord,
    /// AssociatedTypeDescriptor.
    AssociatedTypeDescriptor,
    /// ProtocolRequirementsBaseDescriptor.
    ProtocolRequirementsBaseDescriptor,
    /// AssociatedConformanceDescriptor.
    AssociatedConformanceDescriptor,
    /// DefaultAssociatedConformanceAccessor.
    DefaultAssociatedConformanceAccessor,
    /// BaseConformanceDescriptor.
    BaseConformanceDescriptor,
    /// DefaultAssociatedTypeMetadataAccessor.
    DefaultAssociatedTypeMetadataAccessor,
    /// PropertyDescriptor.
    PropertyDescriptor,
    /// OpaqueTypeDescriptor.
    OpaqueTypeDescriptor,
}

/// Accessor codes (stored in [`NodeData::Accessor`]).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
#[repr(u8)]
#[non_exhaustive]
pub enum Accessor {
    /// Getter.
    Getter,
    /// GlobalGetter.
    GlobalGetter,
    /// Setter.
    Setter,
    /// MaterializeForSet.
    MaterializeForSet,
    /// Read.
    Read,
    /// Modify.
    Modify,
    /// WillSet.
    WillSet,
    /// DidSet.
    DidSet,
    /// Modify2.
    Modify2,
    /// YieldingBorrow.
    YieldingBorrow,
    /// Borrow.
    Borrow,
    /// YieldingModify.
    YieldingModify,
    /// MutableAddressor.
    MutableAddressor,
    /// NonMutableAddressor.
    NonMutableAddressor,
    /// Pseudo.
    Pseudo,
    // Accessors only the legacy `_T` grammar emits.
    /// `owningAddressor` (legacy `_T` mangling).
    /// OwningAddressor.
    OwningAddressor,
    /// `nativeOwningAddressor` (legacy `_T` mangling).
    /// NativeOwningAddressor.
    NativeOwningAddressor,
    /// `nativePinningAddressor` (legacy `_T` mangling).
    /// NativePinningAddressor.
    NativePinningAddressor,
    /// `owningMutableAddressor` (legacy `_T` mangling).
    /// OwningMutableAddressor.
    OwningMutableAddressor,
    /// `nativeOwningMutableAddressor` (legacy `_T` mangling).
    /// NativeOwningMutableAddressor.
    NativeOwningMutableAddressor,
    /// `nativePinningMutableAddressor` (legacy `_T` mangling).
    /// NativePinningMutableAddressor.
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
pub(crate) type Text<'a> = Option<Cow<'a, str>>;

/// Maximum nodes one parse may allocate.
pub(crate) const MAX_NODES: usize = 0x8_0000;

/// Maximum generic-signature depth groups the parser and printer support.
pub(crate) const MAX_GENERIC_DEPTHS: usize = 8;

/// Maximum generic-signature requirements the parser and printer support.
pub(crate) const MAX_GENERIC_REQUIREMENTS: usize = 16;

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
    fn default_for(kind: Kind) -> Self {
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
            Kind::Variable | Kind::Subscript | Kind::Accessor => Self::None,
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

    /// The primary number value, or zero.
    pub(crate) fn number_value(&self) -> u32 {
        match self.data {
            NodeData::Number { value, .. } => value,
            _ => 0,
        }
    }

    /// The secondary number value, or zero.
    pub(crate) fn number_len(&self) -> u32 {
        match self.data {
            NodeData::Number { len, .. } => len,
            _ => 0,
        }
    }

    /// The closure/default-argument index, or zero.
    pub(crate) fn entity_index(&self) -> u32 {
        match self.data {
            NodeData::Index(index) => index,
            _ => 0,
        }
    }

    /// The global operator, if this is a global wrapper.
    pub(crate) fn global_code(&self) -> Option<Global> {
        match self.data {
            NodeData::Global { code, .. } => Some(code),
            _ => None,
        }
    }

    /// The global operand.
    pub(crate) fn global_operand(&self) -> GlobalOperand {
        match self.data {
            NodeData::Global { operand, .. } => operand,
            _ => GlobalOperand::None,
        }
    }

    /// The accessor code, if any.
    pub(crate) fn accessor_code(&self) -> Option<Accessor> {
        match self.data {
            NodeData::Accessor(code) => Some(code),
            _ => None,
        }
    }

    /// The kind-scoped impl code, or zero.
    pub(crate) fn impl_flag(&self) -> u8 {
        match self.data {
            NodeData::Impl { code } => code,
            _ => 0,
        }
    }

    /// The macro role, if any.
    pub(crate) fn macro_role(&self) -> Option<MacroRole> {
        match self.data {
            NodeData::MacroRole(role) => Some(role),
            _ => None,
        }
    }

    /// The autodiff code.
    pub(crate) fn autodiff_code(&self) -> AutoDiffCode {
        match self.data {
            NodeData::AutoDiff(code) => code,
            _ => AutoDiffCode::Forward,
        }
    }

    /// The generic-specialization kind and representation-changed flag.
    pub(crate) fn generic_specialization(&self) -> Option<(GenericSpecializationKind, bool)> {
        match self.data {
            NodeData::GenericSpecialization {
                kind,
                representation_changed,
            } => Some((kind, representation_changed)),
            _ => None,
        }
    }

    /// The function-signature specialization representation-changed flag.
    pub(crate) fn function_signature_representation_changed(&self) -> bool {
        match self.data {
            NodeData::FunctionSignatureSpecialization {
                representation_changed,
            } => representation_changed,
            _ => false,
        }
    }

    /// True for the return-value entry of a function specialization.
    pub(crate) fn function_spec_is_return(&self) -> bool {
        match self.data {
            NodeData::FunctionSpec { is_return } => is_return,
            _ => false,
        }
    }

    /// The key-path thunk flavour.
    pub(crate) fn keypath_kind(&self) -> KeyPathKind {
        match self.data {
            NodeData::KeyPathThunk(kind) => kind,
            _ => KeyPathKind::Getter,
        }
    }

    /// The layout constraint.
    pub(crate) fn layout_code(&self) -> LayoutCode {
        match self.data {
            NodeData::Layout(code) => code,
            _ => LayoutCode::UnknownLayout,
        }
    }

    /// The deinitializer form.
    pub(crate) fn deinitializer_kind(&self) -> DeinitializerKind {
        match self.data {
            NodeData::Deinitializer(kind) => kind,
            _ => DeinitializerKind::Deinit,
        }
    }

    /// The function attribute.
    pub(crate) fn attribute_kind(&self) -> AttributeKind {
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
pub(crate) struct Builder<'a> {
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
    pub(crate) fn set_data(&mut self, data: NodeData) {
        self.data = data;
    }

    /// Replace the text.
    pub(crate) fn set_text(&mut self, text: Text<'a>) {
        self.text = text;
    }

    /// Set the primary number value.
    pub(crate) fn set_value(&mut self, value: u32) {
        if let NodeData::Number { value: current, .. } = &mut self.data {
            *current = value;
        }
    }

    /// Set the secondary number value.
    pub(crate) fn set_len(&mut self, len: u32) {
        if let NodeData::Number { len: current, .. } = &mut self.data {
            *current = len;
        }
    }

    /// Seal the child list and wrap the node.
    pub(crate) fn build(self) -> NodeRef<'a> {
        Rc::new(Node {
            kind: self.kind,
            text: self.text,
            data: self.data,
            children: self.children.into_boxed_slice(),
        })
    }
}

/// Parsing context: substitutions, word substitutions and the node budget.
pub(crate) struct Context<'a> {
    subs: Vec<NodeRef<'a>>,
    words: Vec<&'a [u8]>,
    nodes: usize,
    failed: bool,
}

impl<'a> Context<'a> {
    /// Create an empty context.
    pub(crate) const fn new() -> Self {
        Self {
            subs: Vec::new(),
            words: Vec::new(),
            nodes: 0,
            failed: false,
        }
    }

    /// Whether the node budget was exceeded.
    pub(crate) fn failed(&self) -> bool {
        self.failed
    }

    fn charge(&mut self) {
        self.nodes = self.nodes.saturating_add(1);
        if self.nodes > MAX_NODES {
            self.failed = true;
        }
    }

    fn builder_with(&mut self, kind: Kind, text: Text<'a>, data: NodeData) -> Builder<'a> {
        self.charge();
        Builder {
            kind,
            text,
            data,
            children: Vec::new(),
        }
    }

    /// A structural builder with the kind's default payload.
    pub(crate) fn builder(&mut self, kind: Kind) -> Builder<'a> {
        self.builder_with(kind, None, NodeData::default_for(kind))
    }

    /// A text-bearing builder.
    pub(crate) fn builder_text(&mut self, kind: Kind, text: &'a str) -> Builder<'a> {
        self.builder_with(kind, Some(Cow::Borrowed(text)), NodeData::default_for(kind))
    }

    /// A text-bearing builder from either borrowed or escaped text.
    pub(crate) fn builder_text_cow(&mut self, kind: Kind, text: Cow<'a, str>) -> Builder<'a> {
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
    pub(crate) fn builder_value(&mut self, kind: Kind, value: u32) -> Builder<'a> {
        self.builder_with(kind, None, NodeData::Number { value, len: 0 })
    }

    /// An indexed-entity builder.
    pub(crate) fn builder_index(&mut self, kind: Kind, index: u32) -> Builder<'a> {
        self.builder_with(kind, None, NodeData::Index(index))
    }

    /// A global wrapper builder.
    pub(crate) fn builder_global(&mut self, code: Global, operand: GlobalOperand) -> Builder<'a> {
        self.builder_with(Kind::Global, None, NodeData::Global { code, operand })
    }

    /// An accessor-coded builder.
    pub(crate) fn builder_accessor(&mut self, kind: Kind, code: Accessor) -> Builder<'a> {
        self.builder_with(kind, None, NodeData::Accessor(code))
    }

    /// An impl-code builder.
    pub(crate) fn builder_impl(&mut self, kind: Kind, code: u8) -> Builder<'a> {
        self.builder_with(kind, None, NodeData::Impl { code })
    }

    /// A macro-role builder.
    pub(crate) fn builder_macro_role(&mut self, kind: Kind, role: MacroRole) -> Builder<'a> {
        self.builder_with(kind, None, NodeData::MacroRole(role))
    }

    /// An autodiff builder.
    pub(crate) fn builder_autodiff(&mut self, kind: Kind, code: AutoDiffCode) -> Builder<'a> {
        self.builder_with(kind, None, NodeData::AutoDiff(code))
    }

    /// A generic-specialization builder.
    pub(crate) fn builder_generic_specialization(
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
    pub(crate) fn builder_function_signature_specialization(
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
    pub(crate) fn builder_function_spec(&mut self, is_return: bool) -> Builder<'a> {
        self.builder_with(
            Kind::FuncSpecParam,
            None,
            NodeData::FunctionSpec { is_return },
        )
    }

    /// A key-path thunk builder.
    pub(crate) fn builder_keypath(&mut self, kind: KeyPathKind) -> Builder<'a> {
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
    pub(crate) fn builder_deinit(&mut self, kind: DeinitializerKind) -> Builder<'a> {
        self.builder_with(Kind::Deinitializer, None, NodeData::Deinitializer(kind))
    }

    /// A function-attribute builder.
    pub(crate) fn builder_attribute(&mut self, kind: AttributeKind) -> Builder<'a> {
        self.builder_with(Kind::Attribute, None, NodeData::Attribute(kind))
    }

    /// Copy `id` with a different kind, preserving payload and children.
    pub(crate) fn relabel(&self, id: &NodeRef<'a>, kind: Kind) -> NodeRef<'a> {
        Rc::new(Node {
            kind,
            text: id.text_cow(),
            data: *id.data(),
            children: id.children().to_vec().into_boxed_slice(),
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
        Rc::new(Node {
            kind,
            text: id.text_cow(),
            data: *id.data(),
            children: children.into_boxed_slice(),
        })
    }

    /// Copy `id` with one more child; used only where the reference mutates a
    /// node that may already be shared.
    pub(crate) fn append(&self, id: &NodeRef<'a>, child: NodeRef<'a>) -> NodeRef<'a> {
        let mut children = Vec::with_capacity(id.child_count() + 1);
        children.extend(id.children().iter().cloned());
        children.push(child);
        Rc::new(Node {
            kind: id.kind(),
            text: id.text_cow(),
            data: *id.data(),
            children: children.into_boxed_slice(),
        })
    }

    /// Node kind.
    pub(crate) fn kind(&self, id: &NodeRef<'a>) -> Kind {
        id.kind()
    }

    /// Nth child.
    pub(crate) fn child(&self, parent: &NodeRef<'a>, index: usize) -> Option<NodeRef<'a>> {
        parent.child(index)
    }

    /// Number of children.
    pub(crate) fn child_count(&self, parent: &NodeRef<'a>) -> usize {
        parent.child_count()
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
    pub(crate) fn word_count(&self) -> usize {
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
    use super::*;

    #[test]
    fn nodes_are_immutable_and_shared() {
        let mut context = Context::new();
        let child = context.builder_text(Kind::Identifier, "main").build();
        let mut parent = context.builder(Kind::Tuple);
        parent.push(child.clone());
        let parent = parent.build();
        assert_eq!(parent.child_count(), 1);
        assert!(Rc::ptr_eq(&parent.child(0).unwrap(), &child));

        let substitution = parent.clone();
        assert!(Rc::ptr_eq(&substitution, &parent));
        assert_eq!(substitution.kind(), Kind::Tuple);
    }

    #[test]
    fn payload_accessors_read_the_data() {
        let mut context = Context::new();

        let number = context.builder_value(Kind::Number, 7).build();
        assert_eq!(number.number_value(), 7);
        assert_eq!(number.number_len(), 0);

        let index = context.builder_index(Kind::ExplicitClosure, 4).build();
        assert_eq!(index.entity_index(), 4);

        let slot = context
            .builder_global(Global::FieldOffset, GlobalOperand::Directness(true))
            .build();
        assert_eq!(slot.global_code(), Some(Global::FieldOffset));
        assert_eq!(slot.global_operand(), GlobalOperand::Directness(true));

        let accessor = context
            .builder_accessor(Kind::Variable, Accessor::Getter)
            .build();
        assert_eq!(accessor.accessor_code(), Some(Accessor::Getter));

        let role = context
            .builder_macro_role(Kind::AttachedMacroExpansion, MacroRole::Preamble)
            .build();
        assert_eq!(role.macro_role(), Some(MacroRole::Preamble));

        let layout = context.builder_layout(b'T').unwrap().build();
        assert_eq!(layout.layout_code(), LayoutCode::Trivial);

        assert!(LayoutCode::from_byte(b'x').is_none());
    }

    #[test]
    fn relabel_and_append_preserve_payload_and_children() {
        let mut context = Context::new();
        let child = context.builder_text(Kind::Identifier, "A").build();
        let mut parent = context.builder_accessor(Kind::Subscript, Accessor::Setter);
        parent.push(child.clone());
        let parent = parent.build();

        let rebuilt = context.relabel(&parent, Kind::Variable);
        assert_eq!(rebuilt.kind(), Kind::Variable);
        assert_eq!(rebuilt.accessor_code(), Some(Accessor::Setter));
        assert!(Rc::ptr_eq(&rebuilt.child(0).unwrap(), &child));

        let extra = context.builder_text(Kind::Identifier, "B").build();
        let appended = context.append(&parent, extra.clone());
        assert_eq!(appended.child_count(), 2);
        assert!(Rc::ptr_eq(&appended.child(1).unwrap(), &extra));
        assert_eq!(parent.child_count(), 1, "the source stays immutable");
    }

    #[test]
    fn budget_is_enforced() {
        let mut context = Context::new();
        for _ in 0..MAX_NODES {
            let _ = context.builder(Kind::Identifier);
        }
        assert!(!context.failed());
        let _ = context.builder(Kind::Identifier);
        assert!(context.failed());
    }

    #[test]
    fn every_payload_family_relabels() {
        let mut context = Context::new();
        let nodes = [
            context.builder_text(Kind::Identifier, "Foo").build(),
            context.builder_value(Kind::Number, 1).build(),
            context.builder_index(Kind::ExplicitClosure, 2).build(),
            context.builder(Kind::Tuple).build(),
            context
                .builder_global(Global::TypeMetadata, GlobalOperand::None)
                .build(),
            context
                .builder_accessor(Kind::Subscript, Accessor::Getter)
                .build(),
            context.builder_impl(Kind::ImplConvention, b'g').build(),
            context
                .builder_macro_role(Kind::AttachedMacroExpansion, MacroRole::Peer)
                .build(),
            context
                .builder_autodiff(Kind::AutoDiffFunctionKind, AutoDiffCode::Reverse)
                .build(),
            context
                .builder_generic_specialization(GenericSpecializationKind::Inlined, false)
                .build(),
            context
                .builder_function_signature_specialization(true)
                .build(),
            context.builder_function_spec(true).build(),
            context.builder_keypath(KeyPathKind::AppliedMethod).build(),
            context.builder_layout(b'e').unwrap().build(),
            context
                .builder_deinit(DeinitializerKind::Deallocating)
                .build(),
            context.builder_attribute(AttributeKind::Override).build(),
        ];
        for node in &nodes {
            let _ = core::format_args!("{:?}", node);
            let relabelled = context.relabel(node, Kind::Tuple);
            assert_eq!(relabelled.kind(), Kind::Tuple);
        }
        let _ = core::format_args!("{:?}", context);
    }
}
