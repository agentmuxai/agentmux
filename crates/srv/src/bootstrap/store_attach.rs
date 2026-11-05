// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The steps of [`open_stores_and_migrate`](super::open_stores_and_migrate) that attach the
//! registries and open each store, one function each, in the order they ran. Moved out of that
//! function without a change in behaviour.

use super::stores::restore_repaired_history;
use super::*;
use crate::backend::memory::global_record::Recorder;

/// Attaches the cross-version named-agent registry, and the catch-up pass over its records.
pub(super) fn attach_agent_registry(mstore_raw: &Store) {
    // Attach the cross-version named-agent registry. Falls back to a
    // disabled registry when the shared home can't be resolved (CI,
    // unusual envs); mutations still hit SQLite, just don't mirror.
    // See docs/specs/SPEC_SHARED_AGENT_REGISTRY_2026_05_12.md.
    if let Some(root) = registry::resolve_shared_registry_dir() {
        match crate::boot_timing::time("open_agent_registry", || {
            registry::Registry::open(root.clone())
        }) {
            Ok(reg) => {
                tracing::info!(root = %root.display(), "registry: shared agent registry attached");
                // Best-effort catch-up pass, run every startup (not just the
                // one-shot m0010 migration): fills session_id for any agent
                // record that still lacks one, e.g. one named after m0010
                // already ran. Idempotent — skips records that already carry
                // a non-empty session_id. Without this, an agent created
                // after the migration ran would never get a resumable
                // session_id in the registry, so a cross-tab/cross-restart
                // open would silently orphan its conversation (the still-open
                // half of docs/retro/retro-cross-channel-conversation-continuity-regression-2026-06-16.md).
                if let Some(shared_dir) = root.parent().and_then(|p| p.parent()) {
                    let filled = crate::boot_timing::time("registry_session_backfill", || {
                        backend::session_backfill::backfill_session_ids(&reg, shared_dir)
                    });
                    if filled > 0 {
                        tracing::info!(filled, "registry: backfilled session_id for cross-channel resume (startup pass)");
                    }
                }
                mstore_raw.set_registry(Arc::new(reg));
                if let Some(base) = std::env::var_os("AGENTMUX_AGENTS_DIR") {
                    if !base.is_empty() {
                        mstore_raw.set_registry_agents_base(std::path::PathBuf::from(base));
                    }
                }
            }
            Err(e) => tracing::warn!(
                root = %root.display(),
                error = %e,
                "registry: failed to open shared agent registry — SQLite remains authoritative"
            ),
        }
    } else {
        tracing::warn!("registry: could not resolve shared registry dir — mirror disabled");
    }
}

/// Attaches the global agent-definition store and returns the ids of the active user agents, for the
/// transcript backfill.
pub(super) fn attach_definition_store(mstore_raw: &Store) -> Vec<String> {
    // Attach the GLOBAL (cross-channel) agent-definition store. Sibling of the
    // instance registry above — since P0.3b both live under ~/.agentmux/shared/
    // (definitions/ and registry/), so user agents created in one channel are
    // visible in every channel. Best-effort: disabled when the shared dir can't
    // be resolved. See SPEC_CROSS_CHANNEL_AGENT_PERSISTENCE_2026-06-13.md (P0.2/P0.3).
    // Captured for the transcript backfill below (after the global transcript
    // store opens): the user-agent definition ids to seed conversations for.
    let mut backfill_def_ids: Vec<String> = Vec::new();
    if let Some(def_dir) = registry::resolve_shared_definitions_dir() {
        match crate::boot_timing::time("open_definition_store", || {
            registry::DefinitionStore::open(def_dir.clone())
        }) {
            Ok(def_store) => {
                // Capture user-agent ids for the transcript backfill (below).
                backfill_def_ids = crate::boot_timing::time("def_store_list_active", || {
                    def_store
                        .list_active()
                        .map(|v| v.into_iter().map(|r| r.data.id).collect())
                        .unwrap_or_default()
                });
                mstore_raw.set_def_registry(Arc::new(def_store));
                tracing::info!(dir = %def_dir.display(), "def registry: global definition store attached");
                // New slugs are checked against every channel's and version's
                // objects.db too, the root the fork slug migration reads.
                match backend::slug_migration::Home::from_env() {
                    Ok(home) => mstore_raw.set_slug_home(home),
                    Err(e) => {
                        tracing::warn!(error = %e, "agent slugs: other channels' agents can't be checked; only the shared definitions are")
                    }
                }
                // This channel's copy of every agent's former slugs, in step
                // with the shared records (SPEC_FORK_SLUG_MIGRATION_2026_10_22.md Phase 1).
                if let Err(e) = mstore_raw.agent_former_slugs_sync_from_shared() {
                    tracing::warn!(error = %e, "former slugs: not copied from the shared definitions");
                }
            }
            Err(e) => tracing::warn!(
                dir = %def_dir.display(),
                error = %e,
                "def registry: failed to open global definition store — definitions stay channel-local"
            ),
        }
    } else {
        tracing::warn!(
            "def registry: could not resolve shared definitions dir — global definitions disabled"
        );
    }
    backfill_def_ids
}

