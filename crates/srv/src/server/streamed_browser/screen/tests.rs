// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use super::*;

#[test]
fn a_scroll_is_a_whole_number_of_wheel_notches_between_one_and_ten() {
    assert_eq!(notches(0.0), 0);
    assert_eq!(notches(0.4), 0);
    assert_eq!(notches(1.0), 1);
    assert_eq!(notches(120.0), 1);
    assert_eq!(notches(-240.0), 2);
    assert_eq!(notches(100_000.0), 10);
}

#[test]
fn a_mouse_button_is_the_x_button_number_and_no_button_is_none() {
    assert_eq!((button_number(Button::Left), button_number(Button::Middle), button_number(Button::Right)), (Some(1), Some(2), Some(3)));
    assert_eq!(button_number(Button::None), None);
}

#[test]
fn screen_mode_is_on_only_when_asked_with_a_one() {
    // The process's environment is shared: this only reads it, and the variable is not set by any test.
    assert_eq!(enabled(), std::env::var(ENABLE_ENV).is_ok_and(|v| v == "1"));
}

/// A page that is red when the browser says it is not remote-controlled (black when it says it is) and turns green when it is clicked.
const PAGE: &str = "data:text/html,<html style=\"height:100%25\"><body style=\"height:100%25;margin:0;background:red\" \
                    onload=\"if(navigator.webdriver)document.body.style.background='black'\" \
                    onclick=\"document.body.style.background='lime'\"></body></html>";

/// The colour (blue, green, red) of the pixel at (x, y) of a grabbed screen.
fn pixel(screen: &x::Screen, bgrx: &[u8], x: usize, y: usize) -> [u8; 3] {
    let i = (y * usize::from(screen.w) + x) * 4;
    [bgrx[i], bgrx[i + 1], bgrx[i + 2]]
}

