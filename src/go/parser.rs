//! Bounded parser for Go linker symbol names.
//!
//! The grammar is the linker's own name layout, documented in `refs/go.md`;
//! the parser never allocates beyond the input-proportional suffix list and
//! rejects anything it cannot classify instead of guessing.
//!
//! The grammar is matched on bytes through the shared [`Cursor`]: a symbol name
//! is a byte string, so the parser never requires the whole input to be UTF-8.
//! Text is interpreted only when it is printed (see [`super::text`]).

use alloc::vec::Vec;

use super::node::{
    Abi, Constant, ConstantClass, DataName, Function, HARD_DEPTH, Local, Name, Package,
    ParseLimits, Receiver, Special, Suffix, Symbol, TypeDescriptor, TypeKind,
};
use super::{Ast, ParseError};
use crate::parse::{Cursor, Fail, FailKind, State, find, rfind};

/// Compiler data symbol prefixes (`gcbits.0100`, `gclocals.<base64>`, …).
const DATA_PREFIXES: [&[u8]; 4] = [b"gcbits", b"gclocals", b"gcargs", b"gcbss"];

/// Compiler-internal function suffixes that carry digits or a `<n>` index.
const INTERNAL_SUFFIXES: [&[u8]; 7] = [
    b"arginfo",
    b"argliveinfo",
    b"args_stackmap",
    b"stkobj",
    b"opendefer",
    b"wrapinfo",
    b"spill",
];

/// The UTF-8 encoding of `·`, the separator Go assembly uses.
const MIDDLE_DOT: &[u8] = "·".as_bytes();

/// The byte length of `·`.
const MIDDLE_DOT_LEN: usize = MIDDLE_DOT.len();

/// Parse a Go symbol name from raw bytes within `limits`.
pub fn parse_bytes(symbol: &[u8], limits: ParseLimits) -> Result<Ast<'_>, ParseError> {
    let mut state = State::new(limits.cursor());
    let mut cursor = Cursor::new(symbol, 0, &mut state).map_err(map_fail)?;
    let (name, abi, version) = parse_name(symbol, &mut cursor)?;
    Ok(Ast { name, abi, version })
}

/// Translate a shared-cursor failure into this backend's public error.
const fn map_fail(fail: Fail) -> ParseError {
    match fail.kind {
        FailKind::Depth => ParseError::RecursedTooDeep,
        FailKind::InputBudget | FailKind::Work => ParseError::BudgetExceeded,
        _ => ParseError::Invalid,
    }
}

/// Charge `units` of work against the shared budget without moving the cursor.
///
/// The shared cursor is the only work meter the combinator layer exposes, so a
/// scan that works on a slice charges by advancing over it and rewinding.
fn charge(cursor: &mut Cursor<'_, '_>, units: usize) -> Result<(), ParseError> {
    let checkpoint = cursor.checkpoint();
    let _ = cursor.advance(units).map_err(map_fail)?;
    cursor.reset(checkpoint);
    Ok(())
}

/// A classified name with its stripped ABI marker and symbol version.
type ParsedName<'a> = (Name<'a>, Option<Abi>, Option<&'a [u8]>);

/// Classify a whole name, returning it with any stripped ABI marker and
/// object-file symbol version.
fn parse_name<'a>(
    symbol: &'a [u8],
    cursor: &mut Cursor<'_, '_>,
) -> Result<ParsedName<'a>, ParseError> {
    if symbol.is_empty() {
        return Err(ParseError::Invalid);
    }

    if let Some(rest) = symbol.strip_prefix(b"type:") {
        return Ok((
            Name::TypeDescriptor(parse_type_descriptor(rest)?),
            None,
            None,
        ));
    }
    if let Some(rest) = symbol.strip_prefix(b"go:") {
        return Ok((Name::Special(parse_special(rest, cursor)?), None, None));
    }
    if let Some(rest) = symbol.strip_prefix(b"go.shape.") {
        if rest.is_empty() {
            return Err(ParseError::Invalid);
        }
        return Ok((Name::Shape(rest), None, None));
    }
    if let Some(rest) = symbol.strip_prefix(b"$") {
        return Ok((Name::Constant(parse_constant(rest)?), None, None));
    }

    // The object-file version marker is appended by `go tool nm` after the
    // whole name, so strip it before splitting the package and local parts.
    let (symbol, version) = split_version(symbol);

    // `gclocals·<hash>` and friends are package-less compiler data symbols.
    // Recognize them before package splitting, so the printed `gclocals.<hash>`
    // form still parses as one symbol.
    if let Some(data) = parse_data(symbol) {
        return Ok((
            Name::Symbol(Symbol {
                package: None,
                local: Local::Data(data),
            }),
            None,
            version,
        ));
    }

    let (body, abi) = split_abi(symbol);
    let (package, local) = split_package(body);
    if local.is_empty() {
        return Err(ParseError::Invalid);
    }
    let package = if package.is_empty() {
        None
    } else {
        Some(Package::new(package))
    };
    let local = parse_local(local, cursor)?;
    Ok((Name::Symbol(Symbol { package, local }), abi, version))
}

