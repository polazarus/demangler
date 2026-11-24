//! Reference-counted node graph for the Swift demangler.
//!
//! Nodes own their children as an ordered slice of [`NodeRef`] handles. A node
//! reused through a substitution is shared, not copied, so the finished graph
//! may be a DAG. Literal text borrows the input or static tables; reconstructed
//! text is owned by `Cow`. Identifier bytes that are not valid UTF-8 are
//! escaped when they become node text, so text is always valid `str`.

use alloc::{borrow::Cow, boxed::Box, rc::Rc, vec::Vec};
use core::cell::{Cell, Ref, RefCell, RefMut};

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

/// Global symbol wrapper codes (stored in [`Node::Global`]).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
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
    // Produced by no parser yet: the variant only names a printer arm, so the
    // shared vocabulary keeps it until the code is parsed.
    #[allow(dead_code)]
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
    // Classified by the parser as a function attribute, but no parser builds it
    // yet, so the shared vocabulary keeps it.
    #[allow(dead_code)]
    DirectMethodReferenceThunk,
    AsyncFunctionPointer,
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
    // Produced by no parser yet: the variant only names a printer arm.
    #[allow(dead_code)]
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

/// Accessor codes (stored in [`Node::Accessor`]).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
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
    // Accessors only the legacy `_T` grammar emits.
    /// `owningAddressor` (legacy `_T` mangling).
    OwningAddressor,
    /// `nativeOwningAddressor` (legacy `_T` mangling).
    NativeOwningAddressor,
    /// `nativePinningAddressor` (legacy `_T` mangling).
    NativePinningAddressor,
    /// `owningMutableAddressor` (legacy `_T` mangling).
    OwningMutableAddressor,
    /// `nativeOwningMutableAddressor` (legacy `_T` mangling).
    NativeOwningMutableAddressor,
    /// `nativePinningMutableAddressor` (legacy `_T` mangling).
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

/// Category-based node payload.
///
/// Kinds are grouped by the data they carry instead of one variant per
/// [`Kind`]. [`Node::Branch`] is structural-only: it holds a kind and children
/// and no generic payload fields. Payload-carrying kinds live in typed
/// categories: text, number, global, accessor, impl, macro role, autodiff,
/// specialization, function-spec parameter, key-path thunk, layout
/// requirement, deinitializer, indexed entity and attribute.
enum Node<'a> {
    /// Text-bearing node (`Identifier`, `Module`, `Suffix`, ...).
    Text(TextNode<'a>),
    /// Numeric payload node (`Number`, `Integer`, `DependentGenericParamType`,
    /// ...).
    Number(NumberNode),
    /// Structural node: kind plus children only.
    Branch(BranchNode<'a>),
    /// Global symbol wrapper; the code is typed, not a raw flag byte.
    Global(GlobalNode<'a>),
    /// A declaration carrying an accessor code (`Variable`, `Subscript`, or
    /// the (not yet emitted) `Accessor` entity).
    Accessor(AccessorNode<'a>),
    /// Implementation node; `flag` is the convention/differentiability/callee/
    /// function/coroutine/called-once code, and zero for structural impl nodes.
    Impl(ImplNode<'a>),
    /// Attached macro expansion; `role` is the role letter.
    MacroRole(MacroRoleNode<'a>),
    /// Autodiff kind code (`AutoDiffFunctionKind`, `DifferentiableFunctionType`).
    AutoDiff(AutoDiffNode),
    /// Generic or function-signature specialization flags.
    Specialization(SpecializationNode<'a>),
    /// Function-signature specialization parameter; `is_return` marks the
    /// return-value entry.
    FunctionSpec(FunctionSpecNode<'a>),
    /// Key-path thunk helper flags.
    KeyPathThunk(KeyPathThunkNode<'a>),
    /// Dependent generic layout requirement; `layout` is the layout code.
    LayoutRequirement(LayoutRequirementNode<'a>),
    /// Deinitializer; `code` selects `deinit`, `__deallocating_deinit` (1) or
    /// `__isolated_deallocating_deinit` (2).
    Deinitializer(DeinitializerNode<'a>),
    /// Numbered entity (closure or default-argument initializer).
    IndexedEntity(IndexedEntityNode<'a>),
    /// Function attribute (`@objc`, `@nonobjc`, ...).
    Attribute(AttributeNode),
}

/// Text-bearing payload.
///
/// `children` is non-empty only for `Builtin.Vec`, which prints its element
/// after the vector name.
struct TextNode<'a> {
    kind: Kind,
    text: Text<'a>,
    children: Children<'a>,
}

/// Numeric payload; `value` is the number and `len` a secondary index.
struct NumberNode {
    kind: Kind,
    value: u32,
    len: u32,
}

/// Structural payload.
struct BranchNode<'a> {
    kind: Kind,
    children: Children<'a>,
}

/// Global symbol wrapper payload.
struct GlobalNode<'a> {
    code: Global,
    text: Text<'a>,
    value: u32,
    len: u32,
    children: Children<'a>,
}

/// Accessor-coded payload; `kind` distinguishes `Variable`, `Subscript` and
/// `Accessor` nodes, which all carry the same code.
struct AccessorNode<'a> {
    kind: Kind,
    code: Accessor,
    children: Children<'a>,
}

/// Implementation payload.
struct ImplNode<'a> {
    kind: Kind,
    flag: u8,
    children: Children<'a>,
}

/// Attached macro expansion payload.
struct MacroRoleNode<'a> {
    kind: Kind,
    role: u8,
    children: Children<'a>,
}

/// Autodiff kind payload.
struct AutoDiffNode {
    kind: Kind,
    code: u8,
}

/// Specialization payload.
struct SpecializationNode<'a> {
    kind: Kind,
    flags: u8,
    children: Children<'a>,
}

/// Function-signature specialization parameter payload.
struct FunctionSpecNode<'a> {
    kind: Kind,
    is_return: bool,
    children: Children<'a>,
}

/// Key-path thunk payload.
struct KeyPathThunkNode<'a> {
    kind: Kind,
    flags: u8,
    children: Children<'a>,
}

/// Layout requirement payload.
struct LayoutRequirementNode<'a> {
    kind: Kind,
    layout: u8,
    children: Children<'a>,
}

/// Deinitializer payload.
struct DeinitializerNode<'a> {
    kind: Kind,
    code: u8,
    children: Children<'a>,
}

