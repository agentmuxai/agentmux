// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Which window `CaptureWindow` picks, and what it tells the caller and the audit when it
//! cannot, with stand-in windows (the real ones need a desktop). Pinned so `capture_window_impl`
//! could be split without changing who can be captured or what a miss reveals.

use super::*;

struct Win {
    pid: u32,
    title: &'static str,
    agentmux: bool,
    tier: CaptureTier,
}

impl WindowFacts for Win {
    fn pid(&self) -> u32 {
        self.pid
    }
    fn title(&self) -> &str {
        self.title
    }
    fn is_agentmux(&self) -> bool {
        self.agentmux
    }
    fn tier(&self) -> CaptureTier {
        self.tier
    }
}

fn win(pid: u32, title: &'static str, agentmux: bool, tier: CaptureTier) -> Win {
    Win { pid, title, agentmux, tier }
}

/// Mine, a second of mine, another instance of mine, a browser, and another user's.
fn desktop() -> Vec<Win> {
    vec![
        win(10, "AgentMux main", true, CaptureTier::SameInstance),
        win(10, "AgentMux pool", true, CaptureTier::SameInstance),
        win(20, "AgentMux other", true, CaptureTier::OtherInstance),
        win(30, "Secret Vault - KeePass", false, CaptureTier::ForeignApp),
        win(40, "Their AgentMux notes", true, CaptureTier::OtherUser),
        win(41, "", true, CaptureTier::SameInstance),
    ]
}

fn pick(all: &[Win], title: Option<&str>, index: Option<usize>, pid: Option<u32>) -> (Result<u32>, Option<(CaptureTier, String)>) {
    let mut resolved = None;
    let picked = select_target(all, title, index, pid, &mut resolved).map(|w| w.pid);
    (picked, resolved)
}

fn error(all: &[Win], title: Option<&str>, index: Option<usize>, pid: Option<u32>) -> String {
    pick(all, title, index, pid).0.expect_err("should not pick").to_string()
}

#[test]
fn a_pid_picks_its_window_and_a_shared_pid_needs_an_index() {
    let all = desktop();
    assert_eq!(pick(&all, None, None, Some(20)).0.unwrap(), 20);
    assert_eq!(pick(&all, None, None, Some(30)).0.unwrap(), 30, "a foreign app's window is reachable by its pid");
    assert_eq!(pick(&all, None, Some(0), Some(10)).0.unwrap(), 10);
    assert_eq!(pick(&all, None, Some(1), Some(10)).0.unwrap(), 10);
    assert_eq!(
        error(&all, None, None, Some(10)),
        "2 windows matched pid 10 — a single process can own multiple top-level windows; pass an explicit index (0-based) alongside pid to disambiguate. Candidates: pid=10 title=\"AgentMux main\"; pid=10 title=\"AgentMux pool\""
    );
    assert_eq!(error(&all, None, Some(2), Some(10)), "index 2 out of range — only 2 window(s) matched pid 10");
    assert_eq!(error(&all, None, None, Some(77)), "no window found for pid 77 — use UI's windows action to see current candidates");
    // A pid wins over a title.
    assert_eq!(pick(&all, Some("AgentMux other"), None, Some(30)).0.unwrap(), 30);
}

#[test]
fn what_is_picked_is_recorded_for_the_audit_by_the_caller_but_a_withheld_pid_is_recorded_here() {
    let all = desktop();
    // Picking a window records nothing yet: the caller records the one it captures.
    assert!(pick(&all, None, None, Some(20)).1.is_none());
    // A pid of another user's window is named as withheld, and audited with its tier and no title.
    let (picked, resolved) = pick(&all, None, None, Some(40));
    assert_eq!(
        picked.expect_err("withheld").to_string(),
        "window pid=40 is T3-other-user — capture is withheld for windows owned by a different OS user; every other tier is available"
    );
    assert_eq!(resolved, Some((CaptureTier::OtherUser, "pid=40 T3-other-user <title withheld>".to_string())));
    // A pid nobody has is not audited as a target.
    assert!(pick(&all, None, None, Some(77)).1.is_none());
}

#[test]
fn a_title_picks_a_window_whatever_its_case_and_several_need_an_index() {
    let all = desktop();
    assert_eq!(pick(&all, Some("OTHER"), None, None).0.unwrap(), 20);
    assert_eq!(pick(&all, Some("keepass"), None, None).0.unwrap(), 30, "a foreign app's window is reachable by its title");
    assert_eq!(pick(&all, Some("agentmux"), Some(2), None).0.unwrap(), 20);
    assert_eq!(error(&all, Some("agentmux"), Some(9), None), "index 9 out of range — only 3 window(s) matched \"agentmux\"");
    // Several matches: every candidate by pid, a foreign app's title never among them.
    let e = error(&all, Some("a"), None, None);
    assert!(e.starts_with("4 windows matched \"a\" — pass an explicit index (0-based), or better, one of these pids directly (preferred: stable, unlike title). Candidates: "), "{e}");
    assert!(e.ends_with("pid=10 title=\"AgentMux main\"; pid=10 title=\"AgentMux pool\"; pid=20 title=\"AgentMux other\"; pid=30 <non-AgentMux window; pass include_foreign to UI's windows action to identify it>"), "{e}");
    assert!(!e.contains("KeePass") && !e.contains("Their AgentMux notes"), "{e}");
}

#[test]
fn neither_a_pid_nor_a_title_is_refused() {
    assert_eq!(error(&desktop(), None, None, None), "must provide either pid or title_contains");
    assert_eq!(error(&desktop(), None, Some(0), None), "must provide either pid or title_contains");
}

