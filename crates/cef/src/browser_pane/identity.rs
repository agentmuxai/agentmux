// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Which cookie jar (CEF request context) each browser pane browses in
//! (docs/specs/SPEC_BROWSER_PANE_PROFILES_MENU_2026_10_09.md §6,
//! SPEC_BROWSER_PANE_IDENTITIES_2026_09_22.md §4.1).
//!
//! A tab's identity is its block's `browser:identity`, which the frontend
//! passes with `browser_pane_create`: absent for Personal (the global jar, as
//! every pane had before), `incognito:<jar>` for an in-memory jar of its own.
//! Popup panes carry their opener's value, so they share its jar. The jar is
//! kept here, keyed by identity, for as long as srv says a block uses it, so
//! closing and recreating the native pane (tear-off, redock, a remount) keeps
//! the tab signed in; when no block uses it any more it is dropped, and with
//! it everything the tab saved.
//!
//! `profile:<id>` is a named profile: a disk-backed jar at
//! `<cef-cache>/profile-<id>` (a direct child of the cache root, the one
//! layout Chrome accepts), one per profile, shared by its tabs and kept across
//! restarts. A disk-backed profile is created asynchronously, and a browser
//! created in it before it is ready never finishes
//! (SPEC_BROWSER_PANE_IDENTITIES_2026_09_22.md §1.5 A), so its jar is made
//! first and its tabs' panes wait until CEF says it is initialized.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use cef::*;

#[derive(Default)]
struct Registry {
    /// Block → its identity, as the frontend last said when creating it.
    by_block: HashMap<String, String>,
    /// srv's last list of the identities blocks use (`retain_jars`), so a
    /// closed tab's jar doesn't count toward the cap until the next push.
    live: Option<HashSet<String>>,
    /// Identity → its jar. Only `incognito:` identities have one here.
    jars: HashMap<String, Jar>,
    /// Profile id → its disk-backed jar.
    profiles: HashMap<String, ProfileJar>,
    /// Every profile opened by this process, deleted ones included: CEF keeps
    /// using a profile's folder until the process ends, so their folders go
    /// at the next start.
    opened: HashSet<String>,
    /// Pane label → how many times its creation has waited for its profile.
    waits: HashMap<String, u32>,
}

struct ProfileJar {
    ctx: cef::RequestContext,
    ready: Arc<AtomicBool>,
}

wrap_request_context_handler! {
    struct ProfileReady {
        ready: Arc<AtomicBool>,
    }

    impl RequestContextHandler {
        fn on_request_context_initialized(&self, _request_context: Option<&mut RequestContext>) {
            self.ready.store(true, Ordering::SeqCst);
        }
    }
}

/// How the pane for a block is created: in the shared jar, in this one, or
/// not yet (its profile is still being made ready; try again shortly).
pub enum PaneJar {
    Shared,
    Ready(cef::RequestContext),
    Pending,
}

/// A pane waits for its profile at most this many times, 100 ms apart.
const MAX_PROFILE_WAITS: u32 = 100;

struct Jar {
    ctx: cef::RequestContext,
    created: std::time::Instant,
}

/// A new jar is kept this long even when srv's list doesn't name it yet: the
/// list can have been taken just before the tab's block existed and reach
/// the host just after the jar was made.
const NEW_JAR_GRACE: std::time::Duration = std::time::Duration::from_secs(30);

// SAFETY: a `RequestContext` is a refcounted CEF handle, safe to hold and drop
// from any thread; it is only used (`create_browser`) on the CEF UI thread.
unsafe impl Send for Registry {}

fn registry() -> std::sync::MutexGuard<'static, Registry> {
    static R: OnceLock<Mutex<Registry>> = OnceLock::new();
    R.get_or_init(Default::default).lock().unwrap_or_else(|p| p.into_inner())
}

/// Most Incognito jars at once: each is its own Chrome profile with its own
/// renderer (identities spec §4.3). srv refuses a ninth tab with a message;
/// this is the host's own backstop.
pub const MAX_INCOGNITO_JARS: usize = 8;

