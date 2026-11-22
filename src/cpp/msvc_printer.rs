//! Microsoft C++ printer: renders the shared [`Ast`] as demangled text.
//!
//! The output matches `llvm-undname`, which defines the demangled text for the
//! Microsoft scheme: the same type spelling, `const`/`volatile` placement,
//! declarator parentheses, `{for ...}` base-class lists and special-name
//! wrappers. `msvc-demangler` is only a test oracle; this module never calls
//! it.
//!
//! Types are printed in two halves. [`Printer::pre`] writes everything that
//! precedes the declarator name (`int *` for a pointer, `int (__cdecl *` for a
//! pointer to a function) and [`Printer::post`] writes everything that follows
//! it (parameter lists, array bounds, qualifiers on the pointee). Splitting the
//! rendering this way is what produces `int (*x)(int)` rather than
//! `int *x(int)`.
//!
//! Not every parsed node can be rendered from the AST alone. Productions whose
//! encoding the parser does not retain — the discarded member-pointer offsets
//! of `$F`/`$G`/`$H`/`$I`/`$J` template arguments, and the discarded signature
//! of a dynamic initializer for a static data member — report
//! [`PrintErrorKind::Unsupported`] instead of emitting text that does not match
//! the symbol.

use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;
use core::fmt;

use super::node::{Kind, NodeRef};
use super::Ast;

/// Maximum nesting depth the printer will render.
const MAX_PRINT_DEPTH: u32 = 512;

/// Maximum node visits for one print run. Shared nodes are visited on every
/// reference, so this bounds work for a DAG whose expansion is exponential.
const MAX_PRINT_VISITS: u32 = 1 << 17;

/// Maximum output length in bytes for one print run.
const MAX_OUTPUT_BYTES: usize = 1 << 20;

/// Why a Microsoft C++ AST could not be printed.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[non_exhaustive]
pub enum PrintErrorKind {
    /// The production has no Microsoft C++ text rendering, or the AST does not
    /// retain the data needed to render it.
    Unsupported,
    /// A node's children or text violate the documented node contract.
    InvalidNode,
    /// The nesting budget was exceeded.
    TooDeep,
    /// The visit budget was exceeded.
    TooManyVisits,
    /// The output budget was exceeded.
    TooLarge,
    /// The output sink rejected a write.
    Fmt,
}

impl fmt::Display for PrintErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::Unsupported => "production cannot be printed",
            Self::InvalidNode => "malformed node",
            Self::TooDeep => "print depth budget exceeded",
            Self::TooManyVisits => "print visit budget exceeded",
            Self::TooLarge => "output budget exceeded",
            Self::Fmt => "output sink failed",
        };
        f.write_str(message)
    }
}

/// A print failure with the input offset of the offending node.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct PrintError {
    position: usize,
    kind: PrintErrorKind,
}

impl PrintError {
    /// Byte offset in the mangled input where the offending node starts.
    ///
    /// Zero when the failure has no input position, as for a failed sink write.
    #[must_use]
    pub const fn position(&self) -> usize {
        self.position
    }

    /// What went wrong.
    #[must_use]
    pub const fn kind(&self) -> PrintErrorKind {
        self.kind
    }
}

impl fmt::Display for PrintError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} at byte {}", self.kind, self.position)
    }
}

impl core::error::Error for PrintError {}

/// Render a parsed Microsoft C++ symbol into `out`.
///
/// # Errors
///
/// Returns [`PrintError`] when the tree contains a production the printer
/// cannot render from the AST, when the node contract is violated, when a
/// print budget is exceeded, or when `out` rejects a write. No partial text is
/// meaningful after an error.
pub fn print_msvc<W: fmt::Write + ?Sized>(ast: &Ast<'_>, out: &mut W) -> Result<(), PrintError> {
    let mut printer = Printer {
        out,
        depth: 0,
        visits: 0,
        written: 0,
        last: None,
    };
    printer.symbol(&ast.root())
}

fn error_at(node: &NodeRef<'_>, kind: PrintErrorKind) -> PrintError {
    PrintError {
        position: node.span().map_or(0, |range| range.start as usize),
        kind,
    }
}

fn child<'a>(node: &NodeRef<'a>, index: usize) -> Result<NodeRef<'a>, PrintError> {
    node.child(index)
        .ok_or_else(|| error_at(node, PrintErrorKind::InvalidNode))
}

fn text<'s>(node: &'s NodeRef<'_>) -> Result<&'s str, PrintError> {
    node.text()
        .ok_or_else(|| error_at(node, PrintErrorKind::InvalidNode))
}

/// A cv/extension qualifier set, decoded from a [`Kind::QualifiedType`] text.
#[derive(Clone, Default)]
struct QualSet {
    const_: bool,
    volatile: bool,
    restrict: bool,
    unaligned: bool,
    /// `__ptrauth(<key>, <address-discrimination>, <extra>)`, always last.
    ptrauth: Option<String>,
}

impl QualSet {
    /// Decode the space-separated qualifier text the parser writes.
    fn parse(node: &NodeRef<'_>) -> Result<Self, PrintError> {
        let text = text(node)?;
        let mut set = Self::default();
        let rest = if let Some(index) = text.find("__ptrauth(") {
            set.ptrauth = Some(String::from(&text[index..]));
            text.get(..index).unwrap_or("").trim_end()
        } else {
            text
        };
        for token in rest.split(' ') {
            match token {
                "" => {}
                "const" => set.const_ = true,
                "volatile" => set.volatile = true,
                "__restrict" => set.restrict = true,
                "__unaligned" => set.unaligned = true,
                _ => return Err(error_at(node, PrintErrorKind::Unsupported)),
            }
        }
        Ok(set)
    }

