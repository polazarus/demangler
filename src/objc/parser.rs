//! Bounded parser for Objective-C linker metadata and method display forms.
//!
//! Objective-C metadata is selected by a known, case-sensitive prefix; the
//! class, category and protocol names after it are kept verbatim and are never
//! split on `_`. Category records use the runtime's `_$_` separator, which is
//! the only structural separator this parser relies on. GNUstep ABI 2 names
//! carry an object-format decoration in front of `_OBJC_`, which is removed
//! before the namespace is matched.
//!
//! The grammar is matched on bytes: a symbol name is a byte string, so the
//! parser never requires the whole input to be UTF-8. Text is interpreted only
//! when it is printed (see [`super::text`]).

use super::node::{
    Ast, BlockDescriptor, Item, Metadata, MetadataKind, Method, MethodKind, ParseLimits,
};
use crate::parse::{Cursor, State, find, rfind};
use crate::{BudgetKind, ParseError, ParseErrorKind};

/// Apple/Mach-O and GNUstep metadata namespaces, longest first.
const OBJC_PREFIXES: [&[u8]; 4] = [b"__OBJC_", b"_OBJC_", b"OBJC_", b"__objc_"];

/// Block descriptor prefixes, longest first.
const BLOCK_PREFIXES: [&[u8]; 2] = [b"___block_descriptor_", b"__block_descriptor_"];

/// Parse an Objective-C symbol name from raw bytes within `limits`.
pub fn parse_bytes(symbol: &[u8], limits: ParseLimits) -> Result<Ast<'_>, ParseError> {
    if symbol.is_empty() {
        return Err(ParseError::new(ParseErrorKind::Invalid));
    }
    let mut state = State::new(limits.cursor());
    let mut cursor = Cursor::new(symbol, 0, &mut state)?;
    charge(&mut cursor, symbol.len())?;
    check_brackets(&mut cursor)?;

    if symbol.starts_with(b"-") || symbol.starts_with(b"+") {
        return Ok(Ast {
            item: Item::Method(parse_method(symbol, &mut cursor)?),
        });
    }
    if BLOCK_PREFIXES
        .iter()
        .any(|prefix| symbol.starts_with(prefix))
    {
        return Ok(Ast {
            item: Item::BlockDescriptor(parse_block_descriptor(symbol, &mut cursor)?),
        });
    }
    Ok(Ast {
        item: Item::Metadata(parse_metadata(symbol, &mut cursor)?),
    })
}

/// Charge `units` of work against the shared budget without moving the cursor.
///
/// The shared cursor is the only work meter the combinator layer exposes, so a
/// scan that works on a slice charges by advancing over it and rewinding.
fn charge(cursor: &mut Cursor<'_, '_>, units: usize) -> Result<(), ParseError> {
    let checkpoint = cursor.checkpoint();
    let _ = cursor.advance(units)?;
    cursor.reset(checkpoint);
    Ok(())
}

/// Reject unbalanced or too-deep brackets before any family-specific scan.
fn check_brackets(cursor: &mut Cursor<'_, '_>) -> Result<(), ParseError> {
    let checkpoint = cursor.checkpoint();
    let result = scan_brackets(cursor);
    cursor.reset(checkpoint);
    result
}

/// Scan the whole symbol for bracket balance.
fn scan_brackets(cursor: &mut Cursor<'_, '_>) -> Result<(), ParseError> {
    let depth_limit = usize::from(cursor.depth_limit());
    let mut depth = 0_usize;
    while let Some(byte) = cursor.peek() {
        // Check before consuming, so an error names the offending bracket.
        match byte {
            b'[' => {
                depth += 1;
                if depth > depth_limit {
                    return Err(cursor.budget(BudgetKind::ParseDepth));
                }
            }
            b']' => depth = depth.checked_sub(1).ok_or_else(|| cursor.invalid())?,
            _ => {}
        }
        let _ = cursor.advance(1)?;
    }
    if depth == 0 {
        Ok(())
    } else {
        Err(cursor.error(ParseErrorKind::UnexpectedEnd))
    }
}

