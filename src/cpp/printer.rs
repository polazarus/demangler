//! Itanium C++ ABI demangling printer.
//!
//! Renders a parsed [`Ast`] as demangled C++ text. The text follows the
//! conventions of the LLVM demangler (`llvm-cxxfilt`), which the vendored
//! libc++abi corpus pins (`refs/cpp/itanium/DemangleTestCases.inc`).
//!
//! Printing has two halves. Declarators cannot be written left to right from
//! the tree alone: `RA5_i` is `int (&) [5]`, with the reference inside the
//! parentheses and the array bounds after them, and a function's return type
//! surrounds the function's own name. Every node therefore exposes a left part
//! ([`Printer::print_left`]) and, when [`Printer::has_rhs`] holds, a right part
//! ([`Printer::print_right`]). Expressions additionally carry a precedence so
//! that operands are parenthesised exactly where C++ needs it.
//!
//! The node graph may be a DAG, so a print is bounded by three deterministic
//! budgets: recursion depth, node visits and output bytes.
//!
//! Two productions are lossy in the AST, and the printer documents the gap at
//! the point of use:
//!
//! - cast expressions (`dc`, `sc`, `cc`, `rc`) share the `Expression` node but
//!   carry their distinct spelling in `text`;
//! - an argument pack (`J...E`) and a pack expansion type (`Dp <type>`) share
//!   [`Kind::Pack`]; the parser records which one it built in the node text.

use alloc::collections::BTreeMap;
use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;
use core::fmt::{self, Write};

use super::node::{Kind, NodeRef};
use super::Ast;

/// Maximum print recursion depth.
const MAX_PRINT_DEPTH: u32 = 256;

/// Maximum node visits for one symbol.
const MAX_PRINT_VISITS: usize = 1 << 18;

/// Maximum bytes written for one symbol.
const MAX_OUTPUT_BYTES: usize = 1 << 20;

/// Pack index sentinel: no pack expansion is in progress.
const NO_PACK: usize = usize::MAX;

/// Render `ast` into `out`.
pub(crate) fn print<W: Write + ?Sized>(ast: &Ast<'_>, out: &mut W) -> fmt::Result {
    let mut printer = Printer::new();
    printer.collect_lambda_params(&ast.root());
    printer.print(ast.root())?;
    out.write_str(&printer.out)
}

/// Operator precedence, weakest last.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Precedence {
    Primary,
    Postfix,
    Unary,
    Cast,
    PtrMem,
    Multiplicative,
    Additive,
    Shift,
    Spaceship,
    Relational,
    Equality,
    And,
    Xor,
    Ior,
    AndIf,
    OrIf,
    Conditional,
    Assign,
    Comma,
    Default,
}

/// A declarator property that decides how a type is parenthesised.
#[derive(Clone, Copy, PartialEq, Eq)]
enum TypeProperty {
    /// The type has a right part.
    Right,
    /// The type is a (possibly qualified) array.
    Array,
    /// The type is a (possibly qualified) function.
    Function,
}

