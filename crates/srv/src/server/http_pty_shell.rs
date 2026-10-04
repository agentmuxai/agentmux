// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! PTY shell HTTP handlers.
//! Split out of server/mod.rs unchanged (SPEC_LARGE_FILE_MODULE_ANALYSIS_2026_09_30.md §4.1).

use super::*;

/// `POST /api/v1/ptyshell/create` — attach to (creating if needed) "this
/// pane's one shell."
///
/// Reuses the same sub-block a human's composer-drawer shell already uses
/// or will use (`META_KEY_SHELL_SUBBLOCK_ID`), in either direction: if the
/// human already opened the drawer, the agent joins that exact PTY — same
/// scrollback, same live output. If nothing exists yet, this creates it
/// (the same `view: "term", controller: "shell"` headless-sub-block shape
/// `createsubblock`'s WS handler builds for `AgentShellSubblock.tsx`) and
/// persists the pointer, so a drawer opened *afterward* attaches to what
/// the agent already started instead of spawning a second, independent
/// shell. Either way, `resync_controller` runs before returning — unlike
/// the WS path (which waits for a frontend to mount a `term` view and call
/// `ControllerResyncCommand`), the PTY is live the moment this call
/// returns; a UI viewer for it is optional, not a prerequisite.
pub(super) async fn handle_pty_shell_create(
    State(state): State<AppState>,
    Json(req): Json<PtyShellCreateRequest>,
) -> impl IntoResponse {
    // Where the shell runs, checked the way a pane's connection is: a refused
    // one fails the call with its reason rather than opening a local shell.
    use super::app_api::connections::{self, AgentTarget};
    let target = match connections::for_agent(&state.broker, req.connection.as_deref()).await {
        Ok(t) => t,
        Err(e) => {
            return (StatusCode::BAD_REQUEST, Json(json!({ "error": e }))).into_response();
        }
    };
    let connection = match &target {
        AgentTarget::Local => "local".to_string(),
        AgentTarget::Wsl(d) => crate::backend::remote::ConnTarget::Wsl(d.clone()).name(),
        AgentTarget::Ssh(dest) => crate::backend::remote::ConnTarget::Ssh(dest.clone()).name(),
    };
    // An SSH host runs with the user's identity: their consent first, and the
    // askpass helper must exist, since its prompts may never reach the
    // terminal the agent reads (`remote::askpass`).
    let mut ssh_agent = String::new();
    if let AgentTarget::Ssh(_) = &target {
        if crate::backend::remote::askpass::program().is_none() {
            let e = "this AgentMux has no askpass helper (agentmux-bashwrap) to run an agent's SSH shell with";
            return (StatusCode::BAD_REQUEST, Json(json!({ "error": e }))).into_response();
        }
        ssh_agent = match connections::verified_agent(&state, &req.agent_block_id, req.auth.as_ref()) {
            Ok(a) => a,
            Err(e) => return (StatusCode::FORBIDDEN, Json(json!({ "error": e }))).into_response(),
        };
        // A running shell on another connection is refused before anything
        // is asked: consent (and an "always") for a shell that then cannot
        // start would be a grant the user gave for nothing.
        if let Some(conflict) = running_shell_conflict(&state, &req.agent_block_id, &connection) {
            return conflict;
        }
        if let Err(e) = connections::consent_for_ssh(&state, &req.agent_block_id, &ssh_agent, &connection, "an interactive shell (PtyShell)").await {
            return (StatusCode::FORBIDDEN, Json(json!({ "error": e }))).into_response();
        }
    }
    let remote = !matches!(target, AgentTarget::Local);
    let existing_id = state
        .mstore
        .get::<crate::backend::obj::Block>(&req.agent_block_id)
        .ok()
        .flatten()
        .and_then(|b| {
            b.meta
                .get(META_KEY_SHELL_SUBBLOCK_ID)
                .and_then(|v| v.as_str())
                .map(str::to_string)
        });

    // Reuse path: the pointer exists and still resolves to a real block.
    // Deliberately ignores req.cwd/rows/cols here — an already-running
    // shell's cwd can't change after spawn, and geometry is a separate,
    // legitimate post-hoc operation via PtyShellResize, not a create-time
    // parameter for a process that already has a size.
    if let Some(id) = existing_id {
        if let Some(resp) = try_attach_to_existing_shell(&state, &req.agent_block_id, &id, &connection).await {
            return resp;
        }
        // `None` here means either a stale pointer (the block was deleted
        // some other way — rare) or, as of this PR, the pane's shell
        // having already exited (the common case this PR fixes — see
        // `try_attach_to_existing_shell`'s own doc comment). Either way,
        // fall through to creating a fresh one, same as if none existed.
    }

    let child_id = uuid::Uuid::new_v4().to_string();

    let mut meta = crate::backend::obj::MetaMapType::new();
    // Which srv run created this shell: a block with no controller is one this
    // run is still starting (a concurrent create), not one left from before a
    // restart (`try_attach_to_existing_shell`).
    meta.insert(META_KEY_PTYSHELL_BOOT_ID.to_string(), json!(&*state.boot_id));
    // The agent's plain-`gh` guard, as its Shell and its own CLI get it: the
    // agent types into this shell, so `gh` here must not act as the user.
    let agent_env: std::collections::HashMap<String, String> = state
        .mstore
        .get::<crate::backend::obj::Block>(&req.agent_block_id)
        .ok()
        .flatten()
        .map(|b| crate::backend::blockcontroller::cmd_env_of(&b.meta))
        .unwrap_or_default();
    meta.insert(
        crate::backend::gh_guard::META_KEY_PTYSHELL_GH_CONFIG_DIR.to_string(),
        json!(crate::backend::gh_guard::guard_dir_for(&agent_env)),
    );
    // And the publish guard's git hooks, so a push typed here is scanned too.
    if let Some(hooks) = crate::backend::publish_guard::host_hooks_dir() {
        meta.insert(
            crate::backend::publish_guard::META_KEY_PTYSHELL_GIT_HOOKS_PATH.to_string(),
            json!(hooks),
        );
    }
    meta.insert("view".to_string(), json!("term"));
    meta.insert(
        blockcontroller::META_KEY_CONTROLLER.to_string(),
        json!(blockcontroller::BLOCK_CONTROLLER_SHELL),
    );
    // Force cmd.exe directly on Windows rather than the "shell" controller's
    // default pwsh/powershell auto-detection (`detect_local_shell_path_windows`,
    // which tries pwsh first). Discovered live, not assumed: PowerShell's
    // PSReadLine issues a cursor-position query (`ESC[6n`, Device Status
    // Report) on interactive startup and blocks forever waiting for a
    // `ESC[row;colR` reply — which only a real terminal emulator answering
    // on the other end provides (xterm.js does, for the `term` widget /
    // AgentShellSubblock's human-facing sessions). This facility has no
    // terminal emulator reading the byte stream, only a raw capture, so
    // every PtyShell() would otherwise hang before printing even its first
    // prompt. cmd.exe has no such startup handshake. Non-Windows shells
    // (bash/zsh via $SHELL) aren't affected — left as the "shell"
    // controller's own default. This is a known-narrow mitigation, not a
    // general fix: any OTHER program that queries terminal capabilities via
    // escape sequences and blocks for a reply would hit the same class of
    // issue — see docs/specs/SPEC_AGENT_INTERACTIVE_PTY_SHELL_API_2026_09_10.md §5.
    //
    // A WSL shell is the distro's own login shell (`wsl.exe -d`), which has no
    // such handshake, and `cmd.exe` would be looked up inside the distro.
    #[cfg(windows)]
    if !remote {
        meta.insert(blockcontroller::META_KEY_CMD.to_string(), json!("cmd.exe"));
        meta.insert("cmd:interactive".to_string(), json!(true));
    }
    if remote {
        meta.insert(blockcontroller::META_KEY_CONNECTION.to_string(), json!(connection));
    }
    // If the caller didn't supply a cwd, fall back to the agent block's own
    // cmd:cwd — the project directory the agent pane was launched with.
    // Mirrors `handle_shell_create`'s identical fallback (Codex P1 on PR
    // #3177): without this, the PTY spawns in agentmux-srv's own cwd
    // (typically the portable runtime/ dir), not the agent's worktree.
    //
    // In WSL the cwd is a distro path, kept as given; no Windows fallback.
    if let AgentTarget::Ssh(_) = &target {
        let agent = ssh_agent.clone();
        meta.insert(crate::backend::remote::askpass::META_KEY_AGENT_BLOCK.to_string(), json!(req.agent_block_id));
        meta.insert(crate::backend::remote::askpass::META_KEY_AGENT.to_string(), json!(agent));
    }
    if remote {
        if let Some(cwd) = req.cwd.as_deref().filter(|c| !c.trim().is_empty()) {
            meta.insert(blockcontroller::META_KEY_CMD_CWD.to_string(), json!(cwd));
        }
    }
    let effective_cwd = req.cwd.clone().or_else(|| {
        state
            .mstore
            .get::<crate::backend::obj::Block>(&req.agent_block_id)
            .ok()
            .flatten()
            .and_then(|b| {
                let cwd = crate::backend::obj::meta_get_string(&b.meta, "cmd:cwd", "");
                if cwd.is_empty() { None } else { Some(cwd) }
            })
    });
    let effective_cwd = effective_cwd.filter(|_| !remote);
    if let Some(cwd) = &effective_cwd {
        // Same MSYS→native normalization createsubblock's WS handler applies —
        // the PTY spawn path reads cmd:cwd raw with no conversion, so a
        // Git-Bash-style path would otherwise fail with os error 267 on
        // Windows.
        match crate::backend::base::normalize_working_dir(cwd)
            .filter(|p| std::path::Path::new(p).is_absolute())
        {
            Some(norm) => {
                meta.insert(blockcontroller::META_KEY_CMD_CWD.to_string(), json!(norm));
            }
            None => {
                tracing::warn!(
                    raw_cwd = %cwd,
                    "ptyshell.create: invalid or non-absolute cwd, dropping (no cwd)"
                );
            }
        }
    }

    // Initial PTY geometry, if the caller specified one — otherwise
    // `resync_controller`'s own fallback (25x200) applies. Both `rows`/`cols`
    // being absent is the common case; either one alone still takes effect
    // (see `pty_size_from_rt_opts`'s per-field guards).
    let runtimeopts = if req.rows.is_none() && req.cols.is_none() {
        None
    } else {
        Some(crate::backend::obj::RuntimeOpts {
            termsize: crate::backend::obj::TermSize {
                rows: req.rows.unwrap_or(25) as i64,
                cols: req.cols.unwrap_or(200) as i64,
            },
            ..Default::default()
        })
    };
    let rt_opts_json = runtimeopts.as_ref().map(|rt| {
        json!({ "termsize": { "rows": rt.termsize.rows, "cols": rt.termsize.cols } })
    });

    let mut block = crate::backend::obj::Block {
        oid: child_id.clone(),
        parentoref: format!("block:{}", req.agent_block_id),
        meta,
        runtimeopts,
        ..Default::default()
    };

    // Atomic claim-or-lose (Codex P2 on PR #3194): without this, two
    // concurrent `PtyShell` calls that both observe an unset
    // `term:shellsubblockid` (the check at the top of this function is
    // NOT authoritative — it's a plain read, before this point) would each
    // insert their own block and race to overwrite the other's pointer
    // write, leaving one controller registered but orphaned from the
    // pointer that's supposed to track it, and the two callers ending up
    // attached to DIFFERENT shells despite both believing they're on "the
    // pane's one shell." `with_tx` holds the store's single connection
    // lock for its whole closure, so re-checking the pointer INSIDE it,
    // atomically with the insert+link+set that follows, serializes any
    // concurrent `PtyShell` call against this same agent_block_id.
    //
    // Does NOT close the equivalent race against the FRONTEND's own
    // `createsubblock` path (`AgentShellSubblock.tsx`'s "no
    // existingSubBlockId yet" branch): that's a separate, non-transactional
    // multi-step RPC sequence (create, then a follow-up `SetMetaCommand`)
    // this backend call has no visibility into or ability to serialize
    // against. A human opening the drawer for the very first time on a
    // pane in the same narrow window as an agent's first `PtyShell` call
    // on that pane can still each end up with their own shell — a real,
    // documented, accepted gap (see the spec's §10 for the full
    // reasoning), not silently ignored, just out of scope for what a
    // single backend transaction can enforce unilaterally.
    let claim = state.mstore.with_tx(|tx| {
        let mut parent = tx.must_get::<crate::backend::obj::Block>(&req.agent_block_id)?;
        if let Some(winner_id) = parent
            .meta
            .get(META_KEY_SHELL_SUBBLOCK_ID)
            .and_then(|v| v.as_str())
        {
            return Ok(Some(winner_id.to_string()));
        }
        tx.insert(&mut block)?;
        parent.subblockids.get_or_insert_with(Vec::new).push(child_id.clone());
        parent
            .meta
            .insert(META_KEY_SHELL_SUBBLOCK_ID.to_string(), json!(child_id));
        tx.update(&mut parent)?;
        Ok(None)
    });

    let (shell_id, block) = match claim {
        // We lost the race — someone else's transaction committed the
        // pointer first. Pivot to reusing THEIRS rather than proceeding
        // with the block we prepared but never actually inserted.
        Ok(Some(winner_id)) => {
            if let Some(resp) = try_attach_to_existing_shell(&state, &req.agent_block_id, &winner_id, &connection).await {
                return resp;
            }
            // Winner's block vanished between the transaction and now, OR
            // (as of this PR) resolved to a shell that had already exited.
            // Both stay exceedingly unlikely to land HERE specifically:
            // `try_attach_to_existing_shell` now clears the parent's
            // pointer itself the moment it finds an exited shell (see its
            // own comment on the `RESYNC_ERR_ALREADY_EXITED` branch), so
            // the common "human typed `exit`, then something calls
            // PtyShellCreate again" case is intercepted one level up — the
            // CLAIM transaction above reads the pointer as already-cleared
            // and takes the plain `Ok(None)` fresh-insert path, never
            // reaching this branch at all. Getting here for an exited
            // shell specifically requires a genuine concurrent-call race
            // (two callers both observe the stale pointer before either
            // one's clear commits) — same rarity class as the pre-existing
            // deleted-out-from-under-us case this comment originally
            // described, not the common case. `block` above was only ever
            // prepared in memory: the
            // `with_tx` closure's `tx.insert` never ran for it (that only
            // happens in the `Ok(None)` arm, when WE win the claim), so it
            // has no backing row. Falling through with it as-is would send
            // `resync_controller` a phantom block and return a `shell_id`
            // with nothing in the store behind it — every later
            // PtyShellInput/Read/Status/Stop call against it would then
            // fail ownership/lookup checks (ReAgent P2 on PR #3194).
            //
            // Actually create it for real instead, mirroring what the
            // `Ok(None)` arm does inside its transaction — just as a plain
            // sequence, since we're no longer inside that transaction.
            let insert_result = (|| -> Result<(), crate::backend::storage::error::StoreError> {
                state.mstore.insert(&mut block)?;
                let mut parent = state
                    .mstore
                    .must_get::<crate::backend::obj::Block>(&req.agent_block_id)?;
                parent
                    .subblockids
                    .get_or_insert_with(Vec::new)
                    .push(child_id.clone());
                parent
                    .meta
                    .insert(META_KEY_SHELL_SUBBLOCK_ID.to_string(), json!(child_id));
                state.mstore.update(&mut parent)?;
                Ok(())
            })();
            if let Err(e) = insert_result {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({ "error": format!("ptyshell.create: fallback insert: {e}") })),
                )
                    .into_response();
            }
            if let Ok(Some(parent)) = state
                .mstore
                .get::<crate::backend::obj::Block>(&req.agent_block_id)
            {
                state.event_bus.broadcast_event(&crate::backend::eventbus::WSEventType {
                    eventtype: "waveobj:update".to_string(),
                    oref: format!("block:{}", req.agent_block_id),
                    data: Some(serde_json::to_value(&crate::backend::obj::MuxObjUpdate {
                        updatetype: "update".into(),
                        otype: "block".into(),
                        oid: req.agent_block_id.clone(),
                        obj: Some(crate::backend::obj::mux_obj_to_value(&parent)),
                    }).unwrap_or_default()),
                });
            }
            (child_id.clone(), block)
        }
        Ok(None) => {
            // We won the claim and the parent's meta changed (inside the
            // transaction, via a raw `tx.update`, which — unlike
            // `broadcast_meta_update` — does NOT itself notify any
            // subscribed frontend). Broadcast now so a drawer already open
            // (or opened moments later) actually sees the pointer reactively
            // instead of only on its next unrelated refetch.
            if let Ok(Some(parent)) = state.mstore.get::<crate::backend::obj::Block>(&req.agent_block_id) {
                state.event_bus.broadcast_event(&crate::backend::eventbus::WSEventType {
                    eventtype: "waveobj:update".to_string(),
                    oref: format!("block:{}", req.agent_block_id),
                    data: Some(serde_json::to_value(&crate::backend::obj::MuxObjUpdate {
                        updatetype: "update".into(),
                        otype: "block".into(),
                        oid: req.agent_block_id.clone(),
                        obj: Some(crate::backend::obj::mux_obj_to_value(&parent)),
                    }).unwrap_or_default()),
                });
            }
            (child_id.clone(), block)
        }
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": format!("ptyshell.create: claim: {e}") })),
            )
                .into_response();
        }
    };
    let child_id = shell_id;

    let registry = state.mstore.shared_agent_registry();
    if let Err(e) = blockcontroller::resync_controller(
        &block,
        "ptyshell", // headless — no real tab; only used for tracing/scoping.
        rt_opts_json,
        false,
        true, // respawn_if_done — irrelevant here, this block is always freshly inserted (STATUS_INIT), never STATUS_DONE
        Some(Arc::clone(&state.broker)),
        Some(Arc::clone(&state.event_bus)),
        Some(Arc::clone(&state.mstore)),
        Some(Arc::clone(&state.filestore)),
        Some(Arc::clone(&state.id_store)),
        Some(Arc::clone(&state.identity_store)),
        registry,
        state.boot_id.clone(),
        &state.auth_key,
        Some(Arc::clone(&state.config_watcher)),
    ) {
        // Roll back — the block was already inserted and linked into the
        // parent's subblockids above (Codex P2 on PR #3177): without this,
        // a failed spawn leaves a stale block + parent link + (possibly)
        // registered controller behind that the caller has no way to clean
        // up, since it never received a `shell_id` to call PtyShellStop with.
        crate::sagas::agent_teardown::discard(&child_id);
        let _ = state.mstore.delete::<crate::backend::obj::Block>(&child_id);
        if let Ok(mut parent) = state
            .mstore
            .must_get::<crate::backend::obj::Block>(&req.agent_block_id)
        {
            let mut changed = false;
            if let Some(ids) = parent.subblockids.as_mut() {
                let before = ids.len();
                ids.retain(|id| id != &child_id);
                changed |= ids.len() != before;
            }
            // The pointer was already persisted before `resync_controller`
            // ran — either inside the atomic claim's own transaction, or by
            // the "winner vanished" fallback above — so unlinking
            // `subblockids` alone isn't enough: without also clearing it
            // here, `term:shellsubblockid` is left pointing at a block that
            // was just deleted, and a human opening the composer drawer in
            // this exact window (`agent-view.tsx`'s
            // `existingSubBlockId={block()?.meta?.["term:shellsubblockid"]}`)
            // attaches to nothing instead of getting a working shell
            // (ReAgent P1, round 3 on PR #3194).
            if parent.meta.get(META_KEY_SHELL_SUBBLOCK_ID).and_then(|v| v.as_str()) == Some(child_id.as_str())
            {
                parent.meta.remove(META_KEY_SHELL_SUBBLOCK_ID);
                changed = true;
            }
            if changed {
                let _ = state.mstore.update(&mut parent);
                state.event_bus.broadcast_event(&crate::backend::eventbus::WSEventType {
                    eventtype: "waveobj:update".to_string(),
                    oref: format!("block:{}", req.agent_block_id),
                    data: Some(serde_json::to_value(&crate::backend::obj::MuxObjUpdate {
                        updatetype: "update".into(),
                        otype: "block".into(),
                        oid: req.agent_block_id.clone(),
                        obj: Some(crate::backend::obj::mux_obj_to_value(&parent)),
                    }).unwrap_or_default()),
                });
            }
        }
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": format!("ptyshell.create: spawn: {e}") })),
        )
            .into_response();
    }

    // baseline_len=0: a genuinely new block has no prior "term" file content
    // to accidentally match against (see the reuse path's identical
    // baseline logic above for why this matters at all).
    answer_conpty_handshake_if_seen(&state, &child_id, 0).await;

    // NOTE: the shellsubblockid pointer is already persisted — the atomic
    // claim transaction above sets it on the parent in the same
    // transaction as the insert, which is also strictly more correct than
    // a separate post-hoc write here would be (no window where the block
    // exists but isn't yet pointed to).
    tracing::info!(block_id = %child_id, parent_id = %req.agent_block_id, "ptyshell.create: fresh");
    (StatusCode::OK, Json(PtyShellCreateResponse { shell_id: child_id })).into_response()
}