/// Parse a `-[Class selector:]` or `+[Class selector:]` display form.
fn parse_method<'a>(
    symbol: &'a [u8],
    cursor: &mut Cursor<'_, '_>,
) -> Result<Method<'a>, ParseError> {
    let kind = match symbol.first() {
        Some(b'-') => MethodKind::Instance,
        Some(b'+') => MethodKind::Class,
        _ => return Err(cursor.invalid_in(symbol, 0)),
    };
    let rest = symbol
        .get(1..)
        .ok_or_else(|| cursor.invalid_in(symbol, 1))?;
    let rest = rest
        .strip_prefix(b"[")
        .ok_or_else(|| cursor.invalid_in(symbol, 1))?;
    let rest = rest
        .strip_suffix(b"]")
        .ok_or_else(|| cursor.invalid_in(symbol, symbol.len().saturating_sub(1)))?;

    // Exactly one space separates the receiver spelling from the selector.
    let (receiver, selector) =
        split_once(rest, b" ").ok_or_else(|| cursor.invalid_in(rest, rest.len()))?;
    if selector.is_empty() || selector.contains(&b' ') {
        return Err(cursor.invalid_in(selector, 0));
    }

    let (class, category) = parse_receiver(receiver, cursor)?;
    let selector = validate_selector(selector, cursor)?;
    Ok(Method {
        kind,
        class,
        category,
        selector,
    })
}

/// Parse `Class` or `Class(Category)` from a method display form.
fn parse_receiver<'a>(
    receiver: &'a [u8],
    cursor: &mut Cursor<'_, '_>,
) -> Result<(&'a [u8], Option<&'a [u8]>), ParseError> {
    let (class, category) = if let Some((class, rest)) = split_once(receiver, b"(") {
        let category = rest
            .strip_suffix(b")")
            .ok_or_else(|| cursor.invalid_in(rest, rest.len()))?;
        if category.contains(&b'(') || category.contains(&b')') {
            return Err(cursor.invalid_in(category, 0));
        }
        (class, Some(category))
    } else {
        (receiver, None)
    };
    let class = validate_name(class, cursor)?;
    let category = match category {
        Some(category) => Some(validate_name(category, cursor)?),
        None => None,
    };
    Ok((class, category))
}

/// Validate a selector, preserving its colons.
fn validate_selector<'a>(
    selector: &'a [u8],
    cursor: &mut Cursor<'_, '_>,
) -> Result<&'a [u8], ParseError> {
    if let Some(body) = selector.strip_suffix(b":") {
        if body.is_empty() {
            return Err(cursor.invalid_in(selector, 0));
        }
        for part in body.split(|byte| *byte == b':') {
            if part.is_empty() {
                return Err(cursor.invalid_in(part, 0));
            }
            let _ = validate_name(part, cursor)?;
        }
        Ok(selector)
    } else if selector.contains(&b':') {
        Err(cursor.invalid_in(selector, 0))
    } else {
        validate_name(selector, cursor)
    }
}

/// Parse a block descriptor symbol.
fn parse_block_descriptor<'a>(
    symbol: &'a [u8],
    cursor: &mut Cursor<'_, '_>,
) -> Result<BlockDescriptor<'a>, ParseError> {
    let payload = BLOCK_PREFIXES
        .iter()
        .find_map(|prefix| symbol.strip_prefix(*prefix))
        .ok_or_else(|| cursor.invalid_in(symbol, 0))?;
    if payload.is_empty() {
        return Err(cursor.invalid_in(payload, 0));
    }
    for (index, byte) in payload.iter().enumerate() {
        charge(cursor, 1)?;
        // Clang encodes the block signature with a `\x01` separator, so the
        // payload is not restricted to graphic ASCII. Other control bytes and
        // DEL are rejected so a hostile symbol cannot move the terminal cursor
        // or rewrite the output line.
        if !(*byte == 0x01 || (0x20..=0x7e).contains(byte)) {
            return Err(cursor.invalid_in(payload, index));
        }
    }
    Ok(BlockDescriptor { payload })
}