/// Reconciles the two host-global registries against each other.
pub(super) fn reconcile_registries(mstore_raw: &Store) {
    // Both host-global trees are attached now, so they can be reconciled
    // against each other. Best-effort catch-up pass run every startup, same
    // shape as the session_id backfill above: drops instance records whose
    // definition is tombstoned, which the picker would otherwise keep
    // rendering as an iconless "(missing definition)" row — a deleted agent
    // whose card never went away. New deletes sweep the registry themselves
    // (`Store::agent_def_delete`); this is for the orphans already on disk.
    if let (Some(reg), Some(defs)) = (
        mstore_raw.shared_agent_registry(),
        mstore_raw.shared_def_registry(),
    ) {
        let pruned = backend::registry_reconcile::prune_tombstoned_instance_records(&reg, &defs);
        if pruned > 0 {
            tracing::info!(
                pruned,
                "registry: dropped instance records for deleted agents (startup pass)"
            );
        }
    }
}

/// Attaches the agent token index and the channel-wide WAN identity store.
pub(super) fn attach_token_index_and_wan_identity(mstore_raw: &Store) {
    // Identity M4a: token → UID for request attribution, on this channel's
    // object store only (spec §6.5.3).
    match crate::boot_timing::time("attach_token_index", || mstore_raw.attach_token_index()) {
        Ok(_) => tracing::info!("identity: agent token index attached"),
        Err(e) => {
            tracing::warn!(error = %e, "identity: agent token index unavailable — requests stay unattributed")
        }
    }
    // W3-S (SPEC_WAN_JEKT_VERIFICATION_2026_09_24.md §2.2): the channel-wide
    // WAN identity store. Opened and its instance minted here, once, so srv
    // signs every WAN jekt as the same instance. Best-effort and never
    // redirected: if the file can't be opened or the instance can't be
    // minted, WAN signing is simply off for this process, and its WAN jekts
    // go unsigned (`TRUST=network-claimed` at the receiver).
    match backend::storage::wan_identity::resolve_wan_identity_path() {
        Some(path) => match crate::boot_timing::time("open_wan_db", || {
            backend::storage::wan_identity::WanIdentityStore::open(&path)
        })
        .and_then(|store| {
            let instance =
                store.instance_ensure(&crate::backend::reactive::registry::local_host_label())?;
            Ok((store, instance))
        }) {
            Ok((store, instance)) => {
                tracing::info!(
                    path = %path.display(),
                    instance = %instance.instance_id,
                    host_hint = %instance.host_hint,
                    "wan identity: store attached"
                );
                let store = Arc::new(store);
                backend::storage::wan_identity::install_global(store.clone());
                mstore_raw.set_wan_identity(store);
            }
            Err(e) => tracing::warn!(
                path = %path.display(),
                error = %e,
                "wan identity: store unavailable — WAN jekt signing is off for this process"
            ),
        },
        None => tracing::warn!(
            "wan identity: channel dir unresolved — WAN jekt signing is off for this process"
        ),
    }
}

/// Opens this channel's file store.
pub(super) fn open_filestore(db_dir: &std::path::Path) -> Arc<FileStore> {
    Arc::new(
        crate::boot_timing::time("open_filestore_db", || {
            FileStore::open(&db_dir.join("filestore.db"))
        })
        .unwrap_or_else(|e| {
            tracing::error!("Failed to open file store: {}", e);
            std::process::exit(1);
        }),
    )
}

