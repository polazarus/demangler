/// The programming language that a mangled symbol belongs to.
///
/// A language groups one or more [`Scheme`]s: [`Language::Cpp`] covers the six
/// C++ ABIs and dialects, [`Language::Rust`] the two rustc encodings and
/// [`Language::Swift`] the two Swift manglings. [`Scheme::detect`] reports the
/// precise scheme; [`Scheme::language`] and [`Language::schemes`] convert
/// between the two views.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Language {
    /// Plain C (no mangling).
    C,
    /// C++: Itanium, MSVC and the legacy GNU v2, ARM, EDG and HP aCC dialects.
    Cpp,
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
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::C => "C",
            Self::Cpp => "C++",
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

    /// Every scheme of this language, in canonical order.
    ///
    /// The C++ group is ordered Itanium, MSVC, GNU v2, ARM, EDG and HP aCC.
    /// Schemes whose backend feature is disabled stay in the group: their
    /// candidates simply fail and the caller falls through to the next one.
    #[must_use]
    pub const fn schemes(&self) -> &'static [Scheme] {
        match self {
            Self::C => &[Scheme::C],
            Self::Cpp => &[
                Scheme::CppItanium,
                Scheme::CppMsvc,
                Scheme::CppGnuV2,
                Scheme::CppArm,
                Scheme::CppEdg,
                Scheme::CppHp,
            ],
            Self::ObjectiveC => &[Scheme::ObjC],
            Self::Rust => &[Scheme::RustLegacy, Scheme::RustV0],
            Self::Swift => &[Scheme::SwiftStable, Scheme::SwiftLegacy],
            Self::Go => &[Scheme::Go],
            Self::KotlinNative => &[Scheme::KotlinNative],
            Self::D => &[Scheme::D],
            Self::Java => &[Scheme::Jni],
            Self::Ada => &[Scheme::Ada],
            Self::Fortran => &[Scheme::Fortran],
            Self::Delphi => &[Scheme::Delphi],
            Self::OCaml => &[Scheme::OCaml],
            Self::Unknown => &[Scheme::Unknown],
        }
    }
}

