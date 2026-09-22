// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! One-time sweep for identity conversation history stranded before
//! `ensure_history_link` (`SPEC_AGENT_IDENTITY_HISTORY_PERSISTENCE_PROTOCOL_2026_08_16.md`,
//! shipped in `0007c6b64051e8` / PR #2605) existed to keep it linked into
//! the always-global `shared/identities/<id>/<provider>/<subdir>/` tree.
//!
//! ## What went wrong
//!
//! `ensure_history_link` already does the hard part — migrate a real,
//! pre-existing directory's contents into the global target (never
//! clobbering an existing entry) and replace it with a junction/symlink —
//! but it only ever runs at `auth.start` and at ordinary agent spawn
//! (`identity_auth_dirs::link_history_if_isolated`, called from
//! `identity/resolver/inject.rs`). A channel that stopped being spawned
//! into before the fix landed — retired, superseded by a newer dev
//! branch, or simply abandoned — never spawns again, so that call site
//! never re-runs for it. Its real `identities/<bundle>/<provider>/<subdir>/`
//! directory is left exactly as it was: live conversation history that
//! `ClaudeHistoryAdapter`'s per-channel fallback scan can still find and
//! read (so it isn't *lost*), but that will never be linked into the
//! global tree, and will never benefit from a fix to the linking logic
//! itself, because nothing ever calls it again for that channel.
//!
//! Confirmed on a live install: two channels
//! (`local-agentx-fix-lan-discovery-tx-fc5809-ec08a24c`,
//! `local-main-b28b7a-0e5d07ad`) born on 2026-08-16 — the exact day the
//! fix shipped — carry four such real, unlinked, non-empty identity
//! history directories, last written 2026-09-03 and never touched since.
//!
//! ## What this does
//!
//! Walks every `identities/` directory found up to 2 levels under
//! `<home>/channels/*` and `<home>/dev/*` — covering `channels/<slug>/`,
//! `dev/<branch>/`, and the clone-scoped `dev/<branch>/<clone_id>/` shape
//! (see [`identities_roots`], and `ClaudeHistoryAdapter`'s identical
//! bounded traversal, which this mirrors on purpose). For every
//! `identities/<bundle_id>/<provider.auth_dir_name>/<history_subdir>` path
//! that exists on disk, calls the live `ensure_history_link` to merge it
//! into `<home>/shared/identities/<bundle_id>/<provider.auth_dir_name>/<history_subdir>`
//! and replace it with a link — the exact same operation a spawn would
//! have performed, just run once, retroactively, for every channel that
//! will never spawn again.
//!
//! Deliberately calls the LIVE `ensure_history_link` rather than freezing
//! a copy (contrast the guidance in this module's own doc comment about
//! migrations that produce DB rows): that guidance protects rows whose
//! *shape* is meaningful and time-bound. This migration produces no rows
//! at all — it performs a filesystem merge-and-link operation that
//! `ensure_history_link` already defines as idempotent and safe
//! ("never deletes or overwrites data"). Freezing a copy would mean a
//! future safety fix to that function (it has already had one real bug
//! fixed under it, per PR #2605's review) never applies to this
//! migration, on any machine that runs it after that fix ships — the
//! opposite of what freezing is meant to protect against.
//!
//! Scoped `Global`: it only ever writes under `<home>/shared/`, and must
//! run at most once regardless of which channel happens to run it first.
//!
//! Never invents new structure: only acts where a real
//! `identities/<bundle>/<provider>/<subdir>` path already exists on disk.
//! Does not create a link (or the target directory) for a bundle/provider
//! combination that was never present, which `ensure_history_link` would
//! otherwise happily do (it unconditionally `create_dir_all`s the
//! target). Best-effort per entry, matching `link_history_if_isolated`'s
//! own philosophy: one bundle's I/O error (e.g. a genuine name collision
//! `ensure_history_link` refuses to resolve automatically) is logged and
//! skipped, never aborts the sweep for every other bundle.

use agentmux_common::ensure_history_link;

use crate::backend::providers::all_providers;

use super::{Migration, MigrationContext, MigrationError, MigrationScope};

pub struct M0034IdentityHistoryBackfill;