/// Answer Windows ConPTY's own startup handshake, if seen, before the
/// caller proceeds. Discovered live (not assumed): the moment a Windows
/// ConPTY is created, it queries the cursor position (`ESC[6n`, Device
/// Status Report) and blocks its own internal state — and therefore every
/// later write — until it sees a `ESC[<row>;<col>R` reply. A human-facing
/// `term` pane gets this for free because xterm.js, running in the
/// browser, answers real cursor queries as part of being an actual
/// terminal emulator; this headless facility has nothing playing that
/// role, so without this the PTY would sit permanently stuck before
/// printing even its first prompt — confirmed by a live round trip that
/// hung indefinitely until this was added. `ESC[1;1R` (cursor at row 1,
/// col 1) is a plausible-enough fixed answer: nothing here has rendered
/// anything yet, so "top-left" is as true as any other coordinate, and
/// ConPTY only wants to reconcile its internal buffer, not validate the
/// value against anything a person would see.
///
/// `baseline_len` MUST be the "term" file's byte length taken immediately
/// before `resync_controller` ran (0 for a genuinely new block) — see the
/// call sites. Only bytes appended AFTER that offset are ever inspected.
/// **Not** safe to scan the whole file regardless of reuse/fresh (an
/// earlier version of this function did exactly that): the "term" file is
/// append-only for the block's entire lifetime
/// (`handle_append_block_file`'s `FILE_OP_APPEND`), so a long-running
/// reused shell's history almost always still contains its own original,
/// long-since-answered `ESC[6n` from whenever it first started — matching
/// against that stale byte sequence would inject an unsolicited
/// `ESC[1;1R` into whatever a human is doing RIGHT NOW on that shell
/// (Codex P1 on PR #3194, confirmed live before this fix: `create` on an
/// already-running reused shell always found the old query and misfired).
/// The baseline makes every case correct uniformly: nothing new appears
/// (the common reuse case) → correctly finds nothing; `resync_controller`
/// had to respawn a dead process → correctly sees only ITS fresh query,
/// never anything from the block's former life.
#[cfg_attr(not(windows), allow(unused_variables))]
pub(super) async fn answer_conpty_handshake_if_seen(state: &AppState, block_id: &str, baseline_len: usize) {
    #[cfg(windows)]
    {
        let mut answered = false;
        for _ in 0..20 {
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            let saw_dsr = state
                .filestore
                .read_file(block_id, "term")
                .ok()
                .flatten()
                .map(|bytes| {
                    let new_bytes = bytes.get(baseline_len..).unwrap_or(&[]);
                    new_bytes.windows(4).any(|w| w == b"\x1b[6n")
                })
                .unwrap_or(false);
            if saw_dsr {
                let _ = blockcontroller::send_input(
                    block_id,
                    blockcontroller::BlockInputUnion::data(b"\x1b[1;1R".to_vec()),
                    None,
                );
                answered = true;
                break;
            }
        }
        tracing::info!(block_id = %block_id, answered_dsr = answered, "ptyshell: ConPTY handshake check");
    }
}

