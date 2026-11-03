// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Native memory RPCs — read/write the agent's `~/.claude/projects/<sanitized>/memory/`
//! folder that Claude Code uses for autonomous, cross-session fact storage.
//!
//! Three commands:
//!   agent:memory:list       — list *.md files in the memory dir
//!   agent:memory:read_file  — read one file by filename (no path traversal)
//!   agent:memory:write_file — write/create one file atomically (tmp→rename)
//!
//! Spec: docs/specs/SPEC_AGENT_PANE_MEMORY_IDENTITY_MODALS_2026_06_19.md §7
//!
//! All three also write through into `db_agent_native_memory` (via
//! `state.id_store`) — a durable mirror keyed by the stable
//! `AgentDefinition.id`, since the live filesystem path above is
//! channel-relative by design and not the same across channels/instances for
//! the same logical agent. `list`/`read_file` merge the live-FS view with
//! the mirror so a file written from one channel stays visible from another.
//! See docs/specs/SPEC_NATIVE_MEMORY_DURABLE_SYNC_2026_08_07.md.

use std::sync::Arc;

use crate::backend::rpc::engine::WshRpcEngine;
use crate::backend::rpc_types::{
    COMMAND_NATIVE_MEMORY_ADOPTION_LIST,
    COMMAND_NATIVE_MEMORY_CLAIMS,
    COMMAND_NATIVE_MEMORY_DIFF,
    COMMAND_NATIVE_MEMORY_HISTORY,
    COMMAND_NATIVE_MEMORY_LIST,
    COMMAND_NATIVE_MEMORY_READ_FILE,
    COMMAND_NATIVE_MEMORY_REVERT,
    COMMAND_NATIVE_MEMORY_WRITE_FILE,
    CommandNativeMemoryAdoptionListData,
    CommandNativeMemoryClaimsData,
    CommandNativeMemoryDiffData,
    CommandNativeMemoryHistoryData,
    CommandNativeMemoryListData,
    CommandNativeMemoryReadFileData,
    CommandNativeMemoryRevertData,
    CommandNativeMemoryWriteFileData,
    NativeMemoryAdoptionListResult,
    NativeMemoryDiffResult,
    NativeMemoryFileMeta,
    NativeMemoryHistoryResult,
    NativeMemoryListResult,
    NativeMemoryReadFileResult,
    NativeMemoryRevertResult,
};

use super::AppState;
pub(crate) use super::native_memory_diff::line_diff;
use super::native_memory_versions::{version_summary_to_meta, version_to_meta};
use agentmux_common::tokens::{count_lines, estimate_tokens, tokens_for_chars};
pub(crate) use crate::backend::memory::native_files::*;