    /// Union two qualifier sets, keeping the first pointer-authentication
    /// annotation.
    fn merge(&mut self, other: &Self) {
        self.const_ |= other.const_;
        self.volatile |= other.volatile;
        self.restrict |= other.restrict;
        self.unaligned |= other.unaligned;
        if self.ptrauth.is_none() {
            self.ptrauth.clone_from(&other.ptrauth);
        }
    }

}

/// Function modifiers decoded from a function node's text.
///
/// The parser stores them as one space-separated string in source order:
/// `extern "C"`, access, `static`/`virtual`, the calling convention, then the
/// cv/ref qualifiers. Only the calling convention and everything before it is
/// written before the return type.
struct Modifiers<'a> {
    /// Tokens written before the return type.
    leading: Vec<&'a str>,
    /// The calling convention, written after the return type.
    calling_conv: Option<&'a str>,
    /// Qualifier tokens written after the parameter list.
    trailing: Vec<&'a str>,
}

fn is_calling_conv(token: &str) -> bool {
    matches!(
        token,
        "__cdecl"
            | "__pascal"
            | "__thiscall"
            | "__stdcall"
            | "__fastcall"
            | "__clrcall"
            | "__eabi"
            | "__vectorcall"
            | "__attribute__((__swiftcall__))"
            | "__attribute__((__swiftasynccall__))"
    )
}

fn split_modifiers(text: Option<&str>) -> Modifiers<'_> {
    let Some(text) = text else {
        return Modifiers {
            leading: Vec::new(),
            calling_conv: None,
            trailing: Vec::new(),
        };
    };
    let tokens: Vec<&str> = text.split(' ').filter(|token| !token.is_empty()).collect();
    let conv_index = tokens.iter().position(|token| is_calling_conv(token));
    let (leading, rest) = tokens.split_at(conv_index.unwrap_or(tokens.len()));
    let (calling_conv, trailing) = match rest.split_first() {
        Some((first, tail)) => (Some(*first), tail),
        None => (None, &[][..]),
    };
    Modifiers {
        leading: leading.to_vec(),
        calling_conv,
        trailing: trailing.to_vec(),
    }
}

/// Optional substitutions applied while rendering a function encoding.
#[derive(Clone, Copy, Default)]
struct FunctionText<'a> {
    /// Text written before the mangled name, inside the name position.
    name_prefix: Option<&'a str>,
    /// Text written after the mangled name.
    name_suffix: Option<&'a str>,
    /// Thunk adjustment written after the name, without backtick and quote.
    adjustment: Option<&'a str>,
}

/// Which special table a [`Kind::Vftable`] or [`Kind::Vbtable`] node is.
#[derive(Clone, Copy, PartialEq, Eq)]
enum TableKind {
    Vftable,
    Vbtable,
}

struct Printer<'w, W: fmt::Write + ?Sized> {
    out: &'w mut W,
    depth: u32,
    visits: u32,
    written: usize,
    last: Option<char>,
}