/// `POST /api/v1/ptyshell/input` — write raw text to the PTY, as if typed.
/// Unlike `/api/v1/shell/input`, no newline is appended: a real terminal
/// doesn't add one either, and PTY programs (readline, Sharprompt) expect
/// the caller to send exactly what a keyboard would (e.g. `"y\n"` to answer
/// a prompt and press Enter, or `"\x03"` for Ctrl+C — the OS PTY layer
/// translates that raw byte into a real interrupt delivered to the
/// foreground process, same as a human pressing Ctrl+C in a real terminal,
/// without ending the shell).
///
/// There is deliberately no separate "send a signal" endpoint: an earlier
/// version of this API had one (`PtyShellSignal`, `BlockInputUnion::signal`),
/// removed after Codex correctly flagged (PR #3177) that
/// `ShellController`'s input loop treats ANY signal as "close the PTY" —
/// `if input.sig_name.is_some() { break; }` in
/// `blockcontroller/shell/lifecycle.rs` — so it terminated the whole shell
/// instead of interrupting just the foreground command, the opposite of
/// what it advertised. Real signal delivery through a PTY is the raw
/// control byte, handled here, not a separate primitive.
pub(super) async fn handle_pty_shell_input(
    State(state): State<AppState>,
    Json(req): Json<PtyShellInputRequest>,
) -> impl IntoResponse {
    if !is_owned_by_agent(&state.mstore, &req.shell_id, &req.agent_block_id) {
        return (
            StatusCode::OK,
            Json(PtyShellInputResponse {
                written: false,
                error: Some("not a shell belonging to this agent's own pane".to_string()),
            }),
        );
    }
    // Lock BEFORE writing, not after (ReAgent P1 on PR #3194): the
    // previous ordering left `term:agentlockuntil` unset for the entire
    // duration of `send_input` — the exact window `controllerinput`'s own
    // lock check (added for Codex's earlier P1 on this same PR) needs the
    // lock to already exist to catch anything. Locking first doesn't make
    // the race provably impossible (a human keystroke processed in the
    // sliver of time before this write actually commits could still slip
    // through — true elimination would need a real mutex shared between
    // this HTTP path and the WS `controllerinput` path, disproportionate
    // machinery for a UX-level collision guard, not a hard security
    // boundary), but it shrinks the window from "all of send_input's own
    // duration plus the lock write's" down to just the lock write's own
    // commit latency — the actual, meaningful fix here.
    lock_shell_for_agent(&state, &req.shell_id);
    let result = blockcontroller::send_input(
        &req.shell_id,
        blockcontroller::BlockInputUnion::data(req.text.into_bytes()),
        None,
    );
    match result {
        Ok(()) => (StatusCode::OK, Json(PtyShellInputResponse { written: true, error: None })),
        Err(e) => (StatusCode::OK, Json(PtyShellInputResponse { written: false, error: Some(e) })),
    }
}

