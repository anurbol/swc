pub use self::{
    hygiene::{ExpnInfo, Mark, SyntaxContext},
    span_encoding::{Span, DUMMY_SP},
};
use crate::{rustc_data_structures::stable_hasher::StableHasher, sync::Lock, SourceMap};
use serde::{Deserialize, Serialize};
use std::{
    borrow::Cow,
    cell::Cell,
    cmp::{self, Ordering},
    fmt,
    hash::{Hash, Hasher},
    ops::{Add, Sub},
    path::PathBuf,
    sync::Arc,
};

mod analyze_source_file;
pub mod hygiene;
mod span_encoding;

pub struct Globals {
    span_interner: Lock<span_encoding::SpanInterner>,
    hygiene_data: Lock<hygiene::HygieneData>,
}

impl Globals {
    pub fn new() -> Globals {
        Globals {
            span_interner: Lock::new(span_encoding::SpanInterner::default()),
            hygiene_data: Lock::new(hygiene::HygieneData::new()),
        }
    }
}

// scoped_thread_local!(pub static GLOBALS: Globals);
pub static GLOBALS: ::scoped_tls::ScopedKey<Globals> = ::scoped_tls::ScopedKey {
    inner: {
        thread_local!(static FOO: ::std::cell::Cell<usize> = {
            ::std::cell::Cell::new(0)
        });
        &FOO
    },
    _marker: ::std::marker::PhantomData,
};

/// SourceMap used while serializing Span.
pub static CM: ::scoped_tls::ScopedKey<SourceMap> = ::scoped_tls::ScopedKey {
    inner: {
        thread_local!(static FOO: ::std::cell::Cell<usize> = {
            ::std::cell::Cell::new(0)
        });
        &FOO
    },
    _marker: ::std::marker::PhantomData,
};

/// Differentiates between real files and common virtual files.
#[derive(Debug, Eq, PartialEq, Clone, Ord, PartialOrd, Hash)]
pub enum FileName {
    Real(PathBuf),
    /// A macro.  This includes the full name of the macro, so that there are no
    /// clashes.
    Macros(String),
    /// call to `quote!`
    QuoteExpansion,
    /// Command line
    Anon,
    /// Hack in src/libsyntax/parse.rs
    /// FIXME(jseyfried)
    MacroExpansion,
    ProcMacroSourceCode,
    /// Custom sources for explicit parser calls from plugins and drivers
    Custom(String),
}

impl std::fmt::Display for FileName {
    fn fmt(&self, fmt: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match *self {
            FileName::Real(ref path) => write!(fmt, "{}", path.display()),
            FileName::Macros(ref name) => write!(fmt, "<{} macros>", name),
            FileName::QuoteExpansion => write!(fmt, "<quote expansion>"),
            FileName::MacroExpansion => write!(fmt, "<macro expansion>"),
            FileName::Anon => write!(fmt, "<anon>"),
            FileName::ProcMacroSourceCode => write!(fmt, "<proc-macro source code>"),
            FileName::Custom(ref s) => write!(fmt, "<{}>", s),
        }
    }
}

impl From<PathBuf> for FileName {
    fn from(p: PathBuf) -> Self {
        assert!(!p.to_string_lossy().ends_with('>'));
        FileName::Real(p)
    }
}

impl FileName {
    pub fn is_real(&self) -> bool {
        match *self {
            FileName::Real(_) => true,
            FileName::Macros(_)
            | FileName::Anon
            | FileName::MacroExpansion
            | FileName::ProcMacroSourceCode
            | FileName::Custom(_)
            | FileName::QuoteExpansion => false,
        }
    }

    pub fn is_macros(&self) -> bool {
        match *self {
            FileName::Real(_)
            | FileName::Anon
            | FileName::MacroExpansion
            | FileName::ProcMacroSourceCode
            | FileName::Custom(_)
            | FileName::QuoteExpansion => false,
            FileName::Macros(_) => true,
        }
    }
}