#[test]
fn a_title_that_matches_nothing_lists_only_the_agentmux_titles_it_may() {
    let all = desktop();
    assert_eq!(
        error(&all, Some("zzz"), None, None),
        "no AgentMux window title contains \"zzz\". Open AgentMux window titles: AgentMux main, AgentMux pool, AgentMux other"
    );
    // No titles to list: another thing to say, and still nothing of a foreign or withheld window.
    let bare = vec![win(30, "Secret Vault - KeePass", false, CaptureTier::ForeignApp), win(40, "Their notes", true, CaptureTier::OtherUser), win(41, "", true, CaptureTier::SameInstance)];
    assert_eq!(error(&bare, Some("zzz"), None, None), "no AgentMux window title contains \"zzz\" — no other AgentMux windows are currently open");
    assert!(error(&[], Some("zzz"), None, None).contains("no other AgentMux windows are currently open"));
}

#[test]
fn a_title_of_a_withheld_window_is_a_miss_that_looks_like_any_other_but_is_audited() {
    let all = desktop();
    // The caller must not be able to tell that another user's window matched: the same words, to the needle.
    let hit = error(&all, Some("their agentmux"), None, None);
    let miss = error(&all, Some("zzzzzzzzzzzzzz"), None, None);
    assert_eq!(hit.replace("their agentmux", "N"), miss.replace("zzzzzzzzzzzzzz", "N"));
    assert!(!hit.contains("T3") && !hit.contains("withheld") && !hit.contains("Their AgentMux notes"), "{hit}");
    // The audit knows.
    let (_, resolved) = pick(&all, Some("their agentmux"), None, None);
    assert_eq!(resolved, Some((CaptureTier::OtherUser, "pid=40 T3-other-user <title withheld>".to_string())));
    assert!(pick(&all, Some("zzzzzzzzzzzzzz"), None, None).1.is_none());
    // A withheld window is never a candidate, even when it is the only one that matches.
    let only_withheld = vec![win(40, "Their AgentMux notes", true, CaptureTier::OtherUser)];
    assert!(error(&only_withheld, Some("notes"), None, None).contains("no other AgentMux windows are currently open"));
    assert!(error(&only_withheld, Some("notes"), Some(0), None).contains("no other AgentMux windows"));
}

#[test]
fn the_audit_names_a_window_in_full_only_where_the_caller_could_have_captured_it() {
    assert_eq!(audit_target_label(&win(10, "AgentMux main", true, CaptureTier::SameInstance)), "pid=10 title=\"AgentMux main\"");
    assert_eq!(audit_target_label(&win(30, "KeePass", false, CaptureTier::ForeignApp)), "pid=30 title=\"KeePass\"");
    assert_eq!(audit_target_label(&win(40, "Their notes", true, CaptureTier::OtherUser)), "pid=40 T3-other-user <title withheld>");
    assert_eq!(candidate_label(&win(10, "AgentMux main", true, CaptureTier::SameInstance)), "pid=10 title=\"AgentMux main\"");
    assert!(!candidate_label(&win(30, "KeePass", false, CaptureTier::ForeignApp)).contains("KeePass"));
}

#[test]
fn a_capture_is_saved_as_a_png_of_its_own_in_the_directory_made_for_it() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("not").join("yet");
    let image = image::RgbaImage::from_pixel(3, 2, image::Rgba([10, 20, 30, 255]));
    let first = save_capture(&image, &dir).unwrap();
    let second = save_capture(&image, &dir).unwrap();
    assert_ne!(first, second);
    assert_eq!(first.parent(), Some(dir.as_path()));
    assert_eq!(first.extension().and_then(|e| e.to_str()), Some("png"));
    let read = image::open(&first).unwrap().to_rgba8();
    assert_eq!((read.width(), read.height(), read.get_pixel(0, 0).0), (3, 2, [10, 20, 30, 255]));
    assert_eq!(sha256_file(&first).map(|h| h.len()), Some(64));
    // A directory that cannot be made says so.
    let file = tmp.path().join("a-file");
    std::fs::write(&file, b"x").unwrap();
    let err = save_capture(&image, &file.join("sub")).unwrap_err().to_string();
    assert!(err.starts_with("failed to create capture dir "), "{err}");
}

#[test]
fn the_caller_is_told_where_the_capture_is_and_when_it_may_be_blank() {
    let path = std::path::Path::new("/tmp/x.png");
    let said = outcome_message("AgentMux main", path, false);
    assert_eq!(
        said,
        format!("Captured window \"AgentMux main\" to {} — use Read on that path to view it yourself, or Pane (action \"open\", view \"media\") to show it to the user.", path.display())
    );
    let blank = outcome_message("AgentMux main", path, true);
    assert!(blank.starts_with(&format!("Captured window \"AgentMux main\" to {} (likely_unrendered: true — the captured frame looks solid or near-solid-color even after retrying", path.display())), "{blank}");
    assert!(blank.contains("consider capturing it again shortly if that's unexpected) — use Read"), "{blank}");
    assert!(!blank.contains("  "), "no runs of spaces: {blank:?}");
}

#[test]
fn a_frame_of_one_colour_looks_unpainted_and_a_real_one_does_not() {
    assert!(looks_unrendered(&image::RgbaImage::from_pixel(40, 40, image::Rgba([5, 5, 5, 255]))));
    assert!(looks_unrendered(&image::RgbaImage::new(0, 0)));
    let mut busy = image::RgbaImage::from_pixel(40, 40, image::Rgba([5, 5, 5, 255]));
    for x in 0..40 {
        busy.put_pixel(x, 20, image::Rgba([200, 100, 50, 255]));
    }
    // The sample is ~200 evenly spaced pixels of 1,600: a painted frame has some that differ.
    for y in 0..40 {
        busy.put_pixel(y, y, image::Rgba([250, 250, 250, 255]));
    }
    assert!(!looks_unrendered(&busy));
}