/// The base name of a name prefix, used by constructors and destructors.
enum BaseName<'a> {
    /// A name node whose leaf identifier is the base name.
    Name(NodeRef<'a>),
    /// The base name of a well-known substitution.
    Text(&'static str),
}

/// One pass over an [`Ast`], writing text into an owned buffer.
struct Printer<'a> {
    out: String,
    /// Element of the pack being expanded, or [`NO_PACK`].
    pack_index: usize,
    /// Number of elements in the pack being expanded, or [`NO_PACK`].
    pack_max: usize,
    /// Template argument lists in scope, outermost first.
    scopes: Vec<NodeRef<'a>>,
    /// Template parameters that appear in a lambda signature, with the lambda
    /// node they belong to.
    lambda_params: Vec<(usize, NodeRef<'a>)>,
    /// Template template parameters whose inner declarations are being
    /// printed, outermost first.
    ///
    /// Each one introduces one more template parameter level above the lambda
    /// signature it belongs to.
    template_template_decls: Vec<NodeRef<'a>>,
    /// Template parameters currently being resolved, keyed by node address.
    ///
    /// A malformed symbol can make a parameter refer to itself through a
    /// substitution; a reference already on this stack prints as nothing.
    resolving: Vec<usize>,
    /// Invented `$T`-style indices, counted per parameter kind in print order.
    synthetic_indices: BTreeMap<usize, usize>,
    /// Next invented index for each parameter kind: type, non-type, template.
    synthetic_next: [usize; 3],
    /// Template parameter resolutions, keyed by node address.
    ///
    /// The AST resolves a parameter reference to the argument list of the
    /// template that lexically contains it, which is fixed once and for all;
    /// caching the first resolution keeps a node shared through a substitution
    /// printing the same way everywhere.
    resolved: BTreeMap<usize, Option<NodeRef<'a>>>,
    /// Base name of the name component most recently printed.
    base: Option<BaseName<'a>>,
    /// Depth of open parentheses inside template arguments.
    paren_depth: u32,
    /// True while printing template arguments.
    inside_template: bool,
    depth: u32,
    visits: usize,
}

impl<'a> Printer<'a> {
    fn new() -> Self {
        Self {
            out: String::new(),
            pack_index: NO_PACK,
            pack_max: NO_PACK,
            scopes: Vec::new(),
            lambda_params: Vec::new(),
            template_template_decls: Vec::new(),
            resolving: Vec::new(),
            synthetic_indices: BTreeMap::new(),
            synthetic_next: [0; 3],
            resolved: BTreeMap::new(),
            base: None,
            paren_depth: 0,
            inside_template: false,
            depth: 0,
            visits: 0,
        }
    }

    // -- buffer plumbing ---------------------------------------------------

    fn enter(&mut self) -> fmt::Result {
        self.visits = self.visits.checked_add(1).ok_or(fmt::Error)?;
        if self.visits > MAX_PRINT_VISITS {
            return Err(fmt::Error);
        }
        self.depth = self.depth.checked_add(1).ok_or(fmt::Error)?;
        if self.depth > MAX_PRINT_DEPTH {
            self.depth -= 1;
            return Err(fmt::Error);
        }
        Ok(())
    }

    fn leave(&mut self) {
        self.depth -= 1;
    }

    fn write(&mut self, text: &str) -> fmt::Result {
        if self.out.len().checked_add(text.len()).is_none_or(|len| len > MAX_OUTPUT_BYTES) {
            return Err(fmt::Error);
        }
        self.out.push_str(text);
        Ok(())
    }

    fn write_char(&mut self, ch: char) -> fmt::Result {
        if self.out.len() >= MAX_OUTPUT_BYTES {
            return Err(fmt::Error);
        }
        self.out.push(ch);
        Ok(())
    }

    fn write_decimal(&mut self, value: usize) -> fmt::Result {
        let mut buffer = [0_u8; 20];
        let mut index = buffer.len();
        let mut value = value;
        loop {
            index -= 1;
            buffer[index] = b'0' + u8::try_from(value % 10).unwrap_or(0);
            value /= 10;
            if value == 0 {
                break;
            }
        }
        let text = core::str::from_utf8(&buffer[index..]).map_err(|_| fmt::Error)?;
        self.write(text)
    }

    fn position(&self) -> usize {
        self.out.len()
    }

    fn truncate(&mut self, position: usize) {
        self.out.truncate(position);
    }

    fn last_char(&self) -> Option<char> {
        self.out.chars().next_back()
    }

    /// Open a parenthesis, tracking it inside template arguments.
    fn print_open(&mut self, ch: char) -> fmt::Result {
        if self.inside_template {
            self.paren_depth += 1;
        }
        self.write_char(ch)
    }

    /// Close a parenthesis opened with [`Self::print_open`].
    fn print_close(&mut self, ch: char) -> fmt::Result {
        if self.inside_template {
            self.paren_depth = self.paren_depth.saturating_sub(1);
        }
        self.write_char(ch)
    }

    fn in_parens_in_template_args(&self) -> bool {
        self.paren_depth > 0
    }

    fn required(node: &NodeRef<'a>, index: usize) -> Result<NodeRef<'a>, fmt::Error> {
        node.child(index).ok_or(fmt::Error)
    }

    // -- the left/right protocol ------------------------------------------

    /// Print a node: its left part and, if it has one, its right part.
    fn print(&mut self, node: NodeRef<'a>) -> fmt::Result {
        self.enter()?;
        let result = (|| {
            self.print_left(node.clone())?;
            if self.has_rhs(&node)? {
                self.print_right(node)?;
            }
            Ok(())
        })();
        self.leave();
        result
    }

    /// Print a node as an operand, parenthesising it when its precedence is
    /// weaker than `precedence`.
    fn print_as_operand(
        &mut self,
        node: NodeRef<'a>,
        precedence: Precedence,
        strictly_weaker: bool,
    ) -> fmt::Result {
        let paren = self.precedence(&node)? >= precedence as u8 + u8::from(strictly_weaker);
        if paren {
            self.print_open('(')?;
        }
        self.print(node)?;
        if paren {
            self.print_close(')')?;
        }
        Ok(())
    }

    /// Print a comma-separated list of operands, dropping separators whose
    /// element printed nothing (an empty pack expansion).
    fn print_with_comma(&mut self, nodes: &[NodeRef<'a>]) -> fmt::Result {
        let mut first = true;
        for node in nodes {
            let before = self.position();
            if !first {
                self.write(", ")?;
            }
            let after = self.position();
            self.print_as_operand(node.clone(), Precedence::Comma, false)?;
            if after == self.position() {
                self.truncate(before);
                continue;
            }
            first = false;
        }
        Ok(())
    }

    /// True when the node has a component that prints after its children.
    ///
    /// Only the qualified-type wrapper, packs and resolved template parameters
    /// forward this property: a pointer, reference or member pointer is itself
    /// printed whole once its pointee's right part is done.
    fn has_rhs(&mut self, node: &NodeRef<'a>) -> Result<bool, fmt::Error> {
        self.type_property(node, TypeProperty::Right)
    }

    /// True for a (possibly qualified) array type.
    fn has_array(&mut self, node: &NodeRef<'a>) -> Result<bool, fmt::Error> {
        self.type_property(node, TypeProperty::Array)
    }

    /// True for a (possibly qualified) function type.
    fn has_function(&mut self, node: &NodeRef<'a>) -> Result<bool, fmt::Error> {
        self.type_property(node, TypeProperty::Function)
    }

    fn type_property(
        &mut self,
        node: &NodeRef<'a>,
        property: TypeProperty,
    ) -> Result<bool, fmt::Error> {
        Ok(match node.kind() {
            Kind::ArrayType => property != TypeProperty::Function,
            Kind::FunctionType => property != TypeProperty::Array,
            Kind::QualifiedType | Kind::AbiTag => {
                let inner = Self::required(node, 0)?;
                self.type_property(&inner, property)?
            }
            Kind::PointerType
            | Kind::ReferenceType
            | Kind::RvalueReferenceType => {
                if property == TypeProperty::Right {
                    let inner = Self::required(node, 0)?;
                    self.type_property(&inner, property)?
                } else {
                    false
                }
            }
            Kind::MemberPointerType => {
                if property == TypeProperty::Right {
                    let member = Self::required(node, 1)?;
                    self.type_property(&member, property)?
                } else {
                    false
                }
            }
            Kind::Function => property == TypeProperty::Right,
            Kind::TemplateParam => {
                if self.lambda_param(node).is_some() || !self.begin_resolve(node) {
                    false
                } else {
                    let result = match self.resolve_template_param(node) {
                        Ok(Some(argument)) => self.type_property(&argument, property),
                        Ok(None) => Ok(false),
                        Err(error) => Err(error),
                    };
                    self.end_resolve();
                    result?
                }
            }
            // A `J...E` argument pack reports the property of the element it
            // is currently printing; a `Dp` expansion reports none.
            Kind::Pack => {
                if !self.is_argument_pack(node)? {
                    return Ok(false);
                }
                self.start_pack_expansion(node);
                match node.child(self.pack_index) {
                    Some(element) => self.type_property(&element, property)?,
                    None => false,
                }
            }
            _ => false,
        })
    }

    /// The expression precedence of a node; declarations and names are
    /// primary.
    fn precedence(&mut self, node: &NodeRef<'a>) -> Result<u8, fmt::Error> {
        let precedence = match node.kind() {
            Kind::Expression => expression_precedence(node)?,
            Kind::TemplateParam => {
                if self.lambda_param(node).is_some() || !self.begin_resolve(node) {
                    Precedence::Primary
                } else {
                    let result = match self.resolve_template_param(node) {
                        Ok(Some(argument)) => self.precedence(&argument),
                        Ok(None) => Ok(Precedence::Primary as u8),
                        Err(error) => Err(error),
                    };
                    self.end_resolve();
                    return result;
                }
            }
            _ => Precedence::Primary,
        };
        Ok(precedence as u8)
    }

    // -- left parts --------------------------------------------------------

    fn print_left(&mut self, node: NodeRef<'a>) -> fmt::Result {
        match node.kind() {
            Kind::Data => {
                let name = Self::required(&node, 0)?;
                let template_args = name_template_args(&name);
                if let Some(args) = &template_args {
                    self.scopes.push(args.clone());
                }
                let result = self.print(name);
                if template_args.is_some() {
                    let _ = self.scopes.pop();
                }
                result
            }
            Kind::Encoding | Kind::UnqualifiedName | Kind::Parameter => {
                self.print(Self::required(&node, 0)?)
            }
            Kind::Function => self.print_function_left(&node),
            Kind::VTable => self.print_special_name("vtable for ", &node),
            Kind::Vtt => self.print_special_name("VTT for ", &node),
            Kind::TypeInfo => self.print_special_name("typeinfo for ", &node),
            Kind::TypeInfoName => self.print_special_name("typeinfo name for ", &node),
            Kind::GuardVariable => self.print_special_name("guard variable for ", &node),
            Kind::ReferenceTemporary => self.print_special_name("reference temporary for ", &node),
            Kind::TlsInit => {
                self.print_special_name("thread-local initialization routine for ", &node)
            }
            Kind::TlsWrapper => self.print_special_name("thread-local wrapper routine for ", &node),
            Kind::CtorVtable => {
                self.write("construction vtable for ")?;
                self.print(Self::required(&node, 1)?)?;
                self.write("-in-")?;
                self.print(Self::required(&node, 0)?)
            }
            Kind::Thunk => self.print_thunk(&node),
            Kind::TemplateParamObject => {
                self.print_special_name("template parameter object for ", &node)
            }
            Kind::Name | Kind::NestedName | Kind::LocalName | Kind::AbiTag => {
                self.print_name(node)
            }
            Kind::Identifier | Kind::Substitution => self.print_identifier(&node),
            Kind::VendorQualifier => self.write(node.text().ok_or(fmt::Error)?),
            Kind::OperatorName => self.print_operator_name(&node),
            Kind::CtorDtorName => self.print_ctor_dtor(&node),
            Kind::UnnamedType => {
                // The AST stores the one-based type index; the ABI number is
                // one less.
                let index = node.text().and_then(parse_usize).ok_or(fmt::Error)?;
                self.write("'unnamed")?;
                self.write_decimal(index.saturating_sub(1))?;
                self.write("'")
            }
            Kind::Lambda => self.print_lambda(&node),
            Kind::TemplateArgs => self.print_template_args(&node),
            Kind::TemplateParam => {
                if let Some(lambda) = self.lambda_param(&node) {
                    return self.print_lambda_template_param(&node, &lambda);
                }
                if !self.begin_resolve(&node) {
                    return Ok(());
                }
                let result = match self.resolve_template_param(&node) {
                    Ok(Some(argument)) => self.print_template_param_argument(&argument),
                    Ok(None) => Err(fmt::Error),
                    Err(error) => Err(error),
                };
                self.end_resolve();
                result
            }
            Kind::TemplateParamDecl => self.print_template_param_decl(&node),
            Kind::ParameterList => self.print_parameters(&node),
            Kind::Pack => self.print_pack(&node),
            Kind::Literal => self.print_literal(&node),
            Kind::Expression => self.print_expression(&node),
            Kind::Decltype => {
                self.write("decltype")?;
                self.print_open('(')?;
                self.print(Self::required(&node, 0)?)?;
                self.print_close(')')
            }
            Kind::VendorType => {
                // `u <source-name>`: a vendor extended type, printed as the
                // extended name.
                self.print(Self::required(&node, 0)?)
            }
            Kind::BuiltinType => self.write(node.text().ok_or(fmt::Error)?),
            Kind::PointerType => {
                let pointee = Self::required(&node, 0)?;
                self.print_left(pointee.clone())?;
                if self.has_array(&pointee)? {
                    self.write(" ")?;
                }
                if self.has_array(&pointee)? || self.has_function(&pointee)? {
                    self.write_char('(')?;
                }
                self.write("*")
            }
            Kind::ReferenceType | Kind::RvalueReferenceType => {
                let Some((rvalue, pointee)) = self.collapse_reference(&node)? else {
                    return Ok(());
                };
                self.print_left(pointee.clone())?;
                if self.has_array(&pointee)? {
                    self.write(" ")?;
                }
                if self.has_array(&pointee)? || self.has_function(&pointee)? {
                    self.write_char('(')?;
                }
                self.write(if rvalue { "&&" } else { "&" })
            }
            Kind::QualifiedType => {
                let child = Self::required(&node, 0)?;
                self.print_left(child.clone())?;
                if child.kind() != Kind::FunctionType {
                    self.print_qualifiers(&node)?;
                }
                Ok(())
            }
            Kind::ArrayType => {
                let base = Self::required(&node, node.child_count() - 1)?;
                self.print_left(base)
            }
            Kind::FunctionType => {
                self.print_left(Self::required(&node, 0)?)?;
                self.write(" ")
            }
            Kind::MemberPointerType => {
                let member = Self::required(&node, 1)?;
                self.print_left(member.clone())?;
                if self.has_array(&member)? || self.has_function(&member)? {
                    self.write_char('(')?;
                } else {
                    self.write(" ")?;
                }
                self.print(Self::required(&node, 0)?)?;
                self.write("::*")
            }
            Kind::Opaque => Err(fmt::Error),
            // A kind this printer does not know can only come from a newer AST
            // vocabulary. Refuse to print rather than emit misleading text.
            #[allow(unreachable_patterns)]
            _ => Err(fmt::Error),
        }
    }

    // -- right parts -------------------------------------------------------

    fn print_right(&mut self, node: NodeRef<'a>) -> fmt::Result {
        match node.kind() {
            Kind::PointerType => {
                let pointee = Self::required(&node, 0)?;
                if self.has_array(&pointee)? || self.has_function(&pointee)? {
                    self.write_char(')')?;
                }
                self.print_right(pointee)
            }
            Kind::ReferenceType | Kind::RvalueReferenceType => {
                let Some((_, pointee)) = self.collapse_reference(&node)? else {
                    return Ok(());
                };
                if self.has_array(&pointee)? || self.has_function(&pointee)? {
                    self.write_char(')')?;
                }
                self.print_right(pointee)
            }
            Kind::QualifiedType => {
                let child = Self::required(&node, 0)?;
                if child.kind() == Kind::FunctionType {
                    // The qualifiers belong to the function type: they print
                    // before its ref-qualifier.
                    self.print_function_type_right(&child, node.text())?;
                    Ok(())
                } else {
                    self.print_right(child)
                }
            }
            Kind::ArrayType => {
                if self.last_char() != Some(']') {
                    self.write(" ")?;
                }
                self.write_char('[')?;
                match (node.text(), node.child_count()) {
                    (Some(dimension), _) => self.write(dimension)?,
                    (None, 2) => self.print(Self::required(&node, 0)?)?,
                    _ => {}
                }
                self.write_char(']')?;
                let base = Self::required(&node, node.child_count() - 1)?;
                self.print_right(base)
            }
            Kind::FunctionType => self.print_function_type_right(&node, None),
            Kind::MemberPointerType => {
                let member = Self::required(&node, 1)?;
                if self.has_array(&member)? || self.has_function(&member)? {
                    self.write_char(')')?;
                }
                self.print_right(member)
            }
            Kind::Function => self.print_function_right(&node),
            Kind::TemplateParam => {
                if self.lambda_param(&node).is_some() {
                    // A lambda's artificial parameter has no right part; its
                    // name was printed by the declarator.
                    return Ok(());
                }
                if !self.begin_resolve(&node) {
                    return Ok(());
                }
                let result = match self.resolve_template_param(&node) {
                    Ok(Some(argument)) => self.print_template_param_right(&argument),
                    Ok(None) => Ok(()),
                    Err(error) => Err(error),
                };
                self.end_resolve();
                result
            }
            Kind::Pack => self.print_pack_right(&node),
            _ => Ok(()),
        }
    }

    /// Print the cv-qualifiers of a qualified type.
    fn print_qualifiers(&mut self, node: &NodeRef<'a>) -> fmt::Result {
        let text = node.text().ok_or(fmt::Error)?;
        self.write(" ")?;
        self.write(text)
    }

    /// Print the right part of a function type: parameters, the return type's
    /// right part, then any cv-qualifiers and the ref-qualifier.
    fn print_function_type_right(
        &mut self,
        node: &NodeRef<'a>,
        cv_qualifiers: Option<&str>,
    ) -> fmt::Result {
        self.print_open('(')?;
        self.print_parameters(&Self::required(node, 1)?)?;
        self.print_close(')')?;
        self.print_right(Self::required(node, 0)?)?;
        if let Some(qualifiers) = cv_qualifiers {
            self.write(" ")?;
            self.write(qualifiers)?;
        }
        let Some(text) = node.text() else {
            return Ok(());
        };
        let ref_qualifier = text.strip_prefix("extern \"C\"").map_or(text, str::trim);
        if ref_qualifier.is_empty() {
            return Ok(());
        }
        self.write(" ")?;
        self.write(ref_qualifier)
    }

    /// Follow a chain of references, collapsing it to a single reference kind.
    /// Returns `None` when the chain refers back to itself through a template
    /// parameter; such a reference prints as nothing.
    fn collapse_reference(
        &mut self,
        node: &NodeRef<'a>,
    ) -> Result<Option<(bool, NodeRef<'a>)>, fmt::Error> {
        let mut rvalue = node.kind() == Kind::RvalueReferenceType;
        let mut current = self.syntax_node(&Self::required(node, 0)?)?;
        let mut visited: Vec<usize> = Vec::new();
        loop {
            match current.kind() {
                Kind::RvalueReferenceType | Kind::ReferenceType => {
                    if current.kind() == Kind::ReferenceType {
                        rvalue = false;
                    }
                    let inner = Self::required(&current, 0)?;
                    if inner.kind() == Kind::TemplateParam {
                        let address = Rc::as_ptr(&inner) as usize;
                        if visited.contains(&address) {
                            return Ok(None);
                        }
                        visited.push(address);
                    }
                    current = self.syntax_node(&inner)?;
                }
                _ => return Ok(Some((rvalue, current))),
            }
        }
    }

    /// Mark `node` as being resolved; false when it already is.
    fn begin_resolve(&mut self, node: &NodeRef<'a>) -> bool {
        let address = Rc::as_ptr(node) as usize;
        if self.resolving.contains(&address) {
            false
        } else {
            self.resolving.push(address);
            true
        }
    }

    fn end_resolve(&mut self) {
        let _ = self.resolving.pop();
    }

    /// Resolve the node that actually carries the syntax: template parameters
    /// are replaced by the argument they refer to.
    fn syntax_node(&mut self, node: &NodeRef<'a>) -> Result<NodeRef<'a>, fmt::Error> {
        let mut current = node.clone();
        let mut visited: Vec<usize> = Vec::new();
        loop {
            match current.kind() {
                Kind::TemplateParam => {
                    if self.lambda_param(&current).is_some() {
                        return Ok(current);
                    }
                    let address = Rc::as_ptr(&current) as usize;
                    if visited.contains(&address) {
                        return Ok(current);
                    }
                    visited.push(address);
                    let Some(argument) = self.resolve_template_param(&current)? else {
                        return Ok(current);
                    };
                    if self.is_pack_table_entry(&argument)? {
                        self.start_pack_expansion(&argument);
                        match argument.child(self.pack_index) {
                            Some(element) => current = element,
                            None => return Ok(argument),
                        }
                    } else {
                        current = argument;
                    }
                }
                _ => return Ok(current),
            }
        }
    }

    // -- names -------------------------------------------------------------

    fn print_special_name(&mut self, prefix: &str, node: &NodeRef<'a>) -> fmt::Result {
        self.write(prefix)?;
        self.print(Self::required(node, 0)?)
    }

    fn print_thunk(&mut self, node: &NodeRef<'a>) -> fmt::Result {
        let offset = node.text().ok_or(fmt::Error)?;
        if offset.matches("offset(").count() > 1 {
            self.write("covariant return thunk to ")?;
        } else if offset.starts_with("virtual offset") {
            self.write("virtual thunk to ")?;
        } else {
            self.write("non-virtual thunk to ")?;
        }
        self.print(Self::required(node, 0)?)
    }

    fn print_identifier(&mut self, node: &NodeRef<'a>) -> fmt::Result {
        let text = node.text().ok_or(fmt::Error)?;
        if node.kind() == Kind::Substitution {
            return self.write(short_substitution(text));
        }
        if text.starts_with("_GLOBAL__N") {
            self.write("(anonymous namespace)")
        } else {
            self.write(text)
        }
    }

    fn print_operator_name(&mut self, node: &NodeRef<'a>) -> fmt::Result {
        let text = node.text().ok_or(fmt::Error)?;
        self.write(text)?;
        // A conversion operator stores only `operator`; its target type is the
        // single child.
        if text == "operator" {
            let target = Self::required(node, 0)?;
            self.write(" ")?;
            self.print(target)?;
        }
        Ok(())
    }

    fn print_ctor_dtor(&mut self, node: &NodeRef<'a>) -> fmt::Result {
        let code = node.text().ok_or(fmt::Error)?;
        let is_dtor = code.starts_with('D');
        let base = self.base.take().ok_or(fmt::Error)?;
        let result = (|| {
            if is_dtor {
                self.write("~")?;
            }
            match &base {
                BaseName::Text(text) => self.write(text),
                BaseName::Name(node) => self.print_identifier(&node.clone()),
            }
        })();
        self.base = Some(base);
        result
    }

    fn print_lambda(&mut self, node: &NodeRef<'a>) -> fmt::Result {
        self.write("'lambda")?;
        // The AST stores the one-based closure index; the ABI number is two
        // less, and is omitted for the first closure.
        if let Some(index) = node.text().and_then(parse_usize)
            && index > 1
        {
            self.write_decimal(index - 2)?;
        }
        self.write("'")?;
        self.print_lambda_declarator(node)
    }

    /// Print `<template-params> (params)` of a lambda.
    fn print_lambda_declarator(&mut self, node: &NodeRef<'a>) -> fmt::Result {
        let (decls, params) = split_lambda_children(node);
        if !decls.is_empty() {
            let saved = core::mem::replace(&mut self.inside_template, true);
            self.write_char('<')?;
            for (index, decl) in decls.iter().enumerate() {
                if index > 0 {
                    self.write(", ")?;
                }
                self.print_template_param_decl(decl)?;
            }
            self.write_char('>')?;
            self.inside_template = saved;
        }
        self.print_open('(')?;
        self.print_with_comma(&params)?;
        self.print_close(')')
    }

    fn print_template_param_decl(&mut self, node: &NodeRef<'a>) -> fmt::Result {
        let keyword = node.text().ok_or(fmt::Error)?;
        match keyword {
            // `Ty`: an unnamed type parameter.
            "Ty" => {
                self.write("typename ")?;
                self.print_decl_name(node)
            }
            // `Tn <type>`: an unnamed non-type parameter.
            "Tn" => {
                let ty = Self::required(node, 0)?;
                self.print_left(ty.clone())?;
                if !self.has_rhs(&ty)? {
                    self.write(" ")?;
                }
                self.print_decl_name(node)?;
                if self.has_rhs(&ty)? {
                    self.print_right(ty)?;
                }
                Ok(())
            }
            // `Tk <name> [<template-args>]`: a constrained type parameter.
            "Tk" => {
                self.print(Self::required(node, 0)?)?;
                self.write(" ")?;
                self.print_decl_name(node)
            }
            // `Tt <decl>* E`: a template template parameter.
            "Tt" => {
                let saved = core::mem::replace(&mut self.inside_template, true);
                self.write("template<")?;
                self.template_template_decls.push(node.clone());
                for (position, decl) in node.children().iter().enumerate() {
                    if position > 0 {
                        self.write(", ")?;
                    }
                    self.print_template_param_decl(decl)?;
                }
                let _ = self.template_template_decls.pop();
                self.write("> typename ")?;
                self.inside_template = saved;
                self.print_decl_name(node)
            }
            // `Tp <decl>`: a parameter pack declaration. The ellipsis sits in
            // the declarator, between the type's left part and the name.
            "Tp" => {
                let inner = Self::required(node, 0)?;
                match inner.text() {
                    Some("Tn") => {
                        let ty = Self::required(&inner, 0)?;
                        self.print_left(ty.clone())?;
                        if !self.has_rhs(&ty)? {
                            self.write(" ")?;
                        }
                        self.write("...")?;
                        self.print_decl_name(&inner)?;
                        if self.has_rhs(&ty)? {
                            self.print_right(ty)?;
                        }
                        Ok(())
                    }
                    Some("Ty") => {
                        self.write("typename ...")?;
                        self.print_decl_name(&inner)
                    }
                    Some("Tk") => {
                        self.print(Self::required(&inner, 0)?)?;
                        self.write(" ...")?;
                        self.print_decl_name(&inner)
                    }
                    _ => Err(fmt::Error),
                }
            }
            _ => Err(fmt::Error),
        }
    }

    /// Print the `$T`-style name of a template parameter declaration.
    ///
    /// The invented index is global to the symbol and counts declarations of
    /// the same kind in print order, matching the reference demangler.
    fn print_decl_name(&mut self, node: &NodeRef<'a>) -> fmt::Result {
        let keyword = node.text().ok_or(fmt::Error)?;
        let kind = match keyword {
            "Ty" | "Tk" | "Tp" => 0,
            "Tn" => 1,
            "Tt" => 2,
            _ => return Err(fmt::Error),
        };
        let prefix = match kind {
            0 => "$T",
            1 => "$N",
            _ => "$TT",
        };
        self.write(prefix)?;
        let address = Rc::as_ptr(node) as usize;
        let index = match self.synthetic_indices.get(&address) {
            Some(index) => *index,
            None => {
                let index = self.synthetic_next[kind];
                self.synthetic_next[kind] = index.checked_add(1).ok_or(fmt::Error)?;
                let _ = self.synthetic_indices.insert(address, index);
                index
            }
        };
        if index > 0 {
            self.write_decimal(index - 1)?;
        }
        Ok(())
    }

    fn print_template_args(&mut self, node: &NodeRef<'a>) -> fmt::Result {
        // A `<template-param-decl>` inside an argument list declares a
        // parameter for the enclosing lambda; only its argument prints.
        let arguments: Vec<NodeRef<'a>> = node
            .children()
            .iter()
            .filter(|child| child.kind() != Kind::TemplateParamDecl)
            .cloned()
            .collect();
        let saved = core::mem::replace(&mut self.inside_template, true);
        self.write_char('<')?;
        let mut pending_vendors: Vec<NodeRef<'a>> = Vec::new();
        let mut first = true;
        for argument in &arguments {
            if argument.kind() == Kind::VendorQualifier {
                pending_vendors.push(argument.clone());
                continue;
            }
            let before = self.position();
            if !first {
                self.write(", ")?;
            }
            let after = self.position();
            self.print_as_operand(argument.clone(), Precedence::Comma, false)?;
            for vendor in core::mem::take(&mut pending_vendors) {
                self.write_char('@')?;
                self.print(vendor)?;
            }
            if after == self.position() {
                self.truncate(before);
                continue;
            }
            first = false;
        }
        self.write_char('>')?;
        self.inside_template = saved;
        Ok(())
    }

    /// Print a flattened name.
    fn print_name(&mut self, node: NodeRef<'a>) -> fmt::Result {
        let saved_base = self.base.take();
        let mut components = Vec::new();
        flatten_name(node, &mut components);
        let count = components.len();
        let mut pending_vendors: Vec<NodeRef<'a>> = Vec::new();
        let mut vendor_transform = false;
        let mut printed = false;
        for index in 0..count {
            let component = components[index].clone();
            let node = match component {
                Component::AbiTag(tag) => {
                    self.write("[abi:")?;
                    self.write(tag.text().ok_or(fmt::Error)?)?;
                    self.write_char(']')?;
                    continue;
                }
                Component::Node(node) => node,
            };
            match node.kind() {
                Kind::VendorQualifier => {
                    pending_vendors.push(node);
                    continue;
                }
                Kind::TemplateArgs => {
                    if vendor_transform {
                        // `u <source-name> I <type> E` transforms the type:
                        // `__remove_const(int const)`.
                        self.print_open('(')?;
                        self.print_with_comma(node.children())?;
                        self.print_close(')')?;
                    } else {
                        self.print_template_args(&node)?;
                    }
                    continue;
                }
                _ => {}
            }
            if printed {
                self.write("::")?;
            }
            if node.kind() == Kind::Substitution && is_ctor_prefix(&components, index) {
                // A well-known substitution used as a constructor's prefix
                // expands to the class template it abbreviates.
                let text = node.text().ok_or(fmt::Error)?;
                self.write(expanded_substitution(text))?;
                self.base = Some(BaseName::Text(substitution_leaf(text)));
            } else {
                self.print(node.clone())?;
                if let Some(base) = base_name(&node) {
                    self.base = Some(base);
                }
            }
            printed = true;
            vendor_transform = node.kind() == Kind::VendorType;
            for (position, vendor) in core::mem::take(&mut pending_vendors)
                .into_iter()
                .enumerate()
            {
                self.write(if position == 0 { "@" } else { "." })?;
                self.print(vendor)?;
            }
        }
        self.base = saved_base;
        Ok(())
    }

    // -- functions and parameters -----------------------------------------

    fn print_function_left(&mut self, node: &NodeRef<'a>) -> fmt::Result {
        let name = Self::required(node, 0)?;
        let template_args = name_template_args(&name);
        if let Some(args) = &template_args {
            self.scopes.push(args.clone());
        }
        let result = (|| {
            if node.child_count() == 3 {
                let return_type = Self::required(node, 1)?;
                self.print_left(return_type.clone())?;
                if !self.has_rhs(&return_type)? {
                    self.write(" ")?;
                }
            }
            self.print(name)
        })();
        if template_args.is_some() {
            let _ = self.scopes.pop();
        }
        result
    }

    fn print_function_right(&mut self, node: &NodeRef<'a>) -> fmt::Result {
        let name = Self::required(node, 0)?;
        let template_args = name_template_args(&name);
        if let Some(args) = &template_args {
            self.scopes.push(args.clone());
        }
        let result = (|| {
            self.print_open('(')?;
            self.print_parameters(&Self::required(node, node.child_count() - 1)?)?;
            self.print_close(')')?;
            if node.child_count() == 3 {
                self.print_right(Self::required(node, 1)?)?;
            }
            if let Some(qualifier) = name_qualifier_node(&name) {
                self.write(" ")?;
                self.write(qualifier.text().ok_or(fmt::Error)?)?;
            }
            Ok(())
        })();
        if template_args.is_some() {
            let _ = self.scopes.pop();
        }
        result
    }

    fn print_parameters(&mut self, node: &NodeRef<'a>) -> fmt::Result {
        let mut parameters = Vec::with_capacity(node.child_count());
        for parameter in node.children() {
            parameters.push(match parameter.kind() {
                Kind::Parameter => Self::required(parameter, 0)?,
                _ => parameter.clone(),
            });
        }
        self.print_with_comma(&parameters)
    }

    // -- template parameters and packs ------------------------------------

    fn resolve_template_param(
        &mut self,
        node: &NodeRef<'a>,
    ) -> Result<Option<NodeRef<'a>>, fmt::Error> {
        let text = node.text().ok_or(fmt::Error)?;
        let Some(index) = template_param_index(text) else {
            return Err(fmt::Error);
        };
        let address = Rc::as_ptr(node) as usize;
        if let Some(cached) = self.resolved.get(&address) {
            return Ok(cached.clone());
        }
        let mut found = None;
        for scope in self.scopes.iter().rev() {
            // Parameter declarations take a slot in the argument list but are
            // not arguments themselves.
            let mut argument_index = 0;
            for argument in scope.children() {
                if argument.kind() == Kind::TemplateParamDecl {
                    continue;
                }
                if argument_index == index {
                    found = Some(argument.clone());
                    break;
                }
                argument_index += 1;
            }
            if found.is_some() {
                break;
            }
        }
        let _ = self.resolved.insert(address, found.clone());
        Ok(found)
    }

    /// Print a resolved template parameter: an unexpanded pack prints its
    /// current element.
    fn print_template_param_argument(&mut self, argument: &NodeRef<'a>) -> fmt::Result {
        if self.is_pack_table_entry(argument)? {
            self.start_pack_expansion(argument);
            let index = self.pack_index;
            if index < argument.child_count() {
                let element = Self::required(argument, index)?;
                self.print_left(element)?;
            }
            return Ok(());
        }
        self.print_left(argument.clone())
    }

    fn print_template_param_right(&mut self, argument: &NodeRef<'a>) -> fmt::Result {
        if self.is_pack_table_entry(argument)? {
            if self.pack_index < argument.child_count() {
                let element = Self::required(argument, self.pack_index)?;
                self.print_right(element)?;
            }
            return Ok(());
        }
        self.print_right(argument.clone())
    }

    /// True for a `J...E` argument pack used as a template parameter table
    /// entry, which prints one element at a time.
    fn is_pack_table_entry(&self, node: &NodeRef<'a>) -> Result<bool, fmt::Error> {
        Ok(node.kind() == Kind::Pack && self.is_argument_pack(node)?)
    }

    /// Start a pack expansion unless one is already running.
    fn start_pack_expansion(&mut self, pack: &NodeRef<'a>) {
        if self.pack_max == NO_PACK {
            self.pack_max = pack.child_count();
            self.pack_index = 0;
        }
    }

    /// Print a template parameter reference inside a lambda signature.
    ///
    /// Uses of `auto` in a generic lambda are mangled as artificial template
    /// parameters: they print as the declared parameter name, or as `auto`
    /// when the declaration is not in the signature.
    fn print_lambda_template_param(
        &mut self,
        node: &NodeRef<'a>,
        lambda: &NodeRef<'a>,
    ) -> fmt::Result {
        let text = node.text().ok_or(fmt::Error)?;
        let (level, index) = template_param_level_index(text).ok_or(fmt::Error)?;
        let base = self.lambda_base_level(lambda);
        if level < base {
            // A reference to an enclosing template argument list.
            let Some(scope) = self.scopes.get(level).cloned() else {
                return self.write("auto");
            };
            let mut argument_index = 0;
            for argument in scope.children() {
                if argument.kind() == Kind::TemplateParamDecl {
                    continue;
                }
                if argument_index == index {
                    return self.print_template_param_argument(argument);
                }
                argument_index += 1;
            }
            return self.write("auto");
        }
        if level == base {
            // A reference to one of the lambda's own declarations.
            let (decls, _) = split_lambda_children(lambda);
            return match decls.get(index) {
                Some(decl) => self.print_decl_name(decl),
                None => self.write("auto"),
            };
        }
        // A reference to the declarations of an enclosing `Tt` parameter.
        let depth = level - base - 1;
        if let Some(tt) = self.template_template_decls.get(depth).cloned()
            && let Some(decl) = tt.child(index)
        {
            return self.print_decl_name(&decl);
        }
        Err(fmt::Error)
    }

    /// The template parameter level of a lambda's own declarations.
    ///
    /// A lambda's declarations sit one level above every template argument
    /// list that was already in scope where the lambda appears. Scopes that
    /// start after the lambda (a conversion operator's own arguments) were not
    /// in scope while it was parsed.
    fn lambda_base_level(&self, lambda: &NodeRef<'a>) -> usize {
        let Some(lambda_span) = lambda.span() else {
            return 0;
        };
        self.scopes
            .iter()
            .filter(|scope| {
                scope
                    .span()
                    .is_some_and(|span| span.start < lambda_span.start)
            })
            .count()
    }

    /// The lambda whose signature contains `node`, if any.
    fn lambda_param(&self, node: &NodeRef<'a>) -> Option<NodeRef<'a>> {
        let address = Rc::as_ptr(node) as usize;
        self.lambda_params
            .iter()
            .find(|(param, _)| *param == address)
            .map(|(_, lambda)| lambda.clone())
    }

    /// Record the template parameters that appear in every lambda signature.
    fn collect_lambda_params(&mut self, root: &NodeRef<'a>) {
        let mut pending = alloc::vec![root.clone()];
        while let Some(node) = pending.pop() {
            if node.kind() == Kind::Lambda {
                // Declarations and parameter types are both part of the
                // signature; a nested lambda owns its own references.
                let mut inner: Vec<NodeRef<'a>> = node.children().to_vec();
                while let Some(parameter) = inner.pop() {
                    if parameter.kind() == Kind::Lambda {
                        continue;
                    }
                    // A node shared through a substitution may have been
                    // created outside the signature; only nodes whose span lies
                    // inside the lambda were written as `auto`.
                    if parameter.kind() == Kind::TemplateParam && is_within(&node, &parameter) {
                        let address = Rc::as_ptr(&parameter) as usize;
                        self.lambda_params.push((address, node.clone()));
                    }
                    inner.extend(parameter.children().iter().cloned());
                }
            }
            pending.extend(node.children().iter().cloned());
        }
    }

    /// True for a `J...E` argument pack, false for a `Dp <type>` expansion.
    ///
    /// Both share [`Kind::Pack`]; the parser records the encoded form in the
    /// node text. ASTs built before that marker fall back to the offset of the
    /// last child's span, which is one byte short of the closing `E` for a
    /// `J...E` pack and ends together with a `Dp` pattern.
    fn is_argument_pack(&self, node: &NodeRef<'a>) -> Result<bool, fmt::Error> {
        if let Some(text) = node.text() {
            return Ok(text == "J");
        }
        let Some(span) = node.span() else {
            return Err(fmt::Error);
        };
        let Some(last) = node.child(node.child_count().saturating_sub(1)) else {
            // An empty pack can only be the `J...E` form.
            return Ok(true);
        };
        let child_span = last.span().ok_or(fmt::Error)?;
        // `Dp <type>` ends with its pattern, so the spans end together.
        // `J <template-arg>* E` ends with the closing `E`; an `X <expr> E`
        // argument already ends with its own delimiter, so the gap can be two
        // bytes.
        match span.end.checked_sub(child_span.end) {
            Some(0) => Ok(false),
            Some(1 | 2) => Ok(true),
            _ => Err(fmt::Error),
        }
    }

    fn print_pack(&mut self, node: &NodeRef<'a>) -> fmt::Result {
        if self.is_argument_pack(node)? {
            // A `J...E` argument pack prints its elements.
            return self.print_with_comma(node.children());
        }
        self.print_pack_expansion(node)
    }

    fn print_pack_right(&mut self, node: &NodeRef<'a>) -> fmt::Result {
        if self.is_argument_pack(node)? {
            return Ok(());
        }
        let pattern = Self::required(node, 0)?;
        self.print_right(pattern)
    }

    /// Print a `Dp <type>` pack expansion: the pattern once per pack element.
    fn print_pack_expansion(&mut self, node: &NodeRef<'a>) -> fmt::Result {
        let pattern = Self::required(node, 0)?;
        self.print_pack_expansion_pattern(pattern)
    }

    /// Print `pattern` once per element of the pack it references.
    fn print_pack_expansion_pattern(&mut self, pattern: NodeRef<'a>) -> fmt::Result {
        let saved_index = self.pack_index;
        let saved_max = self.pack_max;
        self.pack_index = NO_PACK;
        self.pack_max = NO_PACK;
        let start = self.position();
        self.print(pattern.clone())?;
        match self.pack_max {
            // No pack below the pattern: a non-dependent expansion.
            NO_PACK => self.write("...")?,
            // An empty pack prints nothing at all.
            0 => self.truncate(start),
            max => {
                for index in 1..max {
                    self.write(", ")?;
                    self.pack_index = index;
                    self.print(pattern.clone())?;
                }
            }
        }
        self.pack_index = saved_index;
        self.pack_max = saved_max;
        Ok(())
    }

    // -- literals and expressions -----------------------------------------

    fn print_literal(&mut self, node: &NodeRef<'a>) -> fmt::Result {
        let ty = Self::required(node, 0)?;
        if !is_type(ty.kind()) {
            // `L <mangled-name> E` names an entity.
            return self.print(ty);
        }
        // A lambda in expression position prints as a closure expression.
        if ty.kind() == Kind::Lambda {
            self.write("[]")?;
            self.print_lambda_declarator(&ty)?;
            return self.write("{...}");
        }
        let Some(value) = node.text() else {
            // A string literal has no encoded value.
            self.write("\"<")?;
            self.print(ty)?;
            return self.write(">\"");
        };
        match ty.kind() {
            Kind::BuiltinType => {}
            _ => {
                self.print_open('(')?;
                self.print(ty)?;
                self.print_close(')')?;
                return self.print_integer_value(value, "");
            }
        }
        let name = ty.text().ok_or(fmt::Error)?;
        match name {
            "bool" => match value {
                "0" => self.write("false"),
                "1" => self.write("true"),
                _ => {
                    self.write("(bool)")?;
                    self.write(value)
                }
            },
            "std::nullptr_t" => self.write("nullptr"),
            _ => {
                let suffix = match name {
                    "int" => "",
                    "unsigned int" => "u",
                    "long" => "l",
                    "unsigned long" => "ul",
                    "long long" => "ll",
                    "unsigned long long" => "ull",
                    _ => {
                        // Every other type is spelled out in parentheses.
                        self.print_open('(')?;
                        self.print(ty)?;
                        self.print_close(')')?;
                        ""
                    }
                };
                self.print_integer_value(value, suffix)
            }
        }
    }

    /// Print an encoded integer value, with a leading `n` meaning negative.
    fn print_integer_value(&mut self, value: &str, suffix: &str) -> fmt::Result {
        let digits = if let Some(digits) = value.strip_prefix('n') {
            self.write_char('-')?;
            digits
        } else {
            value
        };
        self.write(digits)?;
        self.write(suffix)
    }

    fn print_expression(&mut self, node: &NodeRef<'a>) -> fmt::Result {
        let form = node.text().ok_or(fmt::Error)?;
        match form {
            "call" => {
                let callee = Self::required(node, 0)?;
                self.print(callee)?;
                self.print_open('(')?;
                self.print_with_comma(&node.children()[1..])?;
                self.print_close(')')
            }
            "conversion" => {
                let ty = Self::required(node, 0)?;
                self.print_open('(')?;
                self.print(ty)?;
                self.print_close(')')?;
                self.print_open('(')?;
                self.print_with_comma(&node.children()[1..])?;
                self.print_close(')')
            }
            "type-list" => self.print_init_list(node),
            "initializer-list" => {
                self.write_char('{')?;
                self.print_with_comma(node.children())?;
                self.write_char('}')
            }
            "static_cast" | "dynamic_cast" | "const_cast" | "reinterpret_cast" => {
                let ty = Self::required(node, 0)?;
                let operand = Self::required(node, 1)?;
                self.write(form)?;
                let saved = core::mem::replace(&mut self.inside_template, true);
                self.write_char('<')?;
                self.print(ty)?;
                self.write_char('>')?;
                self.inside_template = saved;
                self.print_open('(')?;
                self.print_as_operand(operand, Precedence::Default, false)?;
                self.print_close(')')
            }
            "typeid" | "sizeof" | "alignof" | "noexcept" => {
                self.write(form)?;
                self.write(" ")?;
                self.print_open('(')?;
                self.print(Self::required(node, 0)?)?;
                self.print_close(')')
            }
            "throw" => {
                self.write("throw")?;
                if let Some(operand) = node.child(0) {
                    self.write(" ")?;
                    self.print(operand)?;
                }
                Ok(())
            }
            "member" => {
                let object = Self::required(node, 0)?;
                let member = Self::required(node, 1)?;
                self.print_as_operand(object, Precedence::Postfix, true)?;
                self.write_char('.')?;
                self.print(member)
            }
            "pack" => self.print_pack_expansion(node),
            "sizeof-pack" => {
                self.write("sizeof...")?;
                self.print_open('(')?;
                self.print_pack_expansion_pattern(Self::required(node, 0)?)?;
                self.print_close(')')
            }
            "template-param" => self.print(Self::required(node, 0)?),
            _ => self.print_operator_expression(node, form),
        }
    }

    fn print_operator_expression(&mut self, node: &NodeRef<'a>, spelling: &str) -> fmt::Result {
        let symbol = spelling
            .strip_prefix("operator")
            .map_or(spelling, str::trim_start);
        let operands = node.children();
        match (operands, expression_precedence(node)?) {
            ([operand], precedence) => {
                // `pp`/`mm` are postfix in expression context.
                if matches!(spelling, "operator++" | "operator--") {
                    self.print_as_operand(operand.clone(), precedence, true)?;
                    return self.write(symbol);
                }
                self.write(symbol)?;
                self.print_as_operand(operand.clone(), precedence, false)
            }
            ([left, right], precedence) if matches!(symbol, "->" | "->*") => {
                // Member access operators print without spaces.
                self.print_as_operand(left.clone(), precedence, true)?;
                self.write(symbol)?;
                self.print_as_operand(right.clone(), precedence, false)
            }
            ([left, right], precedence) => {
                let paren_all = !self.in_parens_in_template_args()
                    && matches!(symbol, ">" | ">>");
                if paren_all {
                    self.print_open('(')?;
                }
                let assign = precedence == Precedence::Assign;
                self.print_as_operand(
                    left.clone(),
                    if assign { Precedence::OrIf } else { precedence },
                    !assign,
                )?;
                if symbol != "," {
                    self.write(" ")?;
                }
                self.write(symbol)?;
                self.write(" ")?;
                self.print_as_operand(right.clone(), precedence, assign)?;
                if paren_all {
                    self.print_close(')')?;
                }
                Ok(())
            }
            ([condition, consequent, alternative, ..], _) => {
                self.print_as_operand(condition.clone(), Precedence::Conditional, false)?;
                self.write(" ? ")?;
                self.print_as_operand(consequent.clone(), Precedence::Default, false)?;
                self.write(" : ")?;
                self.print_as_operand(alternative.clone(), Precedence::Assign, true)
            }
            _ => Err(fmt::Error),
        }
    }

    /// Print a braced initializer list, spelling out character arrays as
    /// string literals.
    fn print_init_list(&mut self, node: &NodeRef<'a>) -> fmt::Result {
        let ty = Self::required(node, 0)?;
        let inits = &node.children()[1..];
        if is_char_array(&ty)? && self.print_string_literal(inits)?.is_some() {
            return Ok(());
        }
        self.print(ty)?;
        self.write_char('{')?;
        self.print_with_comma(inits)?;
        self.write_char('}')
    }

    /// Print initializer elements as a C string literal, if they all are byte
    /// values.
    fn print_string_literal(&mut self, elements: &[NodeRef<'a>]) -> Result<Option<()>, fmt::Error> {
        let mut bytes = Vec::with_capacity(elements.len());
        for element in elements {
            let Some(value) = integer_literal_value(element) else {
                return Ok(None);
            };
            let value = value as u32;
            if value > 0xff {
                return Ok(None);
            }
            bytes.push(value as u8);
        }
        let Ok(text) = String::from_utf8(bytes) else {
            return Ok(None);
        };
        self.write_char('"')?;
        let mut last_numeric_escape = false;
        for value in text.chars() {
            // A numeric escape would swallow a following hex digit, so close
            // and reopen the literal.
            if last_numeric_escape && value.is_ascii_hexdigit() {
                self.write("\"\"")?;
            }
            last_numeric_escape = false;
            match value {
                '\u{7}' => self.write("\\a")?,
                '\u{8}' => self.write("\\b")?,
                '\u{c}' => self.write("\\f")?,
                '\n' => self.write("\\n")?,
                '\r' => self.write("\\r")?,
                '\t' => self.write("\\t")?,
                '\u{b}' => self.write("\\v")?,
                '"' => self.write("\\\"")?,
                '\\' => self.write("\\\\")?,
                value if (value as u32) < 32 || value == '\u{7f}' => {
                    const HEX: &[u8; 16] = b"0123456789ABCDEF";
                    let value = value as usize;
                    self.write_char('\\')?;
                    if value > 7 {
                        self.write_char('x')?;
                    }
                    if value >= 16 {
                        self.write_char(char::from(HEX[value >> 4]))?;
                    }
                    self.write_char(char::from(HEX[value & 0xf]))?;
                    last_numeric_escape = true;
                }
                value => self.write_char(value)?,
            }
        }
        self.write_char('"')?;
        Ok(Some(()))
    }
}

/// One component of a flattened name.
#[derive(Clone)]
enum Component<'a> {
    /// A name component, printed with `::` separators.
    Node(NodeRef<'a>),
    /// An ABI tag, printed directly after the name it tags.
    AbiTag(NodeRef<'a>),
}

/// Flatten a name into printable components.
fn flatten_name<'a>(node: NodeRef<'a>, out: &mut Vec<Component<'a>>) {
    match node.kind() {
        Kind::NestedName => {
            for child in node.children() {
                flatten_name(child.clone(), out);
            }
        }
        Kind::Name => {
            if let Some(base) = node.child(0) {
                flatten_name(base, out);
            }
            if let Some(args) = node.child(1) {
                out.push(Component::Node(args));
            }
        }
        Kind::LocalName => {
            if let Some(encoding) = node.child(0) {
                out.push(Component::Node(encoding));
            }
            if let Some(entity) = node.child(1) {
                flatten_name(entity, out);
            }
        }
        Kind::UnqualifiedName => match node.child(0) {
            Some(child) => flatten_name(child, out),
            None => out.push(Component::Node(node)),
        },
        Kind::AbiTag => {
            if let Some(inner) = node.child(0) {
                flatten_name(inner, out);
            }
            out.push(Component::AbiTag(node));
        }
        _ => out.push(Component::Node(node)),
    }
}

/// True when the component at `index` is followed by a constructor or
/// destructor name, which expands well-known substitutions.
fn is_ctor_prefix(components: &[Component<'_>], index: usize) -> bool {
    components[index + 1..].iter().any(|later| match later {
        Component::Node(node) => match node.kind() {
            Kind::TemplateArgs | Kind::VendorQualifier => false,
            Kind::CtorDtorName => true,
            _ => false,
        },
        Component::AbiTag(_) => false,
    })
}

/// True when `inner`'s span lies inside `outer`'s span.
fn is_within(outer: &NodeRef<'_>, inner: &NodeRef<'_>) -> bool {
    match (outer.span(), inner.span()) {
        (Some(outer), Some(inner)) => inner.start >= outer.start && inner.end <= outer.end,
        _ => false,
    }
}

/// The base name of a name component, used by constructors and destructors.
fn base_name<'a>(node: &NodeRef<'a>) -> Option<BaseName<'a>> {
    match node.kind() {
        Kind::Identifier => Some(BaseName::Name(node.clone())),
        Kind::Substitution => node.text().map(|text| BaseName::Text(substitution_leaf(text))),
        _ => None,
    }
}

/// The cv/ref qualifier of the name a function encoding prints.
fn name_qualifier_node<'a>(node: &NodeRef<'a>) -> Option<NodeRef<'a>> {
    match node.kind() {
        Kind::NestedName if node.text().is_some() => Some(node.clone()),
        Kind::LocalName => node
            .child(1)
            .and_then(|entity| name_qualifier_node(&entity)),
        Kind::Name => node.child(0).and_then(|base| name_qualifier_node(&base)),
        _ => None,
    }
}

/// The template argument list in scope for a function's parameters.
fn name_template_args<'a>(node: &NodeRef<'a>) -> Option<NodeRef<'a>> {
    match node.kind() {
        Kind::Name => node.child(1),
        // An inherited constructor's template parameter table is the argument
        // list of the class it inherits from.
        Kind::NestedName
        | Kind::LocalName
        | Kind::OperatorName
        | Kind::UnqualifiedName
        | Kind::CtorDtorName => node
            .children()
            .iter()
            .rev()
            .find_map(|child| name_template_args(child)),
        Kind::TemplateArgs => Some(node.clone()),
        _ => None,
    }
}

