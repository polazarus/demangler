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
    /// Kotlin/Native linker symbols.
    KotlinNative,
    /// D (DMD/LDC/GDC ABI).
    D,
    /// Java Native Interface (JNI) native-method names.
    Java,
    /// Ada (GNAT external names).
    Ada,
    /// Fortran external names.
    Fortran,
    /// Delphi/Object Pascal external names.
    Delphi,
    /// OCaml external names.
    OCaml,
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
            Self::KotlinNative => "Kotlin/Native",
            Self::D => "D",
            Self::Java => "Java",
            Self::Ada => "Ada",
            Self::Fortran => "Fortran",
            Self::Delphi => "Delphi/Object Pascal",
            Self::OCaml => "OCaml",
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
    /// Kotlin/Native linker symbols (`kfun:`, `ktype:`, `kclass:`, …).
    KotlinNative,
    /// D (DMD/LDC/GDC ABI).
    D,
    /// Java Native Interface native-method names.
    Jni,
    /// Ada/GNAT external names.
    Ada,
    /// Fortran external names.
    Fortran,
    /// Delphi/Object Pascal external names.
    Delphi,
    /// OCaml external names.
    OCaml,
    /// Unknown or unsupported scheme.
    Unknown,
}

impl Scheme {
    /// Detect a raw symbol byte string.
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
    /// Detection uses byte prefixes, so non-UTF-8 identifier bytes do not
    /// prevent a recognized scheme from being returned.
    #[must_use]
    pub fn detect_bytes(name: &[u8]) -> Self {
        if name.is_empty() {
            return Self::Unknown;
        }
        // COFF decorations are orthogonal and may contain `@`, so they must
        // be decoded before the GNU version-suffix stripper sees the name.
        #[cfg(feature = "coff")]
        if let Some(decoration) = crate::coff::parse_bytes(name) {
            return Self::detect_bytes(decoration.symbol());
        }
        #[cfg(feature = "go")]
        let decorated = name;
        let name = strip_decorations_bytes(name);
        if name.is_empty() {
            return Self::Unknown;
        }

        #[cfg(feature = "rust")]
        if crate::rust::is_v0_prefix_bytes(name)
            || crate::rust::is_legacy_hash_bytes(name)
        {
            return Self::Rust;
        }

        #[cfg(feature = "d")]
        if crate::d::is_d(name) {
            return Self::D;
        }

        #[cfg(feature = "cpp-itanium")]
        if crate::cpp::is_itanium_bytes(name) {
            return Self::Cpp;
        }

        #[cfg(all(feature = "rust", not(feature = "cpp-itanium")))]
        if is_rust_legacy_prefix_bytes(name) {
            return Self::Rust;
        }

        #[cfg(feature = "swift")]
        if crate::swift::prefix_len_bytes(name).is_some() {
            return Self::Swift;
        }

        #[cfg(feature = "cpp-msvc")]
        if crate::cpp::is_msvc_bytes(name) {
            return Self::Cpp;
        }

        #[cfg(feature = "objc")]
        if crate::objc::is_objc_bytes(name) {
            return Self::ObjC;
        }

        #[cfg(feature = "objc")]
        if has_objc_namespace_bytes(name) {
            return Self::Unknown;
        }

        #[cfg(feature = "kotlin-native")]
        if crate::kotlin_native::is_kotlin_native_bytes(name) {
            return Self::KotlinNative;
        }

        #[cfg(feature = "delphi")]
        if crate::delphi::is_delphi(name) {
            return Self::Delphi;
        }

        #[cfg(feature = "ocaml")]
        if crate::ocaml::is_ocaml(name) {
            return Self::OCaml;
        }

        #[cfg(feature = "go")]
        if crate::go::is_go_name_bytes(decorated) {
            return Self::Go;
        }

        #[cfg(feature = "jni")]
        if crate::jni::is_jni(name) {
            return Self::Jni;
        }

        #[cfg(feature = "ada")]
        if crate::ada::is_ada(name) {
            return Self::Ada;
        }

        #[cfg(feature = "fortran")]
        if crate::fortran::is_fortran(name) {
            return Self::Fortran;
        }

        #[cfg(feature = "cpp-gnu-v2")]
        if crate::cpp::legacy::is_legacy_name_bytes(name) {
            return Self::Cpp;
        }

        if is_plain_c_name(name) {
            return Self::C;
        }

        Self::Unknown
    }

    /// Detect the mangling scheme from a UTF-8 symbol name.
    ///
    /// UTF-8 wrapper around [`Self::detect_bytes`]; the same precedence and
    /// full-parse rules apply.
    pub fn detect(name: &str) -> Self {
        Self::detect_bytes(name.as_bytes())
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
            Self::KotlinNative => Language::KotlinNative,
            Self::D => Language::D,
            Self::Jni => Language::Java,
            Self::Ada => Language::Ada,
            Self::Fortran => Language::Fortran,
            Self::Delphi => Language::Delphi,
            Self::OCaml => Language::OCaml,
            Self::Unknown => Language::Unknown,
        }
    }
}