pub fn register_native_memory_handlers(engine: &Arc<WshRpcEngine>, state: &AppState) {
    super::memory_launch_cost::register(engine, state);
    let mstore_list = state.mstore.clone();
    let id_store_list = state.id_store.clone();
    engine.register_typed(
        COMMAND_NATIVE_MEMORY_LIST,
        move |cmd: CommandNativeMemoryListData, _ctx| {
            let mstore = mstore_list.clone();
            let id_store = id_store_list.clone();
            async move {

                let agent = mstore
                    .agent_def_get(&cmd.agent_id)
                    .map_err(|e| format!("agent:memory:list: store: {e}"))?
                    .ok_or_else(|| format!("agent:memory:list: agent {} not found", cmd.agent_id))?;

                // A blank working_directory is the DEFAULT state, not a
                // reason to short-circuit: `agent.open` substitutes a real
                // directory whenever this field is blank, so a blank row
                // still describes an agent with real memories on disk. The
                // old inline "blank → Ok(files: [])" here was a duplicate,
                // un-synced copy of the exact blank-workdir bug
                // SPEC_FIX_PERSONAL_MEMORY_EMPTY_WORKDIR_2026_09_01.md
                // (#2901) already fixed once in `memory_dir_for_agent_by_id`
                // itself — that fix never propagated to this handler (or to
                // read_file/write_file/revert below), so it silently kept
                // reporting "no memories" for every agent #2901 was
                // supposed to have fixed. See
                // SPEC_MEMORY_RPC_HANDLERS_BLANK_WORKDIR_2026_09_02.md.
                let resolved = resolve_memory_dir_by_id(&mstore, &agent).ok_or_else(|| {
                    format!("agent:memory:list: agent {} has no resolvable memory directory", cmd.agent_id)
                })?;
                let unverified = resolved.provenance == MemoryDirProvenance::Unverified;
                let memory_dir = resolved.path;

                // Existing mirror metadata (no content) for this agent, keyed by
                // filename — lets the loop below skip the expensive full-content
                // read+upsert for a file that hasn't changed since it was last
                // mirrored, instead of doing it unconditionally on every list
                // call (reagent P1 on PR #2459: `list` fires on every Stash
                // Bundle tab open/refresh, so an unconditional full read + SQLite
                // write per file would mean synchronous, potentially many-MB
                // disk I/O on a call meant to be a lightweight metadata listing).
                //
                // Compares BOTH size and mtime, not size alone: a same-byte-length
                // edit is common (e.g. correcting a typo) and size-only comparison
                // would silently leave the mirror stale for it. This matters for
                // exactly the case this table exists for — a channel with no live
                // copy of a file relies entirely on the mirror (reagent P1 on
                // PR #2459's first pass: read_file only "always re-reads fresh"
                // on the channel that still HAS a live copy; a channel that never
                // did has nothing to self-correct with, so a stale mirror row
                // there is permanent, not "briefly stale").
                let mirrored_meta: std::collections::HashMap<String, (i64, i64)> = id_store
                    .agent_native_memory_list_meta(&agent.id)
                    .unwrap_or_default()
                    .into_iter()
                    .map(|row| (row.filename, (row.size_bytes, row.last_seen_mtime_ms)))
                    .collect();

                let mut files: Vec<NativeMemoryFileMeta> = Vec::new();
                let mut live_filenames: std::collections::HashSet<String> = std::collections::HashSet::new();
                // Only a missing directory (never-yet-written, or wiped after this
                // channel last had files) is treated as "no live files" — any
                // other read_dir error (permissions, I/O) propagates, matching the
                // pre-mirror behavior (reagent P2 on PR #2459: silently swallowing
                // every error here would hide a real access problem behind what
                // looks like an empty listing). The mirror merge below still runs
                // regardless, so a wiped live folder doesn't lose durability for
                // files it mirrored previously.
                let entries = match std::fs::read_dir(&memory_dir) {
                    Ok(e) => Some(e),
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
                    Err(e) => return Err(format!("agent:memory:list: read_dir: {e}")),
                };

                if let Some(entries) = entries {
                    for entry in entries {
                        let entry = entry.map_err(|e| format!("agent:memory:list: read_dir entry: {e}"))?;
                        let name = entry.file_name().to_string_lossy().into_owned();
                        if !name.ends_with(".md") {
                            continue;
                        }
                        // Reject symlinks — entry.file_type() does NOT follow symlinks.
                        let file_type = entry
                            .file_type()
                            .map_err(|e| format!("agent:memory:list: file_type {name}: {e}"))?;
                        if !file_type.is_file() {
                            continue;
                        }
                        // The file may be deleted after read_dir but before metadata —
                        // skip the entry on NotFound rather than aborting the whole listing.
                        let meta = match entry.metadata() {
                            Ok(m) => m,
                            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                            Err(e) => return Err(format!("agent:memory:list: metadata {name}: {e}")),
                        };
                        let size_bytes = meta.len();
                        let modified_at = meta
                            .modified()
                            .ok()
                            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                            .map(|d| d.as_millis() as i64)
                            .unwrap_or(0);

                        // Read up to 512 bytes for frontmatter type parsing — same
                        // cheap preview the pre-mirror code used. Take + read_to_end
                        // loops internally to fill the buffer.
                        let preview_content = {
                            use std::io::Read;
                            std::fs::File::open(entry.path())
                                .map(|f| {
                                    let mut buf = Vec::with_capacity(512);
                                    f.take(512).read_to_end(&mut buf).ok();
                                    String::from_utf8_lossy(&buf).into_owned()
                                })
                                .unwrap_or_default()
                        };
                        let metadata_type = parse_frontmatter_type(&preview_content);
                        let is_index = name == "MEMORY.md";

                        let unchanged_since_last_mirror =
                            mirrored_meta.get(&name) == Some(&(size_bytes as i64, modified_at));
                        let mut counts = None;
                        if !unchanged_since_last_mirror {
                            // A read failure here (permission change, TOCTOU
                            // delete, AV/NFS lock, concurrent editor) must NOT
                            // upsert an empty string — that would overwrite any
                            // previously-durable mirrored content, destroying the
                            // exact cross-channel durability guarantee this table
                            // exists for on the very first transient read hiccup
                            // (reagent P0 on PR #2459). Skip the upsert entirely
                            // this round instead; it retries on the next list().
                            let full_content_read = {
                                use std::io::Read;
                                std::fs::File::open(entry.path()).and_then(|f| {
                                    let mut buf = Vec::new();
                                    f.take(MAX_MEMORY_FILE_BYTES).read_to_end(&mut buf)?;
                                    Ok(buf)
                                })
                            };
                            match full_content_read {
                                Ok(buf) => {
                                    let full_content = String::from_utf8_lossy(&buf).into_owned();
                                    counts = Some((estimate_tokens(&full_content), count_lines(&full_content)));
                                    let full_metadata_type = parse_frontmatter_type(&full_content);
                                    if let Err(e) = id_store.agent_native_memory_upsert(
                                        &agent.id,
                                        &name,
                                        &full_content,
                                        full_metadata_type.as_deref(),
                                        &entry.path().to_string_lossy(),
                                        size_bytes as i64,
                                        modified_at,
                                    ) {
                                        tracing::warn!(agent_id = %agent.id, filename = %name, error = %e, "agent:memory:list: mirror upsert failed (non-fatal)");
                                    }
                                }
                                Err(e) => {
                                    tracing::warn!(agent_id = %agent.id, filename = %name, error = %e, "agent:memory:list: full-content read failed, skipping mirror upsert this round (non-fatal)");
                                }
                            }
                        }
                        live_filenames.insert(name.clone());
                        // Not read above: from the mirror; never mirrored: from its bytes.
                        let counts = counts.or_else(|| id_store.agent_native_memory_counts(&agent.id, &name).ok().flatten());
                        let (tokens_est, lines) = counts.unwrap_or((tokens_for_chars(size_bytes), 0));

                        files.push(NativeMemoryFileMeta {
                            filename: name,
                            is_index,
                            metadata_type,
                            size_bytes,
                            tokens_est,
                            lines,
                            modified_at,
                        });
                    }
                }

                // Merge in mirror-only files — present in a different channel's
                // write (or the live folder was wiped) but not on this channel's
                // live FS. Served transparently, with no distinguishing treatment.
                match id_store.agent_native_memory_list_meta(&agent.id) {
                    Ok(mirrored) => {
                        for row in mirrored {
                            if live_filenames.contains(&row.filename) {
                                continue;
                            }
                            let (tokens_est, lines) = id_store
                                .agent_native_memory_counts(&agent.id, &row.filename)
                                .ok()
                                .flatten()
                                .unwrap_or((tokens_for_chars(row.size_bytes as u64), 0));
                            files.push(NativeMemoryFileMeta {
                                is_index: row.filename == "MEMORY.md",
                                metadata_type: row.metadata_type,
                                size_bytes: row.size_bytes as u64,
                                tokens_est,
                                lines,
                                modified_at: row.last_seen_mtime_ms,
                                filename: row.filename,
                            });
                        }
                    }
                    Err(e) => {
                        tracing::warn!(agent_id = %agent.id, error = %e, "agent:memory:list: mirror list failed (non-fatal)");
                    }
                }

                // MEMORY.md first, then alphabetical
                files.sort_by(|a, b| {
                    b.is_index.cmp(&a.is_index).then(a.filename.cmp(&b.filename))
                });

                Ok(NativeMemoryListResult { files, unverified })
            }
        },
    );

    let mstore_read = state.mstore.clone();
    let id_store_read = state.id_store.clone();
    engine.register_typed(
        COMMAND_NATIVE_MEMORY_READ_FILE,
        move |cmd: CommandNativeMemoryReadFileData, _ctx| {
            let mstore = mstore_read.clone();
            let id_store = id_store_read.clone();
            async move {

                validate_filename(&cmd.filename)
                    .map_err(|e| format!("agent:memory:read_file: {e}"))?;

                let agent = mstore
                    .agent_def_get(&cmd.agent_id)
                    .map_err(|e| format!("agent:memory:read_file: store: {e}"))?
                    .ok_or_else(|| format!("agent:memory:read_file: agent {} not found", cmd.agent_id))?;

                // See the identical comment on the list handler above — a
                // blank working_directory is not "no memory dir".
                let path = memory_dir_for_agent_by_id(&mstore, &agent)
                    .ok_or_else(|| {
                        format!("agent:memory:read_file: agent {} has no resolvable memory directory", cmd.agent_id)
                    })?
                    .join(&cmd.filename);

                // Live FS is the freshest copy when present — Claude may have
                // written moments ago, before this call's mirror upsert even
                // runs. Fall back to the mirror only when the live file is
                // genuinely absent (a different channel's write, or the live
                // folder was wiped) — a live path that exists but fails to
                // read for another reason (permissions, non-regular file)
                // still surfaces as an error rather than silently masking it
                // with stale mirrored content.
                // Distinguish "genuinely absent" (fall back to the mirror) from
                // every other symlink_metadata outcome, matching the
                // pre-durable-sync behavior exactly: a real access error
                // (permissions, I/O) must surface as an error, not silently
                // fall back to possibly-stale mirrored content, and an existing
                // non-regular-file path must be explicitly rejected, not treated
                // as "absent" either (reagent P1 on PR #2459, second pass —
                // collapsing every outcome into "absent" the first time around
                // could serve stale content or a misleading "not found" for a
                // path that actually exists but errored or is the wrong type).
                let content = match std::fs::symlink_metadata(&path) {
                    Ok(live_meta) if live_meta.file_type().is_file() => {
                        let mut buf = Vec::new();
                        std::fs::File::open(&path)
                            .and_then(|f| {
                                use std::io::Read;
                                f.take(MAX_MEMORY_FILE_BYTES).read_to_end(&mut buf)
                            })
                            .map_err(|e| format!("agent:memory:read_file: {}: {e}", cmd.filename))?;
                        let content = String::from_utf8_lossy(&buf).into_owned();

                        let metadata_type = parse_frontmatter_type(&content);
                        let mtime_ms = live_meta
                            .modified()
                            .ok()
                            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                            .map(|d| d.as_millis() as i64)
                            .unwrap_or(0);
                        if let Err(e) = id_store.agent_native_memory_upsert(
                            &agent.id,
                            &cmd.filename,
                            &content,
                            metadata_type.as_deref(),
                            &path.to_string_lossy(),
                            live_meta.len() as i64,
                            mtime_ms,
                        ) {
                            tracing::warn!(agent_id = %agent.id, filename = %cmd.filename, error = %e, "agent:memory:read_file: mirror upsert failed (non-fatal)");
                        }
                        content
                    }
                    Ok(_) => {
                        return Err(format!("agent:memory:read_file: {} is not a regular file", cmd.filename));
                    }
                    Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
                        return Err(format!("agent:memory:read_file: {}: {e}", cmd.filename));
                    }
                    Err(_) => match id_store.agent_native_memory_read(&agent.id, &cmd.filename) {
                        Ok(Some(mirrored)) => mirrored,
                        Ok(None) => {
                            return Err(format!("agent:memory:read_file: {}: not found", cmd.filename));
                        }
                        Err(e) => {
                            return Err(format!("agent:memory:read_file: {}: not found on this channel and mirror lookup failed: {e}", cmd.filename));
                        }
                    }
                };

                Ok(NativeMemoryReadFileResult { content })
            }
        },
    );

    let mstore_write = state.mstore.clone();
    let id_store_write = state.id_store.clone();
    let broker_write = state.broker.clone();
    engine.register_typed(
        COMMAND_NATIVE_MEMORY_WRITE_FILE,
        move |cmd: CommandNativeMemoryWriteFileData, _ctx| {
            let mstore = mstore_write.clone();
            let id_store = id_store_write.clone();
            let broker = broker_write.clone();
            async move {

                validate_filename(&cmd.filename)
                    .map_err(|e| format!("agent:memory:write_file: {e}"))?;

                const MAX_CONTENT_BYTES: usize = 10 * 1024 * 1024; // 10 MiB
                if cmd.content.len() > MAX_CONTENT_BYTES {
                    return Err(format!(
                        "agent:memory:write_file: content too large ({} bytes, max {})",
                        cmd.content.len(),
                        MAX_CONTENT_BYTES,
                    ));
                }

                let agent = mstore
                    .agent_def_get(&cmd.agent_id)
                    .map_err(|e| format!("agent:memory:write_file: store: {e}"))?
                    .ok_or_else(|| format!("agent:memory:write_file: agent {} not found", cmd.agent_id))?;

                // See the identical comment on the list handler above — a
                // blank working_directory is not "no memory dir".
                let dir = memory_dir_for_write_by_id(&mstore, &agent)
                    .map_err(|e| format!("agent:memory:write_file: {e}"))?;
                std::fs::create_dir_all(&dir)
                    .map_err(|e| format!("agent:memory:write_file: mkdir: {e}"))?;

                // Conditional save (SPEC_MEMORY_FOLLOWS_THE_AGENT_2026_09_24.md
                // §2.4): a caller that says which content its draft started
                // from gets a refusal, not an overwrite, when that content
                // has since moved. Checked before the version row below so a
                // refused write leaves no trace in history. Not atomic with
                // the rename further down — a write landing in between still
                // wins — but it closes the window from minutes (a human
                // editing) to microseconds, which is the case this exists for.
                if let Some(base) = cmd.base_sha256.as_deref() {
                    let current = current_memory_content(&id_store, &agent.id, &dir.join(&cmd.filename), &cmd.filename)
                        .map_err(|e| format!("agent:memory:write_file: {e}"))?;
                    check_memory_base_sha256(&cmd.filename, base, current.as_deref())
                        .map_err(|e| format!("agent:memory:write_file: {e}"))?;
                }

                // Version history — recorded BEFORE the live-file write below,
                // not after. reagent P1: the drift detector's fast path
                // (native_memory_drift.rs) subscribes to fs-watch events on
                // this same directory; if the version row were inserted AFTER
                // the write (as an earlier revision of this handler did), a
                // fs-watch event for the write below can be processed before
                // this version exists, see a hash that doesn't match anything
                // recorded yet, and log this legitimate RPC write as a
                // spurious "external_fs_write". Recording the version first
                // establishes a real happens-before: the file-modify event
                // that write can possibly generate cannot fire until the
                // write below actually executes, by which point this version
                // already exists to match against. Non-fatal on failure — a
                // durability/review layer on top of the write, not the write
                // itself (mirrors the mirror-upsert failure handling below).
                let (version_source, version_detail) = match &cmd.provenance {
                    Some(p) => (p.source.as_str(), p.detail.to_string()),
                    None => ("agent_inferred", "{}".to_string()),
                };
                if let Err(e) = id_store.agent_native_memory_version_insert(
                    &agent.id,
                    &cmd.filename,
                    &cmd.content,
                    version_source,
                    &version_detail,
                    "",
                ) {
                    tracing::warn!(agent_id = %agent.id, filename = %cmd.filename, error = %e, "agent:memory:write_file: version insert failed (non-fatal)");
                }
                // And into the agent's memory record, likewise first
                // (SPEC_MEMORY_FOLLOWS_THE_AGENT_2026_09_24.md §2.1.1).
                crate::backend::memory::reconcile::record_agentmux_write(
                    &agent.id,
                    &dir,
                    &cmd.filename,
                    cmd.content.as_bytes(),
                    version_source,
                    &version_detail,
                );

                let dest = dir.join(&cmd.filename);
                // Per-write UUID suffix prevents concurrent writes to the same
                // filename from sharing a tmp path and silently corrupting each
                // other's content (reagent P1 on PR #1588).
                let tmp = dir.join(format!(".{}.{}.tmp", cmd.filename, uuid::Uuid::new_v4()));

                // Clean up tmp on both write failure (partial file) and rename failure.
                if let Err(e) = std::fs::write(&tmp, &cmd.content) {
                    let _ = std::fs::remove_file(&tmp);
                    return Err(format!("agent:memory:write_file: write tmp: {e}"));
                }
                if let Err(e) = std::fs::rename(&tmp, &dest) {
                    let _ = std::fs::remove_file(&tmp);
                    return Err(format!("agent:memory:write_file: rename: {e}"));
                }

                let metadata_type = parse_frontmatter_type(&cmd.content);
                // Re-stat the just-written file for its real on-disk size/mtime
                // rather than deriving them from cmd.content here — keeps this
                // in exact agreement with what a subsequent list() will compute,
                // so the size+mtime change check there doesn't spuriously treat
                // this write as "changed again" due to clock/precision drift.
                let dest_meta = std::fs::metadata(&dest).ok();
                let size_bytes = dest_meta.as_ref().map(|m| m.len() as i64).unwrap_or(cmd.content.len() as i64);
                let mtime_ms = dest_meta
                    .and_then(|m| m.modified().ok())
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_millis() as i64)
                    .unwrap_or(0);
                if let Err(e) = id_store.agent_native_memory_upsert(
                    &agent.id,
                    &cmd.filename,
                    &cmd.content,
                    metadata_type.as_deref(),
                    &dest.to_string_lossy(),
                    size_bytes,
                    mtime_ms,
                ) {
                    tracing::warn!(agent_id = %agent.id, filename = %cmd.filename, error = %e, "agent:memory:write_file: mirror upsert failed (non-fatal)");
                }

                // Reactive Memory updates (SPEC_ARMORY_REACTIVE_UPDATES_2026_09_02.md):
                // same event, same `agent:memory:changed:{canonical_uuid}`
                // naming convention `app_api::mod`'s write/revert already use
                // for the MemoryWrite MCP tool's own path — keying by
                // `agent.id` (the canonical UUID this whole file's other
                // handlers already key by, per `resolve_agent_uuid`'s own
                // doc comment on why a slug-keyed event here would silently
                // miss frontend subscribers that only ever have the UUID).
                broker.publish_event_for::<crate::backend::mps_events::AgentMemoryChanged>(&agent.id, &(), vec![], 0);

                tracing::info!(
                    agent_id = %cmd.agent_id,
                    filename = %cmd.filename,
                    bytes = cmd.content.len(),
                    "agent:memory:write_file"
                );
                // `register_typed` always sends a `data` field, so this `()`
                // puts `"data": null` on the wire where the old untyped
                // `Ok(None)` omitted the key entirely. All three frontend
                // callers (the memory models' draft `save` and
                // `createFile`, NativeMemoryFileView.tsx's draft `save`) await
                // and discard the result, and no other crate invokes this
                // command, so the difference is unobservable.
                Ok(())
            }
        },
    );

    // Offers only: adopting a listed folder goes through the host's
    // confirmation window (`service/memory_adopt.rs`), never this RPC — every
    // agent holds the key this one is called with.
    let mstore_adoption = state.mstore.clone();
    engine.register_typed(
        COMMAND_NATIVE_MEMORY_ADOPTION_LIST,
        move |cmd: CommandNativeMemoryAdoptionListData, _ctx| {
            let mstore = mstore_adoption.clone();
            async move {
                let agent = mstore
                    .agent_def_get(&cmd.agent_id)
                    .map_err(|e| format!("agent:memory:adoption_list: store: {e}"))?
                    .ok_or_else(|| format!("agent:memory:adoption_list: agent {} not found", cmd.agent_id))?;
                let Some(fs) = crate::backend::transcript::damage::global_for(&crate::backend::transcript::damage::memory_zone(&agent.id)) else {
                    return Ok(NativeMemoryAdoptionListResult { list: None });
                };
                let list = tokio::task::spawn_blocking(move || crate::backend::memory::adopt::list(&fs, &mstore, &agent))
                    .await
                    .map_err(|e| format!("agent:memory:adoption_list: {e}"))?
                    .map_err(|e| format!("agent:memory:adoption_list: {e}"))?;
                Ok(NativeMemoryAdoptionListResult { list })
            }
        },
    );

    // Offers only, like the adoption list: releasing goes through the host.
    let mstore_claims = state.mstore.clone();
    engine.register_typed(
        COMMAND_NATIVE_MEMORY_CLAIMS,
        move |cmd: CommandNativeMemoryClaimsData, _ctx| {
            let mstore = mstore_claims.clone();
            async move {
                let agent = mstore
                    .agent_def_get(&cmd.agent_id)
                    .map_err(|e| format!("agent:memory:claims: store: {e}"))?
                    .ok_or_else(|| format!("agent:memory:claims: agent {} not found", cmd.agent_id))?;
                let fs = crate::backend::transcript::damage::global_for(&crate::backend::transcript::damage::memory_zone(&agent.id))
                    .ok_or_else(|| "agent:memory:claims: memory record store unavailable, or its record is damaged".to_string())?;
                tokio::task::spawn_blocking(move || crate::backend::memory::release::list(&fs, &agent.id))
                    .await
                    .map_err(|e| format!("agent:memory:claims: {e}"))?
                    .map_err(|e| format!("agent:memory:claims: {e}"))
            }
        },
    );

    let mstore_history = state.mstore.clone();
    let id_store_history = state.id_store.clone();
    engine.register_typed(
        COMMAND_NATIVE_MEMORY_HISTORY,
        move |cmd: CommandNativeMemoryHistoryData, _ctx| {
            let mstore = mstore_history.clone();
            let id_store = id_store_history.clone();
            async move {

                validate_filename(&cmd.filename)
                    .map_err(|e| format!("agent:memory:history: {e}"))?;

                // Resolve to the same canonical agent.id write_file keys
                // by, for the same reason app_api::memory_history_impl
                // does (reagent P1) — cmd.agent_id is expected to already
                // be that id for this surface (the frontend passes
                // AgentDefinition.id), but resolving explicitly here
                // rather than trusting it verbatim matches write_file's
                // own validation and closes the gap if that assumption
                // ever stops holding for some caller.
                let agent = mstore
                    .agent_def_get(&cmd.agent_id)
                    .map_err(|e| format!("agent:memory:history: store: {e}"))?
                    .ok_or_else(|| format!("agent:memory:history: agent {} not found", cmd.agent_id))?;

                let versions = id_store
                    .agent_native_memory_version_list(&agent.id, &cmd.filename)
                    .map_err(|e| format!("agent:memory:history: store: {e}"))?
                    .into_iter()
                    .map(version_summary_to_meta)
                    .collect();

                Ok(NativeMemoryHistoryResult { versions })
            }
        },
    );

    let mstore_diff = state.mstore.clone();
    let id_store_diff = state.id_store.clone();
    engine.register_typed(
        COMMAND_NATIVE_MEMORY_DIFF,
        move |cmd: CommandNativeMemoryDiffData, _ctx| {
            let mstore = mstore_diff.clone();
            let id_store = id_store_diff.clone();
            async move {

                // Resolve to the same canonical agent.id write_file keys by
                // — see the identical comment on the history handler above.
                let agent = mstore
                    .agent_def_get(&cmd.agent_id)
                    .map_err(|e| format!("agent:memory:diff: store: {e}"))?
                    .ok_or_else(|| format!("agent:memory:diff: agent {} not found", cmd.agent_id))?;

                let from = id_store
                    .agent_native_memory_version_get(&cmd.from_version_id)
                    .map_err(|e| format!("agent:memory:diff: store: {e}"))?
                    .ok_or_else(|| format!("agent:memory:diff: version {} not found", cmd.from_version_id))?;
                let to = id_store
                    .agent_native_memory_version_get(&cmd.to_version_id)
                    .map_err(|e| format!("agent:memory:diff: store: {e}"))?
                    .ok_or_else(|| format!("agent:memory:diff: version {} not found", cmd.to_version_id))?;
                // reagent P1: unlike list/read/write/history/revert, this
                // handler has no other agent-scoping — every caller shares
                // one instance-wide X-AuthKey, so without this check any
                // caller could read any other agent's memory content by
                // version id.
                if from.agent_id != agent.id || to.agent_id != agent.id {
                    return Err(format!(
                        "agent:memory:diff: one or both versions do not belong to {}",
                        cmd.agent_id
                    ));
                }
                // reagent P2: ownership alone isn't enough — from/to must
                // also be versions of the SAME file, or the "diff" is a
                // meaningless line-by-line comparison of two unrelated
                // files with no error to signal that.
                if from.filename != to.filename {
                    return Err(format!(
                        "agent:memory:diff: from_version_id and to_version_id are versions of different files ({} vs {})",
                        from.filename, to.filename
                    ));
                }

                let diff = line_diff(&from.content, &to.content);
                Ok(NativeMemoryDiffResult { diff })
            }
        },
    );

    let mstore_revert = state.mstore.clone();
    let id_store_revert = state.id_store.clone();
    let broker_revert = state.broker.clone();
    engine.register_typed(
        COMMAND_NATIVE_MEMORY_REVERT,
        move |cmd: CommandNativeMemoryRevertData, _ctx| {
            let mstore = mstore_revert.clone();
            let id_store = id_store_revert.clone();
            let broker = broker_revert.clone();
            async move {

                validate_filename(&cmd.filename)
                    .map_err(|e| format!("agent:memory:revert: {e}"))?;

                // Resolve to the same canonical agent.id write_file keys by
                // — see the identical comment on the history handler above.
                // Resolved BEFORE the ownership check below (not after, as
                // an earlier revision of this handler did) so that check
                // compares against the same id the version was actually
                // stored under, not the raw, possibly-different cmd.agent_id.
                let agent = mstore
                    .agent_def_get(&cmd.agent_id)
                    .map_err(|e| format!("agent:memory:revert: store: {e}"))?
                    .ok_or_else(|| format!("agent:memory:revert: agent {} not found", cmd.agent_id))?;

                let target = id_store
                    .agent_native_memory_version_get(&cmd.target_version_id)
                    .map_err(|e| format!("agent:memory:revert: store: {e}"))?
                    .ok_or_else(|| format!("agent:memory:revert: version {} not found", cmd.target_version_id))?;
                if target.agent_id != agent.id || target.filename != cmd.filename {
                    return Err(format!(
                        "agent:memory:revert: version {} does not belong to {}/{}",
                        cmd.target_version_id, cmd.agent_id, cmd.filename
                    ));
                }

                // Revert is implemented as a NEW write through the same
                // path as agent:memory:write_file (live file + mirror +
                // version), not a rewrite of history — this is the
                // git-revert-not-git-reset guarantee from the spec's §4.3.
                // See the identical comment on the list handler above — a
                // blank working_directory is not "no memory dir".
                let dir = memory_dir_for_write_by_id(&mstore, &agent).map_err(|e| {
                    format!("agent:memory:revert: {e}")
                })?;
                std::fs::create_dir_all(&dir)
                    .map_err(|e| format!("agent:memory:revert: mkdir: {e}"))?;

                // Version recorded BEFORE the live-file write below — same
                // fs-watch-race rationale as agent:memory:write_file's own
                // handler above (reagent P1). Unlike that handler, a failure
                // here IS fatal: the RPC's whole contract is "return the new
                // version," so silently reverting the file but not returning
                // a version would leave the caller with no way to know what
                // they just reverted to.
                let detail = serde_json::json!({ "reverted_to": cmd.target_version_id }).to_string();
                let new_version = id_store
                    .agent_native_memory_version_insert(&agent.id, &cmd.filename, &target.content, "revert", &detail, "")
                    .map_err(|e| format!("agent:memory:revert: version insert: {e}"))?;
                crate::backend::memory::reconcile::record_agentmux_write(
                    &agent.id,
                    &dir,
                    &cmd.filename,
                    target.content.as_bytes(),
                    "revert",
                    &detail,
                );

                let dest = dir.join(&cmd.filename);
                let tmp = dir.join(format!(".{}.{}.tmp", cmd.filename, uuid::Uuid::new_v4()));
                if let Err(e) = std::fs::write(&tmp, &target.content) {
                    let _ = std::fs::remove_file(&tmp);
                    return Err(format!("agent:memory:revert: write tmp: {e}"));
                }
                if let Err(e) = std::fs::rename(&tmp, &dest) {
                    let _ = std::fs::remove_file(&tmp);
                    return Err(format!("agent:memory:revert: rename: {e}"));
                }

                let metadata_type = parse_frontmatter_type(&target.content);
                let dest_meta = std::fs::metadata(&dest).ok();
                let size_bytes = dest_meta.as_ref().map(|m| m.len() as i64).unwrap_or(target.content.len() as i64);
                let mtime_ms = dest_meta
                    .and_then(|m| m.modified().ok())
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_millis() as i64)
                    .unwrap_or(0);
                if let Err(e) = id_store.agent_native_memory_upsert(
                    &agent.id,
                    &cmd.filename,
                    &target.content,
                    metadata_type.as_deref(),
                    &dest.to_string_lossy(),
                    size_bytes,
                    mtime_ms,
                ) {
                    tracing::warn!(agent_id = %agent.id, filename = %cmd.filename, error = %e, "agent:memory:revert: mirror upsert failed (non-fatal)");
                }

                // See the identical comment on write_file's own handler above.
                broker.publish_event_for::<crate::backend::mps_events::AgentMemoryChanged>(&agent.id, &(), vec![], 0);

                tracing::info!(
                    agent_id = %cmd.agent_id,
                    filename = %cmd.filename,
                    target_version_id = %cmd.target_version_id,
                    "agent:memory:revert"
                );
                Ok(NativeMemoryRevertResult { version: version_to_meta(&new_version) })
            }
        },
    );
}

// Request-shape tests for the six `agent:memory:*` commands.
//
// These exist because NOTHING else catches a Req/payload mismatch: `tsc` only
// checks the frontend against the GENERATED types, and
// `scripts/check-rpc-bindings.sh` only checks that a generated type exists for
// each command — neither one ever deserializes a real payload into the Rust
// struct. A `Req` that cannot parse what the stub sends compiles, typechecks,
// passes the binding gate, and then fails on every call at runtime. That is
// exactly the bug found on `bookmarks.list` (`Req = ()` rejects `{}`), so each
// command below is pinned to the literal JSON its call site actually sends.
#[cfg(test)]
#[path = "tests/native_memory_handlers/req_shape_tests.rs"]
mod req_shape_tests;

#[cfg(test)]
#[path = "tests/native_memory_handlers/tests.rs"]
mod tests;