/// Lock `shell_id` out from human keyboard input for `AGENT_LOCK_WINDOW_MS`
/// from now — called BEFORE every agent write (input/resize), not after
/// (see the call sites' own comment for why the ordering matters).
/// Best-effort: a failure to write the lock meta just means the human's
/// input isn't blocked this time, not that the agent's write itself failed.
pub(super) fn lock_shell_for_agent(state: &AppState, shell_id: &str) {
    let until_ms = agentmux_common::time::now_ms() + AGENT_LOCK_WINDOW_MS;

    // In-memory first, and unconditionally: this is what `controllerinput`
    // actually enforces against (see `blockcontroller::agent_lock`'s module
    // doc for why the enforcement path can't afford a `Store` read). It
    // cannot fail and costs a hash insert, so the lock is live before the
    // persisted/broadcast half is even attempted — which also shrinks the
    // pre-existing "human keystroke in the sliver before the write commits"
    // window described above to the width of a mutex acquire.
    blockcontroller::agent_lock::lock_until(shell_id, until_ms);

    // Then the meta write, which drives the frontend's own badge + gate
    // (`AgentShellSubblock.tsx`). Best-effort, exactly as before: a failure
    // here costs the human the "Agent is using this shell" indicator, not
    // the enforcement itself.
    let mut meta = crate::backend::obj::MetaMapType::new();
    meta.insert(META_KEY_AGENT_LOCK_UNTIL.to_string(), json!(until_ms));
    if let Err(e) = broadcast_meta_update(state, shell_id, &meta) {
        tracing::warn!(shell_id = %shell_id, error = %e, "ptyshell: failed to write agent lock");
    }
}

