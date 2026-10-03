// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Shell drawer HTTP handlers and the agent shell lock.
//! Split out of server/mod.rs unchanged (SPEC_LARGE_FILE_MODULE_ANALYSIS_2026_09_30.md §4.1).

use super::*;

/// `POST /api/v1/shell/create` — start a persistent background shell.
///
/// Called by `agentmux-mcp`'s `Shell` tool. Returns immediately with a
/// `shell_id`; the `ShellNodeRunner` streams stdout/stderr to the frontend
/// as `shell_chunk` MPS events without blocking the agent.
pub(super) async fn handle_shell_create(
    State(state): State<AppState>,
    Json(req): Json<ShellCreateRequest>,
) -> impl IntoResponse {
    // Where it runs. Checked before anything is published, so a refused
    // connection (SSH before P2, a distro that isn't installed, a bad name)
    // fails the call with its reason instead of running the command here.
    let wsl_distro = match super::app_api::connections::for_agent(
        &state.broker,
        req.connection.as_deref(),
    )
    .await
    {
        Ok(d) => d,
        Err(e) => {
            return (StatusCode::BAD_REQUEST, Json(json!({ "error": e }))).into_response();
        }
    };

    let shell_id = uuid::Uuid::new_v4().to_string();
    let title = req.title.as_deref().unwrap_or(&req.cmd).to_string();
    let now_ms = agentmux_common::time::now_ms_u64();

    // Read the agent block once for both the cwd and env fallbacks below.
    let agent_block = state.mstore
        .get::<crate::backend::obj::Block>(&req.agent_block_id)
        .ok()
        .flatten();

    // If cwd wasn't supplied by the caller, fall back to the agent block's
    // cmd:cwd — the working directory the agent pane was launched with.
    // Without this, ShellNodeRunner would inherit agentmux-srv's cwd
    // (typically the portable runtime/ dir) instead of the project dir.
    //
    // In WSL the cwd is a path inside the distro, given as is, and the agent's
    // own (Windows) directory is no fallback for it: the distro's home is.
    let effective_cwd = if wsl_distro.is_some() {
        req.cwd.clone()
    } else {
        req.cwd.clone().or_else(|| {
        agent_block.as_ref().and_then(|block| {
            let cwd = crate::backend::obj::meta_get_string(&block.meta, "cmd:cwd", "");
            if cwd.is_empty() { None } else { Some(cwd.to_string()) }
        })
        })
    };

    // Normalize the cwd before it reaches the spawner. Agents on Windows run
    // inside a bash shell and emit MSYS paths like `/c/Users/asafe/project`;
    // passing those straight to `Command::current_dir` fails with os error 267
    // (ERROR_DIRECTORY). This converts them to native form and expands `~`.
    let effective_cwd = if wsl_distro.is_some() {
        effective_cwd
    } else {
        effective_cwd.and_then(|c| crate::backend::base::normalize_working_dir(&c))
    };

    // Env parity with the agent CLI: start from the agent block's stored
    // cmd:env (the per-agent env the agent process is launched with — same
    // shape app_api.rs / websocket.rs read), then let the caller-supplied
    // req.env override on top (explicit Shell env wins). This forwards the
    // concrete per-agent env so `Shell(...)` runs with the same env the agent
    // itself sees, mirroring the cmd:cwd fallback above.
    //
    // NOT forwarded here: the dynamic identity bindings (resolver.rs) and the
    // bundled tools/bin PATH prefix that blockcontroller/shell.rs injects at
    // agent-CLI spawn time — those are resolved live per spawn, not stored in
    // cmd:env. The MCP server (agentmux-mcp) is itself launched by the agent
    // CLI through the bundled tools/bin, so tools it spawns inherit that PATH;
    // shells created here run from agentmux-srv's env plus cmd:env + req.env.
    let mut effective_env: std::collections::HashMap<String, String> = agent_block
        .as_ref()
        .and_then(|block| match block.meta.get("cmd:env") {
            Some(serde_json::Value::Object(obj)) => Some(
                obj.iter()
                    .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                    .collect(),
            ),
            _ => None,
        })
        .unwrap_or_default();
    let caller_keys: Vec<String> = req.env.iter().flat_map(|e| e.keys().cloned()).collect();
    if let Some(req_env) = req.env {
        effective_env.extend(req_env);
    }
    // After the caller's overrides: a command run on an agent's behalf gets the
    // same plain-`gh` guard as the agent itself, and `req.env` can't lift it.
    // Nor can it hand that command the account's cloud login.
    crate::backend::gh_guard::apply_gh_guard(&mut effective_env);
    crate::backend::account_login_guard::strip_account_login(&mut effective_env);
    // WSL passes on only what WSLENV names: the caller's own variables and the
    // gh guard, never the rest of the agent's env.
    let wsl_args = wsl_distro.as_ref().map(|distro| {
        effective_env.insert(
            "WSLENV".to_string(),
            crate::backend::remote::wsl::agent_wslenv(
                &std::env::var("WSLENV").unwrap_or_default(),
                caller_keys.iter().map(String::as_str),
            ),
        );
        crate::backend::remote::wsl::launch(
            distro,
            &req.cmd,
            &[],
            effective_cwd.as_deref().unwrap_or_default(),
        )
        .args
    });

    tracing::info!(
        block_id = %req.agent_block_id,
        shell_id = %shell_id,
        cmd = %req.cmd,
        cwd = ?effective_cwd,
        wsl = ?wsl_distro,
        "shell.create"
    );

    // Publish shell_node_create so the frontend inserts the row
    // before the first chunk arrives (avoids a flash of orphaned chunks).
    // persist: 64 — retain up to 64 shell_node_create events per block scope
    // so multiple shells in a pane all replay on WS reconnect / pane remount.
    // (persist: 1 meant only the last shell's create event was kept; earlier
    // shells lost their create event while their shell_chunk events at
    // persist: 1024 still replayed, causing the reducer to silently drop
    // orphaned chunks.)
    state.broker.publish(crate::backend::mps::MuxEvent {
        event: crate::backend::mps::EVENT_SHELL_NODE_CREATE.to_string(),
        scopes: vec![format!("block:{}", req.agent_block_id)],
        sender: String::new(),
        persist: 64,
        data: Some(json!({
            "shell_id": shell_id,
            "cmd": req.cmd,
            "cwd": effective_cwd,
            "title": title,
            "timestamp": now_ms,
        })),
    });

    // Spawn the runner — fire-and-forget; events flow independently.
    let runner = crate::backend::shell_node::ShellNodeRunner {
        shell_id: shell_id.clone(),
        block_id: req.agent_block_id,
        cmd: req.cmd,
        title,
        cwd: if wsl_args.is_some() { None } else { effective_cwd },
        wsl_args,
        extra_env: effective_env,
        broker: Arc::clone(&state.broker),
        registry: Arc::clone(&state.shell_sessions),
        capture_stdin: req.capture_stdin.unwrap_or(false),
    };
    tokio::spawn(runner.run());

    (StatusCode::OK, Json(ShellCreateResponse { shell_id })).into_response()
}

