#[cfg(feature = "d")]
use alloc::string::String;

use crate::budget::{Budget, EffectiveBudget};
use crate::error::Error;
use crate::result::Demangled;
use crate::scheme::{Language, Scheme};
use crate::selection::Selection;

/// Demangles symbol names using the appropriate demangling scheme.
///
/// Detects the scheme from the symbol name and dispatches to the appropriate
/// in-tree backend. No external demangler is invoked at runtime.
///
/// Report a parser failure as a budget error when the backend identified the
/// dimension, otherwise as a malformed symbol.
#[cfg(any(
    feature = "rust",
    feature = "go",
    feature = "objc",
    feature = "kotlin-native",
    feature = "d",
    feature = "jni",
    feature = "ada",
    feature = "fortran",
    feature = "delphi",
    feature = "ocaml",
    feature = "cpp-itanium",
    feature = "cpp-msvc",
    feature = "cpp-gnu-v2"
))]
fn budget_error(scheme: Scheme, kind: Option<crate::error::BudgetKind>) -> Error {
    kind.map_or_else(
        || Error::InvalidSymbol(scheme),
        |kind| Error::Budget { scheme, kind },
    )
}

/// A demangler carries one [`Budget`].
///
/// The budget resolves the parser limits for every candidate. The default
/// budget keeps every backend's compiled-in limits; every backend enforces the
/// resolved overrides. See [`Budget`](crate::Budget) for the per-scheme defaults and
/// hard caps.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Demangler {
    budget: Budget,
}

impl Demangler {
    /// Create a new [`Demangler`] with the default [`Budget`].
    #[must_use]
    pub const fn new() -> Self {
        Self {
            budget: Budget::new(),
        }
    }

    /// Create a [`Demangler`] with an explicit [`Budget`].
    ///
    /// The budget is resolved per scheme: an override is clamped to the hard
    /// cap of the scheme being demangled, and an unset dimension keeps that
    /// scheme's default. Every backend enforces the resolved values, so a
    /// configured budget is never bypassed by the detected path: detection
    /// stays default-only and a tree it retained is re-parsed with the
    /// configured budget.
    #[must_use]
    pub const fn with_budget(budget: Budget) -> Self {
        Self { budget }
    }

    /// The configured [`Budget`].
    #[must_use]
    pub const fn budget(&self) -> Budget {
        self.budget
    }

    /// Replace the configured [`Budget`].
    pub const fn set_budget(&mut self, budget: Budget) {
        self.budget = budget;
    }

    /// The limits this demangler resolves for `scheme`.
    ///
    /// Each override is clamped to the scheme's [`Scheme::hard_budget`]; an
    /// unset dimension keeps [`Scheme::default_budget`].
    #[must_use]
    pub fn effective_budget(&self, scheme: Scheme) -> EffectiveBudget {
        self.budget.effective(scheme)
    }

    /// Start a [`Selection`] of candidate schemes.
    ///
    /// An empty selection behaves exactly like [`Self::demangle`]; add targets
    /// with [`Selection::allow`] or [`Selection::prefer`] to restrict or order
    /// the candidates.
    #[must_use]
    pub const fn selection(&self) -> Selection {
        Selection::new(*self)
    }

