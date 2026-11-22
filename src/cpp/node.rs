//! Immutable node graph for the Itanium C++ AST.
//!
//! Nodes own their children as a sealed slice of [`NodeRef`] handles. A node
//! reused through a substitution is shared, not copied, so the finished graph
//! may be a DAG. Literal text borrows the input; reconstructed text is owned by
//! `Cow`.

use alloc::{borrow::Cow, boxed::Box, rc::Rc, vec::Vec};
use core::fmt;
use core::ops::Range;

/// Shared, reference-counted node handle.
///
/// Cloning a handle only bumps a reference count. Because the graph is
/// `Rc`-owned, [`Node`] (and therefore [`super::Ast`]) is `!Send + !Sync`.
pub type NodeRef<'a> = Rc<Node<'a>>;

/// Node kinds produced by the C++ parsers.
///
/// The core vocabulary is Itanium-shaped and deliberately separate from the
/// Swift and Rust vocabularies. The MSVC parser reuses it where the shape is
/// the same and adds documented MSVC-specific kinds where it is not. Child
/// order is part of the API; see [`Node`] for the per-kind child contract.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
#[non_exhaustive]
pub enum Kind {
    /// Top-level container of an encoding.
    ///
    /// Children: `[shape]`, where `shape` is [`Kind::Function`], [`Kind::Data`]
    /// or one of the special-name kinds ([`Kind::VTable`], [`Kind::TypeInfo`],
    /// [`Kind::Thunk`], ...). The Itanium grammar has the same three-way split:
    /// `<encoding> ::= <function> | <data> | <special-name>`.
    Encoding,
    /// A function encoding.
    ///
    /// Children: `[name, return_type?, ParameterList]`. The return type is
    /// present only when the mangling encodes it (function templates);
    /// constructors, destructors and conversion operators do not encode one.
    /// `text` is the space-separated modifier list for the dialects that
    /// encode one (MSVC access/calling convention/cv qualifiers, legacy GNU v2
    /// `static`/cv qualifiers) and is absent for Itanium.
    Function,
    /// A data encoding.
    ///
    /// Children: `[name]`. `text` is the space-separated modifier list for the
    /// legacy dialects that encode one for data names (ARM `static`/cv
    /// qualifiers) and is absent otherwise.
    Data,
    /// A virtual table (`TV <type>`).
    ///
    /// Children: `[type]`.
    VTable,
    /// A virtual table group, the VTT (`TT <type>`).
    ///
    /// Children: `[type]`.
    Vtt,
    /// A run-time type information object (`TI <type>`).
    ///
    /// Children: `[type]`.
    TypeInfo,
    /// A run-time type information name (`TS <type>`).
    ///
    /// Children: `[type]`.
    TypeInfoName,
    /// A guard variable for a function-local static (`GV <name>`).
    ///
    /// Children: `[name]`.
    GuardVariable,
    /// A reference temporary (`GR <name> [<seq-id>] _`).
    ///
    /// Children: `[name]`. `text` is the temporary's index: `0` for the `_`
    /// form, otherwise the decoded sequence id.
    ReferenceTemporary,
    /// A thread-local storage init function (`TH <name>`).
    ///
    /// Children: `[name]`.
    TlsInit,
    /// A thread-local storage wrapper function (`TW <name>`).
    ///
    /// Children: `[name]`.
    TlsWrapper,
    /// A construction vtable (`TC <type> <type>`).
    ///
    /// Children: `[complete_object_type, base_object_type]`.
    CtorVtable,
    /// A virtual or non-virtual thunk (`Th`, `Tv`, `Tc`).
    ///
    /// Children: `[encoding]`. `text` is the rendered call offset: one offset
    /// for `Th`/`Tv`, two for the covariant `Tc` form, for example
    /// `offset(-8)` or `virtual offset(0, -16), offset(8)`.
    Thunk,
    /// A template parameter object (`TA <template-arg>`).
    ///
    /// Children: `[template-arg]`.
    TemplateParamObject,
    /// A name or type with template arguments (a template-id).
    ///
    /// Children: `[base_name_or_type, TemplateArgs]`.
    Name,
    /// A local name: a name declared inside a function or local class.
    ///
    /// Children: `[encoding, entity]`, where `encoding` is the enclosing
    /// function encoding and `entity` is the local name (an `Identifier` with
    /// the text `string literal` for the `Es` form). `text` is the
    /// discriminator digits when the mangling carries one.
    ///
    /// The MSVC parser also uses this kind for a scope-only local discriminator
    /// (`?<number>?<encoding>`). That form has children `[encoding]` and the
    /// discriminator in `text`.
    LocalName,
    /// A name with an ABI tag (`B <source-name>`).
    ///
    /// Children: `[name]`. `text` is the tag, without the `[abi:` decoration.
    /// Several tags nest.
    AbiTag,
    /// A nested name (a qualified name).
    ///
    /// Children are `[qualifier, unqualified_name]`, or `[name]` when a
    /// cv/ref qualifier applies to a single-component name. The qualifier is
    /// one of `UnqualifiedName`, `Name`, `NestedName`, `Substitution` or
    /// `TemplateParam`. The qualifier child is the nested-name prefix; there is
    /// no separate prefix node. `text` holds the encoded name's cv/ref
    /// qualifiers (for example `const &`) when present.
    NestedName,
    /// A simple (single-component) name.
    ///
    /// Children: `[Identifier | OperatorName | CtorDtorName]`.
    UnqualifiedName,
    /// A source-name identifier. `text` borrows the identifier bytes from the
    /// symbol.
    Identifier,
    /// An unnamed type introduced by a lambda or a local class (`Ut`).
    ///
    /// `text` is the decoded type index, counting from one.
    UnnamedType,
    /// A closure type introduced by a lambda (`Ul <lambda-sig> E [<number>] _`).
    ///
    /// Children are the lambda's template parameter declarations followed by
    /// its parameter types, in that order; the declarations are
    /// [`Kind::TemplateParamDecl`] nodes. `text` is the decoded closure index,
    /// counting from one, or `block-literal` for the `Ub` form.
    Lambda,
    /// A vendor extended qualifier (`W <source-name>`, a MIPS extension).
    ///
    /// Children are the qualified name's components as usual; this node marks
    /// the vendor namespace the following component belongs to. `text` is the
    /// vendor name.
    VendorQualifier,
    /// A `decltype` type (`Dt <expression> E` or `DT <expression> E`).
    ///
    /// Children: `[expression]`.
    Decltype,
    /// An operator name. `text` is the operator spelling (for example
    /// `operator+`); a conversion operator has the text `operator` and one
    /// child, the conversion target type; a literal operator has the text
    /// `operator"" <suffix>` and one child, the suffix identifier.
    OperatorName,
    /// A constructor or destructor name. `text` is the ABI code (`C1`-`C5`,
    /// `CI1`, `CI2`, `D0`-`D2`, `D4` or `D5`).
    CtorDtorName,
    /// A well-known substitution (`St`, `Sa`, `Sb`, `Ss`, `Si`, `So`, `Sd`).
    ///
    /// `text` is the substituted name. Ordinary substitution references reuse
    /// the referenced node instead of producing a `Substitution` node.
    Substitution,
    /// A template argument list.
    ///
    /// Children: `[argument...]`.
    TemplateArgs,
    /// A template parameter reference. `text` is the encoded reference (`T_`,
    /// `T0_`, `TL1_`, ...).
    TemplateParam,
    /// A template parameter declaration inside a template argument list.
    ///
    /// `text` is the encoded keyword. Children depend on it:
    ///
    /// - `Ty`: none;
    /// - `Tn <type>`: `[type]`;
    /// - `Tk <name> [<template-args>]`: `[name]`, plus the argument list when
    ///   the concept is a template-id;
    /// - `Tt <decl>* E`: the nested declarations;
    /// - `Tp <decl>`: `[decl]`.
    TemplateParamDecl,
    /// A function parameter list.
    ///
    /// Children: `[Parameter...]`.
    ParameterList,
    /// One function parameter.
    ///
    /// Children: `[type]`.
    Parameter,
    /// A builtin type. `text` is the readable type name (`int`,
    /// `unsigned long`, `_BitInt(8)`, ...).
    BuiltinType,
    /// A pointer type.
    ///
    /// Children: `[pointee]`.
    PointerType,
    /// An lvalue reference type.
    ///
    /// Children: `[pointee]`.
    ReferenceType,
    /// An rvalue reference type.
    ///
    /// Children: `[pointee]`.
    RvalueReferenceType,
    /// A cv-qualified type. `text` is the qualifier string.
    ///
    /// Children: `[type]`.
    QualifiedType,
    /// An array type. `text` is the encoded element count, or `None` for an
    /// unknown bound (`A_`).
    ///
    /// Children: `[element]`.
    ArrayType,
    /// A function type.
    ///
    /// Children: `[return_type, ParameterList]`. The return type is part of the
    /// encoded function type and is never dropped. The MSVC and legacy GNU v2
    /// encodings carry no return type, so they build the one-child form
    /// `[ParameterList]`. `text` holds a ref-qualifier (Itanium) or the
    /// cv-qualifiers of a member function pointer (legacy GNU v2).
    FunctionType,
    /// A pointer to member type.
    ///
    /// Children: `[class, member]`.
    MemberPointerType,
    /// A vendor extended type (`u <source-name>`).
    ///
    /// Children: `[Identifier]`.
    VendorType,
    /// A parameter pack.
    ///
    /// Children are the pack elements: one pattern for `Dp <type>`, or the
    /// template arguments of a `J...E` argument pack.
    Pack,
    /// A literal expression, the `<expr-primary>` production.
    ///
    /// Children: `[type]`, or `[encoding]` for the external-name form
    /// `L <mangled-name> E`. `text` is the literal value as encoded (`1`, `n3`,
    /// `3f800000`), and `None` for a string literal (`L <array type> E`) or an
    /// external name.
    Literal,
    /// An expression, as used in template arguments and `decltype`.
    ///
    /// Children are the operands in source order. `text` names the form:
    /// the operator spelling for operator expressions (`operator+`, ...), or
    /// one of `call`, `conversion`, `type-list`, `initializer-list`, `cast`,
    /// `typeid`, `sizeof`, `alignof`, `noexcept`, `throw`, `member`, `pack`,
    /// `fold` and `template-param` for the non-operator productions.
    Expression,
    /// An MSVC data symbol.
    ///
    /// Children: `[name, type]`. `text` is the storage class: `global`,
    /// `private-static`, `protected-static`, `public-static` or
    /// `function-local-static`.
    Variable,
    /// An MSVC member function encoding.
    ///
    /// Children: `[name, return_type?, ParameterList]`. Constructors,
    /// destructors and other structors have no return type. `text` is the
    /// space-separated modifier list: access (`public`, `protected`,
    /// `private`), `static` or `virtual`, the calling convention, and any
    /// cv/ref qualifiers (`const`, `volatile`, `__restrict`, `__unaligned`,
    /// `&`, `&&`).
    MemberFunction,
    /// An MSVC virtual function table (`??_7`) or local vftable (`??_S`).
    ///
    /// Children: `[class, target_class...]`, where the target classes are the
    /// `{for ...}` base classes. `text` is `const` for a const vftable and
    /// `local` for the `??_S` local form; both may appear as `local const`.
    Vftable,
    /// An MSVC virtual base table (`??_8`).
    ///
    /// Children: `[class, target_class...]`. `text` is `const` for a const
    /// vbtable.
    Vbtable,
    /// An MSVC run-time type information descriptor (`??_R0` through `??_R4`,
    /// and `.?` type descriptor names).
    ///
    /// `text` is the descriptor kind: `type-descriptor`,
    /// `type-descriptor-name`, `base-class-descriptor`,
    /// `base-class-array`, `class-hierarchy-descriptor` or
    /// `complete-object-locator`. A base-class descriptor appends its four
    /// decoded fields, for example `base-class-descriptor (0, -1, 0, 64)`.
    /// Children are `[type]` for a type descriptor and `[class]` for the other
    /// descriptors.
    Rtti,
    /// An MSVC dynamic initializer (`??__E`).
    ///
    /// Children: `[initialized_entity]`.
    DynamicInitializer,
    /// An MSVC dynamic atexit destructor (`??__F`).
    ///
    /// Children: `[destroyed_entity]`.
    DynamicAtexitDestructor,
    /// An MSVC encoded string literal (`??_C`).
    ///
    /// `text` is the decoded literal with trailing NUL characters removed.
    StringLiteral,
    /// An MSVC MD5-mangled name (`??@<32 hex digits>@`).
    ///
    /// `text` is the 32-character hexadecimal digest.
    Md5Name,
    /// An MSVC vcall thunk (`??_9`).
    ///
    /// Children: `[class]`. `text` is the rendered vtable offset and calling
    /// convention, for example `vcall{8, __cdecl}`.
    VCallThunk,
    /// A tagged MSVC type name (`struct`, `class`, `union`, `enum`).
    ///
    /// Children: `[name]`. `text` is the tag keyword.
    TaggedType,
    /// A legacy C++ special name with no Itanium or MSVC equivalent.
    ///
    /// Children: `[shape]`, where `shape` is the wrapped entity. `text` names
    /// the wrapper: `constructors` for the ARM/EDG `__sti__` static
    /// initialiser and `destructors` for the `__std__` static destructor. The
    /// legacy printer renders `global <text> keyed to <shape>`.
    LegacySpecial,
    /// A production the parser can delimit but not model yet.
    ///
    /// `text` is the raw encoded text of the production. Opaque nodes are
    /// reported as partial coverage by the corpus test; they are never emitted
    /// for a fully modelled production.
    Opaque,
}

