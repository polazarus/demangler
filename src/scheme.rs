/// The programming language that a mangled symbol belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Language {
    /// Plain C (no mangling).
    C,
    /// C++ (Itanium, GNU, MSVC).
    Cxx,
    /// Objective-C (Apple/Mach-O and GNUstep runtime metadata).
    ObjectiveC,
    /// Rust (rustc mangling).
    Rust,
    /// Swift (Swift mangling).
    Swift,
    /// Go linker names.
    Go,
    /// Unknown language.
    Unknown,
}

impl Language {
    /// Human-readable language name.
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::C => "C",
            Self::Cxx => "C++",
            Self::ObjectiveC => "Objective-C",
            Self::Rust => "Rust",
            Self::Swift => "Swift",
            Self::Go => "Go",
            Self::Unknown => "Unknown",
        }
    }
}

/// The symbol mangling scheme.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Scheme {
    /// Plain C (no mangling).
    C,
    /// C++ (Itanium ABI, GCC/Clang) or MSVC.
    Cpp,
    /// Objective-C runtime metadata and method display forms.
    ObjC,
    /// Rust mangled by rustc.
    Rust,
    /// Swift mangled.
    Swift,
    /// Go linker names.
    Go,
    /// Unknown or unsupported scheme.
    Unknown,
}

impl Scheme {
    /// Detect the mangling scheme from a symbol name.
    ///
    /// Prefix-based schemes are recognised after stripping the decorations a
    /// linker or symbol table may append: a ThinLTO `.llvm.<hex>` suffix and a
    /// GNU symbol-version `@`/`@@` suffix. Platform leading underscores are
    /// part of the prefix: `__Z` is Itanium C++ and `__R` is rustc v0, just
    /// like their single-underscore spellings.
    ///
    /// Itanium (`_Z`), Microsoft (`?`) and Swift have unambiguous prefixes, so
    /// those checks are a cheap prefix test. Objective-C metadata also has
    /// explicit prefixes (`_OBJC_`, `__OBJC_`, `OBJC_`, `__objc_`) and method
    /// display forms, so it is checked before Go and before the legacy C++
    /// dialects: dotted metadata such as `_OBJC_IVAR_$_Foo._ivar` looks like a
    /// package-qualified Go name, and legacy C++ has no unambiguous prefix.
    /// rustc v0 uses `_R`; `_RN` is only
    /// the common nested-path tag, not a special case. Legacy GNU v2 C++ has
    /// no version prefix — `foo__Fiv` is a name plus a `__`-separated
    /// signature — so a name that could be legacy is validated with a bounded
    /// parse. A plain C name is never reported as C++: the legacy parse has to
    /// consume the whole symbol.
    pub fn detect(name: &str) -> Self {
        if name.is_empty() {
            return Self::Unknown;
        }
        #[cfg(feature = "go")]
        let decorated = name;
        let name = strip_decorations(name);
        if name.is_empty() {
            return Self::Unknown;
        }

        // rustc v0 uses `_R`; `_RN` is only one of several tag bytes. A
        // platform leading underscore (`__R`) is equivalent. Legacy rustc
        // names share `_Z` with Itanium C++, so only the trailing crate hash
        // makes them unambiguous.
        #[cfg(feature = "rust")]
        if is_rust_v0_prefix(name) || is_rust_legacy_hash(name) {
            return Self::Rust;
        }

        // `_Z` and its underscored variants introduce Itanium C++ names.
        #[cfg(feature = "cpp-itanium")]
        if crate::cpp::is_itanium(name) {
            return Self::Cpp;
        }

        // Without an Itanium parser, `_ZN`/`__ZN` can only be rustc legacy.
        #[cfg(all(feature = "rust", not(feature = "cpp-itanium")))]
        if is_rust_legacy_prefix(name) {
            return Self::Rust;
        }

        #[cfg(feature = "swift")]
        if crate::swift::is_swift(name) {
            return Self::Swift;
        }

        // Microsoft C++ names start with `?`; the `.?` RTTI descriptor form is
        // also recognised by the parser.
        #[cfg(feature = "cpp-msvc")]
        if crate::cpp::is_msvc(name) {
            return Self::Cpp;
        }

        // Objective-C metadata has explicit prefixes, so it is safe to check
        // before the legacy C++ dialects, which have no unambiguous prefix and
        // would otherwise claim `__OBJC_$_...` as a C++ name. It is also
        // checked before Go: dotted metadata such as
        // `_OBJC_IVAR_$_Foo._ivar` looks like a package-qualified Go name.
        // Itanium and Objective-C++ `_Z` names were already handled above.
        #[cfg(feature = "objc")]
        if crate::objc::is_objc(name) {
            return Self::ObjC;
        }

        // A name in the Objective-C metadata namespace that is not a
        // recognised record is left unknown. The legacy C++ dialects have no
        // unambiguous prefix and would otherwise guess at it.
        #[cfg(feature = "objc")]
        if crate::objc::has_objc_namespace(name) {
            return Self::Unknown;
        }

        // Plain C identifiers cannot contain `.`, so a dotted name (or an
        // explicit Go prefix) is Go. `is_go_name` keeps other schemes'
        // prefixes, ambiguous names and `.llvm.`-decorated names out, so it
        // sees the original spelling rather than the decoration-stripped one.
        #[cfg(feature = "go")]
        if crate::go::is_go_name(decorated) {
            return Self::Go;
        }

        #[cfg(feature = "cpp-gnu-v2")]
        if crate::cpp::legacy::is_legacy_name(name) {
            return Self::Cpp;
        }

        if is_plain_c_name(name) {
            return Self::C;
        }

        Self::Unknown
    }

