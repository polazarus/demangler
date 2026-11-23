//! Canonical printer for D symbols.
//!
//! The spelling follows `c++filt -s dlang`: primitive and aggregate types,
//! function and delegate signatures, template arguments and compiler
//! specials. It writes straight into the formatter and allocates nothing.

use core::fmt;

use super::node::{
    Attribute, CharWidth, Component, Convention, FunctionSig, FunctionType, HexFloat, IntegerKind,
    IntegerSuffix, Modifier, NameExpr, Parameter, ParameterKind, ParameterPrefix, Primitive, Root,
    StringWidth, Symbol, TemplateArg, Type, Value, Variadic,
};
use crate::parse::{Escape, write_escaped};

/// Write a parsed symbol.
pub(crate) fn print<W: fmt::Write + ?Sized>(root: &Root<'_>, out: &mut W) -> fmt::Result {
    match root {
        Root::Main => out.write_str("D main"),
        Root::Symbol(symbol) => print_symbol(symbol, out, true),
    }
}

/// Write a qualified name.
fn print_symbol<W: fmt::Write + ?Sized>(
    symbol: &Symbol<'_>,
    out: &mut W,
    suffix_modifiers: bool,
) -> fmt::Result {
    if let Some((last, parents)) = symbol.components.split_last()
        && let Some(prefix) = prepend_prefix(&last.name)
    {
        if parents.is_empty() {
            // `c++filt` removes the trailing separator; with no parent that
            // separator is the space in `... for `.
            return out.write_str(prefix.trim_end());
        }
        out.write_str(prefix)?;
        for (index, component) in parents.iter().enumerate() {
            if index > 0 {
                out.write_char('.')?;
            }
            print_component(component, out, suffix_modifiers)?;
        }
        return Ok(());
    }

    for (index, component) in symbol.components.iter().enumerate() {
        if index > 0 {
            out.write_char('.')?;
        }
        print_component(component, out, suffix_modifiers)?;
    }
    Ok(())
}