/// Spans represent a region of code, used for error reporting. Positions in
/// spans are *absolute* positions from the beginning of the source_map, not
/// positions relative to SourceFiles. Methods on the SourceMap can be used to
/// relate spans back to the original source.
/// You must be careful if the span crosses more than one file - you will not be
/// able to use many of the functions on spans in source_map and you cannot
/// assume that the length of the span = hi - lo; there may be space in the
/// BytePos range between files.
///
/// `SpanData` is public because `Span` uses a thread-local interner and can't
/// be sent to other threads, but some pieces of performance infra run in a
/// separate thread. Using `Span` is generally preferred.
#[derive(Clone, Copy, Hash, PartialEq, Eq, Ord, PartialOrd, Serialize, Deserialize)]
pub struct SpanData {
    #[serde(rename = "start")]
    pub lo: BytePos,
    #[serde(rename = "end")]
    pub hi: BytePos,
    /// Information about where the macro came from, if this piece of
    /// code was created by a macro expansion.
    pub ctxt: SyntaxContext,
}

impl SpanData {
    #[inline]
    pub fn with_lo(&self, lo: BytePos) -> Span {
        Span::new(lo, self.hi, self.ctxt)
    }
    #[inline]
    pub fn with_hi(&self, hi: BytePos) -> Span {
        Span::new(self.lo, hi, self.ctxt)
    }
    #[inline]
    pub fn with_ctxt(&self, ctxt: SyntaxContext) -> Span {
        Span::new(self.lo, self.hi, ctxt)
    }
}

impl PartialOrd for Span {
    fn partial_cmp(&self, rhs: &Self) -> Option<Ordering> {
        PartialOrd::partial_cmp(&self.data(), &rhs.data())
    }
}
impl Ord for Span {
    fn cmp(&self, rhs: &Self) -> Ordering {
        Ord::cmp(&self.data(), &rhs.data())
    }
}

/// A collection of spans. Spans have two orthogonal attributes:
///
/// - they can be *primary spans*. In this case they are the locus of the error,
///   and would be rendered with `^^^`.
/// - they can have a *label*. In this case, the label is written next to the
///   mark in the snippet when we render.
#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub struct MultiSpan {
    primary_spans: Vec<Span>,
    span_labels: Vec<(Span, String)>,
}

impl Span {
    #[inline]
    pub fn lo(self) -> BytePos {
        self.data().lo
    }
    #[inline]
    pub fn with_lo(self, lo: BytePos) -> Span {
        self.data().with_lo(lo)
    }
    #[inline]
    pub fn hi(self) -> BytePos {
        self.data().hi
    }
    #[inline]
    pub fn with_hi(self, hi: BytePos) -> Span {
        self.data().with_hi(hi)
    }
    #[inline]
    pub fn ctxt(self) -> SyntaxContext {
        self.data().ctxt
    }
    #[inline]
    pub fn with_ctxt(self, ctxt: SyntaxContext) -> Span {
        self.data().with_ctxt(ctxt)
    }

    /// Returns `true` if this is a dummy span with any hygienic context.
    #[inline]
    pub fn is_dummy(self) -> bool {
        let span = self.data();
        span.lo.0 == 0 && span.hi.0 == 0
    }

    /// Returns a new span representing an empty span at the beginning of this
    /// span
    #[inline]
    pub fn shrink_to_lo(self) -> Span {
        let span = self.data();
        span.with_hi(span.lo)
    }
    /// Returns a new span representing an empty span at the end of this span
    #[inline]
    pub fn shrink_to_hi(self) -> Span {
        let span = self.data();
        span.with_lo(span.hi)
    }

    /// Returns `self` if `self` is not the dummy span, and `other` otherwise.
    pub fn substitute_dummy(self, other: Span) -> Span {
        if self.is_dummy() {
            other
        } else {
            self
        }
    }

    /// Return true if `self` fully encloses `other`.
    pub fn contains(self, other: Span) -> bool {
        let span = self.data();
        let other = other.data();
        span.lo <= other.lo && other.hi <= span.hi
    }