impl Migration for M0034IdentityHistoryBackfill {
    fn id(&self) -> &'static str {
        "0034_identity_history_backfill"
    }
    fn scope(&self) -> MigrationScope {
        MigrationScope::Global
    }
    fn description(&self) -> &'static str {
        "Link orphaned pre-fix per-channel identity history into the global shared tree"
    }

    fn up(&self, ctx: &MigrationContext) -> Result<(), MigrationError> {
        let mut linked = 0usize;
        let mut failed = 0usize;

        for identities_root in identities_roots(&ctx.home) {
            let Ok(bundle_entries) = std::fs::read_dir(&identities_root) else {
                continue;
            };

            for bundle_entry in bundle_entries.flatten() {
                if !bundle_entry.path().is_dir() {
                    continue;
                }
                let bundle_id = bundle_entry.file_name().to_string_lossy().into_owned();
                if !is_safe_path_segment(&bundle_id) {
                    tracing::warn!(
                        bundle_id,
                        identities_root = %identities_root.display(),
                        "identity_history_backfill: skipping unsafe bundle id"
                    );
                    continue;
                }

                for provider in all_providers() {
                    let Some(subdir) = provider.history_native_subdir else {
                        continue;
                    };
                    let link_path = identities_root
                        .join(&bundle_id)
                        .join(provider.auth_dir_name)
                        .join(subdir);
                    // Only act where something already exists — never
                    // invent a bundle/provider pairing that wasn't here.
                    if std::fs::symlink_metadata(&link_path).is_err() {
                        continue;
                    }
                    let target_dir = ctx
                        .home
                        .join("shared")
                        .join("identities")
                        .join(&bundle_id)
                        .join(provider.auth_dir_name)
                        .join(subdir);

                    match ensure_history_link(&link_path, &target_dir) {
                        Ok(()) => linked += 1,
                        Err(e) => {
                            failed += 1;
                            tracing::warn!(
                                bundle_id,
                                provider_id = provider.id,
                                link_path = %link_path.display(),
                                error = %e,
                                "identity_history_backfill: failed to link a stranded \
                                 identity history directory; leaving it in place"
                            );
                        }
                    }
                }
            }
        }

        tracing::info!(linked, failed, "identity_history_backfill: complete");
        Ok(())
    }
}

/// Every `identities/` directory up to 2 levels under `<home>/channels/`
/// and `<home>/dev/`. Deliberately mirrors
/// `ClaudeHistoryAdapter::scan_isolated_identities_under`'s bounded
/// traversal (`agentmux-srv/src/backend/history/claude_adapter.rs`) rather
/// than assuming a fixed one-level shape: `DataPaths::resolve` places a
/// dev instance at `dev/<branch>/` when there is no clone id, but at
/// `dev/<branch>/<clone_id>/` whenever one is derived — which
/// `agentmux_common::runtime_mode::derive_clone_id` does for essentially
/// every real dev checkout (anywhere a `.git`/`Cargo.toml`/`Taskfile.yml`
/// is found up the tree), not an edge case. An earlier revision of this
/// function only checked one level under `dev/`, which silently found
/// nothing for that layout and would have marked the migration applied
/// without ever backfilling clone-scoped dev instances — caught in review
/// (ReAgent + Codex, independently, on the same commit) before merge.
/// Depth 2 covers every known layout: `channels/<slug>/identities/`,
/// `dev/<branch>/identities/`, and `dev/<branch>/<clone_id>/identities/`.
/// Kept as its own copy rather than calling the adapter's function
/// directly: that one is Claude-specific (hardcodes `claude/projects`)
/// and private to its module; this migration needs the `identities/`
/// directory itself, for every provider.
fn identities_roots(home: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    for base in ["channels", "dev"] {
        collect_identities_roots(&home.join(base), &mut out, 2);
    }
    out
}

fn collect_identities_roots(
    root: &std::path::Path,
    out: &mut Vec<std::path::PathBuf>,
    max_depth: u8,
) {
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let identities = path.join("identities");
        if identities.is_dir() {
            out.push(identities);
        }
        if max_depth > 1 {
            collect_identities_roots(&path, out, max_depth - 1);
        }
    }
}

