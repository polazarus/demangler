//! Printer for parsed Go symbol names.
//!
//! Printing never allocates for the common case: identifiers and separators
//! are written straight into the formatter, and `%XX` escapes are decoded into
//! a bounded `String` only when the input contains one. Text is rendered from
//! symbol bytes, so a name that is not valid UTF-8 prints `\xNN` escapes.

use core::fmt;

use super::node::{
    Constant, Function, Local, Name, Package, Receiver, Special, Suffix, Symbol, TypeDescriptor,
};
use super::text::{decoded, text};
use crate::parse::Escape;

/// Write `bytes` as identifier text.
fn write_text<W: fmt::Write + ?Sized>(f: &mut W, bytes: &[u8]) -> fmt::Result {
    f.write_str(&text(bytes, Escape::Identifier))
}

/// Write `bytes` with `%XX` escapes decoded.
fn write_unescaped<W: fmt::Write + ?Sized>(f: &mut W, bytes: &[u8]) -> fmt::Result {
    f.write_str(&decoded(bytes))
}

/// Write opaque compiler payload `bytes` as they are.
fn write_verbatim<W: fmt::Write + ?Sized>(f: &mut W, bytes: &[u8]) -> fmt::Result {
    f.write_str(&text(bytes, Escape::Verbatim))
}

/// Write a parsed name.
pub fn print<W: fmt::Write + ?Sized>(name: &Name<'_>, f: &mut W) -> fmt::Result {
    match name {
        Name::Symbol(symbol) => print_symbol(symbol, f),
        Name::TypeDescriptor(descriptor) => print_type_descriptor(descriptor, f),
        Name::Special(special) => print_special(special, f),
        Name::Shape(payload) => {
            f.write_str("go.shape.")?;
            write_unescaped(f, payload)
        }
        Name::Constant(constant) => print_constant(constant, f),
    }
}

/// Write the object-file symbol version marker (`<N>`) appended by `go tool nm`.
pub fn print_version<W: fmt::Write + ?Sized>(version: Option<&[u8]>, f: &mut W) -> fmt::Result {
    let Some(version) = version else {
        return Ok(());
    };
    f.write_str("<")?;
    write_text(f, version)?;
    f.write_str(">")
}

/// Write a package-qualified or package-local symbol.
fn print_symbol<W: fmt::Write + ?Sized>(symbol: &Symbol<'_>, f: &mut W) -> fmt::Result {
    if let Some(package) = symbol.package() {
        write_package(package, f)?;
        f.write_str(".")?;
    }
    print_local(symbol.local(), f)
}

/// Write an escaped package path.
fn write_package<W: fmt::Write + ?Sized>(package: &Package<'_>, f: &mut W) -> fmt::Result {
    f.write_str(&package.decoded())
}

/// Write the local part of a symbol.
fn print_local<W: fmt::Write + ?Sized>(local: &Local<'_>, f: &mut W) -> fmt::Result {
    match local {
        Local::Function(function) => print_function(function, f),
        Local::GlobalClosure { suffix } => {
            f.write_str("glob..")?;
            write_text(f, suffix)
        }
        Local::InitTask => f.write_str(".inittask"),
        Local::Dictionary { ty } => {
            f.write_str(".dict.")?;
            write_unescaped(f, ty)
        }
        Local::InterfaceSwitch { index } => {
            f.write_str(".interfaceSwitch.")?;
            write_text(f, index)
        }
        Local::TypeAssert { index } => {
            f.write_str(".typeAssert.")?;
            write_text(f, index)
        }
        Local::GoBytes { index } => {
            f.write_str(".gobytes.")?;
            write_text(f, index)
        }
        Local::StaticTemp { index } => {
            f.write_str(".stmp_")?;
            write_text(f, index)
        }
        Local::Data(data) => {
            write_text(f, data.name)?;
            f.write_str(".")?;
            write_text(f, data.payload)
        }
    }
}

/// Write a function, method, closure or wrapper.
fn print_function<W: fmt::Write + ?Sized>(function: &Function<'_>, f: &mut W) -> fmt::Result {
    if let Some(receiver) = function.receiver() {
        print_receiver(receiver, f)?;
        f.write_str(".")?;
    }
    write_unescaped(f, function.name)?;
    for suffix in function.suffixes() {
        print_suffix(suffix, f)?;
    }
    Ok(())
}

/// Write a method receiver.
fn print_receiver<W: fmt::Write + ?Sized>(receiver: &Receiver<'_>, f: &mut W) -> fmt::Result {
    f.write_str("(")?;
    if receiver.pointer {
        f.write_str("*")?;
    }
    write_unescaped(f, receiver.ty)?;
    f.write_str(")")
}

/// Write one compiler-generated function suffix.
fn print_suffix<W: fmt::Write + ?Sized>(suffix: &Suffix<'_>, f: &mut W) -> fmt::Result {
    match suffix {
        Suffix::Closure(index) => {
            f.write_str(".func")?;
            write_text(f, index)
        }
        Suffix::GoWrap(index) => {
            f.write_str(".gowrap")?;
            write_text(f, index)
        }
        Suffix::DeferWrap(index) => {
            f.write_str(".deferwrap")?;
            write_text(f, index)
        }
        Suffix::Range(index) => {
            f.write_str("-range")?;
            write_text(f, index)
        }
        Suffix::MethodValue => f.write_str("-fm"),
        Suffix::FuncValue => f.write_str(".f"),
        Suffix::Nested(index) => {
            f.write_str(".")?;
            write_text(f, index)
        }
        Suffix::Inlined(name) => {
            f.write_str(".")?;
            write_unescaped(f, name)
        }
        Suffix::JumpTable(index) => {
            f.write_str(".jump")?;
            write_text(f, index)
        }
        Suffix::Internal(name) => {
            f.write_str(".")?;
            write_text(f, name)
        }
    }
}

/// Write a `type:` descriptor.
fn print_type_descriptor<W: fmt::Write + ?Sized>(
    descriptor: &TypeDescriptor<'_>,
    f: &mut W,
) -> fmt::Result {
    f.write_str("type:")?;
    f.write_str(descriptor.kind().prefix())?;
    if descriptor.kind().is_opaque() {
        write_verbatim(f, descriptor.payload)
    } else {
        write_unescaped(f, descriptor.payload)
    }
}

/// Write a `go:` special symbol.
fn print_special<W: fmt::Write + ?Sized>(special: &Special<'_>, f: &mut W) -> fmt::Result {
    match special {
        Special::Itab { concrete, iface } => {
            f.write_str("go:itab.")?;
            write_unescaped(f, concrete)?;
            f.write_str(",")?;
            write_unescaped(f, iface)
        }
        Special::String { payload } => {
            f.write_str("go:string.")?;
            write_verbatim(f, payload)
        }
        Special::Other { kind, payload } => {
            f.write_str("go:")?;
            write_text(f, kind)?;
            if !payload.is_empty() {
                f.write_str(".")?;
                write_unescaped(f, payload)?;
            }
            Ok(())
        }
    }
}

/// Write a compiler constant.
fn print_constant<W: fmt::Write + ?Sized>(constant: &Constant<'_>, f: &mut W) -> fmt::Result {
    f.write_str("$")?;
    f.write_str(constant.class().prefix())?;
    write_text(f, constant.value)
}

#[cfg(test)]
mod tests;
