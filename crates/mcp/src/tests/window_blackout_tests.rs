// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use super::*;

fn mask(window: &str, title: &str) -> WindowMask {
    WindowMask {
        window_id: window.into(),
        title: title.into(),
        width: 100.0,
        height: 50.0,
        scale: 2.0,
        // The Vault pane, and a Vault request card inside an agent pane.
        rects: vec![MaskRect { x: 10.0, y: 5.0, w: 20.0, h: 10.0 }, MaskRect { x: 60.0, y: 30.0, w: 5.0, h: 5.0 }],
        ..Default::default()
    }
}

fn frame() -> image::RgbaImage {
    image::RgbaImage::from_pixel(200, 100, image::Rgba([200, 200, 200, 255]))
}

#[test]
fn each_private_rectangle_is_blacked_out_and_the_rest_is_kept() {
    let m = mask("w1", "AgentMux");
    let mut img = frame();
    black_out(&mut img, &m);
    let black = image::Rgba([0, 0, 0, 255]);
    // Inside the first rectangle (device pixels 20..60 × 10..30), at its edges and a pixel past them.
    for (x, y) in [(20, 10), (59, 29), (40, 20), (19, 9), (60, 30)] {
        assert_eq!(*img.get_pixel(x, y), black, "({x},{y})");
    }
    // Inside the second (120..130 × 60..70).
    assert_eq!(*img.get_pixel(125, 65), black);
    // Outside both: an agent pane, a terminal.
    for (x, y) in [(0, 0), (100, 50), (199, 99), (17, 20), (40, 33)] {
        assert_eq!(*img.get_pixel(x, y), image::Rgba([200, 200, 200, 255]), "({x},{y})");
    }
}

#[test]
fn a_rectangle_past_the_frame_is_clipped_not_a_panic() {
    let mut m = mask("w1", "AgentMux");
    m.rects = vec![MaskRect { x: -10.0, y: 40.0, w: 500.0, h: 500.0 }];
    let mut img = frame();
    black_out(&mut img, &m);
    assert_eq!(*img.get_pixel(0, 99), image::Rgba([0, 0, 0, 255]));
    assert_eq!(*img.get_pixel(0, 0), image::Rgba([200, 200, 200, 255]));
}

#[test]
fn a_window_that_gave_no_rectangles_is_refused() {
    let err = before(&[mask("w1", "Other")], "AgentMux").unwrap_err().to_string();
    assert!(err.contains("did not give the rectangles"), "{err}");
    assert!(err.contains("UI screenshot of your own pane"), "{err}");
}

#[test]
fn two_windows_of_one_title_are_refused() {
    let err = before(&[mask("w1", "AgentMux"), mask("w2", "AgentMux")], "AgentMux").unwrap_err().to_string();
    assert!(err.contains("can't be told apart"), "{err}");
}

#[test]
fn a_moving_layout_is_refused_before_the_frame() {
    let mut m = mask("w1", "AgentMux");
    m.moving = true;
    assert!(before(&[m], "AgentMux").unwrap_err().to_string().contains("moving"));
}

#[test]
fn an_overlay_the_window_cannot_measure_refuses_the_capture() {
    let mut m = mask("w1", "AgentMux");
    m.refuse = Some("a pane is being dragged".into());
    let err = before(&[m], "AgentMux").unwrap_err().to_string();
    assert!(err.contains("a pane is being dragged"), "{err}");
}

#[test]
fn a_layout_that_changed_between_the_rectangles_and_the_frame_is_refused() {
    let b = mask("w1", "AgentMux");
    let mut changed = b.clone();
    changed.changed = true;
    assert!(after(&b, &[changed], 200, 100).unwrap_err().to_string().contains("changed"));
    let mut moved = b.clone();
    moved.rects[0].x += 1.0;
    assert!(after(&b, &[moved], 200, 100).is_err(), "other rectangles at the end are a change too");
    assert!(after(&b, &[], 200, 100).unwrap_err().to_string().contains("did not answer"));
}

#[test]
fn a_frame_of_another_size_or_scale_is_refused() {
    let b = mask("w1", "AgentMux");
    assert!(after(&b, &[b.clone()], 200, 100).is_ok());
    assert!(after(&b, &[b.clone()], 201, 101).is_ok(), "a pixel of rounding is the same size");
    let err = after(&b, &[b.clone()], 150, 75).unwrap_err().to_string();
    assert!(err.contains("not the size or scale"), "{err}");
    let mut rescaled = b.clone();
    rescaled.scale = 1.0;
    assert!(after(&b, &[rescaled], 200, 100).is_err());
}

#[test]
fn the_container_refusal_says_what_to_do_instead() {
    assert!(IN_CONTAINER.contains("UI screenshot of your own pane"));
    assert!(OTHER_INSTANCE.contains("capture it from an agent of that AgentMux"));
}

#[test]
fn the_title_linux_adds_without_the_sandbox_still_finds_the_window() {
    let m = mask("w1", "AgentMux · work");
    assert!(before(std::slice::from_ref(&m), &format!("AgentMux · work{UNSANDBOXED_SUFFIX}")).is_ok());
    assert!(before(std::slice::from_ref(&m), "AgentMux · work — something else").is_err());
    let host = include_str!("../../../cef/src/client/display.rs");
    assert!(host.contains(&format!("push_str(\"{UNSANDBOXED_SUFFIX}\")")), "the host's suffix changed: change UNSANDBOXED_SUFFIX with it");
}