/// Split the object-file symbol version (`<N>`) appended to static symbols.
///
/// `cmd/internal/objfile`'s `goobjName` appends `<1>` to every static symbol
/// read from an object file or export archive. The marker is not part of the
/// Go name; it is kept separately so printing can reproduce the tool's output.
fn split_version(name: &[u8]) -> (&[u8], Option<&[u8]>) {
    let Some(rest) = name.strip_suffix(b">") else {
        return (name, None);
    };
    let Some(open) = rfind(rest, b"<") else {
        return (name, None);
    };
    let version = &rest[open + 1..];
    let body = &name[..open];
    if body.is_empty() || version.is_empty() || !version.iter().all(u8::is_ascii_digit) {
        return (name, None);
    }
    (body, Some(version))
}

/// Strip one trailing ABI marker.
fn split_abi(name: &[u8]) -> (&[u8], Option<Abi>) {
    for abi in [Abi::Abi0, Abi::Internal, Abi::Wrap] {
        if let Some(body) = name.strip_suffix(abi.suffix().as_bytes())
            && !body.is_empty()
        {
            return (body, Some(abi));
        }
    }
    (name, None)
}

/// Split the escaped package path from the local name.
///
/// This mirrors `ir.splitPkg` in the Go compiler: the package path ends at the
/// first `.` after the last `/` that precedes any byte illegal in an import
/// path. Only ASCII `.` separates a package from its local name; `·` is a name
/// component (see [`parse_suffix`]).
pub fn split_package(name: &[u8]) -> (&[u8], &[u8]) {
    let bytes = name;
    let mut last_slash = 0;
    let mut i = 0;
    while i < bytes.len() {
        let byte = bytes[i];
        if byte == b'/' {
            last_slash = i;
        } else if !is_import_path_byte(byte) {
            break;
        }
        i += 1;
    }

    let mut j = last_slash;
    while j < bytes.len() {
        if bytes[j] == b'.' {
            return (&name[..j], &name[j + 1..]);
        }
        j += 1;
    }
    (&[], name)
}

/// Bytes allowed in a `PathToPrefix`-escaped import path.
pub const fn is_import_path_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~' | b'+' | b'/' | b'%')
}

/// Parse a `type:` descriptor.
fn parse_type_descriptor(rest: &[u8]) -> Result<TypeDescriptor<'_>, ParseError> {
    if rest.is_empty() {
        return Err(ParseError::Invalid);
    }
    let (kind, payload) =
        if let Some(payload) = rest.strip_prefix(TypeKind::Pointer.prefix().as_bytes()) {
            (TypeKind::Pointer, payload)
        } else if let Some(payload) = rest.strip_prefix(TypeKind::Equal.prefix().as_bytes()) {
            (TypeKind::Equal, payload)
        } else if let Some(payload) = rest.strip_prefix(TypeKind::EqualFunc.prefix().as_bytes()) {
            (TypeKind::EqualFunc, payload)
        } else if let Some(payload) = rest.strip_prefix(TypeKind::Hash.prefix().as_bytes()) {
            (TypeKind::Hash, payload)
        } else if let Some(payload) = rest.strip_prefix(TypeKind::HashFunc.prefix().as_bytes()) {
            (TypeKind::HashFunc, payload)
        } else if let Some(payload) = rest.strip_prefix(TypeKind::GcProg.prefix().as_bytes()) {
            (TypeKind::GcProg, payload)
        } else if let Some(payload) = rest.strip_prefix(TypeKind::NameData.prefix().as_bytes()) {
            (TypeKind::NameData, payload)
        } else if let Some(payload) = rest.strip_prefix(TypeKind::ImportPath.prefix().as_bytes()) {
            (TypeKind::ImportPath, payload)
        } else if let Some(payload) = rest.strip_prefix(TypeKind::NoAlg.prefix().as_bytes()) {
            (TypeKind::NoAlg, payload)
        } else if rest.starts_with(b".") {
            // An unknown `.tag.` descriptor would be silently misread as a type
            // string; reject it instead.
            return Err(ParseError::Invalid);
        } else {
            (TypeKind::Plain, rest)
        };
    Ok(TypeDescriptor { kind, payload })
}

/// Parse a `go:` special symbol.
fn parse_special<'a>(
    rest: &'a [u8],
    cursor: &mut Cursor<'_, '_>,
) -> Result<Special<'a>, ParseError> {
    let (kind, payload) = find(rest, b".").map_or((rest, &[][..]), |index| {
        (&rest[..index], &rest[index + 1..])
    });
    if kind.is_empty() {
        return Err(ParseError::Invalid);
    }
    if kind == b"itab" {
        let (concrete, iface) = split_itab(payload, cursor)?;
        return Ok(Special::Itab { concrete, iface });
    }
    if kind == b"string" {
        if payload.is_empty() {
            return Err(ParseError::Invalid);
        }
        return Ok(Special::String { payload });
    }
    Ok(Special::Other { kind, payload })
}

