// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The `impl Controller for ShellController` block: start/stop/send_input/
//! get_runtime_status. Kept whole in one file (a trait impl cannot be split
//! across modules); it orchestrates the helpers extracted into the sibling
//! `pty`, `file_ops`, and `translation` modules.

use std::io::Read as _;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

#[cfg(unix)]
use libc;

use portable_pty::{native_pty_system, CommandBuilder, PtySize};
use tokio::sync::mpsc;

use super::super::{
    BlockControllerRuntimeStatus, BlockInputUnion, Controller, META_KEY_CMD_ENV, STATUS_DONE,
    STATUS_RUNNING,
};
#[cfg(unix)]
use super::controller::KILL_GRACE_SECS;
use super::controller::{ShellController, SHELL_INPUT_CH_SIZE};
use super::file_ops::handle_append_block_file;
use super::pty::{
    detect_local_shell_path_windows, PTY_CHANNEL_CAPACITY, PTY_COALESCE_MAX_BYTES,
    PTY_COALESCE_WINDOW, PTY_READ_BUF_SIZE,
};
use super::translation::accumulate_and_translate;
use crate::backend::obj::{self, MetaMapType};
use crate::backend::shellexec::ShellProc;
use crate::backend::wps;

/// One PTY read's worth of already-OSC-cleaned output, on its way from the
/// blocking read loop to the coalescing flusher (`start()`, below). See
/// docs/reports/REPORT_RENDERER_CPU_UNBATCHED_PTY_OUTPUT_2026_09_11.md.
struct PtyReadChunk {
    data: Vec<u8>,
    osc_events: Vec<crate::backend::osc_extractor::OscEvent>,
}

/// Do the actual per-batch work the read loop used to do per-read: write
/// through to the FileStore, broadcast the raw bytes, publish any OSC
/// activity, and (for agent panes) feed the JSON-stream translator. Called
/// once per coalesced batch instead of once per (up to 4KB) PTY read.
#[allow(clippy::too_many_arguments)]
fn flush_pty_batch(
    broker: &wps::Broker,
    block_id: &str,
    data: &[u8],
    osc_events: &[crate::backend::osc_extractor::OscEvent],
    filestore: Option<&Arc<crate::backend::storage::filestore::FileStore>>,
    line_buf: &mut Vec<u8>,
    translator: Option<&mut crate::agents::translator::claude::ClaudeTranslator>,
) {
    if data.is_empty() && osc_events.is_empty() {
        return;
    }

    handle_append_block_file(
        broker,
        block_id,
        "term",
        data,
        // Write-through so scrollback survives a reconnect
        // (SPEC_TERMINAL_SCROLLBACK_PERSISTENCE_2026_07_23.md
        // §2.1) — was `None` (raw PTY bytes discarded after
        // the live broadcast), which meant every `view:"term"`
        // pane (standalone Terminal panes and the agent-shell
        // drawer alike) lost all output on remount.
        filestore,
        None, // not an agent output stream; no global mirror
    );

    for ev in osc_events {
        wps::publish_block_activity(broker, block_id, &ev.payload);
    }

    if let Some(t) = translator {
        accumulate_and_translate(broker, block_id, line_buf, data, t);
    }
}

/// Drain `rx`, coalescing chunks into batches — waiting up to
/// `PTY_COALESCE_WINDOW` after the first chunk of a new batch for more to
/// arrive, or until `PTY_COALESCE_MAX_BYTES`, whichever comes first — and
/// flushing each batch through [`flush_pty_batch`]. Runs until `rx` closes
/// (the PTY read loop's sender dropped, i.e. the PTY hit EOF or an error),
/// flushing any final partial batch before returning.
///
/// A standalone fn rather than inlined into its `tokio::spawn` call site so
/// it can be driven directly in a test with a synthetic channel and no real
/// PTY — see `pty_output_flusher_tests` below.
async fn run_pty_output_flusher(
    mut rx: mpsc::Receiver<PtyReadChunk>,
    broker: Option<Arc<wps::Broker>>,
    block_id: String,
    filestore: Option<Arc<crate::backend::storage::filestore::FileStore>>,
    is_agent: bool,
) {
    let Some(broker) = broker else {
        // No broker means the read loop's sender never had anything worth
        // sending (see its own `broker_read.is_some()` gate) — nothing to
        // drain or flush.
        return;
    };
    // Phase 1.5 PR 1 (additive): if this is an agent pane, also try to
    // interpret stdout as Claude Code stream-json line-by-line, feeding
    // successful parses through ClaudeTranslator and emitting AgentEvents
    // on a new WPS scope `agent_event:<block_id>`. The existing raw-chunk
    // path stays byte-equal — interactive panes (which don't emit JSON) see
    // no behavior change because every line fails the JSON parse and is
    // silently dropped. The future stream-json-mode pane and the drone
    // inspector (issue #830 / Phase 1.5 PR 3) will be the first real
    // consumers.
    let mut translator: Option<crate::agents::translator::claude::ClaudeTranslator> = if is_agent
    {
        Some(crate::agents::translator::claude::ClaudeTranslator::new())
    } else {
        None
    };
    // Per-block line-buffer (raw bytes — see `extract_agent_events`). Capped
    // to AGENT_LINE_BUFFER_CAP so a producer that never emits a newline
    // can't grow the buffer unboundedly.
    let mut line_buf: Vec<u8> = Vec::new();

    loop {
        let Some(first) = rx.recv().await else {
            break; // channel closed, nothing pending
        };
        let mut batch = first.data;
        let mut batch_osc = first.osc_events;
        let deadline = tokio::time::Instant::now() + PTY_COALESCE_WINDOW;

        // Opportunistically drain more already-queued (or about-to-arrive)
        // chunks into the same batch, bounded by whichever limit hits first.
        let closed_mid_batch = loop {
            if batch.len() >= PTY_COALESCE_MAX_BYTES {
                break false;
            }
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                break false;
            }
            match tokio::time::timeout(remaining, rx.recv()).await {
                Ok(Some(next)) => {
                    batch.extend_from_slice(&next.data);
                    batch_osc.extend(next.osc_events);
                }
                Ok(None) => break true, // channel closed mid-accumulation
                Err(_elapsed) => break false, // coalescing window elapsed
            }
        };

        // spawn_blocking, not block_in_place (reagentx P2 on PR #3206,
        // correcting the round before it): flush_pty_batch does blocking
        // SQLite I/O and takes std::sync::Mutex locks, so it must not run
        // on a Tokio async worker directly — this closure is inside a
        // plain `tokio::spawn`, not `spawn_blocking`. The first fix for
        // that reached for `block_in_place` specifically to avoid moving
        // `&mut line_buf`/`translator.as_mut()` across a 'static+Send
        // boundary — but `block_in_place` gets its own replacement worker
        // via THE SAME shared `spawn_blocking` pool internally (verified
        // directly against the vendored tokio 1.52.3 source,
        // `runtime/scheduler/multi_thread/worker.rs`'s `block_in_place`:
        // `runtime::spawn_blocking(move || run(worker))`), so per flush it
        // draws on that shared pool TWICE — once for the replacement
        // worker, once implicitly for the original thread's own blocking
        // — for work `spawn_blocking` alone would cover with one. Tokio's
        // own docs single out `block_in_place`'s one real advantage
        // (protecting OTHER concurrent work in the SAME task, e.g. across
        // a `join!`) as the reason to prefer it over `spawn_blocking` —
        // this task does nothing else concurrently, so that advantage
        // never applies here, and `spawn_blocking` is strictly the better
        // fit. The ownership problem is solved properly instead: `batch`/
        // `batch_osc` are freshly built each iteration (cheap to move),
        // and `line_buf`/`translator` — the only state the flush actually
        // MUTATES — are moved into the closure and handed back out via the
        // `JoinHandle`'s return value, rather than trying to keep them
        // borrowed across the call.
        let broker_owned = broker.clone();
        let block_id_owned = block_id.clone();
        let filestore_owned = filestore.clone();
        let (new_line_buf, new_translator) = tokio::task::spawn_blocking(move || {
            flush_pty_batch(
                &broker_owned,
                &block_id_owned,
                &batch,
                &batch_osc,
                filestore_owned.as_ref(),
                &mut line_buf,
                translator.as_mut(),
            );
            (line_buf, translator)
        })
        .await
        .expect("PTY output flush task panicked");
        line_buf = new_line_buf;
        translator = new_translator;

        if closed_mid_batch {
            break;
        }
    }
}