/// The symbol mangling scheme.
///
/// A scheme is one grammar or encoding, not one source language: the six C++
/// dialects, the two rustc encodings and the two Swift manglings are separate
/// schemes. [`Language`] groups them.
///
/// Objective-C and Go stay one scheme each: Apple and GNUstep metadata share
/// one backend contract, and Go names are a linker-name layout rather than a
/// family of manglings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Scheme {
    /// Plain C (no mangling).
    C,
    /// Itanium C++ ABI (`_Z...`), GCC/Clang.
    CppItanium,
    /// Microsoft C++ (`?...`).
    CppMsvc,
    /// Legacy GNU v2 C++ (`foo__Fiv`).
    CppGnuV2,
    /// ARM C++ (`__ct__3fooFv`, `__vtbl__3foo`, `__sti__foo`).
    CppArm,
    /// EDG C++ (`foo__Fi_v`, `foo__tm__2_i`).
    CppEdg,
    /// HP aCC C++ (`foo__XSP5_Fv`, `foo__Fi_1`).
    CppHp,
    /// Legacy rustc encoding (`_ZN...17h<hash>E`).
    RustLegacy,
    /// rustc v0 encoding (`_R...`).
    RustV0,
    /// Stable Swift mangling (`$s`, `$S`, `_$s`, `_$S`, `_T0`).
    SwiftStable,
    /// Pre-stable Swift mangling (`_T...` without the `_T0` version marker).
    SwiftLegacy,
    /// Objective-C runtime metadata and method display forms.
    ObjC,
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
    /// Number of [`Scheme`] variants, including [`Scheme::Unknown`].
    pub const COUNT: usize = 21;

    /// Every scheme in taxonomy order, which is also the declaration order.
    pub const ALL: [Self; Self::COUNT] = [
        Self::C,
        Self::CppItanium,
        Self::CppMsvc,
        Self::CppGnuV2,
        Self::CppArm,
        Self::CppEdg,
        Self::CppHp,
        Self::RustLegacy,
        Self::RustV0,
        Self::SwiftStable,
        Self::SwiftLegacy,
        Self::ObjC,
        Self::Go,
        Self::KotlinNative,
        Self::D,
        Self::Jni,
        Self::Ada,
        Self::Fortran,
        Self::Delphi,
        Self::OCaml,
        Self::Unknown,
    ];

    /// Human-readable scheme label.
    ///
    /// The label is precise: it names the C++ dialect, the rustc encoding and
    /// the Swift mangling family. [`Language::as_str`] returns the group name
    /// instead.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::C => "C",
            Self::CppItanium => "C++ (Itanium)",
            Self::CppMsvc => "C++ (MSVC)",
            Self::CppGnuV2 => "C++ (GNU v2)",
            Self::CppArm => "C++ (ARM)",
            Self::CppEdg => "C++ (EDG)",
            Self::CppHp => "C++ (HP aCC)",
            Self::RustLegacy => "Rust (legacy)",
            Self::RustV0 => "Rust (v0)",
            Self::SwiftStable => "Swift",
            Self::SwiftLegacy => "Swift (legacy)",
            Self::ObjC => "Objective-C",
            Self::Go => "Go",
            Self::KotlinNative => "Kotlin/Native",
            Self::D => "D",
            Self::Jni => "JNI",
            Self::Ada => "Ada",
            Self::Fortran => "Fortran",
            Self::Delphi => "Delphi/Object Pascal",
            Self::OCaml => "OCaml",
            Self::Unknown => "Unknown",
        }
    }

    /// Detect a raw symbol byte string.
    ///
    /// The result is the precise scheme, not a language group. Unambiguous
    /// prefixes are a cheap test; the two grammars without one — the legacy
    /// C++ dialects and pre-stable Swift — are claimed only after a bounded
    /// parse of the whole symbol.
    ///
    /// Prefix-based schemes are recognised after stripping the decorations a
    /// linker or symbol table may append: a ThinLTO `.llvm.<hex>` suffix and a
    /// GNU symbol-version `@`/`@@` suffix. Platform leading underscores are
    /// part of the prefix: `__Z` is Itanium C++ and `__R` is rustc v0, just
    /// like their single-underscore spellings.
    ///
    /// Itanium (`_Z`), Microsoft (`?`), rustc v0 (`_R`), the rustc legacy
    /// crate hash and stable Swift have unambiguous prefixes, so those checks
    /// are prefix tests. Objective-C metadata also has explicit prefixes
    /// (`_OBJC_`, `__OBJC_`, `OBJC_`, `__objc_`) and method display forms, so
    /// it is checked before Go and before the legacy C++ dialects: dotted
    /// metadata such as `_OBJC_IVAR_$_Foo._ivar` looks like a
    /// package-qualified Go name, and legacy C++ has no unambiguous prefix.
    /// Legacy GNU v2 C++ has no version prefix — `foo__Fiv` is a name plus a
    /// `__`-separated signature — so a name that could be legacy is validated
    /// with a bounded parse that also reports the winning dialect. Pre-stable
    /// Swift shares the `_T` stem with plain C names, so it needs the same kind
    /// of bounded parse. A plain C name is never reported as C++: the legacy
    /// parse has to consume the whole symbol.
    ///
    /// Detection uses byte prefixes, so non-UTF-8 identifier bytes do not
    /// prevent a recognized scheme from being returned.
    #[must_use]
    pub fn detect_bytes(name: &[u8]) -> Self {
        crate::detection::detect(name).scheme
    }

    /// Detect the mangling scheme from a UTF-8 symbol name.
    ///
    /// UTF-8 wrapper around [`Self::detect_bytes`]; the same precedence and
    /// full-parse rules apply.
    #[must_use]
    pub fn detect(name: &str) -> Self {
        Self::detect_bytes(name.as_bytes())
    }

    /// Return the language this scheme demangles.
    #[must_use]
    pub const fn language(&self) -> Language {
        match self {
            Self::C => Language::C,
            Self::CppItanium
            | Self::CppMsvc
            | Self::CppGnuV2
            | Self::CppArm
            | Self::CppEdg
            | Self::CppHp => Language::Cpp,
            Self::ObjC => Language::ObjectiveC,
            Self::RustLegacy | Self::RustV0 => Language::Rust,
            Self::SwiftStable | Self::SwiftLegacy => Language::Swift,
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

impl core::fmt::Display for Scheme {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The precise scheme of one C++ [`crate::cpp::Dialect`].
#[cfg(any(feature = "cpp-itanium", feature = "cpp-msvc", feature = "cpp-gnu-v2"))]
pub(crate) const fn scheme_for_cpp_dialect(dialect: crate::cpp::Dialect) -> Scheme {
    match dialect {
        crate::cpp::Dialect::Itanium => Scheme::CppItanium,
        crate::cpp::Dialect::Msvc => Scheme::CppMsvc,
        crate::cpp::Dialect::GnuV2 => Scheme::CppGnuV2,
        crate::cpp::Dialect::Arm => Scheme::CppArm,
        crate::cpp::Dialect::Edg => Scheme::CppEdg,
        crate::cpp::Dialect::Hp => Scheme::CppHp,
    }
}

/// The precise scheme of one rustc [`crate::rust::Encoding`].
#[cfg(feature = "rust")]
pub(crate) const fn scheme_for_rust_encoding(encoding: crate::rust::Encoding) -> Scheme {
    match encoding {
        crate::rust::Encoding::Legacy => Scheme::RustLegacy,
        crate::rust::Encoding::V0 => Scheme::RustV0,
    }
}

/// The precise scheme of one Swift [`crate::swift::Encoding`].
#[cfg(feature = "swift")]
pub(crate) const fn scheme_for_swift_encoding(encoding: crate::swift::Encoding) -> Scheme {
    match encoding {
        crate::swift::Encoding::Stable => Scheme::SwiftStable,
        crate::swift::Encoding::Legacy => Scheme::SwiftLegacy,
    }
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

/// A set of [`Scheme`] values, stored as a bit set.
///
/// Precise schemes number fewer than 32, so one `u32` holds every one of them
/// and a selection never allocates.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct SchemeSet(u32);

/// The scheme set has to fit its storage.
const _: () = assert!(Scheme::COUNT <= 32);

impl SchemeSet {
    /// The empty set.
    pub(crate) const EMPTY: Self = Self(0);

    /// The single bit for `scheme`.
    ///
    /// The match is exhaustive, so a new [`Scheme`] variant has to be given a
    /// bit here as well.
    const fn bit(scheme: Scheme) -> u32 {
        match scheme {
            Scheme::C => 1 << 0,
            Scheme::CppItanium => 1 << 1,
            Scheme::CppMsvc => 1 << 2,
            Scheme::CppGnuV2 => 1 << 3,
            Scheme::CppArm => 1 << 4,
            Scheme::CppEdg => 1 << 5,
            Scheme::CppHp => 1 << 6,
            Scheme::RustLegacy => 1 << 7,
            Scheme::RustV0 => 1 << 8,
            Scheme::SwiftStable => 1 << 9,
            Scheme::SwiftLegacy => 1 << 10,
            Scheme::ObjC => 1 << 11,
            Scheme::Go => 1 << 12,
            Scheme::KotlinNative => 1 << 13,
            Scheme::D => 1 << 14,
            Scheme::Jni => 1 << 15,
            Scheme::Ada => 1 << 16,
            Scheme::Fortran => 1 << 17,
            Scheme::Delphi => 1 << 18,
            Scheme::OCaml => 1 << 19,
            Scheme::Unknown => 1 << 20,
        }
    }

    /// Add `scheme` to the set.
    pub(crate) const fn insert(&mut self, scheme: Scheme) {
        self.0 |= Self::bit(scheme);
    }

    /// True when `scheme` is in the set.
    pub(crate) const fn contains(self, scheme: Scheme) -> bool {
        self.0 & Self::bit(scheme) != 0
    }

    /// True when the set holds no scheme.
    pub(crate) const fn is_empty(self) -> bool {
        self.0 == 0
    }
}

/// Strip a trailing `.llvm.<hex>` ThinLTO suffix from a byte string.
pub(crate) fn strip_thin_lto_bytes(name: &[u8]) -> &[u8] {
    const LLVM: &[u8] = b".llvm.";
    let Some(index) = crate::parse::find(name, LLVM) else {
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

#[cfg(test)]
mod tests;
