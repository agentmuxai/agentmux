// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A new version starts with the browser profile of the version before it
//! (`docs/specs/SPEC_UPGRADE_FOR_USERS_AND_AGENTS_2026_10_15.md` §6.3).
//!
//! CEF's cache folder is per version (`channels/<ch>/versions/<v>/cef-cache`),
//! like the databases srv carries forward (`srv/src/bootstrap/carry_forward.rs`),
//! and it holds what browser panes are signed in with: the Personal profile's
//! cookies, local storage and IndexedDB, and each named profile
//! (`profile-<id>`, `identity.rs`), plus the app UI's own
//! `localStorage`. Without this, every upgrade signed every browser pane out
//! of every site.
//!
//! On a version's first start (no `Local State` in its cache yet), before
//! CEF opens the folder, those are copied from the newest older version in
//! the same channel. `Local State` comes too: it holds the key the cookies
//! are encrypted with (`os_crypt`), without which the copies can't be read.
//! Copied, not moved, so the older version still works for a rollback, and
//! only forward: Chromium refuses a profile a newer Chromium wrote. The caches
//! that rebuild themselves (HTTP, code, GPU, shader) aren't copied. Anything
//! that fails is logged and left out: the version then starts that part
//! fresh, as it did before.

use std::path::{Path, PathBuf};

/// What a profile keeps that is worth carrying: sign-ins and site data.
const PROFILE_ITEMS: [&str; 4] = ["Network", "Local Storage", "IndexedDB", "Login Data"];
/// Folders that rebuild themselves, left out wherever they are.
const SKIPPED: [&str; 8] = ["Cache", "Code Cache", "GPUCache", "DawnCache", "DawnGraphiteCache", "DawnWebGPUCache", "GrShaderCache", "ShaderCache"];
/// The file that marks a cache folder as used, and holds the cookie key.
const LOCAL_STATE: &str = "Local State";

/// `major.minor.patch` of a version directory's name.
fn version_core(name: &str) -> Option<(u64, u64, u64)> {
    let core = name.trim().trim_start_matches('v').split(['+', '-']).next()?;
    let mut parts = core.split('.').map(|p| p.parse::<u64>().ok());
    let v = (parts.next()??, parts.next()??, parts.next()??);
    parts.next().is_none().then_some(v)
}

/// The `cef-cache` of the newest version older than the one `cache_dir`
/// belongs to (`versions/<v>/cef-cache`) that has been used.
fn previous_cache(cache_dir: &Path) -> Option<PathBuf> {
    let version_dir = cache_dir.parent()?;
    let current = version_core(version_dir.file_name()?.to_str()?)?;
    std::fs::read_dir(version_dir.parent()?)
        .ok()?
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let v = version_core(e.file_name().to_str()?)?;
            let cache = e.path().join("cef-cache");
            (v < current && cache.join(LOCAL_STATE).is_file()).then_some((v, cache))
        })
        .max_by_key(|(v, _)| *v)
        .map(|(_, cache)| cache)
}

/// Copy `src` to `dst`, a folder with everything under it but [`SKIPPED`].
fn copy_tree(src: &Path, dst: &Path) -> std::io::Result<u64> {
    if src.is_file() {
        if let Some(parent) = dst.parent() {
            std::fs::create_dir_all(parent)?;
        }
        return std::fs::copy(src, dst);
    }
    std::fs::create_dir_all(dst)?;
    let mut bytes = 0;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let name = entry.file_name();
        if SKIPPED.iter().any(|s| name == *s) || entry.file_name().to_string_lossy().ends_with("LOCK") {
            continue;
        }
        bytes += copy_tree(&entry.path(), &dst.join(&name))?;
    }
    Ok(bytes)
}

/// The profiles under a cache folder: `Default` and each `profile-<id>`.
fn profiles(cache: &Path) -> Vec<String> {
    let mut out = vec!["Default".to_string()];
    if let Ok(entries) = std::fs::read_dir(cache) {
        out.extend(
            entries
                .filter_map(|e| e.ok())
                .filter(|e| e.path().is_dir())
                .filter_map(|e| e.file_name().to_str().map(str::to_string))
                .filter(|n| n.starts_with("profile-")),
        );
    }
    out
}

/// The host's CEF cache folder, about to be opened: log it, and on a
/// version's first start carry the previous version's profiles into it.
pub fn prepare(cache_dir: &Path) {
    tracing::info!("CEF cache dir: {}", cache_dir.display());
    from_previous_version(cache_dir);
}