/// `POST /api/v1/shell/stop` — stop a running persistent shell.
///
/// Called by `agentmux-mcp`'s `ShellStop` tool. Tree-kills the shell's
/// process group (so `task dev` → `task.exe`/`node` grandchildren die too),
/// which makes the runner publish a `stopped` exit event. Returns `{ stopped }`
/// — false if the id is unknown (never started or already exited).
pub(super) async fn handle_shell_stop(
    State(state): State<AppState>,
    Json(req): Json<ShellStopRequest>,
) -> impl IntoResponse {
    let stopped = state.shell_sessions.stop(&req.shell_id);
    tracing::info!(shell_id = %req.shell_id, stopped, "shell.stop");
    (StatusCode::OK, Json(json!({ "stopped": stopped })))
}

/// `POST /api/v1/shell/input` — write text to a running shell's stdin (Phase 3b).
///
/// Appends a newline so single answers like "y" work without the caller
/// needing to know the line discipline. Returns `{ written: false }` if the
/// shell is not running or the write fails (e.g. the process closed its stdin).
pub(super) async fn handle_shell_input(
    State(state): State<AppState>,
    Json(req): Json<ShellInputRequest>,
) -> impl IntoResponse {
    // Non-blocking send to the stdin relay task — no mutex, no risk of
    // blocking if the child's pipe buffer is full (the relay owns that concern).
    // resolve_stdin distinguishes "not running" from "running but no captured
    // stdin" so the caller gets an actionable reason instead of a bare false.
    let (written, reason) = match state.shell_sessions.resolve_stdin(&req.shell_id) {
        Ok(tx) => {
            let mut text = req.text.clone();
            if !text.ends_with('\n') {
                text.push('\n');
            }
            if tx.send(text).is_ok() {
                tracing::debug!(shell_id = %req.shell_id, "shell.input: written");
                (true, None)
            } else {
                // Relay task gone / channel closed — process closed its stdin.
                tracing::debug!(shell_id = %req.shell_id, "shell.input: write failed");
                (false, Some(ShellInputFailure::WriteFailed))
            }
        }
        Err(failure) => {
            tracing::debug!(shell_id = %req.shell_id, ?failure, "shell.input: no target");
            (false, Some(failure))
        }
    };
    (StatusCode::OK, Json(ShellInputResponse { written, reason }))
}

