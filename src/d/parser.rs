//! Bounded parser for D ABI symbol names.
//!
//! The grammar is the D ABI name-mangling grammar. The parser matches bytes,
//! so a symbol never has to be UTF-8, and it rejects every production it does
//! not implement instead of guessing. Position, depth and work live in the
//! shared [`Cursor`], so the input, nesting and work budgets are metered in one
//! place; the node budget stays here because the AST is built here. Back
//! references re-scan an earlier part of the input, which the work budget
//! bounds separately from the input length.

use alloc::boxed::Box;
use alloc::vec::Vec;

use super::node::{
    Attribute, CharWidth, Component, Convention, FunctionSig, FunctionType, HexFloat, IntegerKind,
    IntegerSuffix, MAX_DEPTH, MAX_INPUT_LEN, MAX_NODES, MAX_WORK, Modifier, NameExpr, Parameter,
    ParameterKind, ParameterPrefix, Primitive, Root, StringWidth, Symbol, TemplateArg, Type, Value,
    Variadic,
};
use super::{Ast, ParseError};
use crate::parse::{Cursor, Fail, FailKind, Limits, State};

/// Parse a D symbol from raw bytes.
pub(crate) fn parse_bytes(symbol: &[u8]) -> Result<Ast<'_>, ParseError> {
    let mut state = State::new(limits());
    let mut cursor = Cursor::new(symbol, 0, &mut state).map_err(map_fail)?;

    // A Mach-O symbol carries one leading underscore before the ABI prefix.
    let base = if symbol.starts_with(b"__D") {
        1
    } else if symbol.starts_with(b"_D") {
        0
    } else {
        return Err(ParseError::Invalid);
    };

    if symbol.get(base..) == Some(b"_Dmain") {
        return Ok(Ast { root: Root::Main });
    }

    // A this-adjusting thunk is named `_DThn<offset>_` followed by the mangled
    // symbol it thunks to; the wrapper does not change the name.
    let mut start = base + 2;
    if symbol.get(start..start + 3) == Some(b"Thn") {
        let digits_start = start + 3;
        let mut index = digits_start;
        while symbol.get(index).is_some_and(u8::is_ascii_digit) {
            index += 1;
        }
        if index > digits_start && symbol.get(index) == Some(&b'_') {
            start = index + 1;
        }
    }

    cursor.seek(start).map_err(map_fail)?;
    let mut parser = Parser {
        cursor,
        base,
        nodes: 0,
        last_backref: symbol.len() - base,
    };

    let root = parser.parse_mangle()?;
    if !parser.cursor.remaining().is_empty() {
        return Err(ParseError::Invalid);
    }
    Ok(Ast {
        root: Root::Symbol(root),
    })
}

/// The budgets the shared cursor enforces.
fn limits() -> Limits {
    Limits {
        input: MAX_INPUT_LEN as u32,
        depth: MAX_DEPTH as u16,
        work: MAX_WORK as u32,
    }
}

/// Translate a shared-cursor failure into this backend's public error.
fn map_fail(fail: Fail) -> ParseError {
    match fail.kind {
        FailKind::Depth => ParseError::RecursedTooDeep,
        FailKind::InputBudget | FailKind::Work => ParseError::BudgetExceeded,
        _ => ParseError::Invalid,
    }
}

/// Mutable state for one parse.
struct Parser<'a, 's> {
    cursor: Cursor<'a, 's>,
    /// Absolute index of the `_D` prefix; back references are relative to it.
    base: usize,
    nodes: usize,
    /// Relative position of the innermost type back reference.
    last_backref: usize,
}

/// The cursor-backed primitives the grammar is written against.
///
/// Each one forwards to the shared cursor and maps the shared failure to the
/// public [`ParseError`], so the grammar below reads exactly as it did when the
/// position, depth and work counters were private fields.
impl<'a, 's> Parser<'a, 's> {
    fn pos(&self) -> usize {
        self.cursor.pos()
    }

    fn input_len(&self) -> usize {
        self.cursor.input_len()
    }

    fn peek(&self) -> Option<u8> {
        self.cursor.peek()
    }

    fn peek_at(&self, offset: usize) -> Option<u8> {
        self.cursor.peek_at(offset)
    }

    fn starts_with(&self, tag: &[u8]) -> bool {
        self.cursor.starts_with(tag)
    }

    fn starts_with_at(&self, pos: usize, tag: &[u8]) -> bool {
        self.cursor.starts_with_at(pos, tag)
    }

    /// The byte at the absolute position `pos`, which may lie behind the
    /// cursor when a back reference is resolved.
    fn byte_at(&self, pos: usize) -> Option<u8> {
        self.cursor.byte_at(pos)
    }

