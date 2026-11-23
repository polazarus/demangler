//! Canonical printer for Kotlin/Native symbols.
//!
//! Kotlin/Native has no standard demangled spelling; stack traces show the raw
//! `kfun:` names. This printer defines the crate's stable, Kotlin-like
//! spelling. It writes straight into the formatter and allocates nothing.

use core::fmt;

use super::node::{Function, Symbol, Type, TypeArgument, TypeParameter, Variance};
use super::text::text;
use crate::parse::Escape;

/// Write a parsed symbol.
pub(crate) fn print<W: fmt::Write + ?Sized>(symbol: &Symbol<'_>, out: &mut W) -> fmt::Result {
    match symbol {
        Symbol::Function(function) => print_function(function, out),
        Symbol::Type(ty) => print_type(ty, out),
        Symbol::Class(path)
        | Symbol::Property(path)
        | Symbol::Object(path)
        | Symbol::Field(path)
        | Symbol::TypeAlias(path)
        | Symbol::Package(path)
        | Symbol::Namespace(path)
        | Symbol::Variable(path) => print_path(path, out),
    }
}

/// Write a qualified path, decoding the `#` member separator to `.`.
fn print_path<W: fmt::Write + ?Sized>(path: &[u8], out: &mut W) -> fmt::Result {
    for ch in text(path, Escape::Identifier).chars() {
        if ch == '#' {
            out.write_char('.')?;
        } else {
            out.write_char(ch)?;
        }
    }
    Ok(())
}

/// Write a parsed function.
fn print_function<W: fmt::Write + ?Sized>(function: &Function<'_>, out: &mut W) -> fmt::Result {
    if function.is_static() {
        out.write_str("static ")?;
    }
    if function.is_suspend() {
        out.write_str("suspend ")?;
    }
    print_path(function.path, out)?;
    out.write_char('.')?;
    out.write_str(&function.name())?;
    if let Some(companion) = function.companion() {
        out.write_str(" companion@")?;
        out.write_str(&companion)?;
    }
    if function.has_signature() {
        if !function.context_params().is_empty() {
            out.write_str(" context(")?;
            print_types(function.context_params(), out)?;
            out.write_char(')')?;
        }
        if let Some(receiver) = function.receiver() {
            out.write_char('@')?;
            print_type(receiver, out)?;
        }
        out.write_char('(')?;
        print_types(function.params(), out)?;
        out.write_char(')')?;
        if !function.type_params().is_empty() {
            print_type_parameters(function.type_params(), out)?;
        }
        if let Some(ret) = function.return_type() {
            out.write_str(": ")?;
            print_type(ret, out)?;
        }
    }
    if let Some(platform) = function.platform() {
        out.write_str(" %")?;
        out.write_str(&platform)?;
    }
    Ok(())
}