/// Split a lambda's children into parameter declarations and parameter types.
fn split_lambda_children<'a>(node: &NodeRef<'a>) -> (Vec<NodeRef<'a>>, Vec<NodeRef<'a>>) {
    let mut decls = Vec::new();
    let mut params = Vec::new();
    for child in node.children() {
        if child.kind() == Kind::TemplateParamDecl {
            decls.push(child.clone());
        } else {
            params.push(child.clone());
        }
    }
    (decls, params)
}

/// The precedence of an expression node.
fn expression_precedence(node: &NodeRef<'_>) -> Result<Precedence, fmt::Error> {
    let form = node.text().ok_or(fmt::Error)?;
    let precedence = match form {
        "call" | "static_cast" | "dynamic_cast" | "const_cast" | "reinterpret_cast"
        | "typeid" => Precedence::Postfix,
        "conversion" => Precedence::Cast,
        "sizeof" | "alignof" | "noexcept" => Precedence::Unary,
        "member" => Precedence::Postfix,
        "type-list" | "initializer-list" | "pack" | "sizeof-pack" | "template-param"
        | "throw" => Precedence::Primary,
        _ => operator_precedence(form, node.child_count())?,
    };
    Ok(precedence)
}

/// The precedence of an operator expression.
fn operator_precedence(spelling: &str, arity: usize) -> Result<Precedence, fmt::Error> {
    let precedence = match spelling {
        "operator?" => Precedence::Conditional,
        "operator," => Precedence::Comma,
        "operator->*" => Precedence::PtrMem,
        "operator->" => Precedence::Postfix,
        "operator=" | "operator+=" | "operator-=" | "operator*=" | "operator/="
        | "operator%=" | "operator<<=" | "operator>>=" | "operator&=" | "operator|="
        | "operator^=" => Precedence::Assign,
        "operator&&" => Precedence::AndIf,
        "operator||" => Precedence::OrIf,
        "operator|" => Precedence::Ior,
        "operator^" => Precedence::Xor,
        "operator==" | "operator!=" => Precedence::Equality,
        "operator<=>" => Precedence::Spaceship,
        "operator<<" | "operator>>" => Precedence::Shift,
        _ if arity == 1 => Precedence::Unary,
        "operator+" | "operator-" => Precedence::Additive,
        "operator*" | "operator/" | "operator%" => Precedence::Multiplicative,
        "operator<" | "operator>" | "operator<=" | "operator>=" => Precedence::Relational,
        "operator&" => Precedence::And,
        _ => return Err(fmt::Error),
    };
    Ok(precedence)
}

