//! PE/COFF symbol decorations.
//!
//! COFF decorations are orthogonal to the language mangling: `_foo@8` is a C
//! symbol with an x86 stdcall marker, while `__imp_?foo@@...` is an imported
//! MSVC C++ symbol. [`crate::Demangled`] keeps the underlying scheme and
//! exposes the decoration separately through
//! [`crate::Demangled::coff_decoration`].
//!
//! Only explicit decoration markers are decoded. A bare leading underscore
//! (`_foo`) is not evidence enough and remains pass-through, as do GNU
//! `@`/`@@` versions and ThinLTO suffixes.

/// Maximum number of nested `__imp_` prefixes accepted.
const MAX_IMPORTS: u8 = 8;

/// Calling convention carried by a COFF decoration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CallingConvention {
    /// C declaration (`cdecl`), the default for imported C symbols.
    Cdecl,
    /// x86 `stdcall`, written `_name@N`.
    Stdcall,
    /// x86 `fastcall`, written `@name@N`.
    Fastcall,
    /// A convention not encoded by the supported decorations.
    Unknown,
}

/// Decoded PE/COFF decoration metadata.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Decoration<'a> {
    symbol: &'a [u8],
    convention: CallingConvention,
    stack_bytes: Option<u16>,
    import: bool,
}

impl<'a> Decoration<'a> {
    /// The underlying symbol bytes, without the COFF decoration.
    #[must_use]
    pub const fn symbol(&self) -> &'a [u8] {
        self.symbol
    }

    /// The decoded calling convention.
    #[must_use]
    pub const fn convention(&self) -> CallingConvention {
        self.convention
    }

    /// Stack bytes encoded by `@N`, when present.
    #[must_use]
    pub const fn stack_bytes(&self) -> Option<u16> {
        self.stack_bytes
    }

    /// Whether the symbol had one or more `__imp_` import prefixes.
    #[must_use]
    pub const fn import(&self) -> bool {
        self.import
    }

    const fn new(
        symbol: &'a [u8],
        convention: CallingConvention,
        stack_bytes: Option<u16>,
        import: bool,
    ) -> Self {
        Self {
            symbol,
            convention,
            stack_bytes,
            import,
        }
    }
}

/// Parse a PE/COFF-decorated symbol into the underlying name and metadata.
///
/// Returns `None` when `input` has no explicit COFF decoration marker, when a
/// marker is malformed, or when more than eight `__imp_` prefixes are nested.
/// GNU `@`/`@@` versions and ThinLTO `@` suffixes are not COFF decorations.
#[must_use]
pub fn parse_bytes(input: &[u8]) -> Option<Decoration<'_>> {
    if input.is_empty() {
        return None;
    }
    let mut rest = input;
    let mut import = false;
    let mut imports = 0_u8;
    while let Some(after) = rest.strip_prefix(b"__imp_") {
        import = true;
        imports = imports.saturating_add(1);
        if imports > MAX_IMPORTS || after.is_empty() {
            return None;
        }
        rest = after;
    }

    if let Some(decoration) = parse_staplike(rest, import) {
        return Some(decoration);
    }
    if import {
        let symbol = rest.strip_prefix(b"_").unwrap_or(rest);
        let mangled = symbol.starts_with(b"?")
            || symbol.starts_with(b"_Z")
            || symbol.starts_with(b"_R");
        if symbol.is_empty() || (symbol.contains(&b'@') && !mangled) {
            return None;
        }
        return Some(Decoration::new(
            symbol,
            CallingConvention::Cdecl,
            None,
            true,
        ));
    }
    None
}

fn parse_staplike(input: &[u8], import: bool) -> Option<Decoration<'_>> {
    if input.starts_with(b"@") {
        let rest = input.strip_prefix(b"@")?;
        let (symbol, stack) = split_at_stack(rest)?;
        if symbol.is_empty() || symbol.contains(&b'@') {
            return None;
        }
        return Some(Decoration::new(
            symbol,
            CallingConvention::Fastcall,
            Some(stack),
            import,
        ));
    }
    let rest = input.strip_prefix(b"_")?;
    let (symbol, stack) = split_at_stack(rest)?;
    if symbol.is_empty() || symbol.contains(&b'@') {
        return None;
    }
    Some(Decoration::new(
        symbol,
        CallingConvention::Stdcall,
        Some(stack),
        import,
    ))
}

fn split_at_stack(input: &[u8]) -> Option<(&[u8], u16)> {
    let index = input.iter().rposition(|byte| *byte == b'@')?;
    let symbol = input.get(..index)?;
    let suffix = input.get(index + 1..)?;
    if suffix.is_empty() || !suffix.iter().all(u8::is_ascii_digit) {
        return None;
    }
    let stack = suffix.iter().try_fold(0_u16, |value, byte| {
        value
            .checked_mul(10)?
            .checked_add(u16::from(byte - b'0'))
    })?;
    Some((symbol, stack))
}

/// True when `input` has a recognized, well-formed COFF decoration.
#[must_use]
pub fn is_coff(input: &[u8]) -> bool {
    parse_bytes(input).is_some()
}

/// Parse a UTF-8 PE/COFF-decorated symbol.
#[must_use]
pub fn parse(input: &str) -> Option<Decoration<'_>> {
    parse_bytes(input.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_stdcall_and_fastcall() {
        let stdcall = parse("_foo@8").unwrap();
        assert_eq!(stdcall.symbol(), b"foo");
        assert_eq!(stdcall.convention(), CallingConvention::Stdcall);
        assert_eq!(stdcall.stack_bytes(), Some(8));

        let fastcall = parse("@foo@8").unwrap();
        assert_eq!(fastcall.symbol(), b"foo");
        assert_eq!(fastcall.convention(), CallingConvention::Fastcall);
    }

    #[test]
    fn decodes_imports() {
        let import = parse("__imp__foo").unwrap();
        assert_eq!(import.symbol(), b"foo");
        assert!(import.import());

        let cpp = parse("__imp_?foo@@YAXXZ").unwrap();
        assert_eq!(cpp.symbol(), b"?foo@@YAXXZ");
        assert!(cpp.import());
    }

    #[test]
    fn rejects_versions_and_plain_underscores() {
        for symbol in [
            "_foo",
            "foo@8",
            "main@@GLIBC_2.2.5",
            "_ZN4test4main17h1234567890abcdefE@@16",
            "__Z3foov.llvm.9D1C9369@@16",
            "_foo@",
            "__imp_",
        ] {
            assert!(parse(symbol).is_none(), "{symbol}");
        }
    }
}