/// Opens the global transcript store; `None` falls back to the per-channel store.
pub(super) fn open_global_transcripts() -> Option<Arc<FileStore>> {
    // GLOBAL transcript store — backs the `agent:<defId>:current` zone so a
    // conversation loads when the agent is opened from any build/channel
    // (finishes the cross-channel arc #1387–#1396). A second FileStore over an
    // independent SQLite/WAL file is safe alongside the per-channel one. This is
    // best-effort: if the shared root can't be resolved, or the store can't be
    // opened, we log and fall back to the per-channel `filestore` (global
    // transcripts disabled) — never fatal, unlike the per-channel store above.
    // See `docs/analysis/ANALYSIS_CROSS_CHANNEL_CONVERSATION_HISTORY_2026_06_14.md`.
    match registry::resolve_shared_transcripts_dir() {
        Some(dir) => {
            if let Err(e) = std::fs::create_dir_all(&dir) {
                tracing::warn!(dir = %dir.display(), error = %e, "global transcripts: failed to create dir — disabled, falling back to per-channel store");
                None
            } else {
                let path = dir.join("filestore.db");
                // Give freed space back, if the Storage view asked and no other srv
                // runs (SPEC_HISTORY_STORE_RECLAIM_SPACE_2026_10_23 §4), then hold the
                // gate shared while opening, so an srv starting meanwhile waits. The
                // rewrite takes longer than a supervisor's ESTART wait (about 40 s for
                // 3.6 GB), so it says so first, as a migration does, and the splash
                // shows it as a step.
                {
                    use crate::backend::storage::filestore::{reclaim, repair};
                    reclaim::recover_if_alone(&reclaim::Paths::for_db(&path));
                    // A store the last run found damaged is rebuilt first, from what reads, if no
                    // other srv runs; the damaged file is kept beside it.
                    if repair::needs_start(&path) {
                        eprintln!(
                            "{}",
                            agentmux_common::srv_stderr::migrating_line("repairing-history", 1)
                        );
                        crate::boot_timing::step(
                            "repair-history",
                            "Repairing the history database",
                            || {
                                repair::run_at_start(
                                    &path,
                                    crate::backend::transcript::framer::other_srv,
                                    restore_repaired_history,
                                    agentmux_common::time::now_ms_u64(),
                                )
                            },
                        );
                    }
                    if reclaim::requested(&path) {
                        eprintln!(
                            "{}",
                            agentmux_common::srv_stderr::migrating_line("reclaiming-history", 1)
                        );
                        crate::boot_timing::step(
                            "reclaim-history",
                            "Giving back history disk space",
                            || {
                                reclaim::run_at_start(
                                    &path,
                                    crate::backend::transcript::framer::other_srv,
                                    agentmux_common::time::now_ms_u64(),
                                )
                            },
                        );
                    }
                }
                let gate = crate::backend::storage::filestore::reclaim::gate_shared(&path);
                let opened = crate::boot_timing::time("open_transcripts_db", || {
                    FileStore::open_global_transcripts(&path)
                        .inspect_err(super::shared_guard::note_open_failure)
                });
                drop(gate);
                match opened {
                    Ok(fs) => {
                        tracing::info!(dir = %dir.display(), "global transcripts: store attached");
                        // This srv reads compressed frames (transcript::framer).
                        crate::backend::transcript::framer::mark_reader(&dir);
                        Some(Arc::new(fs))
                    }
                    Err(e) => {
                        tracing::warn!(dir = %dir.display(), error = %e, "global transcripts: failed to open store — disabled, falling back to per-channel store");
                        None
                    }
                }
            }
        }
        None => {
            tracing::warn!("global transcripts: could not resolve shared transcripts dir — disabled, falling back to per-channel store");
            None
        }
    }
}