/// `POST /api/v1/ptyshell/resize` — resize the PTY. Some TUIs render
/// differently or wrap badly at the fallback geometry (25x200).
pub(super) async fn handle_pty_shell_resize(
    State(state): State<AppState>,
    Json(req): Json<PtyShellResizeRequest>,
) -> impl IntoResponse {
    if !is_owned_by_agent(&state.mstore, &req.shell_id, &req.agent_block_id) {
        return (
            StatusCode::OK,
            Json(PtyShellResizeResponse {
                resized: false,
                error: Some("not a shell belonging to this agent's own pane".to_string()),
            }),
        );
    }
    // Lock BEFORE writing — see the identical comment in
    // `handle_pty_shell_input` for why the ordering matters.
    lock_shell_for_agent(&state, &req.shell_id);
    let result = blockcontroller::send_input(
        &req.shell_id,
        blockcontroller::BlockInputUnion::resize(crate::backend::obj::TermSize {
            rows: req.rows as i64,
            cols: req.cols as i64,
        }),
        None,
    );
    match result {
        Ok(()) => (StatusCode::OK, Json(PtyShellResizeResponse { resized: true, error: None })),
        Err(e) => (StatusCode::OK, Json(PtyShellResizeResponse { resized: false, error: Some(e) })),
    }
}