    /// Return true if the spans are equal with regards to the source text.
    ///
    /// Use this instead of `==` when either span could be generated code,
    /// and you only care that they point to the same bytes of source text.
    pub fn source_equal(self, other: Span) -> bool {
        let span = self.data();
        let other = other.data();
        span.lo == other.lo && span.hi == other.hi
    }

    /// Returns `Some(span)`, where the start is trimmed by the end of `other`
    pub fn trim_start(self, other: Span) -> Option<Span> {
        let span = self.data();
        let other = other.data();
        if span.hi > other.hi {
            Some(span.with_lo(cmp::max(span.lo, other.hi)))
        } else {
            None
        }
    }

    /// Return the source span - this is either the supplied span, or the span
    /// for the macro callsite that expanded to it.
    pub fn source_callsite(self) -> Span {
        self.ctxt()
            .outer()
            .expn_info()
            .map(|info| info.call_site.source_callsite())
            .unwrap_or(self)
    }

    /// The `Span` for the tokens in the previous macro expansion from which
    /// `self` was generated, if any
    pub fn parent(self) -> Option<Span> {
        self.ctxt().outer().expn_info().map(|i| i.call_site)
    }

    /// Return the source callee.
    ///
    /// Returns `None` if the supplied span has no expansion trace,
    /// else returns the `ExpnInfo` for the macro definition
    /// corresponding to the source callsite.
    pub fn source_callee(self) -> Option<ExpnInfo> {
        fn source_callee(info: ExpnInfo) -> ExpnInfo {
            match info.call_site.ctxt().outer().expn_info() {
                Some(info) => source_callee(info),
                None => info,
            }
        }
        self.ctxt().outer().expn_info().map(source_callee)
    }

    /// Check if a span is "internal" to a macro in which #[unstable]
    /// items can be used (that is, a macro marked with
    /// `#[allow_internal_unstable]`).
    pub fn allows_unstable(self) -> bool {
        match self.ctxt().outer().expn_info() {
            Some(info) => info.allow_internal_unstable,
            None => false,
        }
    }
    /// Return a `Span` that would enclose both `self` and `end`.
    pub fn to(self, end: Span) -> Span {
        let span_data = self.data();
        let end_data = end.data();
        // FIXME(jseyfried): self.ctxt should always equal end.ctxt here (c.f. issue
        // #23480) Return the macro span on its own to avoid weird diagnostic
        // output. It is preferable to have an incomplete span than a completely
        // nonsensical one.
        if span_data.ctxt != end_data.ctxt {
            if span_data.ctxt == SyntaxContext::empty() {
                return end;
            } else if end_data.ctxt == SyntaxContext::empty() {
                return self;
            }
            // both span fall within a macro
            // FIXME(estebank) check if it is the *same* macro
        }
        Span::new(
            cmp::min(span_data.lo, end_data.lo),
            cmp::max(span_data.hi, end_data.hi),
            if span_data.ctxt == SyntaxContext::empty() {
                end_data.ctxt
            } else {
                span_data.ctxt
            },
        )
    }

    /// Return a `Span` between the end of `self` to the beginning of `end`.
    pub fn between(self, end: Span) -> Span {
        let span = self.data();
        let end = end.data();
        Span::new(
            span.hi,
            end.lo,
            if end.ctxt == SyntaxContext::empty() {
                end.ctxt
            } else {
                span.ctxt
            },
        )
    }

    /// Return a `Span` between the beginning of `self` to the beginning of
    /// `end`.
    pub fn until(self, end: Span) -> Span {
        let span = self.data();
        let end = end.data();
        Span::new(
            span.lo,
            end.lo,
            if end.ctxt == SyntaxContext::empty() {
                end.ctxt
            } else {
                span.ctxt
            },
        )
    }

    pub fn from_inner_byte_pos(self, start: usize, end: usize) -> Span {
        let span = self.data();
        Span::new(
            span.lo + BytePos::from_usize(start),
            span.lo + BytePos::from_usize(end),
            span.ctxt,
        )
    }