/// The prefix a compiler special prepends to the whole qualified name.
fn prepend_prefix(name: &NameExpr<'_>) -> Option<&'static str> {
    match name {
        NameExpr::Initializer => Some("initializer for "),
        NameExpr::Vtable => Some("vtable for "),
        NameExpr::ClassInfo => Some("ClassInfo for "),
        NameExpr::Interface => Some("Interface for "),
        NameExpr::ModuleInfo => Some("ModuleInfo for "),
        _ => None,
    }
}

fn print_component<W: fmt::Write + ?Sized>(
    component: &Component<'_>,
    out: &mut W,
    suffix_modifiers: bool,
) -> fmt::Result {
    print_name(&component.name, out)?;
    if let Some(function) = &component.function {
        print_function_sig(function, out, suffix_modifiers)?;
    }
    Ok(())
}

fn print_name<W: fmt::Write + ?Sized>(name: &NameExpr<'_>, out: &mut W) -> fmt::Result {
    match name {
        NameExpr::Plain(bytes) => write_escaped(out, bytes, Escape::Identifier),
        NameExpr::This => out.write_str("this"),
        NameExpr::Destructor => out.write_str("~this"),
        NameExpr::Postblit => out.write_str("this(this)"),
        // These are handled at symbol level; a nested occurrence has no
        // canonical spelling.
        NameExpr::Initializer
        | NameExpr::Vtable
        | NameExpr::ClassInfo
        | NameExpr::Interface
        | NameExpr::ModuleInfo => Ok(()),
        NameExpr::Template { name, args } => {
            print_name(name, out)?;
            out.write_str("!(")?;
            for (index, arg) in args.iter().enumerate() {
                if index > 0 {
                    out.write_str(", ")?;
                }
                print_template_arg(arg, out)?;
            }
            out.write_char(')')
        }
    }
}

fn print_function_sig<W: fmt::Write + ?Sized>(
    function: &FunctionSig<'_>,
    out: &mut W,
    suffix_modifiers: bool,
) -> fmt::Result {
    print_signature(&function.params, function.variadic, out)?;
    if suffix_modifiers {
        print_modifiers(&function.modifiers, out)?;
    }
    Ok(())
}

fn print_template_arg<W: fmt::Write + ?Sized>(arg: &TemplateArg<'_>, out: &mut W) -> fmt::Result {
    match arg {
        TemplateArg::Type(ty) => print_type(ty, out),
        TemplateArg::Value { value } => print_value(value, out),
        TemplateArg::Symbol(symbol) => print_symbol(symbol, out, false),
        TemplateArg::External(bytes) => write_escaped(out, bytes, Escape::Identifier),
    }
}

fn print_type<W: fmt::Write + ?Sized>(ty: &Type<'_>, out: &mut W) -> fmt::Result {
    match ty {
        Type::Primitive(primitive) => out.write_str(primitive_name(*primitive)),
        Type::Shared(inner) => print_wrapped("shared", inner, out),
        Type::Const(inner) => print_wrapped("const", inner, out),
        Type::Immutable(inner) => print_wrapped("immutable", inner, out),
        Type::Inout(inner) => print_wrapped("inout", inner, out),
        Type::Vector(inner) => print_wrapped("__vector", inner, out),
        Type::TypeofNull => out.write_str("typeof(null)"),
        Type::TypeofStarNull => out.write_str("typeof(*null)"),
        Type::DynamicArray(element) => {
            print_type(element, out)?;
            out.write_str("[]")
        }
        Type::StaticArray { element, length } => {
            print_type(element, out)?;
            out.write_char('[')?;
            write_escaped(out, length, Escape::Identifier)?;
            out.write_char(']')
        }
        Type::AssocArray { key, value } => {
            print_type(value, out)?;
            out.write_char('[')?;
            print_type(key, out)?;
            out.write_char(']')
        }
        Type::Pointer(inner) => {
            print_type(inner, out)?;
            out.write_char('*')
        }
        Type::Function(function) => {
            print_function_type(function, out)?;
            out.write_str("function")
        }
        Type::Delegate {
            function,
            modifiers,
        } => {
            print_function_type(function, out)?;
            out.write_str("delegate")?;
            print_modifiers(modifiers, out)
        }
        Type::Named(symbol) => print_symbol(symbol, out, false),
        Type::Tuple(types) => {
            out.write_str("Tuple!(")?;
            for (index, ty) in types.iter().enumerate() {
                if index > 0 {
                    out.write_str(", ")?;
                }
                print_type(ty, out)?;
            }
            out.write_char(')')
        }
    }
}

fn print_wrapped<W: fmt::Write + ?Sized>(name: &str, inner: &Type<'_>, out: &mut W) -> fmt::Result {
    out.write_str(name)?;
    out.write_char('(')?;
    print_type(inner, out)?;
    out.write_char(')')
}

fn print_function_type<W: fmt::Write + ?Sized>(
    function: &FunctionType<'_>,
    out: &mut W,
) -> fmt::Result {
    if let Some(prefix) = convention_prefix(function.convention) {
        out.write_str(prefix)?;
    }
    print_type(&function.ret, out)?;
    print_signature(&function.params, function.variadic, out)?;
    out.write_char(' ')?;
    for attr in &function.attrs {
        out.write_str(attribute_name(*attr))?;
        out.write_char(' ')?;
    }
    Ok(())
}

fn convention_prefix(convention: Convention) -> Option<&'static str> {
    match convention {
        Convention::D => None,
        Convention::C => Some("extern(C) "),
        Convention::Windows => Some("extern(Windows) "),
        Convention::Pascal => Some("extern(Pascal) "),
        Convention::Cpp => Some("extern(C++) "),
        Convention::ObjectiveC => Some("extern(Objective-C) "),
    }
}