/// True for a `char` array type, whose initializer can print as a string.
fn is_char_array(node: &NodeRef<'_>) -> Result<bool, fmt::Error> {
    let mut current = node.clone();
    loop {
        match current.kind() {
            Kind::ArrayType => {
                current = current
                    .child(current.child_count() - 1)
                    .ok_or(fmt::Error)?;
            }
            Kind::BuiltinType => return Ok(current.text() == Some("char")),
            _ => return Ok(false),
        }
    }
}

/// The value of an integer literal node, if it is a byte value.
fn integer_literal_value(node: &NodeRef<'_>) -> Option<char> {
    if node.kind() != Kind::Literal {
        return None;
    }
    let text = node.text()?;
    if text.is_empty() {
        return None;
    }
    let mut value = 0_u32;
    for byte in text.bytes() {
        let digit = byte.checked_sub(b'0').filter(|digit| *digit < 10)?;
        value = value.checked_mul(10)?.checked_add(u32::from(digit))?;
        if value > 255 {
            return None;
        }
    }
    char::from_u32(value)
}

/// True for the kinds that can appear in a type position.
fn is_type(kind: Kind) -> bool {
    matches!(
        kind,
        Kind::BuiltinType
            | Kind::PointerType
            | Kind::ReferenceType
            | Kind::RvalueReferenceType
            | Kind::QualifiedType
            | Kind::ArrayType
            | Kind::FunctionType
            | Kind::MemberPointerType
            | Kind::VendorType
            | Kind::Decltype
            | Kind::TemplateParam
            | Kind::Pack
            | Kind::Name
            | Kind::NestedName
            | Kind::UnqualifiedName
            | Kind::Identifier
            | Kind::Substitution
            | Kind::AbiTag
            | Kind::Lambda
            | Kind::UnnamedType
    )
}