impl<W: fmt::Write + ?Sized> Printer<'_, W> {
    // -- output primitives -------------------------------------------------

    fn put(&mut self, text: &str) -> Result<(), PrintError> {
        if text.is_empty() {
            return Ok(());
        }
        self.written = self.written.saturating_add(text.len());
        if self.written > MAX_OUTPUT_BYTES {
            return Err(PrintError {
                position: 0,
                kind: PrintErrorKind::TooLarge,
            });
        }
        self.last = text.chars().next_back();
        self.out.write_str(text).map_err(|_| PrintError {
            position: 0,
            kind: PrintErrorKind::Fmt,
        })
    }

    /// Write a space when the previous output would otherwise fuse with the
    /// next token. Mirrors `llvm-undname`'s pointer spacing.
    fn space_ptr(&mut self) -> Result<(), PrintError> {
        if let Some(last) = self.last
            && (last.is_ascii_alphanumeric() || matches!(last, '>' | ')'))
        {
            self.put(" ")?;
        }
        Ok(())
    }

    /// Write a space before an operator name. Also accepts `*` and `&`.
    fn space(&mut self) -> Result<(), PrintError> {
        if let Some(last) = self.last
            && (last.is_ascii_alphanumeric() || matches!(last, '*' | '&' | '>' | ')'))
        {
            self.put(" ")?;
        }
        Ok(())
    }

    /// Write a calling convention. The separator before whatever follows is
    /// the caller's job, so the token itself has no trailing space.
    fn calling_conv(&mut self, conv: &str) -> Result<(), PrintError> {
        if !matches!(self.last, Some(' ' | '(')) {
            self.put(" ")?;
        }
        self.put(conv)
    }

    fn enter(&mut self, node: &NodeRef<'_>) -> Result<(), PrintError> {
        self.visits = self.visits.saturating_add(1);
        if self.visits > MAX_PRINT_VISITS {
            return Err(error_at(node, PrintErrorKind::TooManyVisits));
        }
        if self.depth >= MAX_PRINT_DEPTH {
            return Err(error_at(node, PrintErrorKind::TooDeep));
        }
        self.depth += 1;
        Ok(())
    }

    // -- symbols -----------------------------------------------------------

    fn symbol(&mut self, root: &NodeRef<'_>) -> Result<(), PrintError> {
        if root.kind() != Kind::Encoding {
            return Err(error_at(root, PrintErrorKind::InvalidNode));
        }
        let shape = child(root, 0)?;
        self.shape(&shape)
    }

    fn shape(&mut self, shape: &NodeRef<'_>) -> Result<(), PrintError> {
        match shape.kind() {
            Kind::Function | Kind::MemberFunction => {
                self.function(shape, FunctionText::default())
            }
            Kind::Variable => self.variable(shape),
            Kind::Vftable => self.table(shape, TableKind::Vftable),
            Kind::Vbtable => self.table(shape, TableKind::Vbtable),
            Kind::Rtti => self.rtti(shape),
            Kind::DynamicInitializer => self.dynamic(shape, "`dynamic initializer for '"),
            Kind::DynamicAtexitDestructor => {
                self.dynamic(shape, "`dynamic atexit destructor for '")
            }
            Kind::StringLiteral => self.put("`string'"),
            Kind::Md5Name => self.md5_name(shape),
            Kind::VCallThunk => self.vcall_thunk(shape),
            Kind::GuardVariable => self.guard_variable(shape),
            Kind::Thunk => self.thunk(shape),
            _ => Err(error_at(shape, PrintErrorKind::Unsupported)),
        }
    }

    fn md5_name(&mut self, node: &NodeRef<'_>) -> Result<(), PrintError> {
        let digest = text(node)?;
        self.put("??@")?;
        self.put(digest)?;
        self.put("@")
    }

    fn variable(&mut self, node: &NodeRef<'_>) -> Result<(), PrintError> {
        self.enter(node)?;
        let storage = match node.text() {
            Some("private-static") => "private: static ",
            Some("protected-static") => "protected: static ",
            Some("public-static") => "public: static ",
            Some("global" | "function-local-static") | None => "",
            Some(_) => return Err(error_at(node, PrintErrorKind::Unsupported)),
        };
        let name = child(node, 0)?;
        let ty = child(node, 1)?;
        self.put(storage)?;
        self.pre(&ty)?;
        self.write_name(&name)?;
        let result = self.post(&ty);
        self.depth -= 1;
        result
    }

    fn function(&mut self, node: &NodeRef<'_>, opts: FunctionText<'_>) -> Result<(), PrintError> {
        self.enter(node)?;
        let result = self.function_inner(node, opts);
        self.depth -= 1;
        result
    }

    fn function_inner(
        &mut self,
        node: &NodeRef<'_>,
        opts: FunctionText<'_>,
    ) -> Result<(), PrintError> {
        let modifiers = split_modifiers(node.text());
        let ret = function_return(node)?;
        let params = function_params(node)?;
        if node.text().is_none() && ret.is_none() && params.child_count() == 0 {
            // A `?<name>@@9` name encodes no function type: function class `9`
            // is `extern "C"`, global and without a parameter list.
            let name = child(node, 0)?;
            self.put("extern \"C\" ")?;
            return self.write_name(&name);
        }
        for token in &modifiers.leading {
            self.put(token)?;
            self.put(" ")?;
        }
        if let Some(ret) = &ret {
            self.pre(ret)?;
        }
        if let Some(conv) = modifiers.calling_conv {
            self.calling_conv(conv)?;
        }
        if let Some(prefix) = opts.name_prefix {
            self.space_ptr()?;
            self.put(prefix)?;
            let name = child(node, 0)?;
            self.qualified_name(&name, None)?;
            self.put(opts.name_suffix.unwrap_or(""))?;
        } else {
            let name = child(node, 0)?;
            self.write_name(&name)?;
        }
        if let Some(adjustment) = opts.adjustment {
            self.put("`")?;
            self.put(adjustment)?;
            self.put("'")?;
        }
        if opts.name_prefix.is_none()
            && let Some(ret) = &ret
            && let Some(name) = node.child(0)
            && is_conversion_operator(&name)
        {
            // A conversion operator whose name is a template-id keeps its
            // target type in the encoded return type.
            self.put(" ")?;
            self.pre(ret)?;
            self.post(ret)?;
        }
        self.put("(")?;
        self.parameters(&params)?;
        self.put(")")?;
        for token in &modifiers.trailing {
            self.put(" ")?;
            self.put(token)?;
        }
        if let Some(ret) = &ret {
            self.post(ret)?;
        }
        Ok(())
    }

    fn parameters(&mut self, list: &NodeRef<'_>) -> Result<(), PrintError> {
        if list.child_count() == 0 {
            // `X` encodes the single parameter `void`.
            return self.put("void");
        }
        for (index, parameter) in list.children().iter().enumerate() {
            if index > 0 {
                self.put(", ")?;
            }
            let ty = child(parameter, 0)?;
            self.pre(&ty)?;
            self.post(&ty)?;
        }
        Ok(())
    }

    fn thunk(&mut self, node: &NodeRef<'_>) -> Result<(), PrintError> {
        let adjustment = text(node)?;
        let inner = child(node, 0)?;
        if !matches!(inner.kind(), Kind::Function | Kind::MemberFunction) {
            return Err(error_at(&inner, PrintErrorKind::Unsupported));
        }
        self.put("[thunk]: ")?;
        self.function(
            &inner,
            FunctionText {
                adjustment: Some(adjustment),
                ..FunctionText::default()
            },
        )
    }

    fn vcall_thunk(&mut self, node: &NodeRef<'_>) -> Result<(), PrintError> {
        let name = child(node, 0)?;
        let rendered = text(node)?;
        let body = rendered
            .strip_prefix("vcall{")
            .and_then(|rest| rest.strip_suffix('}'))
            .ok_or_else(|| error_at(node, PrintErrorKind::InvalidNode))?;
        let (offset, conv) = body
            .split_once(", ")
            .ok_or_else(|| error_at(node, PrintErrorKind::InvalidNode))?;
        self.put("[thunk]: ")?;
        self.calling_conv(conv)?;
        self.write_name(&name)?;
        self.put("{")?;
        self.put(offset)?;
        self.put(", {flat}}")
    }

    fn guard_variable(&mut self, node: &NodeRef<'_>) -> Result<(), PrintError> {
        let name = child(node, 0)?;
        let index = text(node)?;
        if is_local_guard(&name) {
            self.write_name(&name)?;
            self.put("{")?;
            self.put(index)?;
            self.put("}")
        } else {
            // The `?$TSS<n>@` thread-safe guard form carries no guard name.
            self.put("TSS")?;
            self.put(index)?;
            self.write_name(&name)
        }
    }

    fn dynamic(&mut self, node: &NodeRef<'_>, prefix: &str) -> Result<(), PrintError> {
        let entity = child(node, 0)?;
        if entity.kind() != Kind::Function {
            // The parser drops the encoded signature for a static data member.
            return Err(error_at(&entity, PrintErrorKind::Unsupported));
        }
        self.function(
            &entity,
            FunctionText {
                name_prefix: Some(prefix),
                name_suffix: Some("''"),
                ..FunctionText::default()
            },
        )
    }

    fn table(&mut self, node: &NodeRef<'_>, table: TableKind) -> Result<(), PrintError> {
        self.enter(node)?;
        let result = self.table_inner(node, table);
        self.depth -= 1;
        result
    }

    fn table_inner(&mut self, node: &NodeRef<'_>, table: TableKind) -> Result<(), PrintError> {
        let name = child(node, 0)?;
        let mut local = false;
        let mut const_ = false;
        for token in node.text().unwrap_or("").split(' ') {
            match token {
                "" => {}
                "local" => local = true,
                "const" => const_ = true,
                _ => return Err(error_at(node, PrintErrorKind::Unsupported)),
            }
        }
        if const_ {
            self.put("const")?;
        }
        self.space_ptr()?;
        let operator = match (table, local) {
            (TableKind::Vftable, false) => "`vftable'",
            (TableKind::Vftable, true) => "`local vftable'",
            (TableKind::Vbtable, false) => "`vbtable'",
            (TableKind::Vbtable, true) => "`local vbtable'",
        };
        self.qualified_operator(&name, operator)?;
        if table == TableKind::Vftable {
            let mut targets = node.children().iter().skip(1);
            if let Some(first) = targets.next() {
                self.put("{for `")?;
                self.qualified_name(first, None)?;
                self.put("'")?;
                for target in targets {
                    self.put("s `")?;
                    self.qualified_name(target, None)?;
                    self.put("'")?;
                }
                self.put("}")?;
            }
        }
        Ok(())
    }

    fn rtti(&mut self, node: &NodeRef<'_>) -> Result<(), PrintError> {
        self.enter(node)?;
        let result = self.rtti_inner(node);
        self.depth -= 1;
        result
    }

    fn rtti_inner(&mut self, node: &NodeRef<'_>) -> Result<(), PrintError> {
        let rendered = text(node)?;
        let target = child(node, 0)?;
        if rendered == "type-descriptor" {
            self.pre(&target)?;
            self.post(&target)?;
            return self.put(" `RTTI Type Descriptor'");
        }
        if rendered == "type-descriptor-name" {
            self.pre(&target)?;
            self.post(&target)?;
            return self.put(" `RTTI Type Descriptor Name'");
        }
        if let Some(fields) = rendered.strip_prefix("base-class-descriptor ") {
            let operator = alloc::format!("`RTTI Base Class Descriptor at {fields}'");
            return self.qualified_operator(&target, &operator);
        }
        if let Some(qualifiers) = rendered.strip_prefix("complete-object-locator") {
            let qualifiers = qualifiers.trim();
            let qualifiers = qualifiers
                .strip_prefix('(')
                .and_then(|rest| rest.strip_suffix(')'))
                .unwrap_or(qualifiers);
            let mut set = QualSet::default();
            for token in qualifiers.split(' ') {
                match token {
                    "" => {}
                    "const" => set.const_ = true,
                    _ => return Err(error_at(node, PrintErrorKind::Unsupported)),
                }
            }
            self.write_quals(&set)?;
            self.space_ptr()?;
            return self.qualified_operator(&target, "`RTTI Complete Object Locator'");
        }
        let operator = match rendered {
            "base-class-array" => "`RTTI Base Class Array'",
            "class-hierarchy-descriptor" => "`RTTI Class Hierarchy Descriptor'",
            _ => return Err(error_at(node, PrintErrorKind::Unsupported)),
        };
        self.qualified_operator(&target, operator)
    }

    // -- names -------------------------------------------------------------

    fn write_name(&mut self, node: &NodeRef<'_>) -> Result<(), PrintError> {
        self.space_ptr()?;
        self.qualified_name(node, None)
    }

    fn qualified_name(
        &mut self,
        node: &NodeRef<'_>,
        prev: Option<&NodeRef<'_>>,
    ) -> Result<(), PrintError> {
        self.enter(node)?;
        let result = self.qualified_name_inner(node, prev);
        self.depth -= 1;
        result
    }

    fn qualified_name_inner(
        &mut self,
        node: &NodeRef<'_>,
        prev: Option<&NodeRef<'_>>,
    ) -> Result<(), PrintError> {
        // A qualified name is a left-nested chain. Flatten it so a long scope
        // chain neither recurses nor spends the depth budget.
        let mut components: Vec<NodeRef<'_>> = Vec::new();
        let mut current = Rc::clone(node);
        while current.kind() == Kind::NestedName {
            components.push(child(&current, 1)?);
            current = child(&current, 0)?;
        }
        if components.is_empty() {
            return self.name_component(node, prev);
        }
        components.push(current);
        components.reverse();
        for (index, component) in components.iter().enumerate() {
            if index > 0 {
                self.put("::")?;
            }
            let previous = index.checked_sub(1).map(|previous| &components[previous]);
            self.name_component(component, previous)?;
        }
        Ok(())
    }

    /// Write one component of a qualified name.
    fn name_component(
        &mut self,
        node: &NodeRef<'_>,
        prev: Option<&NodeRef<'_>>,
    ) -> Result<(), PrintError> {
        match node.kind() {
            Kind::UnqualifiedName => {
                let leaf = child(node, 0)?;
                self.leaf_name(&leaf, prev)
            }
            Kind::Name => {
                let base = child(node, 0)?;
                let args = child(node, 1)?;
                self.name_component(&base, prev)?;
                self.template_args(&args)
            }
            Kind::LocalName => self.local_name(node),
            Kind::Identifier => self.put(text(node)?),
            Kind::UnnamedType => {
                self.put("<")?;
                self.put(text(node)?)?;
                self.put(">")
            }
            Kind::OperatorName => self.operator_name(node),
            Kind::CtorDtorName => self.ctor_dtor_name(node, prev),
            _ => Err(error_at(node, PrintErrorKind::Unsupported)),
        }
    }

    fn leaf_name(
        &mut self,
        leaf: &NodeRef<'_>,
        prev: Option<&NodeRef<'_>>,
    ) -> Result<(), PrintError> {
        match leaf.kind() {
            Kind::Identifier => self.put(text(leaf)?),
            Kind::UnnamedType => {
                self.put("<")?;
                self.put(text(leaf)?)?;
                self.put(">")
            }
            Kind::OperatorName => self.operator_name(leaf),
            Kind::CtorDtorName => self.ctor_dtor_name(leaf, prev),
            _ => Err(error_at(leaf, PrintErrorKind::Unsupported)),
        }
    }

    fn operator_name(&mut self, node: &NodeRef<'_>) -> Result<(), PrintError> {
        let rendered = text(node)?;
        if rendered == "operator" && node.child_count() == 1 {
            let target = child(node, 0)?;
            self.put("operator ")?;
            self.pre(&target)?;
            return self.post(&target);
        }
        self.space()?;
        self.put(rendered)
    }

    fn ctor_dtor_name(
        &mut self,
        node: &NodeRef<'_>,
        prev: Option<&NodeRef<'_>>,
    ) -> Result<(), PrintError> {
        let rendered = text(node)?;
        let prev = prev.ok_or_else(|| error_at(node, PrintErrorKind::InvalidNode))?;
        match rendered.as_bytes().first() {
            Some(b'C') => {}
            Some(b'D') => self.put("~")?,
            _ => return Err(error_at(node, PrintErrorKind::Unsupported)),
        }
        self.qualified_name(prev, None)
    }

    fn local_name(&mut self, node: &NodeRef<'_>) -> Result<(), PrintError> {
        // `?<discriminator>?<symbol>`: the scope is the enclosing symbol, then
        // the discriminator as its own component.
        if node.child_count() != 1 {
            return Err(error_at(node, PrintErrorKind::Unsupported));
        }
        let encoding = child(node, 0)?;
        let index = text(node)?;
        self.put("`")?;
        self.symbol(&encoding)?;
        self.put("'::`")?;
        self.put(index)?;
        self.put("'")
    }

    fn template_args(&mut self, args: &NodeRef<'_>) -> Result<(), PrintError> {
        if args.kind() != Kind::TemplateArgs {
            return Err(error_at(args, PrintErrorKind::InvalidNode));
        }
        self.put("<")?;
        for (index, arg) in args.children().iter().enumerate() {
            if index > 0 {
                self.put(", ")?;
            }
            self.template_arg(arg)?;
        }
        self.put(">")
    }

    fn template_arg(&mut self, arg: &NodeRef<'_>) -> Result<(), PrintError> {
        match arg.kind() {
            Kind::Literal => self.put(text(arg)?),
            Kind::TemplateParam => self.template_param(arg),
            _ => {
                self.pre(arg)?;
                self.post(arg)
            }
        }
    }

    fn template_param(&mut self, node: &NodeRef<'_>) -> Result<(), PrintError> {
        let rendered = text(node)?;
        if rendered == "&symbol" {
            // `$E <symbol>`: the referenced symbol, without a leading `&`.
            let symbol = child(node, 0)?;
            return self.shape(&symbol);
        }
        if rendered == "$1" {
            // `$1 <symbol>`: the symbol's address as a non-type argument.
            let symbol = child(node, 0)?;
            self.put("&")?;
            return self.shape(&symbol);
        }
        // `$F`, `$G`, `$H`, `$I` and `$J` carry decoded offsets the parser
        // drops, so their member-pointer text cannot be reconstructed.
        Err(error_at(node, PrintErrorKind::Unsupported))
    }

    // -- types -------------------------------------------------------------

    fn pre(&mut self, node: &NodeRef<'_>) -> Result<(), PrintError> {
        self.enter(node)?;
        let result = self.pre_inner(node);
        self.depth -= 1;
        result
    }

    fn pre_inner(&mut self, node: &NodeRef<'_>) -> Result<(), PrintError> {
        match node.kind() {
            Kind::BuiltinType => self.put(text(node)?),
            Kind::QualifiedType => self.qualified_pre(node),
            Kind::PointerType => self.pointer_pre(node, "*"),
            Kind::ReferenceType => self.pointer_pre(node, "&"),
            Kind::RvalueReferenceType => self.pointer_pre(node, "&&"),
            Kind::ArrayType => self.pre(&child(node, 0)?),
            Kind::FunctionType => self.function_type_pre(node),
            Kind::MemberPointerType => self.member_pointer_pre(node),
            Kind::TaggedType => {
                self.put(text(node)?)?;
                self.put(" ")?;
                let name = child(node, 0)?;
                self.qualified_name(&name, None)
            }
            Kind::UnqualifiedName
            | Kind::NestedName
            | Kind::Name
            | Kind::Identifier
            | Kind::UnnamedType
            | Kind::LocalName => self.write_name(node),
            Kind::Literal => self.put(text(node)?),
            Kind::TemplateParam => self.template_param(node),
            _ => Err(error_at(node, PrintErrorKind::Unsupported)),
        }
    }

    fn post(&mut self, node: &NodeRef<'_>) -> Result<(), PrintError> {
        self.enter(node)?;
        let result = self.post_inner(node);
        self.depth -= 1;
        result
    }

    fn post_inner(&mut self, node: &NodeRef<'_>) -> Result<(), PrintError> {
        match node.kind() {
            Kind::QualifiedType => self.post(&child(node, 0)?),
            Kind::PointerType | Kind::ReferenceType | Kind::RvalueReferenceType => {
                let inner = child(node, 0)?;
                if matches!(
                    declarator_kind(&inner),
                    Kind::FunctionType | Kind::ArrayType
                ) {
                    self.put(")")?;
                }
                self.post(&inner)
            }
            Kind::ArrayType => {
                self.put("[")?;
                let bound = text(node)?;
                if bound != "0" {
                    self.put(bound)?;
                }
                self.put("]")?;
                self.post(&child(node, 0)?)
            }
            Kind::FunctionType => self.function_type_post(node),
            Kind::MemberPointerType => self.member_pointer_post(node),
            _ => Ok(()),
        }
    }

    fn qualified_pre(&mut self, node: &NodeRef<'_>) -> Result<(), PrintError> {
        // Adjacent qualifier wrappers describe the same declarator, so their
        // qualifiers are idempotent: merge them and write each one once.
        let mut quals = QualSet::default();
        let mut inner = Rc::clone(node);
        while inner.kind() == Kind::QualifiedType {
            quals.merge(&QualSet::parse(&inner)?);
            inner = child(&inner, 0)?;
        }
        self.pre(&inner)?;
        self.write_quals(&quals)
    }

    fn write_quals(&mut self, quals: &QualSet) -> Result<(), PrintError> {
        for (present, token) in [
            (quals.const_, "const"),
            (quals.volatile, "volatile"),
            (quals.restrict, "__restrict"),
            (quals.unaligned, "__unaligned"),
        ] {
            if present {
                self.space_ptr()?;
                self.put(token)?;
            }
        }
        if let Some(auth) = &quals.ptrauth {
            self.space_ptr()?;
            self.put(auth)?;
        }
        Ok(())
    }

    fn pointer_pre(&mut self, node: &NodeRef<'_>, operator: &str) -> Result<(), PrintError> {
        let inner = child(node, 0)?;
        match declarator_kind(&inner) {
            Kind::FunctionType => {
                let function = innermost_declarator(&inner);
                if function.kind() != Kind::FunctionType {
                    // A cv-qualified function type under a pointer has no
                    // printable declarator form.
                    return Err(error_at(&inner, PrintErrorKind::Unsupported));
                }
                if let Some(ret) = function_return(&function)? {
                    self.pre(&ret)?;
                }
                self.space()?;
                self.put("(")?;
                if let Some(conv) = calling_convention(function.text()) {
                    self.calling_conv(conv)?;
                }
            }
            Kind::ArrayType => {
                self.pre(&inner)?;
                self.space_ptr()?;
                self.put("(")?;
            }
            _ => self.pre(&inner)?,
        }
        self.space_ptr()?;
        self.put(operator)
    }

    fn function_type_pre(&mut self, node: &NodeRef<'_>) -> Result<(), PrintError> {
        if let Some(ret) = function_return(node)? {
            self.pre(&ret)?;
        }
        if let Some(conv) = calling_convention(node.text()) {
            self.calling_conv(conv)?;
        }
        Ok(())
    }

    fn function_type_post(&mut self, node: &NodeRef<'_>) -> Result<(), PrintError> {
        let params = function_params(node)?;
        self.put("(")?;
        self.parameters(&params)?;
        self.put(")")?;
        let modifiers = split_modifiers(node.text());
        for token in &modifiers.trailing {
            self.put(" ")?;
            self.put(token)?;
        }
        if let Some(ret) = function_return(node)? {
            self.post(&ret)?;
        }
        Ok(())
    }

    fn member_pointer_pre(&mut self, node: &NodeRef<'_>) -> Result<(), PrintError> {
        let class = child(node, 0)?;
        let member = child(node, 1)?;
        if member.kind() == Kind::FunctionType {
            if let Some(ret) = function_return(&member)? {
                self.pre(&ret)?;
            }
            self.space()?;
            self.put("(")?;
            if let Some(conv) = calling_convention(member.text()) {
                self.calling_conv(conv)?;
            }
            self.space()?;
        } else {
            self.pre(&member)?;
            self.space_ptr()?;
        }
        self.space_ptr()?;
        self.qualified_name(&class, None)?;
        self.put("::*")
    }

    fn member_pointer_post(&mut self, node: &NodeRef<'_>) -> Result<(), PrintError> {
        let member = child(node, 1)?;
        if member.kind() != Kind::FunctionType {
            return self.post(&member);
        }
        let params = function_params(&member)?;
        self.put(")(")?;
        self.parameters(&params)?;
        self.put(")")?;
        if let Some(ret) = function_return(&member)? {
            self.post(&ret)?;
        }
        let modifiers = split_modifiers(member.text());
        for token in &modifiers.trailing {
            self.put(" ")?;
            self.put(token)?;
        }
        Ok(())
    }

    /// Write a qualified name whose innermost component is `operator`.
    fn qualified_operator(
        &mut self,
        name: &NodeRef<'_>,
        operator: &str,
    ) -> Result<(), PrintError> {
        match name.kind() {
            Kind::NestedName => {
                let prefix = child(name, 0)?;
                self.qualified_name(&prefix, None)?;
                self.put("::")?;
                self.put(operator)
            }
            _ => self.put(operator),
        }
    }
}

