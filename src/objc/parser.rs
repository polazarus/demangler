//! Bounded parser for Objective-C linker metadata and method display forms.
//!
//! Objective-C metadata is selected by a known, case-sensitive prefix; the
//! class, category and protocol names after it are kept verbatim and are never
//! split on `_`. Category records use the runtime's `_$_` separator, which is
//! the only structural separator this parser relies on.

use super::node::{
    Ast, BlockDescriptor, Item, MAX_DEPTH, MAX_INPUT_LEN, MAX_WORK, Metadata, MetadataKind,
    Method, MethodKind,
};
use super::ParseError;

/// Parse an Objective-C symbol name.
pub(crate) fn parse(symbol: &str) -> Result<Ast<'_>, ParseError> {
    if symbol.is_empty() {
        return Err(ParseError::Invalid);
    }
    if symbol.len() > MAX_INPUT_LEN {
        return Err(ParseError::BudgetExceeded);
    }

    let mut budget = Budget::new();
    budget.spend(symbol.len())?;
    check_brackets(symbol, &mut budget)?;

    if symbol.starts_with('-') || symbol.starts_with('+') {
        return Ok(Ast {
            item: Item::Method(parse_method(symbol, &mut budget)?),
        });
    }
    if symbol.starts_with("___block_descriptor_") || symbol.starts_with("__block_descriptor_") {
        return Ok(Ast {
            item: Item::BlockDescriptor(parse_block_descriptor(symbol, &mut budget)?),
        });
    }
    Ok(Ast {
        item: Item::Metadata(parse_metadata(symbol, &mut budget)?),
    })
}

/// A bounded work counter: one unit per byte scanned.
struct Budget {
    remaining: usize,
}

impl Budget {
    const fn new() -> Self {
        Self {
            remaining: MAX_WORK,
        }
    }

    fn spend(&mut self, units: usize) -> Result<(), ParseError> {
        self.remaining = self
            .remaining
            .checked_sub(units)
            .ok_or(ParseError::BudgetExceeded)?;
        Ok(())
    }
}

/// Reject unbalanced or too-deep brackets before any family-specific scan.
fn check_brackets(symbol: &str, budget: &mut Budget) -> Result<(), ParseError> {
    let mut depth = 0_usize;
    for byte in symbol.bytes() {
        budget.spend(1)?;
        match byte {
            b'[' => {
                depth += 1;
                if depth > MAX_DEPTH {
                    return Err(ParseError::RecursedTooDeep);
                }
            }
            b']' => {
                depth = depth.checked_sub(1).ok_or(ParseError::Invalid)?;
            }
            _ => {}
        }
    }
    if depth == 0 {
        Ok(())
    } else {
        Err(ParseError::Invalid)
    }
}

/// Parse a `-[Class selector:]` or `+[Class selector:]` display form.
fn parse_method<'a>(symbol: &'a str, budget: &mut Budget) -> Result<Method<'a>, ParseError> {
    let kind = match symbol.as_bytes().first() {
        Some(b'-') => MethodKind::Instance,
        Some(b'+') => MethodKind::Class,
        _ => return Err(ParseError::Invalid),
    };
    let rest = symbol.get(1..).ok_or(ParseError::Invalid)?;
    let rest = rest.strip_prefix('[').ok_or(ParseError::Invalid)?;
    let rest = rest.strip_suffix(']').ok_or(ParseError::Invalid)?;

    // Exactly one space separates the receiver spelling from the selector.
    let (receiver, selector) = rest.split_once(' ').ok_or(ParseError::Invalid)?;
    if selector.is_empty() || selector.contains(' ') {
        return Err(ParseError::Invalid);
    }

    let (class, category) = parse_receiver(receiver, budget)?;
    validate_selector(selector, budget)?;
    Ok(Method {
        kind,
        class,
        category,
        selector,
    })
}

