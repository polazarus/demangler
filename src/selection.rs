use crate::demangler::Demangler;
use crate::error::Error;
use crate::result::Demangled;
use crate::scheme::{Language, SCHEME_COUNT, Scheme, SchemeSet};

/// One selection target: an exact scheme or a whole language group.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Selector {
    /// One exact scheme.
    Scheme(Scheme),
    /// Every scheme of one language.
    Language(Language),
}

impl From<Scheme> for Selector {
    fn from(scheme: Scheme) -> Self {
        Self::Scheme(scheme)
    }
}

impl From<Language> for Selector {
    fn from(language: Language) -> Self {
        Self::Language(language)
    }
}

/// Maximum number of preferred schemes a [`Selection`] holds.
///
/// Preferred schemes are deduplicated, so the list never holds more entries
/// than `SCHEME_COUNT`.
const MAX_PREFERRED: usize = 32;

/// Candidate order for allowed schemes that were not preferred.
///
/// The order mirrors [`Scheme::detect_bytes`]: unambiguous prefixes first, the
/// grammars that need a bounded parse last, and the two pass-through schemes at
/// the very end so they only win when nothing else matched. Preference order
/// overrides it, so `prefer(Scheme::C)` does put C first.
const DETECTION_ORDER: [Scheme; SCHEME_COUNT] = [
    Scheme::RustV0,
    Scheme::RustLegacy,
    Scheme::D,
    Scheme::CppItanium,
    Scheme::SwiftStable,
    Scheme::CppMsvc,
    Scheme::ObjC,
    Scheme::KotlinNative,
    Scheme::Delphi,
    Scheme::OCaml,
    Scheme::Go,
    Scheme::Jni,
    Scheme::Ada,
    Scheme::Fortran,
    Scheme::SwiftLegacy,
    Scheme::CppGnuV2,
    Scheme::CppArm,
    Scheme::CppEdg,
    Scheme::CppHp,
    Scheme::C,
    Scheme::Unknown,
];

/// The preferred list has to hold every scheme.
const _: () = assert!(SCHEME_COUNT <= MAX_PREFERRED);

/// An ordered set of candidate schemes.
///
/// Build one with [`Demangler::selection`], add targets with [`Selection::allow`]
/// or [`Selection::prefer`], then demangle. A `Selection` is `Copy`, holds no
/// allocation and can be reused for as many symbols as needed. It copies the
/// source [`Demangler`] and therefore snapshots its [`Budget`](crate::Budget):
/// changing the demangler afterwards does not change an existing selection.
///
/// Selection never falls back to another language: every candidate is demangled
/// through the exact-scheme path, so a failed candidate moves on to the next
/// allowed scheme instead of being retagged. An empty selection is exactly
/// [`Demangler::demangle`], which keeps the detected C++ to Rust fallback.
///
/// Candidate order is: preferred schemes in call order, then the remaining
/// allowed schemes in detection order, with [`Scheme::C`] and
/// [`Scheme::Unknown`] last unless they were preferred.
///
/// # Example
///
/// ```rust
/// use demangler::{Demangler, Language, Scheme};
///
/// let demangler = Demangler::new();
/// let demangled = demangler
///     .selection()
///     .allow(Language::C)
///     .demangle("main")
///     .unwrap();
/// assert_eq!(demangled.scheme(), Scheme::C);
/// assert_eq!(demangled.to_string(), "main");
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Selection {
    demangler: Demangler,
    allowed: SchemeSet,
    preferred: [Option<Scheme>; MAX_PREFERRED],
    preferred_len: usize,
}

impl Selection {
    /// An empty selection over `demangler`.
    pub(crate) const fn new(demangler: Demangler) -> Self {
        Self {
            demangler,
            allowed: SchemeSet::EMPTY,
            preferred: [None; MAX_PREFERRED],
            preferred_len: 0,
        }
    }

    /// Allow one scheme or every scheme of one language.
    #[must_use]
    pub fn allow(self, selector: impl Into<Selector>) -> Self {
        let mut selection = self;
        selection.allow_selector(selector.into());
        selection
    }

    /// Allow every scheme in `schemes`.
    ///
    /// An empty slice is a no-op.
    #[must_use]
    pub fn allow_schemes(mut self, schemes: &[Scheme]) -> Self {
        for scheme in schemes {
            self.allowed.insert(*scheme);
        }
        self
    }

    /// Allow every scheme of every language in `languages`.
    ///
    /// An empty slice is a no-op.
    #[must_use]
    pub fn allow_languages(mut self, languages: &[Language]) -> Self {
        for language in languages {
            self.allow_language(*language);
        }
        self
    }

    /// Allow a target and move its schemes to the front of the candidate order.
    ///
    /// Preferred schemes are tried in call order, before the remaining allowed
    /// schemes. Preferring a target that is already allowed only changes its
    /// position.
    #[must_use]
    pub fn prefer(self, selector: impl Into<Selector>) -> Self {
        let mut selection = self;
        selection.prefer_selector(selector.into());
        selection
    }

    /// Allow `schemes` and move them to the front of the candidate order.
    #[must_use]
    pub fn prefer_schemes(mut self, schemes: &[Scheme]) -> Self {
        for scheme in schemes {
            self.allowed.insert(*scheme);
            self.push_preferred(*scheme);
        }
        self
    }