/// The abbreviated spelling of a well-known substitution.
///
/// The AST stores the standard stream types expanded; the demangled text uses
/// the `std::` typedef names.
fn short_substitution(text: &str) -> &str {
    match text {
        "std::basic_istream<char, std::char_traits<char> >" => "std::istream",
        "std::basic_iostream<char, std::char_traits<char> >" => "std::iostream",
        other => other,
    }
}

/// The expanded spelling of a well-known substitution used as a prefix.
///
/// `Ss` abbreviates `std::basic_string<char, std::char_traits<char>,
/// std::allocator<char>>`; a nested name such as `SsC1` names a constructor of
/// that class template, so the abbreviation is spelled out.
fn expanded_substitution(text: &str) -> &str {
    match text {
        "std::string" => "std::basic_string<char, std::char_traits<char>, std::allocator<char>>",
        "std::basic_istream<char, std::char_traits<char> >" => {
            "std::basic_istream<char, std::char_traits<char>>"
        }
        "std::ostream" => "std::basic_ostream<char, std::char_traits<char>>",
        "std::basic_iostream<char, std::char_traits<char> >" => {
            "std::basic_iostream<char, std::char_traits<char>>"
        }
        other => other,
    }
}

/// The leaf name of a well-known substitution, used by constructors.
fn substitution_leaf(text: &str) -> &'static str {
    match text {
        "std::string" | "std::basic_string" => "basic_string",
        "std::basic_istream<char, std::char_traits<char> >" => "basic_istream",
        "std::ostream" => "basic_ostream",
        "std::basic_iostream<char, std::char_traits<char> >" => "basic_iostream",
        "std::allocator" => "allocator",
        "std" => "std",
        _ => "",
    }
}