/// Parse `Class` or `Class(Category)` from a method display form.
fn parse_receiver<'a>(
    receiver: &'a str,
    budget: &mut Budget,
) -> Result<(&'a str, Option<&'a str>), ParseError> {
    let (class, category) = if let Some((class, rest)) = receiver.split_once('(') {
        let category = rest.strip_suffix(')').ok_or(ParseError::Invalid)?;
        if category.contains('(') || category.contains(')') {
            return Err(ParseError::Invalid);
        }
        (class, Some(category))
    } else {
        (receiver, None)
    };
    validate_identifier(class, budget)?;
    if let Some(category) = category {
        validate_identifier(category, budget)?;
    }
    Ok((class, category))
}

/// Validate a selector, preserving its colons.
fn validate_selector(selector: &str, budget: &mut Budget) -> Result<(), ParseError> {
    if selector.ends_with(':') {
        let body = selector.strip_suffix(':').ok_or(ParseError::Invalid)?;
        if body.is_empty() {
            return Err(ParseError::Invalid);
        }
        for part in body.split(':') {
            if part.is_empty() {
                return Err(ParseError::Invalid);
            }
            validate_identifier(part, budget)?;
        }
        Ok(())
    } else if selector.contains(':') {
        Err(ParseError::Invalid)
    } else {
        validate_identifier(selector, budget)
    }
}

/// Validate an Objective-C identifier used in a display form.
fn validate_identifier(name: &str, budget: &mut Budget) -> Result<(), ParseError> {
    if name.is_empty() {
        return Err(ParseError::Invalid);
    }
    for byte in name.bytes() {
        budget.spend(1)?;
        if !is_identifier_byte(byte) {
            return Err(ParseError::Invalid);
        }
    }
    Ok(())
}

/// Bytes allowed in a method-display identifier.
const fn is_identifier_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'$' | b'.')
}

/// Parse a block descriptor symbol.
fn parse_block_descriptor<'a>(
    symbol: &'a str,
    budget: &mut Budget,
) -> Result<BlockDescriptor<'a>, ParseError> {
    let payload = symbol
        .strip_prefix("___block_descriptor_")
        .or_else(|| symbol.strip_prefix("__block_descriptor_"))
        .ok_or(ParseError::Invalid)?;
    if payload.is_empty() {
        return Err(ParseError::Invalid);
    }
    for byte in payload.bytes() {
        budget.spend(1)?;
        // Clang encodes the block signature with a `\x01` separator, so the
        // payload is not restricted to graphic ASCII. NUL and line breaks are
        // still rejected so the printed name stays a single line.
        if !byte.is_ascii() || matches!(byte, 0 | b'\n' | b'\r') {
            return Err(ParseError::Invalid);
        }
    }
    Ok(BlockDescriptor { payload })
}

/// Parse a runtime metadata name.
fn parse_metadata<'a>(symbol: &'a str, budget: &mut Budget) -> Result<Metadata<'a>, ParseError> {
    // Apple/Mach-O accepts one or two leading underscores and the bare
    // spelling. GNUstep's classic runtime always uses `__objc_`.
    if let Some(rest) = symbol.strip_prefix("__OBJC_") {
        return parse_apple(rest, budget);
    }
    if let Some(rest) = symbol.strip_prefix("_OBJC_") {
        return parse_apple(rest, budget);
    }
    if let Some(rest) = symbol.strip_prefix("OBJC_") {
        return parse_apple(rest, budget);
    }
    if let Some(rest) = symbol.strip_prefix("__objc_") {
        return parse_gnustep(rest, budget);
    }
    Err(ParseError::Invalid)
}