    /// The bytes in `start..end`, which may lie behind the cursor.
    fn bytes(&self, start: usize, end: usize) -> &'a [u8] {
        self.cursor.bytes_between(start, end).unwrap_or(&[])
    }

    fn charge(&mut self, units: usize) -> Result<(), ParseError> {
        self.cursor.charge(units).map_err(map_fail)
    }

    fn seek(&mut self, pos: usize) -> Result<(), ParseError> {
        self.cursor.seek(pos).map_err(map_fail)
    }

    fn advance(&mut self, len: usize) -> Result<(), ParseError> {
        self.cursor.advance(len).map_err(map_fail).map(|_| ())
    }

    fn take(&mut self, len: usize) -> Result<&'a [u8], ParseError> {
        let start = self.pos();
        self.advance(len)?;
        Ok(self.bytes(start, self.pos()))
    }

    fn enter(&mut self) -> Result<(), ParseError> {
        self.cursor.enter().map_err(map_fail)
    }

    fn leave(&mut self) {
        self.cursor.leave();
    }

    fn node(&mut self) -> Result<(), ParseError> {
        self.nodes = self.nodes.saturating_add(1);
        if self.nodes > MAX_NODES {
            return Err(ParseError::BudgetExceeded);
        }
        Ok(())
    }

    /// Parse `_D QualifiedName Type` and require the whole input.
    fn parse_mangle(&mut self) -> Result<Symbol<'a>, ParseError> {
        let symbol = self.parse_qualified()?;
        self.parse_symbol_tail(&symbol)?;
        Ok(symbol)
    }

    /// Consume the `Type` (or `Z`) that terminates a symbol.
    ///
    /// A signature that came from a back reference already carries the return
    /// type of the referenced function type, so nothing follows it.
    fn parse_symbol_tail(&mut self, symbol: &Symbol<'a>) -> Result<(), ParseError> {
        if self.peek() == Some(b'Z') {
            self.advance(1)?;
            return Ok(());
        }
        let complete = symbol
            .components
            .last()
            .and_then(|component| component.function.as_ref())
            .is_some_and(|function| function.ret_consumed);
        if complete {
            return Ok(());
        }
        let _ = self.parse_type()?;
        Ok(())
    }

    /// Parse a nested `_D QualifiedName Type` without requiring the end.
    fn parse_nested_mangle(&mut self, start: usize) -> Result<Symbol<'a>, ParseError> {
        if !self.starts_with_at(start, b"_D") {
            return Err(ParseError::Invalid);
        }
        self.seek(start + 2)?;
        let symbol = self.parse_qualified()?;
        self.parse_symbol_tail(&symbol)?;
        Ok(symbol)
    }

    /// Parse a qualified name.
    fn parse_qualified(&mut self) -> Result<Symbol<'a>, ParseError> {
        self.enter()?;
        let result = self.parse_qualified_inner();
        self.leave();
        result
    }

    fn parse_qualified_inner(&mut self) -> Result<Symbol<'a>, ParseError> {
        let mut components = Vec::new();
        loop {
            if self.peek() == Some(b'0') {
                // Anonymous symbols are skipped without creating a component.
                while self.peek() == Some(b'0') {
                    self.advance(1)?;
                }
            } else {
                let name = self.parse_identifier()?;
                let function = self.parse_optional_function_sig();
                self.node()?;
                components.push(Component { name, function });
            }

            if !self.symbol_name_p(self.pos()) {
                break;
            }
        }

        if components.is_empty() {
            return Err(ParseError::Invalid);
        }

        // A phrase-prepending special (`initializer for `, …) transforms the
        // whole qualified name, so it is only meaningful as the final
        // component and cannot carry a function signature.
        if let Some((last, rest)) = components.split_last() {
            if rest.iter().any(|component| is_prepend(&component.name)) {
                return Err(ParseError::Invalid);
            }
            if last.function.is_some() && is_prepend(&last.name) {
                return Err(ParseError::Invalid);
            }
        }

        Ok(Symbol { components })
    }

    /// Parse the optional `M TypeModifiers? TypeFunctionNoReturn` suffix of a
    /// component, backtracking when it is not a complete continuation.
    fn parse_optional_function_sig(&mut self) -> Option<FunctionSig<'a>> {
        if self.peek() != Some(b'M') && !self.is_call_convention(self.peek()) {
            return None;
        }

        let start = self.pos();
        match self.parse_function_sig() {
            // A back-referenced function type carries its return type, so it
            // may be the last thing in the symbol.
            Ok(sig) if self.pos() < self.input_len() || sig.ret_consumed => Some(sig),
            _ => {
                // `start` is a position this parser already reached, so the
                // rewind cannot fail.
                let _ = self.seek(start);
                None
            }
        }
    }

    fn parse_function_sig(&mut self) -> Result<FunctionSig<'a>, ParseError> {
        let modifiers = if self.peek() == Some(b'M') {
            self.advance(1)?;
            let modifiers = self.parse_type_modifiers_suffix()?;
            // The mangler emits a type back reference instead of the whole
            // function type once an identical one has been emitted, so the
            // signature and its return type can be behind `M Q<ref>`.
            if self.function_type_backref_p() {
                let function = self.parse_function_backref()?;
                return Ok(FunctionSig {
                    params: function.params,
                    variadic: function.variadic,
                    modifiers,
                    ret_consumed: true,
                });
            }
            modifiers
        } else {
            Vec::new()
        };

        let _ = self.parse_call_convention()?;
        let _ = self.parse_attributes()?;
        let (params, variadic) = self.parse_parameters()?;
        Ok(FunctionSig {
            params,
            variadic,
            modifiers,
            ret_consumed: false,
        })
    }

    /// True when the current byte is a back reference to a type.
    ///
    /// An identifier back reference points at a length digit and continues the
    /// qualified name; a type back reference points at a letter.
    fn function_type_backref_p(&self) -> bool {
        if self.peek() != Some(b'Q') {
            return false;
        }
        let Some((value, _)) = self.decode_backref(self.pos() + 1) else {
            return false;
        };
        let Some(target) = self.pos().checked_sub(value) else {
            return false;
        };
        if target < self.base {
            return false;
        }
        self.byte_at(target)
            .is_some_and(|byte| !byte.is_ascii_digit())
    }

    /// True when `byte` starts a function calling convention.
    fn is_call_convention(&self, byte: Option<u8>) -> bool {
        matches!(byte, Some(b'F' | b'U' | b'W' | b'V' | b'R' | b'Y'))
    }

    /// True when `pos` starts a `SymbolName`.
    fn symbol_name_p(&self, pos: usize) -> bool {
        match self.byte_at(pos) {
            Some(b'0'..=b'9') => true,
            Some(b'_')
                if self.byte_at(pos + 1) == Some(b'_')
                    && matches!(self.byte_at(pos + 2), Some(b'T' | b'U')) =>
            {
                true
            }
            Some(b'Q') => {
                let Some((value, _)) = self.decode_backref(pos + 1) else {
                    return false;
                };
                let Some(target) = pos.checked_sub(value) else {
                    return false;
                };
                if target < self.base {
                    return false;
                }
                matches!(self.byte_at(target), Some(b'0'..=b'9'))
            }
            _ => false,
        }
    }

    /// Decode a base-26 back reference starting at `pos`.
    ///
    /// Returns the value and the position after the reference, or `None` for
    /// an invalid or zero reference.
    fn decode_backref(&self, pos: usize) -> Option<(usize, usize)> {
        let mut value: u64 = 0;
        let mut index = pos;
        while let Some(byte) = self.byte_at(index) {
            if !byte.is_ascii_alphabetic() {
                break;
            }
            if value > (u64::MAX - 25) / 26 {
                break;
            }
            value *= 26;
            if byte.is_ascii_lowercase() {
                value = value.checked_add(u64::from(byte - b'a'))?;
                if value == 0 {
                    return None;
                }
                let value = usize::try_from(value).ok()?;
                return Some((value, index + 1));
            }
            value = value.checked_add(u64::from(byte - b'A'))?;
            index += 1;
        }
        None
    }

    /// Parse one identifier, including templates, back references and fake
    /// function-local parents.
    ///
    /// A `__S<digits>` fake parent is a length-prefixed token that only names
    /// the enclosing scope; the identifier it belongs to follows it. The whole
    /// chain is consumed in a loop rather than by tail recursion, so the input
    /// length and the work budget bound it and the stack does not grow with
    /// the number of fake parents.
    fn parse_identifier(&mut self) -> Result<NameExpr<'a>, ParseError> {
        self.node()?;

        loop {
            if self.peek() == Some(b'Q') {
                return self.parse_symbol_backref();
            }

            if self.starts_with(b"__T") || self.starts_with(b"__U") {
                return self.parse_template(None);
            }

            let (len, after) = self.parse_number_at(self.pos())?;
            let len = len as usize;
            if len == 0 {
                return Err(ParseError::Invalid);
            }
            if self.input_len().saturating_sub(after) < len {
                return Err(ParseError::Invalid);
            }

            if len >= 5
                && (self.starts_with_at(after, b"__T") || self.starts_with_at(after, b"__U"))
            {
                self.seek(after)?;
                return self.parse_template(Some(len));
            }

            if len >= 4 && self.starts_with_at(after, b"__S") {
                let digits = self.bytes(after + 3, after + len);
                if digits.iter().all(u8::is_ascii_digit) {
                    self.seek(after + len)?;
                    continue;
                }
            }

            self.seek(after)?;
            let name = self.lname_expr(self.pos(), len);
            let consumed = if matches!(name, NameExpr::Postblit) {
                len + 3
            } else {
                len
            };
            self.advance(consumed)?;
            return Ok(name);
        }
    }

    /// Classify an LName without consuming it.
    fn lname_expr(&self, start: usize, len: usize) -> NameExpr<'a> {
        let bytes = self.bytes(start, start + len);
        match len {
            6 => {
                if bytes == b"__ctor" {
                    NameExpr::This
                } else if bytes == b"__dtor" {
                    NameExpr::Destructor
                } else if self.starts_with_at(start, b"__initZ") {
                    NameExpr::Initializer
                } else if self.starts_with_at(start, b"__vtblZ") {
                    NameExpr::Vtable
                } else {
                    NameExpr::Plain(bytes)
                }
            }
            7 if self.starts_with_at(start, b"__ClassZ") => NameExpr::ClassInfo,
            10 if self.starts_with_at(start, b"__postblitMFZ") => NameExpr::Postblit,
            11 if self.starts_with_at(start, b"__InterfaceZ") => NameExpr::Interface,
            12 if self.starts_with_at(start, b"__ModuleInfoZ") => NameExpr::ModuleInfo,
            _ => NameExpr::Plain(bytes),
        }
    }

    /// Parse a template instance. `known_len` is the encoded length of the
    /// whole `__T...Z` sequence when it was length-prefixed.
    fn parse_template(&mut self, known_len: Option<usize>) -> Result<NameExpr<'a>, ParseError> {
        self.enter()?;
        let result = self.parse_template_inner(known_len);
        self.leave();
        result
    }

    fn parse_template_inner(
        &mut self,
        known_len: Option<usize>,
    ) -> Result<NameExpr<'a>, ParseError> {
        let start = self.pos();
        if !self.symbol_name_p(start + 3) || self.byte_at(start + 3) == Some(b'0') {
            return Err(ParseError::Invalid);
        }
        self.advance(3)?;
        let name = self.parse_identifier()?;
        if is_prepend(&name) {
            return Err(ParseError::Invalid);
        }
        let args = self.parse_template_args()?;
        if let Some(len) = known_len
            && self.pos() - start != len
        {
            return Err(ParseError::Invalid);
        }
        Ok(NameExpr::Template {
            name: Box::new(name),
            args,
        })
    }

    fn parse_template_args(&mut self) -> Result<Vec<TemplateArg<'a>>, ParseError> {
        self.enter()?;
        let result = self.parse_template_args_inner();
        self.leave();
        result
    }

    fn parse_template_args_inner(&mut self) -> Result<Vec<TemplateArg<'a>>, ParseError> {
        let mut args = Vec::new();
        loop {
            match self.peek() {
                Some(b'Z') => {
                    self.advance(1)?;
                    return Ok(args);
                }
                Some(_) => {}
                None => return Err(ParseError::Invalid),
            }

            if self.peek() == Some(b'H') {
                self.advance(1)?;
            }

            match self.peek() {
                Some(b'S') => {
                    self.advance(1)?;
                    let symbol = self.parse_template_symbol_param()?;
                    args.push(TemplateArg::Symbol(symbol));
                }
                Some(b'T') => {
                    self.advance(1)?;
                    let ty = self.parse_type()?;
                    args.push(TemplateArg::Type(ty));
                }
                Some(b'V') => {
                    self.advance(1)?;
                    args.push(self.parse_value_arg()?);
                }
                Some(b'X') => {
                    self.advance(1)?;
                    let (len, after) = self.parse_number_at(self.pos())?;
                    let len = len as usize;
                    if self.input_len().saturating_sub(after) < len {
                        return Err(ParseError::Invalid);
                    }
                    self.seek(after)?;
                    let bytes = self.take(len)?;
                    args.push(TemplateArg::External(bytes));
                }
                _ => return Err(ParseError::Invalid),
            }
            self.node()?;
        }
    }

    fn parse_value_arg(&mut self) -> Result<TemplateArg<'a>, ParseError> {
        let kind = self.value_type_tag(self.pos())?;
        let ty = self.parse_type()?;
        let value = self.parse_value(kind, Some(ty))?;
        Ok(TemplateArg::Value { value })
    }

    /// The raw first type byte used to format a `V` argument, resolving a
    /// back-referenced type.
    fn value_type_tag(&self, pos: usize) -> Result<u8, ParseError> {
        let byte = self.byte_at(pos).ok_or(ParseError::Invalid)?;
        if byte != b'Q' {
            return Ok(byte);
        }
        let (value, _) = self.decode_backref(pos + 1).ok_or(ParseError::Invalid)?;
        let target = pos.checked_sub(value).ok_or(ParseError::Invalid)?;
        if target < self.base {
            return Err(ParseError::Invalid);
        }
        self.byte_at(target).ok_or(ParseError::Invalid)
    }

    /// Parse `S QualifiedName` after the `S` tag.
    fn parse_template_symbol_param(&mut self) -> Result<Symbol<'a>, ParseError> {
        if self.starts_with(b"_D") && self.symbol_name_p(self.pos() + 2) {
            return self.parse_nested_mangle(self.pos());
        }
        if self.peek() == Some(b'Q') {
            return self.parse_qualified();
        }

        let (len, endptr) = self.parse_number_at(self.pos())?;
        let len = len as usize;
        let digits_start = self.pos();
        let mut psize = len;
        let mut pend = endptr;
        let mut unchecked = false;

        loop {
            let attempt_pos = pend;
            if psize == 0 {
                psize = len;
                pend = endptr;
                unchecked = true;
            }

            self.seek(attempt_pos)?;
            let attempt = if self.symbol_name_p(attempt_pos) {
                self.parse_qualified().map(|symbol| (symbol, self.pos()))
            } else if self.starts_with_at(attempt_pos, b"_D") && self.symbol_name_p(attempt_pos + 2)
            {
                self.parse_nested_mangle(attempt_pos)
                    .map(|symbol| (symbol, self.pos()))
            } else {
                Err(ParseError::Invalid)
            };

            if let Ok((symbol, end)) = attempt
                && (unchecked || end - attempt_pos == psize)
            {
                return Ok(symbol);
            }

            if unchecked {
                break;
            }
            psize /= 10;
            if pend == 0 || pend <= digits_start {
                break;
            }
            pend -= 1;
        }

        Err(ParseError::Invalid)
    }

    /// Parse a back-referenced identifier.
    fn parse_symbol_backref(&mut self) -> Result<NameExpr<'a>, ParseError> {
        let qpos = self.pos();
        let (value, end) = self.decode_backref(qpos + 1).ok_or(ParseError::Invalid)?;
        let target = qpos.checked_sub(value).ok_or(ParseError::Invalid)?;
        if target < self.base {
            return Err(ParseError::Invalid);
        }
        self.advance(end - qpos)?;

        let (len, after) = self.parse_number_at(target)?;
        let len = len as usize;
        if len == 0 || self.input_len().saturating_sub(after) < len {
            return Err(ParseError::Invalid);
        }
        Ok(self.lname_expr(after, len))
    }

    /// Parse a back-referenced type.
    fn parse_type_backref(&mut self) -> Result<Type<'a>, ParseError> {
        self.parse_backref(Self::parse_type)
    }

    fn parse_function_backref(&mut self) -> Result<FunctionType<'a>, ParseError> {
        self.parse_backref(Self::parse_function_type)
    }

    /// Resolve a `Q<ref>` back reference with `parse`.
    ///
    /// A reference must point strictly behind the innermost one already
    /// resolved, so a reference cycle cannot exist; `last_backref` is restored
    /// however the nested parse ends. `parse` is the production the reference
    /// resolves to, which keeps the two callers monomorphised.
    fn parse_backref<T>(
        &mut self,
        parse: fn(&mut Self) -> Result<T, ParseError>,
    ) -> Result<T, ParseError> {
        let qpos = self.pos();
        let relative = qpos - self.base;
        if relative >= self.last_backref {
            return Err(ParseError::Invalid);
        }

        let saved = self.last_backref;
        self.last_backref = relative;

        let result = (|| {
            let (value, end) = self.decode_backref(qpos + 1).ok_or(ParseError::Invalid)?;
            let target = qpos.checked_sub(value).ok_or(ParseError::Invalid)?;
            if target < self.base {
                return Err(ParseError::Invalid);
            }

            let outer = end;
            self.seek(target)?;
            let parsed = parse(self);
            self.seek(outer)?;
            parsed
        })();

        self.last_backref = saved;
        result
    }

    fn parse_type(&mut self) -> Result<Type<'a>, ParseError> {
        self.enter()?;
        let result = self.parse_type_inner();
        self.leave();
        result
    }

    fn parse_type_inner(&mut self) -> Result<Type<'a>, ParseError> {
        if self.peek() == Some(b'Q') {
            return self.parse_type_backref();
        }

        match self.peek() {
            Some(b'O') => {
                self.advance(1)?;
                Ok(Type::Shared(Box::new(self.parse_type()?)))
            }
            Some(b'x') => {
                self.advance(1)?;
                Ok(Type::Const(Box::new(self.parse_type()?)))
            }
            Some(b'y') => {
                self.advance(1)?;
                Ok(Type::Immutable(Box::new(self.parse_type()?)))
            }
            Some(b'N') => {
                self.advance(1)?;
                match self.peek() {
                    Some(b'g') => {
                        self.advance(1)?;
                        Ok(Type::Inout(Box::new(self.parse_type()?)))
                    }
                    Some(b'h') => {
                        self.advance(1)?;
                        Ok(Type::Vector(Box::new(self.parse_type()?)))
                    }
                    Some(b'n') => {
                        self.advance(1)?;
                        Ok(Type::TypeofStarNull)
                    }
                    _ => Err(ParseError::Invalid),
                }
            }
            Some(b'A') => {
                self.advance(1)?;
                Ok(Type::DynamicArray(Box::new(self.parse_type()?)))
            }
            Some(b'G') => {
                self.advance(1)?;
                let start = self.pos();
                while self.peek().is_some_and(|byte| byte.is_ascii_digit()) {
                    self.advance(1)?;
                }
                let length = self.bytes(start, self.pos());
                let element = self.parse_type()?;
                Ok(Type::StaticArray {
                    element: Box::new(element),
                    length,
                })
            }
            Some(b'H') => {
                self.advance(1)?;
                let key = self.parse_type()?;
                let value = self.parse_type()?;
                Ok(Type::AssocArray {
                    key: Box::new(key),
                    value: Box::new(value),
                })
            }
            Some(b'P') => {
                self.advance(1)?;
                if self.is_call_convention(self.peek()) {
                    Ok(Type::Function(self.parse_function_type()?))
                } else {
                    Ok(Type::Pointer(Box::new(self.parse_type()?)))
                }
            }
            Some(b'F' | b'U' | b'W' | b'V' | b'R' | b'Y') => {
                Ok(Type::Function(self.parse_function_type()?))
            }
            Some(b'C' | b'S' | b'E' | b'T' | b'I') => {
                self.advance(1)?;
                Ok(Type::Named(self.parse_qualified()?))
            }
            Some(b'D') => {
                self.advance(1)?;
                let modifiers = self.parse_type_modifiers_suffix()?;
                let function = if self.peek() == Some(b'Q') {
                    self.parse_function_backref()?
                } else {
                    self.parse_function_type()?
                };
                Ok(Type::Delegate {
                    function,
                    modifiers,
                })
            }
            Some(b'B') => {
                self.advance(1)?;
                let (count, after) = self.parse_number_at(self.pos())?;
                self.seek(after)?;
                let mut types = Vec::new();
                for _ in 0..count {
                    types.push(self.parse_type()?);
                }
                Ok(Type::Tuple(types))
            }
            Some(b'n') => {
                self.advance(1)?;
                Ok(Type::TypeofNull)
            }
            Some(byte) => {
                if let Some(primitive) = primitive(byte) {
                    self.advance(1)?;
                    return Ok(Type::Primitive(primitive));
                }
                if byte == b'z' {
                    self.advance(1)?;
                    return match self.peek() {
                        Some(b'i') => {
                            self.advance(1)?;
                            Ok(Type::Primitive(Primitive::Cent))
                        }
                        Some(b'k') => {
                            self.advance(1)?;
                            Ok(Type::Primitive(Primitive::Ucent))
                        }
                        _ => Err(ParseError::Invalid),
                    };
                }
                Err(ParseError::Invalid)
            }
            None => Err(ParseError::Invalid),
        }
    }

    fn parse_function_type(&mut self) -> Result<FunctionType<'a>, ParseError> {
        self.enter()?;
        let result = self.parse_function_type_inner();
        self.leave();
        result
    }

    fn parse_function_type_inner(&mut self) -> Result<FunctionType<'a>, ParseError> {
        let convention = self.parse_call_convention()?;
        let attrs = self.parse_attributes()?;
        let (params, variadic) = self.parse_parameters()?;
        let ret = self.parse_type()?;
        Ok(FunctionType {
            convention,
            attrs,
            params,
            variadic,
            ret: Box::new(ret),
        })
    }

    fn parse_call_convention(&mut self) -> Result<Convention, ParseError> {
        let convention = match self.peek() {
            Some(b'F') => Convention::D,
            Some(b'U') => Convention::C,
            Some(b'W') => Convention::Windows,
            Some(b'V') => Convention::Pascal,
            Some(b'R') => Convention::Cpp,
            Some(b'Y') => Convention::ObjectiveC,
            _ => return Err(ParseError::Invalid),
        };
        self.advance(1)?;
        Ok(convention)
    }

    fn parse_attributes(&mut self) -> Result<Vec<Attribute>, ParseError> {
        let mut attrs = Vec::new();
        while self.peek() == Some(b'N') {
            self.advance(1)?;
            let attr = match self.peek() {
                Some(b'a') => Attribute::Pure,
                Some(b'b') => Attribute::Nothrow,
                Some(b'c') => Attribute::Ref,
                Some(b'd') => Attribute::Property,
                Some(b'e') => Attribute::Trusted,
                Some(b'f') => Attribute::Safe,
                Some(b'i') => Attribute::Nogc,
                Some(b'j') => Attribute::Return,
                Some(b'l') => Attribute::Scope,
                Some(b'm') => Attribute::Live,
                // `Ng`/`Nh`/`Nk`/`Nn` start a parameter, not an attribute.
                Some(b'g' | b'h' | b'k' | b'n') => {
                    self.seek(self.pos() - 1)?;
                    break;
                }
                _ => return Err(ParseError::Invalid),
            };
            self.advance(1)?;
            attrs.push(attr);
        }
        Ok(attrs)
    }

    fn parse_type_modifiers_suffix(&mut self) -> Result<Vec<Modifier>, ParseError> {
        let mut modifiers = Vec::new();
        loop {
            match self.peek() {
                Some(b'x') => {
                    self.advance(1)?;
                    modifiers.push(Modifier::Const);
                    break;
                }
                Some(b'y') => {
                    self.advance(1)?;
                    modifiers.push(Modifier::Immutable);
                    break;
                }
                Some(b'O') => {
                    self.advance(1)?;
                    modifiers.push(Modifier::Shared);
                }
                Some(b'N') if self.peek_at(1) == Some(b'g') => {
                    self.advance(2)?;
                    modifiers.push(Modifier::Inout);
                }
                _ => break,
            }
        }
        Ok(modifiers)
    }

    fn parse_parameters(&mut self) -> Result<(Vec<Parameter<'a>>, Variadic), ParseError> {
        let mut params = Vec::new();
        loop {
            match self.peek() {
                Some(b'X') => {
                    self.advance(1)?;
                    return Ok((params, Variadic::Typesafe));
                }
                Some(b'Y') => {
                    self.advance(1)?;
                    return Ok((params, Variadic::DStyle));
                }
                Some(b'Z') => {
                    self.advance(1)?;
                    return Ok((params, Variadic::None));
                }
                Some(_) => {}
                None => return Err(ParseError::Invalid),
            }

            let prefix = self.parse_parameter_prefix()?;

            let kind = match self.peek() {
                Some(b'I') => {
                    self.advance(1)?;
                    if self.peek() == Some(b'K') {
                        self.advance(1)?;
                        ParameterKind::InRef
                    } else {
                        ParameterKind::In
                    }
                }
                Some(b'J') => {
                    self.advance(1)?;
                    ParameterKind::Out
                }
                Some(b'K') => {
                    self.advance(1)?;
                    ParameterKind::Ref
                }
                Some(b'L') => {
                    self.advance(1)?;
                    ParameterKind::Lazy
                }
                _ => ParameterKind::Plain,
            };

            let ty = self.parse_type()?;
            self.node()?;
            params.push(Parameter { prefix, kind, ty });
        }
    }

    /// Parse the `scope` and `return` markers in front of a parameter.
    ///
    /// Both markers may appear in either order: the mangler emits `NkM` for a
    /// `return scope` parameter and `MNk` for a `scope return` one.
    fn parse_parameter_prefix(&mut self) -> Result<ParameterPrefix, ParseError> {
        let mut scope = false;
        let mut is_return = false;
        let mut return_first = false;
        loop {
            match self.peek() {
                Some(b'M') if !scope => {
                    self.advance(1)?;
                    scope = true;
                }
                Some(b'N') if !is_return && self.peek_at(1) == Some(b'k') => {
                    self.advance(2)?;
                    is_return = true;
                    return_first = !scope;
                }
                _ => break,
            }
        }
        Ok(match (scope, is_return) {
            (false, false) => ParameterPrefix::Plain,
            (true, false) => ParameterPrefix::Scope,
            (false, true) => ParameterPrefix::Return,
            (true, true) if return_first => ParameterPrefix::ReturnScope,
            (true, true) => ParameterPrefix::ScopeReturn,
        })
    }

    /// Parse a template `V` value after the tag.
    ///
    /// `name` is the type spelling of the enclosing `V` argument; only a
    /// direct struct literal prints it.
    fn parse_value(&mut self, kind: u8, name: Option<Type<'a>>) -> Result<Value<'a>, ParseError> {
        self.enter()?;
        let result = self.parse_value_inner(kind, name);
        self.leave();
        result
    }

    fn parse_value_inner(
        &mut self,
        kind: u8,
        name: Option<Type<'a>>,
    ) -> Result<Value<'a>, ParseError> {
        match self.peek() {
            Some(b'n') => {
                self.advance(1)?;
                Ok(Value::Null)
            }
            Some(b'N') => {
                self.advance(1)?;
                let integer = self.parse_integer(kind)?;
                Ok(Value::Integer {
                    negative: true,
                    kind: integer,
                })
            }
            Some(b'i') => {
                self.advance(1)?;
                let integer = self.parse_integer(kind)?;
                Ok(Value::Integer {
                    negative: false,
                    kind: integer,
                })
            }
            Some(b'0'..=b'9') => {
                let integer = self.parse_integer(kind)?;
                Ok(Value::Integer {
                    negative: false,
                    kind: integer,
                })
            }
            Some(b'e') => {
                self.advance(1)?;
                Ok(Value::Real(self.parse_hex_float()?))
            }
            Some(b'c') => {
                self.advance(1)?;
                let re = self.parse_hex_float()?;
                if self.peek() != Some(b'c') {
                    return Err(ParseError::Invalid);
                }
                self.advance(1)?;
                let im = self.parse_hex_float()?;
                Ok(Value::Complex { re, im })
            }
            Some(b'a' | b'w' | b'd') => self.parse_string(),
            Some(b'A') => {
                self.advance(1)?;
                self.parse_array_literal(kind)
            }
            Some(b'S') => {
                self.advance(1)?;
                self.parse_struct_literal(name)
            }
            Some(b'f') => {
                self.advance(1)?;
                if !self.starts_with(b"_D") || !self.symbol_name_p(self.pos() + 2) {
                    return Err(ParseError::Invalid);
                }
                let symbol = self.parse_nested_mangle(self.pos())?;
                Ok(Value::Symbol(symbol))
            }
            _ => Err(ParseError::Invalid),
        }
    }

    fn parse_integer(&mut self, kind: u8) -> Result<IntegerKind<'a>, ParseError> {
        match kind {
            b'a' | b'u' | b'w' => {
                let (value, after) = self.parse_number_at(self.pos())?;
                self.seek(after)?;
                let width = match kind {
                    b'a' => CharWidth::Utf8,
                    b'u' => CharWidth::Utf16,
                    _ => CharWidth::Utf32,
                };
                Ok(IntegerKind::Char(width, u64::from(value)))
            }
            b'b' => {
                let (value, after) = self.parse_number_at(self.pos())?;
                self.seek(after)?;
                Ok(IntegerKind::Bool(value != 0))
            }
            _ => {
                if !self.peek().is_some_and(|byte| byte.is_ascii_digit()) {
                    return Err(ParseError::Invalid);
                }
                let start = self.pos();
                while self.peek().is_some_and(|byte| byte.is_ascii_digit()) {
                    self.advance(1)?;
                }
                let digits = self.bytes(start, self.pos());
                let suffix = match kind {
                    b'h' | b't' | b'k' => IntegerSuffix::U,
                    b'l' => IntegerSuffix::L,
                    b'm' => IntegerSuffix::UL,
                    _ => IntegerSuffix::None,
                };
                Ok(IntegerKind::Number { digits, suffix })
            }
        }
    }

    fn parse_hex_float(&mut self) -> Result<HexFloat<'a>, ParseError> {
        if self.starts_with(b"NAN") {
            self.advance(3)?;
            return Ok(HexFloat::Nan);
        }
        if self.starts_with(b"INF") {
            self.advance(3)?;
            return Ok(HexFloat::Inf);
        }
        if self.starts_with(b"NINF") {
            self.advance(4)?;
            return Ok(HexFloat::NegInf);
        }

        let negative = if self.peek() == Some(b'N') {
            self.advance(1)?;
            true
        } else {
            false
        };

        let first = self.peek().ok_or(ParseError::Invalid)?;
        if !first.is_ascii_hexdigit() {
            return Err(ParseError::Invalid);
        }
        self.advance(1)?;

        let rest_start = self.pos();
        while self.peek().is_some_and(|byte| byte.is_ascii_hexdigit()) {
            self.advance(1)?;
        }
        let rest = self.bytes(rest_start, self.pos());

        if self.peek() != Some(b'P') {
            return Err(ParseError::Invalid);
        }
        self.advance(1)?;

        let exponent_negative = if self.peek() == Some(b'N') {
            self.advance(1)?;
            true
        } else {
            false
        };

        let exponent_start = self.pos();
        while self.peek().is_some_and(|byte| byte.is_ascii_digit()) {
            self.advance(1)?;
        }
        let exponent = self.bytes(exponent_start, self.pos());

        Ok(HexFloat::Number {
            negative,
            first,
            rest,
            exponent_negative,
            exponent,
        })
    }

    fn parse_string(&mut self) -> Result<Value<'a>, ParseError> {
        let width = match self.peek() {
            Some(b'a') => StringWidth::Utf8,
            Some(b'w') => StringWidth::Utf16,
            Some(b'd') => StringWidth::Utf32,
            _ => return Err(ParseError::Invalid),
        };
        self.advance(1)?;

        let (len, after) = self.parse_number_at(self.pos())?;
        self.seek(after)?;
        if self.peek() != Some(b'_') {
            return Err(ParseError::Invalid);
        }
        self.advance(1)?;

        let total = (len as usize).checked_mul(2).ok_or(ParseError::Invalid)?;
        let start = self.pos();
        if self.input_len().saturating_sub(start) < total {
            return Err(ParseError::Invalid);
        }
        let payload = self.bytes(start, start + total);
        if !payload.iter().all(u8::is_ascii_hexdigit) {
            return Err(ParseError::Invalid);
        }
        let hex = self.take(total)?;

        Ok(Value::String { width, hex })
    }

    fn parse_array_literal(&mut self, kind: u8) -> Result<Value<'a>, ParseError> {
        let (count, after) = self.parse_number_at(self.pos())?;
        self.seek(after)?;

        if kind == b'H' {
            let mut pairs = Vec::new();
            for _ in 0..count {
                let key = self.parse_value(0, None)?;
                let value = self.parse_value(0, None)?;
                pairs.push((key, value));
                self.node()?;
            }
            Ok(Value::AssocArray(pairs))
        } else {
            let mut values = Vec::new();
            for _ in 0..count {
                values.push(self.parse_value(0, None)?);
                self.node()?;
            }
            Ok(Value::Array(values))
        }
    }

    fn parse_struct_literal(&mut self, name: Option<Type<'a>>) -> Result<Value<'a>, ParseError> {
        let (count, after) = self.parse_number_at(self.pos())?;
        self.seek(after)?;

        let mut fields = Vec::new();
        for _ in 0..count {
            fields.push(self.parse_value(0, None)?);
            self.node()?;
        }

        Ok(Value::Struct { name, fields })
    }

    /// Parse a decimal number at `pos` without consuming it.
    ///
    /// Mirrors the D ABI oracle: an empty digit run, a value above
    /// `u32::MAX`, or a number at the end of the input is invalid.
    fn parse_number_at(&mut self, pos: usize) -> Result<(u32, usize), ParseError> {
        let mut index = pos;
        let mut value: u64 = 0;
        while let Some(byte) = self.byte_at(index) {
            if !byte.is_ascii_digit() {
                break;
            }
            value = value
                .checked_mul(10)
                .and_then(|value| value.checked_add(u64::from(byte - b'0')))
                .ok_or(ParseError::Invalid)?;
            if value > u64::from(u32::MAX) {
                return Err(ParseError::Invalid);
            }
            index += 1;
        }
        if index == pos || index == self.input_len() {
            return Err(ParseError::Invalid);
        }
        self.charge(index - pos)?;
        Ok((value as u32, index))
    }
}