/// Parse a runtime metadata name.
fn parse_metadata<'a>(
    symbol: &'a [u8],
    cursor: &mut Cursor<'_, '_>,
) -> Result<Metadata<'a>, ParseError> {
    // GNUstep ABI 2 metadata is decorated by the object format, so the
    // decoration is removed before the namespace is matched.
    let symbol = strip_abi2_decoration(symbol);
    // Apple/Mach-O accepts one or two leading underscores and the bare
    // spelling. GNUstep's classic runtime always uses `__objc_`.
    for prefix in OBJC_PREFIXES {
        if let Some(rest) = symbol.strip_prefix(prefix) {
            return if prefix == b"__objc_" {
                parse_gnustep(rest, cursor)
            } else {
                parse_apple(rest, cursor)
            };
        }
    }
    Err(cursor.invalid_in(symbol, 0))
}

/// Object-format decorations GNUstep ABI 2 puts in front of `_OBJC_`, longest
/// first.
///
/// ELF emits `._OBJC_CLASS_Foo`, COFF emits `$_OBJC_CLASS_Foo`, and the i686
/// COFF C decoration adds one underscore, giving `_$_OBJC_CLASS_Foo`.
const ABI2_DECORATIONS: [&[u8]; 3] = [b"_$", b"$", b"."];

/// Strip a GNUstep ABI 2 object-format decoration.
///
/// Only the `_OBJC_` namespace is decorated: the decoration is kept when the
/// rest of the name is not that namespace, so local `.objc_*` records and
/// `l_OBJC_*` labels stay unclaimed.
pub(super) fn strip_abi2_decoration(symbol: &[u8]) -> &[u8] {
    for decoration in ABI2_DECORATIONS {
        if let Some(rest) = symbol.strip_prefix(decoration)
            && rest.starts_with(b"_OBJC_")
        {
            return rest;
        }
    }
    symbol
}

/// Parse the part after `_OBJC_` / `__OBJC_` / `OBJC_`.
fn parse_apple<'a>(
    rest: &'a [u8],
    cursor: &mut Cursor<'_, '_>,
) -> Result<Metadata<'a>, ParseError> {
    // Apple's `$`-separated metadata first, so `CLASS_$_Foo` is not read as a
    // GNUstep `CLASS_Foo`. Longer prefixes come before their prefixes.
    if let Some(name) = rest.strip_prefix(b"CLASS_PROTOCOLS_$_") {
        return optional_category_group(MetadataKind::ClassProtocols, name, cursor);
    }
    if let Some(name) = rest.strip_prefix(b"CATEGORY_PROTOCOLS_$_") {
        return category(MetadataKind::CategoryProtocols, name, cursor);
    }
    for (prefix, kind) in APPLE_PREFIXES {
        if let Some(name) = rest.strip_prefix(prefix) {
            return single(kind, name, cursor);
        }
    }
    if let Some(name) = rest.strip_prefix(b"IVAR_$_") {
        return member(MetadataKind::Ivar, name, cursor);
    }
    if let Some(name) = rest.strip_prefix(b"$_") {
        return parse_apple_dollar(name, cursor);
    }

    // GNUstep 2.0 uses the same `_OBJC_` namespace without `$`.
    for (prefix, kind) in GNUSTEP_2_PREFIXES {
        if let Some(name) = rest.strip_prefix(prefix) {
            return single(kind, name, cursor);
        }
    }

    // Old GNU/NeXT runtime metadata.
    for (prefix, kind) in NEIGHSTEP_PREFIXES {
        if let Some(name) = rest.strip_prefix(prefix) {
            return single(kind, name, cursor);
        }
    }
    if let Some(name) = rest.strip_prefix(b"METH_VAR_NAME_") {
        return index(MetadataKind::MethodName, name, cursor);
    }
    if let Some(name) = rest.strip_prefix(b"METH_VAR_TYPE_") {
        return index(MetadataKind::MethodType, name, cursor);
    }
    if rest == b"Module" {
        return Ok(no_subject(MetadataKind::Module));
    }
    if rest == b"SYMBOLS" {
        return Ok(no_subject(MetadataKind::Symbols));
    }
    if rest == b"SELECTOR_TABLE" {
        return Ok(no_subject(MetadataKind::SelectorTable));
    }
    if let Some(name) = rest.strip_prefix(b"SELECTOR_REFERENCES_") {
        if name.is_empty() {
            return Ok(no_subject(MetadataKind::SelectorReferences));
        }
        return single(MetadataKind::SelectorReferences, name, cursor);
    }

    Err(cursor.invalid_in(rest, 0))
}

