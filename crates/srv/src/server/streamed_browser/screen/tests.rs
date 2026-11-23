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
    let i = (y * usize::from(screen.size().0) + x) * 4;
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
/// `cargo test -p agentmux-srv screen_mode_real -- --ignored --nocapture` on a Linux host (a container or a dedicated test machine) with Xvfb and a
/// Chrome, and the browser's policy file installed for the proxy port in `AGENTMUX_STREAMED_BROWSER_PROXY_PORT` (the file's content is in the
/// error the first run prints). The browser's own sandbox is on: a container that cannot start it cannot run these.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs Xvfb and a Chrome; run by hand on a Linux host"]
async fn screen_mode_real() {
    if which::which("Xvfb").is_err() || chromium_missing() {
        eprintln!("screen_mode_real: no Xvfb or no Chrome here; nothing run");
        return;
    }
    let session = session::start(800, 600, PAGE, true).await.expect("the screen and the browser start");
    let screen = x::Screen::connect(&session.display).expect("the screen is reachable");
    assert_eq!(screen.size(), (800, 600));
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
    let session = session::start(800, 600, POPUP_PAGE, true).await.expect("the screen and the browser start");
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

/// A page whose text box holds a text and has it selected, with a non-US letter in it.
const COPY_PAGE: &str = "data:text/html,<html style=\"height:100%25\"><body style=\"height:100%25;margin:0\"><textarea autofocus onfocus=\"this.select()\" style=\"width:100%25;height:100%25;margin:0;background:blue\">copy me, W%C3%B6rld</textarea></body></html>";

/// What the person copies in the remote browser (Ctrl+C) can be read out of the screen's clipboard, exactly, and nothing is read before that.
/// Run with the same command as `screen_mode_real`, by name `screen_mode_copy_real`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs Xvfb and a Chrome; run by hand on a Linux host"]
async fn screen_mode_copy_real() {
    if which::which("Xvfb").is_err() || chromium_missing() {
        eprintln!("screen_mode_copy_real: no Xvfb or no Chrome here; nothing run");
        return;
    }
    let session = session::start(800, 600, COPY_PAGE, true).await.expect("the screen and the browser start");
    let screen = x::Screen::connect(&session.display).expect("the screen is reachable");
    wait_for_colour(&screen, 700, 500, [255, 0, 0], 30).await.expect("the page is up (blue)");
    let clip = std::sync::Arc::new(clipboard::Clipboard::connect(&session.display).expect("the clipboard connects"));
    let read = |clip: std::sync::Arc<clipboard::Clipboard>| async move { tokio::task::spawn_blocking(move || clip.fetch()).await.unwrap() };
    // Nothing has been copied yet: nothing to bring over.
    assert_eq!(read(clip.clone()).await.expect("a read of an empty clipboard answers"), None);
    let key = |kind, code: &str, vk, mods| ClientMsg::Key { kind, key: String::new(), code: code.to_string(), text: String::new(), vk, mods };
    send_input(&screen, &click_at(MouseKind::Down)).await.unwrap();
    send_input(&screen, &click_at(MouseKind::Up)).await.unwrap();
    send_input(&screen, &key(KeyKind::Down, "ControlLeft", 17, 2)).await.unwrap();
    // The click put the caret in the text and dropped the page's own selection: select all again, then copy.
    send_input(&screen, &key(KeyKind::Down, "KeyA", 65, 2)).await.unwrap();
    send_input(&screen, &key(KeyKind::Up, "KeyA", 65, 2)).await.unwrap();
    send_input(&screen, &key(KeyKind::Down, "KeyC", 67, 2)).await.unwrap();
    send_input(&screen, &key(KeyKind::Up, "KeyC", 67, 2)).await.unwrap();
    send_input(&screen, &key(KeyKind::Up, "ControlLeft", 17, 0)).await.unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;
    let text = read(clip).await.expect("the copy is read").expect("the browser put text on the clipboard");
    assert_eq!(text, "copy me, Wörld");
}

fn click_at(kind: MouseKind) -> ClientMsg {
    ClientMsg::Mouse { kind, x: 400.0, y: 400.0, button: Button::Left, buttons: 1, clicks: 1, mods: 0 }
}

/// A page that is blue everywhere, so a screen that is bigger than the browser's window shows where the window ends.
const BLUE_PAGE: &str = "data:text/html,<html style=\"height:100%25\"><body style=\"height:100%25;margin:0;background:blue\"></body></html>";

/// The screen follows the size it is asked for, bigger and smaller, and the browser's maximised window follows the screen: the corner is still
/// the page's. Run with the same command as `screen_mode_real`, by name `screen_mode_resize_real`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs Xvfb, openbox and a Chrome; run by hand on a Linux host"]
async fn screen_mode_resize_real() {
    if which::which("Xvfb").is_err() || which::which("openbox").is_err() || chromium_missing() {
        eprintln!("screen_mode_resize_real: no Xvfb, openbox or Chrome here; nothing run");
        return;
    }
    let session = session::start(800, 600, BLUE_PAGE, true).await.expect("the screen and the browser start");
    let screen = x::Screen::connect(&session.display).expect("the screen is reachable");
    assert_eq!(screen.size(), (800, 600), "the screen is the size the pane asked for");
    wait_for_colour(&screen, 790, 590, [255, 0, 0], 30).await.expect("the page is up (blue)");
    for (w, h) in [(1024u16, 700u16), (640, 480), (1200, 800), (800, 600)] {
        screen.resize(w, h).unwrap_or_else(|e| panic!("resize to {w}x{h}: {e}"));
        assert_eq!(screen.size(), (w, h));
        assert_eq!(screen.grab().expect("the screen is read").len(), usize::from(w) * usize::from(h) * 4);
        wait_for_colour(&screen, usize::from(w) - 10, usize::from(h) - 10, [255, 0, 0], 10).await.unwrap_or_else(|e| panic!("the window did not follow the screen to {w}x{h}: {e}"));
    }
    // A size that is too small or too large is cut, and the same size twice does nothing.
    screen.resize(10, 10).unwrap();
    assert_eq!(screen.size(), (randr::MIN_SIDE, randr::MIN_SIDE));
    screen.resize(randr::MIN_SIDE, randr::MIN_SIDE).unwrap();
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

/// A screen that is not changing makes (next to) no frames, and a change makes one within a moment, because the screen says where it changed.
/// Run with the same command as `screen_mode_real`, by name `screen_mode_idle_real`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs Xvfb and a Chrome; run by hand on a Linux host"]
async fn screen_mode_idle_real() {
    if which::which("Xvfb").is_err() || chromium_missing() {
        eprintln!("screen_mode_idle_real: no Xvfb or no Chrome here; nothing run");
        return;
    }
    let session = session::start(800, 600, PAGE, true).await.expect("the screen and the browser start");
    let screen = x::Screen::connect(&session.display).expect("the screen is reachable");
    wait_for_colour(&screen, 400, 400, [0, 0, 255], 30).await.expect("the page is up");
    let (tx, mut rx) = mpsc::channel::<Vec<u8>>(8);
    capture(&session.display, tx, pace::Activity::default()).expect("frames start");
    assert!(tokio::time::timeout(Duration::from_secs(5), rx.recv()).await.is_ok(), "the first picture of the screen is sent");
    // Let the browser settle, then watch a still page.
    tokio::time::sleep(Duration::from_secs(3)).await;
    while rx.try_recv().is_ok() {}
    tokio::time::sleep(Duration::from_secs(6)).await;
    let mut idle = 0;
    while rx.try_recv().is_ok() {
        idle += 1;
    }
    assert!(idle <= 3, "a still page made {idle} frames in 6 seconds");
    // A click turns the page green: a frame follows at once.
    let click = |kind| ClientMsg::Mouse { kind, x: 400.0, y: 400.0, button: Button::Left, buttons: 1, clicks: 1, mods: 0 };
    send_input(&screen, &click(MouseKind::Down)).await.unwrap();
    send_input(&screen, &click(MouseKind::Up)).await.unwrap();
    let frame = tokio::time::timeout(Duration::from_secs(1), rx.recv()).await.expect("a change makes a frame within a second").unwrap();
    assert_eq!(frame[0], frames::FRAME);
}

/// A page with a text box that turns green when it holds exactly the text the test pastes (with a newline and a hash sign), and red for any
/// other text. (No hash sign or question mark in the address itself: a data address ends at the first hash sign.)
const PASTE_PAGE: &str = "data:text/html,<html style=\"height:100%25\"><body style=\"height:100%25;margin:0;background:blue\">\
                          <textarea autofocus style=\"width:100%25;height:100%25;margin:0;background:blue\" \
                          oninput=\"this.style.background=this.value==['Hello, World! %2342','line two'].join(String.fromCharCode(10))&&'lime'||'red'\">\
                          </textarea></body></html>";

/// Text pasted by the person (the pane's paste message) lands in the page's text box exactly, newline included.
/// Run with the same command as `screen_mode_real`, by name `screen_mode_paste_real`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs Xvfb and a Chrome; run by hand on a Linux host"]
async fn screen_mode_paste_real() {
    if which::which("Xvfb").is_err() || chromium_missing() {
        eprintln!("screen_mode_paste_real: no Xvfb or no Chrome here; nothing run");
        return;
    }
    let session = session::start(800, 600, PASTE_PAGE, true).await.expect("the screen and the browser start");
    let screen = x::Screen::connect(&session.display).expect("the screen is reachable");
    wait_for_colour(&screen, 700, 500, [255, 0, 0], 30).await.expect("the page is up (blue)");
    // The text box has the focus (autofocus); a click makes sure.
    let click = |kind| ClientMsg::Mouse { kind, x: 400.0, y: 400.0, button: Button::Left, buttons: 1, clicks: 1, mods: 0 };
    send_input(&screen, &click(MouseKind::Down)).await.unwrap();
    send_input(&screen, &click(MouseKind::Up)).await.unwrap();
    let text = "Hello, World! #42
line two".to_string();
    send_input(&screen, &ClientMsg::Paste { text }).await.unwrap();
    wait_for_colour(&screen, 700, 500, [0, 255, 0], 10).await.expect("the pasted text arrived exactly in the page");
}

#[tokio::test]
async fn typing_that_is_cut_short_says_so_and_types_nothing_more() {
    use std::cell::Cell;
    let typed = std::cell::RefCell::new(Vec::new());
    let calls = Cell::new(0);
    // The screen changes before the fourth character is typed.
    let stop = || {
        calls.set(calls.get() + 1);
        calls.get() > 3
    };
    let out = type_chars("abcdef", stop, |key, shift| {
        typed.borrow_mut().push((key, shift));
        Ok(())
    })
    .await
    .unwrap();
    assert_eq!(out, Typed::Interrupted);
    assert_eq!(typed.borrow().len(), 3, "only what came before the change was typed");
    let all = type_chars("ab", || false, |_, _| Ok(())).await.unwrap();
    assert_eq!(all, Typed::Done);
}

#[tokio::test]
async fn a_key_that_cannot_be_pressed_is_an_error_and_a_character_with_no_us_key_is_skipped() {
    assert!(type_chars("a", || false, |_, _| Err("no".to_string())).await.is_err());
    let n = std::cell::Cell::new(0);
    type_chars("a\u{e9}b", || false, |_, _| {
        n.set(n.get() + 1);
        Ok(())
    })
    .await
    .unwrap();
    assert_eq!(n.get(), 2, "the accented letter has no US key and is skipped");
}