/// True for a compiler special that prepends a phrase to the whole name.
fn is_prepend(name: &NameExpr<'_>) -> bool {
    matches!(
        name,
        NameExpr::Initializer
            | NameExpr::Vtable
            | NameExpr::ClassInfo
            | NameExpr::Interface
            | NameExpr::ModuleInfo
    )
}

/// Map a single-byte primitive type tag.
fn primitive(byte: u8) -> Option<Primitive> {
    Some(match byte {
        b'v' => Primitive::Void,
        b'g' => Primitive::Byte,
        b'h' => Primitive::Ubyte,
        b's' => Primitive::Short,
        b't' => Primitive::Ushort,
        b'i' => Primitive::Int,
        b'k' => Primitive::Uint,
        b'l' => Primitive::Long,
        b'm' => Primitive::Ulong,
        b'f' => Primitive::Float,
        b'd' => Primitive::Double,
        b'e' => Primitive::Real,
        b'o' => Primitive::Ifloat,
        b'p' => Primitive::Idouble,
        b'j' => Primitive::Ireal,
        b'q' => Primitive::Cfloat,
        b'r' => Primitive::Cdouble,
        b'c' => Primitive::Creal,
        b'b' => Primitive::Bool,
        b'a' => Primitive::Char,
        b'u' => Primitive::Wchar,
        b'w' => Primitive::Dchar,
        _ => return None,
    })
}