/// One immutable AST node.
///
/// Nodes are constructed by the parser and are never mutated. Public accessors
/// expose the kind, text, input span and sealed children without exposing any
/// interior mutability.
pub struct Node<'a> {
    kind: Kind,
    text: Option<Cow<'a, str>>,
    span: Option<Range<u32>>,
    children: Box<[NodeRef<'a>]>,
}

impl<'a> Node<'a> {
    /// Build a node from parsed parts.
    pub(crate) fn new(
        kind: Kind,
        text: Option<Cow<'a, str>>,
        span: Option<Range<u32>>,
        children: Vec<NodeRef<'a>>,
    ) -> NodeRef<'a> {
        Rc::new(Self {
            kind,
            text,
            span,
            children: children.into_boxed_slice(),
        })
    }

    /// The node kind.
    #[must_use]
    pub const fn kind(&self) -> Kind {
        self.kind
    }

    /// The node's text, if it has any.
    ///
    /// Borrowed text points into the input symbol; reconstructed text is owned
    /// by the node.
    #[must_use]
    pub fn text(&self) -> Option<&str> {
        self.text.as_deref()
    }

    /// The byte range of the mangled input that produced this node.
    ///
    /// The range covers the whole encoded production, including delimiters and
    /// source-name length prefixes. It is `None` only when the range does not
    /// fit in `u32`.
    #[must_use]
    pub fn span(&self) -> Option<Range<u32>> {
        self.span.clone()
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
}

impl fmt::Debug for Node<'_> {
    /// Print the node without walking the graph, so a shared DAG cannot make
    /// `Debug` unbounded.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Node")
            .field("kind", &self.kind)
            .field("text", &self.text)
            .field("span", &self.span)
            .field("children", &self.children.len())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_is_bounded() {
        let leaf = Node::new(
            Kind::BuiltinType,
            Some(Cow::Borrowed("int")),
            None,
            Vec::new(),
        );
        let parent = Node::new(Kind::PointerType, None, None, alloc::vec![leaf]);
        let rendered = alloc::format!("{parent:?}");
        assert!(rendered.contains("PointerType"));
        assert!(rendered.contains("children: 1"));
        assert!(!rendered.contains("BuiltinType"));
    }

    #[test]
    fn children_are_sealed() {
        let leaf = Node::new(Kind::Identifier, Some(Cow::Borrowed("x")), None, Vec::new());
        let node = Node::new(Kind::UnqualifiedName, None, None, alloc::vec![leaf]);
        assert_eq!(node.child_count(), 1);
        assert_eq!(node.children().len(), 1);
        assert!(!node.is_leaf());
        assert!(node.child(1).is_none());
    }
}
