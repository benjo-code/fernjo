use super::*;

fn rect(x: u16, y: u16, width: u16, height: u16) -> Rect {
    Rect {
        x,
        y,
        width,
        height,
    }
}

fn emitter(capacity: usize) -> (Emitter, mpsc::Receiver<SessionEvent>) {
    let (tx, rx) = mpsc::channel(capacity);
    (Emitter { tx, pending: None }, rx)
}

#[test]
fn damage_is_sent_at_once_when_the_queue_has_room() {
    let (mut emitter, mut rx) = emitter(2);
    emitter.damage(rect(1, 2, 3, 4));
    assert_eq!(emitter.pending, None);
    assert_eq!(
        rx.try_recv().unwrap(),
        SessionEvent::Damage(rect(1, 2, 3, 4))
    );
}

#[test]
fn damage_is_merged_while_the_queue_is_full_and_sent_when_there_is_room() {
    let (mut emitter, mut rx) = emitter(1);
    emitter.damage(rect(0, 0, 10, 10));
    // the queue (one slot) is full now: the next rectangles are merged, not queued
    emitter.damage(rect(20, 0, 5, 5));
    emitter.damage(rect(0, 30, 5, 5));
    assert_eq!(emitter.pending, Some(rect(0, 0, 25, 35)));
    assert_eq!(
        rx.try_recv().unwrap(),
        SessionEvent::Damage(rect(0, 0, 10, 10))
    );
    assert!(rx.try_recv().is_err(), "nothing else was queued");
    // with room again, the merged rectangle goes out together with the next one
    emitter.damage(rect(40, 40, 2, 2));
    assert_eq!(emitter.pending, None);
    assert_eq!(
        rx.try_recv().unwrap(),
        SessionEvent::Damage(rect(0, 0, 42, 42))
    );
}

#[test]
fn mouse_coordinates_are_clamped_to_the_desktop() {
    let mut keys = Database::new();
    let events = to_fast_path(
        &mut keys,
        vec![InputEvent::MouseMove { x: 5000, y: 6000 }],
        800,
        600,
    );
    let positions: Vec<(u16, u16)> = events
        .iter()
        .filter_map(|e| match e {
            FastPathInputEvent::MouseEvent(m) => Some((m.x_position, m.y_position)),
            _ => None,
        })
        .collect();
    assert_eq!(positions, vec![(799, 599)]);
}

#[test]
fn keys_buttons_and_wheel_become_fast_path_events() {
    let mut keys = Database::new();
    let events = to_fast_path(
        &mut keys,
        vec![
            InputEvent::Key {
                scancode: 0x1e,
                extended: false,
                pressed: true,
            },
            InputEvent::Key {
                scancode: 0x1e,
                extended: false,
                pressed: false,
            },
            InputEvent::MouseButton {
                button: MouseButton::Left,
                pressed: true,
            },
            InputEvent::Wheel {
                vertical: true,
                delta: -120,
            },
        ],
        800,
        600,
    );
    let keyboard = events
        .iter()
        .filter(|e| matches!(e, FastPathInputEvent::KeyboardEvent(_, 0x1e)))
        .count();
    let mouse = events
        .iter()
        .filter(|e| matches!(e, FastPathInputEvent::MouseEvent(_)))
        .count();
    assert_eq!(keyboard, 2, "{events:?}");
    assert_eq!(mouse, 2, "{events:?}");
}

#[test]
fn copy_region_clips_to_the_image_and_refuses_an_inverted_rectangle() {
    let image = DecodedImage::new(PixelFormat::RgbA32, 8, 6);
    let fb = Framebuffer::new(8, 6).unwrap();
    let inside = InclusiveRectangle {
        left: 1,
        top: 2,
        right: 3,
        bottom: 4,
    };
    assert_eq!(copy_region(&image, &inside, &fb), Some(rect(1, 2, 3, 3)));
    let sticking_out = InclusiveRectangle {
        left: 6,
        top: 4,
        right: 20,
        bottom: 20,
    };
    assert_eq!(
        copy_region(&image, &sticking_out, &fb),
        Some(rect(6, 4, 2, 2))
    );
    let inverted = InclusiveRectangle {
        left: 5,
        top: 5,
        right: 2,
        bottom: 2,
    };
    assert_eq!(copy_region(&image, &inverted, &fb), None);
    let outside = InclusiveRectangle {
        left: 30,
        top: 30,
        right: 31,
        bottom: 31,
    };
    assert_eq!(copy_region(&image, &outside, &fb), None);
}