/// Is `identity` one this host gives a jar of its own?
pub fn is_incognito(identity: &str) -> bool {
    identity
        .strip_prefix("incognito:")
        .is_some_and(|jar| (8..=64).contains(&jar.len()) && jar.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-'))
}

/// The profile id of a `profile:<id>` identity.
pub fn profile_id(identity: &str) -> Option<&str> {
    identity
        .strip_prefix("profile:")
        .filter(|id| (1..=64).contains(&id.len()) && id.bytes().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-'))
}

/// Remember the identity `browser_pane_create` gave for `block` (absent or
/// unknown: Personal). Called before the pane is created, so every creation
/// path for the block (a fresh pane, one replayed after a close) finds it.
pub fn set_for_block(block: &str, identity: Option<&str>) {
    let mut r = registry();
    match identity.filter(|i| is_incognito(i) || profile_id(i).is_some()) {
        Some(i) => {
            r.by_block.insert(block.to_string(), i.to_string());
        }
        None => {
            r.by_block.remove(block);
        }
    }
}

/// Drop the jars no tab uses any more, by srv's last list, past their grace.
fn prune(r: &mut Registry) -> Vec<String> {
    let Some(live) = r.live.as_ref() else { return Vec::new() };
    let dropped: Vec<String> = r
        .jars
        .iter()
        .filter(|(identity, jar)| !live.contains(*identity) && jar.created.elapsed() >= NEW_JAR_GRACE)
        .map(|(identity, _)| identity.clone())
        .collect();
    for identity in &dropped {
        r.jars.remove(identity);
    }
    // Only the blocks of the jars dropped here: a block whose jar isn't made
    // yet keeps its identity, or its pane would be created in the shared jar.
    r.by_block.retain(|_, identity| !dropped.contains(identity));
    dropped
}

/// Can `block` have its jar? Checked when `browser_pane_create` arrives, so
/// a refusal goes back to the pane, which shows it, rather than leaving it
/// blank. Also refuses an Incognito tab where panes can't have a jar of their
/// own yet (Linux and macOS): it must not browse in the shared jar instead.
pub fn check_capacity(block: &str) -> Result<(), String> {
    let mut r = registry();
    let Some(identity) = r.by_block.get(block).cloned() else { return Ok(()) };
    if !cfg!(windows) {
        return Err("Incognito tabs and browser profiles are Windows only for now".to_string());
    }
    if profile_id(&identity).is_some() {
        return Ok(());
    }
    prune(&mut r);
    if r.jars.contains_key(&identity) || r.jars.len() < MAX_INCOGNITO_JARS {
        Ok(())
    } else {
        Err(format!("at most {MAX_INCOGNITO_JARS} Incognito tabs can be open at once: close one to open another"))
    }
}

/// Does `block` browse as an Incognito identity?
pub fn has_identity(block: &str) -> bool {
    registry().by_block.contains_key(block)
}

/// The jar `block`'s pane is created in (CEF UI thread). Creates an
/// Incognito jar, or a profile's, the first time it is used; a profile's is
/// `Pending` until CEF has made it ready. `Err` when the identity needs a jar
/// of its own and none can be made: the pane must not be created in the
/// shared jar instead, which would put the tab in your Personal session.
pub fn context_for_block(block: &str, cache_root: Option<&str>) -> Result<PaneJar, String> {
    let mut r = registry();
    let Some(identity) = r.by_block.get(block).cloned() else { return Ok(PaneJar::Shared) };
    if let Some(id) = profile_id(&identity) {
        if let Some(p) = r.profiles.get(id) {
            return Ok(if p.ready.load(Ordering::SeqCst) { PaneJar::Ready(p.ctx.clone()) } else { PaneJar::Pending });
        }
        let root = cache_root.filter(|r| !r.is_empty()).ok_or("no profile folder: the cache root isn't known")?;
        let path = std::path::Path::new(root).join(format!("profile-{id}"));
        let settings = cef::RequestContextSettings {
            cache_path: cef::CefString::from(path.to_string_lossy().as_ref()),
            persist_session_cookies: 1,
            ..Default::default()
        };
        let ready = Arc::new(AtomicBool::new(false));
        let mut handler = ProfileReady::new(ready.clone());
        let ctx = cef::request_context_create_context(Some(&settings), Some(&mut handler))
            .ok_or_else(|| format!("CEF couldn't open browser profile {id:?}"))?;
        tracing::info!(block, profile = id, path = %path.display(), "[browser-identity] opening a browser profile");
        let now_ready = ready.load(Ordering::SeqCst);
        r.profiles.insert(id.to_string(), ProfileJar { ctx: ctx.clone(), ready });
        r.opened.insert(id.to_string());
        return Ok(if now_ready { PaneJar::Ready(ctx) } else { PaneJar::Pending });
    }
    if let Some(jar) = r.jars.get(&identity) {
        return Ok(PaneJar::Ready(jar.ctx.clone()));
    }
    prune(&mut r);
    if r.jars.len() >= MAX_INCOGNITO_JARS {
        return Err(format!("at most {MAX_INCOGNITO_JARS} Incognito tabs at once"));
    }
    // Empty cache_path: a unique in-memory profile, nothing written to disk,
    // the same path every secondary window already uses
    // (commands::create_isolated_request_context).
    let settings = cef::RequestContextSettings {
        cache_path: cef::CefString::from(""),
        persist_session_cookies: 0,
        ..Default::default()
    };
    let ctx = cef::request_context_create_context(Some(&settings), None)
        .ok_or_else(|| "CEF couldn't create an Incognito jar".to_string())?;
    tracing::info!(block, identity = %identity, "[browser-identity] created an Incognito jar");
    r.jars.insert(identity, Jar { ctx: ctx.clone(), created: std::time::Instant::now() });
    Ok(PaneJar::Ready(ctx))
}

/// Another wait for pane `label`'s profile, if it has waits left; false once
/// it has waited `MAX_PROFILE_WAITS` times (the count then starts over).
pub fn wait_once_more(label: &str) -> bool {
    let mut r = registry();
    let n = r.waits.entry(label.to_string()).or_insert(0);
    *n += 1;
    if *n > MAX_PROFILE_WAITS {
        r.waits.remove(label);
        return false;
    }
    true
}

/// Whether pane `label` has waited for its profile: a creation run now is a
/// retry.
pub fn is_waiting(label: &str) -> bool {
    registry().waits.contains_key(label)
}

/// Pane `label` was created: forget its waits.
pub fn done_waiting(label: &str) {
    registry().waits.remove(label);
}

// Sign a deleted profile out at once: its cookies go now. CEF keeps an
// opened profile's files in use for the life of the process, so its folder
// itself goes at the next start (`retain_profiles`).
wrap_task! {
    struct ForgetProfileTask {
        ctx: cef::RequestContext,
    }

    impl Task {
        fn execute(&self) {
            // Each call gets a callback, if one that does nothing: with none,
            // libcef crashed (an access violation) a few seconds later.
            if let Some(cookies) = self.ctx.cookie_manager(None) {
                cookies.delete_cookies(None, None, Some(&mut CookiesDeleted::new()));
                cookies.flush_store(Some(&mut Done::new()));
            }
            self.ctx.clear_http_auth_credentials(Some(&mut Done::new()));
        }
    }
}

wrap_delete_cookies_callback! {
    struct CookiesDeleted;

    impl DeleteCookiesCallback {
        fn on_complete(&self, _num_deleted: ::std::os::raw::c_int) {}
    }
}

wrap_completion_callback! {
    struct Done;

    impl CompletionCallback {
        fn on_complete(&self) {}
    }
}

/// Writes the cookies of `block`'s profile, if it has one, to disk now.
/// Chromium writes them every 30 seconds or so, and the host doesn't shut
/// CEF down on quit, so a sign-in made just before quitting was lost; a
/// sign-in ends in a page load, which calls this.
pub fn flush_profile_of_block(block: &str) {
    let ctx = {
        let r = registry();
        let Some(id) = r.by_block.get(block).and_then(|i| profile_id(i)) else { return };
        let Some(p) = r.profiles.get(id) else { return };
        p.ctx.clone()
    };
    if let Some(cookies) = ctx.cookie_manager(None) {
        cookies.flush_store(Some(&mut Done::new()));
    }
}

/// srv's list of the named profiles there are: the jar of one not in it is
/// dropped and its sign-ins cleared at once. Folders under `cache_root` of
/// profiles not in it are deleted, except those of profiles this process
/// opened: CEF uses an opened profile's folder until the process ends, so
/// those go at the next start, before any profile is open.
pub fn retain_profiles(ids: &HashSet<String>, cache_root: Option<&str>) {
    let mut r = registry();
    let gone: Vec<String> = r.profiles.keys().filter(|id| !ids.contains(*id)).cloned().collect();
    for id in &gone {
        if let Some(p) = r.profiles.remove(id) {
            let mut task = ForgetProfileTask::new(p.ctx);
            post_task(ThreadId::UI, Some(&mut task));
        }
        tracing::info!(profile = %id, "[browser-identity] closed a deleted browser profile and cleared its sign-ins");
    }
    let opened = r.opened.clone();
    drop(r);
    let Some(root) = cache_root.filter(|r| !r.is_empty()) else { return };
    let Ok(entries) = std::fs::read_dir(root) else { return };
    for e in entries.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        let Some(id) = name.strip_prefix("profile-") else { continue };
        if ids.contains(id) || opened.contains(id) || profile_id(&format!("profile:{id}")).is_none() {
            continue;
        }
        match std::fs::remove_dir_all(e.path()) {
            Ok(()) => tracing::info!(profile = %id, "[browser-identity] deleted a deleted profile's folder"),
            Err(err) => tracing::debug!(profile = %id, error = %err, "[browser-identity] profile folder not deleted yet"),
        }
    }
}

/// srv's list of the identities blocks still use: every jar not in it (past
/// its grace period) is dropped, since its tabs were closed, and with it all
/// it held. So is what this host remembers of the blocks that used it.
pub fn retain_jars(live: &HashSet<String>) {
    let mut r = registry();
    r.live = Some(live.clone());
    let dropped = prune(&mut r);
    if !dropped.is_empty() {
        tracing::info!(dropped = dropped.len(), "[browser-identity] dropped jars no tab uses");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_well_formed_incognito_identities_get_a_jar() {
        assert!(is_incognito("incognito:0f0e2d1c-aaaa-bbbb-cccc-0123456789ab"));
        assert!(!is_incognito("incognito:short"));
        assert!(!is_incognito("incognito:has spaces in it"));
        assert!(!is_incognito("profile:work"));
        assert!(!is_incognito(""));
    }

    #[test]
    fn a_block_without_an_identity_uses_the_global_jar() {
        set_for_block("idt-a", None);
        assert!(matches!(context_for_block("idt-a", None).unwrap(), PaneJar::Shared));
        set_for_block("idt-b", Some("profile:Not Valid"));
        assert!(matches!(context_for_block("idt-b", None).unwrap(), PaneJar::Shared));
    }

    #[test]
    fn deleting_a_profile_leaves_the_folder_of_one_opened_this_run() {
        let root = std::env::temp_dir().join(format!("idt-sweep-{}", std::process::id()));
        for d in ["profile-p-sweepopen", "profile-p-sweepold", "profile-not valid"] {
            std::fs::create_dir_all(root.join(d)).unwrap();
        }
        // Opened by this process, then deleted: CEF still uses its folder.
        registry().opened.insert("p-sweepopen".to_string());
        retain_profiles(&HashSet::new(), root.to_str());
        assert!(root.join("profile-p-sweepopen").exists());
        assert!(!root.join("profile-p-sweepold").exists());
        assert!(root.join("profile-not valid").exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn profile_identities_are_recognised() {
        assert_eq!(profile_id("profile:p-work1"), Some("p-work1"));
        assert_eq!(profile_id("profile:Work"), None);
        assert_eq!(profile_id("profile:"), None);
        assert_eq!(profile_id("incognito:0f0e2d1c-aaaa"), None);
        set_for_block("idt-p", Some("profile:p-work1"));
        assert!(has_identity("idt-p"));
        assert_eq!(check_capacity("idt-p").is_ok(), cfg!(windows));
        // No cache root: no folder to put the profile in, so no pane.
        assert!(context_for_block("idt-p", None).is_err());
        set_for_block("idt-p", None);
    }

    #[test]
    fn a_pane_waits_for_its_profile_a_bounded_number_of_times() {
        assert!(!is_waiting("idt-wait"));
        for _ in 0..MAX_PROFILE_WAITS {
            assert!(wait_once_more("idt-wait"));
        }
        assert!(is_waiting("idt-wait"), "its next creation is a retry");
        assert!(!wait_once_more("idt-wait"));
        assert!(wait_once_more("idt-wait"), "the count starts over");
        done_waiting("idt-wait");
        assert!(!is_waiting("idt-wait"));
    }

    #[test]
    fn a_deleted_profiles_folder_is_removed_and_others_kept() {
        let dir = tempfile::tempdir().unwrap();
        for name in ["profile-p-keep", "profile-p-gone", "Default", "profile-Bad Name"] {
            std::fs::create_dir(dir.path().join(name)).unwrap();
        }
        retain_profiles(&["p-keep".to_string()].into(), Some(dir.path().to_str().unwrap()));
        assert!(dir.path().join("profile-p-keep").exists());
        assert!(!dir.path().join("profile-p-gone").exists());
        assert!(dir.path().join("Default").exists());
        assert!(dir.path().join("profile-Bad Name").exists());
    }

    #[test]
    fn a_block_whose_jar_isnt_made_yet_keeps_its_identity() {
        // srv's list can lag the block: the identity must survive it, or the
        // pane would be created in the shared jar.
        set_for_block("idt-c", Some("incognito:0f0e2d1c-cccc"));
        retain_jars(&HashSet::new());
        assert_eq!(registry().by_block.get("idt-c").map(String::as_str), Some("incognito:0f0e2d1c-cccc"));
        set_for_block("idt-c", None);
    }
}
