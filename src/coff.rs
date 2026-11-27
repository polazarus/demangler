//! PE/COFF symbol decorations.
//!
//! COFF decorations are orthogonal to the language mangling: `_foo@8` is a C
//! symbol with an x86 stdcall marker, while `__imp_?foo@@...` is an imported
//! MSVC C++ symbol. [`crate::Demangled`] keeps the underlying scheme and
//! exposes the decoration separately through
//! [`crate::Demangled::coff_decoration`].
//!
//! # Entry points
//!
//! * `parse_bytes` / `parse` / `is_coff` are platform-agnostic. They claim only
//!   markers that no other platform spells the same way: `_name@N`, `@name@N`
//!   and `__imp_...`. These are the entry points the crate's dispatcher uses,
//!   so scheme detection is unchanged.
//! * `parse_bytes_for` / `parse_for` apply the decoration rules of an
//!   explicitly declared `Target`. They decode the forms that only become
//!   unambiguous once the target is known: the x86 bare `_name`, the `name@@N`
//!   vectorcall suffix shared by x86 and x64, the `.weak.<name>.default`
//!   COMDAT weak-alias wrapper and the `.cold`/`.cold.N`/`.part.N` linker
//!   fragment suffixes. The rules classify every non-empty input: a name
//!   without a decoration is reported as `DecorationKind::Undecorated` with
//!   its bytes unchanged.
//! * `parse_leading_underscore_bytes` / `parse_leading_underscore` decode only
//!   the bare x86 `cdecl` decoration `_name`.
//!
//! # Decoration forms
//!
//! | Spelling | Target | `DecorationKind` | `CallingConvention` |
//! |---|---|---|---|
//! | `name` | x64, ARM64 | `Undecorated` | `Unknown` |
//! | `_name` | x86 | `LeadingUnderscore` | `Cdecl` |
//! | `_name@N` | x86 | `ConventionSuffix` | `Stdcall` |
//! | `@name@N` | x86 | `ConventionSuffix` | `Fastcall` |
//! | `name@@N` | x86, x64 | `ConventionSuffix` | `Vectorcall` |
//! | `__imp_...` | x86, x64, ARM64 | `Import` | `Cdecl` |
//! | `.weak.name.default` | x86, x64, ARM64 | `WeakAlias` | as the inner name |
//! | `name.cold`, `name.cold.N`, `name.part.N` | x86, x64, ARM64 | `Fragment` | as the inner name |
//!
//! The wrappers compose: `__imp__foo@8` is an import *and* a stdcall
//! decoration, so `Decoration::kind` reports the outermost wrapper (`Import`),
//! while `Decoration::convention` and `Decoration::stack_bytes` describe the
//! inner name (`Stdcall`, `8`). `Decoration::import`, `Decoration::weak_alias`
//! and `Decoration::fragment` report the wrappers independently, and
//! `Decoration::symbol` is the undecorated name. A fragment suffix is appended
//! after every other decoration, so `_coff_stdcall@8.cold.1` is a `Fragment`
//! with `convention == Stdcall`, `stack_bytes == Some(8)` and
//! `symbol == b"coff_stdcall"`.
//!
//! `N` is the decimal size suffix. For `stdcall`/`fastcall` it is the number of
//! stack bytes the callee pops; for `vectorcall` it is the argument-block size
//! the compiler encodes (4 bytes per parameter on x86, 8 on x64). Values above
//! 65535 are rejected.
//!
//! # Not claimed
//!
//! * A bare `_name` is never claimed by `parse_bytes`: a leading underscore is
//!   also a Mach-O and an assembly-label convention, so it is decoded only by
//!   the explicit entry points above.
//! * `name@@N` is never claimed by `parse_bytes`: on ELF the same spelling is a
//!   GNU symbol version (`_ZN...@@16`), so it is decoded only when the caller
//!   declares an x86 or x64 COFF `Target`.
//! * `.weak.<name>.default` is never claimed by `parse_bytes`: decoding the
//!   inner name needs the target (x86 strips a decoration underscore, x64 and
//!   ARM64 do not), so it is decoded only by `parse_bytes_for`.
//! * `.cold`, `.cold.N` and `.part.N` fragments are never claimed by
//!   `parse_bytes`: they are language-neutral linker suffixes, so decoding one
//!   would steal the name from a backend that may still demangle the whole
//!   symbol. They are decoded only by `parse_bytes_for`.
//! * The irregular `.weak.<name>.default.<assoc>` spelling that clang emits for
//!   some C++ weak COMDATs is rejected rather than guessed: the wrapper must
//!   end in `.default`, and only one weak-alias wrapper and one fragment
//!   suffix are decoded.
//! * `thiscall` has no free-standing COFF decoration, so no spelling is
//!   decoded as it; MSVC encodes the convention inside the `?` mangling and
//!   the C++ backend reports it.
//! * GNU `@`/`@@` versions and ThinLTO `.llvm.<hex>` suffixes are not COFF
//!   decorations.
//!
//! The parser is allocation-free, linear in the input and never recurses: it
//! peels every `__imp_` prefix in one pass (each consumes six input bytes),
//! at most one weak-alias wrapper and at most one fragment suffix.