/// `POST /api/v1/shell/status` — query a shell's running state (Phase 3b).
///
/// Returns `{ running, exit_code, line_count }`. If the shell_id is unknown
/// (never started or never seen by this sidecar), `running` is false and
/// `exit_code` is absent.
pub(super) async fn handle_shell_status(
    State(state): State<AppState>,
    Json(req): Json<ShellStatusRequest>,
) -> impl IntoResponse {
    let s = state.shell_sessions.get_status(&req.shell_id);
    (StatusCode::OK, Json(ShellStatusResponse {
        running: s.running,
        exit_code: s.exit_code,
        line_count: s.line_count,
    }))
}

// Meta key persisting which sub-block is "this pane's one shell" — the same
// key `agent-view.tsx` reads/writes for the human-facing composer drawer
// (`agent-view.tsx:2883,2890-2894`). `PtyShell` reuses this instead of
// minting an independent, invisible shell: reusing it means the agent
// drives the SAME shell a human sees in the drawer, and a drawer opened
// later attaches to whatever the agent already started, live output and
// scrollback included.
pub(super) const META_KEY_SHELL_SUBBLOCK_ID: &str = "term:shellsubblockid";

// How long an agent's write (input/resize) locks the shell out from human
// keyboard input, refreshed on every subsequent write. Deliberately a lease,
// not an explicit lock/unlock pair: an explicit "release" call is one the
// agent could simply never make (error, crash, forgetting), which would
// leave a human locked out indefinitely — the opposite of "unlock as soon
// as the agent stops." A short, self-expiring window means the frontend
// needs no server push to release it either — it just compares the
// timestamp against its own clock.
pub(super) const AGENT_LOCK_WINDOW_MS: i64 = 4000;

// Meta key holding the lock's absolute expiry (epoch ms). The frontend
// gates human keyboard input purely on `Date.now() < this`, reactively —
// no server-side timer to leak or get stuck; a quiet agent just lets the
// timestamp lapse and control returns on its own.
// pub(crate): `server::websocket`'s `controllerinput` handler also reads
// this — see that handler's own doc comment for why.
pub(crate) const META_KEY_AGENT_LOCK_UNTIL: &str = "term:agentlockuntil";