/// Apple `$`-separated metadata prefixes that carry a single name.
///
/// The two protocol-list prefixes are handled before this table because they
/// take a category group, not a plain name.
const APPLE_PREFIXES: [(&[u8], MetadataKind); 8] = [
    (b"CLASS_RO_$_", MetadataKind::ClassData),
    (b"METACLASS_RO_$_", MetadataKind::MetaclassData),
    (b"LABEL_PROTOCOL_$_", MetadataKind::ProtocolLabel),
    (b"PROTOCOL_REFERENCE_$_", MetadataKind::ProtocolRef),
    (b"PROTOCOL_$_", MetadataKind::Protocol),
    (b"EHTYPE_$_", MetadataKind::ExceptionType),
    (b"CLASS_$_", MetadataKind::Class),
    (b"METACLASS_$_", MetadataKind::Metaclass),
];

/// GNUstep 2.0 `_OBJC_` metadata prefixes and their kinds, longest first.
const GNUSTEP_2_PREFIXES: [(&[u8], MetadataKind); 5] = [
    (b"REF_CLASS_", MetadataKind::ClassRef),
    (b"INIT_CLASS_", MetadataKind::ClassInit),
    (b"CLASS_", MetadataKind::Class),
    (b"METACLASS_", MetadataKind::Metaclass),
    (b"PROTOCOL_", MetadataKind::Protocol),
];

/// Old GNU/NeXT runtime metadata prefixes and their kinds, longest first.
const NEIGHSTEP_PREFIXES: [(&[u8], MetadataKind); 7] = [
    (b"ClassName_", MetadataKind::ClassName),
    (b"Class_", MetadataKind::Class),
    (b"MetaClass_", MetadataKind::Metaclass),
    (b"InstanceMethods_", MetadataKind::InstanceMethods),
    (b"ClassMethods_", MetadataKind::ClassMethods),
    (b"InstanceIvars_", MetadataKind::InstanceVariables),
    (b"ClassIvars_", MetadataKind::ClassVariables),
];

/// Parse the part after Apple's `__OBJC_$_` / `_OBJC_$_`.
fn parse_apple_dollar<'a>(
    rest: &'a [u8],
    cursor: &mut Cursor<'_, '_>,
) -> Result<Metadata<'a>, ParseError> {
    if let Some(name) = rest.strip_prefix(b"CATEGORY_INSTANCE_METHODS_") {
        return category(MetadataKind::CategoryInstanceMethods, name, cursor);
    }
    if let Some(name) = rest.strip_prefix(b"CATEGORY_CLASS_METHODS_") {
        return category(MetadataKind::CategoryClassMethods, name, cursor);
    }
    if let Some(name) = rest.strip_prefix(b"CATEGORY_PROP_LIST_") {
        return category(MetadataKind::CategoryPropertyList, name, cursor);
    }
    if let Some(name) = rest.strip_prefix(b"CATEGORY_PROPERTIES_") {
        return category(MetadataKind::CategoryPropertyList, name, cursor);
    }
    if let Some(name) = rest.strip_prefix(b"CATEGORY_") {
        return category(MetadataKind::Category, name, cursor);
    }
    for (prefix, kind) in GROUP_PREFIXES {
        if let Some(name) = rest.strip_prefix(prefix) {
            return optional_category_group(kind, name, cursor);
        }
    }
    for (prefix, kind) in PROTOCOL_PREFIXES {
        if let Some(name) = rest.strip_prefix(prefix) {
            return single(kind, name, cursor);
        }
    }
    Err(cursor.invalid_in(rest, 0))
}

/// Apple metadata prefixes for lists that may name several categories.
const GROUP_PREFIXES: [(&[u8], MetadataKind); 6] = [
    (b"INSTANCE_METHODS_", MetadataKind::InstanceMethods),
    (b"CLASS_METHODS_", MetadataKind::ClassMethods),
    (b"INSTANCE_VARIABLES_", MetadataKind::InstanceVariables),
    (b"CLASS_PROP_LIST_", MetadataKind::ClassPropertyList),
    (b"CLASS_PROPERTIES_", MetadataKind::ClassPropertyList),
    (b"PROP_LIST_", MetadataKind::PropertyList),
];