/// Decode a `<template-param>` reference into a zero-based index.
///
/// The text is the encoded form: `T_` or `T<n>_`. Multi-level references
/// (`TL<level>_..._`) are handled by [`template_param_level_index`] in lambda
/// signatures and rejected elsewhere.
fn template_param_index(text: &str) -> Option<usize> {
    let rest = text.strip_prefix('T')?;
    if rest.starts_with('L') {
        return None;
    }
    let digits = rest.strip_suffix('_')?;
    if digits.is_empty() {
        return Some(0);
    }
    parse_usize(digits)?.checked_add(1)
}

/// Decode a `<template-param>` reference into a level and a zero-based index.
///
/// The ABI counts levels and indices from one, so `TL0__` is level 1, index 0.
fn template_param_level_index(text: &str) -> Option<(usize, usize)> {
    let mut rest = text.strip_prefix('T')?;
    let mut level = 0;
    if let Some(after_level) = rest.strip_prefix('L') {
        let underscore = after_level.find('_')?;
        let digits = &after_level[..underscore];
        if digits.is_empty() {
            return None;
        }
        level = parse_usize(digits)?.checked_add(1)?;
        rest = &after_level[underscore + 1..];
    }
    let index = if rest == "_" {
        0
    } else {
        parse_usize(rest.strip_suffix('_')?)?.checked_add(1)?
    };
    Some((level, index))
}

