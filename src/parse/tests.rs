use std::string::String;

use super::*;

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
fn alt_backtracks_and_cut_commits() {
    let mut state = make_state(100);
    let mut cursor = Cursor::new(b"abc", 0, &mut state).unwrap();
    let parsed = alt(tag(b"x"), tag(b"ab")).parse(&mut cursor).unwrap();
    assert_eq!(parsed, Span::new(0, 2));

    let mut state = make_state(100);
    let mut cursor = Cursor::new(b"abc", 0, &mut state).unwrap();
    let result = alt(cut(tag(b"x")), tag(b"ab")).parse(&mut cursor);
    assert!(result.is_err());
}

#[test]
fn work_budget_is_enforced() {
    let mut state = make_state(1);
    let mut cursor = Cursor::new(b"abc", 0, &mut state).unwrap();
    assert!(take(3).parse(&mut cursor).is_err());
}