/// Apple protocol metadata prefixes, longest first.
const PROTOCOL_PREFIXES: [(&[u8], MetadataKind); 10] = [
    (
        b"PROTOCOL_INSTANCE_METHODS_OPT_",
        MetadataKind::ProtocolOptionalInstanceMethods,
    ),
    (
        b"PROTOCOL_CLASS_METHODS_OPT_",
        MetadataKind::ProtocolOptionalClassMethods,
    ),
    (
        b"PROTOCOL_OPTIONAL_INSTANCE_METHODS_",
        MetadataKind::ProtocolOptionalInstanceMethods,
    ),
    (
        b"PROTOCOL_OPTIONAL_CLASS_METHODS_",
        MetadataKind::ProtocolOptionalClassMethods,
    ),
    (
        b"PROTOCOL_INSTANCE_METHODS_",
        MetadataKind::ProtocolInstanceMethods,
    ),
    (
        b"PROTOCOL_CLASS_METHODS_",
        MetadataKind::ProtocolClassMethods,
    ),
    (b"PROTOCOL_PROPERTIES_", MetadataKind::ProtocolPropertyList),
    (b"PROTOCOL_PROP_LIST_", MetadataKind::ProtocolPropertyList),
    (b"PROTOCOL_REFS_", MetadataKind::ProtocolRefs),
    (b"PROTOCOL_METHOD_TYPES_", MetadataKind::ProtocolMethodTypes),
];

/// Parse the part after GNUstep's `__objc_`.
fn parse_gnustep<'a>(
    rest: &'a [u8],
    cursor: &mut Cursor<'_, '_>,
) -> Result<Metadata<'a>, ParseError> {
    if let Some(name) = rest.strip_prefix(b"class_name_") {
        return single(MetadataKind::Class, name, cursor);
    }
    if let Some(name) = rest.strip_prefix(b"class_ref_") {
        return single(MetadataKind::ClassRef, name, cursor);
    }
    if let Some(name) = rest.strip_prefix(b"ivar_offset_value_") {
        // GNUstep 2.0 emits an optional trailing `.` after the ivar selector.
        return member(
            MetadataKind::IvarOffsetValue,
            trim_selector_end(name),
            cursor,
        );
    }
    if let Some(name) = rest.strip_prefix(b"ivar_offset_") {
        return member(MetadataKind::IvarOffset, trim_selector_end(name), cursor);
    }
    // `__objc_category_name_` separates class and category with `_`, which is
    // also legal inside both names, so it cannot be split unambiguously.
    Err(cursor.invalid_in(rest, 0))
}

/// Drop the optional `.` or `\x01` that terminates a GNUstep ivar selector.
fn trim_selector_end(name: &[u8]) -> &[u8] {
    let mut end = name.len();
    while end > 0 && matches!(name[end - 1], b'.' | 0x01) {
        end -= 1;
    }
    &name[..end]
}

/// Build a record with only a subject.
fn single<'a>(
    kind: MetadataKind,
    name: &'a [u8],
    cursor: &mut Cursor<'_, '_>,
) -> Result<Metadata<'a>, ParseError> {
    Ok(Metadata {
        kind,
        subject: Some(validate_name(name, cursor)?),
        category: None,
        member: None,
    })
}

/// Build a record with no subject.
const fn no_subject(kind: MetadataKind) -> Metadata<'static> {
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
    rest: &'a [u8],
    cursor: &mut Cursor<'_, '_>,
) -> Result<Metadata<'a>, ParseError> {
    let (class, category) = split_once(rest, b"_$_").ok_or_else(|| cursor.invalid_in(rest, 0))?;
    if contains(category, b"_$_") {
        return Err(cursor.invalid_in(category, 0));
    }
    Ok(Metadata {
        kind,
        subject: Some(validate_name(class, cursor)?),
        category: Some(validate_name(category, cursor)?),
        member: None,
    })
}

