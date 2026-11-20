// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The "Updating to <version>" window of a `.dmg` upgrade's hand-off (`updating_window`): the splash's card, with
//! the version coming in its footer, from the moment the old launcher is gone until the new launcher's splash is up.
//! Its own file so `splash_mac.rs` stays under the file-size limit; it uses that file's AppKit helpers.

use super::*;

/// Whether process `pid` is running (`kill(pid, 0)`: no signal is sent). A process this user can't signal exists.
fn process_is_running(pid: u32) -> bool {
    if pid == 0 || pid > i32::MAX as u32 {
        return false; // 0 and negatives address a group, never one process
    }
    // SAFETY: signal 0 only checks that the process exists and can be signalled.
    unsafe { libc::kill(pid as libc::pid_t, 0) == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM) }
}

/// Show `lines` in the footer labels and `headline` in the first stage row (what the splash says there).
unsafe fn show_phase(footer_labels: &[id], stage: (id, id), lines: &[String], headline: &str) {
    let pool = send(class(b"NSAutoreleasePool\0"), sel(b"alloc\0"));
    let pool = send(pool, sel(b"init\0"));
    for (label, line) in footer_labels.iter().zip(lines) {
        send_void_id(*label, sel(b"setStringValue:\0"), nsstring(line));
    }
    let (name_field, time_field) = stage;
    let width = SPLASH_W - STAGE_MARGIN_R - STAGE_MARGIN_L;
    send_void_rect(
        name_field,
        sel(b"setFrame:\0"),
        CGRect {
            origin: CGPoint { x: STAGE_MARGIN_L, y: FOOTER_BAND_H + STAGE_AREA_H - STAGE_PAD_TOP - STAGE_ROW_H },
            size: CGSize { width, height: STAGE_ROW_H },
        },
    );
    send_void_id(name_field, sel(b"setStringValue:\0"), nsstring(headline));
    send_void_id(name_field, sel(b"setTextColor:\0"), color(STAGE_R, STAGE_G, STAGE_B));
    send_void_bool(name_field, sel(b"setHidden:\0"), 0);
    send_void_bool(time_field, sel(b"setHidden:\0"), 1);
    send_void(pool, sel(b"drain\0"));
}

/// The brain's alpha `t` seconds in: the splash's own fade-in and pulse.
fn brain_alpha(t: f64) -> f64 {
    if t < 0.2 {
        t / 0.2
    } else {
        0.73 + ((t - 0.2) * std::f64::consts::TAU * 1.1).sin().mul_add(0.5, 0.5) * 0.27
    }
}

/// The footer's lines for a phase, clamped to the card.
fn updating_lines(version: &str, p: crate::updating_window::Phase) -> Vec<String> {
    let max_chars = ((SPLASH_W - 24.0) / 7.5) as usize;
    crate::updating_window::footer_mac(version, p).iter().map(|l| crate::splash_info::ellipsize(l, max_chars.max(8))).collect()
}

/// The window of an in-app upgrade's hand-off, run by a clone of the old app's launcher (`--updating-window`). It
/// ends by what `updating_window::phase` decides, so it can't outlive a failed install; it takes no focus (the app
/// is never activated, the window never becomes key) and shows nothing but the card. `script_pid` is the swap
/// script; `dir` holds the markers it and the new launcher leave.
pub fn run_updating_window(version: &str, script_pid: u32, dir: &std::path::Path) {
    use crate::updating_window as uw;
    // A dev build started for a look check keeps its window off every display, like the rest of the app.
    agentmux_common::test_offscreen::begin("launcher");
    // The same line the splash writes, so the launcher log says which launcher painted this and why.
    let _ = crate::splash_info::SplashInfo::gather_and_log("upgrade");
    unsafe {
        let pool = send(class(b"NSAutoreleasePool\0"), sel(b"alloc\0"));
        let pool = send(pool, sel(b"init\0"));
        let (window, image_view, stage_fields, footer_labels) = build_window_with(Some(&updating_lines(version, uw::Phase::Updating)), false);
        send_void(pool, sel(b"drain\0"));
        let stage = stage_fields[0];
        let mut shown = uw::Phase::Updating;
        show_phase(&footer_labels, stage, &updating_lines(version, shown), uw::headline(shown));

        // Dev only: a card per phase, in both appearances, as PNGs; nothing else (see `dump_phases`).
        if let Ok(out) = std::env::var("AGENTMUX_UPDATING_DUMP_DIR") {
            if agentmux_common::build_kind::build_kind() != agentmux_common::build_kind::BuildKind::Release {
                dump_phases(window, &footer_labels, stage, image_view, version, std::path::Path::new(&out));
                send_void_id(window, sel(b"orderOut:\0"), NIL);
                return;
            }
        }

        let started = Instant::now();
        let mut script_ended_at: Option<Instant> = None;
        loop {
            pump_app_events(0.016);
            if script_ended_at.is_none() && !process_is_running(script_pid) {
                script_ended_at = Some(Instant::now());
            }
            let p = uw::phase(&uw::Inputs {
                elapsed: started.elapsed(),
                script_ended_for: script_ended_at.map(|t| t.elapsed()),
                new_splash: uw::new_splash_marker(dir).exists(),
                failed: script_ended_at.is_some() && uw::failed_marker(dir).exists(),
            });
            if p == uw::Phase::Close {
                break;
            }
            if p != shown {
                shown = p;
                show_phase(&footer_labels, stage, &updating_lines(version, p), uw::headline(p));
            }
            send_void_f64(image_view, sel(b"setAlphaValue:\0"), brain_alpha(started.elapsed().as_secs_f64()));
            std::thread::sleep(Duration::from_millis(8));
        }
        let fade = Instant::now();
        loop {
            let p = (fade.elapsed().as_secs_f64() / FADE_OUT).min(1.0);
            send_void_f64(window, sel(b"setAlphaValue:\0"), 1.0 - p);
            if p >= 1.0 {
                break;
            }
            pump_app_events(0.016);
        }
        send_void_id(window, sel(b"orderOut:\0"), NIL);
        pump_app_events(0.05);
    }
}