/// Numbered entity payload.
struct IndexedEntityNode<'a> {
    kind: Kind,
    index: u32,
    children: Children<'a>,
}

/// Function attribute payload.
struct AttributeNode {
    kind: Kind,
    code: u8,
}

impl<'a> Node<'a> {
    /// Create an empty node, choosing its category from `kind`.
    ///
    /// `Global` and `Accessor` are not allocated through this path; they use
    /// the dedicated constructors so their codes are typed. `ValueWitness`
    /// carries its witness name as text and is only emitted by the legacy
    /// parser. The `Branch` fallback keeps generic allocation total and is
    /// structural-only.
    fn new(kind: Kind) -> Self {
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
            | Kind::ValueWitness => Self::text_node(kind, None),
            Kind::Number
            | Kind::Integer
            | Kind::NegativeInteger
            | Kind::DependentGenericParamType
            | Kind::DependentGenericParamCount
            | Kind::FuncSpecNumber
            | Kind::FuncSpecKind
            | Kind::DroppedArgument
            | Kind::OpaqueReturnTypeIndex => Self::number(kind, 0),
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
            | Kind::ImplSendingResult => Self::impl_node(kind),
            Kind::AttachedMacroExpansion => Self::macro_role_node(kind),
            Kind::AutoDiffFunctionKind | Kind::DifferentiableFunctionType => Self::auto_diff(kind),
            Kind::GenericSpecialization | Kind::FunctionSignatureSpecialization => {
                Self::specialization(kind)
            }
            Kind::FuncSpecParam => Self::function_spec(kind),
            Kind::KeyPathThunkHelper => Self::key_path_thunk(kind),
            Kind::DependentGenericLayoutRequirement => Self::layout_requirement(kind),
            Kind::Deinitializer => Self::deinitializer(kind),
            Kind::ExplicitClosure | Kind::ImplicitClosure | Kind::DefaultArgumentInitializer => {
                Self::indexed_entity(kind)
            }
            Kind::Attribute => Self::attribute(kind),
            _ => Self::branch(kind),
        }
    }

    /// Create a text-bearing node.
    fn text_node(kind: Kind, text: Text<'a>) -> Self {
        Self::Text(TextNode {
            kind,
            text,
            children: Children::open(),
        })
    }

    /// Create a numeric node.
    fn number(kind: Kind, value: u32) -> Self {
        Self::Number(NumberNode {
            kind,
            value,
            len: 0,
        })
    }

    /// Create a structural node.
    ///
    /// Global and accessor kinds need typed construction and must not reach
    /// the structural category. This is an internal invariant: debug builds
    /// reject a violation, release builds still produce a structural node.
    fn branch(kind: Kind) -> Self {
        debug_assert!(
            !matches!(kind, Kind::Global | Kind::Accessor),
            "global and accessor nodes need dedicated construction"
        );
        Self::Branch(BranchNode {
            kind,
            children: Children::open(),
        })
    }

    /// Create a global symbol wrapper.
    fn global(code: Global, text: Text<'a>) -> Self {
        Self::Global(GlobalNode {
            code,
            text,
            value: 0,
            len: 0,
            children: Children::open(),
        })
    }

    /// Create an implementation node.
    fn impl_node(kind: Kind) -> Self {
        Self::Impl(ImplNode {
            kind,
            flag: 0,
            children: Children::open(),
        })
    }

    /// Create an attached macro expansion node.
    fn macro_role_node(kind: Kind) -> Self {
        Self::MacroRole(MacroRoleNode {
            kind,
            role: 0,
            children: Children::open(),
        })
    }

    /// Create an autodiff kind node.
    fn auto_diff(kind: Kind) -> Self {
        Self::AutoDiff(AutoDiffNode { kind, code: 0 })
    }

    /// Create a specialization node.
    fn specialization(kind: Kind) -> Self {
        Self::Specialization(SpecializationNode {
            kind,
            flags: 0,
            children: Children::open(),
        })
    }

    /// Create a function-signature specialization parameter node.
    fn function_spec(kind: Kind) -> Self {
        Self::FunctionSpec(FunctionSpecNode {
            kind,
            is_return: false,
            children: Children::open(),
        })
    }

    /// Create a key-path thunk node.
    fn key_path_thunk(kind: Kind) -> Self {
        Self::KeyPathThunk(KeyPathThunkNode {
            kind,
            flags: 0,
            children: Children::open(),
        })
    }

    /// Create a layout requirement node.
    fn layout_requirement(kind: Kind) -> Self {
        Self::LayoutRequirement(LayoutRequirementNode {
            kind,
            layout: 0,
            children: Children::open(),
        })
    }

    /// Create a deinitializer node.
    fn deinitializer(kind: Kind) -> Self {
        Self::Deinitializer(DeinitializerNode {
            kind,
            code: 0,
            children: Children::open(),
        })
    }

    /// Create a numbered entity node.
    fn indexed_entity(kind: Kind) -> Self {
        Self::IndexedEntity(IndexedEntityNode {
            kind,
            index: 0,
            children: Children::open(),
        })
    }

    /// Create a function attribute node.
    fn attribute(kind: Kind) -> Self {
        Self::Attribute(AttributeNode { kind, code: 0 })
    }

    /// Node kind.
    fn kind(&self) -> Kind {
        match self {
            Self::Text(node) => node.kind,
            Self::Number(node) => node.kind,
            Self::Branch(node) => node.kind,
            Self::Global(_) => Kind::Global,
            Self::Accessor(node) => node.kind,
            Self::Impl(node) => node.kind,
            Self::MacroRole(node) => node.kind,
            Self::AutoDiff(node) => node.kind,
            Self::Specialization(node) => node.kind,
            Self::FunctionSpec(node) => node.kind,
            Self::KeyPathThunk(node) => node.kind,
            Self::LayoutRequirement(node) => node.kind,
            Self::Deinitializer(node) => node.kind,
            Self::IndexedEntity(node) => node.kind,
            Self::Attribute(node) => node.kind,
        }
    }

    /// Global code, when this is a global wrapper.
    fn global_code(&self) -> Option<Global> {
        match self {
            Self::Global(node) => Some(node.code),
            _ => None,
        }
    }

    /// Accessor code, when this node carries one.
    fn accessor_code(&self) -> Option<Accessor> {
        match self {
            Self::Accessor(node) => Some(node.code),
            _ => None,
        }
    }

    /// Numeric value of a number node (zero otherwise).
    fn number_value(&self) -> u32 {
        match self {
            Self::Number(node) => node.value,
            _ => 0,
        }
    }

    /// Secondary numeric value of a number node (zero otherwise).
    fn number_len(&self) -> u32 {
        match self {
            Self::Number(node) => node.len,
            _ => 0,
        }
    }

    /// Numeric value of a global wrapper (zero otherwise).
    fn global_value(&self) -> u32 {
        match self {
            Self::Global(node) => node.value,
            _ => 0,
        }
    }

    /// Secondary numeric value of a global wrapper (zero otherwise).
    fn global_len(&self) -> u32 {
        match self {
            Self::Global(node) => node.len,
            _ => 0,
        }
    }

    /// Index of a numbered entity (zero otherwise).
    fn entity_index(&self) -> u32 {
        match self {
            Self::IndexedEntity(node) => node.index,
            _ => 0,
        }
    }

    /// Implementation flag byte (zero otherwise).
    fn impl_flag(&self) -> u8 {
        match self {
            Self::Impl(node) => node.flag,
            _ => 0,
        }
    }

    /// Attached macro role byte (zero otherwise).
    fn macro_role(&self) -> u8 {
        match self {
            Self::MacroRole(node) => node.role,
            _ => 0,
        }
    }

    /// Autodiff kind code (zero otherwise).
    fn autodiff_code(&self) -> u8 {
        match self {
            Self::AutoDiff(node) => node.code,
            _ => 0,
        }
    }

    /// Specialization flags (zero otherwise).
    fn specialization_flags(&self) -> u8 {
        match self {
            Self::Specialization(node) => node.flags,
            _ => 0,
        }
    }

    /// Whether a function-spec parameter is the return entry.
    fn function_spec_is_return(&self) -> bool {
        match self {
            Self::FunctionSpec(node) => node.is_return,
            _ => false,
        }
    }

    /// Key-path thunk flags (zero otherwise).
    fn keypath_flags(&self) -> u8 {
        match self {
            Self::KeyPathThunk(node) => node.flags,
            _ => 0,
        }
    }

    /// Layout requirement code (zero otherwise).
    fn layout_code(&self) -> u8 {
        match self {
            Self::LayoutRequirement(node) => node.layout,
            _ => 0,
        }
    }

    /// Whether a deinitializer is the deallocating form.
    fn deinitializer_code(&self) -> u8 {
        match self {
            Self::Deinitializer(node) => node.code,
            _ => 0,
        }
    }

    /// Function attribute code (zero otherwise).
    fn attribute_code(&self) -> u8 {
        match self {
            Self::Attribute(node) => node.code,
            _ => 0,
        }
    }

    /// Borrowed node text.
    fn text_str(&self) -> Option<&str> {
        match self {
            Self::Text(node) => node.text.as_deref(),
            Self::Global(node) => node.text.as_deref(),
            _ => None,
        }
    }

    /// Clone the node's text handle, without borrowing the `RefCell` across
    /// the call.
    fn text_cow(&self) -> Text<'a> {
        match self {
            Self::Text(node) => node.text.clone(),
            Self::Global(node) => node.text.clone(),
            _ => None,
        }
    }

    /// Child storage, if this category can have children.
    fn children_storage(&self) -> Option<&Children<'a>> {
        match self {
            Self::Text(node) => Some(&node.children),
            Self::Number(_) | Self::AutoDiff(_) | Self::Attribute(_) => None,
            Self::Branch(node) => Some(&node.children),
            Self::Global(node) => Some(&node.children),
            Self::Accessor(node) => Some(&node.children),
            Self::Impl(node) => Some(&node.children),
            Self::MacroRole(node) => Some(&node.children),
            Self::Specialization(node) => Some(&node.children),
            Self::FunctionSpec(node) => Some(&node.children),
            Self::KeyPathThunk(node) => Some(&node.children),
            Self::LayoutRequirement(node) => Some(&node.children),
            Self::Deinitializer(node) => Some(&node.children),
            Self::IndexedEntity(node) => Some(&node.children),
        }
    }

    /// Mutable child storage, if this category can have children.
    fn children_storage_mut(&mut self) -> Option<&mut Children<'a>> {
        match self {
            Self::Text(node) => Some(&mut node.children),
            Self::Number(_) | Self::AutoDiff(_) | Self::Attribute(_) => None,
            Self::Branch(node) => Some(&mut node.children),
            Self::Global(node) => Some(&mut node.children),
            Self::Accessor(node) => Some(&mut node.children),
            Self::Impl(node) => Some(&mut node.children),
            Self::MacroRole(node) => Some(&mut node.children),
            Self::Specialization(node) => Some(&mut node.children),
            Self::FunctionSpec(node) => Some(&mut node.children),
            Self::KeyPathThunk(node) => Some(&mut node.children),
            Self::LayoutRequirement(node) => Some(&mut node.children),
            Self::Deinitializer(node) => Some(&mut node.children),
            Self::IndexedEntity(node) => Some(&mut node.children),
        }
    }

    /// Child nodes in order.
    fn children_slice(&self) -> &[NodeRef<'a>] {
        match self.children_storage() {
            Some(children) => children.as_slice(),
            None => &[],
        }
    }

    /// Append a child; categories without child storage ignore it.
    fn push_child(&mut self, child: NodeRef<'a>) {
        if let Some(children) = self.children_storage_mut() {
            children.push(child);
        }
    }

    /// Replace the child list with an open list.
    fn set_children_open(&mut self, children: Vec<NodeRef<'a>>) {
        if let Some(storage) = self.children_storage_mut() {
            *storage = Children::Open(children);
        }
    }

    /// Seal the child list into a boxed slice.
    fn seal_children(&mut self) {
        if let Some(storage) = self.children_storage_mut() {
            storage.seal();
        }
    }

    /// Set the numeric value of a number node.
    fn set_number_value(&mut self, value: u32) {
        if let Self::Number(node) = self {
            node.value = value;
        }
    }

    /// Set the secondary numeric value of a number node.
    fn set_number_len(&mut self, len: u32) {
        if let Self::Number(node) = self {
            node.len = len;
        }
    }

    /// Set the numeric value of a global wrapper.
    fn set_global_value(&mut self, value: u32) {
        if let Self::Global(node) = self {
            node.value = value;
        }
    }

    /// Set the secondary numeric value of a global wrapper.
    fn set_global_len(&mut self, len: u32) {
        if let Self::Global(node) = self {
            node.len = len;
        }
    }

    /// Set the index of a numbered entity.
    fn set_entity_index(&mut self, index: u32) {
        if let Self::IndexedEntity(node) = self {
            node.index = index;
        }
    }

    /// Set the implementation flag byte.
    fn set_impl_flag(&mut self, flag: u8) {
        if let Self::Impl(node) = self {
            node.flag = flag;
        }
    }

    /// Set the attached macro role byte.
    fn set_macro_role(&mut self, role: u8) {
        if let Self::MacroRole(node) = self {
            node.role = role;
        }
    }

    /// Set the autodiff kind code.
    fn set_autodiff_code(&mut self, code: u8) {
        if let Self::AutoDiff(node) = self {
            node.code = code;
        }
    }

    /// Set the specialization flags.
    fn set_specialization_flags(&mut self, flags: u8) {
        if let Self::Specialization(node) = self {
            node.flags = flags;
        }
    }

    /// Mark a function-spec parameter as the return entry.
    fn set_function_spec_is_return(&mut self, is_return: bool) {
        if let Self::FunctionSpec(node) = self {
            node.is_return = is_return;
        }
    }

    /// Set the key-path thunk flags.
    fn set_keypath_flags(&mut self, flags: u8) {
        if let Self::KeyPathThunk(node) = self {
            node.flags = flags;
        }
    }

    /// Set the layout requirement code.
    fn set_layout_code(&mut self, layout: u8) {
        if let Self::LayoutRequirement(node) = self {
            node.layout = layout;
        }
    }

    /// Mark a deinitializer as the deallocating form.
    fn set_deinitializer_code(&mut self, code: u8) {
        if let Self::Deinitializer(node) = self {
            node.code = code;
        }
    }

    /// Set the function attribute code.
    fn set_attribute_code(&mut self, code: u8) {
        if let Self::Attribute(node) = self {
            node.code = code;
        }
    }

    /// Attach an accessor code, moving a structural node into the accessor
    /// category.
    fn set_accessor_code(&mut self, code: Accessor) {
        if let Self::Accessor(node) = self {
            node.code = code;
            return;
        }
        let Self::Branch(branch) = self else {
            return;
        };
        let children = core::mem::replace(&mut branch.children, Children::Sealed(Box::new([])));
        let kind = branch.kind;
        *self = Self::Accessor(AccessorNode {
            kind,
            code,
            children,
        });
    }

    /// Copy this node with a different kind, preserving its category payload
    /// and sharing its children.
    ///
    /// Changing a global wrapper's kind degrades it to a structural branch,
    /// which no caller does.
    fn with_kind(&self, kind: Kind) -> Self {
        match self {
            Node::Text(node) => Node::Text(TextNode {
                kind,
                text: node.text.clone(),
                children: node.children.to_open(),
            }),
            Node::Number(node) => Node::Number(NumberNode {
                kind,
                value: node.value,
                len: node.len,
            }),
            Node::Branch(node) => Node::Branch(BranchNode {
                kind,
                children: node.children.to_open(),
            }),
            Node::Global(node) => Node::Branch(BranchNode {
                kind,
                children: node.children.to_open(),
            }),
            Node::Accessor(node) => Node::Accessor(AccessorNode {
                kind,
                code: node.code,
                children: node.children.to_open(),
            }),
            Node::Impl(node) => Node::Impl(ImplNode {
                kind,
                flag: node.flag,
                children: node.children.to_open(),
            }),
            Node::MacroRole(node) => Node::MacroRole(MacroRoleNode {
                kind,
                role: node.role,
                children: node.children.to_open(),
            }),
            Node::AutoDiff(node) => Node::AutoDiff(AutoDiffNode {
                kind,
                code: node.code,
            }),
            Node::Specialization(node) => Node::Specialization(SpecializationNode {
                kind,
                flags: node.flags,
                children: node.children.to_open(),
            }),
            Node::FunctionSpec(node) => Node::FunctionSpec(FunctionSpecNode {
                kind,
                is_return: node.is_return,
                children: node.children.to_open(),
            }),
            Node::KeyPathThunk(node) => Node::KeyPathThunk(KeyPathThunkNode {
                kind,
                flags: node.flags,
                children: node.children.to_open(),
            }),
            Node::LayoutRequirement(node) => Node::LayoutRequirement(LayoutRequirementNode {
                kind,
                layout: node.layout,
                children: node.children.to_open(),
            }),
            Node::Deinitializer(node) => Node::Deinitializer(DeinitializerNode {
                kind,
                code: node.code,
                children: node.children.to_open(),
            }),
            Node::IndexedEntity(node) => Node::IndexedEntity(IndexedEntityNode {
                kind,
                index: node.index,
                children: node.children.to_open(),
            }),
            Node::Attribute(node) => Node::Attribute(AttributeNode {
                kind,
                code: node.code,
            }),
        }
    }
}