/// Split `go:itab`'s payload at its first depth-zero comma.
fn split_itab<'a>(
    payload: &'a [u8],
    cursor: &mut Cursor<'_, '_>,
) -> Result<(&'a [u8], &'a [u8]), ParseError> {
    let mut depth = 0_usize;
    for (index, byte) in payload.iter().enumerate() {
        charge(cursor, 1)?;
        match byte {
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => depth = depth.saturating_sub(1),
            b',' if depth == 0 => {
                let (concrete, rest) = payload.split_at(index);
                let iface = rest.get(1..).unwrap_or(&[]);
                if concrete.is_empty() || iface.is_empty() {
                    return Err(ParseError::Invalid);
                }
                return Ok((concrete, iface));
            }
            _ => {}
        }
    }
    Err(ParseError::Invalid)
}

/// Parse a `$` compiler constant.
fn parse_constant(rest: &[u8]) -> Result<Constant<'_>, ParseError> {
    let (class, value) =
        if let Some(value) = rest.strip_prefix(ConstantClass::F32.prefix().as_bytes()) {
            (ConstantClass::F32, value)
        } else if let Some(value) = rest.strip_prefix(ConstantClass::F64.prefix().as_bytes()) {
            (ConstantClass::F64, value)
        } else if let Some(value) = rest.strip_prefix(ConstantClass::I32.prefix().as_bytes()) {
            (ConstantClass::I32, value)
        } else if let Some(value) = rest.strip_prefix(ConstantClass::I64.prefix().as_bytes()) {
            (ConstantClass::I64, value)
        } else if let Some(value) = rest.strip_prefix(ConstantClass::I128.prefix().as_bytes()) {
            (ConstantClass::I128, value)
        } else {
            return Err(ParseError::Invalid);
        };
    if value.is_empty()
        || !value
            .iter()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'+'))
    {
        return Err(ParseError::Invalid);
    }
    Ok(Constant { class, value })
}

/// Parse the local part of a symbol.
fn parse_local<'a>(local: &'a [u8], cursor: &mut Cursor<'_, '_>) -> Result<Local<'a>, ParseError> {
    // `pkg..inittask` splits at the first `.`, leaving `.inittask` as the
    // local part; the doubled dot is part of the compiler's naming.
    if let Some(rest) = local.strip_prefix(b".") {
        if rest == b"inittask" {
            return Ok(Local::InitTask);
        }
        if let Some(ty) = rest.strip_prefix(b"dict.")
            && !ty.is_empty()
        {
            return Ok(Local::Dictionary { ty });
        }
        if let Some(index) = rest.strip_prefix(b"interfaceSwitch.")
            && is_index(index)
        {
            return Ok(Local::InterfaceSwitch { index });
        }
        if let Some(index) = rest.strip_prefix(b"typeAssert.")
            && is_index(index)
        {
            return Ok(Local::TypeAssert { index });
        }
        if let Some(index) = rest.strip_prefix(b"gobytes.")
            && is_index(index)
        {
            return Ok(Local::GoBytes { index });
        }
        if let Some(index) = rest.strip_prefix(b"stmp_")
            && is_index(index)
        {
            return Ok(Local::StaticTemp { index });
        }
        return Err(ParseError::Invalid);
    }

    if let Some(rest) = local.strip_prefix(b"glob..") {
        if rest.is_empty()
            || !rest
                .iter()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_'))
        {
            return Err(ParseError::Invalid);
        }
        return Ok(Local::GlobalClosure { suffix: rest });
    }

    if let Some(data) = parse_data(local) {
        return Ok(Local::Data(data));
    }

    Ok(Local::Function(parse_function(local, cursor)?))
}

/// True for a non-empty run of ASCII digits.
fn is_index(index: &[u8]) -> bool {
    !index.is_empty() && index.iter().all(u8::is_ascii_digit)
}

/// Parse a compiler data symbol, if the local part has that shape.
fn parse_data(local: &[u8]) -> Option<DataName<'_>> {
    let end = ident_end(local, 0)?;
    let name = local.get(..end)?;
    if !DATA_PREFIXES.contains(&name) {
        return None;
    }
    let separator = dot_len(local, end)?;
    let payload = local.get(end + separator..)?;
    if !payload.iter().all(|byte| {
        byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'/' | b'=' | b'*' | b'_' | b'-')
    }) {
        return None;
    }
    Some(DataName { name, payload })
}