/// Prefix of the COMDAT weak-alias symbol clang/LLVM emits.
const WEAK_ALIAS_PREFIX: &[u8] = b".weak.";

/// Suffix of the COMDAT weak-alias symbol clang/LLVM emits.
const WEAK_ALIAS_SUFFIX: &[u8] = b".default";

/// Marker of the outlined cold half of a hot/cold split.
const COLD_FRAGMENT: &[u8] = b".cold";

/// Marker of a partially inlined body.
const PART_FRAGMENT: &[u8] = b".part";

/// Calling convention carried by a COFF decoration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum CallingConvention {
    /// C declaration (`cdecl`); also the neutral default reported for an
    /// import pointer, which does not encode a convention.
    Cdecl,
    /// x86 `stdcall`, written `_name@N`.
    Stdcall,
    /// x86 `fastcall`, written `@name@N`.
    Fastcall,
    /// `vectorcall`, written `name@@N` on x86 and x64.
    Vectorcall,
    /// A convention the decoration does not encode.
    Unknown,
}

/// The COFF target whose decoration rules apply.
///
/// The target is not encoded in a symbol name, so it is the caller's
/// declaration: use it only for names read from an object file built for that
/// machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Target {
    /// 32-bit x86: `_name`, `_name@N`, `@name@N`, `name@@N` and `__imp_...`.
    X86,
    /// x64: `name@@N` and `__imp_...`; calling conventions are otherwise
    /// undecorated.
    X64,
    /// ARM64: `__imp_...` only; calling conventions are not decorated.
    Arm64,
}

/// The decoration form that was recognised.
///
/// The variant names the outermost marker. A calling-convention suffix inside
/// an import pointer or weak-alias wrapper is reported through
/// [`Decoration::convention`] instead.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum DecorationKind {
    /// No decoration marker: the name is returned unchanged.
    Undecorated,
    /// The x86 `cdecl` leading underscore (`_name`).
    LeadingUnderscore,
    /// A calling-convention suffix: `_name@N`, `@name@N` or `name@@N`.
    ConventionSuffix,
    /// One or more `__imp_` import prefixes.
    Import,
    /// A COMDAT weak-alias wrapper: `.weak.<name>.default`.
    WeakAlias,
    /// A linker fragment suffix: `.cold`, `.cold.N` or `.part.N`.
    Fragment,
}

/// A linker fragment suffix appended to a decorated COFF symbol.
///
/// A compiler that splits a function appends the fragment to the name it
/// emits, so the suffix follows every other decoration:
/// `_coff_stdcall@8.cold.1` is the cold half of `_coff_stdcall@8`. Only the
/// target-aware entry points decode a fragment; the platform-agnostic ones
/// leave the whole symbol to the language backend.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Fragment {
    /// The outlined cold half of a hot/cold split.
    ///
    /// LLVM's `HotColdSplitting` pass names it `<name>.cold.<N>`; GCC's
    /// `-freorder-blocks-and-partition` names it `<name>.cold`.
    Cold {
        /// The `.N` index, present when the compiler appends one.
        index: Option<u16>,
    },
    /// A partially inlined body, written `<name>.part.<N>`.
    Part {
        /// The `.N` index.
        index: u16,
    },
}

/// Decoded PE/COFF decoration metadata.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Decoration<'a> {
    symbol: &'a [u8],
    kind: DecorationKind,
    convention: CallingConvention,
    stack_bytes: Option<u16>,
    import: bool,
    weak_alias: bool,
    fragment: Option<Fragment>,
}