/// `POST /api/v1/ptyshell/read` — read back the shell's raw output tail.
///
/// Reads the block's `term` file straight from `FileStore` — the same file
/// `blockcontroller::shell::lifecycle`'s real PTY read loop write-throughs
/// to (`handle_append_block_file(..., "term", ..., filestore, ...)`,
/// `SPEC_TERMINAL_SCROLLBACK_PERSISTENCE_2026_07_23.md` §2.1). Deliberately
/// NOT the optimized `blockfile:read_range` path (byte-offset index,
/// cross-channel global-transcript fallback, and a different filename —
/// "output" — used for agent CLI transcripts, a different content stream
/// entirely), which solves problems this endpoint doesn't have: a PTY shell
/// created through this API is always local and short-lived, so a plain
/// whole-file read + in-memory tail is simpler and correct. This is a raw
/// line log, not a rendered-screen snapshot — see the type's own doc
/// comment in `api_types.rs`.
pub(super) async fn handle_pty_shell_read(
    State(state): State<AppState>,
    Json(req): Json<PtyShellReadRequest>,
) -> impl IntoResponse {
    if !is_owned_by_agent(&state.mstore, &req.shell_id, &req.agent_block_id) {
        return (
            StatusCode::OK,
            Json(PtyShellReadResponse { content: String::new(), truncated: false }),
        )
            .into_response();
    }
    let content = match state.filestore.read_file(&req.shell_id, "term") {
        Ok(Some(bytes)) => String::from_utf8_lossy(&bytes).to_string(),
        Ok(None) => String::new(),
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": format!("ptyshell.read: {e}") })),
            )
                .into_response();
        }
    };
    let tail_lines = req.tail_lines.unwrap_or(200).max(1) as usize;
    let lines: Vec<&str> = content.lines().collect();
    let start = lines.len().saturating_sub(tail_lines);
    let truncated = start > 0;
    let tail = lines[start..].join("\n");
    (StatusCode::OK, Json(PtyShellReadResponse { content: tail, truncated })).into_response()
}