/// Wait until the pixel at (x, y) is `want` (within a margin), up to `secs`.
async fn wait_for_colour(screen: &x::Screen, x: usize, y: usize, want: [u8; 3], secs: u64) -> Result<(), String> {
    let near = |a: [u8; 3]| a.iter().zip(want).all(|(a, w)| a.abs_diff(w) < 40);
    let until = Instant::now() + Duration::from_secs(secs);
    loop {
        let now = pixel(screen, &screen.grab()?, x, y);
        if near(now) {
            return Ok(());
        }
        if Instant::now() > until {
            return Err(format!("the pixel at ({x}, {y}) is {now:?}, not {want:?}"));
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

/// A page that opens a small pop-up window when it is clicked.
const POPUP_PAGE: &str = "data:text/html,<html style=\"height:100%25\"><body style=\"height:100%25;margin:0;background:red\" \
                          onclick=\"window.open('about:blank','','popup,width=300,height=300')\"></body></html>";

/// The real thing, with an Xvfb and a Chrome on this machine: the browser starts on a virtual screen, the page paints (red: the
/// browser does not report itself as remote-controlled), a click sent at screen level reaches the page (green), a frame of the screen
/// is a JPEG of the right size, and a typed address changes the page.
/// `cargo test -p agentmux-srv screen_mode_real -- --ignored --nocapture` on a Linux host with Xvfb and a Chrome (set
/// `AGENTMUX_STREAMED_BROWSER_NO_SANDBOX=1` if Chrome refuses to start without it).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs Xvfb and a Chrome; run by hand on a Linux host"]
async fn screen_mode_real() {
    if which::which("Xvfb").is_err() || chromium_missing() {
        eprintln!("screen_mode_real: no Xvfb or no Chrome here; nothing run");
        return;
    }
    let session = session::start(800, 600, PAGE, true).await.expect("the screen and the browser start");
    let screen = x::Screen::connect(&session.display).expect("the screen is reachable");
    assert_eq!((screen.w, screen.h), (800, 600));
    // Under the browser's own tab strip and address bar: the page.
    let (px, py) = (400, 400);
    wait_for_colour(&screen, px, py, [0, 0, 255], 30).await.expect("the page is red: the browser does not say it is remote-controlled");
    let frame = frames::frame_message(&screen.grab().unwrap(), 800, 600).unwrap();
    let img = image::load_from_memory_with_format(&frame[1..], image::ImageFormat::Jpeg).unwrap();
    assert_eq!((img.width(), img.height()), (800, 600));
    let click = |kind| ClientMsg::Mouse { kind, x: px as f64, y: py as f64, button: Button::Left, buttons: 1, clicks: 1, mods: 0 };
    send_input(&screen, &click(MouseKind::Down)).await.unwrap();
    send_input(&screen, &click(MouseKind::Up)).await.unwrap();
    wait_for_colour(&screen, px, py, [0, 255, 0], 10).await.expect("the click reached the page");
    // An address typed in goes to the browser: a page of another colour.
    let other = "data:text/html,<body style=\"background:blue\"></body>";
    navigate(&screen, Go::Url, other).await.unwrap();
    wait_for_colour(&screen, px, py, [255, 0, 0], 10).await.expect("the typed address loaded");
    // A local file typed into the address bar does not open: the managed policy blocks it (the page would be green).
    let file = std::env::temp_dir().join("agentmux-screen-mode-blocked.html");
    std::fs::write(&file, "<body style=\"background:lime\"></body>").unwrap();
    navigate(&screen, Go::Url, &format!("file://{}", file.display())).await.unwrap();
    tokio::time::sleep(Duration::from_secs(5)).await;
    let now = pixel(&screen, &screen.grab().unwrap(), px, py);
    let _ = std::fs::remove_file(&file);
    assert!(now.iter().zip([0u8, 255, 0]).any(|(a, w)| a.abs_diff(w) >= 40), "the local file opened: {now:?}");
}

/// A pop-up the page opens is counted as a second window on the screen (the notice and the key hold follow from it).
/// Run with the same command as `screen_mode_real`, by name `screen_mode_popup_real`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs Xvfb and a Chrome; run by hand on a Linux host"]
async fn screen_mode_popup_real() {
    if which::which("Xvfb").is_err() || chromium_missing() {
        eprintln!("screen_mode_popup_real: no Xvfb or no Chrome here; nothing run");
        return;
    }
    let session = session::start(800, 600, POPUP_PAGE).await.expect("the screen and the browser start");
    let screen = x::Screen::connect(&session.display).expect("the screen is reachable");
    wait_for_colour(&screen, 400, 400, [0, 0, 255], 30).await.expect("the page is up");
    assert_eq!(screen.visible_windows().unwrap(), 1);
    let click = |kind| ClientMsg::Mouse { kind, x: 400.0, y: 400.0, button: Button::Left, buttons: 1, clicks: 1, mods: 0 };
    send_input(&screen, &click(MouseKind::Down)).await.unwrap();
    send_input(&screen, &click(MouseKind::Up)).await.unwrap();
    let until = Instant::now() + Duration::from_secs(15);
    while screen.visible_windows().unwrap() < 2 {
        assert!(Instant::now() < until, "the pop-up was not counted as a window");
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

fn chromium_missing() -> bool {
    super::super::chromium::find_binary().is_err()
}

/// A client holding Alt and dragging cannot move the browser off the screen, and nothing else is reachable: the window manager binds nothing.
/// Run with the same command as `screen_mode_real`, by name `screen_mode_window_stays_put_real`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs Xvfb, openbox and a Chrome; run by hand on a Linux host"]
async fn screen_mode_window_stays_put_real() {
    if which::which("Xvfb").is_err() || which::which("openbox").is_err() || chromium_missing() {
        eprintln!("screen_mode_window_stays_put_real: no Xvfb, openbox or Chrome here; nothing run");
        return;
    }
    let session = session::start(800, 600, PAGE, true).await.expect("the screen and the browser start");
    let screen = x::Screen::connect(&session.display).expect("the screen is reachable");
    wait_for_colour(&screen, 400, 400, [0, 0, 255], 30).await.expect("the page is up");
    let alt = keymap::keycode("AltLeft").unwrap();
    screen.key(alt, true).unwrap();
    screen.move_to(300.0, 300.0).unwrap();
    screen.button(1, true).unwrap();
    screen.move_to(500.0, 450.0).unwrap();
    screen.button(1, false).unwrap();
    screen.key(alt, false).unwrap();
    tokio::time::sleep(Duration::from_secs(2)).await;
    // The page still fills the screen: its bottom right corner is the page (red, or green if the click reached it), not the bare screen.
    let corner = pixel(&screen, &screen.grab().unwrap(), 790, 590);
    let page = |want: [u8; 3]| corner.iter().zip(want).all(|(a, w)| a.abs_diff(w) < 40);
    assert!(page([0, 0, 255]) || page([0, 255, 0]), "the window moved: the corner is {corner:?}");
}
