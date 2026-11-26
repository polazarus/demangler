use core::fmt::Write as _;
use std::string::String;

use super::combinator::{decimal_length, tag};
use super::cursor::{Cursor, Limits, State};
use super::error::{Fail, FailKind, FailMode};
use super::escape::{Escape, write_escaped};
use super::output::LimitedWriter;
use super::parser::Parser;
use super::search::{find, rfind};
use super::span::Span;

fn make_state(work: u32) -> State {
    State::new(Limits {
        input: 1024,
        depth: 8,
        work,
    })
}

#[test]
fn tag_matches_a_whole_literal_and_consumes_it() {
    let mut state = make_state(100);
    let mut cursor = Cursor::new(b"abc", 0, &mut state).unwrap();
    assert_eq!(tag(b"ab").parse(&mut cursor).unwrap(), Span::new(0, 2));
    assert_eq!(cursor.pos(), 2);
    assert!(tag(b"a").parse(&mut cursor).is_err());
    assert_eq!(cursor.pos(), 2, "a failed tag leaves the cursor alone");
}

#[test]
fn decimal_length_rejects_empty_input_and_overflow() {
    let mut state = make_state(100);
    let mut cursor = Cursor::new(b"12x", 0, &mut state).unwrap();
    assert_eq!(decimal_length().parse(&mut cursor).unwrap(), 12);
    assert_eq!(cursor.pos(), 2);

    let mut state = make_state(100);
    let mut cursor = Cursor::new(b"x", 0, &mut state).unwrap();
    let fail = decimal_length().parse(&mut cursor).unwrap_err();
    assert_eq!(fail.mode, FailMode::Backtrack);

    let mut state = make_state(100);
    let mut cursor = Cursor::new(b"99999999999999999999999999", 0, &mut state).unwrap();
    let fail = decimal_length().parse(&mut cursor).unwrap_err();
    assert_eq!(fail.mode, FailMode::Cut);
    assert_eq!(fail.kind, FailKind::Invalid);
}

#[test]
fn cursor_reads_bytes_without_consuming_them() {
    let mut state = make_state(100);
    let mut cursor = Cursor::new(b"abcd", 0, &mut state).unwrap();
    assert_eq!(cursor.remaining(), b"abcd");
    assert_eq!(cursor.peek(), Some(b'a'));
    assert_eq!(cursor.peek_at(1), Some(b'b'));
    assert_eq!(cursor.peek_at(4), None);
    assert!(cursor.starts_with(b"ab"));
    assert!(!cursor.starts_with(b"b"));

    assert_eq!(cursor.advance(2).unwrap(), b"ab");
    assert_eq!(cursor.remaining(), b"cd");
    assert_eq!(cursor.bytes_between(1, 3).unwrap(), b"bc");
    assert_eq!(cursor.bytes_between(2, 5), None);

    // A checkpoint restores position and depth; consumed work is not
    // refunded.
    let checkpoint = cursor.checkpoint();
    cursor.enter().unwrap();
    cursor.leave();
    cursor.reset(checkpoint);
    assert_eq!(cursor.pos(), 2);
}

#[test]
fn spans_and_offsets_include_the_cursor_base() {
    let mut state = make_state(100);
    let mut cursor = Cursor::new(b"ab", 7, &mut state).unwrap();
    assert_eq!(cursor.offset(), 7);
    assert_eq!(tag(b"a").parse(&mut cursor).unwrap(), Span::new(7, 8));
    assert_eq!(cursor.offset(), 8);
}

#[test]
fn input_and_depth_budgets_are_enforced() {
    let mut state = make_state(100);
    let fail = Cursor::new(&[0_u8; 1025], 0, &mut state).unwrap_err();
    assert_eq!(fail.mode, FailMode::Cut);
    assert_eq!(fail.kind, FailKind::InputBudget);

    let mut state = make_state(100);
    let mut cursor = Cursor::new(b"", 0, &mut state).unwrap();
    for _ in 0..8 {
        cursor.enter().unwrap();
    }
    let fail = cursor.enter().unwrap_err();
    assert_eq!(fail.mode, FailMode::Cut);
    assert_eq!(fail.kind, FailKind::Depth);
}

#[test]
fn work_budget_is_enforced() {
    let mut state = make_state(1);
    let mut cursor = Cursor::new(b"abc", 0, &mut state).unwrap();
    let fail = cursor.advance(3).unwrap_err();
    assert_eq!(fail.kind, FailKind::Work);
    assert_eq!(fail.offset, 0);
}

#[test]
fn every_cursor_failure_carries_its_kind() {
    // The cursor builds each failure from the constructor that names the bound
    // it tripped, so a backend maps a kind instead of matching a label.
    let mut state = make_state(100);
    let mut cursor = Cursor::new(b"ab", 0, &mut state).unwrap();
    let fail = cursor.advance(3).unwrap_err();
    assert_eq!(fail.kind, FailKind::EndOfInput);
    assert_eq!(fail.mode, FailMode::Backtrack);

    // A committed grammar failure is `Invalid`; only the budget
    // constructors introduce another kind.
    let fail = tag(b"z").parse(&mut cursor).unwrap_err();
    assert_eq!(fail.kind, FailKind::Invalid);
    assert_eq!(
        decimal_length().parse(&mut cursor).unwrap_err().kind,
        FailKind::Invalid
    );
    assert_eq!(Fail::backtrack(0).kind, FailKind::Invalid);
    assert_eq!(Fail::cut(0).kind, FailKind::Invalid);
}