fn print_signature<W: fmt::Write + ?Sized>(
    params: &[Parameter<'_>],
    variadic: Variadic,
    out: &mut W,
) -> fmt::Result {
    out.write_char('(')?;
    for (index, param) in params.iter().enumerate() {
        if index > 0 {
            out.write_str(", ")?;
        }
        print_parameter(param, out)?;
    }
    print_variadic(variadic, !params.is_empty(), out)?;
    out.write_char(')')
}

fn print_parameter<W: fmt::Write + ?Sized>(param: &Parameter<'_>, out: &mut W) -> fmt::Result {
    out.write_str(match param.prefix {
        ParameterPrefix::Plain => "",
        ParameterPrefix::Scope => "scope ",
        ParameterPrefix::Return => "return ",
        ParameterPrefix::ScopeReturn => "scope return ",
        ParameterPrefix::ReturnScope => "return scope ",
    })?;
    out.write_str(match param.kind {
        ParameterKind::Plain => "",
        ParameterKind::In => "in ",
        ParameterKind::InRef => "in ref ",
        ParameterKind::Out => "out ",
        ParameterKind::Ref => "ref ",
        ParameterKind::Lazy => "lazy ",
    })?;
    print_type(&param.ty, out)
}

fn print_variadic<W: fmt::Write + ?Sized>(
    variadic: Variadic,
    has_params: bool,
    out: &mut W,
) -> fmt::Result {
    match variadic {
        Variadic::None => Ok(()),
        Variadic::Typesafe => out.write_str("..."),
        Variadic::DStyle => {
            if has_params {
                out.write_str(", ")?;
            }
            out.write_str("...")
        }
    }
}

fn print_modifiers<W: fmt::Write + ?Sized>(modifiers: &[Modifier], out: &mut W) -> fmt::Result {
    for modifier in modifiers {
        out.write_str(match modifier {
            Modifier::Const => " const",
            Modifier::Immutable => " immutable",
            Modifier::Shared => " shared",
            Modifier::Inout => " inout",
        })?;
    }
    Ok(())
}

fn print_value<W: fmt::Write + ?Sized>(value: &Value<'_>, out: &mut W) -> fmt::Result {
    match value {
        Value::Null => out.write_str("null"),
        Value::Integer { negative, kind } => {
            if *negative {
                out.write_char('-')?;
            }
            print_integer(kind, out)
        }
        Value::Real(hex) => print_hex_float(hex, out),
        Value::Complex { re, im } => {
            print_hex_float(re, out)?;
            out.write_char('+')?;
            print_hex_float(im, out)?;
            out.write_char('i')
        }
        Value::String { width, hex } => print_string(*width, hex, out),
        Value::Array(values) => {
            out.write_char('[')?;
            for (index, value) in values.iter().enumerate() {
                if index > 0 {
                    out.write_str(", ")?;
                }
                print_value(value, out)?;
            }
            out.write_char(']')
        }
        Value::AssocArray(pairs) => {
            out.write_char('[')?;
            for (index, (key, value)) in pairs.iter().enumerate() {
                if index > 0 {
                    out.write_str(", ")?;
                }
                print_value(key, out)?;
                out.write_char(':')?;
                print_value(value, out)?;
            }
            out.write_char(']')
        }
        Value::Struct { name, fields } => {
            if let Some(name) = name {
                print_type(name, out)?;
            }
            out.write_char('(')?;
            for (index, field) in fields.iter().enumerate() {
                if index > 0 {
                    out.write_str(", ")?;
                }
                print_value(field, out)?;
            }
            out.write_char(')')
        }
        Value::Symbol(symbol) => print_symbol(symbol, out, true),
    }
}

fn print_integer<W: fmt::Write + ?Sized>(kind: &IntegerKind<'_>, out: &mut W) -> fmt::Result {
    match kind {
        IntegerKind::Char(width, value) => {
            out.write_char('\'')?;
            if *width == CharWidth::Utf8 && (0x20..0x7f).contains(value) {
                let byte = u8::try_from(*value).unwrap_or(b'?');
                out.write_char(char::from(byte))?;
            } else {
                let (prefix, digits) = match width {
                    CharWidth::Utf8 => ("\\x", 2),
                    CharWidth::Utf16 => ("\\u", 4),
                    CharWidth::Utf32 => ("\\U", 8),
                };
                out.write_str(prefix)?;
                write_hex_padded(*value, digits, out)?;
            }
            out.write_char('\'')
        }
        IntegerKind::Bool(value) => out.write_str(if *value { "true" } else { "false" }),
        IntegerKind::Number { digits, suffix } => {
            write_escaped(out, digits, Escape::Identifier)?;
            out.write_str(match suffix {
                IntegerSuffix::None => "",
                IntegerSuffix::U => "u",
                IntegerSuffix::L => "L",
                IntegerSuffix::UL => "uL",
            })
        }
    }
}

fn write_hex_padded<W: fmt::Write + ?Sized>(value: u64, width: usize, out: &mut W) -> fmt::Result {
    let mut buffer = [0_u8; 16];
    let mut index = buffer.len();
    let mut remaining = value;
    loop {
        let digit = u8::try_from(remaining % 16).unwrap_or(0);
        index -= 1;
        buffer[index] = if digit < 10 {
            b'0' + digit
        } else {
            b'a' + (digit - 10)
        };
        remaining /= 16;
        if remaining == 0 {
            break;
        }
    }
    while buffer.len() - index < width {
        index -= 1;
        buffer[index] = b'0';
    }
    let text = core::str::from_utf8(&buffer[index..]).map_err(|_| fmt::Error)?;
    out.write_str(text)
}

fn print_hex_float<W: fmt::Write + ?Sized>(hex: &HexFloat<'_>, out: &mut W) -> fmt::Result {
    match hex {
        HexFloat::Nan => out.write_str("NaN"),
        HexFloat::Inf => out.write_str("Inf"),
        HexFloat::NegInf => out.write_str("-Inf"),
        HexFloat::Number {
            negative,
            first,
            rest,
            exponent_negative,
            exponent,
        } => {
            if *negative {
                out.write_char('-')?;
            }
            out.write_str("0x")?;
            out.write_char(char::from(*first))?;
            out.write_char('.')?;
            write_escaped(out, rest, Escape::Identifier)?;
            out.write_char('p')?;
            if *exponent_negative {
                out.write_char('-')?;
            }
            write_escaped(out, exponent, Escape::Identifier)
        }
    }
}

fn print_string<W: fmt::Write + ?Sized>(
    width: StringWidth,
    hex: &[u8],
    out: &mut W,
) -> fmt::Result {
    out.write_char('"')?;
    for pair in hex.chunks(2) {
        let value = hex_pair(pair);
        match value {
            0x09 => out.write_str("\\t")?,
            0x0a => out.write_str("\\n")?,
            0x0d => out.write_str("\\r")?,
            0x0c => out.write_str("\\f")?,
            0x0b => out.write_str("\\v")?,
            0x20..=0x7e => out.write_char(char::from(value))?,
            _ => {
                out.write_str("\\x")?;
                write_escaped(out, pair, Escape::Verbatim)?;
            }
        }
    }
    out.write_char('"')?;
    match width {
        StringWidth::Utf8 => Ok(()),
        StringWidth::Utf16 => out.write_char('w'),
        StringWidth::Utf32 => out.write_char('d'),
    }
}

fn hex_pair(pair: &[u8]) -> u8 {
    let mut value = 0_u8;
    for &byte in pair {
        let digit = match byte {
            b'0'..=b'9' => byte - b'0',
            b'a'..=b'f' => byte - b'a' + 10,
            b'A'..=b'F' => byte - b'A' + 10,
            _ => 0,
        };
        value = (value << 4) | digit;
    }
    value
}

fn primitive_name(primitive: Primitive) -> &'static str {
    match primitive {
        Primitive::Void => "void",
        Primitive::Byte => "byte",
        Primitive::Ubyte => "ubyte",
        Primitive::Short => "short",
        Primitive::Ushort => "ushort",
        Primitive::Int => "int",
        Primitive::Uint => "uint",
        Primitive::Long => "long",
        Primitive::Ulong => "ulong",
        Primitive::Cent => "cent",
        Primitive::Ucent => "ucent",
        Primitive::Float => "float",
        Primitive::Double => "double",
        Primitive::Real => "real",
        Primitive::Ifloat => "ifloat",
        Primitive::Idouble => "idouble",
        Primitive::Ireal => "ireal",
        Primitive::Cfloat => "cfloat",
        Primitive::Cdouble => "cdouble",
        Primitive::Creal => "creal",
        Primitive::Bool => "bool",
        Primitive::Char => "char",
        Primitive::Wchar => "wchar",
        Primitive::Dchar => "dchar",
    }
}

fn attribute_name(attribute: Attribute) -> &'static str {
    match attribute {
        Attribute::Pure => "pure",
        Attribute::Nothrow => "nothrow",
        Attribute::Ref => "ref",
        Attribute::Property => "@property",
        Attribute::Trusted => "@trusted",
        Attribute::Safe => "@safe",
        Attribute::Nogc => "@nogc",
        Attribute::Return => "return",
        Attribute::Scope => "scope",
        Attribute::Live => "@live",
    }
}