    #[inline]
    pub fn apply_mark(self, mark: Mark) -> Span {
        let span = self.data();
        span.with_ctxt(span.ctxt.apply_mark(mark))
    }

    #[inline]
    pub fn remove_mark(&mut self) -> Mark {
        let mut span = self.data();
        let mark = span.ctxt.remove_mark();
        *self = Span::new(span.lo, span.hi, span.ctxt);
        mark
    }

    #[inline]
    pub fn adjust(&mut self, expansion: Mark) -> Option<Mark> {
        let mut span = self.data();
        let mark = span.ctxt.adjust(expansion);
        *self = Span::new(span.lo, span.hi, span.ctxt);
        mark
    }

    #[inline]
    pub fn glob_adjust(
        &mut self,
        expansion: Mark,
        glob_ctxt: SyntaxContext,
    ) -> Option<Option<Mark>> {
        let mut span = self.data();
        let mark = span.ctxt.glob_adjust(expansion, glob_ctxt);
        *self = Span::new(span.lo, span.hi, span.ctxt);
        mark
    }

    #[inline]
    pub fn reverse_glob_adjust(
        &mut self,
        expansion: Mark,
        glob_ctxt: SyntaxContext,
    ) -> Option<Option<Mark>> {
        let mut span = self.data();
        let mark = span.ctxt.reverse_glob_adjust(expansion, glob_ctxt);
        *self = Span::new(span.lo, span.hi, span.ctxt);
        mark
    }

    #[inline]
    pub fn modern(self) -> Span {
        let span = self.data();
        span.with_ctxt(span.ctxt.modern())
    }

    #[inline]
    pub fn modern_and_legacy(self) -> Span {
        let span = self.data();
        span.with_ctxt(span.ctxt.modern_and_legacy())
    }
}

#[derive(Clone, Debug)]
pub struct SpanLabel {
    /// The span we are going to include in the final snippet.
    pub span: Span,

    /// Is this a primary span? This is the "locus" of the message,
    /// and is indicated with a `^^^^` underline, versus `----`.
    pub is_primary: bool,

    /// What label should we attach to this span (if any)?
    pub label: Option<String>,
}

impl Default for Span {
    fn default() -> Self {
        DUMMY_SP
    }
}

fn default_span_debug(span: Span, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    let span = span.data();
    f.debug_struct("Span")
        .field("lo", &span.lo)
        .field("hi", &span.hi)
        .field("ctxt", &span.ctxt)
        .finish()
}

impl fmt::Debug for Span {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        SPAN_DEBUG.with(|span_debug| span_debug.get()(*self, f))
    }
}

impl fmt::Debug for SpanData {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        SPAN_DEBUG.with(|span_debug| span_debug.get()(Span::new(self.lo, self.hi, self.ctxt), f))
    }
}

impl MultiSpan {
    #[inline]
    pub fn new() -> MultiSpan {
        MultiSpan {
            primary_spans: vec![],
            span_labels: vec![],
        }
    }

    pub fn from_span(primary_span: Span) -> MultiSpan {
        MultiSpan {
            primary_spans: vec![primary_span],
            span_labels: vec![],
        }
    }

    pub fn from_spans(vec: Vec<Span>) -> MultiSpan {
        MultiSpan {
            primary_spans: vec,
            span_labels: vec![],
        }
    }

    pub fn push_span_label(&mut self, span: Span, label: String) {
        self.span_labels.push((span, label));
    }

    /// Selects the first primary span (if any)
    pub fn primary_span(&self) -> Option<Span> {
        self.primary_spans.first().cloned()
    }

    /// Returns all primary spans.
    pub fn primary_spans(&self) -> &[Span] {
        &self.primary_spans
    }

    /// Returns `true` if this contains only a dummy primary span with any
    /// hygienic context.
    pub fn is_dummy(&self) -> bool {
        let mut is_dummy = true;
        for span in &self.primary_spans {
            if !span.is_dummy() {
                is_dummy = false;
            }
        }
        is_dummy
    }