/// Build a method/property/ivar list that may name several categories as
/// `Class(Cat1|Cat2)` (LLVM combines category metadata this way).
fn optional_category_group<'a>(
    kind: MetadataKind,
    rest: &'a [u8],
    cursor: &mut Cursor<'_, '_>,
) -> Result<Metadata<'a>, ParseError> {
    if let Some((class, category)) = split_once(rest, b"_$_") {
        if contains(category, b"_$_") {
            return Err(cursor.invalid_in(category, 0));
        }
        return Ok(Metadata {
            kind,
            subject: Some(validate_name(class, cursor)?),
            category: Some(validate_name(category, cursor)?),
            member: None,
        });
    }
    if let Some((class, group)) = split_once(rest, b"(") {
        let group = group
            .strip_suffix(b")")
            .ok_or_else(|| cursor.invalid_in(group, group.len()))?;
        if group.is_empty() || group.iter().any(|byte| matches!(byte, b'(' | b')')) {
            return Err(cursor.invalid_in(group, 0));
        }
        for part in group.split(|byte| *byte == b'|') {
            let _ = validate_name(part, cursor)?;
        }
        return Ok(Metadata {
            kind,
            subject: Some(validate_name(class, cursor)?),
            // The group keeps its `|` separators; each part was validated above.
            category: Some(group),
            member: None,
        });
    }
    single(kind, rest, cursor)
}

/// Build a member record from `Class.member`.
fn member<'a>(
    kind: MetadataKind,
    rest: &'a [u8],
    cursor: &mut Cursor<'_, '_>,
) -> Result<Metadata<'a>, ParseError> {
    let (class, member) = rsplit_once(rest, b".").ok_or_else(|| cursor.invalid_in(rest, 0))?;
    Ok(Metadata {
        kind,
        subject: Some(validate_name(class, cursor)?),
        category: None,
        member: Some(validate_name(member, cursor)?),
    })
}

/// Build an indexed method-name/type record.
fn index<'a>(
    kind: MetadataKind,
    index: &'a [u8],
    cursor: &mut Cursor<'_, '_>,
) -> Result<Metadata<'a>, ParseError> {
    if index.is_empty() || !index.iter().all(u8::is_ascii_digit) {
        return Err(cursor.invalid_in(index, 0));
    }
    charge(cursor, index.len())?;
    Ok(Metadata {
        kind,
        subject: Some(index),
        category: None,
        member: None,
    })
}

/// Validate a class, category, protocol, ivar or selector-part name and
/// return it verbatim.
fn validate_name<'a>(name: &'a [u8], cursor: &mut Cursor<'_, '_>) -> Result<&'a [u8], ParseError> {
    if name.is_empty() {
        return Err(cursor.invalid_in(name, 0));
    }
    for (index, byte) in name.iter().enumerate() {
        charge(cursor, 1)?;
        if !is_name_byte(*byte) {
            return Err(cursor.invalid_in(name, index));
        }
    }
    Ok(name)
}

/// Bytes allowed in names.
///
/// Class, category, protocol, ivar and selector-part names are C identifiers
/// with `_`, `$` and the `.` that separates an ivar from its class. Parentheses
/// and `|` are handled by the category-group parser and the colons of a
/// selector by [`validate_selector`], not by this predicate. Bytes from `0x80`
/// up are part of a UTF-8 sequence (or a broken one, which prints escaped).
const fn is_name_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'$' | b'.') || byte >= 0x80
}

/// Split `haystack` at the first occurrence of `needle`.
fn split_once<'a>(haystack: &'a [u8], needle: &[u8]) -> Option<(&'a [u8], &'a [u8])> {
    let index = find(haystack, needle)?;
    Some((&haystack[..index], &haystack[index + needle.len()..]))
}

/// Split `haystack` at the last occurrence of `needle`.
fn rsplit_once<'a>(haystack: &'a [u8], needle: &[u8]) -> Option<(&'a [u8], &'a [u8])> {
    let index = rfind(haystack, needle)?;
    Some((&haystack[..index], &haystack[index + needle.len()..]))
}

/// True when `haystack` contains `needle`.
fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    find(haystack, needle).is_some()
}

#[cfg(test)]
mod tests;
