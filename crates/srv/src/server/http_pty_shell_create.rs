// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The steps of `handle_pty_shell_create`, one function each, in the order the handler runs them:
//! where the shell runs, the SSH consent, the reuse of the pane's existing shell, the new shell's
//! block, the atomic claim of the pane's one shell pointer, the controller start and its rollback.
//! Moved out of the handler without a change in behaviour. A child module of `http_pty_shell.rs`.

// The `Err` is the response that ends the request early, built at most once per call.
#![allow(clippy::result_large_err)]

use super::app_api::connections::{self, AgentTarget};
use super::*;

type Early = axum::response::Response;

/// `POST /api/v1/ptyshell/create`, step by step. `Err` carries a response that ends the call early:
/// a refusal, or the attachment to the pane's existing shell.
pub(super) async fn create_pty_shell(state: &AppState, req: &PtyShellCreateRequest) -> Result<Early, Early> {
    let (target, connection) = resolve_connection(state, req).await?;
    let ssh_agent = ssh_gate(state, req, &target, &connection).await?;
    if let Some(resp) = attach_to_pointed_shell(state, req, &connection).await {
        return Ok(resp);
    }

    let child_id = uuid::Uuid::new_v4().to_string();
    let mut meta = shell_block_meta(state, req, &target, &connection);
    add_ssh_and_cwd_meta(state, req, &target, &ssh_agent, &mut meta);
    let (runtimeopts, rt_opts_json) = initial_geometry(req);
    let block = crate::backend::obj::Block {
        oid: child_id.clone(),
        parentoref: format!("block:{}", req.agent_block_id),
        meta,
        runtimeopts,
        ..Default::default()
    };
    let block = claim_shell_block(state, req, block, &child_id, &connection).await?;

    if let Err(e) = start_shell_controller(state, &block, rt_opts_json) {
        roll_back_failed_spawn(state, req, &child_id);
        return Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": format!("ptyshell.create: spawn: {e}") })),
        )
            .into_response());
    }

    // baseline_len=0: a genuinely new block has no prior "term" file content
    // to accidentally match against (see
    // `try_attach_to_existing_shell`'s identical baseline logic for why this matters at all).
    answer_conpty_handshake_if_seen(state, &child_id, 0).await;

    // NOTE: the shellsubblockid pointer is already persisted — the atomic
    // claim transaction (`claim_shell_block`) sets it on the parent in the same
    // transaction as the insert, which is also strictly more correct than
    // a separate post-hoc write here would be (no window where the block
    // exists but isn't yet pointed to).
    tracing::info!(block_id = %child_id, parent_id = %req.agent_block_id, "ptyshell.create: fresh");
    Ok((StatusCode::OK, Json(PtyShellCreateResponse { shell_id: child_id })).into_response())
}

/// Where the shell runs and its connection name, or the refusal. A refused connection fails the call
/// with its reason rather than opening a local shell.
pub(super) async fn resolve_connection(state: &AppState, req: &PtyShellCreateRequest) -> Result<(AgentTarget, String), Early> {
    // Where the shell runs, checked the way a pane's connection is: a refused
    // one fails the call with its reason rather than opening a local shell.
    let target = match connections::for_agent(&state.broker, req.connection.as_deref()).await {
        Ok(t) => t,
        Err(e) => {
            return Err((StatusCode::BAD_REQUEST, Json(json!({ "error": e }))).into_response());
        }
    };
    let connection = match &target {
        AgentTarget::Local => "local".to_string(),
        AgentTarget::Wsl(d) => crate::backend::remote::ConnTarget::Wsl(d.clone()).name(),
        AgentTarget::Ssh(dest) => dest.name(),
    };
    Ok((target, connection))
}