#[cfg(all(feature = "rust", not(feature = "cpp-itanium")))]
fn is_rust_legacy_prefix_bytes(name: &[u8]) -> bool {
    name.len() > 3 && (name.starts_with(b"_ZN") || name.starts_with(b"__ZN"))
}

#[cfg(feature = "objc")]
fn has_objc_namespace_bytes(name: &[u8]) -> bool {
    name.starts_with(b"_OBJC_")
        || name.starts_with(b"__OBJC_")
        || name.starts_with(b"__objc_")
}

/// True when `name` is a plain C identifier: non-empty ASCII alphanumerics and
/// underscores that do not start with `_`, `$` or `?` and are not all digits.
///
/// Detection uses it for the C pass-through, and the Mach-O
/// leading-underscore entry point uses it to decide whether a stripped name is
/// still a C symbol.
pub(crate) fn is_plain_c_name(name: &[u8]) -> bool {
    !name.is_empty()
        && !name.starts_with(b"_")
        && !name.starts_with(b"$")
        && !name.starts_with(b"?")
        && !name.iter().all(u8::is_ascii_digit)
        && name
            .iter()
            .all(|byte| byte.is_ascii_alphanumeric() || *byte == b'_')
}

/// Strip a trailing `.llvm.<hex>` ThinLTO suffix from a byte string.
pub(crate) fn strip_thin_lto_bytes(name: &[u8]) -> &[u8] {
    const LLVM: &[u8] = b".llvm.";
    let Some(index) = find_bytes(name, LLVM) else {
        return name;
    };
    let Some(candidate) = name.get(index + LLVM.len()..) else {
        return name;
    };
    if !candidate.is_empty()
        && candidate
            .iter()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'A'..=b'F' | b'@'))
    {
        name.get(..index).unwrap_or(name)
    } else {
        name
    }
}

/// Strip a GNU symbol-version `@`/`@@` suffix from a byte string.
pub(crate) fn strip_version_suffix_bytes(name: &[u8]) -> &[u8] {
    let Some(index) = name.iter().position(|byte| *byte == b'@') else {
        return name;
    };
    let prefix = name.get(..index).unwrap_or(name);
    if prefix.ends_with(b".") {
        return name;
    }
    prefix
}

pub(crate) fn strip_decorations_bytes(name: &[u8]) -> &[u8] {
    strip_version_suffix_bytes(strip_thin_lto_bytes(name))
}

fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|window| window == needle)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_language_has_a_stable_name() {
        for (language, name) in [
            (Language::C, "C"),
            (Language::Cxx, "C++"),
            (Language::ObjectiveC, "Objective-C"),
            (Language::Rust, "Rust"),
            (Language::Swift, "Swift"),
            (Language::Go, "Go"),
            (Language::KotlinNative, "Kotlin/Native"),
            (Language::D, "D"),
            (Language::Java, "Java"),
            (Language::Ada, "Ada"),
            (Language::Fortran, "Fortran"),
            (Language::Delphi, "Delphi/Object Pascal"),
            (Language::OCaml, "OCaml"),
            (Language::Unknown, "Unknown"),
        ] {
            assert_eq!(language.as_str(), name);
        }
    }

    #[test]
    fn every_scheme_maps_to_its_language() {
        for (scheme, language) in [
            (Scheme::C, Language::C),
            (Scheme::Cpp, Language::Cxx),
            (Scheme::ObjC, Language::ObjectiveC),
            (Scheme::Rust, Language::Rust),
            (Scheme::Swift, Language::Swift),
            (Scheme::Go, Language::Go),
            (Scheme::KotlinNative, Language::KotlinNative),
            (Scheme::D, Language::D),
            (Scheme::Jni, Language::Java),
            (Scheme::Ada, Language::Ada),
            (Scheme::Fortran, Language::Fortran),
            (Scheme::Delphi, Language::Delphi),
            (Scheme::OCaml, Language::OCaml),
            (Scheme::Unknown, Language::Unknown),
        ] {
            assert_eq!(scheme.language(), language);
        }
    }

    #[test]
    fn strips_only_complete_linker_suffixes() {
        assert_eq!(
            strip_thin_lto_bytes(b"_Z3foov.llvm.9D1C9369"),
            b"_Z3foov"
        );
        assert_eq!(
            strip_thin_lto_bytes(b"_Z3foov.llvm.9D1C9369@@16"),
            b"_Z3foov"
        );
        assert_eq!(
            strip_thin_lto_bytes(b"_Z3foov.llvm.lowercase"),
            b"_Z3foov.llvm.lowercase"
        );
        assert_eq!(
            strip_thin_lto_bytes(b"_Z3foov.llvm."),
            b"_Z3foov.llvm."
        );
        assert_eq!(strip_thin_lto_bytes(b"_Z3foov"), b"_Z3foov");

        assert_eq!(strip_version_suffix_bytes(b"main@@GLIBC_2.2.5"), b"main");
        assert_eq!(strip_version_suffix_bytes(b"foo@1"), b"foo");
        assert_eq!(
            strip_version_suffix_bytes(b"main.@GLIBC"),
            b"main.@GLIBC"
        );
    }
}