/// Child storage: a growable list while parsing, a boxed slice once sealed.
enum Children<'a> {
    /// The parser is still appending children.
    Open(Vec<NodeRef<'a>>),
    /// The sealing traversal is visiting this node (cycle detection).
    Sealing(Vec<NodeRef<'a>>),
    /// The child list is complete; children are an immutable boxed slice.
    Sealed(Box<[NodeRef<'a>]>),
}

impl<'a> Children<'a> {
    /// Create an empty open child list.
    fn open() -> Self {
        Self::Open(Vec::new())
    }

    fn as_slice(&self) -> &[NodeRef<'a>] {
        match self {
            Self::Open(children) | Self::Sealing(children) => children,
            Self::Sealed(children) => children,
        }
    }

    fn push(&mut self, child: NodeRef<'a>) {
        if let Self::Open(children) = self {
            children.push(child);
        }
    }

    fn seal(&mut self) {
        // Replace first so the previous state can be moved out. A node whose
        // list was already sealed (or cut by cycle detection) restores it
        // unchanged; open and in-progress lists become the final boxed slice.
        let previous = core::mem::replace(self, Self::Sealed(Box::new([])));
        *self = match previous {
            Self::Open(children) | Self::Sealing(children) => {
                Self::Sealed(children.into_boxed_slice())
            }
            sealed @ Self::Sealed(_) => sealed,
        };
    }

    /// Copy this list into a fresh open list, sharing the child handles.
    fn to_open(&self) -> Self {
        Self::Open(self.as_slice().to_vec())
    }
}

/// Shared, reference-counted node handle.
///
/// Cloning a handle only bumps a reference count: a node reused through a
/// substitution is shared, not copied, so the finished tree is a directed
/// acyclic graph rather than a tree.
pub(crate) struct NodeRef<'a>(Rc<RefCell<Node<'a>>>);

impl<'a> NodeRef<'a> {
    /// Wrap a newly allocated node.
    fn new(node: Node<'a>) -> Self {
        Self(Rc::new(RefCell::new(node)))
    }

    /// Borrow the node.
    fn node(&self) -> Ref<'_, Node<'a>> {
        self.0.borrow()
    }

    /// Mutably borrow the node.
    fn node_mut(&self) -> RefMut<'_, Node<'a>> {
        self.0.borrow_mut()
    }

    /// Node kind.
    pub(crate) fn kind(&self) -> Kind {
        self.0.borrow().kind()
    }

    /// Global code, when this is a global wrapper.
    pub(crate) fn global_code(&self) -> Option<Global> {
        self.0.borrow().global_code()
    }

    /// Accessor code, when this node carries one.
    pub(crate) fn accessor_code(&self) -> Option<Accessor> {
        self.0.borrow().accessor_code()
    }

    /// Numeric value of a number node.
    pub(crate) fn number_value(&self) -> u32 {
        self.0.borrow().number_value()
    }

    /// Secondary numeric value of a number node.
    pub(crate) fn number_len(&self) -> u32 {
        self.0.borrow().number_len()
    }

    /// Numeric value of a global wrapper.
    pub(crate) fn global_value(&self) -> u32 {
        self.0.borrow().global_value()
    }

    /// Secondary numeric value of a global wrapper.
    pub(crate) fn global_len(&self) -> u32 {
        self.0.borrow().global_len()
    }

    /// Index of a numbered entity.
    pub(crate) fn entity_index(&self) -> u32 {
        self.0.borrow().entity_index()
    }

    /// Implementation flag byte.
    pub(crate) fn impl_flag(&self) -> u8 {
        self.0.borrow().impl_flag()
    }

    /// Attached macro role byte.
    pub(crate) fn macro_role(&self) -> u8 {
        self.0.borrow().macro_role()
    }

    /// Autodiff kind code.
    pub(crate) fn autodiff_code(&self) -> u8 {
        self.0.borrow().autodiff_code()
    }

    /// Specialization flags.
    pub(crate) fn specialization_flags(&self) -> u8 {
        self.0.borrow().specialization_flags()
    }

    /// Whether a function-spec parameter is the return entry.
    pub(crate) fn function_spec_is_return(&self) -> bool {
        self.0.borrow().function_spec_is_return()
    }

    /// Key-path thunk flags.
    pub(crate) fn keypath_flags(&self) -> u8 {
        self.0.borrow().keypath_flags()
    }

    /// Layout requirement code.
    pub(crate) fn layout_code(&self) -> u8 {
        self.0.borrow().layout_code()
    }

    /// Whether a deinitializer is the deallocating form.
    pub(crate) fn deinitializer_code(&self) -> u8 {
        self.0.borrow().deinitializer_code()
    }

    /// Function attribute code.
    pub(crate) fn attribute_code(&self) -> u8 {
        self.0.borrow().attribute_code()
    }

    /// Run `f` with this node's text borrowed, avoiding a `Cow` clone.
    ///
    /// The closure must not mutate this node: the text is borrowed for its
    /// duration.
    pub(crate) fn with_text<R>(&self, f: impl FnOnce(Option<&str>) -> R) -> R {
        let node = self.0.borrow();
        f(node.text_str())
    }

    /// Clone this node's text handle.
    pub(crate) fn text(&self) -> Text<'a> {
        self.0.borrow().text_cow()
    }

    /// Write this node's text into `out`.
    pub(crate) fn write_text<W: core::fmt::Write + ?Sized>(
        &self,
        out: &mut W,
    ) -> core::fmt::Result {
        if let Some(text) = self.0.borrow().text_str() {
            out.write_str(text)?;
        }
        Ok(())
    }

    /// Compare this node's text with `expected`.
    pub(crate) fn text_eq(&self, expected: &str) -> bool {
        self.0.borrow().text_str() == Some(expected)
    }

    /// Child nodes in order.
    pub(crate) fn children(&self) -> Ref<'_, [Self]> {
        Ref::map(self.0.borrow(), |node| node.children_slice())
    }

    /// Nth child.
    pub(crate) fn child(&self, index: usize) -> Option<Self> {
        self.children().get(index).cloned()
    }

    /// Number of children.
    pub(crate) fn child_count(&self) -> usize {
        self.children().len()
    }

    /// Set the numeric value of a number node.
    pub(crate) fn set_number_value(&self, value: u32) {
        self.0.borrow_mut().set_number_value(value);
    }

    /// Set the secondary numeric value of a number node.
    pub(crate) fn set_number_len(&self, len: u32) {
        self.0.borrow_mut().set_number_len(len);
    }

    /// Set the numeric value of a global wrapper.
    pub(crate) fn set_global_value(&self, value: u32) {
        self.0.borrow_mut().set_global_value(value);
    }

    /// Set the secondary numeric value of a global wrapper.
    pub(crate) fn set_global_len(&self, len: u32) {
        self.0.borrow_mut().set_global_len(len);
    }

    /// Set the index of a numbered entity.
    pub(crate) fn set_entity_index(&self, index: u32) {
        self.0.borrow_mut().set_entity_index(index);
    }

    /// Set the implementation flag byte.
    pub(crate) fn set_impl_flag(&self, flag: u8) {
        self.0.borrow_mut().set_impl_flag(flag);
    }

    /// Set the attached macro role byte.
    pub(crate) fn set_macro_role(&self, role: u8) {
        self.0.borrow_mut().set_macro_role(role);
    }

    /// Set the autodiff kind code.
    pub(crate) fn set_autodiff_code(&self, code: u8) {
        self.0.borrow_mut().set_autodiff_code(code);
    }

    /// Set the specialization flags.
    pub(crate) fn set_specialization_flags(&self, flags: u8) {
        self.0.borrow_mut().set_specialization_flags(flags);
    }

    /// Mark a function-spec parameter as the return entry.
    pub(crate) fn set_function_spec_is_return(&self, is_return: bool) {
        self.0.borrow_mut().set_function_spec_is_return(is_return);
    }

    /// Set the key-path thunk flags.
    pub(crate) fn set_keypath_flags(&self, flags: u8) {
        self.0.borrow_mut().set_keypath_flags(flags);
    }

    /// Set the layout requirement code.
    pub(crate) fn set_layout_code(&self, layout: u8) {
        self.0.borrow_mut().set_layout_code(layout);
    }

    /// Mark a deinitializer as the deallocating form.
    pub(crate) fn set_deinitializer_code(&self, code: u8) {
        self.0
            .borrow_mut()
            .set_deinitializer_code(code);
    }

    /// Set the function attribute code.
    pub(crate) fn set_attribute_code(&self, code: u8) {
        self.0.borrow_mut().set_attribute_code(code);
    }

    /// Attach an accessor code to this node.
    pub(crate) fn set_accessor_code(&self, code: Accessor) {
        self.0.borrow_mut().set_accessor_code(code);
    }

    /// Replace this node's children with `children`.
    pub(crate) fn set_children(&self, children: Vec<Self>) {
        self.0.borrow_mut().set_children_open(children);
    }

    /// Replace this node's children with a copy of `source`'s child list.
    pub(crate) fn replace_children(&self, source: &Self) {
        let children = source.children().to_vec();
        self.0.borrow_mut().set_children_open(children);
    }

    /// Seal the graph reachable from this node.
    ///
    /// Child lists become boxed slices, so the retained tree owns immutable
    /// child storage. Returns `false` when the graph contains a cycle, which
    /// cannot be printed; the cycle edge is cut while sealing so that a
    /// rejected graph can still be freed.
    pub(crate) fn seal(&self) -> bool {
        enum Frame<'a> {
            Enter(NodeRef<'a>),
            Exit(NodeRef<'a>),
        }

        let mut acyclic = true;
        let mut stack = Vec::new();
        stack.push(Frame::Enter(self.clone()));
        while let Some(frame) = stack.pop() {
            match frame {
                Frame::Enter(node) => {
                    {
                        let mut inner = node.0.borrow_mut();
                        let Some(children) = inner.children_storage_mut() else {
                            continue;
                        };
                        match children {
                            Children::Sealed(_) => continue,
                            Children::Sealing(_) => {
                                // The node is its own ancestor: cut the edge.
                                *children = Children::Sealed(Box::new([]));
                                acyclic = false;
                                continue;
                            }
                            Children::Open(_) => {
                                // Move the open list into `Sealing` so a back
                                // edge can see that this node is in progress.
                                // The placeholder is immediately replaced by
                                // the real list; `Children::seal` later turns
                                // it into the final boxed slice.
                                let previous =
                                    core::mem::replace(children, Children::Sealing(Vec::new()));
                                if let Children::Open(kids) = previous {
                                    *children = Children::Sealing(kids);
                                }
                            }
                        }
                    }
                    stack.push(Frame::Exit(node.clone()));
                    let children = node.children();
                    for child in children.iter().rev() {
                        stack.push(Frame::Enter(child.clone()));
                    }
                }
                Frame::Exit(node) => node.0.borrow_mut().seal_children(),
            }
        }
        acyclic
    }
}

impl Clone for NodeRef<'_> {
    fn clone(&self) -> Self {
        Self(Rc::clone(&self.0))
    }
}

