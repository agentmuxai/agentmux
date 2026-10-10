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

use std::collections::{HashMap, HashSet};
use std::sync::{Mutex, OnceLock};

#[derive(Default)]
struct Registry {
    /// Block → its identity, as the frontend last said when creating it.
    by_block: HashMap<String, String>,
    /// Identity → its jar. Only `incognito:` identities have one here.
    jars: HashMap<String, Jar>,
}

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

/// Remember the identity `browser_pane_create` gave for `block` (absent or
/// unknown: Personal). Called before the pane is created, so every creation
/// path for the block (a fresh pane, one replayed after a close) finds it.
pub fn set_for_block(block: &str, identity: Option<&str>) {
    let mut r = registry();
    match identity.filter(|i| is_incognito(i)) {
        Some(i) => {
            r.by_block.insert(block.to_string(), i.to_string());
        }
        None => {
            r.by_block.remove(block);
        }
    }
}

/// The jar `block`'s pane is created in: `None` for the global jar. Creates
/// an Incognito jar the first time its identity is used (CEF UI thread).
/// `Err` when the identity needs a jar of its own and none can be made: the
/// pane must not be created in the shared jar instead, which would put an
/// Incognito tab in your Personal session.
pub fn context_for_block(block: &str) -> Result<Option<cef::RequestContext>, String> {
    let mut r = registry();
    let Some(identity) = r.by_block.get(block).cloned() else { return Ok(None) };
    if let Some(jar) = r.jars.get(&identity) {
        return Ok(Some(jar.ctx.clone()));
    }
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
    Ok(Some(ctx))
}

/// srv's list of the identities blocks still use: every jar not in it (past
/// its grace period) is dropped, since its tabs were closed, and with it all
/// it held. So is what this host remembers of the blocks that used it.
pub fn retain_jars(live: &HashSet<String>) {
    let mut r = registry();
    let dropped: Vec<String> = r
        .jars
        .iter()
        .filter(|(identity, jar)| !live.contains(*identity) && jar.created.elapsed() >= NEW_JAR_GRACE)
        .map(|(identity, _)| identity.clone())
        .collect();
    if dropped.is_empty() {
        return;
    }
    for identity in &dropped {
        r.jars.remove(identity);
    }
    // Only the blocks of the jars dropped here: a block whose jar isn't made
    // yet keeps its identity, or its pane would be created in the shared jar.
    r.by_block.retain(|_, identity| !dropped.contains(identity));
    tracing::info!(dropped = dropped.len(), "[browser-identity] dropped jars no tab uses");
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
    fn a_block_without_an_incognito_identity_uses_the_global_jar() {
        set_for_block("idt-a", None);
        assert!(context_for_block("idt-a").unwrap().is_none());
        set_for_block("idt-b", Some("profile:work"));
        assert!(context_for_block("idt-b").unwrap().is_none());
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