impl<'a> Decoration<'a> {
    /// The underlying symbol bytes, without the COFF decoration.
    #[must_use]
    pub const fn symbol(&self) -> &'a [u8] {
        self.symbol
    }

    /// The outermost decoration form that was recognised.
    #[must_use]
    pub const fn kind(&self) -> DecorationKind {
        self.kind
    }

    /// The decoded calling convention.
    #[must_use]
    pub const fn convention(&self) -> CallingConvention {
        self.convention
    }

    /// The decimal `@N`/`@@N` suffix, when present.
    ///
    /// This is the callee-popped stack size for `stdcall`/`fastcall` and the
    /// argument-block size the compiler encodes for `vectorcall`.
    #[must_use]
    pub const fn stack_bytes(&self) -> Option<u16> {
        self.stack_bytes
    }

    /// Whether the symbol had one or more `__imp_` import prefixes.
    #[must_use]
    pub const fn import(&self) -> bool {
        self.import
    }

    /// Whether the symbol was a COMDAT weak-alias wrapper
    /// (`.weak.<name>.default`).
    #[must_use]
    pub const fn weak_alias(&self) -> bool {
        self.weak_alias
    }

    /// The linker fragment suffix that was decoded, when present.
    ///
    /// Only the target-aware entry points decode a fragment; see
    /// [`Fragment`].
    #[must_use]
    pub const fn fragment(&self) -> Option<Fragment> {
        self.fragment
    }

    const fn new(
        symbol: &'a [u8],
        kind: DecorationKind,
        convention: CallingConvention,
        stack_bytes: Option<u16>,
        import: bool,
        weak_alias: bool,
        fragment: Option<Fragment>,
    ) -> Self {
        Self {
            symbol,
            kind,
            convention,
            stack_bytes,
            import,
            weak_alias,
            fragment,
        }
    }
}

/// The rules used by one parsing entry point.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Rules {
    /// Platform-agnostic: only markers that no other platform shares.
    Auto,
    /// The declared decoration rules of one COFF target.
    Target(Target),
}

/// Parse a PE/COFF-decorated symbol into the underlying name and metadata.
///
/// Returns `None` when `input` has no explicit COFF decoration marker, when a
/// marker is malformed, or when more than eight `__imp_` prefixes are nested.
/// GNU `@`/`@@` versions, ThinLTO `@`/`.llvm.` suffixes, bare `_name`,
/// `name@@N` vectorcall, `.weak.<name>.default` weak aliases and
/// `.cold`/`.cold.N`/`.part.N` fragments are not claimed; see
/// [`parse_bytes_for`] for those.
#[must_use]
pub fn parse_bytes(input: &[u8]) -> Option<Decoration<'_>> {
    parse_with(input, Rules::Auto)
}

/// Parse a UTF-8 PE/COFF-decorated symbol.
#[must_use]
pub fn parse(input: &str) -> Option<Decoration<'_>> {
    parse_bytes(input.as_bytes())
}

/// True when `input` has a recognized, well-formed COFF decoration.
#[must_use]
pub fn is_coff(input: &[u8]) -> bool {
    parse_bytes(input).is_some()
}

/// Parse a symbol with the decoration rules of an explicit COFF [`Target`].
///
/// The rules classify every non-empty input: a name the target does not
/// decorate is returned as [`DecorationKind::Undecorated`] with its bytes
/// unchanged, and a `.cold`/`.cold.N`/`.part.N` fragment suffix is peeled into
/// [`Decoration::fragment`]. `None` means the input is empty or starts with a
/// decoration marker that the target's rules cannot decode.
#[must_use]
pub fn parse_bytes_for(input: &[u8], target: Target) -> Option<Decoration<'_>> {
    parse_with(input, Rules::Target(target))
}

/// Parse a UTF-8 symbol with the decoration rules of an explicit [`Target`].
#[must_use]
pub fn parse_for(input: &str, target: Target) -> Option<Decoration<'_>> {
    parse_bytes_for(input.as_bytes(), target)
}

/// Decode the bare x86 `cdecl` decoration `_name`.
///
/// A bare leading underscore is also a Mach-O and an assembly-label convention,
/// so [`parse_bytes`] never claims it. Callers that know the name came from a
/// 32-bit COFF object opt in here, or use [`parse_bytes_for`] with
/// [`Target::X86`]. Returns `None` for input without a leading underscore, an
/// empty name, or a name containing `@`.
#[must_use]
pub fn parse_leading_underscore_bytes(input: &[u8]) -> Option<Decoration<'_>> {
    let symbol = leading_underscore_name(input)?;
    Some(Decoration::new(
        symbol,
        DecorationKind::LeadingUnderscore,
        CallingConvention::Cdecl,
        None,
        false,
        false,
        None,
    ))
}