    /// Replaces all occurrences of one Span with another. Used to move Spans in
    /// areas that don't display well (like std macros). Returns true if
    /// replacements occurred.
    pub fn replace(&mut self, before: Span, after: Span) -> bool {
        let mut replacements_occurred = false;
        for primary_span in &mut self.primary_spans {
            if *primary_span == before {
                *primary_span = after;
                replacements_occurred = true;
            }
        }
        for span_label in &mut self.span_labels {
            if span_label.0 == before {
                span_label.0 = after;
                replacements_occurred = true;
            }
        }
        replacements_occurred
    }

    /// Returns the strings to highlight. We always ensure that there
    /// is an entry for each of the primary spans -- for each primary
    /// span P, if there is at least one label with span P, we return
    /// those labels (marked as primary). But otherwise we return
    /// `SpanLabel` instances with empty labels.
    pub fn span_labels(&self) -> Vec<SpanLabel> {
        let is_primary = |span| self.primary_spans.contains(&span);

        let mut span_labels = self
            .span_labels
            .iter()
            .map(|&(span, ref label)| SpanLabel {
                span,
                is_primary: is_primary(span),
                label: Some(label.clone()),
            })
            .collect::<Vec<_>>();

        for &span in &self.primary_spans {
            if !span_labels.iter().any(|sl| sl.span == span) {
                span_labels.push(SpanLabel {
                    span,
                    is_primary: true,
                    label: None,
                });
            }
        }

        span_labels
    }
}

impl From<Span> for MultiSpan {
    fn from(span: Span) -> MultiSpan {
        MultiSpan::from_span(span)
    }
}

impl From<Vec<Span>> for MultiSpan {
    fn from(spans: Vec<Span>) -> MultiSpan {
        MultiSpan::from_spans(spans)
    }
}

pub const NO_EXPANSION: SyntaxContext = SyntaxContext::empty();

/// Identifies an offset of a multi-byte character in a SourceFile
#[derive(Copy, Clone, Eq, PartialEq, Debug)]
pub struct MultiByteChar {
    /// The absolute offset of the character in the SourceMap
    pub pos: BytePos,
    /// The number of bytes, >=2
    pub bytes: u8,
}

/// Identifies an offset of a non-narrow character in a SourceFile
#[derive(Copy, Clone, Eq, PartialEq, Debug)]
pub enum NonNarrowChar {
    /// Represents a zero-width character
    ZeroWidth(BytePos),
    /// Represents a wide (fullwidth) character
    Wide(BytePos),
    /// Represents a tab character, represented visually with a width of 4
    /// characters
    Tab(BytePos),
}

impl NonNarrowChar {
    fn new(pos: BytePos, width: usize) -> Self {
        match width {
            0 => NonNarrowChar::ZeroWidth(pos),
            2 => NonNarrowChar::Wide(pos),
            4 => NonNarrowChar::Tab(pos),
            _ => panic!("width {} given for non-narrow character", width),
        }
    }

    /// Returns the absolute offset of the character in the SourceMap
    pub fn pos(self) -> BytePos {
        match self {
            NonNarrowChar::ZeroWidth(p) | NonNarrowChar::Wide(p) | NonNarrowChar::Tab(p) => p,
        }
    }

    /// Returns the width of the character, 0 (zero-width) or 2 (wide)
    pub fn width(self) -> usize {
        match self {
            NonNarrowChar::ZeroWidth(_) => 0,
            NonNarrowChar::Wide(_) => 2,
            NonNarrowChar::Tab(_) => 4,
        }
    }
}

impl Add<BytePos> for NonNarrowChar {
    type Output = Self;

    fn add(self, rhs: BytePos) -> Self {
        match self {
            NonNarrowChar::ZeroWidth(pos) => NonNarrowChar::ZeroWidth(pos + rhs),
            NonNarrowChar::Wide(pos) => NonNarrowChar::Wide(pos + rhs),
            NonNarrowChar::Tab(pos) => NonNarrowChar::Tab(pos + rhs),
        }
    }
}