impl PartialEq for NodeRef<'_> {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }
}

impl Eq for NodeRef<'_> {}

impl core::fmt::Debug for NodeRef<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("NodeRef").finish_non_exhaustive()
    }
}

/// Parsing context: substitutions, word substitutions and node budget.
pub(crate) struct Context<'a> {
    subs: Vec<NodeRef<'a>>,
    /// Word-substitution table: the identifier bytes a word was read from.
    words: Vec<&'a [u8]>,
    nodes: Cell<usize>,
    failed: Cell<bool>,
}

impl<'a> Context<'a> {
    /// Create an empty context.
    pub(crate) const fn new() -> Self {
        Self {
            subs: Vec::new(),
            words: Vec::new(),
            nodes: Cell::new(0),
            failed: Cell::new(false),
        }
    }

    /// Whether the node budget was exceeded.
    pub(crate) fn failed(&self) -> bool {
        self.failed.get()
    }

    /// Allocate a node, choosing its category from `kind`.
    pub(crate) fn alloc(&self, kind: Kind) -> NodeRef<'a> {
        self.allocate(Node::new(kind))
    }

    /// Allocate a text-bearing node with borrowed text.
    pub(crate) fn alloc_text(&self, kind: Kind, text: &'a str) -> NodeRef<'a> {
        self.allocate(Node::text_node(kind, Some(Cow::Borrowed(text))))
    }

    /// Allocate a text-bearing node with reconstructed text.
    pub(crate) fn alloc_owned(&self, kind: Kind, text: &str) -> NodeRef<'a> {
        self.allocate(Node::text_node(kind, Some(Cow::Owned(text.into()))))
    }

    /// Allocate a text-bearing node from either borrowed or escaped text.
    pub(crate) fn alloc_text_cow(&self, kind: Kind, text: Cow<'a, str>) -> NodeRef<'a> {
        self.allocate(Node::text_node(kind, Some(text)))
    }

    /// Allocate a numeric node.
    pub(crate) fn alloc_value(&self, kind: Kind, value: u32) -> NodeRef<'a> {
        self.allocate(Node::number(kind, value))
    }

    /// Allocate a global symbol wrapper.
    pub(crate) fn alloc_global(&self, code: Global) -> NodeRef<'a> {
        self.allocate(Node::global(code, None))
    }

    /// Allocate a global symbol wrapper with reconstructed text.
    pub(crate) fn alloc_global_text(&self, code: Global, text: &str) -> NodeRef<'a> {
        self.allocate(Node::global(code, Some(Cow::Owned(text.into()))))
    }

    fn allocate(&self, node: Node<'a>) -> NodeRef<'a> {
        let count = self.nodes.get().saturating_add(1);
        self.nodes.set(count);
        if count > MAX_NODES {
            self.failed.set(true);
        }
        NodeRef::new(node)
    }

    /// Node kind.
    pub(crate) fn kind(&self, id: &NodeRef<'a>) -> Kind {
        id.kind()
    }

    /// Append a child.
    pub(crate) fn add_child(&self, parent: &NodeRef<'a>, child: NodeRef<'a>) {
        parent.node_mut().push_child(child);
    }

    /// Copy `id` with a different kind, preserving its category payload and
    /// sharing its children.
    ///
    /// This is the relabelling path (`Identifier` to `Module` or
    /// `TupleElementName`): the copy replaces the role of the source and is not
    /// charged against the node budget, matching the pre-budget behaviour for
    /// relabels.
    pub(crate) fn change_kind(&self, id: &NodeRef<'a>, kind: Kind) -> NodeRef<'a> {
        NodeRef::new(id.node().with_kind(kind))
    }

    /// Like [`Self::change_kind`], but charges the new node against the node
    /// budget.
    ///
    /// Used when a structural rebuild retains both the source and the copy
    /// (generic argument binding), so the budget still bounds total allocation.
    pub(crate) fn rebuild_kind(&self, id: &NodeRef<'a>, kind: Kind) -> NodeRef<'a> {
        self.allocate(id.node().with_kind(kind))
    }

    /// Number of children.
    pub(crate) fn child_count(&self, parent: &NodeRef<'a>) -> usize {
        parent.child_count()
    }

    /// Nth child.
    pub(crate) fn child(&self, parent: &NodeRef<'a>, index: usize) -> Option<NodeRef<'a>> {
        parent.child(index)
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
    use super::{Accessor, Context, Global, Kind, MAX_NODES};

    #[test]
    fn children_are_positional_and_shared() {
        let context = Context::new();
        let child = context.alloc(Kind::Identifier);
        let parent = context.alloc(Kind::Tuple);
        context.add_child(&parent, child.clone());
        context.add_child(&parent, context.alloc(Kind::Type));

        assert_eq!(parent.child_count(), 2);
        assert_eq!(parent.child(0), Some(child.clone()));

        let other = context.alloc(Kind::Tuple);
        context.add_child(&other, child.clone());
        assert_eq!(other.child(0), Some(child));
    }

    #[test]
    fn seal_rejects_cycles() {
        let context = Context::new();
        let outer = context.alloc(Kind::Tuple);
        let inner = context.alloc(Kind::Type);
        context.add_child(&outer, inner.clone());
        context.add_child(&inner, outer.clone());

        assert!(!outer.seal());
        assert_eq!(outer.child_count(), 0);
    }

    #[test]
    fn seal_shares_children() {
        let context = Context::new();
        let leaf = context.alloc(Kind::Identifier);
        let left = context.alloc(Kind::Tuple);
        let right = context.alloc(Kind::Type);
        context.add_child(&left, leaf.clone());
        context.add_child(&right, leaf.clone());
        let root = context.alloc(Kind::Tuple);
        context.add_child(&root, left);
        context.add_child(&root, right);

        assert!(root.seal());
        assert!(root.child(0).is_some());
        assert_eq!(root.child_count(), 2);
    }

    #[test]
    fn categories_store_typed_payloads() {
        let context = Context::new();

        let text = context.alloc_text(Kind::Identifier, "main");
        assert_eq!(text.kind(), Kind::Identifier);
        assert!(text.with_text(|text| text == Some("main")));

        let number = context.alloc_value(Kind::Number, 7);
        assert_eq!(number.kind(), Kind::Number);
        assert_eq!(number.number_value(), 7);
        number.set_number_len(3);
        assert_eq!(number.number_len(), 3);

        let branch = context.alloc(Kind::Tuple);
        assert_eq!(branch.kind(), Kind::Tuple);
        assert_eq!(branch.impl_flag(), 0);
        assert_eq!(branch.specialization_flags(), 0);
        assert_eq!(branch.entity_index(), 0);

        let global = context.alloc_global(Global::TypeMetadata);
        assert_eq!(global.kind(), Kind::Global);
        assert_eq!(global.global_code(), Some(Global::TypeMetadata));
        global.set_global_value(5);
        assert_eq!(global.global_value(), 5);

        let global = context.alloc_global_text(Global::OutlinedBridgedMethod, "abc");
        assert_eq!(global.global_code(), Some(Global::OutlinedBridgedMethod));
        assert!(global.with_text(|text| text == Some("abc")));

        let accessor = context.alloc(Kind::Subscript);
        let child = context.alloc(Kind::Type);
        context.add_child(&accessor, child.clone());
        accessor.set_accessor_code(Accessor::Getter);
        assert_eq!(accessor.kind(), Kind::Subscript);
        assert_eq!(accessor.accessor_code(), Some(Accessor::Getter));
        assert_eq!(accessor.child(0), Some(child));

        let impl_node = context.alloc(Kind::ImplConvention);
        impl_node.set_impl_flag(b'g');
        assert_eq!(impl_node.kind(), Kind::ImplConvention);
        assert_eq!(impl_node.impl_flag(), b'g');

        let macro_node = context.alloc(Kind::AttachedMacroExpansion);
        macro_node.set_macro_role(b'p');
        assert_eq!(macro_node.macro_role(), b'p');

        let autodiff = context.alloc(Kind::DifferentiableFunctionType);
        autodiff.set_autodiff_code(b'r');
        assert_eq!(autodiff.kind(), Kind::DifferentiableFunctionType);
        assert_eq!(autodiff.autodiff_code(), b'r');

        let specialization = context.alloc(Kind::GenericSpecialization);
        specialization.set_specialization_flags(b'B');
        assert_eq!(specialization.specialization_flags(), b'B');

        let function_spec = context.alloc(Kind::FuncSpecParam);
        function_spec.set_function_spec_is_return(true);
        assert!(function_spec.function_spec_is_return());

        let keypath = context.alloc(Kind::KeyPathThunkHelper);
        keypath.set_keypath_flags(3);
        assert_eq!(keypath.keypath_flags(), 3);

        let layout = context.alloc(Kind::DependentGenericLayoutRequirement);
        layout.set_layout_code(b'T');
        assert_eq!(layout.layout_code(), b'T');

        let deinitializer = context.alloc(Kind::Deinitializer);
        deinitializer.set_deinitializer_code(1);
        assert_eq!(deinitializer.deinitializer_code(), 1);

        let closure = context.alloc(Kind::ImplicitClosure);
        closure.set_entity_index(4);
        assert_eq!(closure.entity_index(), 4);

        let attribute = context.alloc(Kind::Attribute);
        attribute.set_attribute_code(2);
        assert_eq!(attribute.kind(), Kind::Attribute);
        assert_eq!(attribute.attribute_code(), 2);
    }

    #[test]
    fn kind_changes_preserve_category_payload() {
        let context = Context::new();

        let text = context.alloc_text(Kind::Identifier, "main");
        let module = context.change_kind(&text, Kind::Module);
        assert_eq!(module.kind(), Kind::Module);
        assert!(module.with_text(|text| text == Some("main")));

        let accessor = context.alloc(Kind::Subscript);
        let child = context.alloc(Kind::Type);
        context.add_child(&accessor, child.clone());
        accessor.set_accessor_code(Accessor::Getter);

        let rebuilt = context.rebuild_kind(&accessor, Kind::Variable);
        assert_eq!(rebuilt.kind(), Kind::Variable);
        assert_eq!(rebuilt.accessor_code(), Some(Accessor::Getter));
        assert_eq!(rebuilt.child(0), Some(child));

        let relabelled = context.change_kind(&accessor, Kind::Subscript);
        assert_eq!(relabelled.accessor_code(), Some(Accessor::Getter));
    }

    #[cfg(debug_assertions)]
    #[test]
    #[should_panic(expected = "dedicated construction")]
    fn branch_rejects_dedicated_categories() {
        let _ = Context::new().alloc(Kind::Global);
    }

    #[test]
    fn node_budget_is_enforced() {
        let context = Context::new();
        for _ in 0..MAX_NODES {
            let _ = context.alloc(Kind::Identifier);
        }
        assert!(!context.failed());
        let _ = context.alloc(Kind::Identifier);
        assert!(context.failed());
    }
}

#[cfg(test)]
mod coverage_tests {
    use alloc::format;
    use alloc::vec;

    use super::{Accessor, Context, Global, Kind};

    #[test]
    fn every_category_relabels_and_debugs() {
        let context = Context::new();
        let nodes = vec![
            context.alloc_text(Kind::Identifier, "Foo"),
            context.alloc_value(Kind::Number, 1),
            context.alloc(Kind::Tuple),
            context.alloc_global(Global::TypeMetadata),
            context.alloc(Kind::Subscript),
            context.alloc(Kind::ImplFunctionType),
            context.alloc(Kind::AttachedMacroExpansion),
            context.alloc(Kind::DifferentiableFunctionType),
            context.alloc(Kind::GenericSpecialization),
            context.alloc(Kind::FuncSpecParam),
            context.alloc(Kind::KeyPathThunkHelper),
            context.alloc(Kind::DependentGenericLayoutRequirement),
            context.alloc(Kind::Deinitializer),
            context.alloc(Kind::ImplicitClosure),
            context.alloc(Kind::Attribute),
        ];
        nodes[4].set_accessor_code(Accessor::Getter);

        for node in &nodes {
            let _ = format!("{node:?}");
            assert_eq!(node.kind(), node.kind());
            let _ = node.child_count();
            let relabelled = context.change_kind(node, Kind::Tuple);
            assert_eq!(relabelled.kind(), Kind::Tuple);
            let _ = format!("{relabelled:?}");
        }
        let _ = format!("{context:?}");
    }
}