/// The agent that consents for an SSH shell (empty for any other connection), after the checks that come
/// before anything is asked.
pub(super) async fn ssh_gate(state: &AppState, req: &PtyShellCreateRequest, target: &AgentTarget, connection: &str) -> Result<String, Early> {
    // An SSH host runs with the user's identity: their consent first, and the
    // askpass helper must exist, since its prompts may never reach the
    // terminal the agent reads (`remote::askpass`).
    let mut ssh_agent = String::new();
    if let AgentTarget::Ssh(_) = target {
        if crate::backend::remote::askpass::program().is_none() {
            let e = "this AgentMux has no askpass helper (agentmux-bashwrap) to run an agent's SSH shell with";
            return Err((StatusCode::BAD_REQUEST, Json(json!({ "error": e }))).into_response());
        }
        ssh_agent = match connections::verified_agent(state, &req.agent_block_id, req.auth.as_ref()) {
            Ok(a) => a,
            Err(e) => return Err((StatusCode::FORBIDDEN, Json(json!({ "error": e }))).into_response()),
        };
        // A running shell on another connection is refused before anything
        // is asked: consent (and an "always") for a shell that then cannot
        // start would be a grant the user gave for nothing.
        if let Some(conflict) = running_shell_conflict(state, &req.agent_block_id, connection) {
            return Err(conflict);
        }
        if let Err(e) = connections::consent_for_ssh(state, &req.agent_block_id, &ssh_agent, connection, "an interactive terminal").await {
            return Err((StatusCode::FORBIDDEN, Json(json!({ "error": e }))).into_response());
        }
    }
    Ok(ssh_agent)
}

/// The pane's existing shell, if its pointer still resolves to one that can be handed out.
pub(super) async fn attach_to_pointed_shell(state: &AppState, req: &PtyShellCreateRequest, connection: &str) -> Option<Early> {
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
    let id = existing_id?;
    let attached = try_attach_to_existing_shell(state, &req.agent_block_id, &id, connection).await;
    // `None` here means either a stale pointer (the block was deleted
    // some other way — rare) or, as of this PR, the pane's shell
    // having already exited (the common case this PR fixes — see
    // `try_attach_to_existing_shell`'s own doc comment). Either way,
    // fall through to creating a fresh one, same as if none existed.
    attached
}

/// The new shell block's meta: the headless `term` shape, the agent's env guards, the automatic shell,
/// and the connection.
pub(super) fn shell_block_meta(state: &AppState, req: &PtyShellCreateRequest, target: &AgentTarget, connection: &str) -> crate::backend::obj::MetaMapType {
    let remote = !matches!(target, AgentTarget::Local);
    let mut meta = crate::backend::obj::MetaMapType::new();
    // Which srv run created this shell: a block with no controller is one this
    // run is still starting (a concurrent create), not one left from before a
    // restart (`try_attach_to_existing_shell`).
    meta.insert(META_KEY_PTYSHELL_BOOT_ID.to_string(), json!(&*state.boot_id));
    // The agent's plain-`gh` guard, as its Shell and its own CLI get it: the
    // agent types into this shell, so `gh` here must not act as the user.
    let mut agent_env: std::collections::HashMap<String, String> = state
        .mstore
        .get::<crate::backend::obj::Block>(&req.agent_block_id)
        .ok()
        .flatten()
        .map(|b| crate::backend::blockcontroller::cmd_env_of(&b.meta))
        .unwrap_or_default();
    // Its current slug, not a persisted one, names the guard directory, and
    // the shell gets the identity it may carry (`pty_child_identity_env`).
    crate::server::agent_handlers::identity_env::carry_block_slug_identity_env(&state.mstore, &req.agent_block_id, &mut agent_env);
    let child_env = crate::server::agent_handlers::identity_env::pty_child_identity_env(&agent_env);
    if !child_env.is_empty() {
        meta.insert(blockcontroller::META_KEY_CMD_ENV.to_string(), serde_json::Value::Object(child_env));
    }
    meta.insert(
        crate::backend::gh_guard::META_KEY_PTYSHELL_GH_CONFIG_DIR.to_string(),
        json!(crate::backend::gh_guard::guard_dir_for_target(&agent_env, remote)),
    );
    // And the publish guard's git hooks, so a push typed here is scanned too.
    // Local shells only: the hooks directory is a host path, which a WSL distro
    // or an SSH host can't run (pushes from there aren't guarded).
    let local = matches!(target, AgentTarget::Local);
    if let Some(hooks) = local.then(|| crate::backend::publish_guard::hooks_dir_for(&agent_env, |k| std::env::var(k).ok())).flatten() {
        meta.insert(
            crate::backend::publish_guard::META_KEY_PTYSHELL_GIT_HOOKS_PATH.to_string(),
            json!(hooks.to_string_lossy()),
        );
    }
    meta.insert("view".to_string(), json!("term"));
    meta.insert(
        blockcontroller::META_KEY_CONTROLLER.to_string(),
        json!(blockcontroller::BLOCK_CONTROLLER_SHELL),
    );
    // An agent's shell doesn't follow the user's default-shell setting
    // (term:localshellpath and term:localshellopts are for their terminals):
    // pin it to the automatic shell with no extra arguments. Only macOS and
    // Linux reach that branch; Windows runs cmd.exe below.
    meta.insert(
        blockcontroller::META_KEY_TERM_LOCAL_SHELL_PATH.to_string(),
        json!(crate::backend::shells::automatic().display().to_string()),
    );
    meta.insert(blockcontroller::META_KEY_TERM_LOCAL_SHELL_OPTS.to_string(), json!([]));
    // Force cmd.exe directly on Windows rather than the "shell" controller's
    // default pwsh/powershell auto-detection (`backend::shells::automatic`,
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
    meta
}

