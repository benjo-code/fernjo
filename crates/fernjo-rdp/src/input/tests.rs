use super::*;

#[test]
fn events_parse_from_json() {
    let events: Vec<InputEvent> = serde_json::from_str(
        r#"[
            {"kind":"key","scancode":30,"extended":false,"pressed":true},
            {"kind":"key","scancode":72,"extended":true,"pressed":false},
            {"kind":"mouseMove","x":640,"y":480},
            {"kind":"mouseButton","button":"left","pressed":true},
            {"kind":"mouseButton","button":"x2","pressed":false},
            {"kind":"wheel","vertical":true,"delta":-120},
            {"kind":"wheel","vertical":false,"delta":120}
        ]"#,
    )
    .unwrap();
    assert_eq!(
        events,
        vec![
            InputEvent::Key {
                scancode: 30,
                extended: false,
                pressed: true
            },
            InputEvent::Key {
                scancode: 72,
                extended: true,
                pressed: false
            },
            InputEvent::MouseMove { x: 640, y: 480 },
            InputEvent::MouseButton {
                button: MouseButton::Left,
                pressed: true
            },
            InputEvent::MouseButton {
                button: MouseButton::X2,
                pressed: false
            },
            InputEvent::Wheel {
                vertical: true,
                delta: -120
            },
            InputEvent::Wheel {
                vertical: false,
                delta: 120
            },
        ]
    );
}

#[test]
fn an_unknown_kind_is_an_error() {
    assert!(serde_json::from_str::<InputEvent>(r#"{"kind":"teleport","x":1}"#).is_err());
    assert!(serde_json::from_str::<InputEvent>(r#"{"scancode":1}"#).is_err());
    // a value that does not fit the wire type is refused instead of wrapping
    assert!(
        serde_json::from_str::<InputEvent>(
            r#"{"kind":"key","scancode":300,"extended":false,"pressed":true}"#
        )
        .is_err()
    );
    assert!(serde_json::from_str::<InputEvent>(r#"{"kind":"mouseMove","x":-1,"y":0}"#).is_err());
    assert!(
        serde_json::from_str::<InputEvent>(
            r#"{"kind":"mouseButton","button":"back","pressed":true}"#
        )
        .is_err()
    );
}

#[test]
fn the_batch_limit_is_256() {
    assert_eq!(MAX_INPUT_EVENTS, 256);
}
