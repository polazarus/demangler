//! Printer for parsed Objective-C symbols.
//!
//! Metadata is rendered with the project's stable canonical spelling; method
//! display forms are printed byte-for-byte as they were written.

use core::fmt;

use super::node::{BlockDescriptor, Item, Metadata, Method};

/// Write a parsed symbol.
pub(crate) fn print<W: fmt::Write + ?Sized>(item: &Item<'_>, out: &mut W) -> fmt::Result {
    match item {
        Item::Metadata(metadata) => print_metadata(metadata, out),
        Item::Method(method) => print_method(method, out),
        Item::BlockDescriptor(descriptor) => print_block_descriptor(descriptor, out),
    }
}

/// Write one metadata record.
fn print_metadata<W: fmt::Write + ?Sized>(metadata: &Metadata<'_>, out: &mut W) -> fmt::Result {
    out.write_str("Objective-C ")?;
    out.write_str(metadata.kind().description())?;
    if let Some(subject) = metadata.subject() {
        out.write_char(' ')?;
        out.write_str(&subject)?;
        if let Some(member) = metadata.member() {
            out.write_char('.')?;
            out.write_str(&member)?;
        }
        if let Some(category) = metadata.category() {
            out.write_str(" (")?;
            out.write_str(&category)?;
            out.write_char(')')?;
        }
    }
    Ok(())
}

/// Write a method display form.
fn print_method<W: fmt::Write + ?Sized>(method: &Method<'_>, out: &mut W) -> fmt::Result {
    out.write_char(method.kind().sign())?;
    out.write_char('[')?;
    out.write_str(&method.class())?;
    if let Some(category) = method.category() {
        out.write_char('(')?;
        out.write_str(&category)?;
        out.write_char(')')?;
    }
    out.write_char(' ')?;
    out.write_str(&method.selector())?;
    out.write_char(']')
}

/// Write a block descriptor.
fn print_block_descriptor<W: fmt::Write + ?Sized>(
    descriptor: &BlockDescriptor<'_>,
    out: &mut W,
) -> fmt::Result {
    out.write_str("Objective-C block descriptor ")?;
    out.write_str(&descriptor.payload())
}