/// Parse the part after `_OBJC_` / `__OBJC_` / `OBJC_`.
fn parse_apple<'a>(rest: &'a str, budget: &mut Budget) -> Result<Metadata<'a>, ParseError> {
    // Apple's `$`-separated metadata first, so `CLASS_$_Foo` is not read as a
    // GNUstep `CLASS_Foo`. Longer prefixes come before their prefixes.
    if let Some(name) = rest.strip_prefix("CLASS_RO_$_") {
        return single(MetadataKind::ClassData, name, budget);
    }
    if let Some(name) = rest.strip_prefix("METACLASS_RO_$_") {
        return single(MetadataKind::MetaclassData, name, budget);
    }
    if let Some(name) = rest.strip_prefix("CLASS_PROTOCOLS_$_") {
        return single(MetadataKind::ClassProtocols, name, budget);
    }
    if let Some(name) = rest.strip_prefix("CATEGORY_PROTOCOLS_$_") {
        return category(MetadataKind::CategoryProtocols, name, budget);
    }
    if let Some(name) = rest.strip_prefix("LABEL_PROTOCOL_$_") {
        return single(MetadataKind::ProtocolLabel, name, budget);
    }
    if let Some(name) = rest.strip_prefix("PROTOCOL_REFERENCE_$_") {
        return single(MetadataKind::ProtocolRef, name, budget);
    }
    if let Some(name) = rest.strip_prefix("PROTOCOL_$_") {
        return single(MetadataKind::Protocol, name, budget);
    }
    if let Some(name) = rest.strip_prefix("CLASS_$_") {
        return single(MetadataKind::Class, name, budget);
    }
    if let Some(name) = rest.strip_prefix("METACLASS_$_") {
        return single(MetadataKind::Metaclass, name, budget);
    }
    if let Some(name) = rest.strip_prefix("IVAR_$_") {
        return member(MetadataKind::Ivar, name, budget);
    }
    if let Some(name) = rest.strip_prefix("$_") {
        return parse_apple_dollar(name, budget);
    }

    // GNUstep 2.0 uses the same `_OBJC_` namespace without `$`.
    if let Some(name) = rest.strip_prefix("REF_CLASS_") {
        return single(MetadataKind::ClassRef, name, budget);
    }
    if let Some(name) = rest.strip_prefix("INIT_CLASS_") {
        return single(MetadataKind::ClassInit, name, budget);
    }
    if let Some(name) = rest.strip_prefix("CLASS_") {
        return single(MetadataKind::Class, name, budget);
    }
    if let Some(name) = rest.strip_prefix("METACLASS_") {
        return single(MetadataKind::Metaclass, name, budget);
    }
    if let Some(name) = rest.strip_prefix("PROTOCOL_") {
        return single(MetadataKind::Protocol, name, budget);
    }

    // Old GNU/NeXT runtime metadata.
    if let Some(name) = rest.strip_prefix("ClassName_") {
        return single(MetadataKind::ClassName, name, budget);
    }
    if let Some(name) = rest.strip_prefix("Class_") {
        return single(MetadataKind::Class, name, budget);
    }
    if let Some(name) = rest.strip_prefix("MetaClass_") {
        return single(MetadataKind::Metaclass, name, budget);
    }
    if let Some(name) = rest.strip_prefix("InstanceMethods_") {
        return single(MetadataKind::InstanceMethods, name, budget);
    }
    if let Some(name) = rest.strip_prefix("ClassMethods_") {
        return single(MetadataKind::ClassMethods, name, budget);
    }
    if let Some(name) = rest.strip_prefix("InstanceIvars_") {
        return single(MetadataKind::InstanceVariables, name, budget);
    }
    if let Some(name) = rest.strip_prefix("ClassIvars_") {
        return single(MetadataKind::ClassVariables, name, budget);
    }
    if let Some(name) = rest.strip_prefix("METH_VAR_NAME_") {
        return index(MetadataKind::MethodName, name, budget);
    }
    if let Some(name) = rest.strip_prefix("METH_VAR_TYPE_") {
        return index(MetadataKind::MethodType, name, budget);
    }
    if rest == "Module" {
        return Ok(no_subject(MetadataKind::Module));
    }
    if rest == "SYMBOLS" {
        return Ok(no_subject(MetadataKind::Symbols));
    }
    if rest == "SELECTOR_TABLE" {
        return Ok(no_subject(MetadataKind::SelectorTable));
    }
    if let Some(name) = rest.strip_prefix("SELECTOR_REFERENCES_") {
        if name.is_empty() {
            return Ok(no_subject(MetadataKind::SelectorReferences));
        }
        return single(MetadataKind::SelectorReferences, name, budget);
    }

    Err(ParseError::Invalid)
}

