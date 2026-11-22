//! Printer for parsed Go symbol names.
//!
//! Printing never allocates for the common case: identifiers and separators
//! are written straight into the formatter, and `%XX` escapes are decoded into
//! a bounded `String` only when the input contains one.

use core::fmt;

use super::node::{
    Constant, Function, Local, Name, Package, Receiver, Special, Suffix, Symbol, TypeDescriptor,
    unescape,
};

/// Write `raw` with `%XX` escapes decoded.
fn write_unescaped<W: fmt::Write + ?Sized>(f: &mut W, raw: &str) -> fmt::Result {
    f.write_str(unescape(raw).as_ref())
}

/// Write a parsed name.
pub(crate) fn print<W: fmt::Write + ?Sized>(name: &Name<'_>, f: &mut W) -> fmt::Result {
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
    write_unescaped(f, package.raw())
}

/// Write the local part of a symbol.
fn print_local<W: fmt::Write + ?Sized>(local: &Local<'_>, f: &mut W) -> fmt::Result {
    match local {
        Local::Function(function) => print_function(function, f),
        Local::GlobalClosure { suffix } => {
            f.write_str("glob..")?;
            f.write_str(suffix)
        }
        Local::InitTask => f.write_str(".inittask"),
        Local::Dictionary { ty } => {
            f.write_str(".dict.")?;
            write_unescaped(f, ty)
        }
        Local::InterfaceSwitch { index } => {
            f.write_str(".interfaceSwitch.")?;
            f.write_str(index)
        }
        Local::Data(data) => {
            f.write_str(data.name)?;
            f.write_str(".")?;
            f.write_str(data.payload)
        }
    }
}

/// Write a function, method, closure or wrapper.
fn print_function<W: fmt::Write + ?Sized>(function: &Function<'_>, f: &mut W) -> fmt::Result {
    if let Some(receiver) = function.receiver() {
        print_receiver(receiver, f)?;
        f.write_str(".")?;
    }
    write_unescaped(f, function.name())?;
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
            f.write_str(index)
        }
        Suffix::GoWrap(index) => {
            f.write_str(".gowrap")?;
            f.write_str(index)
        }
        Suffix::DeferWrap(index) => {
            f.write_str(".deferwrap")?;
            f.write_str(index)
        }
        Suffix::Range(index) => {
            f.write_str("-range")?;
            f.write_str(index)
        }
        Suffix::MethodValue => f.write_str("-fm"),
        Suffix::FuncValue => f.write_str(".f"),
        Suffix::Nested(index) => {
            f.write_str(".")?;
            f.write_str(index)
        }
        Suffix::Inlined(name) => {
            f.write_str(".")?;
            write_unescaped(f, name)
        }
        Suffix::Internal(name) => {
            f.write_str(".")?;
            f.write_str(name)
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
        f.write_str(descriptor.payload())
    } else {
        write_unescaped(f, descriptor.payload())
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
            f.write_str(payload)
        }
        Special::Other { kind, payload } => {
            f.write_str("go:")?;
            f.write_str(kind)?;
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
    f.write_str(constant.value())
}

#[cfg(test)]
mod tests {
    use alloc::string::ToString;

    use crate::go::node::{ConstantClass, TypeKind};
    use crate::go::parse;

    fn text(name: &str) -> alloc::string::String {
        parse(name).unwrap().to_string()
    }

    #[test]
    fn prints_packages_and_receivers() {
        assert_eq!(text("fmt.Println"), "fmt.Println");
        assert_eq!(
            text("fmt.(*pp).doPrintln"),
            "fmt.(*pp).doPrintln"
        );
        assert_eq!(
            text("fmt.(pp).doPrintln"),
            "fmt.(pp).doPrintln"
        );
    }

    #[test]
    fn prints_escapes_decoded() {
        assert_eq!(
            text("example.com/x/pkg%2ev1.Make"),
            "example.com/x/pkg.v1.Make"
        );
        assert_eq!(
            text("go:itab.*example.com/x/pkg%2ev1.T,error"),
            "go:itab.*example.com/x/pkg.v1.T,error"
        );
    }

    #[test]
    fn prints_opaque_payloads_verbatim() {
        assert_eq!(text("go:string.\"100%\""), "go:string.\"100%\"");
        assert_eq!(
            text("type:.namedata.F.json:\"%2e\""),
            "type:.namedata.F.json:\"%2e\""
        );
    }

    #[test]
    fn prints_abi_markers_stripped() {
        assert_eq!(text("runtime.memmove.abi0"), "runtime.memmove");
        assert_eq!(
            text("runtime.memmove.abiinternal"),
            "runtime.memmove"
        );
        assert_eq!(text("runtime.memmove.abiwrap"), "runtime.memmove");
    }

    #[test]
    fn prints_every_name_kind() {
        for name in [
            "fmt.Println",
            "type:.eq.main.T",
            "type:*[]struct { key string; elem func() int }",
            "go:itab.*os.File,io.Writer",
            "go:buildid",
            "go:string.\"hello\"",
            "go.shape.struct { x int }",
            "$f64.3ff0000000000000",
            "main..inittask",
            "cmp..dict.Compare[int]",
            "errors..interfaceSwitch.0",
            "runtime.gcbits.*",
        ] {
            assert_eq!(text(name), name, "{name}");
        }
        // `·` is normalised to `.`, like the linker's ELF symbol table does.
        assert_eq!(
            text("runtime.gclocals·+ISnmBhJ1y+frD3sAmLgtg=="),
            "runtime.gclocals.+ISnmBhJ1y+frD3sAmLgtg=="
        );
    }

    #[test]
    fn prints_constants() {
        for name in [
            "$f32.3f800000",
            "$f64.3ff0000000000000",
            "$i32.00000001",
            "$i64.0000000000000001",
            "$i128.00000000000000000000000000000001",
        ] {
            assert_eq!(text(name), name);
        }
    }

    #[test]
    fn constant_classes_are_reported() {
        assert_eq!(ConstantClass::F32.prefix(), "f32.");
        assert_eq!(ConstantClass::F64.prefix(), "f64.");
        assert_eq!(ConstantClass::I32.prefix(), "i32.");
        assert_eq!(ConstantClass::I64.prefix(), "i64.");
        assert_eq!(ConstantClass::I128.prefix(), "i128.");
        assert!(TypeKind::NameData.is_opaque());
    }
}