/// Mirrors `agentmux_common::data_paths`'s private `sanitize_path_segment`
/// as a boolean check only (that function isn't `pub`, and this call site
/// only ever needs to know whether a name discovered on disk is safe to
/// re-join into a path — never the sanitized value itself, since the
/// original `bundle_entry` path is already known-safe on this filesystem
/// and is what actually gets used). Kept as its own small, stable copy
/// rather than exporting the original: this predicate is simple path
/// hygiene, not the isolation-sensitive logic `ensure_history_link` and
/// the migration framework's own "don't freeze what might change" note
/// are about.
fn is_safe_path_segment(s: &str) -> bool {
    if s != s.trim() || s.is_empty() || s == "." || s == ".." {
        return false;
    }
    !s.chars().any(|c| {
        matches!(
            c,
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' | '\0'
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx_at(home: std::path::PathBuf) -> MigrationContext {
        MigrationContext {
            shared_store_path: home.join("shared").join("store.db"),
            channel_store_path: home.join("unused-objects.db"),
            data_dir: home.clone(),
            home,
        }
    }

    /// The real-world case: a real, populated `projects/` directory left
    /// behind under an old channel's isolated identity dir must end up
    /// merged into the global tree, with the old location replaced by a
    /// link to it — the same outcome a spawn into that channel would
    /// have produced, had one ever happened again.
    #[test]
    fn links_a_stranded_real_history_directory_into_the_shared_tree() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().to_path_buf();

        let stranded = home
            .join("channels")
            .join("local-old-retired-channel")
            .join("identities")
            .join("bundle-1")
            .join("claude")
            .join("projects")
            .join("some-project");
        std::fs::create_dir_all(&stranded).unwrap();
        std::fs::write(stranded.join("session-1.jsonl"), "orphaned history").unwrap();

        M0034IdentityHistoryBackfill
            .up(&ctx_at(home.clone()))
            .unwrap();

        let global_file = home
            .join("shared")
            .join("identities")
            .join("bundle-1")
            .join("claude")
            .join("projects")
            .join("some-project")
            .join("session-1.jsonl");
        assert_eq!(
            std::fs::read_to_string(&global_file).unwrap(),
            "orphaned history",
            "content must be reachable from the global location after the sweep"
        );

        // The precise on-disk link mechanism (junction vs symlink) is
        // `ensure_history_link`'s own, already-tested responsibility;
        // this migration only needs to prove it called that primitive
        // for the right path. `is_symlink()` alone isn't a portable
        // enough check for a Windows junction specifically (see
        // `ensure_history_link`'s own doc comment on why it uses the
        // `junction` crate rather than `symlink_metadata` there), so
        // assert the content-reachability property instead, which is
        // what a caller actually depends on.
        let old_location_file = home
            .join("channels")
            .join("local-old-retired-channel")
            .join("identities")
            .join("bundle-1")
            .join("claude")
            .join("projects")
            .join("some-project")
            .join("session-1.jsonl");
        assert_eq!(
            std::fs::read_to_string(&old_location_file).unwrap(),
            "orphaned history",
            "old location must still resolve to the same content via the new link"
        );
    }

    /// Also sweeps `dev/<branch>/identities/...` — the other base
    /// `DataPaths::instance_dir` resolves to, not just `channels/*`.
    #[test]
    fn sweeps_dev_branch_roots_too() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().to_path_buf();

        let stranded = home
            .join("dev")
            .join("some-branch")
            .join("identities")
            .join("bundle-1")
            .join("codex")
            .join("sessions");
        std::fs::create_dir_all(&stranded).unwrap();
        std::fs::write(stranded.join("s.json"), "dev branch history").unwrap();

        M0034IdentityHistoryBackfill
            .up(&ctx_at(home.clone()))
            .unwrap();

        let global_file = home
            .join("shared")
            .join("identities")
            .join("bundle-1")
            .join("codex")
            .join("sessions")
            .join("s.json");
        assert_eq!(
            std::fs::read_to_string(&global_file).unwrap(),
            "dev branch history"
        );
    }

    /// The gap caught in review (ReAgent + Codex, independently, same
    /// commit): a dev build with a clone id nests one level deeper —
    /// `dev/<branch>/<clone_id>/identities/...`, per
    /// `DataPaths::resolve`'s `RuntimeMode::Dev` branch — which
    /// `sweeps_dev_branch_roots_too` above does not exercise. This is the
    /// common case, not an edge case: `derive_clone_id` succeeds (and so
    /// this layout applies) for essentially every real `task dev`
    /// checkout. Must be found and linked exactly like the shallow form.
    #[test]
    fn sweeps_clone_scoped_dev_roots_too() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().to_path_buf();

        let stranded = home
            .join("dev")
            .join("some-branch")
            .join("a1b2c3d4")
            .join("identities")
            .join("bundle-1")
            .join("gemini")
            .join("history");
        std::fs::create_dir_all(&stranded).unwrap();
        std::fs::write(stranded.join("h.json"), "clone-scoped dev history").unwrap();

        M0034IdentityHistoryBackfill
            .up(&ctx_at(home.clone()))
            .unwrap();

        let global_file = home
            .join("shared")
            .join("identities")
            .join("bundle-1")
            .join("gemini")
            .join("history")
            .join("h.json");
        assert_eq!(
            std::fs::read_to_string(&global_file).unwrap(),
            "clone-scoped dev history"
        );
    }

    /// A channel whose identity history is ALREADY correctly linked
    /// (the common case on any install that's spawned since the 08-16
    /// fix) must be left alone — `ensure_history_link` itself already
    /// treats this as a no-op, and this migration must not error or
    /// duplicate anything on top of it.
    #[test]
    fn is_a_no_op_for_an_already_linked_history_directory() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().to_path_buf();

        let target = home
            .join("shared")
            .join("identities")
            .join("bundle-1")
            .join("claude")
            .join("projects");
        std::fs::create_dir_all(&target).unwrap();
        std::fs::write(target.join("session-1.jsonl"), "already global").unwrap();

        let link_path = home
            .join("channels")
            .join("local-current")
            .join("identities")
            .join("bundle-1")
            .join("claude")
            .join("projects");
        std::fs::create_dir_all(link_path.parent().unwrap()).unwrap();
        ensure_history_link(&link_path, &target).unwrap();

        M0034IdentityHistoryBackfill
            .up(&ctx_at(home.clone()))
            .unwrap();

        assert_eq!(
            std::fs::read_to_string(target.join("session-1.jsonl")).unwrap(),
            "already global"
        );
    }

    /// Never invents a link (or the target dir) for a bundle/provider
    /// pairing that was never present on disk — only real, pre-existing
    /// paths get touched.
    #[test]
    fn never_creates_a_link_for_a_provider_that_was_never_present() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().to_path_buf();
        std::fs::create_dir_all(
            home.join("channels")
                .join("local-current")
                .join("identities")
                .join("bundle-1"),
        )
        .unwrap();

        M0034IdentityHistoryBackfill
            .up(&ctx_at(home.clone()))
            .unwrap();

        assert!(
            !home
                .join("shared")
                .join("identities")
                .join("bundle-1")
                .join("claude")
                .exists(),
            "no claude history dir should be materialized when none ever existed"
        );
    }

    /// Running it twice must not error or double-migrate — an operator
    /// re-running migrations is routine.
    #[test]
    fn is_idempotent_on_a_second_run() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().to_path_buf();

        let stranded = home
            .join("channels")
            .join("local-old")
            .join("identities")
            .join("bundle-1")
            .join("claude")
            .join("projects");
        std::fs::create_dir_all(&stranded).unwrap();
        std::fs::write(stranded.join("session-1.jsonl"), "orphaned history").unwrap();

        let ctx = ctx_at(home.clone());
        M0034IdentityHistoryBackfill.up(&ctx).unwrap();
        M0034IdentityHistoryBackfill.up(&ctx).unwrap();

        let global_file = home
            .join("shared")
            .join("identities")
            .join("bundle-1")
            .join("claude")
            .join("projects")
            .join("session-1.jsonl");
        assert_eq!(
            std::fs::read_to_string(&global_file).unwrap(),
            "orphaned history"
        );
    }

    /// A fresh install with no `channels/` or `dev/` dir at all must not
    /// error — the migration runner calls every migration on every
    /// install, including a brand new one.
    #[test]
    fn is_a_no_op_when_no_channel_roots_exist_yet() {
        let tmp = tempfile::tempdir().unwrap();
        M0034IdentityHistoryBackfill
            .up(&ctx_at(tmp.path().to_path_buf()))
            .unwrap();
    }

    #[test]
    fn rejects_unsafe_bundle_ids_found_on_disk() {
        assert!(is_safe_path_segment("bundle-1"));
        assert!(!is_safe_path_segment(".."));
        assert!(!is_safe_path_segment(""));
        assert!(!is_safe_path_segment("a/b"));
        assert!(!is_safe_path_segment(" padded "));
    }
}
