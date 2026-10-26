use super::*;

#[test]
fn utf16_offsets_and_all_js_line_endings() {
    let text = "a😀中\r\nb\rc\u{2028}d\u{2029}e";
    let ix = LineIndex::new(text);
    assert_eq!(ix.position(5), (1, 3));
    assert_eq!(ix.position(3), (1, 1));
    for (value, line) in [('b', 2), ('c', 3), ('d', 4), ('e', 5)] {
        assert_eq!(ix.position(text.find(value).unwrap() as u32), (line, 0));
    }
    assert_eq!(ix.position(u32::MAX), (5, 1));
}

#[test]
fn minified_line_lookups_match_reference() {
    let text = "x😀中".repeat(20_000);
    let ix = LineIndex::new(&text);
    for offset in (0..text.len()).step_by(1003) {
        let mut boundary = offset;
        while !text.is_char_boundary(boundary) {
            boundary -= 1;
        }
        assert_eq!(
            ix.position(offset as u32),
            (1, text[..boundary].encode_utf16().count() as u32)
        );
    }
    assert_eq!(ix.snippet(Span::new(0, text.len() as u32), 4), "x😀中x…");
}