/// Parse a function, method, closure or wrapper.
fn parse_function<'a>(
    local: &'a [u8],
    cursor: &mut Cursor<'_, '_>,
) -> Result<Function<'a>, ParseError> {
    let mut pos = 0;
    let receiver = if local.starts_with(b"(") {
        let end = scan_balanced(local, 0, cursor)?;
        let inner = local.get(1..end - 1).ok_or(ParseError::Invalid)?;
        let (pointer, ty) = inner
            .strip_prefix(b"*")
            .map_or((false, inner), |ty| (true, ty));
        if ty.is_empty() {
            return Err(ParseError::Invalid);
        }
        let separator = dot_len(local, end).ok_or(ParseError::Invalid)?;
        pos = end + separator;
        Some(Receiver { pointer, ty })
    } else {
        None
    };

    let name_end = if receiver.is_some() {
        qualified_name_end(local, pos).or_else(|| ident_end(local, pos))
    } else {
        ident_end(local, pos)
    }
    .ok_or(ParseError::Invalid)?;
    let mut end = name_end;
    if local.get(end) == Some(&b'[') {
        end = scan_balanced(local, end, cursor)?;
    }
    let name = local.get(pos..end).ok_or(ParseError::Invalid)?;

    let mut suffixes = Vec::new();
    let mut next = end;
    while next < local.len() {
        let (suffix, after) = parse_suffix(local, next, cursor)?;
        suffixes.push(suffix);
        next = after;
    }

    Ok(Function {
        receiver,
        name,
        suffixes,
    })
}

/// Parse one suffix component starting at `pos`.
fn parse_suffix<'a>(
    local: &'a [u8],
    pos: usize,
    cursor: &mut Cursor<'_, '_>,
) -> Result<(Suffix<'a>, usize), ParseError> {
    charge(cursor, 1)?;

    if local.get(pos) == Some(&b'-') {
        let rest = local.get(pos + 1..).ok_or(ParseError::Invalid)?;
        if rest.starts_with(b"fm") {
            return Ok((Suffix::MethodValue, pos + 1 + b"fm".len()));
        }
        if rest.starts_with(b"range") {
            let start = pos + 1 + b"range".len();
            let end = digit_end(local, start);
            if end == start {
                return Err(ParseError::Invalid);
            }
            return Ok((Suffix::Range(local.get(start..end).unwrap_or(&[])), end));
        }
        return Err(ParseError::Invalid);
    }

    let separator = dot_len(local, pos).ok_or(ParseError::Invalid)?;
    let after = pos + separator;

    // `·f` is the function-value marker; the middle dot distinguishes it from
    // an inlined function named `f`.
    if separator == MIDDLE_DOT_LEN && local.get(after..) == Some(b"f") {
        return Ok((Suffix::FuncValue, local.len()));
    }

    for (prefix, kind) in [
        (b"func".as_slice(), SuffixKind::Closure),
        (b"gowrap".as_slice(), SuffixKind::GoWrap),
        (b"deferwrap".as_slice(), SuffixKind::DeferWrap),
    ] {
        if local
            .get(after..)
            .is_some_and(|rest| rest.starts_with(prefix))
        {
            let start = after + prefix.len();
            let end = digit_end(local, start);
            if end > start {
                let digits = local.get(start..end).unwrap_or(&[]);
                let suffix = match kind {
                    SuffixKind::Closure => Suffix::Closure(digits),
                    SuffixKind::GoWrap => Suffix::GoWrap(digits),
                    SuffixKind::DeferWrap => Suffix::DeferWrap(digits),
                };
                return Ok((suffix, end));
            }
        }
    }

    let run_end = component_end(local, after);
    let component = local.get(after..run_end).unwrap_or(&[]);
    if let Some(index) = component.strip_prefix(b"jump")
        && is_index(index)
    {
        return Ok((Suffix::JumpTable(index), run_end));
    }
    for prefix in INTERNAL_SUFFIXES {
        if let Some(tail) = component.strip_prefix(prefix)
            && tail
                .iter()
                .all(|byte| byte.is_ascii_digit() || matches!(byte, b'<' | b'>'))
        {
            return Ok((Suffix::Internal(component), run_end));
        }
    }

    let digits_end = digit_end(local, after);
    if digits_end > after {
        return Ok((
            Suffix::Nested(local.get(after..digits_end).unwrap_or(&[])),
            digits_end,
        ));
    }

    // An inlined component can itself be a method name when the inlined call
    // had a receiver: `(*sweepLocked).sweep.(*mheap).freeSpan.func3`.
    let mut end = if local.get(after) == Some(&b'(') {
        let receiver_end = scan_balanced(local, after, cursor)?;
        let separator = dot_len(local, receiver_end).ok_or(ParseError::Invalid)?;
        ident_end(local, receiver_end + separator).ok_or(ParseError::Invalid)?
    } else {
        ident_end(local, after).ok_or(ParseError::Invalid)?
    };
    if local.get(end) == Some(&b'[') {
        end = scan_balanced(local, end, cursor)?;
    }
    Ok((Suffix::Inlined(local.get(after..end).unwrap_or(&[])), end))
}