/// Decode the bare x86 `cdecl` decoration `_name` from a UTF-8 symbol.
#[must_use]
pub fn parse_leading_underscore(input: &str) -> Option<Decoration<'_>> {
    parse_leading_underscore_bytes(input.as_bytes())
}

fn parse_with(input: &[u8], rules: Rules) -> Option<Decoration<'_>> {
    if input.is_empty() {
        return None;
    }
    let (input, fragment) = match rules {
        Rules::Auto => (input, None),
        Rules::Target(_) => match split_fragment(input) {
            FragmentSplit::Decoded(base, fragment) => (base, Some(fragment)),
            FragmentSplit::None => (input, None),
            // A fragment marker that does not decode is rejected rather than
            // reported as an undecorated name.
            FragmentSplit::Malformed => return None,
        },
    };
    if let Rules::Target(_) = rules {
        if let Some(inner) = strip_weak_alias(input) {
            // Only one weak-alias wrapper is decoded; a nested one would need
            // its own target-specific inner decoding.
            if inner.starts_with(WEAK_ALIAS_PREFIX) {
                return None;
            }
            let mut decoration = parse_name_with_imports(inner, rules)?;
            decoration.weak_alias = true;
            decoration.kind = DecorationKind::WeakAlias;
            return Some(with_fragment(decoration, fragment));
        }
        // A weak-alias marker that does not decode is rejected rather than
        // reported as an undecorated name.
        if input.starts_with(WEAK_ALIAS_PREFIX) {
            return None;
        }
    }
    let decoration = parse_name_with_imports(input, rules)?;
    Some(with_fragment(decoration, fragment))
}

/// Record a decoded fragment suffix as the outermost decoration.
const fn with_fragment(
    mut decoration: Decoration<'_>,
    fragment: Option<Fragment>,
) -> Decoration<'_> {
    decoration.fragment = fragment;
    if fragment.is_some() {
        decoration.kind = DecorationKind::Fragment;
    }
    decoration
}

fn parse_name_with_imports(input: &[u8], rules: Rules) -> Option<Decoration<'_>> {
    let mut rest = input;
    let mut import = false;
    while let Some(after) = rest.strip_prefix(b"__imp_") {
        if after.is_empty() {
            return None;
        }
        import = true;
        rest = after;
    }
    let (symbol, name_kind, convention, stack_bytes) = decode_name(rest, rules, import)?;
    if symbol.is_empty() {
        return None;
    }
    let kind = if import {
        DecorationKind::Import
    } else {
        name_kind
    };
    Some(Decoration::new(
        symbol,
        kind,
        convention,
        stack_bytes,
        import,
        false,
        None,
    ))
}

/// Decode the name that follows any `__imp_` prefixes.
///
/// Returns the undecorated name, the kind of the name decoration, the calling
/// convention and the decimal suffix.
fn decode_name(
    input: &[u8],
    rules: Rules,
    import: bool,
) -> Option<(&[u8], DecorationKind, CallingConvention, Option<u16>)> {
    match rules {
        Rules::Auto => {
            if let Some((symbol, convention, stack)) = parse_staplike(input) {
                return Some((
                    symbol,
                    DecorationKind::ConventionSuffix,
                    convention,
                    Some(stack),
                ));
            }
            if !import {
                return None;
            }
            let symbol = import_symbol(input)?;
            Some((
                symbol,
                DecorationKind::Import,
                CallingConvention::Cdecl,
                None,
            ))
        }
        Rules::Target(target) => {
            // MSVC `?` manglings encode their own convention and contain `@`
            // separators, so no COFF marker applies to them.
            if is_msvc_name(input) {
                let convention = if import {
                    CallingConvention::Cdecl
                } else {
                    CallingConvention::Unknown
                };
                return Some((input, DecorationKind::Undecorated, convention, None));
            }
            match target {
                Target::X86 => {
                    if let Some((symbol, convention, stack)) = parse_staplike(input) {
                        return Some((
                            symbol,
                            DecorationKind::ConventionSuffix,
                            convention,
                            Some(stack),
                        ));
                    }
                    if let Some((symbol, stack)) = parse_vectorcall(input) {
                        return Some((
                            symbol,
                            DecorationKind::ConventionSuffix,
                            CallingConvention::Vectorcall,
                            Some(stack),
                        ));
                    }
                    if let Some(symbol) = leading_underscore_name(input) {
                        return Some((
                            symbol,
                            DecorationKind::LeadingUnderscore,
                            CallingConvention::Cdecl,
                            None,
                        ));
                    }
                }
                Target::X64 => {
                    if let Some((symbol, stack)) = parse_vectorcall(input) {
                        return Some((
                            symbol,
                            DecorationKind::ConventionSuffix,
                            CallingConvention::Vectorcall,
                            Some(stack),
                        ));
                    }
                }
                Target::Arm64 => {}
            }
            // A leftover `@` on a target that decorates with it is a malformed
            // decoration or an ELF version suffix, not an undecorated name.
            if !matches!(target, Target::Arm64) && input.contains(&b'@') {
                return None;
            }
            let convention = if import {
                CallingConvention::Cdecl
            } else {
                CallingConvention::Unknown
            };
            Some((input, DecorationKind::Undecorated, convention, None))
        }
    }
}

