use std::string::String;

use super::combinator::{alt, cut, decimal_length, eof, ident, peek, tag, take};
use super::cursor::{Cursor, Limits, State};
use super::error::FailMode;
use super::escape::{Escape, Ident, write_escaped};
use super::parser::{PResult, Parser};
use super::span::Span;

fn make_state(work: u32) -> State {
    State::new(Limits {
        input: 1024,
        depth: 8,
        work,
        nodes: 8,
    })
}

fn toy<'a>(input: &mut Cursor<'a, '_>) -> PResult<Ident<'a>> {
    let _ = tag(b"_ZN").parse(input)?;
    let ident = ident().parse(input)?;
    let _ = tag(b"E").parse(input)?;
    eof().parse(input)?;
    Ok(ident)
}

#[test]
fn length_prefixes_and_invalid_bytes_are_logical() {
    let mut state = make_state(100);
    let bytes = b"_ZN3f\xffoE";
    let mut cursor = Cursor::new(bytes, 0, &mut state).unwrap();
    let ident = toy(&mut cursor).unwrap();
    assert_eq!(ident.bytes(), b"f\xffo");
    assert_eq!(ident.span(), Span::new(4, 7));

    let mut rendered = String::new();
    write_escaped(&mut rendered, ident.bytes(), Escape::Identifier).unwrap();
    assert_eq!(rendered, "f\\xffo");
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
fn take_and_eof_bound_the_input() {
    let mut state = make_state(100);
    let mut cursor = Cursor::new(b"ab", 0, &mut state).unwrap();
    assert_eq!(take(1).parse(&mut cursor).unwrap(), b"a");
    assert!(eof().parse(&mut cursor).is_err());
    assert_eq!(take(1).parse(&mut cursor).unwrap(), b"b");
    assert_eq!(eof().parse(&mut cursor).unwrap(), ());
    assert!(
        take(1).parse(&mut cursor).is_err(),
        "take past EOF backtracks"
    );
}

#[test]
fn ident_reads_a_decimal_length_prefix() {
    let mut state = make_state(100);
    let mut cursor = Cursor::new(b"3abc", 0, &mut state).unwrap();
    let parsed = ident().parse(&mut cursor).unwrap();
    assert_eq!(parsed.bytes(), b"abc");
    assert_eq!(parsed.span(), Span::new(1, 4));
    assert_eq!(cursor.pos(), 4);

    // A missing length is not an identifier, and a length beyond the input
    // backtracks without consuming the digits.
    let mut state = make_state(100);
    let mut cursor = Cursor::new(b"abc", 0, &mut state).unwrap();
    assert!(ident().parse(&mut cursor).is_err());

    let mut state = make_state(100);
    let mut cursor = Cursor::new(b"9ab", 0, &mut state).unwrap();
    assert!(ident().parse(&mut cursor).is_err());
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
    assert_eq!(fail.expected, b"decimal overflow");
}

#[test]
fn alt_backtracks_and_cut_commits() {
    let mut state = make_state(100);
    let mut cursor = Cursor::new(b"abc", 0, &mut state).unwrap();
    let parsed = alt(tag(b"x"), tag(b"ab")).parse(&mut cursor).unwrap();
    assert_eq!(parsed, Span::new(0, 2));
    assert_eq!(cursor.pos(), 2);

    // A committed failure is not retried by the second alternative.
    let mut state = make_state(100);
    let mut cursor = Cursor::new(b"abc", 0, &mut state).unwrap();
    let result = alt(cut(tag(b"x")), tag(b"ab")).parse(&mut cursor);
    assert!(result.is_err());
    assert_eq!(cursor.pos(), 0);
}

#[test]
fn peek_reports_a_match_without_consuming_input() {
    let mut state = make_state(100);
    let mut cursor = Cursor::new(b"abc", 0, &mut state).unwrap();
    assert_eq!(
        peek(tag(b"ab")).parse(&mut cursor).unwrap(),
        Span::new(0, 2)
    );
    assert_eq!(cursor.pos(), 0);
    assert_eq!(take(1).parse(&mut cursor).unwrap(), b"a");
    assert!(peek(tag(b"z")).parse(&mut cursor).is_err());
    assert_eq!(cursor.pos(), 1, "a failed lookahead restores the position");
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
fn input_depth_and_node_budgets_are_enforced() {
    let mut state = make_state(100);
    let fail = Cursor::new(&[0_u8; 1025], 0, &mut state).unwrap_err();
    assert_eq!(fail.mode, FailMode::Cut);
    assert_eq!(fail.expected, b"input budget");

    let mut state = make_state(100);
    let mut cursor = Cursor::new(b"", 0, &mut state).unwrap();
    for _ in 0..8 {
        cursor.enter().unwrap();
    }
    assert_eq!(cursor.enter().unwrap_err().expected, b"depth budget");

    let mut state = make_state(100);
    for _ in 0..8 {
        state.node().unwrap();
    }
    assert_eq!(state.node().unwrap_err().expected, b"node budget");
}

#[test]
fn work_budget_is_enforced() {
    let mut state = make_state(1);
    let mut cursor = Cursor::new(b"abc", 0, &mut state).unwrap();
    assert!(take(3).parse(&mut cursor).is_err());
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
    // the end of the list.
    let mut state = make_state(100);
    let mut cursor = Cursor::new(b"aab", 0, &mut state).unwrap();
    assert!(many0(cut(tag(b"a"))).parse(&mut cursor).is_err());
}