/// The SSH agent's identity and the working directory, onto the new shell's meta.
pub(super) fn add_ssh_and_cwd_meta(
    state: &AppState,
    req: &PtyShellCreateRequest,
    target: &AgentTarget,
    ssh_agent: &str,
    meta: &mut crate::backend::obj::MetaMapType,
) {
    let remote = !matches!(target, AgentTarget::Local);
    // If the caller didn't supply a cwd, fall back to the agent block's own
    // cmd:cwd — the project directory the agent pane was launched with.
    // Mirrors `handle_shell_create`'s identical fallback (Codex P1 on PR
    // #3177): without this, the PTY spawns in agentmux-srv's own cwd
    // (typically the portable runtime/ dir), not the agent's worktree.
    //
    // In WSL the cwd is a distro path, kept as given; no Windows fallback.
    if let AgentTarget::Ssh(_) = target {
        let agent = ssh_agent.to_string();
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
}

/// The caller's initial PTY geometry, if it gave one, as runtime options and as the JSON the resync takes.
pub(super) fn initial_geometry(req: &PtyShellCreateRequest) -> (Option<crate::backend::obj::RuntimeOpts>, Option<serde_json::Value>) {
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
    (runtimeopts, rt_opts_json)
}

/// Tell subscribed frontends the agent's block changed (its shell pointer or its sub-blocks).
fn broadcast_agent_block_update(state: &AppState, agent_block_id: &str, parent: &crate::backend::obj::Block) {
    let update = crate::backend::obj::MuxObjUpdate {
        updatetype: "update".into(),
        otype: "block".into(),
        oid: agent_block_id.to_string(),
        obj: Some(crate::backend::obj::mux_obj_to_value(parent)),
    };
    state.event_bus.broadcast_event(&crate::backend::eventbus::EventBus::typed_ws_event::<crate::backend::mps_events::MuxObjUpdated>(&update, format!("block:{}", agent_block_id)));
}

/// Insert the new shell block and point the agent's block at it, unless another call got there first,
/// in which case that call's shell is attached to instead (`Err` carries the response to return).
pub(super) async fn claim_shell_block(
    state: &AppState,
    req: &PtyShellCreateRequest,
    mut block: crate::backend::obj::Block,
    child_id: &str,
    connection: &str,
) -> Result<crate::backend::obj::Block, Early> {
    // Atomic claim-or-lose (Codex P2 on PR #3194): without this, two
    // concurrent `PtyShell` calls that both observe an unset
    // `term:shellsubblockid` (the check in
    // `attach_to_pointed_shell` is NOT authoritative — it's a plain read, before this point) would each
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
        parent.subblockids.get_or_insert_with(Vec::new).push(child_id.to_string());
        parent
            .meta
            .insert(META_KEY_SHELL_SUBBLOCK_ID.to_string(), json!(child_id));
        tx.update(&mut parent)?;
        Ok(None)
    });

    match claim {
        // We lost the race — someone else's transaction committed the
        // pointer first. Pivot to reusing THEIRS rather than proceeding
        // with the block we prepared but never actually inserted.
        Ok(Some(winner_id)) => {
            if let Some(resp) = try_attach_to_existing_shell(state, &req.agent_block_id, &winner_id, connection).await {
                return Err(resp);
            }
            insert_after_lost_claim(state, req, block, child_id)
        }
        Ok(None) => {
            // We won the claim and the parent's meta changed (inside the
            // transaction, via a raw `tx.update`, which — unlike
            // `broadcast_meta_update` — does NOT itself notify any
            // subscribed frontend). Broadcast now so a drawer already open
            // (or opened moments later) actually sees the pointer reactively
            // instead of only on its next unrelated refetch.
            if let Ok(Some(parent)) = state.mstore.get::<crate::backend::obj::Block>(&req.agent_block_id) {
                broadcast_agent_block_update(state, &req.agent_block_id, &parent);
            }
            Ok(block)
        }
        Err(e) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": format!("ptyshell.create: claim: {e}") })),
        )
            .into_response()),
    }
}