/// Parse a decimal number.
fn parse_usize(text: &str) -> Option<usize> {
    let mut value = 0_usize;
    let mut any = false;
    for byte in text.bytes() {
        let digit = byte.checked_sub(b'0').filter(|digit| *digit < 10)?;
        value = value.checked_mul(10)?.checked_add(usize::from(digit))?;
        any = true;
    }
    any.then_some(value)
}

#[cfg(test)]
mod tests {
    use alloc::string::String;

    use super::super::parse;

    fn render(symbol: &str) -> String {
        let ast = parse(symbol).unwrap_or_else(|error| panic!("{symbol}: {error}"));
        let mut out = String::new();
        core::fmt::write(&mut out, format_args!("{ast}"))
            .unwrap_or_else(|_| panic!("{symbol} did not print"));
        out
    }

    #[test]
    fn local_name_discriminator_does_not_eat_a_source_name() {
        assert_eq!(render("_ZZ4testvEN1g3fooE5Point"), "test()::g::foo(Point)");
    }

    #[test]
    fn cv_qualified_function_type_takes_one_substitution_slot() {
        assert_eq!(
            render("_Z1fM1XKFivOEMS_FOKivEMS_VFS3_RS2_E"),
            "f(int (X::*)() const &&, int const&& (X::*)(), \
             int const&& (X::*)(int const&) volatile)"
        );
    }