/// Resolve the effective AGENTMUX_AGENT_ID for jekt auto-registration from a
/// block's own spawn metadata — both the canonical key and the legacy
/// WAVEMUX_AGENT_ID alias, checked in the SAME block-scoped `cmd:env` map.
/// Deliberately does NOT fall back to `std::env::var` for either key: that
/// reads THIS SRV PROCESS's own environment, shared by every block/pane
/// this instance spawns — reintroducing exactly the same collision as the
/// global-settings fallback below (reagentx P1, round 2: an earlier version
/// of this fn read `std::env::var("WAVEMUX_AGENT_ID")` here, which is process-
/// global env, not per-pane, despite this file's own line ~404 comment
/// already correctly describing WAVEMUX_AGENT_ID as "process-global env
/// state" in a different context — the two were inconsistent). Nor does it
/// fall back to the global settings' cmd_env.AGENTMUX_AGENT_ID: that value
/// is shared across every pane in the instance/channel, so a pane spawned
/// without its own explicit per-block ID would silently inherit whatever
/// another pane happened to configure there — and register_agent_with_nonce
/// unconditionally evicts whoever previously held that agent_key, so the
/// second such pane to register silently steals the first's jekt identity
/// (same-host cross-instance jekt misdelivery). See the child-env injection
/// loop below (`settings.cmd_env` iteration) for the matching fix that keeps
/// this global value out of the pane's OWN environment too — otherwise a
/// pane could still pick it up via OSC 16162 and re-register through the
/// frontend's `/agentmux/reactive/register` path instead (reagentx P1,
/// round 2), even with THIS function never reading it directly. A pane with
/// no block-scoped identity is simply not jekt-registered, matching
/// persistent.rs's muxbus_agent_id_from_env, which never had either fallback.
fn resolve_agent_id_for_jekt(block_meta: &MetaMapType) -> Option<String> {
    let cmd_env = block_meta.get(META_KEY_CMD_ENV).and_then(|m| m.as_object());
    for key in ["AGENTMUX_AGENT_ID", "WAVEMUX_AGENT_ID"] {
        if let Some(v) = cmd_env.and_then(|obj| obj.get(key)).and_then(|v| v.as_str()) {
            let trimmed = v.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
    }
    None
}

/// Whether `key` may be injected into a pane's actual OS environment from
/// the global settings' `cmd_env` defaults (`start()`'s child-env-injection
/// loop, lowest priority — the block's own `cmd:env` override, highest
/// priority, is never filtered by this and can always set either key
/// explicitly). Identity keys are excluded (reagentx P1, round 2 on #2694):
/// without this, a global cmd_env.AGENTMUX_AGENT_ID setting would still land
/// in every pane's real environment even though `resolve_agent_id_for_jekt`
/// never reads it for registration — shell integration echoes an injected
/// AGENTMUX_AGENT_ID back via OSC 16162, and the frontend's own
/// `POST /agentmux/reactive/register` path (triggered by that OSC) would
/// pick it up and register the pane under the shared value anyway, silently
/// reopening the same-host identity collision this PR removes from the
/// backend auto-register path specifically.
fn is_global_cmd_env_injectable(key: &str) -> bool {
    !matches!(key, "AGENTMUX_AGENT_ID" | "WAVEMUX_AGENT_ID")
}

impl Controller for ShellController {
    fn start(
        &self,
        block_meta: MetaMapType,
        rt_opts: Option<serde_json::Value>,
        force: bool,
    ) -> Result<(), String> {
        let cmd_str_preview = Self::get_cmd_str(&block_meta);
        let interactive_preview = Self::is_interactive(&block_meta);
        tracing::info!(
            block_id = %self.block_id,
            controller = %self.controller_type,
            cmd = %cmd_str_preview,
            interactive = interactive_preview,
            force = force,
            "block start requested"
        );

        // Check if we should run
        if !force && !Self::should_run_on_start(&block_meta) {
            tracing::info!(block_id = %self.block_id, "skipping start: run_on_start is false");
            return Ok(());
        }

        // Try to acquire run lock
        if !self.try_lock_run() {
            return Err("controller is already running".to_string());
        }

        // Get connection info
        let conn_name = Self::get_conn_name(&block_meta);

        // Update status to running.
        {
            let mut inner = self.inner.lock().unwrap();
            Self::set_status(&mut inner, STATUS_RUNNING);
            inner.conn_name = conn_name.clone();
        }

        // Create input channel. Unbounded: input must never be silently
        // dropped on burst (the paste-truncation bug). See SHELL_INPUT_CH_SIZE.
        let (input_tx, input_rx) = mpsc::unbounded_channel();
        {
            let mut inner = self.inner.lock().unwrap();
            inner.input_tx = Some(input_tx);
            inner.input_seq_next = 0;
            inner.input_seq_buf.clear();
        }

        // Publish "running" only AFTER `input_tx` exists, so the event is a
        // truthful readiness signal: a frontend that resizes the PTY the instant
        // it sees "running" will not hit `send_input`'s "controller is not
        // running" guard (which fires while `input_tx` is None). The channel is
        // unbounded, so a resize enqueued before the input task spawns is
        // buffered, not lost.
        // See docs/analysis/AGENT_PANE_PTY_RESIZE_RACE_2026_06_16.md.
        self.publish_status();

        // Check if we have a conn_factory (test/mock path)
        let has_factory = self.conn_factory.lock().unwrap().is_some();

        if has_factory {
            // Mock path: use ConnInterface factory (synchronous, for tests)
            let conn_result = {
                let factory = self.conn_factory.lock().unwrap();
                factory.as_ref().unwrap()(&conn_name, &block_meta)
            };

            let mut conn = match conn_result {
                Ok(c) => c,
                Err(e) => {
                    let mut inner = self.inner.lock().unwrap();
                    Self::set_status(&mut inner, STATUS_DONE);
                    inner.proc_exit_code = -1;
                    inner.input_tx = None;
                    self.unlock_run();
                    return Err(format!("failed to create connection: {e}"));
                }
            };

            if let Err(e) = conn.start() {
                let mut inner = self.inner.lock().unwrap();
                Self::set_status(&mut inner, STATUS_DONE);
                inner.proc_exit_code = -1;
                inner.input_tx = None;
                self.unlock_run();
                return Err(format!("failed to start process: {e}"));
            }

            let mut shell_proc = ShellProc::new(conn_name, conn);
            let _done_rx = shell_proc.take_done_rx();
            let exit_code = shell_proc.wait_and_signal();

            {
                let mut inner = self.inner.lock().unwrap();
                inner.proc_exit_code = exit_code;
                Self::set_status(&mut inner, STATUS_DONE);
                inner.input_tx = None;
            }
            self.publish_status();
            self.unlock_run();
            return Ok(());
        }

        // Real PTY path.
        //
        // Open the PTY at the size the frontend computed from the pane (passed
        // as `rtopts.termsize` on the resync), so the agent CLI and its tools
        // wrap correctly from the very first byte. The earlier code always
        // opened at a fixed 200 cols and relied on a post-spawn resize RPC to
        // correct it — but that RPC races controller startup and could fail
        // outright, leaving output wrapped at 200 all session. Seeding the size
        // here removes that race; `pty_size_from_rt_opts` falls back to the
        // historical 25x200 when no size was supplied (e.g. programmatic
        // spawns). See docs/analysis/AGENT_PANE_PTY_RESIZE_RACE_2026_06_16.md.
        let pty_system = native_pty_system();
        let pty_size = Self::pty_size_from_rt_opts(&rt_opts);

        let pair = pty_system.openpty(pty_size).map_err(|e| {
            tracing::error!(block_id = %self.block_id, error = %e, "failed to open PTY");
            let mut inner = self.inner.lock().unwrap();
            Self::set_status(&mut inner, STATUS_DONE);
            inner.proc_exit_code = -1;
            inner.input_tx = None;
            self.unlock_run();
            format!("failed to open PTY: {e}")
        })?;
        tracing::info!(block_id = %self.block_id, rows = pty_size.rows, cols = pty_size.cols, "PTY opened");

        // Determine shell command
        let cmd_str = Self::get_cmd_str(&block_meta);
        let cmd_args = Self::get_cmd_args(&block_meta);
        let interactive = Self::is_interactive(&block_meta);

        // Resolve effective AGENTMUX_AGENT_ID for jekt auto-registration.
        // See resolve_agent_id_for_jekt's doc comment for why this
        // deliberately does not fall back to global settings.
        let agent_id_for_jekt: Option<String> = resolve_agent_id_for_jekt(&block_meta);

        // Detect agent pane: cmd contains a known agent CLI or has AGENTMUX_AGENT_ID set.
        // Computed here (before spawn) rather than after so the commit-aware admission
        // gate below can use it; also reused post-spawn to set `inner.is_agent_pane`.
        let is_agent = agent_id_for_jekt.is_some()
            || cmd_str.to_lowercase().contains("claude")
            || cmd_str.to_lowercase().contains("codex")
            || cmd_str.to_lowercase().contains("gemini")
            || cmd_str.to_lowercase().contains("qwen");

        // Pillar 3 — commit-aware admission control, extended to the interactive
        // agent pane. The drone Agent block's one-shot spawn (`agents::runner::
        // run_agent`) already refuses to start another `claude.exe` when system
        // commit headroom is below the reserve, to avoid tipping the box into a
        // Chromium OOM abort (0xE0000008). That gate covered only the drone path;
        // this interactive PTY spawn is the OTHER (likely more common) place a
        // fresh agent CLI process gets started, and previously had no protection
        // at all. Reuse the same pure decision + reserve lookup here, gated on
        // `is_agent` so plain shell/cmd panes are unaffected — only a new
        // claude/codex/gemini/qwen process is refused under commit pressure.
        // See SPEC_WIN10_PAGEFILE_OOM_CRASH_2026_06_29.md, PR #1853.
        if is_agent {
            if let Err(e) = crate::agents::runner::admit_spawn(
                crate::backend::sysinfo::available_commit_gb(),
                crate::agents::runner::agent_commit_reserve_gb(),
            ) {
                tracing::warn!(
                    block_id = %self.block_id,
                    cmd = %cmd_str,
                    error = %e,
                    "admission gate: refusing interactive agent spawn under commit pressure"
                );
                let mut inner = self.inner.lock().unwrap();
                Self::set_status(&mut inner, STATUS_DONE);
                inner.proc_exit_code = -1;
                inner.input_tx = None;
                self.unlock_run();
                return Err(format!(
                    "memory full — not enough memory to start a new agent right now, free up memory and try again ({e})"
                ));
            }
        }

        let mut cmd = if !cmd_str.is_empty() && (!cmd_args.is_empty() || interactive) {
            // Direct spawn: cmd:args provided or cmd:interactive set.
            // Spawn the CLI directly (no sh -c wrapper) so args are passed correctly.
            tracing::info!(block_id = %self.block_id, cmd = %cmd_str, args = ?cmd_args, "direct spawn path");
            let mut c = CommandBuilder::new(&cmd_str);
            if !cmd_args.is_empty() {
                let arg_refs: Vec<&str> = cmd_args.iter().map(|s| s.as_str()).collect();
                c.args(arg_refs);
            }
            c
        } else if !cmd_str.is_empty() {
            // "cmd" controller: run a specific command string via shell wrapper
            tracing::info!(block_id = %self.block_id, cmd = %cmd_str, "shell-wrapped spawn path");
            if cfg!(windows) {
                let mut c = CommandBuilder::new("cmd.exe");
                c.args(["/C", &cmd_str]);
                c
            } else {
                let mut c = CommandBuilder::new("/bin/sh");
                c.args(["-c", &cmd_str]);
                c
            }
        } else {
            // "shell" controller: interactive shell with AgentMux integration
            // On Windows: prefer pwsh (PowerShell 7), fall back to powershell.exe (5.x), then cmd.exe
            let shell_path = if cfg!(windows) {
                detect_local_shell_path_windows()
            } else {
                std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".to_string())
            };

            let shell_type = crate::backend::shellintegration::detect_shell_type(&shell_path);

            // Deploy shell integration scripts to ~/.agentmux/ (the user's home-based
            // data dir) instead of AGENTMUX_DATA_HOME.  MSIX packages virtualise writes
            // to %LocalAppData%, so files written by the packaged backend aren't visible
            // to child processes (pwsh, bash, etc.) spawned via ConPTY.  The home dir is
            // never virtualised, so the scripts are always reachable at their literal path.
            let shell_home = crate::backend::base::get_home_dir().join(".agentmux");
            crate::backend::shellintegration::deploy_scripts(&shell_home);

            tracing::info!(block_id = %self.block_id, shell = %shell_path, shell_type = ?shell_type, "interactive shell path");

            let mut c = CommandBuilder::new(&shell_path);

            // Apply shell-specific startup args (--rcfile, -File, etc.)
            if let Some(startup) = crate::backend::shellintegration::get_shell_startup(shell_type, &shell_home) {
                for arg in &startup.extra_args {
                    c.arg(arg);
                }
                for (k, v) in &startup.env_vars {
                    c.env(k, v);
                }
            }

            // Inject terminal capability env vars into the PTY environment.
            // ConPTY on Windows fully supports VT/ANSI sequences, so set TERM
            // on all platforms. Without this, CLI tools (e.g. Claude Code) use
            // different Unicode width tables, causing ANSI color offset on Windows.
            c.env("TERM", "xterm-256color");
            c.env("COLORTERM", "truecolor");
            c.env("TERM_PROGRAM", "agentmux");
            c.env("AGENTMUX_BLOCKID", &self.block_id);
            c.env("AGENTMUX_TABID", &self.tab_id);
            c.env("AGENTMUX_VERSION", env!("CARGO_PKG_VERSION"));

            // Inject log directory so agents can find logs without guessing.
            // Always ~/.agentmux/logs/ — matches both host and sidecar.
            let log_dir = dirs::home_dir()
                .unwrap_or_default()
                .join(".agentmux")
                .join("logs");
            c.env("AGENTMUX_LOG_DIR", log_dir.to_string_lossy().as_ref());

            // Propagate local backend URL so the muxbus client (agentbus-client package) prefers local PTY delivery.
            // Set by main.rs after binding; absent in test/mock contexts (graceful no-op).
            if let Ok(local_url) = std::env::var("AGENTMUX_LOCAL_URL") {
                c.env("AGENTMUX_LOCAL_URL", &local_url);
            }

            // AGENTMUX is a plain "1" sentinel — wsh has been retired.
            // Shell integrations check for the presence of AGENTMUX but no
            // longer prepend a path to $PATH based on its value.
            // See docs/specs/archive/SPEC_RETIRE_WSH_2026_04_12.md.
            c.env("AGENTMUX", "1");

            // Wire AgentMux-managed tool dirs into the agent's PATH.
            //
            // Two stores, two precedence rules:
            //
            //   • Bundled store (`<exe_dir>/tools/bin`) ships the app's OWN
            //     version-locked binaries — notably `agentmux-bashwrap`, the
            //     streaming hook that MUST match the running build. It is
            //     PREPENDED so it wins over any stale copy elsewhere on the
            //     system PATH. A stale system-PATH `agentmux-bashwrap` (from a
            //     leftover portable) silently shadowed the fixed one and
            //     reintroduced the exit-130 bug — see
            //     docs/retro/RETRO_BASHWRAP_STALE_BUNDLE_2026_06_13.md. The
            //     bundled jq/rg winning too is intended: agents get the app's
            //     curated, deterministic tool versions regardless of host.
            //
            //   • User-managed store (`~/.agentmux/tools/bin`) holds tools the
            //     user installed via /tools. APPENDED so the user's own system
            //     PATH still wins for those.
            {
                let sep = if cfg!(windows) { ";" } else { ":" };
                let current_path = std::env::var("PATH").unwrap_or_default();
                let mut prepend: Vec<String> = Vec::new();
                let mut append: Vec<String> = Vec::new();

                // Bundled store — prepended (app-owned, version-locked).
                if let Some(bundled_bin) = crate::backend::tool_store::bundled_tools_dir() {
                    if bundled_bin.exists() {
                        // Guardrail: log which agentmux-bashwrap the agent will
                        // actually run. A stale system-PATH copy silently
                        // shadowing the bundled one is exactly the exit-130
                        // trap (RETRO_BASHWRAP_STALE_BUNDLE_2026_06_13.md); this
                        // one line makes "which binary?" answerable at a glance
                        // (cross-check the version with `agentmux-bashwrap
                        // --version`).
                        let bashwrap_exe = if cfg!(windows) {
                            "agentmux-bashwrap.exe"
                        } else {
                            "agentmux-bashwrap"
                        };
                        let bw = bundled_bin.join(bashwrap_exe);
                        if bw.exists() {
                            tracing::info!(
                                target: "agent-tools",
                                path = %bw.display(),
                                "agent bashwrap: bundled (version-locked, prepended to PATH)"
                            );
                        } else {
                            tracing::warn!(
                                target: "agent-tools",
                                dir = %bundled_bin.display(),
                                "agent bashwrap: bundled store present but agentmux-bashwrap MISSING — agent will resolve via system PATH (risk of a stale copy; see RETRO_BASHWRAP_STALE_BUNDLE_2026_06_13.md)"
                            );
                        }
                        prepend.push(bundled_bin.to_string_lossy().into_owned());
                    } else {
                        tracing::warn!(
                            target: "agent-tools",
                            "agent bashwrap: no bundled tools dir — agent will resolve agentmux-bashwrap via system PATH (risk of a stale copy; see RETRO_BASHWRAP_STALE_BUNDLE_2026_06_13.md)"
                        );
                    }
                }

                // User-managed store — appended (system PATH still wins).
                if let Some(user_bin) = crate::backend::tool_store::user_tools_dir() {
                    if user_bin.exists() {
                        append.push(user_bin.to_string_lossy().into_owned());
                    }
                }

                if !prepend.is_empty() || !append.is_empty() {
                    let mut parts = prepend;
                    if !current_path.is_empty() {
                        parts.push(current_path);
                    }
                    parts.extend(append);
                    c.env("PATH", parts.join(sep));
                }
            }

            // Inject cmd:env from wconfig settings and block metadata.
            // Track whether AGENTMUX_AGENT_ID is explicitly set so we know
            // whether to apply the backward-compat WAVEMUX bridge.
            let mut has_agent_id = false;

            // Settings (global defaults, lowest priority)
            let config = crate::backend::wconfig::ConfigState::with_config(
                crate::backend::wconfig::build_default_config(),
            );
            let settings = config.get_settings();
            for (k, v) in &settings.cmd_env {
                if !is_global_cmd_env_injectable(k) {
                    continue;
                }
                let expanded = crate::backend::base::expand_home_dir_safe(v);
                c.env(k, expanded.to_string_lossy().as_ref());
            }

            // Block metadata (per-block overrides, highest priority)
            if let Some(env_map) = block_meta.get(META_KEY_CMD_ENV) {
                if let Some(obj) = env_map.as_object() {
                    for (k, v) in obj {
                        if let Some(val) = v.as_str() {
                            if k == "AGENTMUX_AGENT_ID" {
                                has_agent_id = true;
                            }
                            let expanded = crate::backend::base::expand_home_dir_safe(val);
                            c.env(k, expanded.to_string_lossy().as_ref());
                        }
                    }
                }
            }

            // Strip host-inherited agent identity unless explicitly configured
            // in settings.cmd_env or block cmd:env metadata.
            // This also supersedes the old WAVEMUX backward-compat bridge —
            // both AGENTMUX_* and WAVEMUX_* vars are removed so new panes
            // start as plain "Terminal".
            if !has_agent_id {
                c.env_remove("AGENTMUX_AGENT_ID");
                c.env_remove("AGENTMUX_AGENT_COLOR");
                c.env_remove("AGENTMUX_AGENT_TEXT_COLOR");
                c.env_remove("WAVEMUX_AGENT_ID");
                c.env_remove("WAVEMUX_AGENT_COLOR");
            }

            c
        };

        // Set working directory if specified
        let cwd = obj::meta_get_string(&block_meta, super::super::META_KEY_CMD_CWD, "");
        if !cwd.is_empty() {
            cmd.cwd(&cwd);
        }

        let mut child = pair.slave.spawn_command(cmd).map_err(|e| {
            tracing::error!(block_id = %self.block_id, error = %e, cmd = %cmd_str, "spawn failed");
            let mut inner = self.inner.lock().unwrap();
            Self::set_status(&mut inner, STATUS_DONE);
            inner.proc_exit_code = -1;
            inner.input_tx = None;
            self.unlock_run();
            format!("failed to spawn command: {e}")
        })?;
        tracing::info!(block_id = %self.block_id, "process spawned successfully");

        // Register PID and record spawn metadata.
        let spawn_ts_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);
        {
            let mut inner = self.inner.lock().unwrap();
            if let Some(pid) = child.process_id() {
                super::super::pidregistry::register(&self.block_id, pid);
                crate::backend::process_tracker::registry::track_spawned(&self.block_id, pid);
                inner.child_pid = Some(pid);
            }
            inner.spawn_ts_ms = Some(spawn_ts_ms);
            inner.is_agent_pane = is_agent;
            inner.agent_id = agent_id_for_jekt.clone();
        }

        // Auto-register with jekt if AGENTMUX_AGENT_ID was set in the block env.
        // This maps agent_id → block_id in the ReactiveHandler so jekt can deliver
        // messages directly to this PTY without a separate /agentmux/reactive/register call.
        if let Some(ref agent_id) = agent_id_for_jekt {
            match crate::backend::reactive::get_global_handler()
                .register_agent(agent_id, &self.block_id, Some(&self.tab_id))
            {
                Ok(()) => {
                    tracing::info!(
                        block_id = %self.block_id,
                        agent_id = %agent_id,
                        "jekt: auto-registered"
                    );
                    // Also write to cross-instance file registry, and its
                    // host-global sibling (Tier 2b, issue #1916) — this
                    // auto-register path bypasses the HTTP register handler
                    // entirely, so it needs its own mirror call too.
                    if let Ok(local_url) = std::env::var("AGENTMUX_LOCAL_URL") {
                        let data_dir = crate::backend::base::get_wave_data_dir();
                        crate::backend::reactive::registry::write(
                            &data_dir,
                            agent_id,
                            &local_url,
                            &self.block_id,
                        );
                        crate::backend::reactive::registry::write_shared_from_env(
                            agent_id,
                            &local_url,
                            &self.block_id,
                        );
                    }
                }
                Err(e) => tracing::warn!(
                    block_id = %self.block_id,
                    agent_id = %agent_id,
                    error = %e,
                    "jekt: auto-register failed"
                ),
            }
        }
        tracing::info!(
            block_id = %self.block_id,
            wstore_present = self.wstore.is_some(),
            event_bus_present = self.event_bus.is_some(),
            "[dnd-debug] pre-seed state after spawn"
        );

        // Seed cmd:cwd in block meta immediately after spawn so drag-and-drop works
        // before the shell emits its first OSC 7 (or for shells without integration).
        if let Some(ref store) = self.wstore {
            let effective_cwd = if !cwd.is_empty() {
                cwd.clone()
            } else {
                std::env::current_dir()
                    .map(|p| p.to_string_lossy().to_string())
                    .unwrap_or_default()
            };
            tracing::debug!(block_id = %self.block_id, cwd = %effective_cwd, "seeding cmd:cwd");
            if !effective_cwd.is_empty() {
                let oref_str = format!("block:{}", self.block_id);
                let mut meta_update = MetaMapType::new();
                meta_update.insert(
                    super::super::META_KEY_CMD_CWD.to_string(),
                    serde_json::Value::String(effective_cwd),
                );
                // Only set if not already populated — don't clobber a restored session CWD
                match store.must_get::<crate::backend::obj::Block>(&self.block_id) {
                    Ok(block) if obj::meta_get_string(&block.meta, super::super::META_KEY_CMD_CWD, "").is_empty() => {
                        match crate::server::service::update_object_meta(store, &oref_str, &meta_update) {
                            Ok(()) => {
                                // Re-read updated block and broadcast obj:update so the
                                // frontend Jotai atom refreshes (update_object_meta only writes
                                // to SQLite — it does NOT send a WebSocket event on its own).
                                if let Ok(updated_block) = store.must_get::<crate::backend::obj::Block>(&self.block_id) {
                                    if let Some(ref event_bus) = self.event_bus {
                                        let update_data = serde_json::to_value(&obj::WaveObjUpdate {
                                            updatetype: "update".into(),
                                            otype: "block".into(),
                                            oid: self.block_id.clone(),
                                            obj: Some(obj::wave_obj_to_value(&updated_block)),
                                        }).ok();
                                        event_bus.broadcast_event(&crate::backend::eventbus::WSEventType {
                                            eventtype: "waveobj:update".to_string(),
                                            oref: oref_str.clone(),
                                            data: update_data,
                                        });
                                        tracing::info!(block_id = %self.block_id, "cmd:cwd seeded and broadcast to frontend");
                                    } else {
                                        tracing::warn!(block_id = %self.block_id, "cmd:cwd written to store but no event_bus to broadcast — frontend won't update");
                                    }
                                }
                            }
                            Err(e) => {
                                tracing::warn!(block_id = %self.block_id, error = %e, "failed to seed cmd:cwd in store");
                            }
                        }
                    }
                    Ok(_) => {
                        tracing::debug!(block_id = %self.block_id, "cmd:cwd already set, skipping seed");
                    }
                    Err(e) => {
                        tracing::warn!(block_id = %self.block_id, error = %e, "failed to read block for cmd:cwd seed");
                    }
                }
            }
        }

        // Get reader/writer from master
        let reader = pair.master.try_clone_reader().map_err(|e| {
            let _ = child.kill();
            let mut inner = self.inner.lock().unwrap();
            Self::set_status(&mut inner, STATUS_DONE);
            inner.proc_exit_code = -1;
            inner.input_tx = None;
            self.unlock_run();
            format!("failed to clone PTY reader: {e}")
        })?;

        let writer = pair.master.take_writer().map_err(|e| {
            let _ = child.kill();
            let mut inner = self.inner.lock().unwrap();
            Self::set_status(&mut inner, STATUS_DONE);
            inner.proc_exit_code = -1;
            inner.input_tx = None;
            self.unlock_run();
            format!("failed to take PTY writer: {e}")
        })?;

        // Spawn PTY read task (blocking I/O → spawn_blocking) + a companion
        // coalescing flusher (async task). Split in two, connected by an
        // mpsc channel, so the actual OS-level `read()` loop's blocking
        // semantics are untouched (this file supports both Unix and
        // Windows PTYs and that boundary is not something to risk getting
        // subtly wrong per-platform) while the expensive per-chunk work —
        // file write, WebSocket broadcast, OSC/translation — is batched.
        // See docs/reports/REPORT_RENDERER_CPU_UNBATCHED_PTY_OUTPUT_2026_09_11.md.
        let block_id_read = self.block_id.clone();
        let block_id_flush = self.block_id.clone();
        let broker_read = self.broker.clone();
        let broker_flush = self.broker.clone();
        let inner_read = self.inner.clone();
        let filestore_flush = self.filestore.clone();
        let is_agent_read = is_agent;
        let is_agent_flush = is_agent;
        // Bounded (reagentx P1 on PR #3206) — see PTY_CHANNEL_CAPACITY's own
        // doc comment for why: an unbounded channel here would let a
        // sustained fast producer grow queued memory without limit if the
        // flusher ever falls behind the read rate.
        let (pty_tx, pty_rx) = mpsc::channel::<PtyReadChunk>(PTY_CHANNEL_CAPACITY);

        tokio::task::spawn_blocking(move || {
            let mut reader = reader;
            let mut buf = [0u8; PTY_READ_BUF_SIZE];

            // OSC extractor: only for agent panes. Terminal panes forward
            // raw bytes to xterm.js which handles OSC natively — stripping
            // there would suppress the native window-title update. Agent
            // panes don't use xterm.js; OSC bytes in FileStore corrupt the
            // document renderer, so we extract and strip them here. Kept
            // in THIS (per-read) loop rather than the flusher: it is
            // cheap, and its own internal state must see every read in
            // order exactly as they arrive — moving it to the batched side
            // would change nothing about correctness but would mean
            // holding it across an .await, so it stays here for simplicity.
            let mut osc_extractor: Option<crate::backend::osc_extractor::OscExtractor> =
                if is_agent_read {
                    Some(crate::backend::osc_extractor::OscExtractor::new())
                } else {
                    None
                };

            loop {
                match reader.read(&mut buf) {
                    Ok(0) => break, // EOF
                    Ok(n) => {
                        inner_read.lock().unwrap().last_pty_output = Some(Instant::now());
                        if broker_read.is_some() {
                            let raw = &buf[..n];

                            // Extract OSC sequences from agent-pane output.
                            // `cleaned` has OSC bytes removed; `osc_events` carries
                            // any normalised Claude Code title strings found in this chunk.
                            let mut cleaned_storage: Vec<u8> = Vec::new();
                            let mut osc_events: Vec<crate::backend::osc_extractor::OscEvent> = Vec::new();
                            if let Some(ref mut ext) = osc_extractor {
                                let (cleaned, evs) = ext.feed(raw);
                                cleaned_storage = cleaned;
                                osc_events = evs;
                            }
                            let chunk: &[u8] = if osc_extractor.is_some() {
                                &cleaned_storage
                            } else {
                                raw
                            };

                            // `blocking_send`, not `send` — this closure runs
                            // on a `spawn_blocking` thread, not async, and a
                            // bounded channel's plain `send` is itself async.
                            // Blocks THIS (already-dedicated-to-blocking)
                            // thread once the channel is full, which is
                            // exactly the backpressure PTY_CHANNEL_CAPACITY's
                            // doc comment describes — never the async runtime.
                            //
                            // Best-effort beyond that: if the flusher task is
                            // gone (block torn down mid-read), there is
                            // nothing useful to do with this chunk — the PTY
                            // read loop itself must keep draining so the
                            // child process's stdout never backs up.
                            let _ = pty_tx.blocking_send(PtyReadChunk {
                                data: chunk.to_vec(),
                                osc_events,
                            });
                        }
                    }
                    Err(e) => {
                        tracing::debug!("PTY read error for {}: {}", block_id_read, e);
                        break;
                    }
                }
            }
            // `pty_tx` drops here, closing the channel — the flusher below
            // sees `recv()` return `None` once it has drained everything
            // already sent, flushes any final partial batch, and exits.
        });

        // Coalescing flusher: batches chunks the read loop above sends,
        // waiting up to PTY_COALESCE_WINDOW for more to arrive (or until
        // PTY_COALESCE_MAX_BYTES, whichever comes first) before doing the
        // actual file write + broadcast + translation — once per batch
        // instead of once per (up to 4KB) PTY read. A standalone fn (not
        // inlined into the spawn) so it can be driven directly in tests
        // with a synthetic channel, no real PTY required.
        tokio::spawn(run_pty_output_flusher(
            pty_rx,
            broker_flush,
            block_id_flush,
            filestore_flush,
            is_agent_flush,
        ));

        // Spawn input task (routes input channel → PTY writer + resize + signals)
        // Owns writer and master — dropping them closes the PTY, causing child to exit.
        let master = pair.master;
        tokio::spawn(async move {
            let mut writer = writer;
            let mut input_rx = input_rx;
            while let Some(input) = input_rx.recv().await {
                if let Some(data) = input.input_data {
                    use std::io::Write;
                    if let Err(e) = writer.write_all(&data) {
                        tracing::debug!("PTY write error: {}", e);
                        break;
                    }
                }
                if let Some(ref size) = input.term_size {
                    let pty_size = PtySize {
                        rows: size.rows as u16,
                        cols: size.cols as u16,
                        pixel_width: 0,
                        pixel_height: 0,
                    };
                    if let Err(e) = master.resize(pty_size) {
                        tracing::debug!("PTY resize error: {}", e);
                    }
                }
                if input.sig_name.is_some() {
                    // Drop writer + master to close PTY, which terminates the child
                    break;
                }
            }
            // writer and master drop here → PTY closes → child gets EOF/terminates
        });

        // Spawn wait task (monitors process exit)
        let inner_wait = Arc::clone(&self.inner);
        let block_id_wait = self.block_id.clone();
        let agent_id_wait = agent_id_for_jekt.clone();
        let broker_wait = self.broker.clone();
        let run_lock = Arc::clone(&self.run_lock);
        tokio::task::spawn_blocking(move || {
            let mut child = child;

            // Wait for child to exit (blocking)
            let exit_status = child.wait();
            let exit_code = match exit_status {
                Ok(status) => {
                    if status.success() {
                        0
                    } else {
                        // portable-pty ExitStatus doesn't expose raw code on all platforms
                        1
                    }
                }
                Err(e) => {
                    tracing::warn!("wait error for block {}: {}", block_id_wait, e);
                    -1
                }
            };

            tracing::info!(block_id = %block_id_wait, exit_code = exit_code, "process exited");

            // Unregister PID from per-pane metrics
            super::super::pidregistry::unregister(&block_id_wait);

            // Deregister from jekt — removes the agent_id → block_id mapping so
            // subsequent jekt attempts fall back to MessageBus rather than a dead PTY.
            crate::backend::reactive::get_global_handler().unregister_block(&block_id_wait);

            // Also remove from cross-instance file registry and cloud subscriber.
            if let Some(ref agent_id) = agent_id_wait {
                let data_dir = crate::backend::base::get_wave_data_dir();
                crate::backend::reactive::registry::remove(&data_dir, agent_id);
                crate::backend::reactive::registry::remove_shared_from_env(agent_id);
                if let Some(sub) = crate::muxbus::cloud_subscriber::get_global_subscriber() {
                    sub.remove_agent(agent_id);
                }
            }

            // Update inner state
            {
                let mut inner = inner_wait.lock().unwrap();
                inner.proc_exit_code = exit_code;
                ShellController::set_status(&mut inner, STATUS_DONE);
                inner.input_tx = None;
            }

            // Publish done status
            if let Some(ref broker) = broker_wait {
                let status = {
                    let inner = inner_wait.lock().unwrap();
                    BlockControllerRuntimeStatus {
                        blockid: block_id_wait.clone(),
                        version: inner.status_version,
                        shellprocstatus: inner.proc_status.clone(),
                        shellprocconnname: inner.conn_name.clone(),
                        shellprocexitcode: inner.proc_exit_code,
                        spawn_ts_ms: inner.spawn_ts_ms,
                        is_agent_pane: inner.is_agent_pane,
                        turn_active: false,
                    }
                };
                super::super::publish_controller_status(broker, &status);
            }

            // Release run lock
            run_lock.store(false, Ordering::SeqCst);
        });

        // Return immediately — PTY tasks run in background
        Ok(())
    }

    fn stop(&self, _graceful: bool, new_status: &str) -> Result<(), String> {
        // Extract what we need from the lock, release it before any async work.
        #[allow(unused_variables)] // used under #[cfg(unix)] only
        let pid_to_kill = {
            let mut inner = self.inner.lock().unwrap();
            if inner.proc_status == new_status {
                return Ok(());
            }
            let pid = inner.child_pid;
            // Drop the input channel — closes PTY writer → delivers EOF/SIGHUP as
            // belt-and-suspenders in case signal delivery fails on the platform.
            inner.input_tx = None;
            Self::set_status(&mut inner, new_status);
            pid
        };

        // Send SIGTERM to the process group so that child processes spawned by
        // the shell (e.g. `claude --dangerously-skip-permissions` and its subtree)
        // are also signalled. Negative pid targets the whole process group.
        // Schedule SIGKILL after KILL_GRACE_SECS as a backstop for processes
        // that ignore or delay on SIGTERM.
        #[cfg(unix)]
        if let Some(pid) = pid_to_kill {
            // SAFETY: kill() is a well-defined POSIX syscall.
            unsafe { libc::kill(-(pid as libc::pid_t), libc::SIGTERM) };
            tokio::spawn(async move {
                tokio::time::sleep(tokio::time::Duration::from_secs(KILL_GRACE_SECS)).await;
                unsafe { libc::kill(-(pid as libc::pid_t), libc::SIGKILL) };
            });
        }

        // A declared-background descendant (bashwrap, and transitively
        // whatever it PTY-spawned, e.g. `task dev`) deliberately detaches
        // into its OWN session at startup (`setsid()` in bash_wrap.rs's
        // `detach_declared_background_session`), specifically so a
        // session-RESTART's narrower kill (`stop_for_replace`, above)
        // can't reach it via PTY-hangup/session-leader-death SIGHUP. But
        // that same detachment also removes it from THIS process's group
        // — so the group-wide kill just above (intended to reach it on a
        // genuine STOP, e.g. pane close) no longer can either. On
        // non-Windows, nothing else fills that gap:
        // `process_tracker::new_tracker` returns the no-op `StubTracker`
        // there (no real cgroup/pgrp tracker is implemented — see
        // `detach_declared_background_session`'s doc comment), so
        // `delete_controller`'s `registry.remove()` was never doing
        // anything for it either. Without this step, `stop()` (the real,
        // unmodified deletion path — used by `delete_tab`/`delete_block`/
        // `wcore::tab`) would leak it forever on Linux/macOS (reagentx
        // finding, PR #2683). Kill each `Running`, known-pid declared-
        // background task's own (detached) process group explicitly, by
        // pid from the durable registry — the one piece of state that
        // still knows about it once it's escaped this controller's own
        // process tree. Windows is unaffected: Job Object membership
        // isn't process-group-based, so `delete_controller`'s existing
        // whole-job close already reaches it there, unchanged.
        #[cfg(unix)]
        if let Some(store) = &self.wstore {
            match store.background_task_list_for_block(&self.block_id) {
                Ok(tasks) => {
                    for task in tasks {
                        if task.status != crate::backend::storage::background_tasks::BackgroundTaskStatus::Running {
                            continue;
                        }
                        let Some(pid) = task.pid else { continue };
                        let pid = pid as libc::pid_t;
                        // SAFETY: kill() is a well-defined POSIX syscall.
                        unsafe { libc::kill(-pid, libc::SIGTERM) };
                        tokio::spawn(async move {
                            tokio::time::sleep(tokio::time::Duration::from_secs(KILL_GRACE_SECS)).await;
                            unsafe { libc::kill(-pid, libc::SIGKILL) };
                        });
                    }
                }
                Err(e) => {
                    tracing::warn!(
                        block_id = %self.block_id,
                        error = %e,
                        "stop(): failed to list declared-background tasks for cleanup",
                    );
                }
            }
        }

        Ok(())
    }

    fn stop_for_replace(&self, new_status: &str) -> Result<(), String> {
        // Same extraction as stop() — but the kill below targets ONLY this
        // one pid, never the process group/job, so a declared-background
        // descendant (e.g. `task dev`) survives. See
        // docs/specs/SPEC_BACKGROUND_TASK_TEARDOWN_SURVIVAL_2026_08_20.md.
        #[allow(unused_variables)] // used under #[cfg(unix)] only
        let pid_to_kill = {
            let mut inner = self.inner.lock().unwrap();
            if inner.proc_status == new_status {
                return Ok(());
            }
            let pid = inner.child_pid;
            inner.input_tx = None;
            Self::set_status(&mut inner, new_status);
            pid
        };

        let Some(pid) = pid_to_kill else { return Ok(()) };

        // Prefer the tracked kill (Windows: OpenProcess+TerminateProcess,
        // with a membership check against the block's job first — see
        // `process_tracker::registry::kill_pid`). This is expected to
        // succeed in every real production case: `track_spawned` already
        // ran for this exact pid right after this controller's own spawn.
        if let Some(registry) = crate::backend::process_tracker::registry::global() {
            if registry.kill_pid(&self.block_id, pid) {
                return Ok(());
            }
        }

        // Fallback — reached when the registry global isn't set (tests;
        // same "silently skip tracker registration" convention
        // `track_spawned`'s own doc comment describes), OR `kill_pid`
        // itself returned `false` because this pid isn't (yet, or ever)
        // a recognized member of the block's tracker — e.g.
        // `track_spawned`'s own `assign_process` call failed at spawn
        // time (a real, already-logged non-fatal warning path in
        // `registry::track_spawned`). Unlike `stop()`, where a failed
        // direct kill is backstopped by `delete_controller`'s whole-job
        // teardown, `stop_for_replace` deliberately never touches the
        // job — so without a fallback here, that failure mode would leak
        // the old CLI process forever once resync_controller replaces it
        // (reagentx P1, PR #2683). A direct, single-PID (not group,
        // not job) kill on both platforms, so this can't reach a
        // declared-background descendant the way stop()'s `-(pid)` /
        // a whole-job close would.
        //
        // On Unix this is ALSO the primary (not just fallback) path in
        // production today: `process_tracker::new_tracker` currently
        // returns the no-op `StubTracker` on every non-Windows platform
        // (no real cgroup/pgrp tracker is implemented yet, despite the
        // aspirational table in `process_tracker/mod.rs`'s module doc
        // comment), so `kill_pid` unconditionally returns `false` there.
        #[cfg(unix)]
        {
            // SAFETY: kill() is a well-defined POSIX syscall.
            unsafe { libc::kill(pid as libc::pid_t, libc::SIGTERM) };
            tokio::spawn(async move {
                tokio::time::sleep(tokio::time::Duration::from_secs(KILL_GRACE_SECS)).await;
                unsafe { libc::kill(pid as libc::pid_t, libc::SIGKILL) };
            });
        }
        #[cfg(windows)]
        {
            // Mirrors process_tracker::windows::JobObjectTracker::kill_pid's
            // own kill step exactly, minus its job-membership pre-check
            // (which is precisely what already failed to get us here) —
            // a plain OpenProcess(PROCESS_TERMINATE) + TerminateProcess on
            // the raw pid, best-effort (the process may have already
            // exited on its own).
            use windows_sys::Win32::Foundation::CloseHandle;
            use windows_sys::Win32::System::Threading::{OpenProcess, TerminateProcess, PROCESS_TERMINATE};
            unsafe {
                let h = OpenProcess(PROCESS_TERMINATE, 0, pid);
                if !h.is_null() {
                    let ok = TerminateProcess(h, 1);
                    CloseHandle(h);
                    if ok == 0 {
                        tracing::warn!(
                            block_id = %self.block_id,
                            pid,
                            error = %std::io::Error::last_os_error(),
                            "stop_for_replace: fallback TerminateProcess failed"
                        );
                    }
                } else {
                    tracing::warn!(
                        block_id = %self.block_id,
                        pid,
                        error = %std::io::Error::last_os_error(),
                        "stop_for_replace: fallback OpenProcess failed — old CLI process may be leaked"
                    );
                }
            }
        }

        Ok(())
    }

    fn get_runtime_status(&self) -> BlockControllerRuntimeStatus {
        self.get_status_snapshot()
    }

    fn send_input(&self, input: BlockInputUnion, seq: Option<u64>) -> Result<(), String> {
        let mut inner = self.inner.lock().unwrap();
        let tx = match &inner.input_tx {
            Some(tx) => tx.clone(),
            None => return Err("controller is not running".to_string()),
        };
        match seq {
            None => tx.send(input).map_err(|e| format!("send_input: {e}")),
            Some(s) => {
                // Detect session reset: seq==0 means the TermViewModel
                // restarted and its per-block counter is back at zero.
                // (A gap-threshold heuristic was tried and removed — a
                // stale/duplicate packet far behind could falsely trigger
                // a reset and replay old input.)
                if s == 0 && inner.input_seq_next > 0 {
                    tracing::info!(
                        block_id = %self.block_id,
                        prev_next = inner.input_seq_next,
                        new_seq = s,
                        "input seq reset (session reset detected)"
                    );
                    inner.input_seq_next = s;
                    inner.input_seq_buf.clear();
                }

                let next = inner.input_seq_next;
                if s == next {
                    // Advance before sending — a send failure must not leave the
                    // backend stuck waiting for a seq the frontend will never resend.
                    inner.input_seq_next += 1;
                    if let Err(e) = tx.send(input) {
                        // Unbounded send only fails if the receiver is gone
                        // (controller stopped) — nothing to do but drop quietly.
                        tracing::warn!(
                            block_id = %self.block_id,
                            seq = s,
                            "send_input: input channel closed, discarding packet: {e}"
                        );
                        return Ok(());
                    }
                    // Drain any buffered out-of-order packets now in order.
                    loop {
                        let expected = inner.input_seq_next;
                        match inner.input_seq_buf.remove(&expected) {
                            Some(buffered) => {
                                inner.input_seq_next += 1;
                                if let Err(e) = tx.send(buffered) {
                                    tracing::warn!(
                                        block_id = %self.block_id,
                                        seq = expected,
                                        "send_input drain: input channel closed, discarding buffered packet: {e}"
                                    );
                                }
                            }
                            None => break,
                        }
                    }
                    Ok(())
                } else if s > next {
                    if inner.input_seq_buf.len() < SHELL_INPUT_CH_SIZE {
                        inner.input_seq_buf.insert(s, input);
                    } else {
                        tracing::warn!(block_id = %self.block_id, seq = s, "input reorder buffer full, dropping");
                    }
                    Ok(())
                } else {
                    tracing::warn!(block_id = %self.block_id, seq = s, next, "duplicate input seq, discarding");
                    Ok(())
                }
            }
        }
    }

    fn controller_type(&self) -> &str {
        &self.controller_type
    }

    fn block_id(&self) -> &str {
        &self.block_id
    }

    fn agent_id(&self) -> Option<String> {
        self.inner.lock().unwrap().agent_id.clone()
    }

    fn set_agent_id(&self, id: Option<String>) {
        self.inner.lock().unwrap().agent_id = id;
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

#[cfg(test)]
mod agent_id_for_jekt_tests {
    use super::{is_global_cmd_env_injectable, resolve_agent_id_for_jekt};
    use crate::backend::blockcontroller::META_KEY_CMD_ENV;
    use crate::backend::obj::MetaMapType;

    #[test]
    fn identity_keys_are_never_globally_injectable() {
        // reagentx P1, round 2 on #2694: these must never come from global
        // settings.cmd_env — only from a block's own explicit override.
        assert!(!is_global_cmd_env_injectable("AGENTMUX_AGENT_ID"));
        assert!(!is_global_cmd_env_injectable("WAVEMUX_AGENT_ID"));
    }

    #[test]
    fn other_global_cmd_env_keys_remain_injectable() {
        // Cosmetic/unrelated global defaults (colors, arbitrary user
        // cmd_env entries) are unaffected — only the two identity keys are
        // excluded.
        assert!(is_global_cmd_env_injectable("AGENTMUX_AGENT_COLOR"));
        assert!(is_global_cmd_env_injectable("SOME_OTHER_VAR"));
    }

    fn meta_with_env(pairs: &[(&str, &str)]) -> MetaMapType {
        let mut obj = serde_json::Map::new();
        for (k, v) in pairs {
            obj.insert(k.to_string(), serde_json::json!(v));
        }
        let mut meta = MetaMapType::new();
        meta.insert(META_KEY_CMD_ENV.to_string(), serde_json::Value::Object(obj));
        meta
    }

    #[test]
    fn no_block_scoped_identity_resolves_to_none() {
        // This is the collision-fix regression case: a pane with neither
        // AGENTMUX_AGENT_ID nor WAVEMUX_AGENT_ID set in its OWN cmd:env is
        // simply not jekt-registered — no fallback to global settings, and
        // (reagentx P1, round 2) no fallback to this srv PROCESS's own
        // std::env::var either, since that's process-global, not per-pane.
        let empty_meta = MetaMapType::new();
        assert_eq!(resolve_agent_id_for_jekt(&empty_meta), None);
    }

    #[test]
    fn resolves_block_scoped_agentmux_agent_id() {
        let meta = meta_with_env(&[("AGENTMUX_AGENT_ID", "agentx")]);
        assert_eq!(resolve_agent_id_for_jekt(&meta), Some("agentx".to_string()));
    }

    #[test]
    fn resolves_block_scoped_legacy_wavemux_agent_id() {
        // The legacy alias is read from the SAME block-scoped cmd:env map,
        // not process env (reagentx P1, round 2) — genuinely per-pane, so
        // it can't reintroduce the same-host collision.
        let meta = meta_with_env(&[("WAVEMUX_AGENT_ID", "legacy-agent")]);
        assert_eq!(
            resolve_agent_id_for_jekt(&meta),
            Some("legacy-agent".to_string())
        );
    }

    #[test]
    fn agentmux_agent_id_wins_over_legacy_wavemux_when_both_set_on_the_same_block() {
        let meta = meta_with_env(&[
            ("AGENTMUX_AGENT_ID", "agentx"),
            ("WAVEMUX_AGENT_ID", "legacy-agent"),
        ]);
        assert_eq!(resolve_agent_id_for_jekt(&meta), Some("agentx".to_string()));
    }

    #[test]
    fn blank_value_is_treated_as_absent() {
        // Matches persistent.rs's muxbus_agent_id_from_env: a present-but-
        // whitespace-only value doesn't count as a real identity.
        let meta = meta_with_env(&[("AGENTMUX_AGENT_ID", "   ")]);
        assert_eq!(resolve_agent_id_for_jekt(&meta), None);
    }

    #[test]
    fn a_different_blocks_env_never_leaks_in() {
        // resolve_agent_id_for_jekt takes ONE block's own metadata — there
        // is no code path here that could read another block's or the
        // process's own env, unlike the bug this fn was rewritten to fix.
        // This test exists to make that structurally obvious: the fn's
        // only input is the map passed in.
        let other_blocks_meta = meta_with_env(&[("AGENTMUX_AGENT_ID", "agenty")]);
        let this_blocks_meta = MetaMapType::new();
        assert_eq!(resolve_agent_id_for_jekt(&other_blocks_meta), Some("agenty".to_string()));
        assert_eq!(resolve_agent_id_for_jekt(&this_blocks_meta), None);
    }
}

#[cfg(test)]
mod controller_agent_id_tests {
    use super::super::controller::ShellController;
    use super::super::super::Controller;

    fn controller() -> ShellController {
        ShellController::new(
            "shell".to_string(),
            "tab1".to_string(),
            "block1".to_string(),
            None,
            None,
            None,
            None,
        )
    }

    /// reagentx P1 on #2697: `agent_id()` was captured once at spawn and
    /// never refreshed on re-registration, so a legitimately renamed or
    /// reconfigured agent's own messages could be falsely rejected as an
    /// identity mismatch by `inject_message_inner`'s recipient-identity
    /// check (#2695). `set_agent_id` (called from `handle_reactive_register`
    /// on every re-registration) must actually overwrite the captured value,
    /// not just the initial spawn-time one.
    #[test]
    fn set_agent_id_overwrites_a_stale_captured_value() {
        let ctrl = controller();
        assert_eq!(ctrl.agent_id(), None);

        ctrl.set_agent_id(Some("agentx".to_string()));
        assert_eq!(ctrl.agent_id(), Some("agentx".to_string()));

        // A later re-registration under a different name (rename,
        // reconfigured cmd:env) must be reflected, not stuck at "agentx".
        ctrl.set_agent_id(Some("agenty".to_string()));
        assert_eq!(ctrl.agent_id(), Some("agenty".to_string()));

        ctrl.set_agent_id(None);
        assert_eq!(ctrl.agent_id(), None);
    }
}

#[cfg(test)]
mod pty_output_flusher_tests {
    use super::{flush_pty_batch, run_pty_output_flusher, PtyReadChunk, PTY_CHANNEL_CAPACITY};
    use crate::backend::storage::filestore::FileStore;
    use crate::backend::wps;
    use std::sync::{Arc, Mutex};

    /// Records every event delivered to it — lets a test count broadcasts
    /// rather than only inspect final file content, which is what actually
    /// distinguishes "coalesced into one broadcast" from "one broadcast per
    /// PTY read" (both produce identical final content; only the broadcast
    /// COUNT tells them apart). Holds an `Arc` (not an owned `Vec`) because
    /// `Broker::set_client` takes ownership of the client, so the test needs
    /// its own handle to read events back out afterward.
    struct RecordingClient {
        events: Arc<Mutex<Vec<wps::WaveEvent>>>,
    }

    impl wps::WpsClient for RecordingClient {
        fn send_event(&self, _route_id: &str, event: wps::WaveEvent) {
            self.events.lock().unwrap().push(event);
        }
    }

    /// A broker wired to a `RecordingClient`, subscribed (all-scopes, so
    /// this doesn't need to know the exact `block:<id>` scope string) to
    /// `EVENT_BLOCK_FILE` — the event `handle_append_block_file` publishes.
    fn broker_recording_block_file_events() -> (wps::Broker, Arc<Mutex<Vec<wps::WaveEvent>>>) {
        let broker = wps::Broker::new();
        let events = Arc::new(Mutex::new(Vec::new()));
        broker.set_client(Box::new(RecordingClient {
            events: events.clone(),
        }));
        broker.subscribe(
            "test-route",
            wps::SubscriptionRequest {
                event: wps::EVENT_BLOCK_FILE.to_string(),
                scopes: vec![],
                allscopes: true,
            },
        );
        (broker, events)
    }

    /// Decode the base64 `data64` payload of a `term`-file `EVENT_BLOCK_FILE`
    /// broadcast back to raw bytes, for asserting on content.
    fn decode_event_data(event: &wps::WaveEvent) -> Vec<u8> {
        use base64::Engine;
        let data: wps::WSFileEventData =
            serde_json::from_value(event.data.clone().expect("event has data")).expect("valid WSFileEventData");
        base64::engine::general_purpose::STANDARD
            .decode(&data.data64)
            .expect("valid base64")
    }

    #[test]
    fn flush_pty_batch_writes_and_broadcasts_one_call() {
        let (broker, events) = broker_recording_block_file_events();
        let fs = Arc::new(FileStore::open_in_memory().expect("open in-memory filestore"));
        let mut line_buf = Vec::new();

        flush_pty_batch(&broker, "block-1", b"hello world", &[], Some(&fs), &mut line_buf, None);

        let data = fs
            .read_file("block-1", "term")
            .expect("read_file ok")
            .expect("data present");
        assert_eq!(data, b"hello world");

        let evs = events.lock().unwrap();
        assert_eq!(evs.len(), 1, "one flush call must produce exactly one broadcast");
        assert_eq!(decode_event_data(&evs[0]), b"hello world");
    }

    #[test]
    fn flush_pty_batch_is_a_no_op_for_empty_input() {
        // Guards against a spurious empty broadcast on a batch that ended
        // up with nothing in it (shouldn't happen given how the flusher
        // calls this, but a no-op guard here is cheap and cheaper than
        // debugging a phantom empty append if that assumption ever breaks).
        let (broker, events) = broker_recording_block_file_events();
        let mut line_buf = Vec::new();
        flush_pty_batch(&broker, "block-1", b"", &[], None, &mut line_buf, None);
        assert!(events.lock().unwrap().is_empty());
    }

    /// The actual property this whole change exists to deliver: N chunks
    /// that arrive back-to-back (well inside the coalescing window and the
    /// byte cap) collapse into far fewer broadcasts than N — ideally one —
    /// with no data lost, duplicated, or reordered.
    ///
    /// Deterministic, not timing-dependent: every chunk is sent to the
    /// channel BEFORE the flusher is ever polled, so its first "opportunistic
    /// drain" pass finds them all already queued and drains them without
    /// actually waiting out any real wall-clock delay — this isn't racing
    /// the 20ms window, it's exercising the "channel closed mid-accumulation"
    /// exit path (see `run_pty_output_flusher`'s `closed_mid_batch`).
    #[tokio::test]
    async fn rapid_chunks_coalesce_into_far_fewer_broadcasts_with_no_data_loss() {
        let (broker, events) = broker_recording_block_file_events();
        let fs = Arc::new(FileStore::open_in_memory().expect("open in-memory filestore"));
        // Bounded, matching production (PTY_CHANNEL_CAPACITY) — comfortably
        // above the 50 messages below, so every send below still completes
        // without actually waiting for capacity; the determinism this test's
        // own doc comment relies on is unaffected by the channel being bounded.
        let (tx, rx) = tokio::sync::mpsc::channel(PTY_CHANNEL_CAPACITY);

        let mut expected = Vec::new();
        for i in 0..50 {
            let piece = format!("line {i}\n");
            expected.extend_from_slice(piece.as_bytes());
            tx.send(PtyReadChunk {
                data: piece.into_bytes(),
                osc_events: Vec::new(),
            })
            .await
            .unwrap();
        }
        drop(tx); // closes the channel once these 50 are drained

        run_pty_output_flusher(rx, Some(Arc::new(broker)), "block-1".to_string(), Some(fs.clone()), false)
            .await;

        let data = fs.read_file("block-1", "term").expect("read_file ok").expect("data present");
        assert_eq!(data, expected, "coalescing must not lose, duplicate, or reorder bytes");

        // Deterministic exact count, not a loose bound: every chunk was
        // already queued before the flusher was ever polled (see the test's
        // own doc comment), so its very first opportunistic-drain pass finds
        // all 50 sitting in the channel and drains them without actually
        // waiting out any real time — one batch, one broadcast. Verified
        // stable across repeated local runs before asserting on it exactly;
        // a looser "< 50" bound would also pass on a much weaker result
        // (say, 40 broadcasts) that wouldn't actually demonstrate the fix.
        let broadcast_count = events.lock().unwrap().len();
        assert_eq!(
            broadcast_count, 1,
            "50 chunks queued before the flusher ever polled should collapse into \
             exactly one broadcast; got {broadcast_count}"
        );
        // Concatenating every broadcast's own payload, in delivery order,
        // must also reconstruct the exact original stream — proves the
        // coalescing boundaries themselves never split/reordered content
        // even if more than one broadcast happened.
        let mut reconstructed = Vec::new();
        for ev in events.lock().unwrap().iter() {
            reconstructed.extend(decode_event_data(ev));
        }
        assert_eq!(reconstructed, expected);
    }

    /// The byte cap must still force a flush even if chunks keep arriving
    /// faster than the coalescing window would otherwise close — otherwise
    /// a sustained fast producer (`yes` piped into a shell pane) could grow
    /// one "batch" unboundedly.
    #[tokio::test]
    async fn byte_cap_forces_a_flush_before_the_channel_closes() {
        let (broker, events) = broker_recording_block_file_events();
        let fs = Arc::new(FileStore::open_in_memory().expect("open in-memory filestore"));
        // Bounded, matching production — 80 messages below is comfortably
        // under PTY_CHANNEL_CAPACITY, so this test still exercises the byte
        // cap (not channel backpressure) as the thing forcing the split.
        let (tx, rx) = tokio::sync::mpsc::channel(PTY_CHANNEL_CAPACITY);

        // Comfortably over PTY_COALESCE_MAX_BYTES (64 * 4096 = 256KiB) in
        // total, sent as many small chunks so the cap — not the channel
        // closing — is what has to trigger the split.
        let piece = vec![b'x'; 4096];
        let mut expected = Vec::new();
        for _ in 0..80 {
            expected.extend_from_slice(&piece);
            tx.send(PtyReadChunk {
                data: piece.clone(),
                osc_events: Vec::new(),
            })
            .await
            .unwrap();
        }
        drop(tx);

        run_pty_output_flusher(rx, Some(Arc::new(broker)), "block-1".to_string(), Some(fs.clone()), false)
            .await;

        let data = fs.read_file("block-1", "term").expect("read_file ok").expect("data present");
        assert_eq!(data, expected);

        let broadcast_count = events.lock().unwrap().len();
        assert!(
            broadcast_count >= 2,
            "80 * 4096 bytes exceeds PTY_COALESCE_MAX_BYTES; expected at least 2 broadcasts \
             (the cap forcing an early flush), got {broadcast_count}"
        );
    }

    /// The actual property reagentx's P1 asked for: a full channel makes
    /// `blocking_send` — what the real PTY read loop calls, from a real
    /// blocking (`spawn_blocking`) thread, not an async one — block until
    /// the receiver drains it, rather than growing queued memory without
    /// limit. A tiny capacity (not `PTY_CHANNEL_CAPACITY`) so the test is
    /// fast and the blocking is unambiguous with only 2 messages.
    #[test]
    fn blocking_send_backpressures_once_the_channel_is_full() {
        let (tx, mut rx) = tokio::sync::mpsc::channel::<PtyReadChunk>(1);

        // Fills the one slot; must return immediately (room available).
        tx.blocking_send(PtyReadChunk { data: vec![1], osc_events: Vec::new() })
            .expect("first send has room");

        // A real background thread — mirrors the production shape exactly:
        // blocking_send is called from a spawn_blocking thread, never from
        // an async task.
        let sent_second = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let sent_second_writer = sent_second.clone();
        let handle = std::thread::spawn(move || {
            tx.blocking_send(PtyReadChunk { data: vec![2], osc_events: Vec::new() })
                .expect("second send eventually succeeds once drained");
            sent_second_writer.store(true, std::sync::atomic::Ordering::SeqCst);
        });

        // The channel is full and nobody has drained it yet — the second
        // send must still be blocked. A short, generous sleep rather than
        // asserting instantaneously: proving a negative ("hasn't returned
        // yet") is inherently a bit timing-sensitive, but 50ms is enormous
        // next to what an unblocked send would take (microseconds), so a
        // false pass here would need a wildly unlikely scheduling delay.
        std::thread::sleep(std::time::Duration::from_millis(50));
        assert!(
            !sent_second.load(std::sync::atomic::Ordering::SeqCst),
            "second blocking_send must still be blocked while the channel is full"
        );

        // Drain the one queued message — this is the "flusher catches up"
        // moment in production. The blocked send must now complete.
        let first = rx.try_recv().expect("first message still queued");
        assert_eq!(first.data, vec![1]);

        handle.join().expect("background thread should complete once drained");
        assert!(sent_second.load(std::sync::atomic::Ordering::SeqCst));

        let second = rx.try_recv().expect("second message now queued");
        assert_eq!(second.data, vec![2]);
    }

    #[tokio::test]
    async fn no_broker_is_a_clean_no_op() {
        // Mirrors the read loop's own `broker_read.is_some()` gate — the
        // flusher must not panic or hang when there is nothing to flush to.
        let (_tx, rx) = tokio::sync::mpsc::channel::<PtyReadChunk>(PTY_CHANNEL_CAPACITY);
        run_pty_output_flusher(rx, None, "block-1".to_string(), None, false).await;
    }
}