/// Whether `shell_id` is a real `view:"term"` sub-block PARENTED TO
/// `agent_block_id` — the actual safety property Codex asked for (PR
/// #3177): `shell_id` lives in the same block-id namespace as every other
/// pane, and `Layout` exposes block ids for ordinary panes, so without this
/// check an agent could point `PtyShellInput`/`PtyShellStop`/etc. at any
/// controller-backed block (another agent's own CLI pane, a human's
/// terminal pane) and inject input into it or tear it down. Framed as
/// parentage rather than "did `PtyShell` create it" (the original,
/// narrower check) specifically so it also covers the reuse case above: a
/// shell the HUMAN created via the drawer is just as legitimate a target as
/// one `PtyShell` created itself, as long as it's this agent's own pane.
pub(super) fn is_owned_by_agent(
    mstore: &crate::backend::storage::store::Store,
    shell_id: &str,
    agent_block_id: &str,
) -> bool {
    mstore
        .get::<crate::backend::obj::Block>(shell_id)
        .ok()
        .flatten()
        .map(|b| {
            b.parentoref == format!("block:{agent_block_id}")
                && b.meta.get("view").and_then(|v| v.as_str()) == Some("term")
        })
        .unwrap_or(false)
}

/// Merge `meta_update` into `block_id`'s meta and broadcast a
/// `waveobj:update` WS event so every subscribed frontend (the drawer, if
/// open) picks it up immediately — the same two steps the `setmeta` WS
/// handler performs (`websocket.rs`'s `COMMAND_SET_META` handler), factored
/// out since `ptyshell/*` needs the identical pattern from a plain HTTP
/// handler with no `WshRpcEngine` in scope.
pub(super) fn broadcast_meta_update(
    state: &AppState,
    block_id: &str,
    meta_update: &crate::backend::obj::MetaMapType,
) -> Result<(), String> {
    let oref_str = format!("block:{block_id}");
    crate::server::service::object_helpers::update_object_meta(&state.mstore, &oref_str, meta_update)?;
    let block = state
        .mstore
        .must_get::<crate::backend::obj::Block>(block_id)
        .map_err(|e| e.to_string())?;
    state.event_bus.broadcast_event(&crate::backend::eventbus::WSEventType {
        eventtype: "waveobj:update".to_string(),
        oref: oref_str,
        data: Some(serde_json::to_value(&crate::backend::obj::MuxObjUpdate {
            updatetype: "update".into(),
            otype: "block".into(),
            oid: block_id.to_string(),
            obj: Some(crate::backend::obj::mux_obj_to_value(&block)),
        }).unwrap_or_default()),
    });
    Ok(())
}

// ---------------------------------------------------------------------------
// PTY shell handlers (docs/specs/SPEC_AGENT_INTERACTIVE_PTY_SHELL_API_2026_09_10.md)
//
// A REAL PTY, unlike the piped `/api/v1/shell/*` family above — for driving
// genuinely interactive programs (Sharprompt-style wizards, `sudo`, `ssh`,
// REPLs) that check for a real terminal and refuse or misbehave on a pipe.
// Reuses `blockcontroller::shell` verbatim (the same controller backing the
// `term` widget and `AgentShellSubblock.tsx`'s composer-drawer shell) —
// this is a new agent-facing entry point onto existing PTY plumbing, not a
// new PTY implementation.
//
// Every handler here is a plain HTTP call with no WS/tab dependency, same as
// `handle_shell_create` above — deliberately NOT wired through the
// `WshRpcEngine` that `createsubblock`/`controllerresync`/`controllerinput`
// use (those are instantiated per-WebSocket-connection, so they exist only
// while a frontend tab is connected). An agent calling these tools gets a
// live, fully functional PTY regardless of whether any window is open,
// focused, or rendering the shell's `term` sub-block at all.
// ---------------------------------------------------------------------------