    /// Allow every scheme of `languages` and move them to the front of the
    /// candidate order.
    #[must_use]
    pub fn prefer_languages(mut self, languages: &[Language]) -> Self {
        for language in languages {
            self.allow_language(*language);
            for scheme in language.schemes() {
                self.push_preferred(*scheme);
            }
        }
        self
    }

    /// Demangle a UTF-8 symbol name with this selection.
    ///
    /// An empty selection is exactly [`Demangler::demangle`], including its
    /// detected C++ to Rust fallback.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Parse`] when exactly one scheme was a candidate
    /// and it rejected the symbol, and [`Error::NoMatchingScheme`] when several
    /// candidates were tried and all of them failed.
    pub fn demangle<'a>(&self, symbol: &'a str) -> Result<Demangled<'a>, Error> {
        self.demangle_bytes(symbol.as_bytes())
    }

    /// Demangle a raw symbol byte string with this selection.
    ///
    /// Raw-byte twin of [`Self::demangle`]; see that method for the candidate
    /// order and the error rules.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Parse`] when exactly one scheme was a candidate
    /// and it rejected the symbol, and [`Error::NoMatchingScheme`] when several
    /// candidates were tried and all of them failed.
    pub fn demangle_bytes<'a>(&self, symbol: &'a [u8]) -> Result<Demangled<'a>, Error> {
        if self.allowed.is_empty() {
            return self.demangler.demangle_bytes(symbol);
        }
        self.demangle_bytes_peeled(symbol)
    }

    /// Peel platform decorations and try every candidate.
    ///
    /// A PE/COFF decoration is decoded once and attached to the winning
    /// candidate, exactly as on the detected path.
    fn demangle_bytes_peeled<'a>(&self, symbol: &'a [u8]) -> Result<Demangled<'a>, Error> {
        #[cfg(feature = "coff")]
        if let Some(decoration) = crate::coff::parse(symbol) {
            let inner = self.demangle_bytes_peeled(decoration.symbol())?;
            return Ok(inner.with_coff_decoration(decoration));
        }
        self.demangle_bytes_candidates(symbol)
    }

    /// Try each candidate scheme in order and return the first success.
    fn demangle_bytes_candidates<'a>(&self, symbol: &'a [u8]) -> Result<Demangled<'a>, Error> {
        let mut attempted = 0_usize;
        let mut last_error = Error::parse(Scheme::Unknown, crate::ParseErrorKind::NotThisScheme);
        let mut budget_error = None;
        for scheme in self.candidates() {
            attempted += 1;
            match self.demangler.dispatch_bytes(scheme, symbol, false) {
                Ok(demangled) => return Ok(demangled),
                Err(error) => {
                    last_error = error;
                    if budget_error.is_none() && error.budget_kind().is_some() {
                        budget_error = Some(error);
                    }
                }
            }
        }
        if attempted == 1 {
            Err(last_error)
        } else if let Some(error) = budget_error {
            // A capacity failure is not a scheme-selection failure: the caller
            // can raise the budget and retry the same candidates.
            Err(error)
        } else {
            Err(Error::NoMatchingScheme)
        }
    }

    /// The candidate schemes in order.
    const fn candidates(&self) -> Candidates<'_> {
        Candidates {
            selection: self,
            preferred: 0,
            ordered: 0,
        }
    }

    /// Add every scheme of `selector` to the allowed set.
    fn allow_selector(&mut self, selector: Selector) {
        match selector {
            Selector::Scheme(scheme) => self.allowed.insert(scheme),
            Selector::Language(language) => self.allow_language(language),
        }
    }

    /// Add every scheme of `language` to the allowed set.
    fn allow_language(&mut self, language: Language) {
        for scheme in language.schemes() {
            self.allowed.insert(*scheme);
        }
    }

    /// Allow `selector` and move its schemes to the front.
    fn prefer_selector(&mut self, selector: Selector) {
        match selector {
            Selector::Scheme(scheme) => {
                self.allowed.insert(scheme);
                self.push_preferred(scheme);
            }
            Selector::Language(language) => {
                self.allow_language(language);
                for scheme in language.schemes() {
                    self.push_preferred(*scheme);
                }
            }
        }
    }

    /// Append `scheme` to the preference list unless it is already preferred.
    fn push_preferred(&mut self, scheme: Scheme) {
        if self.is_preferred(scheme) || self.preferred_len >= MAX_PREFERRED {
            return;
        }
        self.preferred[self.preferred_len] = Some(scheme);
        self.preferred_len += 1;
    }

    /// True when `scheme` is in the preference list.
    fn is_preferred(&self, scheme: Scheme) -> bool {
        self.preferred[..self.preferred_len].contains(&Some(scheme))
    }
}

/// The ordered candidate schemes of one [`Selection`].
#[derive(Clone)]
struct Candidates<'a> {
    selection: &'a Selection,
    preferred: usize,
    ordered: usize,
}

impl Iterator for Candidates<'_> {
    type Item = Scheme;

    fn next(&mut self) -> Option<Self::Item> {
        let selection = self.selection;
        if let Some(scheme) = selection.preferred.get(self.preferred).copied().flatten() {
            self.preferred += 1;
            return Some(scheme);
        }
        while let Some(&scheme) = DETECTION_ORDER.get(self.ordered) {
            self.ordered += 1;
            if selection.allowed.contains(scheme) && !selection.is_preferred(scheme) {
                return Some(scheme);
            }
        }
        None
    }
}

#[cfg(test)]
mod tests;