/// Parse the part after Apple's `__OBJC_$_` / `_OBJC_$_`.
fn parse_apple_dollar<'a>(rest: &'a str, budget: &mut Budget) -> Result<Metadata<'a>, ParseError> {
    if let Some(name) = rest.strip_prefix("CATEGORY_INSTANCE_METHODS_") {
        return category(MetadataKind::CategoryInstanceMethods, name, budget);
    }
    if let Some(name) = rest.strip_prefix("CATEGORY_CLASS_METHODS_") {
        return category(MetadataKind::CategoryClassMethods, name, budget);
    }
    if let Some(name) = rest.strip_prefix("CATEGORY_PROP_LIST_") {
        return category(MetadataKind::CategoryPropertyList, name, budget);
    }
    if let Some(name) = rest.strip_prefix("CATEGORY_PROPERTIES_") {
        return category(MetadataKind::CategoryPropertyList, name, budget);
    }
    if let Some(name) = rest.strip_prefix("CATEGORY_") {
        return category(MetadataKind::Category, name, budget);
    }
    if let Some(name) = rest.strip_prefix("INSTANCE_METHODS_") {
        return single(MetadataKind::InstanceMethods, name, budget);
    }
    if let Some(name) = rest.strip_prefix("CLASS_METHODS_") {
        return single(MetadataKind::ClassMethods, name, budget);
    }
    if let Some(name) = rest.strip_prefix("INSTANCE_VARIABLES_") {
        return single(MetadataKind::InstanceVariables, name, budget);
    }
    if let Some(name) = rest.strip_prefix("PROP_LIST_") {
        return optional_category(MetadataKind::PropertyList, name, budget);
    }
    if let Some(name) = rest.strip_prefix("PROTOCOL_INSTANCE_METHODS_OPT_") {
        return single(MetadataKind::ProtocolOptionalInstanceMethods, name, budget);
    }
    if let Some(name) = rest.strip_prefix("PROTOCOL_CLASS_METHODS_OPT_") {
        return single(MetadataKind::ProtocolOptionalClassMethods, name, budget);
    }
    if let Some(name) = rest.strip_prefix("PROTOCOL_OPTIONAL_INSTANCE_METHODS_") {
        return single(MetadataKind::ProtocolOptionalInstanceMethods, name, budget);
    }
    if let Some(name) = rest.strip_prefix("PROTOCOL_OPTIONAL_CLASS_METHODS_") {
        return single(MetadataKind::ProtocolOptionalClassMethods, name, budget);
    }
    if let Some(name) = rest.strip_prefix("PROTOCOL_INSTANCE_METHODS_") {
        return single(MetadataKind::ProtocolInstanceMethods, name, budget);
    }
    if let Some(name) = rest.strip_prefix("PROTOCOL_CLASS_METHODS_") {
        return single(MetadataKind::ProtocolClassMethods, name, budget);
    }
    if let Some(name) = rest.strip_prefix("PROTOCOL_PROPERTIES_") {
        return single(MetadataKind::ProtocolPropertyList, name, budget);
    }
    if let Some(name) = rest.strip_prefix("PROTOCOL_PROP_LIST_") {
        return single(MetadataKind::ProtocolPropertyList, name, budget);
    }
    if let Some(name) = rest.strip_prefix("PROTOCOL_REFS_") {
        return single(MetadataKind::ProtocolRefs, name, budget);
    }
    if let Some(name) = rest.strip_prefix("PROTOCOL_METHOD_TYPES_") {
        return single(MetadataKind::ProtocolMethodTypes, name, budget);
    }
    Err(ParseError::Invalid)
}

