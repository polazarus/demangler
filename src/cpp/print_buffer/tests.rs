use super::PrintBuffer;

#[test]
fn the_buffer_has_no_output_depth_or_visit_cap() {
    let mut buffer = PrintBuffer::new();
    let filler = "a".repeat(2 * 1024 * 1024);
    buffer.write(&filler).unwrap();
    assert_eq!(buffer.as_str().len(), filler.len());
    for _ in 0..10 {
        buffer.enter().unwrap();
    }
    for _ in 0..10 {
        buffer.leave();
    }
    buffer.write_char('c').unwrap();
    assert_eq!(buffer.last_char(), Some('c'));
}