/// The three `func`/`gowrap`/`deferwrap` closure suffixes.
#[derive(Clone, Copy)]
enum SuffixKind {
    Closure,
    GoWrap,
    DeferWrap,
}

/// The byte length of a `.` or `·` separator at `pos`.
fn dot_len(bytes: &[u8], pos: usize) -> Option<usize> {
    if bytes.get(pos) == Some(&b'.') {
        Some(1)
    } else if bytes.get(pos..pos + MIDDLE_DOT_LEN) == Some(MIDDLE_DOT) {
        Some(MIDDLE_DOT_LEN)
    } else {
        None
    }
}

/// End of an identifier at `pos`, or `None` when there is none.
fn ident_end(bytes: &[u8], pos: usize) -> Option<usize> {
    let first = *bytes.get(pos)?;
    if !(first.is_ascii_alphabetic() || first == b'_') {
        return None;
    }
    let mut end = pos + 1;
    while bytes
        .get(end)
        .is_some_and(|byte| byte.is_ascii_alphanumeric() || *byte == b'_')
    {
        end += 1;
    }
    Some(end)
}

/// End of a package-qualified method name (`pkg/path.Method`) at `pos`.
///
/// `ir.MethodSymSuffix` prefixes an unexported method with its escaped package
/// path when the method comes from a different package than the receiver type,
/// producing `(*T).pkg/path.method`. `objabi.PathToPrefix` escapes every `.` in
/// the last path segment, so the first `.` after the last `/` separates the
/// path from the method name. A `/` cannot occur in an identifier, so its
/// presence distinguishes a qualifier from a plain method name followed by an
/// inlined-function suffix.
fn qualified_name_end(bytes: &[u8], pos: usize) -> Option<usize> {
    let mut last_slash = None;
    let mut i = pos;
    while i < bytes.len() && is_import_path_byte(bytes[i]) {
        if bytes[i] == b'/' {
            last_slash = Some(i);
        }
        i += 1;
    }
    let slash = last_slash?;
    let mut separator = slash;
    while bytes.get(separator).is_some_and(|byte| *byte != b'.') {
        if !is_import_path_byte(bytes[separator]) {
            return None;
        }
        separator += 1;
    }
    if bytes.get(separator) != Some(&b'.') {
        return None;
    }
    ident_end(bytes, separator + 1)
}

/// End of a run of ASCII digits at `pos` (equal to `pos` when there is none).
fn digit_end(bytes: &[u8], pos: usize) -> usize {
    let mut end = pos;
    while bytes.get(end).is_some_and(u8::is_ascii_digit) {
        end += 1;
    }
    end
}

/// End of a compiler-internal suffix component (`arginfo1<1>`, `stkobj`, …).
fn component_end(bytes: &[u8], pos: usize) -> usize {
    let mut end = pos;
    while bytes
        .get(end)
        .is_some_and(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'<' | b'>'))
    {
        end += 1;
    }
    end
}

/// End of the balanced group that starts at `start`.
///
/// The group is scanned iteratively, so the closer stack is sized for the hard
/// depth ceiling and the resolved budget is enforced against it.
fn scan_balanced(
    bytes: &[u8],
    start: usize,
    cursor: &mut Cursor<'_, '_>,
) -> Result<usize, ParseError> {
    let depth_limit = usize::from(cursor.depth_limit());
    let mut closers = [0_u8; HARD_DEPTH];
    let mut depth = 0;
    let mut i = start;
    while i < bytes.len() {
        charge(cursor, 1)?;
        match bytes[i] {
            opener @ (b'(' | b'[' | b'{') => {
                if depth >= depth_limit {
                    return Err(ParseError::RecursedTooDeep);
                }
                closers[depth] = match opener {
                    b'(' => b')',
                    b'[' => b']',
                    _ => b'}',
                };
                depth += 1;
            }
            closer @ (b')' | b']' | b'}') => {
                if depth == 0 || closers[depth - 1] != closer {
                    return Err(ParseError::Invalid);
                }
                depth -= 1;
                if depth == 0 {
                    return Ok(i + 1);
                }
            }
            _ => {}
        }
        i += 1;
    }
    Err(ParseError::Invalid)
}

#[cfg(test)]
mod tests {
    use alloc::string::{String, ToString};

    use super::*;
    use crate::go::{DEFAULT_BUDGET, MAX_DEPTH, MAX_INPUT_LEN, parse};