/// Write a comma-separated list of types.
fn print_types<W: fmt::Write + ?Sized>(types: &[Type<'_>], out: &mut W) -> fmt::Result {
    for (index, ty) in types.iter().enumerate() {
        if index > 0 {
            out.write_str(", ")?;
        }
        print_type(ty, out)?;
    }
    Ok(())
}

/// Write a type-parameter list.
fn print_type_parameters<W: fmt::Write + ?Sized>(
    parameters: &[TypeParameter<'_>],
    out: &mut W,
) -> fmt::Result {
    out.write_char('<')?;
    for (index, parameter) in parameters.iter().enumerate() {
        if index > 0 {
            out.write_str(", ")?;
        }
        out.write_char('T')?;
        write!(out, "{}", parameter.index())?;
        if !parameter.bounds().is_empty() {
            out.write_str(": ")?;
            for (bound_index, bound) in parameter.bounds().iter().enumerate() {
                if bound_index > 0 {
                    out.write_str(" & ")?;
                }
                print_type(bound, out)?;
            }
        }
    }
    out.write_char('>')
}

/// Write one type.
fn print_type<W: fmt::Write + ?Sized>(ty: &Type<'_>, out: &mut W) -> fmt::Result {
    match ty {
        Type::Class {
            name,
            arguments,
            nullable,
        } => {
            out.write_str(&text(name, Escape::Identifier))?;
            if !arguments.is_empty() {
                print_type_arguments(arguments, out)?;
            }
            if *nullable {
                out.write_char('?')?;
            }
            Ok(())
        }
        Type::TypeParameter {
            container,
            index,
            nullable,
        } => {
            out.write_char('T')?;
            write!(out, "{container}:{index}")?;
            if *nullable {
                out.write_char('?')?;
            }
            Ok(())
        }
        Type::Dynamic => out.write_str("dynamic"),
        Type::Error => out.write_str("<ERROR CLASS>"),
        Type::Flexible { lower, upper } => {
            print_type(lower, out)?;
            out.write_str("..")?;
            print_type(upper, out)
        }
        Type::Vararg(inner) => {
            out.write_str("vararg ")?;
            print_type(inner, out)
        }
    }
}

/// Write a type-argument list.
fn print_type_arguments<W: fmt::Write + ?Sized>(
    arguments: &[TypeArgument<'_>],
    out: &mut W,
) -> fmt::Result {
    out.write_char('<')?;
    for (index, argument) in arguments.iter().enumerate() {
        if index > 0 {
            out.write_str(", ")?;
        }
        match argument {
            TypeArgument::Star => out.write_char('*')?,
            TypeArgument::Type { variance, ty } => {
                if *variance != Variance::Invariant {
                    out.write_str(variance.as_str())?;
                    out.write_char(' ')?;
                }
                print_type(ty, out)?;
            }
        }
    }
    out.write_char('>')
}

#[cfg(test)]
mod tests {
    use alloc::string::ToString;

    use crate::kotlin_native::parse;

    fn text(symbol: &str) -> alloc::string::String {
        parse(symbol).unwrap().to_string()
    }

    #[test]
    fn prints_plan_examples() {
        assert_eq!(
            text("kfun:com.example.foo#bar(kotlin.Int){}"),
            "com.example.foo.bar(kotlin.Int)"
        );
        assert_eq!(
            text("kfun:com.example.Box#<init>(kotlin.Int){}"),
            "com.example.Box.<init>(kotlin.Int)"
        );
        assert_eq!(
            text("kfun:com.example.Box#get(){}"),
            "com.example.Box.get()"
        );
        assert_eq!(
            text("kfun:com.example#topLevel(){}"),
            "com.example.topLevel()"
        );
        assert_eq!(
            text("ktype:com.example.Box<kotlin.Int>"),
            "com.example.Box<kotlin.Int>"
        );
    }

    #[test]
    fn prints_every_symbol_family() {
        for (symbol, expected) in [
            ("kclass:com.example.Box", "com.example.Box"),
            ("kproperty:com.example.Box#value", "com.example.Box.value"),
            ("kobj:com.example.Singleton", "com.example.Singleton"),
            ("kfield:com.example.Box.value", "com.example.Box.value"),
            ("ktypealias:com.example.IntList", "com.example.IntList"),
            ("kpackage:com.example", "com.example"),
            ("knamespace:com.example", "com.example"),
            (
                "kvar:com.example.Singleton.$instance",
                "com.example.Singleton.$instance",
            ),
        ] {
            assert_eq!(text(symbol), expected, "{symbol}");
        }
    }

    #[test]
    fn prints_flexible_and_error_types() {
        assert_eq!(
            text("ktype:[kotlin.String~kotlin.String?]"),
            "kotlin.String..kotlin.String?"
        );
        assert_eq!(text("ktype:<dynamic>"), "dynamic");
        assert_eq!(text("ktype:<ERROR CLASS>"), "<ERROR CLASS>");
    }

    #[test]
    fn prints_multiple_bounds() {
        assert_eq!(
            text("kfun:com.example#foo(0:0){0§<kotlin.Any&kotlin.Comparable<0:0>>}0:0"),
            "com.example.foo(T0:0)<T0: kotlin.Any & kotlin.Comparable<T0:0>>: T0:0"
        );
    }
}