impl Sub<BytePos> for NonNarrowChar {
    type Output = Self;

    fn sub(self, rhs: BytePos) -> Self {
        match self {
            NonNarrowChar::ZeroWidth(pos) => NonNarrowChar::ZeroWidth(pos - rhs),
            NonNarrowChar::Wide(pos) => NonNarrowChar::Wide(pos - rhs),
            NonNarrowChar::Tab(pos) => NonNarrowChar::Tab(pos - rhs),
        }
    }
}

/// A single source in the SourceMap.
#[derive(Clone)]
pub struct SourceFile {
    /// The name of the file that the source came from, source that doesn't
    /// originate from files has names between angle brackets by convention,
    /// e.g. `<anon>`
    pub name: FileName,
    /// True if the `name` field above has been modified by --remap-path-prefix
    pub name_was_remapped: bool,
    /// The unmapped path of the file that the source came from.
    /// Set to `None` if the SourceFile was imported from an external crate.
    pub unmapped_path: Option<FileName>,
    /// Indicates which crate this SourceFile was imported from.
    pub crate_of_origin: u32,
    /// The complete source code
    pub src: Arc<String>,
    /// The source code's hash
    pub src_hash: u128,
    /// The start position of this source in the SourceMap
    pub start_pos: BytePos,
    /// The end position of this source in the SourceMap
    pub end_pos: BytePos,
    /// Locations of lines beginnings in the source code
    pub lines: Vec<BytePos>,
    /// Locations of multi-byte characters in the source code
    pub multibyte_chars: Vec<MultiByteChar>,
    /// Width of characters that are not narrow in the source code
    pub non_narrow_chars: Vec<NonNarrowChar>,
    /// A hash of the filename, used for speeding up the incr. comp. hashing.
    pub name_hash: u128,
}

impl fmt::Debug for SourceFile {
    fn fmt(&self, fmt: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(fmt, "SourceFile({})", self.name)
    }
}

impl SourceFile {
    pub fn new(
        name: FileName,
        name_was_remapped: bool,
        unmapped_path: FileName,
        mut src: String,
        start_pos: BytePos,
    ) -> SourceFile {
        remove_bom(&mut src);

        let src_hash = {
            let mut hasher: StableHasher<u128> = StableHasher::new();
            hasher.write(src.as_bytes());
            hasher.finish()
        };
        let name_hash = {
            let mut hasher: StableHasher<u128> = StableHasher::new();
            name.hash(&mut hasher);
            hasher.finish()
        };
        let end_pos = start_pos.to_usize() + src.len();

        let (lines, multibyte_chars, non_narrow_chars) =
            analyze_source_file::analyze_source_file(&src[..], start_pos);

        SourceFile {
            name,
            name_was_remapped,
            unmapped_path: Some(unmapped_path),
            crate_of_origin: 0,
            src: Arc::new(src),
            src_hash,
            start_pos,
            end_pos: Pos::from_usize(end_pos),
            lines,
            multibyte_chars,
            non_narrow_chars,
            name_hash,
        }
    }

    /// Return the BytePos of the beginning of the current line.
    pub fn line_begin_pos(&self, pos: BytePos) -> BytePos {
        let line_index = self.lookup_line(pos).unwrap();
        self.lines[line_index]
    }

    /// Get a line from the list of pre-computed line-beginnings.
    /// The line number here is 0-based.
    pub fn get_line(&self, line_number: usize) -> Option<Cow<'_, str>> {
        fn get_until_newline(src: &str, begin: usize) -> &str {
            // We can't use `lines.get(line_number+1)` because we might
            // be parsing when we call this function and thus the current
            // line is the last one we have line info for.
            let slice = &src[begin..];
            match slice.find('\n') {
                Some(e) => &slice[..e],
                None => slice,
            }
        }

        let begin = {
            let line = if let Some(line) = self.lines.get(line_number) {
                line
            } else {
                return None;
            };
            let begin: BytePos = *line - self.start_pos;
            begin.to_usize()
        };