#[test]
fn find_and_rfind_report_the_first_and_last_occurrence() {
    assert_eq!(find(b"a.b.c", b"."), Some(1));
    assert_eq!(rfind(b"a.b.c", b"."), Some(3));
    assert_eq!(find(b"abcabc", b"abc"), Some(0));
    assert_eq!(rfind(b"abcabc", b"abc"), Some(3));
    assert_eq!(find(b"abc", b"bc"), Some(1));
    assert_eq!(rfind(b"abc", b"bc"), Some(1));
    assert_eq!(find(b"abc", b"abcd"), None);
    assert_eq!(rfind(b"abc", b"abcd"), None);
    assert_eq!(find(b"", b"a"), None);

    // An empty needle has no offset: rejecting it also keeps a zero-width
    // `windows` call out of the callers.
    assert_eq!(find(b"abc", b""), None);
    assert_eq!(rfind(b"abc", b""), None);
    assert_eq!(find(b"", b""), None);
}

#[test]
fn limited_writer_enforces_the_cap_exactly() {
    let mut out = String::new();
    {
        let mut writer = LimitedWriter::new(&mut out, 4);
        writer.write_str("abc").unwrap();
        // A write that lands exactly on the cap is accepted.
        writer.write_str("d").unwrap();
        // One byte past the cap is rejected and not forwarded.
        assert!(writer.write_str("e").is_err());
    }
    assert_eq!(out, "abcd");

    // An over-cap write is rejected whole: no partial text reaches the sink.
    let mut out = String::new();
    {
        let mut writer = LimitedWriter::new(&mut out, 3);
        assert!(writer.write_str("abcd").is_err());
    }
    assert!(out.is_empty());
}

#[test]
fn limited_writer_guards_the_byte_count_overflow() {
    let mut out = String::new();
    {
        let mut writer = LimitedWriter::with_written(&mut out, usize::MAX, usize::MAX);
        assert!(writer.write_str("a").is_err());
    }
    assert!(out.is_empty());
}

#[test]
fn escape_policies_render_bytes_reversibly() {
    let mut identifier = String::new();
    write_escaped(&mut identifier, b"a\\b\xffc", Escape::Identifier).unwrap();
    assert_eq!(identifier, "a\\\\b\\xffc");

    // Payload spelling keeps a literal backslash and only escapes invalid
    // bytes.
    let mut verbatim = String::new();
    write_escaped(&mut verbatim, b"a\\b\xffc", Escape::Verbatim).unwrap();
    assert_eq!(verbatim, "a\\b\\xffc");

    // Valid multi-byte UTF-8 is passed through untouched by both policies.
    let mut utf8 = String::new();
    write_escaped(&mut utf8, "é".as_bytes(), Escape::Identifier).unwrap();
    assert_eq!(utf8, "é");
}

#[cfg(any(
    feature = "cpp-itanium",
    feature = "cpp-msvc",
    feature = "cpp-gnu-v2",
    feature = "swift",
    feature = "go",
    feature = "objc",
    feature = "kotlin-native"
))]
#[test]
fn text_borrows_only_when_the_policy_rewrites_nothing() {
    use std::borrow::Cow;

    use super::text::{owned, text};

    // An identifier borrows the input when it holds no backslash and no
    // control character; those are rebuilt as owned, reversible text.
    assert!(matches!(
        text(b"Foo", Escape::Identifier),
        Cow::Borrowed("Foo")
    ));
    assert_eq!(text(b"a\\b", Escape::Identifier), "a\\\\b");
    assert_eq!(text(b"a\tb", Escape::Identifier), "a\\x09b");

    // A payload keeps every valid byte, so its spelling is owned only when the
    // bytes are not valid UTF-8.
    assert!(matches!(
        text(b"a\\b", Escape::Verbatim),
        Cow::Borrowed("a\\b")
    ));
    assert_eq!(text(b"a\xffb", Escape::Verbatim), "a\\xffb");
    assert_eq!(text(b"a\xffb", Escape::Identifier), "a\\xffb");

    // Reconstructed text is always owned.
    assert_eq!(owned(b"Foo", Escape::Identifier), "Foo");
    assert_eq!(owned(b"a\\b", Escape::Identifier), "a\\\\b");
    assert_eq!(owned(b"a\\b", Escape::Verbatim), "a\\b");
}

#[cfg(feature = "rust")]
#[test]
fn many0_stops_on_a_backtracking_failure() {
    use super::combinator::many0;

    let mut state = make_state(100);
    let mut cursor = Cursor::new(b"aab", 0, &mut state).unwrap();
    let spans = many0(tag(b"a")).parse(&mut cursor).unwrap();
    assert_eq!(spans, [Span::new(0, 1), Span::new(1, 2)]);
    assert_eq!(cursor.pos(), 2);

    // A committed failure inside a repetition is propagated, not treated as
    // the end of the list. An exhausted work budget is such a failure.
    let mut state = make_state(2);
    let mut cursor = Cursor::new(b"aaa", 0, &mut state).unwrap();
    let fail = many0(tag(b"a")).parse(&mut cursor).unwrap_err();
    assert_eq!(fail.mode, FailMode::Cut);
    assert_eq!(fail.kind, FailKind::Work);
}