/// `POST /api/v1/ptyshell/status` — query whether the shell is still running.
pub(super) async fn handle_pty_shell_status(
    State(state): State<AppState>,
    Json(req): Json<PtyShellStatusRequest>,
) -> impl IntoResponse {
    if !is_owned_by_agent(&state.mstore, &req.shell_id, &req.agent_block_id) {
        return (StatusCode::OK, Json(PtyShellStatusResponse { running: false, exit_code: None }));
    }
    match blockcontroller::get_block_controller_status(&req.shell_id) {
        Some(status) => {
            let running = status.shellprocstatus == blockcontroller::STATUS_RUNNING;
            let exit_code = (status.shellprocstatus == blockcontroller::STATUS_DONE)
                .then_some(status.shellprocexitcode);
            (StatusCode::OK, Json(PtyShellStatusResponse { running, exit_code }))
        }
        // No controller registered — unknown id or already torn down via
        // ptyshell/stop. Same "plain not-running answer, not an error"
        // contract ShellStatus already documents.
        None => (StatusCode::OK, Json(PtyShellStatusResponse { running: false, exit_code: None })),
    }
}

/// `POST /api/v1/ptyshell/stop` — release the agent's lock immediately,
/// without waiting for `AGENT_LOCK_WINDOW_MS` to lapse on its own.
///
/// Does not touch the process or the block at all (see
/// `PtyShellStopResponse`'s doc comment in `api_types.rs` for why: this
/// shell is a shared, pane-scoped resource now, not a private one this API
/// owns outright).
pub(super) async fn handle_pty_shell_stop(
    State(state): State<AppState>,
    Json(req): Json<PtyShellStopRequest>,
) -> impl IntoResponse {
    if !is_owned_by_agent(&state.mstore, &req.shell_id, &req.agent_block_id) {
        return (StatusCode::OK, Json(PtyShellStopResponse { released: false }));
    }
    // Released in memory first — that's the copy `controllerinput` enforces
    // against, so the human's keyboard is live again the instant this
    // returns, not once the meta write commits and the frontend hears about
    // it. The return value is the authoritative "was a lease actually held",
    // for the same reason.
    let was_locked = blockcontroller::agent_lock::release(&req.shell_id);

    let mut meta = crate::backend::obj::MetaMapType::new();
    meta.insert(META_KEY_AGENT_LOCK_UNTIL.to_string(), serde_json::Value::Null);
    let released = broadcast_meta_update(&state, &req.shell_id, &meta).is_ok() && was_locked;

    tracing::info!(block_id = %req.shell_id, released, "ptyshell.stop: released lock");
    (StatusCode::OK, Json(PtyShellStopResponse { released }))
}

