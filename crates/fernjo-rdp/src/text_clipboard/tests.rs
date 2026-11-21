use super::*;

fn setup(local: Option<&str>) -> (TextClipboard, mpsc::UnboundedReceiver<ClipEvent>, LocalText) {
    let (tx, rx) = mpsc::unbounded_channel();
    let local: LocalText = Arc::new(Mutex::new(local.map(str::to_string)));
    (TextClipboard::new(tx, local.clone()), rx, local)
}

fn message(rx: &mut mpsc::UnboundedReceiver<ClipEvent>) -> ClipboardMessage {
    match rx.try_recv().expect("an event") {
        ClipEvent::Message(m) => m,
        other => panic!("expected a message, got {other:?}"),
    }
}

#[test]
fn the_initial_format_list_offers_text_only_when_there_is_text() {
    let (mut empty, mut rx, _) = setup(None);
    empty.on_request_format_list();
    match message(&mut rx) {
        ClipboardMessage::SendInitiateCopy(formats) => assert!(formats.is_empty()),
        other => panic!("{other:?}"),
    }
    let (mut with_text, mut rx, _) = setup(Some("hello"));
    with_text.on_request_format_list();
    match message(&mut rx) {
        ClipboardMessage::SendInitiateCopy(formats) => {
            assert_eq!(formats.len(), 1);
            assert_eq!(formats[0].id(), ClipboardFormatId::CF_UNICODETEXT);
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_remote_text_announcement_requests_a_paste_and_other_formats_do_not() {
    let (mut backend, mut rx, _) = setup(None);
    backend.on_remote_copy(&[
        ClipboardFormat::new(ClipboardFormatId::CF_BITMAP),
        ClipboardFormat::new(ClipboardFormatId::CF_UNICODETEXT),
    ]);
    match message(&mut rx) {
        ClipboardMessage::SendInitiatePaste(id) => {
            assert_eq!(id, ClipboardFormatId::CF_UNICODETEXT)
        }
        other => panic!("{other:?}"),
    }
    backend.on_remote_copy(&[ClipboardFormat::new(ClipboardFormatId::CF_BITMAP)]);
    assert!(rx.try_recv().is_err());
}

#[test]
fn local_text_is_served_with_crlf() {
    let (mut backend, mut rx, _) = setup(Some("a\nb"));
    backend.on_format_data_request(FormatDataRequest {
        format: ClipboardFormatId::CF_UNICODETEXT,
    });
    match message(&mut rx) {
        ClipboardMessage::SendFormatData(response) => {
            assert!(!response.is_error());
            assert_eq!(response.to_unicode_string().unwrap(), "a\r\nb");
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_request_without_text_or_for_another_format_is_an_error_response() {
    let (mut none, mut rx, _) = setup(None);
    none.on_format_data_request(FormatDataRequest {
        format: ClipboardFormatId::CF_UNICODETEXT,
    });
    match message(&mut rx) {
        ClipboardMessage::SendFormatData(response) => assert!(response.is_error()),
        other => panic!("{other:?}"),
    }
    let (mut text, mut rx, _) = setup(Some("x"));
    text.on_format_data_request(FormatDataRequest {
        format: ClipboardFormatId::CF_BITMAP,
    });
    match message(&mut rx) {
        ClipboardMessage::SendFormatData(response) => assert!(response.is_error()),
        other => panic!("{other:?}"),
    }
    // text over the cap is not served either
    let (mut big, mut rx, _) = setup(Some(&"a".repeat(crate::MAX_CLIPBOARD_BYTES + 1)));
    big.on_format_data_request(FormatDataRequest {
        format: ClipboardFormatId::CF_UNICODETEXT,
    });
    match message(&mut rx) {
        ClipboardMessage::SendFormatData(response) => assert!(response.is_error()),
        other => panic!("{other:?}"),
    }
}

#[test]
fn remote_text_becomes_an_event_with_lf_line_ends() {
    let (mut backend, mut rx, _) = setup(None);
    backend.on_format_data_response(OwnedFormatDataResponse::new_unicode_string("a\r\nb"));
    match rx.try_recv().unwrap() {
        ClipEvent::RemoteText(text) => assert_eq!(text, "a\nb"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn an_error_response_or_oversize_remote_text_produces_nothing() {
    let (mut backend, mut rx, _) = setup(None);
    backend.on_format_data_response(OwnedFormatDataResponse::new_error());
    backend.on_format_data_response(OwnedFormatDataResponse::new_unicode_string(
        &"a".repeat(crate::MAX_CLIPBOARD_BYTES + 1),
    ));
    assert!(rx.try_recv().is_err());
}