    #[test]
    fn module_substitution_forms_one_entity() {
        assert_eq!(
            render("_ZNW3FOO3TPLIS_3OneE1MEPS1_"),
            "TPL@FOO<One@FOO>::M(One@FOO*)"
        );
        assert_eq!(
            render("_ZNW3FOO3TPLIS_3OneE1NIS_3TwoEEvPS1_PT_"),
            "void TPL@FOO<One@FOO>::N<Two@FOO>(One@FOO*, Two@FOO*)"
        );
    }

    #[test]
    fn constrained_parameter_prints_its_numbering() {
        assert_eq!(
            render("_ZN3FooIiE6methodITk4TrueIT_EiEEvS3_"),
            "void Foo<int>::method<int>(T)"
        );
    }

    #[test]
    fn conversion_operator_keeps_its_template_arguments() {
        assert_eq!(
            render("_ZN8Blizzard6Memory12voidp_returncvPT_IcEEv"),
            "Blizzard::Memory::voidp_return::operator char*<char>()"
        );
        assert_eq!(render("_Zcv1BIRT_EIS1_E"), "operator B<><>");
    }

    #[test]
    fn inherited_constructor_uses_the_inherited_template_arguments() {
        assert_eq!(render("_ZN1DCI21CIiEET_"), "D::D(int)");
    }

    #[test]
    fn argument_pack_and_expansion_are_distinguished_by_text() {
        assert_eq!(render("_Z1fIJR1SS0_EEiDpOT_"), "int f<S&, S>(S&, S&&)");
    }

    #[test]
    fn lambda_template_parameter_levels_resolve() {
        assert_eq!(
            render("_ZN1C1fIiEEvDTtlNS_UlT_TL0__E_EEE"),
            "void C::f<int>(decltype(C::'lambda'(int, auto){}))"
        );
        assert_eq!(
            render("_ZNK1xMUlTyTtTyTnT_TpTnPA3_TL0__ETpTyvE_clIi1XJfEEEDav"),
            "auto x::'lambda'<typename $T, template<typename $T0, $T $N, \
             $T0 (*...$N0) [3]> typename $TT, typename ...$T1>()::operator()<int, X, float>() const"
        );
        assert_eq!(
            render("_ZN1AIiE1fIfEEvDTLUlTyTtTyTnTL1__ETL0_1_T_TL0__E_EE"),
            "void A<int>::f<float>(decltype([]<typename $T, \
             template<typename $T0, $T0 $N> typename $TT>(auto, float, $T){...}))"
        );
    }
}