/// Shared by both the top-level reuse check and the "lost the atomic
/// claim" fallback in `handle_pty_shell_create`: resync `id`'s controller
/// and reply, or return `None` if `id` doesn't resolve to a real block
/// (stale pointer), OR if its shell has already exited, so the caller
/// falls through to creating a fresh one. A pane has one agent shell: asking
/// for it, while it runs, on another connection than its own is refused (409),
/// not answered with the shell on the wrong machine. An exited one is replaced
/// whatever its connection was.
pub(super) async fn try_attach_to_existing_shell(
    state: &AppState,
    agent_block_id: &str,
    id: &str,
    connection: &str,
) -> Option<axum::response::Response> {
    let block = state.mstore.get::<crate::backend::obj::Block>(id).ok().flatten()?;

    // Settled before the resync, which would otherwise start a shell on the
    // block's own connection: a running shell on another connection is refused
    // and left as it is; one with no controller (from before a srv restart) is
    // replaced, like an exited one, rather than revived on the old connection.
    // An exited one is replaced below whatever its connection was.
    if let Some(conflict) = connection_conflict(&block, connection) {
        match blockcontroller::get_block_controller_status(id) {
            Some(s) if s.shellprocstatus == blockcontroller::STATUS_RUNNING => return Some(conflict),
            None => {
                clear_shell_pointer_if(state, agent_block_id, id);
                tracing::info!(
                    block_id = %id,
                    parent_id = %agent_block_id,
                    "ptyshell.create: pane's shell is not running and was on another connection — falling through to a fresh shell"
                );
                return None;
            }
            Some(_) => {}
        }
    }

    // Baseline BEFORE resync — see `answer_conpty_handshake_if_seen`'s doc
    // comment (Codex P1 on PR #3194) for why this matters: the `term` file
    // is append-only for the block's whole lifetime, so an already-running
    // reused shell's history almost certainly still contains its own old,
    // long-since-answered `ESC[6n` from whenever it first started.
    let baseline_len = state
        .filestore
        .stat(id, "term")
        .ok()
        .flatten()
        .map(|info| info.size as usize)
        .unwrap_or(0);

    let registry = state.mstore.shared_agent_registry();
    if let Err(e) = blockcontroller::resync_controller(
        &block,
        "ptyshell",
        None,
        false,
        // respawn_if_done=false: don't resurrect a shell that already
        // exited (e.g. the human typed `exit`). The unconditional-respawn
        // default is correct for crash/backend-restart recovery
        // (`TermResyncHandler` on the WS path) but wrong here: every
        // subsequent `PtyShellCreate` against this same pane-shell pointer
        // would otherwise silently respawn it again, each respawn
        // appending a fresh shell-startup banner to the pane's
        // append-only `term` scrollback — indistinguishable from the
        // exited shell's output looping forever. Checked inside
        // `resync_controller` itself (one status read, not a separate
        // check-then-call here) so a shell that exits in the gap between
        // an earlier check and this call can't slip through and get
        // respawned anyway (Codex P2 on PR #3225) — see that parameter's
        // own doc comment.
        false,
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
        if e == blockcontroller::RESYNC_ERR_ALREADY_EXITED {
            // Treating an exited shell like a stale pointer (return
            // `None`) reuses the existing "create a fresh one" fallback in
            // both callers instead of inventing a new response shape.
            //
            // Also clear the now-stale pointer on the agent's own block:
            // without this, two concurrent `PtyShellCreate` calls against
            // the same exited pane would both fall through to
            // `handle_pty_shell_create`'s atomic-claim transaction, both
            // read the SAME still-set dead pointer, both take the "lost
            // the race, reuse THEIRS" branch pointing at that same dead
            // id, both find it exited too, and both independently execute
            // the non-transactional "vanished, create for real" fallback —
            // leaving one orphaned live controller and two callers holding
            // different shell_ids for what's supposed to be one pane's one
            // shell (Codex P2 on PR #3225). Clearing the pointer first
            // makes the already-correct atomic claim transaction see
            // "unclaimed" instead of "claimed by a dead id," so its
            // existing single-writer serialization (documented on that
            // transaction) resolves concurrent callers to one winner the
            // normal way.
            //
            // GUARDED, not unconditional (ReAgent P1 on this PR, round 2):
            // `resync_controller`'s STATUS_DONE read above happens outside
            // any lock — a concurrent resync with `respawn_if_done=true`
            // (e.g. the WS `ControllerResyncCommand`/`TermResyncHandler`
            // path, which always revives a `STATUS_DONE` controller in
            // place) can respawn THIS SAME shell in the gap between that
            // read and this clear. Nulling the pointer unconditionally
            // would then wipe a pointer that's valid again, orphaning a
            // live, correctly-pointed shell — exactly the one-shell-per-
            // pane invariant this PR exists to protect. Re-read the parent
            // fresh and only clear if its pointer still names `id`, the
            // same compare-before-clear pattern this file already uses a
            // few hundred lines down for the identical class of race (see
            // the `parent.meta.get(META_KEY_SHELL_SUBBLOCK_ID) ==
            // Some(child_id.as_str())` guard in the spawn-failure rollback
            // path). Not fully transactional (same as that existing
            // pattern) — a failed/skipped clear just costs one more round
            // trip through the fallback chain, not correctness.
            clear_shell_pointer_if(state, agent_block_id, id);
            tracing::info!(
                block_id = %id,
                parent_id = %agent_block_id,
                "ptyshell.create: pane's shell already exited, not respawning — falling through to a fresh shell"
            );
            return None;
        }
        return Some(
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": format!("ptyshell.create: resync existing shell: {e}") })),
            )
                .into_response(),
        );
    }
    // A shell that started or changed connection between the check above and
    // the resync (a concurrent caller) is still not handed out on the wrong one.
    if let Some(conflict) = connection_conflict(&block, connection) {
        return Some(conflict);
    }
    answer_conpty_handshake_if_seen(state, id, baseline_len).await;
    tracing::info!(block_id = %id, parent_id = %agent_block_id, "ptyshell.create: reused");
    Some((StatusCode::OK, Json(PtyShellCreateResponse { shell_id: id.to_string() })).into_response())
}