/// Parse the part after GNUstep's `__objc_`.
fn parse_gnustep<'a>(rest: &'a str, budget: &mut Budget) -> Result<Metadata<'a>, ParseError> {
    if let Some(name) = rest.strip_prefix("class_name_") {
        return single(MetadataKind::Class, name, budget);
    }
    if let Some(name) = rest.strip_prefix("class_ref_") {
        return single(MetadataKind::ClassRef, name, budget);
    }
    if let Some(name) = rest.strip_prefix("ivar_offset_value_") {
        // GNUstep 2.0 emits an optional trailing `.` after the ivar selector.
        return member(MetadataKind::IvarOffsetValue, name.trim_end_matches(['.', '\u{1}']), budget);
    }
    if let Some(name) = rest.strip_prefix("ivar_offset_") {
        return member(MetadataKind::IvarOffset, name.trim_end_matches(['.', '\u{1}']), budget);
    }
    // `__objc_category_name_` separates class and category with `_`, which is
    // also legal inside both names, so it cannot be split unambiguously.
    Err(ParseError::Invalid)
}

/// Build a record with only a subject.
fn single<'a>(
    kind: MetadataKind,
    name: &'a str,
    budget: &mut Budget,
) -> Result<Metadata<'a>, ParseError> {
    Ok(Metadata {
        kind,
        subject: Some(validate_name(name, budget)?),
        category: None,
        member: None,
    })
}

/// Build a record with no subject.
fn no_subject(kind: MetadataKind) -> Metadata<'static> {
    Metadata {
        kind,
        subject: None,
        category: None,
        member: None,
    }
}

/// Build a category record from `Class_$_Category`.
fn category<'a>(
    kind: MetadataKind,
    rest: &'a str,
    budget: &mut Budget,
) -> Result<Metadata<'a>, ParseError> {
    let (class, category) = rest.split_once("_$_").ok_or(ParseError::Invalid)?;
    if category.contains("_$_") {
        return Err(ParseError::Invalid);
    }
    Ok(Metadata {
        kind,
        subject: Some(validate_name(class, budget)?),
        category: Some(validate_name(category, budget)?),
        member: None,
    })
}

/// Build a record that may carry a category (`PROP_LIST_Class_$_Category`).
fn optional_category<'a>(
    kind: MetadataKind,
    rest: &'a str,
    budget: &mut Budget,
) -> Result<Metadata<'a>, ParseError> {
    if let Some((class, category)) = rest.split_once("_$_") {
        if category.contains("_$_") {
            return Err(ParseError::Invalid);
        }
        return Ok(Metadata {
            kind,
            subject: Some(validate_name(class, budget)?),
            category: Some(validate_name(category, budget)?),
            member: None,
        });
    }
    single(kind, rest, budget)
}

/// Build a member record from `Class.member`.
fn member<'a>(
    kind: MetadataKind,
    rest: &'a str,
    budget: &mut Budget,
) -> Result<Metadata<'a>, ParseError> {
    let (class, member) = rest.rsplit_once('.').ok_or(ParseError::Invalid)?;
    Ok(Metadata {
        kind,
        subject: Some(validate_name(class, budget)?),
        category: None,
        member: Some(validate_name(member, budget)?),
    })
}

/// Build an indexed method-name/type record.
fn index<'a>(
    kind: MetadataKind,
    index: &'a str,
    budget: &mut Budget,
) -> Result<Metadata<'a>, ParseError> {
    if index.is_empty() || !index.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(ParseError::Invalid);
    }
    budget.spend(index.len())?;
    Ok(Metadata {
        kind,
        subject: Some(index),
        category: None,
        member: None,
    })
}

/// Validate a class, category or protocol name and return it verbatim.
fn validate_name<'a>(name: &'a str, budget: &mut Budget) -> Result<&'a str, ParseError> {
    if name.is_empty() {
        return Err(ParseError::Invalid);
    }
    for byte in name.bytes() {
        budget.spend(1)?;
        if !is_name_byte(byte) {
            return Err(ParseError::Invalid);
        }
    }
    Ok(name)
}

/// Bytes allowed in metadata names.
///
/// Underscores and `$` are ordinary name bytes; the parser never treats them
/// as separators except for the documented `_$_` category separator.
const fn is_name_byte(byte: u8) -> bool {
    byte.is_ascii_graphic()
        && !matches!(
            byte,
            b'[' | b']' | b'(' | b')' | b':' | b';' | b',' | b'"' | b'\'' | b'\\' | b'`'
        )
}