        Some(Cow::from(get_until_newline(&self.src, begin)))
    }

    pub fn is_real_file(&self) -> bool {
        self.name.is_real()
    }

    pub fn byte_length(&self) -> u32 {
        self.end_pos.0 - self.start_pos.0
    }
    pub fn count_lines(&self) -> usize {
        self.lines.len()
    }

    /// Find the line containing the given position. The return value is the
    /// index into the `lines` array of this SourceFile, not the 1-based line
    /// number. If the source_file is empty or the position is located before
    /// the first line, None is returned.
    pub fn lookup_line(&self, pos: BytePos) -> Option<usize> {
        if self.lines.is_empty() {
            return None;
        }

        let line_index = lookup_line(&self.lines[..], pos);
        assert!(line_index < self.lines.len() as isize);
        if line_index >= 0 {
            Some(line_index as usize)
        } else {
            None
        }
    }

    pub fn line_bounds(&self, line_index: usize) -> (BytePos, BytePos) {
        if self.start_pos == self.end_pos {
            return (self.start_pos, self.end_pos);
        }

        assert!(line_index < self.lines.len());
        if line_index == (self.lines.len() - 1) {
            (self.lines[line_index], self.end_pos)
        } else {
            (self.lines[line_index], self.lines[line_index + 1])
        }
    }

    #[inline]
    pub fn contains(&self, byte_pos: BytePos) -> bool {
        byte_pos >= self.start_pos && byte_pos <= self.end_pos
    }
}

/// Remove utf-8 BOM if any.
fn remove_bom(src: &mut String) {
    if src.starts_with("\u{feff}") {
        src.drain(..3);
    }
}

// _____________________________________________________________________________
// Pos, BytePos, CharPos
//

pub trait Pos {
    fn from_usize(n: usize) -> Self;
    fn to_usize(&self) -> usize;
    fn from_u32(n: u32) -> Self;
    fn to_u32(&self) -> u32;
}

/// A byte offset. Keep this small (currently 32-bits), as AST contains
/// a lot of them.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug, Serialize, Deserialize)]
#[serde(transparent)]
pub struct BytePos(pub u32);

/// A character offset. Because of multibyte utf8 characters, a byte offset
/// is not equivalent to a character offset. The SourceMap will convert BytePos
/// values to CharPos values as necessary.
#[derive(Copy, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub struct CharPos(pub usize);

// FIXME: Lots of boilerplate in these impls, but so far my attempts to fix
// have been unsuccessful

impl Pos for BytePos {
    #[inline(always)]
    fn from_usize(n: usize) -> BytePos {
        BytePos(n as u32)
    }

    #[inline(always)]
    fn to_usize(&self) -> usize {
        self.0 as usize
    }

    #[inline(always)]
    fn from_u32(n: u32) -> BytePos {
        BytePos(n)
    }

    #[inline(always)]
    fn to_u32(&self) -> u32 {
        self.0
    }
}

impl Add for BytePos {
    type Output = BytePos;

    #[inline(always)]
    fn add(self, rhs: BytePos) -> BytePos {
        BytePos((self.to_usize() + rhs.to_usize()) as u32)
    }
}

impl Sub for BytePos {
    type Output = BytePos;

    #[inline(always)]
    fn sub(self, rhs: BytePos) -> BytePos {
        BytePos((self.to_usize() - rhs.to_usize()) as u32)
    }
}

impl Pos for CharPos {
    #[inline(always)]
    fn from_usize(n: usize) -> CharPos {
        CharPos(n)
    }

    #[inline(always)]
    fn to_usize(&self) -> usize {
        self.0
    }

    #[inline(always)]
    fn from_u32(n: u32) -> CharPos {
        CharPos(n as usize)
    }

    #[inline(always)]
    fn to_u32(&self) -> u32 {
        self.0 as u32
    }
}

impl Add for CharPos {
    type Output = CharPos;