/// The kind that decides whether a pointer needs declarator parentheses.
///
/// Element qualifiers wrap the array or function type, so the wrapper is
/// transparent here.
fn declarator_kind(node: &NodeRef<'_>) -> Kind {
    let mut current = Rc::clone(node);
    while current.kind() == Kind::QualifiedType {
        let Some(inner) = current.child(0) else {
            return current.kind();
        };
        current = inner;
    }
    current.kind()
}

/// The node under any qualifier wrappers.
fn innermost_declarator<'a>(node: &NodeRef<'a>) -> NodeRef<'a> {
    let mut current = Rc::clone(node);
    while current.kind() == Kind::QualifiedType {
        let Some(inner) = current.child(0) else {
            return current;
        };
        current = inner;
    }
    current
}

/// The encoded return type of a function encoding or function type.
///
/// Function encodings are `[name, return_type?, ParameterList]`; function types
/// are `[return_type?, ParameterList]`.
fn function_return<'a>(node: &NodeRef<'a>) -> Result<Option<NodeRef<'a>>, PrintError> {
    Ok(match (node.kind(), node.child_count()) {
        (Kind::FunctionType, 1) | (Kind::Function, 2) | (Kind::MemberFunction, 2) => None,
        (Kind::FunctionType, 2) => Some(child(node, 0)?),
        (Kind::Function, 3) | (Kind::MemberFunction, 3) => Some(child(node, 1)?),
        _ => return Err(error_at(node, PrintErrorKind::InvalidNode)),
    })
}