/// The 409 for a pane whose agent shell is running (or starting, created by
/// this run) on another connection than `connection`, checked without
/// touching the shell; the same cases `try_attach_to_existing_shell` refuses.
fn running_shell_conflict(state: &AppState, agent_block_id: &str, connection: &str) -> Option<axum::response::Response> {
    let shell_id = state
        .mstore
        .get::<crate::backend::obj::Block>(agent_block_id)
        .ok()
        .flatten()
        .and_then(|b| b.meta.get(META_KEY_SHELL_SUBBLOCK_ID).and_then(|v| v.as_str()).map(str::to_string))?;
    let shell = state.mstore.get::<crate::backend::obj::Block>(&shell_id).ok().flatten()?;
    // Running, or created by this srv run and still starting (a concurrent
    // create won the claim): either way `try_attach_to_existing_shell` would
    // refuse it, so it is refused here too, before anything is asked.
    let live = match blockcontroller::get_block_controller_status(&shell_id) {
        Some(s) => s.shellprocstatus == blockcontroller::STATUS_RUNNING,
        None => {
            crate::backend::obj::meta_get_string(&shell.meta, super::http_shell::META_KEY_PTYSHELL_BOOT_ID, "")
                == *state.boot_id
        }
    };
    if !live {
        return None;
    }
    super::http_shell::connection_conflict(&shell, connection)
}