    #[inline(always)]
    fn add(self, rhs: CharPos) -> CharPos {
        CharPos(self.to_usize() + rhs.to_usize())
    }
}

impl Sub for CharPos {
    type Output = CharPos;

    #[inline(always)]
    fn sub(self, rhs: CharPos) -> CharPos {
        CharPos(self.to_usize() - rhs.to_usize())
    }
}

// _____________________________________________________________________________
// Loc, LocWithOpt, SourceFileAndLine, SourceFileAndBytePos
//

/// A source code location used for error reporting
#[derive(Debug, Clone)]
pub struct Loc {
    /// Information about the original source
    pub file: Arc<SourceFile>,
    /// The (1-based) line number
    pub line: usize,
    /// The (0-based) column offset
    pub col: CharPos,
    /// The (0-based) column offset when displayed
    pub col_display: usize,
}

/// A source code location used as the result of lookup_char_pos_adj
// Actually, *none* of the clients use the filename *or* file field;
// perhaps they should just be removed.
#[derive(Debug)]
pub struct LocWithOpt {
    pub filename: FileName,
    pub line: usize,
    pub col: CharPos,
    pub file: Option<Arc<SourceFile>>,
}

// used to be structural records. Better names, anyone?
#[derive(Debug)]
pub struct SourceFileAndLine {
    pub sf: Arc<SourceFile>,
    pub line: usize,
}
#[derive(Debug)]
pub struct SourceFileAndBytePos {
    pub sf: Arc<SourceFile>,
    pub pos: BytePos,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct LineInfo {
    /// Index of line, starting from 0.
    pub line_index: usize,

    /// Column in line where span begins, starting from 0.
    pub start_col: CharPos,

    /// Column in line where span ends, starting from 0, exclusive.
    pub end_col: CharPos,
}

pub struct FileLines {
    pub file: Arc<SourceFile>,
    pub lines: Vec<LineInfo>,
}

thread_local!(pub static SPAN_DEBUG: Cell<fn(Span, &mut fmt::Formatter<'_>) -> fmt::Result> =
                Cell::new(default_span_debug));

// _____________________________________________________________________________
// SpanLinesError, SpanSnippetError, DistinctSources,
// MalformedSourceMapPositions
//

pub type FileLinesResult = Result<FileLines, SpanLinesError>;

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum SpanLinesError {
    IllFormedSpan(Span),
    DistinctSources(DistinctSources),
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum SpanSnippetError {
    IllFormedSpan(Span),
    DistinctSources(DistinctSources),
    MalformedForSourcemap(MalformedSourceMapPositions),
    SourceNotAvailable { filename: FileName },
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct DistinctSources {
    pub begin: (FileName, BytePos),
    pub end: (FileName, BytePos),
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct MalformedSourceMapPositions {
    pub name: FileName,
    pub source_len: usize,
    pub begin_pos: BytePos,
    pub end_pos: BytePos,
}

// Given a slice of line start positions and a position, returns the index of
// the line the position is on. Returns -1 if the position is located before
// the first line.
fn lookup_line(lines: &[BytePos], pos: BytePos) -> isize {
    match lines.binary_search(&pos) {
        Ok(line) => line as isize,
        Err(line) => line as isize - 1,
    }
}

#[cfg(test)]
mod tests {
    use super::{lookup_line, BytePos};

    #[test]
    fn test_lookup_line() {
        let lines = &[BytePos(3), BytePos(17), BytePos(28)];

        assert_eq!(lookup_line(lines, BytePos(0)), -1);
        assert_eq!(lookup_line(lines, BytePos(3)), 0);
        assert_eq!(lookup_line(lines, BytePos(4)), 0);

        assert_eq!(lookup_line(lines, BytePos(16)), 0);
        assert_eq!(lookup_line(lines, BytePos(17)), 1);
        assert_eq!(lookup_line(lines, BytePos(18)), 1);

        assert_eq!(lookup_line(lines, BytePos(28)), 2);
        assert_eq!(lookup_line(lines, BytePos(29)), 2);
    }
}