/// The parameter list of a function encoding or function type.
fn function_params<'a>(node: &NodeRef<'a>) -> Result<NodeRef<'a>, PrintError> {
    let count = node.child_count();
    if count == 0 {
        return Err(error_at(node, PrintErrorKind::InvalidNode));
    }
    let params = child(node, count - 1)?;
    if params.kind() != Kind::ParameterList {
        return Err(error_at(node, PrintErrorKind::InvalidNode));
    }
    Ok(params)
}

/// The calling convention token of a function-type modifier text.
fn calling_convention(text: Option<&str>) -> Option<&str> {
    let first = text?.split(' ').next()?;
    if is_calling_conv(first) { Some(first) } else { None }
}

/// True when a name is a conversion operator with no substitution target.
///
/// The parser replaces the operator's name with the conversion target for
/// plain conversion operators, but a template-id conversion operator keeps an
/// empty `operator` leaf and encodes the target as the return type.
fn is_conversion_operator(node: &NodeRef<'_>) -> bool {
    match node.kind() {
        Kind::NestedName => node
            .child(1)
            .is_some_and(|base| is_conversion_operator(&base)),
        Kind::Name => node
            .child(0)
            .is_some_and(|base| is_conversion_operator(&base)),
        Kind::UnqualifiedName => node.child(0).is_some_and(|leaf| {
            leaf.kind() == Kind::OperatorName
                && leaf.text() == Some("operator")
                && leaf.child_count() == 0
        }),
        _ => false,
    }
}