/// Opens the global shared store; `None` when it cannot be resolved or opened.
pub(super) fn open_shared_store() -> Option<Arc<Store>> {
    // GLOBAL shared store — accounts, bundles, drone
    // definitions, MuxBus credentials. Best-effort: disabled when the shared
    // root can't be resolved. Falls back to mstore so behavior is unchanged
    // from today. See SPEC_GLOBAL_IDENTITY_MEMORY_DRONE_2026_06_24.md.
    match registry::resolve_shared_store_path() {
        Some(path) => {
            // Ensure the parent dir (shared/) exists before opening.
            let parent = path.parent().unwrap_or(path.as_path());
            if let Err(e) = std::fs::create_dir_all(parent) {
                tracing::warn!(path = %path.display(), error = %e, "shared store: failed to create dir — disabled");
                None
            } else {
                match crate::boot_timing::time("open_shared_store", || {
                    Store::open_shared(&path).inspect_err(super::shared_guard::note_open_failure)
                }) {
                    Ok(s) => {
                        let reason = agentmux_common::isolated_auth_reason();
                        if reason.is_isolated() {
                            tracing::info!(path = %path.display(), reason = reason.as_str(), "shared store: attached (ISOLATED — channel-scoped)");
                        } else {
                            tracing::info!(path = %path.display(), reason = reason.as_str(), "shared store: attached");
                        }
                        Some(Arc::new(s))
                    }
                    Err(e) => {
                        tracing::warn!(path = %path.display(), error = %e, "shared store: failed to open — identity/memory/drone/muxbus stay per-channel");
                        None
                    }
                }
            }
        }
        None => {
            tracing::warn!("shared store: could not resolve shared store path — disabled");
            None
        }
    }
}

/// Chooses where identity, memory, drone and muxbus operations go.
pub(super) fn pick_id_store(shared_store: &Option<Arc<Store>>, mstore: &Arc<Store>) -> Arc<Store> {
    // id_store: routes identity/memory/drone/muxbus ops to the shared store when
    // available AND the shared-store backfill migration has been applied.
    // Checking the specific migration (not a coarse migration_ok flag) avoids
    // a split-brain when 0011 succeeded but a later migration failed: the shared
    // store is already backfilled, so using per-channel would strand writes made
    // this session once the later migration succeeds on next boot.
    match shared_store.as_ref() {
        Some(ss) if ss.migration_is_applied("0011_shared_store_backfill") => ss.clone(),
        Some(_) => {
            tracing::warn!(
                "id_store: 0011_shared_store_backfill not yet applied — using per-channel store"
            );
            mstore.clone()
        }
        None => mstore.clone(),
    }
}

/// Carries what earlier per-channel builds kept to themselves into the shared store.
pub(super) fn adopt_earlier_auth(shared_store: &Option<Arc<Store>>, id_store: &Arc<Store>) {
    // Shared auth: carry what earlier per-channel (isolated) builds kept to
    // themselves into the shared store, once and without touching the sources
    // (SPEC_SHARED_AUTH_ACROSS_CHANNELS_2026_10_03.md). Only when this channel
    // really runs on the shared store; an isolated one never reads another's.
    if !agentmux_common::isolated_auth_enabled() {
        if let (Some(ss), Some(path)) =
            (shared_store.as_ref(), registry::resolve_shared_store_path())
        {
            if Arc::ptr_eq(ss, id_store) {
                crate::boot_timing::time("adoption_scan", || {
                    crate::adoption::run_at_boot(ss, &path)
                });
            }
        }
    }
}

/// Opens the permanently-global identity store; falls back to the per-channel store.
pub(super) fn open_identity_store(mstore: &Arc<Store>) -> Arc<Store> {
    // Permanently-global identity store (agent→account links, memory
    // bundles, drone definitions, muxbus/agent M2M credentials, native
    // memory, cron jobs) — see
    // docs/specs/SPEC_IDENTITY_STORE_SPLIT_2026_08_17.md. Deliberately NOT
    // gated by isolated_auth_enabled() at all, unlike shared_store above:
    // none of this store's tables have a legitimate per-channel-isolation
    // use case. Best-effort, same fallback-to-mstore terms as id_store —
    // a resolution/open failure degrades to today's per-channel behavior
    // rather than being fatal.
    match registry::resolve_identity_store_path() {
        Some(path) => {
            let parent = path.parent().unwrap_or(path.as_path());
            if let Err(e) = std::fs::create_dir_all(parent) {
                tracing::warn!(path = %path.display(), error = %e, "identity store: failed to create dir — falling back to per-channel store");
                mstore.clone()
            } else {
                match crate::boot_timing::time("open_identity_store", || {
                    Store::open_identity_store(&path)
                        .inspect_err(super::shared_guard::note_open_failure)
                }) {
                    Ok(s) => {
                        tracing::info!(path = %path.display(), "identity store: attached (always global)");
                        // An agent's LAN signing key is the host's, not this channel's: a channel switch
                        // must not change the key LAN peers have pinned.
                        match backend::storage::host_lan_keys::HostLanKeys::open(&path) {
                            Ok(keys) => mstore.set_host_lan_keys(Arc::new(keys)),
                            Err(e) => {
                                tracing::warn!(error = %e, "LAN keys: the host key table did not open, this channel keeps its own keys")
                            }
                        }
                        Arc::new(s)
                    }
                    Err(e) => {
                        tracing::warn!(path = %path.display(), error = %e, "identity store: failed to open — falling back to per-channel store");
                        mstore.clone()
                    }
                }
            }
        }
        None => {
            tracing::warn!(
                "identity store: could not resolve path — falling back to per-channel store"
            );
            mstore.clone()
        }
    }
}