/// On the first start of the version `cache_dir` belongs to, copy the
/// browser profiles of the newest older version in the channel. Returns the
/// cache copied from, or `None` when there was nothing to do.
pub fn from_previous_version(cache_dir: &Path) -> Option<PathBuf> {
    if cache_dir.join(LOCAL_STATE).exists() {
        return None;
    }
    let from = previous_cache(cache_dir)?;
    if let Err(e) = std::fs::create_dir_all(cache_dir) {
        tracing::warn!(error = %e, "profile carry: can't create {}", cache_dir.display());
        return None;
    }
    let mut copied = 0u64;
    for profile in profiles(&from) {
        for item in PROFILE_ITEMS {
            let src = from.join(&profile).join(item);
            if !src.exists() {
                continue;
            }
            match copy_tree(&src, &cache_dir.join(&profile).join(item)) {
                Ok(n) => copied += n,
                Err(e) => tracing::warn!(error = %e, "profile carry: left out {}", src.display()),
            }
        }
    }
    // Last: its presence marks this cache as used, so a copy cut short is
    // tried again on the next start.
    if let Err(e) = std::fs::copy(from.join(LOCAL_STATE), cache_dir.join(LOCAL_STATE)) {
        tracing::warn!(error = %e, "profile carry: couldn't copy Local State; the copied sign-ins won't open");
        return None;
    }
    tracing::info!(from = %from.display(), bytes = copied, "profile carry: browser profiles carried from the previous version");
    Some(from)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(path: &Path, text: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    /// A version's used cache: Local State, a Personal profile with cookies,
    /// local storage and an HTTP cache, and a named profile.
    fn used_cache(versions: &Path, version: &str, marker: &str) -> PathBuf {
        let cache = versions.join(version).join("cef-cache");
        file(&cache.join(LOCAL_STATE), &format!("key {marker}"));
        file(&cache.join("Default").join("Network").join("Cookies"), &format!("cookies {marker}"));
        file(&cache.join("Default").join("Local Storage").join("leveldb").join("000003.log"), marker);
        file(&cache.join("Default").join("Local Storage").join("leveldb").join("LOCK"), "");
        file(&cache.join("Default").join("Cache").join("Cache_Data").join("data_0"), "big");
        file(&cache.join("profile-work").join("Network").join("Cookies"), &format!("work {marker}"));
        file(&cache.join("profile-work").join("Code Cache").join("js").join("x"), "big");
        cache
    }

    #[test]
    fn a_new_version_starts_signed_in_where_the_last_one_was() {
        let root = tempfile::tempdir().unwrap();
        let versions = root.path().join("versions");
        used_cache(&versions, "0.59.30", "30");
        used_cache(&versions, "0.59.31", "31");
        let new = versions.join("0.59.32").join("cef-cache");

        let from = from_previous_version(&new).unwrap();
        assert!(from.ends_with("0.59.31/cef-cache") || from.ends_with(r"0.59.31\cef-cache"), "{}", from.display());
        let read = |p: &[&str]| std::fs::read_to_string(p.iter().fold(new.clone(), |acc, s| acc.join(s))).unwrap();
        assert_eq!(read(&[LOCAL_STATE]), "key 31", "the cookie key comes with the cookies");
        assert_eq!(read(&["Default", "Network", "Cookies"]), "cookies 31");
        assert_eq!(read(&["Default", "Local Storage", "leveldb", "000003.log"]), "31");
        assert_eq!(read(&["profile-work", "Network", "Cookies"]), "work 31");
        // Caches that rebuild themselves and lock files stay behind.
        assert!(!new.join("Default").join("Cache").exists());
        assert!(!new.join("profile-work").join("Code Cache").exists());
        assert!(!new.join("Default").join("Local Storage").join("leveldb").join("LOCK").exists());
        // The older version keeps its own.
        assert!(versions.join("0.59.31").join("cef-cache").join("Default").join("Network").join("Cookies").exists());
        // Once used, never again.
        assert_eq!(from_previous_version(&new), None);
    }

    #[test]
    fn nothing_is_copied_without_an_older_used_version() {
        let root = tempfile::tempdir().unwrap();
        let versions = root.path().join("versions");
        // Only a newer one (a downgrade): Chromium refuses its profile.
        used_cache(&versions, "0.59.33", "33");
        // An older one never started: no Local State.
        std::fs::create_dir_all(versions.join("0.59.20").join("cef-cache").join("Default")).unwrap();
        let new = versions.join("0.59.32").join("cef-cache");
        assert_eq!(from_previous_version(&new), None);
        assert!(!new.join(LOCAL_STATE).exists());
        // A dev build's cache isn't under a version folder at all.
        assert_eq!(from_previous_version(&root.path().join("dev").join("cef-cache")), None);
    }
}