/// True when a guard variable name carries a `local static guard` component.
fn is_local_guard(node: &NodeRef<'_>) -> bool {
    match node.kind() {
        Kind::NestedName => node.child(1).is_some_and(|child| is_local_guard(&child)),
        Kind::UnqualifiedName => node
            .child(0)
            .and_then(|leaf| leaf.text().map(|text| text.starts_with("`local static")))
            .unwrap_or(false),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cpp;

    fn render(symbol: &str) -> Result<String, PrintError> {
        let ast = cpp::parse_msvc(symbol).expect("symbol should parse");
        let mut out = String::new();
        print_msvc(&ast, &mut out)?;
        Ok(out)
    }

    fn rendered(symbol: &str) -> String {
        render(symbol).unwrap_or_else(|error| panic!("{symbol}: {error}"))
    }

    #[test]
    fn prints_global_functions() {
        assert_eq!(rendered("?foo@@YAXXZ"), "void __cdecl foo(void)");
        assert_eq!(
            rendered("?x@@YAXMH@Z"),
            "void __cdecl x(float, int)"
        );
        assert_eq!(rendered("?x@@YAXZZ"), "void __cdecl x(...)");
        assert_eq!(
            rendered("?foo_pbd@@YAXPEBD@Z"),
            "void __cdecl foo_pbd(char const *)"
        );
        assert_eq!(
            rendered("?foo_qad@@YAXQEAD@Z"),
            "void __cdecl foo_qad(char *const)"
        );
    }

    #[test]
    fn prints_member_functions() {
        assert_eq!(
            rendered("?method@Struct@@QEAAXXZ"),
            "public: void __cdecl Struct::method(void)"
        );
        assert_eq!(
            rendered("??0klass@@QEAA@XZ"),
            "public: __cdecl klass::klass(void)"
        );
        assert_eq!(
            rendered("??1klass@@QEAA@XZ"),
            "public: __cdecl klass::~klass(void)"
        );
        assert_eq!(
            rendered("??4klass@@QEAAAEBV0@AEBV0@@Z"),
            "public: class klass const & __cdecl klass::operator=(class klass const &)"
        );
        assert_eq!(
            rendered("?fn@?$klass@H@ns@@QEBAIXZ"),
            "public: unsigned int __cdecl ns::klass<int>::fn(void) const"
        );
    }

    #[test]
    fn prints_variables_and_types() {
        assert_eq!(rendered("?x@@3HA"), "int x");
        assert_eq!(rendered("?x@@3PEAPEAHEA"), "int **x");
        assert_eq!(rendered("?foo@@3Y123KA"), "unsigned long foo[3][4]");
        assert_eq!(rendered("?x@@3PEAY02HEA"), "int (*x)[3]");
        assert_eq!(
            rendered("?x@@3P6AHMNH@ZEA"),
            "int (__cdecl *x)(float, double, int)"
        );
        assert_eq!(rendered("?x@@3PEAUty@@EA"), "struct ty *x");
        assert_eq!(rendered("?x@@3PEAW4ty@@EA"), "enum ty *x");
        assert_eq!(rendered("?l@@3P8foo@@AEHH@ZQ1@"), "int (__thiscall foo::*l)(int)");
    }

    #[test]
    fn prints_special_names() {
        assert_eq!(rendered("??_7Base@@6B@"), "const Base::`vftable'");
        assert_eq!(
            rendered("??_7A@@6BB@@C@@D@@@"),
            "const A::`vftable'{for `B's `C's `D'}"
        );
        assert_eq!(rendered("??_SBase@@6B@"), "const Base::`local vftable'");
        assert_eq!(rendered("??_8Middle2@@7B@"), "const Middle2::`vbtable'");
        assert_eq!(
            rendered("??_R0?AUBase@@@8"),
            "struct Base `RTTI Type Descriptor'"
        );
        assert_eq!(
            rendered(".?AUBase@@"),
            "struct Base `RTTI Type Descriptor Name'"
        );
        assert_eq!(
            rendered("??_R1A@?0A@EA@Base@@8"),
            "Base::`RTTI Base Class Descriptor at (0, -1, 0, 64)'"
        );
        assert_eq!(
            rendered("??_R2Base@@8"),
            "Base::`RTTI Base Class Array'"
        );
        assert_eq!(
            rendered("??_R3Base@@8"),
            "Base::`RTTI Class Hierarchy Descriptor'"
        );
        assert_eq!(
            rendered("??_R4Base@@6B@"),
            "const Base::`RTTI Complete Object Locator'"
        );
        assert_eq!(rendered("??_C@_02PCEFGMJL@hi?$AA@"), "`string'");
        assert_eq!(
            rendered("??@a6a285da2eea70dba6b578022be61d81@"),
            "??@a6a285da2eea70dba6b578022be61d81@"
        );
        assert_eq!(
            rendered("??_9Base@@$B7AA"),
            "[thunk]: __cdecl Base::`vcall'{8, {flat}}"
        );
        assert_eq!(
            rendered("??_B?1??getS@@YAAAUS@@XZ@51"),
            "`struct S & __cdecl getS(void)'::`2'::`local static guard'{2}"
        );
        assert_eq!(
            rendered("??__EFoo@@YAXXZ"),
            "void __cdecl `dynamic initializer for 'Foo''(void)"
        );
    }

    #[test]
    fn prints_thunks() {
        assert_eq!(
            rendered("?f@C@@WBA@EAAHXZ"),
            "[thunk]: public: virtual int __cdecl C::f`adjustor{16}'(void)"
        );
    }

    #[test]
    fn prints_templates() {
        assert_eq!(
            rendered("?instance$initializer$@@3P6AXXZEA"),
            "void (__cdecl *instance$initializer$)(void)"
        );
        assert_eq!(
            rendered("?x@@3PEAV?$tmpl@H@@EA"),
            "class tmpl<int> *x"
        );
        assert_eq!(
            rendered("??0?$IntTemplate@$0BAA@@@QEAA@XZ"),
            "public: __cdecl IntTemplate<256>::IntTemplate<256>(void)"
        );
        assert_eq!(
            rendered("?recref@@YAXU?$type1@$E?inst@@3Urecord@@B@@@Z"),
            "void __cdecl recref(struct type1<struct record const inst>)"
        );
    }

    #[test]
    fn prints_llvm_template_and_type_spelling() {
        assert_eq!(
            rendered("?foo_abbb@@YAXV?$A@V?$B@D@@V1@V1@@@@Z"),
            "void __cdecl foo_abbb(class A<class B<char>, class B<char>, class B<char>>)"
        );
        assert_eq!(
            rendered("?callback_void@@3V?$C@$$A6AXXZ@@A"),
            "class C<void __cdecl(void)> callback_void"
        );
        assert_eq!(
            rendered("??0?$Class@$$BY0A@H@@QEAA@XZ"),
            "public: __cdecl Class<int[]>::Class<int[]>(void)"
        );
        assert_eq!(
            rendered("?f@@YAXW4<unnamed-enum-enumerator>@@@Z"),
            "void __cdecl f(enum <unnamed-enum-enumerator>)"
        );
        assert_eq!(
            rendered("?local@?1??extern_c_func@@9@4HA"),
            "int `extern \"C\" extern_c_func'::`2'::local"
        );
        assert_eq!(
            rendered("??$?BH@TemplateOps@@QAEHXZ"),
            "public: int __thiscall TemplateOps::operator<int> int(void)"
        );
    }

    #[test]
    fn reports_unprintable_productions() {
        let ast = cpp::parse_msvc("??__E?i@C@@0HA@@YAXXZ").unwrap();
        let mut out = String::new();
        assert_eq!(
            print_msvc(&ast, &mut out).unwrap_err().kind(),
            PrintErrorKind::Unsupported
        );

        let ast = cpp::parse_msvc("??$ReadField@UV@@$F7A@@@YAHAAUV@@@Z").unwrap();
        let mut out = String::new();
        assert_eq!(
            print_msvc(&ast, &mut out).unwrap_err().kind(),
            PrintErrorKind::Unsupported
        );
    }

    #[test]
    fn prints_deep_scope_chains_without_recursing() {
        let mut symbol = String::from("?a");
        for _ in 0..600 {
            symbol.push_str("@a");
        }
        symbol.push_str("@@3HA");
        let text = rendered(&symbol);
        assert!(text.starts_with("int a::a::a::a"));
        assert_eq!(text.matches("::").count(), 600);
    }

    #[test]
    fn error_reports_the_node_offset() {
        let ast = cpp::parse_msvc("??$ReadField@UV@@$F7A@@@YAHAAUV@@@Z").unwrap();
        let mut out = String::new();
        let error = print_msvc(&ast, &mut out).unwrap_err();
        assert_eq!(error.position(), 17);
        assert_eq!(error.kind(), PrintErrorKind::Unsupported);
        assert!(alloc::format!("{error}").contains("at byte 17"));
    }
}