    /// Parse with the compiled-in budget, the way the public entry point does.
    fn parse_with(symbol: &[u8]) -> Result<Ast<'_>, ParseError> {
        parse_bytes(symbol, ParseLimits::from_effective(DEFAULT_BUDGET))
    }

    fn text(name: &str) -> String {
        parse(name).unwrap().to_string()
    }

    #[test]
    fn splits_package_and_function() {
        assert_eq!(text("fmt.Println"), "fmt.Println");
        assert_eq!(
            text("runtime.(*mLockProfile).captureStack"),
            "runtime.(*mLockProfile).captureStack"
        );
        assert_eq!(
            text("example.com/demangle/corpus/pkg%2ev1.Make"),
            "example.com/demangle/corpus/pkg.v1.Make"
        );
    }

    #[test]
    fn parses_closures_and_wrappers() {
        assert_eq!(text("main.main.func1.1"), "main.main.func1.1");
        assert_eq!(
            text("fmt.(*pp).handleMethods.deferwrap1"),
            "fmt.(*pp).handleMethods.deferwrap1"
        );
        assert_eq!(
            text("runtime.gcBgMarkStartWorkers.gowrap1"),
            "runtime.gcBgMarkStartWorkers.gowrap1"
        );
        assert_eq!(
            text("sync/atomic.(*Uint64).Load-fm"),
            "sync/atomic.(*Uint64).Load-fm"
        );
        assert_eq!(
            text("internal/godebug.update-range1"),
            "internal/godebug.update-range1"
        );
    }

    #[test]
    fn parses_inlined_components() {
        assert_eq!(
            text("example.com/x/lib.init.0.Counter.func1"),
            "example.com/x/lib.init.0.Counter.func1"
        );
        assert_eq!(
            text("runtime.(*sweepLocked).sweep.(*mheap).freeSpan.func3"),
            "runtime.(*sweepLocked).sweep.(*mheap).freeSpan.func3"
        );
        assert_eq!(text("runtime.memequal64·f"), "runtime.memequal64.f");
    }

    #[test]
    fn parses_generics_and_shapes() {
        assert_eq!(
            text("cmp.Compare[go.shape.int]"),
            "cmp.Compare[go.shape.int]"
        );
        assert_eq!(
            text("internal/sync.(*HashTrieMap[go.shape.interface {},go.shape.interface {}]).Load"),
            "internal/sync.(*HashTrieMap[go.shape.interface {},go.shape.interface {}]).Load"
        );
        assert_eq!(text("cmp..dict.Compare[int]"), "cmp..dict.Compare[int]");
        assert_eq!(text("go.shape.int"), "go.shape.int");
    }

    #[test]
    fn parses_compiler_symbols() {
        assert_eq!(text("main..inittask"), "main..inittask");
        assert_eq!(
            text("errors..interfaceSwitch.0"),
            "errors..interfaceSwitch.0"
        );
        assert_eq!(
            text("runtime.gclocals·+ISnmBhJ1y+frD3sAmLgtg=="),
            "runtime.gclocals.+ISnmBhJ1y+frD3sAmLgtg=="
        );
        assert_eq!(text("runtime.gcbits."), "runtime.gcbits.");
        assert_eq!(text("runtime.gcbits.*"), "runtime.gcbits.*");
        assert_eq!(
            text("example.com/x.(*Err).Error.arginfo1<1>"),
            "example.com/x.(*Err).Error.arginfo1<1>"
        );
    }

    #[test]
    fn parses_type_descriptors() {
        let parsed = parse("type:.eq.main.Pair[string,int]").unwrap();
        let Name::TypeDescriptor(descriptor) = parsed.name() else {
            panic!("not a type descriptor");
        };
        assert_eq!(descriptor.kind(), TypeKind::Equal);
        assert_eq!(descriptor.payload(), "main.Pair[string,int]");

        let parsed = parse("type:*").unwrap();
        let Name::TypeDescriptor(descriptor) = parsed.name() else {
            panic!("not a type descriptor");
        };
        assert_eq!(descriptor.kind(), TypeKind::Pointer);
        assert_eq!(descriptor.payload(), "");

        let parsed = parse("type:.importpath.example.com/x.").unwrap();
        let Name::TypeDescriptor(descriptor) = parsed.name() else {
            panic!("not a type descriptor");
        };
        assert_eq!(descriptor.kind(), TypeKind::ImportPath);
        assert_eq!(descriptor.payload(), "example.com/x.");

        assert_eq!(
            text("type:.namedata.*func() int-"),
            "type:.namedata.*func() int-"
        );
    }

    #[test]
    fn parses_equality_and_hash_closures() {
        for (name, kind, payload) in [
            ("type:.eqfunc96", TypeKind::EqualFunc, "96"),
            (
                "type:.eqfunc.go/types.importKey",
                TypeKind::EqualFunc,
                ".go/types.importKey",
            ),
            (
                "type:.hashfunc.go/types.importKey",
                TypeKind::HashFunc,
                ".go/types.importKey",
            ),
        ] {
            let parsed = parse(name).unwrap();
            let Name::TypeDescriptor(descriptor) = parsed.name() else {
                panic!("not a type descriptor: {name}");
            };
            assert_eq!(descriptor.kind(), kind);
            assert_eq!(descriptor.payload(), payload);
            assert_eq!(parsed.to_string(), name);
        }
    }

    #[test]
    fn parses_static_temps_and_jump_tables() {
        let parsed = parse("example.com/x..stmp_31<1>").unwrap();
        assert_eq!(parsed.version().as_deref(), Some("1"));
        let Name::Symbol(symbol) = parsed.name() else {
            panic!("not a symbol");
        };
        let Local::StaticTemp { index } = symbol.local() else {
            panic!("not a static temp");
        };
        assert_eq!(index, b"31");
        assert_eq!(parsed.to_string(), "example.com/x..stmp_31<1>");

        let parsed = parse("fmt.(*pp).printArg.jump41<1>").unwrap();
        let Name::Symbol(symbol) = parsed.name() else {
            panic!("not a symbol");
        };
        let Local::Function(function) = symbol.local() else {
            panic!("not a function");
        };
        assert!(matches!(function.suffixes(), [Suffix::JumpTable(b"41")]));
        assert_eq!(parsed.to_string(), "fmt.(*pp).printArg.jump41<1>");
    }

    #[test]
    fn parses_package_qualified_methods() {
        let name = "text/template.(*Template).text/template/parse.action.arginfo1<1>";
        assert_eq!(text(name), name);

        let parsed = parse("regexp.(*onePassInst).regexp/syntax.op").unwrap();
        let Name::Symbol(symbol) = parsed.name() else {
            panic!("not a symbol");
        };
        let Local::Function(function) = symbol.local() else {
            panic!("not a function");
        };
        assert_eq!(function.name(), "regexp/syntax.op");
        assert_eq!(function.receiver().unwrap().ty, b"onePassInst");
    }

    #[test]
    fn splits_object_file_versions() {
        assert_eq!(parse("fmt.Println").unwrap().version(), None);
        let parsed = parse("fmt.Println<1>").unwrap();
        assert_eq!(parsed.version().as_deref(), Some("1"));
        assert_eq!(parsed.to_string(), "fmt.Println<1>");
        assert!(parse("fmt.Println<x>").is_err());
        assert!(parse("fmt.Println<>").is_err());
    }

    #[test]
    fn parses_specials() {
        let parsed = parse("go:itab.*os.File,io.Writer").unwrap();
        let Name::Special(Special::Itab { concrete, iface }) = parsed.name() else {
            panic!("not an itab");
        };
        assert_eq!(concrete, b"*os.File");
        assert_eq!(iface, b"io.Writer");

        let parsed = parse("go:string.\"100%\"").unwrap();
        let Name::Special(Special::String { payload }) = parsed.name() else {
            panic!("not a string symbol");
        };
        assert_eq!(payload, b"\"100%\"");

        let parsed = parse("go:buildid").unwrap();
        let Name::Special(Special::Other { kind, payload }) = parsed.name() else {
            panic!("not a special");
        };
        assert_eq!(kind, b"buildid");
        assert_eq!(payload, b"");
    }

    #[test]
    fn strips_abi_markers() {
        assert_eq!(
            parse("runtime.memmove.abi0").unwrap().abi(),
            Some(Abi::Abi0)
        );
        assert_eq!(
            parse("runtime.memmove.abiinternal").unwrap().abi(),
            Some(Abi::Internal)
        );
        assert_eq!(
            parse("runtime.memmove.abiwrap").unwrap().abi(),
            Some(Abi::Wrap)
        );
        assert_eq!(parse("runtime.memmove").unwrap().abi(), None);
        assert_eq!(text("runtime.memmove.abi0"), "runtime.memmove");
    }

    #[test]
    fn parses_constants() {
        let parsed = parse("$f64.3ff0000000000000").unwrap();
        let Name::Constant(constant) = parsed.name() else {
            panic!("not a constant");
        };
        assert_eq!(constant.class(), ConstantClass::F64);
        assert_eq!(constant.value(), "3ff0000000000000");
        assert_eq!(text("$i64.0000000000000001"), "$i64.0000000000000001");
    }

    #[test]
    fn rejects_malformed_names() {
        for name in [
            "",
            "type:",
            "type:.unknown.x",
            "go:",
            "go:itab.no-comma",
            "go:string.",
            "go.shape.",
            "$nope",
            "fmt.",
            "fmt.Println(",
            "fmt.Println[",
            "fmt.Println[go.shape.int",
            "fmt.Println-unknown",
            "main..nope",
            "main.(*T).",
            "main.()",
            "main.123",
        ] {
            assert!(parse(name).is_err(), "{name}");
        }
    }

    #[test]
    fn input_budget_is_enforced() {
        let mut name = String::from("fmt.");
        name.push_str(&"a".repeat(MAX_INPUT_LEN));
        assert_eq!(parse(&name).unwrap_err(), ParseError::BudgetExceeded);
    }

    #[test]
    fn depth_budget_is_enforced() {
        let mut name = String::from("pkg.F");
        name.push_str(&"[".repeat(MAX_DEPTH + 1));
        name.push_str(&"]".repeat(MAX_DEPTH + 1));
        assert_eq!(parse(&name).unwrap_err(), ParseError::RecursedTooDeep);
    }

    #[test]
    fn suffix_lists_are_bounded_by_the_input_budget() {
        let name = alloc::format!("pkg.F{}", ".func1".repeat(1024));
        assert!(parse(&name).is_ok());
    }

    #[test]
    fn rejects_unbalanced_groups() {
        assert!(parse("pkg.F[)]").is_err());
        assert!(parse("pkg.F[go.shape.struct { x int })").is_err());
    }

    #[test]
    fn middle_dot_separators_are_accepted() {
        assert_eq!(text("runtime·memmove"), "runtime.memmove");
    }

    #[test]
    fn package_paths_are_decoded() {
        let parsed = parse("gopkg.in/yaml%2ev3.F[go.shape.int]").unwrap();
        let Name::Symbol(symbol) = parsed.name() else {
            panic!("not a symbol");
        };
        let package = symbol.package().unwrap();
        assert_eq!(package.raw(), "gopkg.in/yaml%2ev3");
        assert_eq!(package.decoded().to_string(), "gopkg.in/yaml.v3");
        assert_eq!(parsed.to_string(), "gopkg.in/yaml.v3.F[go.shape.int]");
    }

    #[test]
    fn byte_path_matches_the_text_path() {
        for name in [
            "fmt.Println",
            "runtime.(*mLockProfile).captureStack",
            "example.com/demangle/corpus/pkg%2ev1.Make",
            "type:.eq.main.Pair[string,int]",
            "go:itab.*os.File,io.Writer",
            "go:string.\"100%\"",
            "runtime.gclocals·+ISnmBhJ1y+frD3sAmLgtg==",
            "text/template.(*Template).text/template/parse.action.arginfo1<1>",
            "$f64.3ff0000000000000",
        ] {
            assert_eq!(
                parse_with(name.as_bytes()).unwrap().to_string(),
                parse(name).unwrap().to_string(),
                "{name}"
            );
        }
    }

    #[test]
    fn byte_path_replays_the_corpus() {
        for path in [
            "refs/go/corpus/go1.24.4-binary.txt",
            "refs/go/corpus/go1.24.4-object.txt",
            "refs/go/corpus/go1.24.4-archive.txt",
        ] {
            let path = std::format!("{}/{path}", std::env!("CARGO_MANIFEST_DIR"));
            let Ok(data) = std::fs::read_to_string(&path) else {
                continue;
            };
            for line in data.lines() {
                let name = line.trim_end();
                if name.is_empty() {
                    continue;
                }
                let Ok(text) = parse(name) else {
                    continue;
                };
                let bytes = parse_with(name.as_bytes()).unwrap_or_else(|error| {
                    panic!("{name}: byte path failed: {error}");
                });
                assert_eq!(bytes.to_string(), text.to_string(), "{name}");
            }
        }
    }

    #[test]
    fn invalid_utf8_text_is_escaped() {
        let ast = parse_with(b"type:.eq.main.F\xffoo").unwrap();
        assert_eq!(ast.to_string(), "type:.eq.main.F\\xffoo");
        // A decoded `%XX` escape that is not UTF-8 is escaped, not re-encoded.
        let ast = parse_with(b"type:.eq.example.com/x%2ff%ff.T").unwrap();
        assert_eq!(ast.to_string(), "type:.eq.example.com/x/f\\xff.T");
        // Opaque payloads keep their own quoting and backslashes.
        let ast = parse_with(b"go:string.\"a\\b\"").unwrap();
        assert_eq!(ast.to_string(), "go:string.\"a\\b\"");
    }

    #[test]
    fn valid_unicode_text_stays_verbatim() {
        let ast = parse_with("runtime.memequal64·f".as_bytes()).unwrap();
        assert_eq!(ast.to_string(), "runtime.memequal64.f");
        let ast = parse_with("type:.eq.pkg.Föö".as_bytes()).unwrap();
        assert_eq!(ast.to_string(), "type:.eq.pkg.Föö");
    }

    #[test]
    fn suffix_lists_are_reported() {
        let parsed = parse("pkg.F.func1.func2.func3").unwrap();
        let Name::Symbol(symbol) = parsed.name() else {
            panic!("not a symbol");
        };
        let Local::Function(function) = symbol.local() else {
            panic!("not a function");
        };
        assert_eq!(function.suffixes().len(), 3);
        assert!(matches!(function.suffixes()[0], Suffix::Closure(b"1")));
    }
}