    /// Detect the scheme and demangle a UTF-8 symbol name.
    ///
    /// The returned [`Demangled`] implements [`core::fmt::Display`]. Swift
    /// retains the parsed node tree and prints it lazily; other backends
    /// borrow from `symbol` or store owned text.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidSymbol`] when the backend cannot demangle the
    /// input.
    pub fn demangle<'a>(&self, symbol: &'a str) -> Result<Demangled<'a>, Error> {
        self.demangle_bytes(symbol.as_bytes())
    }

    /// Demangle a UTF-8 symbol name with an explicitly selected scheme.
    ///
    /// This is the text twin of [`Self::demangle`]: it bypasses
    /// [`Scheme::detect`] while still validating the selected backend:
    /// `scheme` must name a backend compiled into this build, and a name the
    /// backend rejects returns [`Error::InvalidSymbol`]. The scheme is exact:
    /// a C++ dialect or a rustc/Swift encoding the parser did not produce is
    /// an [`Error::InvalidSymbol`], never a fallback to another scheme.
    /// It is the integration point for names whose automatic detection is
    /// intentionally conservative: general Ada external names (`pkg__subp`),
    /// gfortran external names (`foo_`), namespaced Free Pascal units and OCaml
    /// external names.
    ///
    /// [`Scheme::C`] and [`Scheme::Unknown`] are pass-through schemes and
    /// return the input as plain text. COFF decorations recognised by
    /// `crate::coff::parse_bytes` are peeled before dispatch, exactly as in
    /// [`Self::demangle_bytes`]; target-specific forms (a bare x86 `_name`, a
    /// `name@@N` vectorcall suffix or a `.weak.` wrapper) are decoded first
    /// with `crate::coff::parse_bytes_for`, and its `Decoration::symbol` is
    /// passed here.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidSymbol`] when the selected backend is not
    /// compiled in or cannot demangle the input.
    pub fn demangle_with_scheme<'a>(
        &self,
        scheme: Scheme,
        symbol: &'a str,
    ) -> Result<Demangled<'a>, Error> {
        self.demangle_bytes_with_scheme(scheme, symbol.as_bytes())
    }

    /// Demangle a raw symbol byte string with an explicitly selected scheme.
    ///
    /// Raw-byte twin of [`Self::demangle_with_scheme`]; see that method for
    /// the detection-bypass and error semantics. Identifier bytes that are not
    /// valid UTF-8 are retained and rendered as `\xNN`, as in
    /// [`Self::demangle_bytes`].
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidSymbol`] when the selected backend is not
    /// compiled in or cannot demangle the input.
    pub fn demangle_bytes_with_scheme<'a>(
        &self,
        scheme: Scheme,
        symbol: &'a [u8],
    ) -> Result<Demangled<'a>, Error> {
        self.demangle_bytes_selected(scheme, symbol, false)
    }

    /// Demangle a UTF-8 symbol name with every scheme of one language.
    ///
    /// The group is tried in [`Language::schemes`] order, each member through
    /// the exact-scheme path.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidSymbol`] when the language has one scheme and it
    /// rejects the symbol, and [`Error::NoMatchingScheme`] when several members
    /// were tried and all of them failed.
    pub fn demangle_with_language<'a>(
        &self,
        language: Language,
        symbol: &'a str,
    ) -> Result<Demangled<'a>, Error> {
        self.selection().allow(language).demangle(symbol)
    }

    /// Demangle a raw symbol byte string with every scheme of one language.
    ///
    /// Raw-byte twin of [`Self::demangle_with_language`].
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidSymbol`] when the language has one scheme and it
    /// rejects the symbol, and [`Error::NoMatchingScheme`] when several members
    /// were tried and all of them failed.
    pub fn demangle_bytes_with_language<'a>(
        &self,
        language: Language,
        symbol: &'a [u8],
    ) -> Result<Demangled<'a>, Error> {
        self.selection().allow(language).demangle_bytes(symbol)
    }

    /// Peel PE/COFF decorations and dispatch to the selected backend.
    ///
    /// Both byte entry points share this path, so a decoration is decoded and
    /// reported exactly once. `allow_rust_fallback` keeps the detected path's
    /// legacy Rust fallback while a forced C++ scheme stays C++.
    fn demangle_bytes_selected<'a>(
        &self,
        scheme: Scheme,
        symbol: &'a [u8],
        allow_rust_fallback: bool,
    ) -> Result<Demangled<'a>, Error> {
        #[cfg(feature = "coff")]
        if let Some(decoration) = crate::coff::parse_bytes(symbol) {
            let inner =
                self.demangle_bytes_selected(scheme, decoration.symbol(), allow_rust_fallback)?;
            return Ok(inner.with_coff_decoration(decoration));
        }
        self.dispatch_bytes(scheme, symbol, allow_rust_fallback)
    }

    /// Demangle a symbol name read from a Mach-O symbol table.
    ///
    /// A Mach-O symbol table prefixes every external symbol with one
    /// underscore, so the C function `main` is stored as `_main`. The prefix
    /// is decoded only when no backend claims the name as written and the
    /// remainder is a plain C identifier:
    ///
    /// - `_main` is the C symbol `main`;
    /// - `__Z3foov` (Mach-O C++), `_$s...` (Mach-O Swift), `_OBJC_*` metadata
    ///   and `_RN...`/`_ZN...` Rust names keep their own scheme, because the
    ///   platform prefix is already part of their detection;
    /// - an assembly label (`_x.y`), a name whose remainder is not a plain C
    ///   identifier (`_123`, `__foo`) and a name with non-ASCII identifier
    ///   bytes are returned unchanged.
    ///
    /// [`Self::demangle`] and [`Self::demangle_bytes`] never strip the prefix,
    /// so an ordinary C name is never reclassified by accident. Linker
    /// fragment suffixes (`.cold`, `.part.N`) are not decoded here; declare a
    /// COFF target and use `crate::coff::parse_bytes_for` for those.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidSymbol`] when a detected backend cannot
    /// demangle the name; an unclaimed name is pass-through text.
    pub fn demangle_macho_bytes<'a>(&self, symbol: &'a [u8]) -> Result<Demangled<'a>, Error> {
        let detection = crate::detection::detect_with(symbol, self.budget);
        if detection.scheme == Scheme::Unknown
            && let Some(rest) = symbol.strip_prefix(b"_")
            && crate::scheme::is_plain_c_name(rest)
            && let Ok(text) = core::str::from_utf8(rest)
        {
            return Ok(Demangled::plain(Scheme::C, text));
        }
        self.demangle_detection(detection, true)
    }

    /// Demangle a UTF-8 symbol name read from a Mach-O symbol table.
    ///
    /// UTF-8 wrapper around [`Self::demangle_macho_bytes`]; see that method for
    /// the decoded prefix and its limits.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidSymbol`] when a detected backend cannot
    /// demangle the name; an unclaimed name is pass-through text.
    pub fn demangle_macho<'a>(&self, symbol: &'a str) -> Result<Demangled<'a>, Error> {
        self.demangle_macho_bytes(symbol.as_bytes())
    }

    /// Demangle a raw symbol byte string.
    ///
    /// Object-file symbol names are byte strings, not UTF-8 strings. Every
    /// backend parses bytes directly. Invalid UTF-8 in identifier text is
    /// retained and rendered as `\xNN`; compiler payload text follows its
    /// documented verifier policy. A name that no backend recognizes is
    /// returned as escaped plain text with [`Scheme::Unknown`].
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidSymbol`] when a detected backend cannot
    /// demangle the name, or [`Error::Budget`] when it exceeds the configured
    /// budget; an unclaimed name is pass-through text.
    pub fn demangle_bytes<'a>(&self, symbol: &'a [u8]) -> Result<Demangled<'a>, Error> {
        let detection = crate::detection::detect_with(symbol, self.budget);
        self.demangle_detection(detection, true)
    }

    /// Dispatch an already selected scheme to its backend.
    /// Dispatch a detection result, reusing a tree that detection parsed.
    fn demangle_detection<'a>(
        &self,
        detection: crate::detection::Detection<'a>,
        allow_rust_fallback: bool,
    ) -> Result<Demangled<'a>, Error> {
        let crate::detection::Detection {
            scheme,
            symbol,
            parse_symbol,
            parsed,
            #[cfg(feature = "coff")]
            decoration,
        } = detection;
        let result = match parsed {
            Some(parsed) => self.dispatch_parsed(parsed, parse_symbol, allow_rust_fallback)?,
            None => self.dispatch_bytes(scheme, symbol, allow_rust_fallback)?,
        };
        #[cfg(feature = "coff")]
        if let Some(decoration) = decoration {
            return Ok(result.with_coff_decoration(decoration));
        }
        Ok(result)
    }

    /// Dispatch a tree already parsed during detection.
    ///
    /// The legacy C++/Swift parses and the D/OCaml/Delphi/JNI prefilters
    /// retain their trees, so the detected path parses each symbol once. The
    /// AST is consumed without cloning; a failed C++ render keeps the
    /// detected-path Rust fallback on the same stripped byte view. Detection
    /// parses with the compiled-in budget, so a configured budget re-parses
    /// the retained bytes instead of reusing a tree that may exceed it. Go,
    /// Objective-C and Kotlin/Native detection is shape-only, so it retains no
    /// tree to re-parse.
    #[expect(
        clippy::too_many_lines,
        reason = "one arm per scheme the dispatch routes"
    )]
    #[cfg_attr(
        not(any(
            feature = "cpp-gnu-v2",
            feature = "swift",
            feature = "d",
            feature = "ocaml",
            feature = "delphi",
            feature = "jni"
        )),
        expect(
            clippy::unused_self,
            clippy::needless_pass_by_value,
            clippy::missing_const_for_fn,
            reason = "no backend in this build retains a detection tree"
        )
    )]
    fn dispatch_parsed<'a>(
        &self,
        parsed: crate::detection::Parsed<'a>,
        parse_symbol: &'a [u8],
        allow_rust_fallback: bool,
    ) -> Result<Demangled<'a>, Error> {
        // Each is used only by some backend arms.
        let _ = (parse_symbol, allow_rust_fallback);
        match parsed {
            #[cfg(feature = "cpp-gnu-v2")]
            crate::detection::Parsed::Cpp(parsed) => {
                let scheme = crate::scheme::scheme_for_cpp_dialect(parsed.dialect());
                // Detection parses legacy names with the compiled-in budget,
                // so a configured budget re-parses the retained bytes instead
                // of reusing a tree that may exceed it.
                let parsed = if self.budget == Budget::default() {
                    Some(parsed)
                } else {
                    match crate::cpp::parse_bytes_with_budget(
                        parse_symbol,
                        self.effective_budget(scheme),
                    ) {
                        Ok(reparsed)
                            if crate::scheme::scheme_for_cpp_dialect(reparsed.dialect())
                                == scheme =>
                        {
                            Some(reparsed)
                        }
                        _ => None,
                    }
                };
                // The retained tree must fit the resolved expansion gates and
                // the printer stack bound derived from the hard depth ceiling,
                // and it must render.
                if let Some(parsed) = parsed.filter(|parsed| {
                    parsed.print_envelope_ok(self.effective_budget(scheme)) && parsed.renders()
                }) {
                    return Ok(Demangled::cpp(scheme, parsed));
                }
                #[cfg(feature = "rust")]
                if allow_rust_fallback
                    && let Ok(parsed) = crate::rust::parse_bytes_with_budget(
                        parse_symbol,
                        self.rust_fallback_budget(parse_symbol),
                    )
                {
                    let scheme = crate::scheme::scheme_for_rust_encoding(parsed.encoding());
                    return Ok(Demangled::rust(scheme, parsed));
                }
                Err(Error::InvalidSymbol(scheme))
            }
            #[cfg(feature = "swift")]
            crate::detection::Parsed::Swift(parsed) => {
                let scheme = crate::scheme::scheme_for_swift_encoding(parsed.encoding());
                // Detection parses legacy `_T` names with the compiled-in
                // budget, so a configured budget re-parses the retained bytes
                // instead of reusing a tree that may exceed it.
                if self.budget == Budget::default() {
                    return Ok(Demangled::swift(scheme, parsed));
                }
                match crate::swift::parse_bytes_with_budget(
                    parse_symbol,
                    self.effective_budget(scheme),
                ) {
                    Ok(parsed)
                        if crate::scheme::scheme_for_swift_encoding(parsed.encoding())
                            == scheme =>
                    {
                        Ok(Demangled::swift(scheme, parsed))
                    }
                    _ => Err(Error::InvalidSymbol(scheme)),
                }
            }
            #[cfg(feature = "d")]
            crate::detection::Parsed::D(parsed) => {
                // Detection parses D names with the compiled-in budget, so a
                // configured budget re-parses the retained bytes instead of
                // reusing a tree that may exceed it.
                let parsed = if self.budget == Budget::default() {
                    parsed
                } else {
                    crate::d::parse_bytes_with_budget(
                        parse_symbol,
                        self.effective_budget(Scheme::D),
                    )
                    .map_err(|error| budget_error(Scheme::D, error.budget_kind()))?
                };
                // `Demangled` keeps no D tree, so render once into owned text.
                // The printer has no output cap (the parse already charged
                // back-reference expansion to the expansion gates); only the
                // sink can fail.
                let mut sink = String::new();
                if core::fmt::Write::write_fmt(&mut sink, format_args!("{parsed}")).is_ok() {
                    Ok(Demangled::owned(Scheme::D, sink))
                } else {
                    Err(Error::InvalidSymbol(Scheme::D))
                }
            }
            #[cfg(feature = "ocaml")]
            crate::detection::Parsed::OCaml(parsed) => {
                // Detection parses `caml` names with the compiled-in budget, so
                // a configured budget re-parses the retained bytes instead of
                // reusing a tree that may exceed it.
                if self.budget == Budget::default() {
                    return Ok(Demangled::ocaml(Scheme::OCaml, parsed));
                }
                crate::ocaml::parse_bytes_with_budget(
                    parse_symbol,
                    self.effective_budget(Scheme::OCaml),
                )
                .map(|parsed| Demangled::ocaml(Scheme::OCaml, parsed))
                .map_err(|error| budget_error(Scheme::OCaml, error.budget_kind()))
            }
            #[cfg(feature = "delphi")]
            crate::detection::Parsed::Delphi(parsed) => {
                // Detection parses Free Pascal names with the compiled-in
                // budget, so a configured budget re-parses the retained bytes
                // instead of reusing a tree that may exceed it.
                if self.budget == Budget::default() {
                    return Ok(Demangled::delphi(Scheme::Delphi, parsed));
                }
                crate::delphi::parse_bytes_with_budget(
                    parse_symbol,
                    self.effective_budget(Scheme::Delphi),
                )
                .map(|parsed| Demangled::delphi(Scheme::Delphi, parsed))
                .map_err(|error| budget_error(Scheme::Delphi, error.budget_kind()))
            }
            #[cfg(feature = "jni")]
            crate::detection::Parsed::Jni(parsed) => {
                // Detection parses JNI names with the compiled-in budget, so a
                // configured budget re-parses the retained bytes instead of
                // reusing a tree that may exceed it.
                if self.budget == Budget::default() {
                    return Ok(Demangled::jni(Scheme::Jni, parsed));
                }
                crate::jni::parse_bytes_with_budget(
                    parse_symbol,
                    self.effective_budget(Scheme::Jni),
                )
                .map(|parsed| Demangled::jni(Scheme::Jni, parsed))
                .map_err(|error| budget_error(Scheme::Jni, error.budget_kind()))
            }
            #[allow(
                unreachable_patterns,
                reason = "reachable when a backend feature is disabled"
            )]
            _ => Err(Error::InvalidSymbol(Scheme::Unknown)),
        }
    }

    /// The Rust budget the C++ fallback parses with.
    ///
    /// The fallback does not know the encoding before parsing, so it resolves
    /// the budget for the encoding the mangling prefix announces. That keeps
    /// v0's tighter input ceiling even when the failed candidate was C++.
    #[cfg(all(
        feature = "rust",
        any(feature = "cpp-itanium", feature = "cpp-msvc", feature = "cpp-gnu-v2")
    ))]
    fn rust_fallback_budget(&self, symbol: &[u8]) -> EffectiveBudget {
        let scheme = if crate::rust::is_v0_prefix_bytes(symbol) {
            Scheme::RustV0
        } else {
            Scheme::RustLegacy
        };
        self.effective_budget(scheme)
    }

    /// Dispatch an already selected scheme to its backend.
    ///
    /// A scheme is exact: a C++ dialect, rustc encoding or Swift mangling the
    /// backend did not produce is [`Error::InvalidSymbol`]. The detected path
    /// (`allow_rust_fallback`) keeps the historical C++ to Rust fallback: when
    /// a C++ candidate fails, the same bytes are parsed as a rustc symbol and
    /// reported with the precise encoding that parsed.
    #[expect(
        clippy::too_many_lines,
        reason = "one arm per scheme the dispatch routes"
    )]
    #[cfg_attr(
        not(any(
            feature = "rust",
            feature = "swift",
            feature = "go",
            feature = "objc",
            feature = "kotlin-native",
            feature = "d",
            feature = "jni",
            feature = "ada",
            feature = "fortran",
            feature = "delphi",
            feature = "ocaml",
            feature = "cpp-itanium",
            feature = "cpp-msvc",
            feature = "cpp-gnu-v2"
        )),
        expect(clippy::unused_self, reason = "no parsing backend is compiled")
    )]
    pub(crate) fn dispatch_bytes<'a>(
        &self,
        scheme: Scheme,
        symbol: &'a [u8],
        allow_rust_fallback: bool,
    ) -> Result<Demangled<'a>, Error> {
        #[cfg(not(all(
            feature = "rust",
            any(feature = "cpp-itanium", feature = "cpp-msvc", feature = "cpp-gnu-v2")
        )))]
        let _ = allow_rust_fallback;
        match scheme {
            Scheme::C | Scheme::Unknown => core::str::from_utf8(symbol).map_or_else(
                |_| Ok(Demangled::bytes(scheme, symbol)),
                |text| Ok(Demangled::plain(scheme, text)),
            ),
            #[cfg(feature = "rust")]
            Scheme::RustLegacy | Scheme::RustV0 => {
                let stripped = crate::scheme::strip_decorations_bytes(symbol);
                match crate::rust::parse_bytes_with_budget(stripped, self.effective_budget(scheme))
                {
                    Ok(parsed)
                        if crate::scheme::scheme_for_rust_encoding(parsed.encoding()) == scheme =>
                    {
                        Ok(Demangled::rust(scheme, parsed))
                    }
                    Err(error) => Err(budget_error(scheme, error.budget_kind())),
                    Ok(_) => Err(Error::InvalidSymbol(scheme)),
                }
            }
            #[cfg(any(feature = "cpp-itanium", feature = "cpp-msvc", feature = "cpp-gnu-v2"))]
            Scheme::CppItanium
            | Scheme::CppMsvc
            | Scheme::CppGnuV2
            | Scheme::CppArm
            | Scheme::CppEdg
            | Scheme::CppHp => {
                let stripped = crate::scheme::strip_thin_lto_bytes(symbol);
                let stripped = if stripped.starts_with(b"?") || stripped.starts_with(b".?") {
                    stripped
                } else {
                    crate::scheme::strip_version_suffix_bytes(stripped)
                };
                let cpp_error = match crate::cpp::parse_bytes_with_budget(
                    stripped,
                    self.effective_budget(scheme),
                ) {
                    Ok(parsed)
                        if crate::scheme::scheme_for_cpp_dialect(parsed.dialect()) == scheme =>
                    {
                        if parsed.print_envelope_ok(self.effective_budget(scheme))
                            && parsed.renders()
                        {
                            return Ok(Demangled::cpp(scheme, parsed));
                        }
                        None
                    }
                    Ok(_) => None,
                    Err(error) => Some(error),
                };
                #[cfg(feature = "rust")]
                if allow_rust_fallback
                    && let Ok(parsed) = crate::rust::parse_bytes_with_budget(
                        stripped,
                        self.rust_fallback_budget(stripped),
                    )
                {
                    let scheme = crate::scheme::scheme_for_rust_encoding(parsed.encoding());
                    return Ok(Demangled::rust(scheme, parsed));
                }
                Err(cpp_error.map_or(Error::InvalidSymbol(scheme), |error| {
                    budget_error(scheme, error.budget_kind())
                }))
            }
            #[cfg(feature = "swift")]
            Scheme::SwiftStable | Scheme::SwiftLegacy => {
                let stripped = crate::scheme::strip_thin_lto_bytes(symbol);
                match crate::swift::parse_bytes_with_budget(stripped, self.effective_budget(scheme))
                {
                    Ok(parsed)
                        if crate::scheme::scheme_for_swift_encoding(parsed.encoding())
                            == scheme =>
                    {
                        Ok(Demangled::swift(scheme, parsed))
                    }
                    _ => Err(Error::InvalidSymbol(scheme)),
                }
            }
            #[cfg(feature = "go")]
            Scheme::Go => crate::go::parse_bytes_with_budget(symbol, self.effective_budget(scheme))
                .map(|parsed| Demangled::go(scheme, parsed))
                .map_err(|error| budget_error(scheme, error.budget_kind())),
            #[cfg(feature = "kotlin-native")]
            Scheme::KotlinNative => {
                let stripped = crate::scheme::strip_thin_lto_bytes(symbol);
                crate::kotlin_native::parse_bytes_with_budget(
                    stripped,
                    self.effective_budget(scheme),
                )
                .map(|parsed| Demangled::kotlin_native(scheme, parsed))
                .map_err(|error| budget_error(scheme, error.budget_kind()))
            }
            #[cfg(feature = "d")]
            Scheme::D => {
                let parsed =
                    crate::d::parse_bytes_with_budget(symbol, self.effective_budget(scheme))
                        .map_err(|error| budget_error(scheme, error.budget_kind()))?;
                // `Demangled` keeps no D tree, so render once into owned text.
                // The printer has no output cap (the parse already charged
                // back-reference expansion to the expansion gates); only the
                // sink can fail.
                let mut sink = String::new();
                if core::fmt::Write::write_fmt(&mut sink, format_args!("{parsed}")).is_err() {
                    return Err(Error::InvalidSymbol(scheme));
                }
                Ok(Demangled::owned(scheme, sink))
            }
            #[cfg(feature = "jni")]
            Scheme::Jni => {
                let stripped = crate::scheme::strip_decorations_bytes(symbol);
                crate::jni::parse_bytes_with_budget(stripped, self.effective_budget(scheme))
                    .map(|parsed| Demangled::jni(scheme, parsed))
                    .map_err(|error| budget_error(scheme, error.budget_kind()))
            }
            #[cfg(feature = "ada")]
            Scheme::Ada => {
                crate::ada::parse_bytes_with_budget(symbol, self.effective_budget(scheme))
                    .map(|parsed| Demangled::ada(scheme, parsed))
                    .map_err(|error| budget_error(scheme, error.budget_kind()))
            }
            #[cfg(feature = "fortran")]
            Scheme::Fortran => {
                crate::fortran::parse_bytes_with_budget(symbol, self.effective_budget(scheme))
                    .map(|parsed| Demangled::fortran(scheme, parsed))
                    .map_err(|error| budget_error(scheme, error.budget_kind()))
            }
            #[cfg(feature = "delphi")]
            Scheme::Delphi => {
                crate::delphi::parse_bytes_with_budget(symbol, self.effective_budget(scheme))
                    .map(|parsed| Demangled::delphi(scheme, parsed))
                    .map_err(|error| budget_error(scheme, error.budget_kind()))
            }
            #[cfg(feature = "ocaml")]
            Scheme::OCaml => {
                crate::ocaml::parse_bytes_with_budget(symbol, self.effective_budget(scheme))
                    .map(|parsed| Demangled::ocaml(scheme, parsed))
                    .map_err(|error| budget_error(scheme, error.budget_kind()))
            }
            #[cfg(feature = "objc")]
            Scheme::ObjC => {
                let stripped = crate::scheme::strip_decorations_bytes(symbol);
                let stripped = stripped
                    .iter()
                    .position(|byte| *byte == b'@')
                    .map_or(stripped, |index| &stripped[..index]);
                crate::objc::parse_bytes_with_budget(stripped, self.effective_budget(scheme))
                    .map(|parsed| Demangled::objc(scheme, parsed))
                    .map_err(|error| budget_error(scheme, error.budget_kind()))
            }
            #[allow(
                unreachable_patterns,
                reason = "reachable when a backend feature is disabled"
            )]
            _ => Err(Error::InvalidSymbol(scheme)),
        }
    }
}

#[cfg(test)]
mod tests;