/// Strip the `.weak.<name>.default` COMDAT weak-alias wrapper.
fn strip_weak_alias(input: &[u8]) -> Option<&[u8]> {
    let rest = input.strip_prefix(WEAK_ALIAS_PREFIX)?;
    let inner = rest.strip_suffix(WEAK_ALIAS_SUFFIX)?;
    if inner.is_empty() {
        return None;
    }
    Some(inner)
}

/// The outcome of peeling a linker fragment suffix.
enum FragmentSplit<'a> {
    /// The input has no fragment marker.
    None,
    /// A well-formed fragment suffix and the name it belongs to.
    Decoded(&'a [u8], Fragment),
    /// The input ends with a fragment marker that does not decode.
    Malformed,
}

/// Split a linker fragment suffix from the decorated name it belongs to.
///
/// The grammar is the compiler-emitted suffix set this module claims:
///
/// - `.cold` — GCC `-freorder-blocks-and-partition`;
/// - `.cold.<N>` — LLVM `HotColdSplitting`, with a decimal `u16` index;
/// - `.part.<N>` — partial inlining, with a required decimal `u16` index.
///
/// A marker with an empty base, an empty or non-decimal index, an index above
/// `u16::MAX` and a nested fragment (`a.cold.1.cold.2`) are malformed: the
/// target rules reject them instead of reporting a guessed base name.
fn split_fragment(input: &[u8]) -> FragmentSplit<'_> {
    if let Some((base, tail)) = fragment_marker(input, COLD_FRAGMENT) {
        return cold_fragment(base, tail).map_or(FragmentSplit::Malformed, |fragment| {
            FragmentSplit::Decoded(base, fragment)
        });
    }
    if let Some((base, tail)) = fragment_marker(input, PART_FRAGMENT) {
        return part_fragment(base, tail).map_or(FragmentSplit::Malformed, |fragment| {
            FragmentSplit::Decoded(base, fragment)
        });
    }
    FragmentSplit::None
}

/// Split `input` at its last `marker` when the marker is followed by the end of
/// the input or by a `.`-separated tail.
fn fragment_marker<'a>(input: &'a [u8], marker: &[u8]) -> Option<(&'a [u8], &'a [u8])> {
    let index = input
        .windows(marker.len())
        .rposition(|window| window == marker)?;
    let tail = input.get(index + marker.len()..)?;
    if tail.is_empty() || tail.starts_with(b".") {
        input.get(..index).map(|base| (base, tail))
    } else {
        None
    }
}

/// Decode a `.cold` / `.cold.<N>` suffix.
fn cold_fragment(base: &[u8], tail: &[u8]) -> Option<Fragment> {
    if !is_fragment_base(base) {
        return None;
    }
    if tail.is_empty() {
        return Some(Fragment::Cold { index: None });
    }
    let index = parse_decimal(tail.strip_prefix(b".")?)?;
    Some(Fragment::Cold { index: Some(index) })
}

/// Decode a `.part.<N>` suffix; the index is mandatory.
fn part_fragment(base: &[u8], tail: &[u8]) -> Option<Fragment> {
    if !is_fragment_base(base) {
        return None;
    }
    let index = parse_decimal(tail.strip_prefix(b".")?)?;
    Some(Fragment::Part { index })
}

/// A fragment base must be a non-empty name that does not end in a separator
/// and does not carry a fragment suffix of its own.
fn is_fragment_base(base: &[u8]) -> bool {
    !base.is_empty()
        && base.last() != Some(&b'.')
        && fragment_marker(base, COLD_FRAGMENT).is_none()
        && fragment_marker(base, PART_FRAGMENT).is_none()
}