/// Attaches Global Memory's record and heals the global transcript snapshots; returns the recorder.
pub(super) fn attach_global_memory(
    id_store: &Arc<Store>,
    global_transcript_store: &Option<Arc<FileStore>>,
    backfill_def_ids: &[String],
) -> Option<Recorder> {
    // Install the process-global handle so the block-controller stdout-reader
    // hot path can mirror agent `output` into the global zone without threading
    // the store through `resync_controller` and every controller constructor.
    // Which Global Memory entries follow the fleet (task #12439, section 4.6): the
    // entries that exist now stay on this machine, so create the table before anything new is written.
    if let Err(e) = id_store.bundle_scope_ensure() {
        tracing::warn!(error = %e, "Global Memory scope table: could not create it");
    }
    // Global Memory's own record (SPEC_MEMORY_FOLLOWS_THE_AGENT_2026_09_24.md
    // §2.1.6): write-through from id_store, attached before this start's
    // seeding so seeded entries keep their seeder's attribution.
    let global_memory_recorder = global_transcript_store
        .as_ref()
        .map(|fs| crate::backend::memory::global_record::attach(fs.clone(), id_store.clone()));
    if let Some(ref fs) = global_transcript_store {
        crate::backend::agent_session::set_global_transcript_store(fs.clone());
        // Heal global snapshots poisoned before the normalize-on-mirror fix: a
        // channel-local `sourceBlockId` mirrored into the global zone makes a
        // cross-channel open render empty (the read fallback can't anchor a block
        // that doesn't exist in the opening channel). Idempotent + cheap. See
        // docs/retro/retro-legacy-agent-history-cross-channel-2026-06-16.md.
        let healed = crate::boot_timing::time("transcript_snapshot_heal", || {
            backend::agent_session::heal_global_snapshot_source_block_ids(fs, backfill_def_ids)
        });
        if healed > 0 {
            tracing::info!(
                healed,
                "global transcripts: healed poisoned snapshot sourceBlockIds"
            );
        }
    }
    global_memory_recorder
}

/// Opens the saga log and reads the saga id to seed the allocator from.
pub(super) fn open_saga_log(db_dir: &std::path::Path) -> (Arc<crate::sagas::log::SagaLog>, u64) {
    // Saga durability — see SPEC_SAGA_DURABILITY_2026-05-01.md.
    // Backed by its own SQLite file (`sagas.db`) so saga writes
    // commit independently of the mstore connection. Failure here
    // is fatal: without the log, a srv crash mid-saga leaves
    // unrecoverable state divergence.
    let saga_log = Arc::new(
        crate::boot_timing::time("open_sagas_db", || {
            crate::sagas::log::SagaLog::open(&db_dir.join("sagas.db"))
        })
        .unwrap_or_else(|e| {
            tracing::error!("Failed to open saga log: {}", e);
            std::process::exit(1);
        }),
    );
    // Seed `saga_id_alloc` from the highest persisted saga_id so
    // restarts don't reuse IDs from prior runs (reagent P1 + codex
    // P1 PR #631). With this seed + the plain INSERT (no OR REPLACE)
    // in `start_saga`, ID collisions become impossible by
    // construction.
    let saga_id_seed = crate::boot_timing::time("saga_id_seed_scan", || saga_log.max_saga_id()).unwrap_or_else(|e| {
        tracing::warn!(
            "[saga] failed to read MAX(saga_id) for allocator seed: {} — defaulting to 0; ID collisions on restart possible until next successful query",
            e
        );
        0
    });
    if saga_id_seed > 0 {
        tracing::info!(
            "[saga] seeded saga_id_alloc from durable log: next saga_id = {}",
            saga_id_seed + 1
        );
    }
    (saga_log, saga_id_seed)
}