/// The winner's shell is gone or had exited: create ours for real, as the won-claim path would have.
fn insert_after_lost_claim(
    state: &AppState,
    req: &PtyShellCreateRequest,
    mut block: crate::backend::obj::Block,
    child_id: &str,
) -> Result<crate::backend::obj::Block, Early> {
    // Winner's block vanished between the transaction and now, OR
    // (as of this PR) resolved to a shell that had already exited.
    // Both stay exceedingly unlikely to land HERE specifically:
    // `try_attach_to_existing_shell` now clears the parent's
    // pointer itself the moment it finds an exited shell (see its
    // own comment on the `RESYNC_ERR_ALREADY_EXITED` branch), so
    // the common "human typed `exit`, then something calls
    // PtyShellCreate again" case is intercepted one level up — the
    // CLAIM transaction (`claim_shell_block`) reads the pointer as already-cleared
    // and takes the plain `Ok(None)` fresh-insert path, never
    // reaching this branch at all. Getting here for an exited
    // shell specifically requires a genuine concurrent-call race
    // (two callers both observe the stale pointer before either
    // one's clear commits) — same rarity class as the pre-existing
    // deleted-out-from-under-us case this comment originally
    // described, not the common case. `block` was only ever
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
            .push(child_id.to_string());
        parent
            .meta
            .insert(META_KEY_SHELL_SUBBLOCK_ID.to_string(), json!(child_id));
        state.mstore.update(&mut parent)?;
        Ok(())
    })();
    if let Err(e) = insert_result {
        return Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": format!("ptyshell.create: fallback insert: {e}") })),
        )
            .into_response());
    }
    if let Ok(Some(parent)) = state
        .mstore
        .get::<crate::backend::obj::Block>(&req.agent_block_id)
    {
        broadcast_agent_block_update(state, &req.agent_block_id, &parent);
    }
    Ok(block)
}

/// Start the shell's controller; the PTY is live when this returns.
pub(super) fn start_shell_controller(state: &AppState, block: &crate::backend::obj::Block, rt_opts_json: Option<serde_json::Value>) -> Result<(), String> {
    let registry = state.mstore.shared_agent_registry();
    blockcontroller::resync_controller(
        block,
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
    )
}

/// A failed spawn leaves nothing behind: the block, its link in the agent's block and the pointer go.
pub(super) fn roll_back_failed_spawn(state: &AppState, req: &PtyShellCreateRequest, child_id: &str) {
    // Roll back — the block was already inserted and linked into the
    // parent's subblockids by the claim (#3177): without this,
    // a failed spawn leaves a stale block + parent link + (possibly)
    // registered controller behind that the caller has no way to clean
    // up, since it never received a `shell_id` to call PtyShellStop with.
    crate::sagas::agent_teardown::discard(child_id);
    let _ = state.mstore.delete::<crate::backend::obj::Block>(child_id);
    if let Ok(mut parent) = state
        .mstore
        .must_get::<crate::backend::obj::Block>(&req.agent_block_id)
    {
        let mut changed = false;
        if let Some(ids) = parent.subblockids.as_mut() {
            let before = ids.len();
            ids.retain(|id| id != child_id);
            changed |= ids.len() != before;
        }
        // The pointer was already persisted before `resync_controller`
        // ran — either inside the atomic claim's own transaction, or by
        // the "winner vanished" fallback (`insert_after_lost_claim`) — so unlinking
        // `subblockids` alone isn't enough: without also clearing it
        // here, `term:shellsubblockid` is left pointing at a block that
        // was just deleted, and a human opening the composer drawer in
        // this exact window (`agent-view.tsx`'s
        // `existingSubBlockId={block()?.meta?.["term:shellsubblockid"]}`)
        // attaches to nothing instead of getting a working shell
        // (ReAgent P1, round 3 on PR #3194).
        if parent.meta.get(META_KEY_SHELL_SUBBLOCK_ID).and_then(|v| v.as_str()) == Some(child_id)
        {
            parent.meta.remove(META_KEY_SHELL_SUBBLOCK_ID);
            changed = true;
        }
        if changed {
            let _ = state.mstore.update(&mut parent);
            broadcast_agent_block_update(state, &req.agent_block_id, &parent);
        }
    }
}