/// Decode the bare x86 `cdecl` decoration `_name`.
fn leading_underscore_name(input: &[u8]) -> Option<&[u8]> {
    let name = input.strip_prefix(b"_")?;
    if name.is_empty() || name.contains(&b'@') {
        return None;
    }
    Some(name)
}

fn is_msvc_name(input: &[u8]) -> bool {
    input.starts_with(b"?") || input.starts_with(b".?")
}

fn is_itanium_import(input: &[u8]) -> bool {
    let Some(rest) = input.strip_prefix(b"_Z") else {
        return false;
    };
    let Some(first) = rest.first() else {
        return false;
    };
    first.is_ascii_digit() || matches!(*first, b'N' | b'S' | b'T' | b'L' | b'Z')
}

fn is_rust_v0_import(input: &[u8]) -> bool {
    let Some(rest) = input.strip_prefix(b"_R") else {
        return false;
    };
    let Some(first) = rest.first() else {
        return false;
    };
    matches!(*first, b'C' | b'N' | b'M' | b'X' | b'Y' | b'I' | b'B')
}

/// Decode the symbol name that follows one or more `__imp_` prefixes.
///
/// A C import carries the x86 leading-underscore decoration (`__imp__foo`
/// imports `foo`), while MSVC C++ names carry none (`__imp_?foo@@...`).
/// 32-bit COFF also decorates MinGW Itanium and Rust names, so
/// `__imp___Z...` and `__imp___R...` leave the mangling underscore after one
/// decoration underscore is removed. A directly spelled `__imp__Z...` /
/// `__imp__R...` keeps the underscore only when the bytes after it start a
/// plausible Itanium or rustc v0 encoding; otherwise `__imp__Zone` would be
/// mistaken for `_Zone` instead of the C name `Zone`.
fn import_symbol(input: &[u8]) -> Option<&[u8]> {
    let symbol = if is_msvc_name(input) || is_itanium_import(input) || is_rust_v0_import(input) {
        input
    } else {
        input.strip_prefix(b"_").unwrap_or(input)
    };
    if symbol.is_empty() {
        return None;
    }
    // MSVC mangles `@` as a grammar separator; GNU versions and ThinLTO
    // suffixes use it as decoration and are not part of an imported name.
    if !is_msvc_name(symbol) && symbol.contains(&b'@') {
        return None;
    }
    Some(symbol)
}

fn parse_staplike(input: &[u8]) -> Option<(&[u8], CallingConvention, u16)> {
    if let Some(rest) = input.strip_prefix(b"@") {
        let (symbol, stack) = split_at_stack(rest)?;
        if symbol.is_empty() || symbol.contains(&b'@') {
            return None;
        }
        return Some((symbol, CallingConvention::Fastcall, stack));
    }
    let rest = input.strip_prefix(b"_")?;
    let (symbol, stack) = split_at_stack(rest)?;
    if symbol.is_empty() || symbol.contains(&b'@') {
        return None;
    }
    Some((symbol, CallingConvention::Stdcall, stack))
}

/// Decode the `name@@N` vectorcall decoration.
///
/// The name may not be empty, may not start with the x86 decoration
/// underscore (vectorcall replaces it) and may not contain `@`, so MSVC
/// `?name@@...` manglings and GNU versions are not claimed.
fn parse_vectorcall(input: &[u8]) -> Option<(&[u8], u16)> {
    let last = input.iter().rposition(|byte| *byte == b'@')?;
    let suffix = input.get(last + 1..)?;
    let name_end = last.checked_sub(1)?;
    if input.get(name_end) != Some(&b'@') {
        return None;
    }
    let name = input.get(..name_end)?;
    if name.is_empty() || name.starts_with(b"_") || name.contains(&b'@') {
        return None;
    }
    let stack = parse_decimal(suffix)?;
    Some((name, stack))
}

fn split_at_stack(input: &[u8]) -> Option<(&[u8], u16)> {
    let index = input.iter().rposition(|byte| *byte == b'@')?;
    let symbol = input.get(..index)?;
    let suffix = input.get(index + 1..)?;
    let stack = parse_decimal(suffix)?;
    Some((symbol, stack))
}

/// Parse a non-empty decimal `u16` without overflow.
fn parse_decimal(input: &[u8]) -> Option<u16> {
    if input.is_empty() || !input.iter().all(u8::is_ascii_digit) {
        return None;
    }
    input.iter().try_fold(0_u16, |value, byte| {
        value.checked_mul(10)?.checked_add(u16::from(byte - b'0'))
    })
}

#[cfg(test)]
mod tests;