/// `AGENTMUX_UPDATING_DUMP_DIR=<folder>` (a dev build): render each phase's card, in the light and the dark
/// appearance, to `updating-window-<phase>-<appearance>.png` in `out`, offscreen (no screen capture permission and
/// nothing shown: the window is never on a display in a test build).
unsafe fn dump_phases(window: id, footer_labels: &[id], stage: (id, id), image_view: id, version: &str, out: &std::path::Path) {
    use crate::updating_window::{headline, Phase};
    let app = send(class(b"NSApplication\0"), sel(b"sharedApplication\0"));
    send_void_f64(image_view, sel(b"setAlphaValue:\0"), 1.0);
    for (appearance, name) in [("light", &b"NSAppearanceNameAqua\0"[..]), ("dark", &b"NSAppearanceNameDarkAqua\0"[..])] {
        let pool = send(class(b"NSAutoreleasePool\0"), sel(b"alloc\0"));
        let pool = send(pool, sel(b"init\0"));
        let named = send_id(class(b"NSAppearance\0"), sel(b"appearanceNamed:\0"), nsstring(std::str::from_utf8(&name[..name.len() - 1]).unwrap_or("")));
        send_void_id(app, sel(b"setAppearance:\0"), named);
        send_void(pool, sel(b"drain\0"));
        for (label, p) in [("updating", Phase::Updating), ("slow", Phase::Slow), ("failed", Phase::Failed)] {
            show_phase(footer_labels, stage, &updating_lines(version, p), headline(p));
            for _ in 0..10 {
                pump_app_events(0.016);
            }
            dump_window_png(window, &out.join(format!("updating-window-{label}-{appearance}.png")).to_string_lossy());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_updating_window_watches_the_script_by_its_pid() {
        assert!(process_is_running(std::process::id()));
        // A process that has ended (and been waited for) is gone; pid 0 is never a process to watch.
        let mut child = std::process::Command::new("/usr/bin/true").spawn().unwrap();
        let pid = child.id();
        child.wait().unwrap();
        assert!(!process_is_running(pid));
        assert!(!process_is_running(0));
        assert!(!process_is_running(u32::MAX));
    }

    #[test]
    fn the_updating_window_pulses_like_the_splash() {
        assert_eq!(brain_alpha(0.0), 0.0);
        assert!((brain_alpha(0.1) - 0.5).abs() < 1e-9);
        for i in 0..200 {
            let a = brain_alpha(0.2 + i as f64 * 0.01);
            assert!((0.73 - 1e-9..=1.0 + 1e-9).contains(&a), "{a}");
        }
    }

    #[test]
    fn the_updating_card_lines_fit_the_card_for_a_longer_version_too() {
        use crate::updating_window::Phase;
        for v in ["0.60.35", "1.2.3-rc.1", "0.60.35+build5"] {
            for p in [Phase::Updating, Phase::Slow, Phase::Failed] {
                assert!(updating_lines(v, p).iter().all(|l| l.len() <= 26), "{v} {p:?}");
            }
        }
    }
}
