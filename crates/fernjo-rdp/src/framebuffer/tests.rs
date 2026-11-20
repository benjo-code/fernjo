use super::*;

fn rect(x: u16, y: u16, width: u16, height: u16) -> Rect {
    Rect {
        x,
        y,
        width,
        height,
    }
}

#[test]
fn refuses_zero_and_oversize_desktops() {
    // Review Focus 1: a server announcing a zero or enormous size must allocate nothing.
    for (w, h) in [
        (0, 600),
        (800, 0),
        (8193, 600),
        (800, 8193),
        (65535, 1),
        (0, 0),
    ] {
        let err = Framebuffer::new(w, h).unwrap_err();
        assert_eq!(err.code(), "desktop_too_large", "{w}x{h}");
    }
    assert!(Framebuffer::new(8192, 1).is_ok());
    assert!(Framebuffer::new(1, 8192).is_ok());
}

#[test]
fn write_then_copy_round_trips_a_region() {
    let fb = Framebuffer::new(10, 6).unwrap();
    let pixels: Vec<u8> = (0..(3 * 2 * 4)).map(|i| i as u8 + 1).collect();
    fb.write_region(rect(2, 1, 3, 2), &pixels);
    assert_eq!(fb.copy_region(rect(2, 1, 3, 2)), pixels);
    // the rest is still black and transparent
    assert_eq!(fb.copy_region(rect(0, 0, 1, 1)), vec![0, 0, 0, 0]);
    // a larger region contains the written rows at the right places
    let all = fb.copy_region(rect(0, 0, 10, 6));
    assert_eq!(all.len(), 10 * 6 * 4);
    let row1 = &all[(10 + 2) * 4..(10 + 5) * 4];
    assert_eq!(row1, &pixels[..12]);
}

#[test]
fn copy_region_clips_to_the_buffer() {
    let fb = Framebuffer::new(4, 4).unwrap();
    fb.write_region(rect(0, 0, 4, 4), &[7u8; 4 * 4 * 4]);
    // 2x2 inside, the rest outside to the right and below
    let out = fb.copy_region(rect(3, 3, 5, 5));
    assert_eq!(out, vec![7u8; 4]);
    assert!(fb.copy_region(rect(4, 0, 2, 2)).is_empty());
    assert!(fb.copy_region(rect(0, 9, 2, 2)).is_empty());
}

#[test]
fn a_short_pixel_slice_or_an_outside_rect_never_panics() {
    let fb = Framebuffer::new(4, 4).unwrap();
    fb.write_region(rect(0, 0, 4, 4), &[1, 2, 3]);
    fb.write_region(rect(20, 20, 4, 4), &[9u8; 64]);
    fb.write_region(rect(2, 2, 10, 10), &vec![9u8; 10 * 10 * 4]);
    // the part inside is written from the unclipped source rows
    assert_eq!(fb.copy_region(rect(3, 3, 1, 1)), vec![9, 9, 9, 9]);
}

#[test]
fn resize_keeps_a_valid_size_and_refuses_a_bad_one() {
    let fb = Framebuffer::new(100, 50).unwrap();
    let shared = fb.clone();
    fb.resize(200, 120).unwrap();
    assert_eq!(shared.size(), (200, 120));
    assert_eq!(
        shared.copy_region(rect(0, 0, 200, 120)).len(),
        200 * 120 * 4
    );
    for (w, h) in [(0, 10), (10, 0), (9000, 10)] {
        assert_eq!(fb.resize(w, h).unwrap_err().code(), "desktop_too_large");
        assert_eq!(fb.size(), (200, 120));
    }
}

#[test]
fn rect_union_is_the_bounding_box() {
    let u = rect(10, 10, 5, 5).union(rect(20, 4, 3, 3));
    assert_eq!(u, rect(10, 4, 13, 11));
    assert_eq!(rect(0, 0, 4, 4).union(rect(1, 1, 1, 1)), rect(0, 0, 4, 4));
    // never wraps around, even for rectangles at the edge of the coordinate range
    let wide = rect(65000, 0, 535, 1).union(rect(0, 0, 1, 1));
    assert_eq!(wide.x, 0);
    assert_eq!(wide.width, u16::MAX);
}

#[test]
fn clamp_to_cuts_at_the_desktop_and_drops_empty_results() {
    assert_eq!(rect(2, 2, 100, 100).clamp_to(10, 8), Some(rect(2, 2, 8, 6)));
    assert_eq!(rect(0, 0, 4, 4).clamp_to(10, 10), Some(rect(0, 0, 4, 4)));
    assert_eq!(rect(10, 0, 4, 4).clamp_to(10, 10), None);
    assert_eq!(rect(0, 0, 0, 4).clamp_to(10, 10), None);
}