/// Clear the agent block's shell pointer, if it still names `id` (see the
/// exited-shell branch of `try_attach_to_existing_shell` for why it is
/// compare-before-clear), so the next create claims a fresh shell.
fn clear_shell_pointer_if(state: &AppState, agent_block_id: &str, id: &str) {
    if let Ok(mut parent) = state.mstore.must_get::<crate::backend::obj::Block>(agent_block_id) {
        if parent.meta.get(META_KEY_SHELL_SUBBLOCK_ID).and_then(|v| v.as_str()) == Some(id) {
            parent.meta.insert(META_KEY_SHELL_SUBBLOCK_ID.to_string(), serde_json::Value::Null);
            if state.mstore.update(&mut parent).is_ok() {
                state.event_bus.broadcast_event(&crate::backend::eventbus::WSEventType {
                    eventtype: "waveobj:update".to_string(),
                    oref: format!("block:{agent_block_id}"),
                    data: Some(serde_json::to_value(&crate::backend::obj::MuxObjUpdate {
                        updatetype: "update".into(),
                        otype: "block".into(),
                        oid: agent_block_id.to_string(),
                        obj: Some(crate::backend::obj::mux_obj_to_value(&parent)),
                    }).unwrap_or_default()),
                });
            }
        }
    }
}

/// 409 when the pane's running shell is on another connection than `wanted`.
/// PtyShellStop releases the keyboard lock and leaves the shell running, so
/// ending the shell is the way to switch.
fn connection_conflict(
    shell_block: &crate::backend::obj::Block,
    wanted: &str,
) -> Option<axum::response::Response> {
    let running_on = crate::backend::obj::meta_get_string(
        &shell_block.meta,
        blockcontroller::META_KEY_CONNECTION,
        "local",
    );
    if crate::backend::remote::conn::same_connection(&running_on, wanted) {
        return None;
    }
    let error = format!(
        "this pane's shell is running on '{running_on}', not '{wanted}'. \
         End it with PtyShellInput(text: \"exit\\r\") and call PtyShell again, \
         or call PtyShell with connection '{running_on}' to use it"
    );
    Some((StatusCode::CONFLICT, Json(json!({ "error": error }))).into_response())
}
