//! Ordered scheme and language selection.
//!
//! A [`Selection`] is an allocation-free, reusable candidate list built from a
//! [`Demangler`]. It lets a caller state which schemes or languages a symbol
//! may use, and in which order they should be tried, instead of accepting
//! automatic detection.
//!
//! Selection never falls back to another language: every candidate is demangled
//! through the exact-scheme path, so a failed candidate moves on to the next
//! allowed scheme instead of being retagged. An empty selection is exactly
//! [`Demangler::demangle`], which keeps the detected C++ to Rust fallback.
//!
//! Reference: `.plans/scheme-selection.md` documents the resolved design.

use crate::demangler::Demangler;
use crate::error::Error;
use crate::result::Demangled;
use crate::scheme::{Language, Scheme, SchemeSet};

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
/// than [`Scheme::COUNT`].
const MAX_PREFERRED: usize = 32;

/// Candidate order for allowed schemes that were not preferred.
///
/// The order mirrors [`Scheme::detect_bytes`]: unambiguous prefixes first, the
/// grammars that need a bounded parse last, and the two pass-through schemes at
/// the very end so they only win when nothing else matched. Preference order
/// overrides it, so `prefer(Scheme::C)` does put C first.
const DETECTION_ORDER: [Scheme; Scheme::COUNT] = [
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
const _: () = assert!(Scheme::COUNT <= MAX_PREFERRED);

/// An ordered set of candidate schemes.
///
/// Build one with [`Demangler::selection`], add targets with [`Selection::allow`]
/// or [`Selection::prefer`], then demangle. A `Selection` is `Copy`, holds no
/// allocation and can be reused for as many symbols as needed. It copies the
/// source [`Demangler`] and therefore snapshots its [`Budget`](crate::Budget):
/// changing the demangler afterwards does not change an existing selection.
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
#[derive(Debug, Clone, Copy)]
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
    pub fn allow(self, selector: impl Into<Selector>) -> Self {
        let mut selection = self;
        selection.allow_selector(selector.into());
        selection
    }

    /// Allow every scheme in `schemes`.
    ///
    /// An empty slice is a no-op.
    pub fn allow_schemes(mut self, schemes: &[Scheme]) -> Self {
        for scheme in schemes {
            self.allowed.insert(*scheme);
        }
        self
    }

    /// Allow every scheme of every language in `languages`.
    ///
    /// An empty slice is a no-op.
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
    pub fn prefer(self, selector: impl Into<Selector>) -> Self {
        let mut selection = self;
        selection.prefer_selector(selector.into());
        selection
    }

    /// Allow `schemes` and move them to the front of the candidate order.
    pub fn prefer_schemes(mut self, schemes: &[Scheme]) -> Self {
        for scheme in schemes {
            self.allowed.insert(*scheme);
            self.push_preferred(*scheme);
        }
        self
    }

    /// Allow every scheme of `languages` and move them to the front of the
    /// candidate order.
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
    /// Returns [`Error::InvalidSymbol`] when exactly one scheme was a candidate
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
    /// Returns [`Error::InvalidSymbol`] when exactly one scheme was a candidate
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
        if let Some(decoration) = crate::coff::parse_bytes(symbol) {
            let inner = self.demangle_bytes_peeled(decoration.symbol())?;
            return Ok(inner.with_coff_decoration(decoration));
        }
        self.demangle_bytes_candidates(symbol)
    }

    /// Try each candidate scheme in order and return the first success.
    fn demangle_bytes_candidates<'a>(&self, symbol: &'a [u8]) -> Result<Demangled<'a>, Error> {
        let mut attempted = 0_usize;
        let mut last = Scheme::Unknown;
        for scheme in self.candidates() {
            attempted += 1;
            last = scheme;
            if let Ok(demangled) = self.demangler.dispatch_bytes(scheme, symbol, false) {
                return Ok(demangled);
            }
        }
        if attempted == 1 {
            Err(Error::InvalidSymbol(last))
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
#[derive(Clone, Copy)]
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
mod tests {
    use std::string::ToString;
    use std::vec::Vec;

    use super::*;

    /// Collect the candidate order of a selection.
    fn order(selection: Selection) -> Vec<Scheme> {
        selection.candidates().collect()
    }

    #[test]
    fn detection_order_covers_every_scheme_once() {
        assert_eq!(DETECTION_ORDER.len(), Scheme::COUNT);
        for scheme in Scheme::ALL {
            assert_eq!(
                DETECTION_ORDER
                    .iter()
                    .filter(|other| **other == scheme)
                    .count(),
                1,
                "{scheme} is listed once"
            );
        }
        // The pass-through schemes are the fallback of last resort.
        assert_eq!(DETECTION_ORDER[Scheme::COUNT - 2], Scheme::C);
        assert_eq!(DETECTION_ORDER[Scheme::COUNT - 1], Scheme::Unknown);
    }

    #[test]
    fn empty_selection_has_no_candidates() {
        assert!(order(Demangler::new().selection()).is_empty());
    }

    #[test]
    fn allow_scheme_uses_detection_order() {
        let selection = Demangler::new()
            .selection()
            .allow(Scheme::Unknown)
            .allow(Scheme::CppMsvc)
            .allow(Scheme::RustV0);
        assert_eq!(
            order(selection),
            [Scheme::RustV0, Scheme::CppMsvc, Scheme::Unknown]
        );
    }

    #[test]
    fn allow_language_expands_the_group() {
        let selection = Demangler::new().selection().allow(Language::Cpp);
        assert_eq!(
            order(selection),
            [
                Scheme::CppItanium,
                Scheme::CppMsvc,
                Scheme::CppGnuV2,
                Scheme::CppArm,
                Scheme::CppEdg,
                Scheme::CppHp,
            ]
        );
    }

    #[test]
    fn prefer_moves_schemes_to_the_front() {
        let selection = Demangler::new()
            .selection()
            .allow(Language::Cpp)
            .prefer(Scheme::CppHp)
            .prefer(Scheme::CppMsvc);
        assert_eq!(
            order(selection)[..2],
            [Scheme::CppHp, Scheme::CppMsvc],
            "preference is in call order"
        );
        assert_eq!(order(selection).len(), 6, "duplicates are dropped");
    }

    #[test]
    fn prefer_language_moves_the_group_to_the_front() {
        let selection = Demangler::new()
            .selection()
            .allow(Language::Cpp)
            .prefer(Language::Rust);
        assert_eq!(order(selection)[..2], [Scheme::RustLegacy, Scheme::RustV0]);
        assert_eq!(order(selection).len(), 8);
    }

    #[test]
    fn prefer_c_and_unknown_override_the_tail_rule() {
        let selection = Demangler::new()
            .selection()
            .allow(Scheme::CppItanium)
            .allow(Scheme::C)
            .prefer(Scheme::C);
        assert_eq!(order(selection), [Scheme::C, Scheme::CppItanium]);

        let selection = Demangler::new()
            .selection()
            .allow(Scheme::CppItanium)
            .allow(Scheme::Unknown)
            .prefer(Scheme::Unknown);
        assert_eq!(order(selection), [Scheme::Unknown, Scheme::CppItanium]);
    }

    #[test]
    fn empty_slices_are_no_ops() {
        let selection = Demangler::new()
            .selection()
            .allow_schemes(&[])
            .allow_languages(&[])
            .prefer_schemes(&[])
            .prefer_languages(&[]);
        assert!(order(selection).is_empty());
    }

    #[test]
    fn plural_methods_match_their_singular_forms() {
        assert_eq!(
            order(Demangler::new()
                .selection()
                .allow_schemes(&[Scheme::Go, Scheme::CppItanium])),
            order(Demangler::new()
                .selection()
                .allow(Scheme::Go)
                .allow(Scheme::CppItanium))
        );
        assert_eq!(
            order(Demangler::new()
                .selection()
                .prefer_languages(&[Language::Rust, Language::C])),
            order(Demangler::new()
                .selection()
                .prefer(Language::Rust)
                .prefer(Language::C))
        );
        assert_eq!(
            order(Demangler::new()
                .selection()
                .prefer_schemes(&[Scheme::ObjC, Scheme::D])),
            order(Demangler::new()
                .selection()
                .prefer(Scheme::ObjC)
                .prefer(Scheme::D))
        );
    }

    #[test]
    fn empty_selection_delegates_to_detection() {
        let demangler = Demangler::new();
        let selection = demangler.selection();
        for symbol in [&b"main"[..], b"12345", b"plain\xff"] {
            let detected = demangler.demangle_bytes(symbol).unwrap();
            let selected = selection.demangle_bytes(symbol).unwrap();
            assert_eq!(selected.scheme(), detected.scheme(), "{symbol:?}");
            assert_eq!(selected.to_string(), detected.to_string(), "{symbol:?}");
        }
        assert_eq!(selection.demangle("main").unwrap().scheme(), Scheme::C);
    }

    #[cfg(feature = "cpp-itanium")]
    #[test]
    fn single_allowed_scheme_reports_invalid_symbol() {
        let error = Demangler::new()
            .selection()
            .allow(Scheme::CppItanium)
            .demangle("not a c++ symbol")
            .unwrap_err();
        assert_eq!(error, Error::InvalidSymbol(Scheme::CppItanium));
    }

    #[cfg(feature = "go")]
    #[test]
    fn one_member_language_behaves_like_one_scheme() {
        let error = Demangler::new()
            .selection()
            .allow(Language::Go)
            .demangle("fmt.Println[")
            .unwrap_err();
        assert_eq!(error.scheme(), Scheme::Go);
    }

    #[cfg(all(feature = "cpp-itanium", feature = "cpp-msvc"))]
    #[test]
    fn several_failed_candidates_report_no_matching_scheme() {
        let error = Demangler::new()
            .selection()
            .allow(Scheme::CppItanium)
            .allow(Scheme::CppMsvc)
            .demangle("not-a-symbol!")
            .unwrap_err();
        assert_eq!(error, Error::NoMatchingScheme);
        assert_eq!(error.scheme(), Scheme::Unknown);
    }

    #[cfg(feature = "cpp-itanium")]
    #[test]
    fn exact_scheme_rejects_another_cpp_dialect() {
        let demangler = Demangler::new();
        let error = demangler
            .selection()
            .allow(Scheme::CppItanium)
            .demangle("?x@@3HA")
            .unwrap_err();
        assert_eq!(error, Error::InvalidSymbol(Scheme::CppItanium));
    }

    #[cfg(feature = "cpp-msvc")]
    #[test]
    fn preferred_scheme_wins_over_detection_order() {
        let demangler = Demangler::new();
        let demangled = demangler
            .selection()
            .allow(Language::Cpp)
            .prefer(Scheme::CppMsvc)
            .demangle("?x@@3HA")
            .unwrap();
        assert_eq!(demangled.scheme(), Scheme::CppMsvc);
        assert_eq!(demangled.to_string(), "int x");
    }

    #[cfg(all(feature = "cpp-itanium", feature = "go"))]
    #[test]
    fn failing_candidate_falls_through_to_the_next() {
        let demangled = Demangler::new()
            .selection()
            .prefer(Scheme::CppItanium)
            .allow(Scheme::Go)
            .demangle("fmt.Println")
            .unwrap();
        assert_eq!(demangled.scheme(), Scheme::Go);
    }

    #[cfg(all(feature = "coff", feature = "cpp-itanium"))]
    #[test]
    fn coff_decoration_is_reported_on_the_winning_candidate() {
        let demangled = Demangler::new()
            .selection()
            .allow(Scheme::CppItanium)
            .demangle("__imp__Z3foov")
            .unwrap();
        assert_eq!(demangled.scheme(), Scheme::CppItanium);
        assert_eq!(demangled.to_string(), "foo()");
        assert!(
            demangled
                .coff_decoration()
                .is_some_and(|decoration| decoration.import())
        );
    }

    #[test]
    fn selection_is_copy_and_reusable() {
        let selection = Demangler::new().selection().allow(Scheme::C);
        let copy = selection;
        assert_eq!(copy.demangle("main").unwrap().to_string(), "main");
        assert_eq!(selection.demangle("foo").unwrap().to_string(), "foo");
    }

    #[cfg(feature = "cpp-itanium")]
    #[test]
    fn selection_snapshots_the_demangler_budget() {
        let mut demangler = Demangler::new();
        let selection = demangler.selection().allow(Scheme::CppItanium);
        demangler.set_budget(crate::Budget::new().with_input_bytes(0));

        // The selection kept the budget it was built with.
        assert_eq!(
            selection.demangle("_Z3foov").unwrap().to_string(),
            "foo()"
        );
        assert!(
            demangler
                .selection()
                .allow(Scheme::CppItanium)
                .demangle("_Z3foov")
                .is_err()
        );
    }

    #[test]
    fn selector_converts_from_both_targets() {
        assert_eq!(Selector::from(Scheme::Go), Selector::Scheme(Scheme::Go));
        assert_eq!(
            Selector::from(Language::Java),
            Selector::Language(Language::Java)
        );
    }
}
