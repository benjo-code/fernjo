use super::*;

#[test]
fn lf_becomes_crlf_on_the_way_out_and_back() {
    assert_eq!(to_remote("a\nb\n").as_deref(), Some("a\r\nb\r\n"));
    assert_eq!(from_remote("a\r\nb\r\n").as_deref(), Some("a\nb\n"));
    // text that already has CRLF is not doubled
    assert_eq!(to_remote("a\r\nb").as_deref(), Some("a\r\nb"));
    assert_eq!(to_remote("").as_deref(), Some(""));
    assert_eq!(from_remote("single line").as_deref(), Some("single line"));
}

#[test]
fn text_over_one_mebibyte_is_dropped_not_truncated() {
    let exact = "a".repeat(MAX_CLIPBOARD_BYTES);
    assert_eq!(to_remote(&exact).as_deref(), Some(exact.as_str()));
    assert_eq!(from_remote(&exact).as_deref(), Some(exact.as_str()));
    let over = "a".repeat(MAX_CLIPBOARD_BYTES + 1);
    assert_eq!(to_remote(&over), None);
    assert_eq!(from_remote(&over), None);
}

#[test]
fn a_trailing_nul_from_the_remote_is_removed() {
    assert_eq!(from_remote("text\0").as_deref(), Some("text"));
    assert_eq!(from_remote("text\0\0").as_deref(), Some("text"));
    assert_eq!(from_remote("\0").as_deref(), Some(""));
    // a NUL inside the text is not ours to cut
    assert_eq!(from_remote("a\0b").as_deref(), Some("a\0b"));
}
