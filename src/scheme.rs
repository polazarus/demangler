/// The programming language that a mangled symbol belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Language {
    /// Plain C (no mangling).
    C,
    /// C++ (Itanium, GNU, MSVC).
    Cxx,
    /// Rust (rustc mangling).
    Rust,
    /// Swift (Swift mangling).
    Swift,
    /// Go (planned).
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
    /// Rust mangled by rustc.
    Rust,
    /// Swift mangled.
    Swift,
    /// Go (planned).
    Go,
    /// Unknown or unsupported scheme.
    Unknown,
}

impl Scheme {
    /// Detect the mangling scheme from a symbol name.
    ///
    /// This is a cheap prefix/character check: no allocation, no parsing.
    pub fn detect(name: &str) -> Self {
        if name.is_empty() {
            return Self::Unknown;
        }

        // `_RN` is the rustc v0 mangling prefix.
        if name.starts_with("_RN") {
            return Self::Rust;
        }

        // `_Z` introduces both old rustc symbols and Itanium C++ names; C++
        // is the more common convention and the Rust fallback catches the rest.
        if name.starts_with("_Z") {
            return Self::Cpp;
        }

        if crate::swift::is_swift(name) {
            return Self::Swift;
        }

        if name.starts_with('?') {
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
            Self::Rust => Language::Rust,
            Self::Swift => Language::Swift,
            Self::Go => Language::Go,
            Self::Unknown => Language::Unknown,
        }
    }
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