    /// Return the language this scheme demangles.
    pub const fn language(&self) -> Language {
        match self {
            Self::C => Language::C,
            Self::Cpp => Language::Cxx,
            Self::ObjC => Language::ObjectiveC,
            Self::Rust => Language::Rust,
            Self::Swift => Language::Swift,
            Self::Go => Language::Go,
            Self::Unknown => Language::Unknown,
        }
    }
}

/// Strip the decorations a linker may append to a mangled name.
///
/// A ThinLTO `.llvm.<hex>` suffix and a GNU symbol-version `@`/`@@` suffix are
/// not part of any mangling grammar. The demangler entry points strip the same
/// decorations before handing a symbol to a backend.
pub(crate) fn strip_decorations(name: &str) -> &str {
    strip_version_suffix(strip_thin_lto(name))
}

/// Strip a trailing `.llvm.<hex>` ThinLTO suffix.
///
/// The suffix is only stripped when the remainder is the hexadecimal hash
/// ThinLTO uses (`[0-9A-F@]`), matching the Rust backend's rule.
pub(crate) fn strip_thin_lto(name: &str) -> &str {
    const LLVM: &str = ".llvm.";
    let Some(index) = name.find(LLVM) else {
        return name;
    };
    let Some(candidate) = name.get(index + LLVM.len()..) else {
        return name;
    };
    if candidate
        .chars()
        .all(|c| matches!(c, '0'..='9' | 'A'..='F' | '@'))
    {
        name.get(..index).unwrap_or(name)
    } else {
        name
    }
}

/// Strip a GNU symbol-version `@`/`@@` suffix.
///
/// A `@` that follows a `.` belongs to a vendor suffix (`.0@1`) and is kept.
/// Microsoft names use `@` as a grammar separator, so the MSVC entry point
/// does not call this.
pub(crate) fn strip_version_suffix(name: &str) -> &str {
    let Some(index) = name.find('@') else {
        return name;
    };
    let prefix = name.get(..index).unwrap_or(name);
    if prefix.contains('.') {
        return name;
    }
    prefix
}

/// True for a rustc v0 `_R`/`__R` prefix with at least one tag byte.
#[cfg(feature = "rust")]
fn is_rust_v0_prefix(name: &str) -> bool {
    (name.len() > 2 && name.starts_with("_R")) || (name.len() > 3 && name.starts_with("__R"))
}

/// True for a rustc legacy `_ZN`/`__ZN` prefix.
#[cfg(all(feature = "rust", not(feature = "cpp-itanium")))]
fn is_rust_legacy_prefix(name: &str) -> bool {
    name.len() > 3 && (name.starts_with("_ZN") || name.starts_with("__ZN"))
}

/// True for a legacy rustc symbol with the trailing 16-hex-digit crate hash.
///
/// The legacy prefix is shared with Itanium C++ names, so only the hash makes
/// the Rust encoding unambiguous. `name` is expected to be decoration-free.
#[cfg(feature = "rust")]
fn is_rust_legacy_hash(name: &str) -> bool {
    let inner = name
        .strip_prefix("__ZN")
        .or_else(|| name.strip_prefix("_ZN"))
        .or_else(|| name.strip_prefix("ZN"));
    let Some(inner) = inner else {
        return false;
    };
    let Some(path) = inner.strip_suffix('E') else {
        return false;
    };
    let Some(hash) = path.get(path.len().saturating_sub(19)..) else {
        return false;
    };
    hash.strip_prefix("17h").is_some_and(|digits| {
        digits.len() == 16 && digits.bytes().all(|byte| byte.is_ascii_hexdigit())
    })
}

/// Heuristic: plain C symbols look like ordinary identifiers.
#[allow(clippy::single_call_fn)]
fn is_plain_c_name(name: &str) -> bool {
    !name.is_empty()
        && !name.starts_with('_')
        && !name.starts_with('$')
        && !name.starts_with('?')
        && !name.chars().all(|c| c.is_ascii_digit())
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}
