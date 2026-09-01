// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// CLI provider-login subsystem for the CEF host — spawning `<cli> auth login`
// (or equivalent) child processes over plain pipes or a PTY, capturing the
// OAuth URL, delivering a pasted code back to the child's stdin, and
// reaping/cancelling the login. Split out of platform.rs (which keeps the
// small system-info IPC getters and file/URL/editor utilities) because this
// cluster is a self-contained subsystem with its own state fields
// (`AppState::cli_login_*`) and its own dedicated test modules.

use std::sync::Arc;

use crate::state::AppState;

/// Stdin handle for an in-progress CLI login, regardless of whether
/// it was spawned via plain pipes or via a PTY. `set_provider_auth`
/// writes the OAuth code / pasted token here.
pub enum CliLoginStdin {
    /// Plain pipe — `tokio::process::Command` with `Stdio::piped()`.
    /// AsyncWrite via tokio. Used by Codex, Gemini, Copilot, Kimi —
    /// anything that doesn't require a TTY for its auth subcommand.
    Pipe(tokio::process::ChildStdin),
    /// PTY writer — `portable_pty` master writer. Sync `std::io::Write`.
    /// Used by providers whose auth subcommand needs an interactive TTY:
    /// Claude (`claude auth login` exits ~5s early when spawned
    /// terminal-less) and OpenClaw (`openclaw models auth login` bails on
    /// `isatty()==0`).
    Pty(Box<dyn std::io::Write + Send>),
}

impl CliLoginStdin {
    /// Write a line (terminated with `\n`) to the child's stdin. Used
    /// by `set_provider_auth` to deliver an OAuth code.
    pub async fn write_line(&mut self, line: &str) -> std::io::Result<()> {
        let payload = format!("{}\n", line);
        match self {
            CliLoginStdin::Pipe(s) => {
                use tokio::io::AsyncWriteExt;
                s.write_all(payload.as_bytes()).await?;
                s.flush().await?;
                Ok(())
            }
            CliLoginStdin::Pty(w) => {
                use std::io::Write;
                // portable_pty's master writer is sync. Run it via
                // `block_in_place` so the brief sync write doesn't
                // starve the tokio reactor on the current worker
                // thread if the PTY input buffer is full.
                tokio::task::block_in_place(|| {
                    w.write_all(payload.as_bytes())?;
                    w.flush()
                })
            }
        }
    }
}

/// Hard cap on how long a login CLI may sit at its paste prompt before the
/// reaper kills it. Slightly longer than the frontend's 5-minute auth poll so
/// the frontend (which also reaps on completion/cancel) wins normal cases;
/// this is the backstop for a login whose frontend driver vanished (e.g. the
/// pane was closed without its cleanup firing).
const LOGIN_REAP_TIMEOUT_SECS: u64 = 6 * 60;

/// Cap on how long run_cli_login_pty blocks waiting for a scrapeable OAuth
/// URL before giving up and returning auth_url=None. Bounds every provider
/// that plausibly prints a URL — Codex/Gemini/OpenClaw, and since
/// SPEC_INAPP_CLAUDE_OAUTH_LOGIN_2026_08_03.md §3.2 Claude as well: the
/// pinned CLI (2.1.198+) prints the full authorize URL
/// (`https://claude.com/cai/oauth/authorize?…`) under this PTY spawn,
/// verified by live probes on 2026-08-03, so catalog.ts's
/// headlessLoginUrlUnsupported flag was dropped for Claude and tier 1
/// reaches this function again. An OLDER Claude CLI (≤2.1.183 behavior,
/// which prints nothing) simply times out here — that's the frontend's
/// behavior-gate: auth_url=None makes runProviderLogin fall through to
/// tiers 2/3 exactly as before, with no CLI version check anywhere.
///
/// Left at 15s, NOT shortened as a "safety margin": reagent P1 on PR #2300
/// caught an earlier attempt to cut this to 5s, which would have killed
/// valid in-progress OpenClaw logins (they can take close to the full 15s
/// to print their URL) and forced an unnecessary terminal fallback.
const URL_CAPTURE_TIMEOUT_SECS: u64 = 15;

/// Spawn a CLI auth login flow.
pub async fn run_cli_login(
    state: Arc<AppState>,
    args: &serde_json::Value,
) -> Result<serde_json::Value, String> {
    let cli_path = args
        .get("cli_path")
        .or_else(|| args.get("cliPath"))
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing cli_path".to_string())?
        .to_string();

    let login_args: Vec<String> = args
        .get("login_args")
        .or_else(|| args.get("loginArgs"))
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default();

    let auth_env: std::collections::HashMap<String, String> = args
        .get("auth_env")
        .or_else(|| args.get("authEnv"))
        .and_then(|v| v.as_object())
        .map(|obj| {
            obj.iter()
                .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                .collect()
        })
        .unwrap_or_default();

    // `requires_tty` (passed by the frontend from the provider config)
    // selects the PTY-spawn branch below. Providers like OpenClaw
    // strictly require an interactive TTY for their auth subcommand —
    // plain piped stdio causes the CLI to exit with
    // "requires an interactive TTY" before printing the OAuth URL.
    let requires_tty = args
        .get("requires_tty")
        .or_else(|| args.get("requiresTty"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    // Which key in `auth_env` holds the provider's isolated config dir —
    // "CLAUDE_CONFIG_DIR" for Claude, "OPENCLAW_HOME" for OpenClaw (the
    // only other requiresLoginTty/awaitTier1Completion provider today),
    // catalog.ts's per-provider `authConfigDirEnvVar`. reagent P1 on
    // PR #2410: capture_cred_baseline previously hardcoded
    // "CLAUDE_CONFIG_DIR", silently no-op'ing the credential-freshness
    // guard for OpenClaw (auth_env has no such key, so the baseline
    // capture always returned None → credential_changed always true —
    // exactly the false-positive-on-reconnect bug this guard exists to
    // close, just for a different provider). Falls back to
    // "CLAUDE_CONFIG_DIR" if omitted, for any caller not yet updated.
    let auth_config_dir_env_var = args
        .get("auth_config_dir_env_var")
        .or_else(|| args.get("authConfigDirEnvVar"))
        .and_then(|v| v.as_str())
        .unwrap_or("CLAUDE_CONFIG_DIR")
        .to_string();

    // §4 instrumentation (SPEC_HOST_CLI_LOGIN_CAPTURE): snapshot the
    // auth-precedence env keys the child will see. A stray ANTHROPIC_API_KEY
    // (precedence #3) overrides subscription OAuth (#6) and, if it belongs to a
    // disabled/expired org, yields the "loggedIn but 401" symptom we're chasing.
    // "override" = passed explicitly in auth_env; "inherited" = present in the
    // host process env the child inherits; "absent" = neither. NAMES/STATES
    // ONLY — values are never logged.
    {
        let source = |k: &str| -> &'static str {
            if auth_env.contains_key(k) {
                "override"
            } else if std::env::var(k).is_ok() {
                "inherited"
            } else {
                "absent"
            }
        };
        tracing::info!(
            target: "login_pty",
            cli = %cli_path,
            requires_tty,
            anthropic_api_key = source("ANTHROPIC_API_KEY"),
            anthropic_auth_token = source("ANTHROPIC_AUTH_TOKEN"),
            claude_code_oauth_token = source("CLAUDE_CODE_OAUTH_TOKEN"),
            "run_cli_login: auth-env precedence snapshot"
        );
    }

    // Supersede any in-progress login so we never accumulate orphaned
    // `auth login` children (one per attempt — the confirmed leak). cancel_cli_login
    // kills both transports (pipe oneshot + PTY kill-by-PID) and is idempotent —
    // a no-op when nothing is in flight. We then bump the generation so this
    // attempt's reaper can tell itself apart from the one we just superseded.
    let _ = cancel_cli_login(&state);
    let generation = state
        .cli_login_generation
        .fetch_add(1, std::sync::atomic::Ordering::SeqCst)
        + 1;

    // Capture the credential-freshness baseline BEFORE either spawn path
    // launches its child — see cli_login_cred_baseline's own doc comment.
    // Single call site: run_cli_login_pty is only ever reached from here.
    *state.cli_login_cred_baseline.lock() = capture_cred_baseline(&auth_env, &auth_config_dir_env_var);

    if requires_tty {
        return run_cli_login_pty(state, cli_path, login_args, auth_env, generation).await;
    }

    let mut cmd = make_cli_cmd(&cli_path);
    cmd.args(&login_args)
        .envs(&auth_env)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());

    #[cfg(windows)]
    {
        cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
    }

    let mut child = cmd
        .spawn()
        .map_err(|e| format!("failed to spawn {cli_path}: {e}"))?;

    tracing::info!(cli = %cli_path, "run_cli_login: spawned (pipes), browser should open");

    // Store the stdin handle so set_provider_auth can deliver the OAuth code.
    {
        let mut stored_stdin = state.cli_login_stdin.lock();
        *stored_stdin = child.stdin.take().map(CliLoginStdin::Pipe);
    }
    state
        .cli_login_active
        .store(true, std::sync::atomic::Ordering::SeqCst);

    // Capture the OAuth URL from stdout/stderr (the CLI prints it within a few
    // hundred ms). CRITICAL: the readers must SURVIVE this capture and keep
    // draining for the child's whole lifetime (see the drain tasks below).
    //
    // The original code dropped these readers right after the URL was found.
    // That closes the read end of the CLI's stdout pipe; the CLI's next write —
    // its `Paste code here >` prompt — then hits a broken pipe and the Node
    // process EPIPE-exits (cleanly, exit 0) within seconds, BEFORE the user can
    // paste the code. That is the login hang: by the time the user finishes
    // browser auth the CLI is already gone, so the pasted code has nothing to
    // be delivered to. (Verified by reproducing the CLI with stdout → a file,
    // where it stays alive at the prompt.)
    use tokio::io::AsyncBufReadExt;
    let mut stdout_lines = child
        .stdout
        .take()
        .map(|s| tokio::io::BufReader::new(s).lines());
    let mut stderr_lines = child
        .stderr
        .take()
        .map(|s| tokio::io::BufReader::new(s).lines());

    let auth_url: Option<String> = tokio::time::timeout(
        std::time::Duration::from_secs(2),
        async {
            let mut count = 0usize;
            if let Some(lines) = stdout_lines.as_mut() {
                while let Ok(Some(line)) = lines.next_line().await {
                    if let Some(url) = extract_url(&line) {
                        return Some(url);
                    }
                    count += 1;
                    if count > 20 { break; }
                }
            }
            if let Some(lines) = stderr_lines.as_mut() {
                while let Ok(Some(line)) = lines.next_line().await {
                    if let Some(url) = extract_url(&line) {
                        return Some(url);
                    }
                    count += 1;
                    if count > 40 { break; }
                }
            }
            None
        },
    )
    .await
    .ok()
    .flatten();

    if let Some(ref url) = auth_url {
        tracing::info!(url = %redact_url_query(url), "run_cli_login: captured auth URL");
    } else {
        tracing::warn!("run_cli_login: no auth URL captured within 2s");
    }

    // Keep draining stdout+stderr for the rest of the child's life so the CLI
    // can write its `Paste code here >` prompt (and any progress) without
    // hitting a closed pipe and EPIPE-exiting. The drain tasks own the readers
    // and end at EOF when the CLI finally exits. This is the fix for the login
    // hang described above — without it the CLI dies seconds after printing the
    // URL, before the user can paste the code.
    if let Some(mut lines) = stdout_lines {
        tokio::spawn(async move { while let Ok(Some(_)) = lines.next_line().await {} });
    }
    if let Some(mut lines) = stderr_lines {
        tokio::spawn(async move { while let Ok(Some(_)) = lines.next_line().await {} });
    }

    let (cancel_tx, cancel_rx) = tokio::sync::oneshot::channel::<()>();
    {
        let mut stored = state.cli_login_cancel.lock();
        *stored = Some(cancel_tx);
    }

    let state_for_cleanup = state.clone();
    tokio::spawn(async move {
        tokio::select! {
            result = child.wait() => {
                match result {
                    Ok(status) => tracing::info!(
                        exit_code = ?status.code(),
                        "run_cli_login: child exited"
                    ),
                    Err(e) => tracing::warn!(
                        error = %e,
                        "run_cli_login: child wait error"
                    ),
                }
            }
            _ = cancel_rx => {
                tracing::info!("run_cli_login: cancel signal received, killing child");
                let _ = child.kill().await;
            }
            _ = tokio::time::sleep(std::time::Duration::from_secs(LOGIN_REAP_TIMEOUT_SECS)) => {
                tracing::warn!("run_cli_login: login timed out, killing child");
                let _ = child.kill().await;
            }
        }
        // Clear the stored stdin handle once the process is done — but only if a
        // newer login hasn't superseded us and repopulated the slot.
        if state_for_cleanup
            .cli_login_generation
            .load(std::sync::atomic::Ordering::SeqCst)
            == generation
        {
            *state_for_cleanup.cli_login_stdin.lock() = None;
            state_for_cleanup
                .cli_login_active
                .store(false, std::sync::atomic::Ordering::SeqCst);
        }
    });

    Ok(serde_json::json!({ "auth_url": auth_url }))
}

/// Minimal wait/kill surface over the two login-child representations:
/// portable_pty's boxed Child on Windows (ConPTY path) and std's Child on
/// Unix (hand-rolled openpty spawn — see `spawn_login_pty_unix` for why).
/// Lets `run_cli_login_pty` keep one shared reaper for both.
trait LoginChildWait: Send {
    fn try_exit_code(&mut self) -> std::io::Result<Option<i64>>;
    fn kill_child(&mut self) -> std::io::Result<()>;
    fn wait_child(&mut self);
}

#[cfg(windows)]
impl LoginChildWait for Box<dyn portable_pty::Child + Send + Sync> {
    fn try_exit_code(&mut self) -> std::io::Result<Option<i64>> {
        Ok(self.try_wait()?.map(|status| status.exit_code() as i64))
    }
    fn kill_child(&mut self) -> std::io::Result<()> {
        self.kill()
    }
    fn wait_child(&mut self) {
        let _ = self.wait();
    }
}

#[cfg(unix)]
impl LoginChildWait for std::process::Child {
    fn try_exit_code(&mut self) -> std::io::Result<Option<i64>> {
        Ok(self
            .try_wait()?
            .map(|status| status.code().map(i64::from).unwrap_or(-1)))
    }
    fn kill_child(&mut self) -> std::io::Result<()> {
        self.kill()
    }
    fn wait_child(&mut self) {
        let _ = self.wait();
    }
}

/// Mark every fd ≥ 3 close-on-exec, using only `fcntl` (async-signal-safe,
/// and — unlike `close` — NOT interposed by libcef). Called from a forked
/// child's `pre_exec`, so it must not allocate or take locks.
///
/// Why mark-CLOEXEC instead of closing outright:
///   1. It never calls the `close` symbol, so Chromium's fd-ownership guard
///      (the whole bug this file fixes) can't fire.
///   2. std's internal exec-error reporting pipe is itself an fd ≥ 3 and is
///      already CLOEXEC. Re-marking it CLOEXEC is a no-op that KEEPS it open
///      until `execve` succeeds — so a failed exec still writes its errno
///      back and `Command::spawn()` returns the real error. Closing that fd
///      outright (the old fallback) made a failed exec look like `Ok(child)`
///      that immediately exits, masking the cause.
///   3. Every inherited fd is neutralized (no host sockets/pipes leak into
///      the exec'd CLI), including on kernels without `close_range`.
///
/// Upper bound: the soft RLIMIT_NOFILE, capped so the loop stays bounded if
/// the limit is huge (this host's is 1048576). `getrlimit` is
/// async-signal-safe; `sysconf` is avoided for that reason.
#[cfg(unix)]
unsafe fn mark_fds_cloexec_from_3() {
    let mut rl: libc::rlimit = std::mem::zeroed();
    let max_fd: libc::c_int = if libc::getrlimit(libc::RLIMIT_NOFILE, &mut rl) == 0 {
        // rlim_cur is (highest fd)+1; cap to keep the fallback loop bounded.
        (rl.rlim_cur.min(65536) as libc::c_int).max(3)
    } else {
        4096
    };
    let mut fd: libc::c_int = 3;
    while fd < max_fd {
        let flags = libc::fcntl(fd, libc::F_GETFD);
        if flags >= 0 && (flags & libc::FD_CLOEXEC) == 0 {
            libc::fcntl(fd, libc::F_SETFD, flags | libc::FD_CLOEXEC);
        }
        fd += 1;
    }
}

/// Spawn the login CLI on a fresh PTY WITHOUT portable_pty's spawn path.
///
/// Why not `pair.slave.spawn_command()`: portable_pty's `pre_exec` calls
/// `close_random_fds()`, which invokes the libc `close()` symbol on every
/// fd ≥ 3 in the forked child. In this host, `close()` resolves to
/// libcef.so's interposed close (Chromium's fd-ownership guard,
/// base/files/scoped_file_linux.cc — libcef exports a strong `close`
/// symbol). The guard hits a ScopedFD-owned fd and IMMEDIATE_CRASHes the
/// child before exec — "Crashing due to FD ownership violation" in the
/// [login-pty] capture, for ANY spawned program and CLI version. Diagnosed
/// live 2026-07-16 by spawning /bin/sh through this path and observing the
/// identical crash stack (same addresses across runs = pre-exec child
/// running the host image, not the CLI). Supersedes the Bun-crash theory
/// in SPEC_HOST_CLI_LOGIN_CAPTURE §2. agentmux-srv never hit this because
/// it doesn't link libcef.
///
/// This variant performs fd hygiene with the raw close_range(2) syscall
/// (invisible to symbol interposition) and otherwise mirrors portable_pty:
/// signal reset, setsid, TIOCSCTTY, stdio on the slave.
#[cfg(unix)]
fn spawn_login_pty_unix(
    cli_path: &str,
    login_args: &[String],
    auth_env: &std::collections::HashMap<String, String>,
) -> Result<(std::process::Child, std::fs::File), String> {
    use std::os::fd::FromRawFd;
    use std::os::unix::process::CommandExt;

    let mut master_fd: libc::c_int = -1;
    let mut slave_fd: libc::c_int = -1;
    let mut ws: libc::winsize = unsafe { std::mem::zeroed() };
    ws.ws_row = 24;
    // Wide enough that the CLI's own line-wrapping never wraps the OAuth
    // URL — see the matching Windows PtySize comment above for why (no
    // OSC-8 hyperlink in the wild; the plain-text URL line hard-wraps at
    // the reported column width and extract_url() only sees a truncated
    // fragment).
    ws.ws_col = 4096;
    let rc = unsafe {
        libc::openpty(
            &mut master_fd,
            &mut slave_fd,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            // `&mut` coerces to both platform bindings: Apple libc declares
            // `winp: *mut winsize`, Linux `*const winsize`. A plain `&ws`
            // only satisfies Linux — breaking macOS builds, which CI never
            // compiles (no macOS runner).
            &mut ws,
        )
    };
    if rc != 0 {
        return Err(format!(
            "openpty for {cli_path}: {}",
            std::io::Error::last_os_error()
        ));
    }
    // Wrap immediately so the fds can't leak on an early return. Parent-side
    // drops go through the interposed close() too, but the guard only fires
    // for fds Chromium registered as owned — these are ours.
    let master = unsafe { std::fs::File::from_raw_fd(master_fd) };
    let slave = unsafe { std::fs::File::from_raw_fd(slave_fd) };

    let stdin = slave.try_clone().map_err(|e| format!("dup pty slave: {e}"))?;
    let stdout = slave.try_clone().map_err(|e| format!("dup pty slave: {e}"))?;

    let mut cmd = std::process::Command::new(cli_path);
    cmd.args(login_args);
    for (k, v) in auth_env {
        cmd.env(k, v);
    }
    if let Ok(cwd) = std::env::current_dir() {
        cmd.current_dir(cwd);
    }
    cmd.stdin(std::process::Stdio::from(stdin))
        .stdout(std::process::Stdio::from(stdout))
        .stderr(std::process::Stdio::from(slave));

    unsafe {
        cmd.pre_exec(|| {
            // Forked child of a Chromium-linked host: nothing in here may
            // call the libc `close` symbol (see fn doc). Raw syscalls only.
            // std has already dup2'd the pty slave onto fds 0/1/2.

            // Default-reset signal dispositions + mask (mirrors
            // portable_pty; the host runs Chromium's custom handlers).
            for signo in [
                libc::SIGCHLD,
                libc::SIGHUP,
                libc::SIGINT,
                libc::SIGQUIT,
                libc::SIGTERM,
                libc::SIGALRM,
            ] {
                libc::signal(signo, libc::SIG_DFL);
            }
            let empty: libc::sigset_t = std::mem::zeroed();
            libc::sigprocmask(libc::SIG_SETMASK, &empty, std::ptr::null_mut());

            if libc::setsid() == -1 {
                return Err(std::io::Error::last_os_error());
            }
            // Adopt the slave (on fd 0) as the controlling terminal.
            if libc::ioctl(0, libc::TIOCSCTTY as _, 0) == -1 {
                return Err(std::io::Error::last_os_error());
            }

            // fd hygiene WITHOUT the `close` symbol. Both paths only ever
            // MARK fds close-on-exec — they never close in-process, so
            // Chromium's guard can't fire and std's exec-error pipe survives
            // to report a failed exec (see mark_fds_cloexec_from_3).
            #[cfg(target_os = "linux")]
            {
                const CLOSE_RANGE_CLOEXEC: libc::c_uint = 4;
                // Fast path (kernel 5.11+): one syscall marks the whole
                // fd ≥ 3 range CLOEXEC.
                if libc::syscall(
                    libc::SYS_close_range,
                    3 as libc::c_uint,
                    libc::c_uint::MAX,
                    CLOSE_RANGE_CLOEXEC,
                ) == -1
                {
                    // close_range absent (pre-5.9, ENOSYS) or CLOEXEC flag
                    // unsupported (5.9–5.10, EINVAL): fall back to the fcntl
                    // marker — same CLOEXEC semantics, no fd left leaked, no
                    // outright close of std's exec-error pipe.
                    mark_fds_cloexec_from_3();
                }
            }
            #[cfg(not(target_os = "linux"))]
            {
                // macOS/BSD: no close_range(2). Chromium's close guard is
                // Linux-only, but we still want clean fd inheritance.
                mark_fds_cloexec_from_3();
            }
            Ok(())
        });
    }

    let child = cmd
        .spawn()
        .map_err(|e| format!("PTY spawn of {cli_path}: {e}"))?;
    // The parent's slave copies were consumed by Stdio and closed by
    // spawn(); only the master remains, so EOF on it tracks child exit.
    Ok((child, master))
}

/// PTY-backed variant of run_cli_login. Used for providers whose auth
/// subcommand requires an interactive TTY: Claude (`claude auth login`
/// exits cleanly ~5s after printing the URL when spawned terminal-less)
/// and OpenClaw (`openclaw models auth login --provider <id>` exits
/// immediately with "requires an interactive TTY" when stdin is a pipe).
///
/// Same return shape as run_cli_login: `{ auth_url: <url or null> }`.
/// Writes the master writer into `state.cli_login_stdin` so
/// `set_provider_auth` can deliver an OAuth code if the CLI prompts
/// for one.
///
/// Spawn is platform-split:
///   - Windows: portable_pty/ConPTY. CRITICAL lifetime contract: the
///     PtyPair (master + slave) MUST stay alive across child.wait(). Same
///     hazard pattern agentmux-bashwrap navigates. The blocking wait task
///     takes ownership of the pair so the destructor runs after the reap.
///   - Unix: `spawn_login_pty_unix` — portable_pty's spawn is UNUSABLE in
///     this host because its pre-exec fd cleanup trips libcef's interposed
///     close() and crashes the child before exec (see that fn's doc).

/// Device Status Report cursor-position query — some CLIs (confirmed:
/// Claude Code, on detecting a real TTY) send this immediately and BLOCK
/// waiting for the terminal to reply with `ESC[<row>;<col>R` before
/// printing anything else at all. A bare `portable_pty` handle has no
/// attached terminal emulator to answer this automatically — nothing
/// does, on any platform, without code specifically watching for it — so
/// the child hangs forever and the whole capture loop below times out
/// having seen zero bytes. This is the confirmed root cause of issue
/// #2429 ("no PTY output captured... despite correct binary resolution"):
/// isolated repro (a standalone portable_pty harness spawning the exact
/// same binary/args) showed the child's first and ONLY output was these
/// 4 bytes, then silence: answering this one query unblocked it
/// immediately, and it printed its OAuth URL within half a second.
const DSR_CURSOR_POSITION_QUERY: &[u8] = b"\x1b[6n";
/// Synthetic "row 1, col 1" reply. The actual reported position doesn't
/// matter to these CLIs — confirmed by the repro above — they only need
/// *something* to answer so their TTY-capability probe stops blocking.
const DSR_CURSOR_POSITION_REPLY: &[u8] = b"\x1b[1;1R";

/// Wraps the raw PTY reader and transparently answers a cursor-position
/// query (see [`DSR_CURSOR_POSITION_QUERY`]) the moment it appears
/// anywhere in the stream — not just at startup, since a TUI could
/// plausibly re-probe after a resize — passing every byte through
/// unchanged to the caller (the query's own bytes included; they're
/// harmless noise to the line-scanning loop below, not worth the extra
/// complexity of stripping them from the pass-through).
///
/// Generic over `on_query` (rather than baking in `Arc<AppState>`
/// directly) so the byte-scanning logic — the actually bug-prone part —
/// is unit-testable without constructing a real `AppState`. The
/// production call site's closure writes through `state.cli_login_stdin`
/// — the SAME handle `set_provider_auth` uses to deliver a pasted OAuth
/// code — rather than a second independent writer, since `portable_pty`'s
/// writer is a single-owner handle already moved into that slot by the
/// time this reader is constructed.
struct DsrRespondingReader<R, F> {
    inner: R,
    on_query: F,
    /// Carry-over from the previous `read()` call — bounded to
    /// `DSR_CURSOR_POSITION_QUERY.len() - 1` bytes — so a query split
    /// across two `read()` calls (e.g. a slow/busy child) is still
    /// detected instead of silently missed at the chunk boundary.
    tail: Vec<u8>,
}

impl<R, F: FnMut()> DsrRespondingReader<R, F> {
    fn new(inner: R, on_query: F) -> Self {
        Self { inner, on_query, tail: Vec::new() }
    }
}

impl<R: std::io::Read, F: FnMut()> std::io::Read for DsrRespondingReader<R, F> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = self.inner.read(buf)?;
        if n == 0 {
            return Ok(0);
        }
        self.tail.extend_from_slice(&buf[..n]);
        if self
            .tail
            .windows(DSR_CURSOR_POSITION_QUERY.len())
            .any(|w| w == DSR_CURSOR_POSITION_QUERY)
        {
            (self.on_query)();
        }
        let keep = DSR_CURSOR_POSITION_QUERY.len().saturating_sub(1);
        if self.tail.len() > keep {
            let drop = self.tail.len() - keep;
            self.tail.drain(0..drop);
        }
        Ok(n)
    }
}

/// Production `on_query` callback: writes [`DSR_CURSOR_POSITION_REPLY`]
/// through `state.cli_login_stdin`. Separated from `DsrRespondingReader`
/// itself (see its doc comment) purely so the reader's byte-scanning
/// logic stays unit-testable without a real `AppState`.
fn reply_to_dsr_via_cli_login_stdin(state: &Arc<AppState>) {
    tracing::debug!(
        target: "login_pty",
        "[login-pty] answering cursor-position query (ESC[6n) — see issue #2429"
    );
    if let Some(CliLoginStdin::Pty(w)) = state.cli_login_stdin.lock().as_mut() {
        let _ = w.write_all(DSR_CURSOR_POSITION_REPLY);
        let _ = w.flush();
    }
}

async fn run_cli_login_pty(
    state: Arc<AppState>,
    cli_path: String,
    login_args: Vec<String>,
    auth_env: std::collections::HashMap<String, String>,
    generation: u64,
) -> Result<serde_json::Value, String> {
    // §5.1: capture the isolated CLAUDE_CONFIG_DIR so the reaper can report
    // whether `setup-token` wrote `.credentials.json` there on completion — that
    // decides whether the agent is auto-authed via the dir (no env-persist needed)
    // or we must persist the captured CLAUDE_CODE_OAUTH_TOKEN into the spawn env.
    let cred_check_dir = auth_env.get("CLAUDE_CONFIG_DIR").cloned();

    #[cfg(windows)]
    let (child, child_pid, reader, writer, keepalive) = {
        use portable_pty::{native_pty_system, CommandBuilder, PtySize};

        let pty_system = native_pty_system();
        let pair = pty_system
            .openpty(PtySize {
                rows: 24,
                // Wide enough that the CLI's own line-wrapping never wraps
                // the OAuth URL. At 80 cols it did — Claude Code prints
                // "If the browser didn't open, visit: <url>" as plain text
                // (no OSC-8 hyperlink in the wild, despite an earlier
                // synthetic probe suggesting otherwise) and hard-wraps it
                // mid-query-string, so extract_url() only ever saw a
                // truncated fragment missing client_id and everything after
                // it. 4096 comfortably covers any realistic query string.
                cols: 4096,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|e| format!("openpty for {cli_path}: {e}"))?;

        // Resolve past the `.cmd`/`.bat` npm shim before spawning under the
        // PTY — CommandBuilder on the raw shim path forces Windows through
        // `cmd.exe /c`, which hangs indefinitely under a real ConPTY (no
        // output, target config dir never created, confirmed live). Every
        // other spawn site in this codebase already resolves through this;
        // this was the one PTY-specific gap.
        //
        // `None` means the shim didn't match either known npm shape — fail
        // fast instead of falling back to `cmd.exe /C`, which hangs just as
        // indefinitely under this real ConPTY as the raw shim path did
        // (that's the exact bug this resolves; silently reintroducing it
        // for an unrecognized shim shape would just move the 15s timeout
        // somewhere less visible).
        let (spawn_program, spawn_prefix_args) = agentmux_common::resolve_cli_spawn_target(&cli_path)
            .ok_or_else(|| format!("could not resolve .cmd/.bat shim for spawn: {cli_path}"))?;
        let mut cmd = CommandBuilder::new(&spawn_program);
        for a in &spawn_prefix_args {
            cmd.arg(a);
        }
        for a in &login_args {
            cmd.arg(a);
        }
        for (k, v) in &auth_env {
            cmd.env(k, v);
        }
        if let Ok(cwd) = std::env::current_dir() {
            cmd.cwd(cwd);
        }

        let child = pair
            .slave
            .spawn_command(cmd)
            .map_err(|e| format!("PTY spawn of {cli_path}: {e}"))?;
        let child_pid = child.process_id();

        let reader = pair
            .master
            .try_clone_reader()
            .map_err(|e| format!("PTY try_clone_reader: {e}"))?;
        let writer = pair
            .master
            .take_writer()
            .map_err(|e| format!("PTY take_writer: {e}"))?;
        (child, child_pid, reader, writer, pair)
    };

    #[cfg(unix)]
    let (child, child_pid, reader, writer, keepalive) = {
        let (child, master) = spawn_login_pty_unix(&cli_path, &login_args, &auth_env)?;
        let child_pid = Some(child.id());
        let reader: Box<dyn std::io::Read + Send> = Box::new(
            master
                .try_clone()
                .map_err(|e| format!("PTY clone reader: {e}"))?,
        );
        let writer: Box<dyn std::io::Write + Send> = Box::new(
            master
                .try_clone()
                .map_err(|e| format!("PTY clone writer: {e}"))?,
        );
        // `master` doubles as the keep-alive: it must outlive the reap so
        // the reader doesn't see a dead pty before the child is waited.
        (child, child_pid, reader, writer, master)
    };

    // Store the child PID before moving the child into the wait
    // task — cancel_cli_login needs it to kill the subprocess
    // platform-side, since aborting the spawn_blocking wait does not
    // propagate to the child.
    if let Some(pid) = child_pid {
        *state.cli_login_pty_pid.lock() = Some(pid);
    }

    tracing::info!(cli = %cli_path, pid = ?child_pid, "run_cli_login: spawned (PTY), waiting for OAuth URL");

    // Store the PTY writer so set_provider_auth can deliver an OAuth
    // code via stdin (some flows prompt the user to paste a code).
    {
        let mut stored = state.cli_login_stdin.lock();
        *stored = Some(CliLoginStdin::Pty(writer));
    }
    state
        .cli_login_active
        .store(true, std::sync::atomic::Ordering::SeqCst);

    // Synchronously read from the master in a blocking task, scanning
    // each line for an OAuth URL. portable_pty's reader is sync.
    // The URL_CAPTURE_TIMEOUT_SECS cap is enforced async-side via
    // tokio::time::timeout —
    // BufRead::read_line itself blocks indefinitely without per-read
    // timeout support, so a child that pauses before its first line
    // (or sits at a prompt with no newline) would wedge `url_rx.await`
    // without it. When the timeout fires we return auth_url=None to
    // the frontend and let the wait task below reap the child whenever
    // it finishes naturally.
    let (url_tx, url_rx) = tokio::sync::oneshot::channel::<Option<String>>();
    let state_for_dsr = state.clone();
    tokio::task::spawn_blocking(move || {
        use std::io::BufRead;
        // See DsrRespondingReader's doc comment / issue #2429: without
        // this, a child that probes cursor position on startup (Claude
        // Code does) hangs forever before printing anything, and
        // read_line below would never even see the query bytes (they
        // carry no trailing newline) to know to respond.
        let reader = DsrRespondingReader::new(reader, || reply_to_dsr_via_cli_login_stdin(&state_for_dsr));
        let mut reader = std::io::BufReader::new(reader);
        // Wrap the oneshot in an Option so we send the URL exactly once and then
        // keep reading. We LOG every line from the first byte AND scan for an
        // OAuth URL until one is found — the CLI's `Paste code here >` prompt
        // and, crucially, its response to the code delivered via
        // set_provider_auth (success vs. an "invalid code" / error). Draining
        // also keeps the CLI from blocking on a full PTY output buffer.
        let mut url_tx = Some(url_tx);
        let mut line = String::new();
        loop {
            line.clear();
            match reader.read_line(&mut line) {
                Ok(0) => break, // EOF
                Ok(_) => {
                    // Log EVERY PTY line from the first byte (§4 of
                    // SPEC_HOST_CLI_LOGIN_CAPTURE). Before this, lines were only
                    // logged AFTER a URL was captured, so a login that never
                    // printed a URL (Claude Code v2.1.x — the URL is clipboard-
                    // on-`c`, not a stdout line) was a black box: we couldn't
                    // tell capture-miss from no-browser from a silent exit.
                    let t = line.trim_end();
                    if !t.trim().is_empty() {
                        // SECURITY: `claude setup-token` (§5.1) prints a live
                        // CLAUDE_CODE_OAUTH_TOKEN (`sk-ant-oat…`) to stdout — never
                        // write it to the host log. redact_secrets masks it while
                        // preserving the surrounding shape so we can still read the
                        // output format from the capture. redact_url_queries_in_line
                        // similarly masks any authorize URL's query (state/
                        // code_challenge — SPEC_INAPP_CLAUDE_OAUTH_LOGIN_2026_08_03.md
                        // §5) wherever it appears in the line, not just in the
                        // separate "captured auth URL" event below (codex P2 on
                        // PR #2410, second round).
                        tracing::info!(
                            target: "login_pty",
                            "[login-pty] {}",
                            redact_url_queries_in_line(&redact_secrets(t))
                        );
                    }
                    // §5.1 (SPEC_HOST_CLI_LOGIN_CAPTURE): a CLAUDE_CODE_OAUTH_TOKEN /
                    // `sk-ant-oat…` line is the setup-token headless contract — the
                    // login completed and the token arrived. Log the capture (redacted)
                    // so we can confirm format + completion without leaking the secret.
                    // (Parser/env-persist wiring lands once this confirms the format.)
                    if t.contains("sk-ant-oat") || t.contains("CLAUDE_CODE_OAUTH_TOKEN") {
                        tracing::info!(
                            target: "login_pty",
                            "[login-pty] setup-token output detected ({} chars; token redacted above)",
                            t.len()
                        );
                    }
                    // Capture the OAuth URL for providers that print one —
                    // Codex/Gemini/OpenClaw, and Claude 2.1.198+ (its "If the
                    // browser didn't open, visit: https://claude.com/cai/oauth/
                    // authorize?…" fallback line, sometimes OSC-8-hyperlinked;
                    // SPEC_INAPP_CLAUDE_OAUTH_LOGIN_2026_08_03.md §2).
                    if let Some(tx) = url_tx.take() {
                        if let Some(u) = extract_url(&line) {
                            let _ = tx.send(Some(u));
                        } else {
                            url_tx = Some(tx); // not the URL line yet
                        }
                    }
                }
                Err(e) => {
                    tracing::warn!(error = %e, "run_cli_login_pty: read error");
                    break;
                }
            }
        }
        // EOF/error before a URL was ever seen — unblock the awaiting caller.
        if let Some(tx) = url_tx.take() {
            let _ = tx.send(None);
        }
    });

    let auth_url: Option<String> = match tokio::time::timeout(
        std::time::Duration::from_secs(URL_CAPTURE_TIMEOUT_SECS),
        url_rx,
    )
    .await
    {
        Ok(Ok(u)) => u,
        Ok(Err(_)) | Err(_) => None,
    };
    if let Some(ref url) = auth_url {
        tracing::info!(url = %redact_url_query(url), "run_cli_login_pty: captured auth URL");
    } else {
        tracing::warn!(
            "run_cli_login_pty: no auth URL captured within {URL_CAPTURE_TIMEOUT_SECS}s"
        );
    }

    // Reap the child in a blocking task. The PtyPair (master + slave)
    // moves into the closure so its destructor runs AFTER child.wait()
    // — necessary for ConPTY on Windows (see retro
    // 2026-05-11-live-log-streaming-wrapper-failures.md §4.2).
    //
    // Cancel handling: `cancel_cli_login` reads `cli_login_pty_pid`
    // and kills the subprocess by PID; once the child dies, this
    // wait task observes the exit and clears the PID slot.
    let state_for_cleanup = state.clone();
    tokio::task::spawn_blocking(move || {
        let mut child = child;
        // Poll for exit with a hard timeout. The previous blocking wait() could
        // only end when the child self-exited, so an abandoned login (user never
        // pastes, or completes OAuth out-of-band) sat at the paste prompt
        // forever — the confirmed process leak.
        let deadline =
            std::time::Instant::now() + std::time::Duration::from_secs(LOGIN_REAP_TIMEOUT_SECS);
        loop {
            match child.try_exit_code() {
                Ok(Some(exit_code)) => {
                    tracing::info!(
                        exit_code,
                        "run_cli_login_pty: child exited"
                    );
                    // §5.1: report whether the login persisted isolated creds. If
                    // this is `true` after a setup-token completion, the agent is
                    // auto-authed via CLAUDE_CONFIG_DIR and no env-persist is needed.
                    if let Some(dir) = &cred_check_dir {
                        let cred = std::path::Path::new(dir).join(".credentials.json");
                        tracing::info!(
                            target: "login_pty",
                            "[login-pty] post-login: {}/.credentials.json exists = {}",
                            dir,
                            cred.exists()
                        );
                    }
                    break;
                }
                Ok(None) => {
                    if std::time::Instant::now() >= deadline {
                        tracing::warn!("run_cli_login_pty: login timed out, killing child");
                        let _ = child.kill_child();
                        child.wait_child();
                        break;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(500));
                }
                Err(e) => {
                    tracing::warn!(error = %e, "run_cli_login_pty: child wait error");
                    break;
                }
            }
        }
        // The pty keep-alive (Windows: the PtyPair — ConPTY lifetime
        // contract; Unix: the master File) drops here, after the child reaps.
        drop(keepalive);
        // Only clear the slots if we still own them — a newer login may have
        // superseded us and repopulated them; clearing would strand the new
        // login's stdin handle (the "stuck login" bug).
        if state_for_cleanup
            .cli_login_generation
            .load(std::sync::atomic::Ordering::SeqCst)
            == generation
        {
            *state_for_cleanup.cli_login_stdin.lock() = None;
            *state_for_cleanup.cli_login_pty_pid.lock() = None;
            state_for_cleanup
                .cli_login_active
                .store(false, std::sync::atomic::Ordering::SeqCst);
        }
    });

    Ok(serde_json::json!({ "auth_url": auth_url }))
}

/// Mask secret tokens before a PTY line is logged. `claude setup-token` (§5.1)
/// prints a live `CLAUDE_CODE_OAUTH_TOKEN` (`sk-ant-oat…`, a ~1-year credential)
/// to stdout; the slice-1 line logging would otherwise write it verbatim to the
/// host log. We keep `sk-ant-` + 4 chars (enough to recognize the output format)
/// and mask the rest of the token run, so the capture stays useful without
/// leaking the credential. ASCII-only token chars → all byte slices are on char
/// boundaries.
fn redact_secrets(line: &str) -> String {
    const PREFIX: &str = "sk-ant-";
    let mut out = String::with_capacity(line.len());
    let mut rest = line;
    while let Some(pos) = rest.find(PREFIX) {
        out.push_str(&rest[..pos]);
        let after = &rest[pos..];
        // Token run = PREFIX followed by [A-Za-z0-9_-]*.
        let tok_end = after
            .char_indices()
            .skip(PREFIX.len())
            .find(|(_, c)| !(c.is_ascii_alphanumeric() || *c == '-' || *c == '_'))
            .map(|(idx, _)| idx)
            .unwrap_or(after.len());
        let token = &after[..tok_end];
        let keep = token.len().min(PREFIX.len() + 4);
        out.push_str(&token[..keep]);
        out.push_str("…REDACTED");
        rest = &after[tok_end..];
    }
    out.push_str(rest);
    out
}

/// Redacts every URL query string found anywhere in a raw log line —
/// unlike `redact_url_query` (which takes a whole URL, used for the
/// already-extracted "captured auth URL" event), this scans arbitrary CLI
/// output for `?` characters and truncates each one's query run at the
/// next whitespace, so it catches the authorize URL wherever it appears
/// in surrounding text (e.g. "Browser didn't open? Use the url below…
/// https://claude.com/…?state=…").
///
/// codex P2 on PR #2410 (second round): `redact_url_query` was only ever
/// applied to the structured "captured auth URL" tracing event — the RAW
/// PTY line logger (`redact_secrets(t)`, a few lines above this file's
/// URL-capture logic) still wrote the full authorize URL, query and all,
/// verbatim to the host log for every line the CLI printed, including the
/// very line the URL was scraped from.
fn redact_url_queries_in_line(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut rest = line;
    while let Some(pos) = rest.find('?') {
        out.push_str(&rest[..=pos]);
        let after = &rest[pos + 1..];
        let end = after.find(char::is_whitespace).unwrap_or(after.len());
        out.push_str("<redacted>");
        rest = &after[end..];
    }
    out.push_str(rest);
    out
}

/// Strips the query string from a captured authorize URL before logging.
/// codex P2 on PR #2410: the query carries `state`/`code_challenge` —
/// SPEC_INAPP_CLAUDE_OAUTH_LOGIN_2026_08_03.md §5 explicitly says never
/// log those. Keeps scheme+host+path (still useful for confirming WHICH
/// provider/endpoint was captured) and appends a marker so a reader knows
/// the query was intentionally dropped, not lost.
fn redact_url_query(url: &str) -> String {
    match url.find('?') {
        Some(idx) => format!("{}?<redacted>", &url[..idx]),
        None => url.to_string(),
    }
}

/// Extract an OAuth URL from a line of CLI output.
/// Strips ANSI escape sequences and looks for `https://...` substrings.
fn extract_url(line: &str) -> Option<String> {
    // Strip ANSI escapes. Two families matter here:
    //   * CSI  — `ESC [ … <final 0x40..=0x7e>` (colors, cursor moves)
    //   * OSC  — `ESC ] … (BEL | ST)` — the Claude CLI can, in principle,
    //     emit an OSC-8 hyperlink here (embeds the URL in the sequence
    //     params AND repeats it as visible link text), though a live
    //     capture under the fixed PTY (see cols comment in run_cli_login)
    //     only ever showed an OSC-0 window-title sequence, no OSC-8 — so
    //     this is defense-in-depth, not the thing that actually fixed
    //     #2429's client_id truncation (the PTY width did). A naive pass
    //     that only knew CSI left the raw `]8;;https://…<BEL>` in place and
    //     captured the URL twice (doubled) whenever OSC-8 IS present, so we
    //     still discard the OSC sequence but stash any URI it carried as a
    //     fallback.
    let mut clean = String::with_capacity(line.len());
    let mut osc_uris: Vec<String> = Vec::new();
    let bytes = line.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == 0x1b && i + 1 < bytes.len() && bytes[i + 1] == b'[' {
            // CSI: ESC [ … <final byte in 0x40..=0x7e>
            i += 2;
            while i < bytes.len() && !(0x40..=0x7e).contains(&bytes[i]) {
                i += 1;
            }
            i += 1; // consume the final byte
        } else if bytes[i] == 0x1b && i + 1 < bytes.len() && bytes[i + 1] == b']' {
            // OSC: ESC ] … terminated by BEL (0x07) or ST (ESC \).
            let seq_start = i + 2;
            i = seq_start;
            let mut seq_end = bytes.len();
            while i < bytes.len() {
                if bytes[i] == 0x07 {
                    seq_end = i;
                    i += 1;
                    break;
                }
                if bytes[i] == 0x1b && i + 1 < bytes.len() && bytes[i + 1] == b'\\' {
                    seq_end = i;
                    i += 2;
                    break;
                }
                i += 1;
            }
            // OSC-8 hyperlink: "8;<params>;<URI>". Stash the URI as a fallback
            // in case the visible link text isn't itself the URL.
            if let Ok(seq) = std::str::from_utf8(&bytes[seq_start..seq_end]) {
                if let Some(rest) = seq.strip_prefix("8;") {
                    if let Some(uri) = rest.splitn(2, ';').nth(1) {
                        if !uri.is_empty() {
                            osc_uris.push(uri.to_string());
                        }
                    }
                }
            }
        } else if bytes[i] == 0x1b {
            // Lone / unrecognised ESC: drop the ESC byte.
            i += 1;
        } else {
            clean.push(bytes[i] as char);
            i += 1;
        }
    }

    // Find https:// and extract until whitespace, a quote, or a stray BEL.
    let pick = |s: &str| -> Option<String> {
        let start = s.find("https://")?;
        let rest = &s[start..];
        let end = rest
            .find(|c: char| c.is_whitespace() || c == '"' || c == '\'' || c == '\u{7}')
            .unwrap_or(rest.len());
        let url = &rest[..end];
        if url.contains("oauth") || url.contains("auth") || url.contains("login") {
            Some(url.to_string())
        } else {
            None
        }
    };

    // Prefer any OSC-8 URI: it's carried inside an escape-sequence payload,
    // so it can never be truncated by the terminal's column-width wrapping.
    // The visible text is only a fallback for CLIs that don't emit OSC-8 —
    // it CAN be wrapped mid-URL by the PTY (see issue #2429 follow-up: the
    // plain "If the browser didn't open, visit: ..." line got hard-wrapped
    // at col 80, silently dropping `client_id` from the captured URL).
    osc_uris
        .iter()
        .find_map(|u| pick(u))
        .or_else(|| pick(&clean))
}

/// Kill the in-progress CLI login process. Covers both transports:
/// the pipe path uses a oneshot to drop the Tokio Child (kill_on_drop
/// terminates the subprocess); the PTY path uses platform-specific
/// kill-by-PID because the `portable_pty::Child` lives inside a
/// `spawn_blocking` task that doesn't react to outer-task abort.
pub fn cancel_cli_login(state: &Arc<AppState>) -> Result<serde_json::Value, String> {
    // Pipe path.
    let sender = {
        let mut stored = state.cli_login_cancel.lock();
        stored.take()
    };
    if let Some(tx) = sender {
        let _ = tx.send(());
        tracing::info!("cancel_cli_login: pipe-path cancel signal sent");
    }
    // PTY path.
    let pid = {
        let mut stored = state.cli_login_pty_pid.lock();
        stored.take()
    };
    if let Some(pid) = pid {
        if let Err(e) = kill_pid(pid) {
            tracing::warn!(pid, error = %e, "cancel_cli_login: kill_pid failed");
        } else {
            tracing::info!(pid, "cancel_cli_login: PTY child killed");
        }
    }
    Ok(serde_json::Value::Null)
}

/// Reads the provider's config-dir env var (`config_dir_env_var` — e.g.
/// "CLAUDE_CONFIG_DIR", "OPENCLAW_HOME") out of the spawn's `auth_env` and
/// stats its `.credentials.json` (same path convention the PTY reap's
/// post-login existence check already uses, ~line 773). `None` inner value
/// = the file doesn't exist yet (a fresh mint — any later appearance
/// counts as "changed"). Returns `None` entirely when the named env var
/// wasn't passed (nothing to compare against; `get_cli_login_status`
/// treats that as "can't tell, don't block on it").
///
/// reagent P1 on PR #2410: this used to hardcode "CLAUDE_CONFIG_DIR",
/// silently no-op'ing the freshness guard for every OTHER
/// requiresLoginTty/awaitTier1Completion provider (OpenClaw uses
/// OPENCLAW_HOME) — auth_env simply had no matching key, so this always
/// returned None and credential_changed always read true, reopening the
/// exact stale-credential-reconnect false-positive the guard exists to
/// close, just for OpenClaw instead of Claude.
fn capture_cred_baseline(
    auth_env: &std::collections::HashMap<String, String>,
    config_dir_env_var: &str,
) -> Option<(std::path::PathBuf, Option<std::time::SystemTime>)> {
    let dir = auth_env.get(config_dir_env_var)?;
    let path = std::path::Path::new(dir).join(".credentials.json");
    let mtime = std::fs::metadata(&path).ok().and_then(|m| m.modified().ok());
    Some((path, mtime))
}

/// Report whether a CLI login child is still in flight.
///
/// `active` is derived from `cli_login_active`, which both spawn transports
/// (pipe and PTY) set at spawn and whose reaper task clears on child exit /
/// cancel / reap-timeout — generation-guarded, so a superseding login that
/// repopulated the slot correctly reads as active. Deliberately NOT derived
/// from `cli_login_stdin`'s presence: `set_provider_auth` `.take()`s that
/// slot the instant a pasted code is delivered (single-use), but the child
/// keeps running afterward while it exchanges the code — reading the stdin
/// slot here reported `active: false` the moment a code was pasted, which
/// could time out and kill a login that was still genuinely completing
/// (codex P1 on PR #2410).
///
/// Added for the in-app Claude login session
/// (SPEC_INAPP_CLAUDE_OAUTH_LOGIN_2026_08_03.md §3.1): tier-1 completion is
/// "child exited successfully AND credential material exists in the isolated
/// dir". The frontend already probes the credential half via
/// CheckCliAuthCommand against the isolated CLAUDE_CONFIG_DIR; this supplies
/// the child-exit half, which run_cli_login can't return (it responds as
/// soon as the URL is captured, while the child lives on). Without it, a
/// reconnect into an EXISTING account dir would false-positive on the very
/// first credential probe (a present-but-expired token still reports
/// "authenticated" — see force-login.ts's doc comment) and reap the login
/// child before the user ever finished authorizing.
pub fn get_cli_login_status(state: &Arc<AppState>) -> Result<serde_json::Value, String> {
    let active = state
        .cli_login_active
        .load(std::sync::atomic::Ordering::SeqCst);
    // credential_changed: has the credential file's mtime moved (or did it
    // start existing) since the baseline captured right before THIS
    // attempt spawned? `None` baseline (no CLAUDE_CONFIG_DIR was passed)
    // reports `true` — nothing to compare against, so don't block callers
    // that have no way to supply this. See cli_login_cred_baseline's doc
    // comment for why this is required alongside `active` for a caller to
    // trust a completion signal (reagent P1 on PR #2410).
    let credential_changed = match &*state.cli_login_cred_baseline.lock() {
        None => true,
        Some((path, baseline_mtime)) => {
            let current_mtime = std::fs::metadata(path).ok().and_then(|m| m.modified().ok());
            current_mtime != *baseline_mtime
        }
    };
    // codex P2 on PR #2410: without this, a poll that started against
    // generation N can't tell it's been superseded by generation N+1 (a
    // DIFFERENT surface starting a newer login) — it would keep reading
    // the newer child's `active`/`credential_changed` as if they were its
    // own, and its unconditional cancelCliLogin() on timeout would kill
    // that unrelated newer login. Callers capture this on their first read
    // and treat any later mismatch as "no longer mine" rather than
    // "timed out" — see pollForInAppLoginCompletion's own doc comment.
    let generation = state
        .cli_login_generation
        .load(std::sync::atomic::Ordering::SeqCst);
    Ok(serde_json::json!({ "active": active, "credential_changed": credential_changed, "generation": generation }))
}

/// Single-quote a value for embedding in the POSIX launch script
/// `open_login_terminal` writes on macOS.
#[cfg(target_os = "macos")]
fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// Spawn the CLI login command in a NEW visible console window so the OS can
/// open a browser (the piped/PTY paths used by `run_cli_login` are headless
/// and block the browser from launching — confirmed for Claude v2.1.x).
///
/// Fire-and-forget: returns immediately; the frontend polls the CLI's own
/// auth-check command (`pollForCliAuthReady`) against the isolated config dir
/// the login was pointed at, and registers the account once it reports
/// authenticated. It previously polled `seed_provider_auth_from_global`, which
/// copied the credential in from the user's personal `~/.claude` — removed
/// 2026-08-31 (per-channel auth enforcement).
pub fn open_login_terminal(args: &serde_json::Value) -> Result<serde_json::Value, String> {
    let cli_path = args
        .get("cli_path")
        .or_else(|| args.get("cliPath"))
        .and_then(|v| v.as_str())
        .ok_or("open_login_terminal: missing cliPath")?;

    let login_args: Vec<String> = args
        .get("loginArgs")
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();

    let auth_env: std::collections::HashMap<String, String> = args
        .get("authEnv")
        .and_then(|v| v.as_object())
        .map(|obj| {
            obj.iter()
                .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                .collect()
        })
        .unwrap_or_default();

    // Quote the CLI path if it contains spaces (cmd.exe /k token split).
    let quoted_cli = if cli_path.contains(' ') {
        format!("\"{}\"", cli_path.replace('"', ""))
    } else {
        cli_path.to_string()
    };
    let cmd_str = if login_args.is_empty() {
        quoted_cli
    } else {
        format!("{} {}", quoted_cli, login_args.join(" "))
    };

    tracing::info!(cmd = %cmd_str, "open_login_terminal: spawning new console");

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // CREATE_NEW_CONSOLE (0x10): the child gets its own visible console
        // window, separate from the host's hidden console. This is what allows
        // the Claude CLI to open the OS default browser for OAuth.
        //
        // Deliberately NOT setting .stdin()/.stdout()/.stderr() here (was
        // .stdin(Stdio::null()) — confirmed live, root-caused this session:
        // the CLI opened a real browser and completed OAuth correctly when
        // run manually with normal stdio, but silently failed to open a
        // browser under this exact spawn once stdin was forced to NUL).
        // Rust only sets STARTF_USESTDHANDLES (which overrides whatever
        // handles CREATE_NEW_CONSOLE would otherwise wire up for the new
        // console) when a stdio method is explicitly called; leaving all
        // three untouched lets CreateProcess give the child fresh handles
        // tied to its own new console, exactly like a normal interactively-
        // launched console app gets. That's required for two things: the
        // CLI's own TTY detection (gates whether it attempts the browser-
        // open call at all) AND the "Paste code here if prompted >" manual
        // fallback this same command prints — impossible to use with a
        // NUL'd stdin regardless of the browser-open outcome.
        const CREATE_NEW_CONSOLE: u32 = 0x0000_0010;
        std::process::Command::new("cmd.exe")
            .args(["/k", &cmd_str])
            .envs(&auth_env)
            .creation_flags(CREATE_NEW_CONSOLE)
            .spawn()
            .map_err(|e| format!("open_login_terminal: spawn failed: {e}"))?;
    }

    #[cfg(target_os = "macos")]
    {
        // `open -a Terminal` can't carry env vars or a shell command directly,
        // so write a disposable script and open Terminal.app on that instead.
        // The script self-deletes on exit so repeated logins don't litter tmp.
        //
        // reagent P1 on #2255: the old name (`agentmux-login-{pid}.command`,
        // just the host process's own PID) is CONSTANT for the process's
        // whole lifetime — a second login attempt (a different agent, or a
        // retry) before the first script finishes executing overwrote the
        // same path, so a running/about-to-run shell could execute mixed or
        // wrong content, or the self-delete (`rm -f -- "$0"`) could remove
        // the file out from under a concurrent attempt. A UUID per call
        // makes every attempt's script path unique regardless of timing.
        let script_path = std::env::temp_dir().join(format!(
            "agentmux-login-{}-{}.command",
            std::process::id(),
            uuid::Uuid::new_v4(),
        ));
        let mut script = String::from("#!/bin/sh\n");
        for (k, v) in &auth_env {
            script.push_str(&format!("export {}={}\n", k, shell_quote(v)));
        }
        script.push_str(&format!("{}\nrm -f -- \"$0\"\n", cmd_str));
        // reagent P1 on #2260 (surfaced via this file's inclusion in that
        // PR's diff, but the code — and the fix — belong here): the old
        // `fs::write` then `set_permissions(0o700)` sequence left a TOCTOU
        // window where the pid-named script, containing exported auth_env
        // values, was briefly world-readable in the shared temp dir under a
        // default umask. Setting the mode AT CREATION via OpenOptions closes
        // that window — the file never exists with broader permissions.
        use std::os::unix::fs::OpenOptionsExt;
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o700)
            .open(&script_path)
            .map_err(|e| format!("open_login_terminal: failed to create launch script: {e}"))?;
        file.write_all(script.as_bytes())
            .map_err(|e| format!("open_login_terminal: failed to write launch script: {e}"))?;
        drop(file);
        std::process::Command::new("open")
            .args(["-a", "Terminal", script_path.to_string_lossy().as_ref()])
            .spawn()
            .map_err(|e| format!("open_login_terminal: spawn failed: {e}"))?;
    }

    #[cfg(target_os = "linux")]
    {
        // No single terminal emulator is guaranteed present; try common ones
        // in order and use whichever spawns successfully. `-e` runs the
        // command and the shell stays open after exit so any CLI error is
        // visible instead of the window vanishing immediately.
        //
        // reagent P2 on #2255: single-instance terminal emulators like
        // gnome-terminal proxy the "open a window" request to an
        // already-running server process via D-Bus — the freshly spawned
        // client process just sends that message and exits, so `.envs()`
        // on THIS Command never reaches the shell the server process
        // actually runs. Embed the env vars as `export` lines in the shell
        // command text itself instead (same approach the macOS branch
        // already uses) — that text is what actually gets relayed to the
        // server-owned shell, regardless of which process env it came from.
        let mut export_prelude = String::new();
        for (k, v) in &auth_env {
            export_prelude.push_str(&format!("export {}='{}'; ", k, v.replace('\'', "'\\''")));
        }
        let sh_cmd = format!(
            "{}{}; echo; echo '[login finished — press Enter to close]'; read _",
            export_prelude, cmd_str,
        );
        let candidates: [(&str, &[&str]); 4] = [
            ("x-terminal-emulator", &["-e", "sh", "-c"]),
            ("gnome-terminal", &["--", "sh", "-c"]),
            ("konsole", &["-e", "sh", "-c"]),
            ("xterm", &["-e", "sh", "-c"]),
        ];
        let mut spawned = false;
        for (bin, prefix_args) in candidates {
            let mut command = std::process::Command::new(bin);
            // Still set .envs() too — harmless for emulators that DON'T
            // proxy via a persistent server (xterm, konsole typically
            // fork a fresh process per invocation and would inherit it),
            // and costs nothing for the ones that ignore it.
            command.args(prefix_args).arg(&sh_cmd).envs(&auth_env);
            if command.spawn().is_ok() {
                spawned = true;
                break;
            }
        }
        if !spawned {
            return Err(
                "open_login_terminal: no terminal emulator found (tried x-terminal-emulator, gnome-terminal, konsole, xterm)"
                    .to_string(),
            );
        }
    }

    Ok(serde_json::json!({ "opened": true }))
}

/// Platform-specific best-effort kill of a child process by PID.
#[cfg(windows)]
fn kill_pid(pid: u32) -> std::io::Result<()> {
    // Use taskkill /F /T so the whole tree dies — `openclaw models
    // auth login` typically spawns a child that opens the browser.
    let status = std::process::Command::new("taskkill")
        .args(["/F", "/T", "/PID", &pid.to_string()])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()?;
    if status.success() {
        Ok(())
    } else {
        Err(std::io::Error::other(format!("taskkill exit {:?}", status.code())))
    }
}

#[cfg(unix)]
fn kill_pid(pid: u32) -> std::io::Result<()> {
    // SIGTERM first; an aborting subprocess gets a chance to clean up.
    let ret = unsafe { libc::kill(pid as i32, libc::SIGTERM) };
    if ret != 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

// --- CLI command helpers ---

fn make_cli_cmd(cli_path: &str) -> tokio::process::Command {
    agentmux_common::make_cli_cmd(cli_path)
}

#[cfg(test)]
mod redact_tests {
    use super::redact_secrets;

    #[test]
    fn masks_oauth_token_keeps_prefix() {
        let red = redact_secrets("CLAUDE_CODE_OAUTH_TOKEN=sk-ant-oat01-AbCdEf123456789");
        assert!(red.starts_with("CLAUDE_CODE_OAUTH_TOKEN=sk-ant-oat0"));
        assert!(red.contains("…REDACTED"));
        assert!(!red.contains("AbCdEf123456789"));
    }

    #[test]
    fn masks_bare_token_and_preserves_surroundings() {
        let red = redact_secrets("  token: sk-ant-api03-SECRETSECRETSECRET done");
        assert!(!red.contains("SECRETSECRETSECRET"));
        assert!(red.contains("sk-ant-api0")); // prefix + 4 kept
        assert!(red.contains("…REDACTED"));
        assert!(red.contains(" done")); // trailing text preserved
    }

    #[test]
    fn passes_through_non_secret_lines() {
        let s = "Visit https://claude.ai/oauth?code=xyz to continue";
        assert_eq!(redact_secrets(s), s);
    }

    #[test]
    fn masks_multiple_occurrences() {
        let red = redact_secrets("sk-ant-oat01-AAAA and sk-ant-oat01-BBBB");
        assert!(!red.contains("AAAA"));
        assert!(!red.contains("BBBB"));
        assert_eq!(red.matches("…REDACTED").count(), 2);
    }
}

#[cfg(test)]
mod url_capture_timeout_tests {
    use super::URL_CAPTURE_TIMEOUT_SECS;

    // Regression guard: this constant bounds every provider that prints a
    // URL — Codex/Gemini/OpenClaw, and (since
    // SPEC_INAPP_CLAUDE_OAUTH_LOGIN_2026_08_03.md revived tier 1) Claude
    // 2.1.198+ too. It doubles as the frontend's behavior-gate window: an
    // older Claude CLI that prints nothing times out here and
    // runProviderLogin falls back to tiers 2/3. reagent P1 on PR #2300: an
    // earlier attempt to shorten this to 5s "as a safety margin" would have
    // killed valid in-progress OpenClaw logins, which can take close to the
    // full 15s to print their URL. Pinned at a sane, bounded value — not
    // "short" for its own sake — so a future edit can't reintroduce that
    // regression.
    #[test]
    fn is_a_sane_bounded_value_for_providers_that_actually_print_a_url() {
        assert!(URL_CAPTURE_TIMEOUT_SECS > 0);
        assert!(URL_CAPTURE_TIMEOUT_SECS <= 30);
    }
}

#[cfg(test)]
mod redact_url_query_tests {
    use super::redact_url_query;

    // codex P2 on PR #2410: SPEC_INAPP_CLAUDE_OAUTH_LOGIN_2026_08_03.md §5
    // says never log the authorize URL's full query (state/code_challenge).
    #[test]
    fn strips_the_query_string() {
        let url = "https://claude.com/cai/oauth/authorize?code=true&state=abc123&code_challenge=xyz";
        assert_eq!(redact_url_query(url), "https://claude.com/cai/oauth/authorize?<redacted>");
    }

    #[test]
    fn leaves_a_query_less_url_untouched() {
        let url = "https://claude.com/cai/oauth/authorize";
        assert_eq!(redact_url_query(url), url);
    }
}

#[cfg(test)]
mod redact_url_queries_in_line_tests {
    use super::redact_url_queries_in_line;

    // codex P2 on PR #2410 (second round): the raw PTY line logger wrote
    // the authorize URL's full query verbatim — redact_url_query alone
    // never reached it, since it was only applied to the separate
    // "captured auth URL" structured event.
    #[test]
    fn redacts_a_url_query_embedded_in_surrounding_text() {
        let line = "Browser didn't open? Use the url below to sign in (c to copy) https://claude.com/cai/oauth/authorize?code=true&state=abc&code_challenge=xyz";
        let redacted = redact_url_queries_in_line(line);
        assert!(redacted.contains("https://claude.com/cai/oauth/authorize?<redacted>"));
        assert!(!redacted.contains("state=abc"));
        assert!(!redacted.contains("code_challenge=xyz"));
        // The leading text (including its OWN "?") is preserved verbatim.
        assert!(redacted.starts_with("Browser didn't open?<redacted> Use the url"));
    }

    #[test]
    fn leaves_a_line_with_no_query_string_untouched() {
        let line = "Paste code here if prompted >";
        assert_eq!(redact_url_queries_in_line(line), line);
    }

    #[test]
    fn redacts_multiple_query_strings_in_the_same_line() {
        let line = "a?x=1 b?y=2";
        assert_eq!(redact_url_queries_in_line(line), "a?<redacted> b?<redacted>");
    }
}

#[cfg(test)]
mod dsr_responding_reader_tests {
    use super::{DsrRespondingReader, DSR_CURSOR_POSITION_QUERY};
    use std::cell::Cell;
    use std::io::Read;

    /// Yields each element of `chunks` on successive `read()` calls,
    /// copying as much as fits in the caller's buffer — lets a test force
    /// a specific byte-boundary split, which a plain `std::io::Cursor`
    /// (single-buffer, fills as much as requested in one call) can't do.
    struct ChunkedReader {
        chunks: std::collections::VecDeque<Vec<u8>>,
    }

    impl ChunkedReader {
        fn new(chunks: Vec<&[u8]>) -> Self {
            Self { chunks: chunks.into_iter().map(|c| c.to_vec()).collect() }
        }
    }

    impl Read for ChunkedReader {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            let Some(chunk) = self.chunks.front_mut() else { return Ok(0) };
            let n = chunk.len().min(buf.len());
            buf[..n].copy_from_slice(&chunk[..n]);
            chunk.drain(..n);
            if chunk.is_empty() {
                self.chunks.pop_front();
            }
            Ok(n)
        }
    }

    #[test]
    fn invokes_callback_when_query_arrives_in_one_read() {
        let inner = ChunkedReader::new(vec![DSR_CURSOR_POSITION_QUERY]);
        let calls = Cell::new(0);
        let mut r = DsrRespondingReader::new(inner, || calls.set(calls.get() + 1));
        let mut buf = [0u8; 64];
        let n = r.read(&mut buf).unwrap();
        assert_eq!(&buf[..n], DSR_CURSOR_POSITION_QUERY);
        assert_eq!(calls.get(), 1, "callback must fire exactly once for the query");
    }

    #[test]
    fn does_not_invoke_callback_for_unrelated_bytes() {
        let inner = ChunkedReader::new(vec![b"Opening browser to sign in...\r\n"]);
        let calls = Cell::new(0);
        let mut r = DsrRespondingReader::new(inner, || calls.set(calls.get() + 1));
        let mut buf = [0u8; 64];
        let n = r.read(&mut buf).unwrap();
        assert_eq!(&buf[..n], b"Opening browser to sign in...\r\n");
        assert_eq!(calls.get(), 0);
    }

    #[test]
    fn detects_query_split_across_two_read_calls() {
        // The exact failure mode a naive single-read scan would miss:
        // "\x1b[6" in one chunk, "n" arriving in the next.
        let inner = ChunkedReader::new(vec![b"\x1b[6", b"n"]);
        let calls = Cell::new(0);
        let mut r = DsrRespondingReader::new(inner, || calls.set(calls.get() + 1));
        let mut buf = [0u8; 64];
        let n1 = r.read(&mut buf).unwrap();
        assert_eq!(&buf[..n1], b"\x1b[6");
        assert_eq!(calls.get(), 0, "must not fire on the incomplete first half");
        let n2 = r.read(&mut buf).unwrap();
        assert_eq!(&buf[..n2], b"n");
        assert_eq!(calls.get(), 1, "must fire once the second half completes the pattern");
    }

    #[test]
    fn passes_every_byte_through_unchanged_query_included() {
        // The query's own bytes are harmless noise to the line-scanning
        // loop downstream — verify they're never stripped, only observed.
        let payload = [DSR_CURSOR_POSITION_QUERY, b"Paste code here if prompted > "].concat();
        let inner = ChunkedReader::new(vec![&payload]);
        let mut r = DsrRespondingReader::new(inner, || {});
        let mut out = Vec::new();
        r.read_to_end(&mut out).unwrap();
        assert_eq!(out, payload);
    }

    #[test]
    fn does_not_refire_on_subsequent_unrelated_reads_after_the_query() {
        let inner = ChunkedReader::new(vec![DSR_CURSOR_POSITION_QUERY, b"more output\r\n"]);
        let calls = Cell::new(0);
        let mut r = DsrRespondingReader::new(inner, || calls.set(calls.get() + 1));
        let mut buf = [0u8; 64];
        r.read(&mut buf).unwrap();
        assert_eq!(calls.get(), 1);
        r.read(&mut buf).unwrap();
        assert_eq!(calls.get(), 1, "later unrelated bytes must not re-trigger the callback");
    }
}

#[cfg(test)]
mod capture_cred_baseline_tests {
    use super::capture_cred_baseline;
    use std::collections::HashMap;

    #[test]
    fn returns_none_when_no_config_dir_env_var_is_present() {
        let auth_env = HashMap::new();
        assert!(capture_cred_baseline(&auth_env, "CLAUDE_CONFIG_DIR").is_none());
    }

    #[test]
    fn reports_none_mtime_for_a_credential_file_that_does_not_exist_yet() {
        let dir = std::env::temp_dir().join(format!("cli-login-baseline-test-{}", std::process::id()));
        let mut auth_env = HashMap::new();
        auth_env.insert("CLAUDE_CONFIG_DIR".to_string(), dir.to_string_lossy().to_string());

        let (path, mtime) = capture_cred_baseline(&auth_env, "CLAUDE_CONFIG_DIR").expect("config dir was set");
        assert_eq!(path, dir.join(".credentials.json"));
        assert!(mtime.is_none(), "fresh mint: no credential file should exist yet");
    }

    #[test]
    fn reports_a_real_mtime_for_an_existing_credential_file() {
        let dir = std::env::temp_dir().join(format!("cli-login-baseline-test-existing-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let cred_path = dir.join(".credentials.json");
        std::fs::write(&cred_path, "{}").unwrap();

        let mut auth_env = HashMap::new();
        auth_env.insert("CLAUDE_CONFIG_DIR".to_string(), dir.to_string_lossy().to_string());

        let (_, mtime) = capture_cred_baseline(&auth_env, "CLAUDE_CONFIG_DIR").expect("config dir was set");
        assert!(mtime.is_some(), "reconnect: the stale credential's baseline mtime must be captured");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn reagent_p1_on_pr_2410_uses_the_providers_own_env_var_not_a_hardcoded_claude_one() {
        // OpenClaw uses OPENCLAW_HOME, not CLAUDE_CONFIG_DIR — before this
        // fix, capture_cred_baseline hardcoded the latter, so this exact
        // auth_env (a real OpenClaw spawn's) always returned None,
        // silently disabling the freshness guard for OpenClaw entirely.
        let dir = std::env::temp_dir().join(format!("cli-login-baseline-test-openclaw-{}", std::process::id()));
        let mut auth_env = HashMap::new();
        auth_env.insert("OPENCLAW_HOME".to_string(), dir.to_string_lossy().to_string());

        assert!(capture_cred_baseline(&auth_env, "CLAUDE_CONFIG_DIR").is_none());
        assert!(capture_cred_baseline(&auth_env, "OPENCLAW_HOME").is_some());
    }
}

#[cfg(test)]
mod extract_url_claude_authorize_tests {
    use super::extract_url;

    // Pins tier-1 URL capture for the revived in-app Claude login
    // (SPEC_INAPP_CLAUDE_OAUTH_LOGIN_2026_08_03.md §2): the pinned CLI
    // (2.1.198+) prints the PKCE authorize URL as a plain fallback line —
    // and, in some renderings, OSC-8-hyperlink-wrapped. Both forms were
    // observed in the 2026-08-03 live probes and both must yield the exact
    // URL (not doubled, not truncated) or tier 1 silently falls back to the
    // terminal tiers for a CLI that fully supports the in-app flow.
    const AUTHORIZE_URL: &str = "https://claude.com/cai/oauth/authorize?code=true&client_id=abc-123&code_challenge=xyz_456&code_challenge_method=S256&state=st-789";

    #[test]
    fn captures_plain_fallback_line() {
        let line = format!("If the browser didn't open, visit: {AUTHORIZE_URL}");
        assert_eq!(extract_url(&line), Some(AUTHORIZE_URL.to_string()));
    }

    #[test]
    fn captures_osc8_hyperlink_bel_terminated() {
        // OSC-8 with the URL as both the sequence param and the visible link
        // text (what the CLI actually emits) — the de-escaped visible text
        // must win, single and intact.
        let line = format!(
            "If the browser didn't open, visit: \u{1b}]8;;{AUTHORIZE_URL}\u{7}{AUTHORIZE_URL}\u{1b}]8;;\u{7}"
        );
        assert_eq!(extract_url(&line), Some(AUTHORIZE_URL.to_string()));
    }

    #[test]
    fn captures_osc8_hyperlink_st_terminated_with_non_url_link_text() {
        // ST-terminated OSC-8 whose visible text is NOT the URL — the URI
        // stashed from the escape sequence itself must be used as fallback.
        let line = format!(
            "Visit \u{1b}]8;;{AUTHORIZE_URL}\u{1b}\\this link\u{1b}]8;;\u{1b}\\ to sign in"
        );
        assert_eq!(extract_url(&line), Some(AUTHORIZE_URL.to_string()));
    }

    #[test]
    fn captures_url_wrapped_in_csi_color_codes() {
        let line = format!("\u{1b}[1m\u{1b}[36m{AUTHORIZE_URL}\u{1b}[0m");
        assert_eq!(extract_url(&line), Some(AUTHORIZE_URL.to_string()));
    }

    #[test]
    fn ignores_non_auth_urls() {
        assert_eq!(extract_url("see https://claude.com/docs for details"), None);
    }

    #[test]
    fn prefers_osc8_uri_when_the_visible_fallback_line_was_wrapped_by_the_pty() {
        // Regression for the #2429 follow-up: at an 80-column PTY width, the
        // CLI's own line-wrapping of the plain "visit: ..." text can split
        // the URL mid-query-string before a `\r\n` is ever reached, so the
        // OSC-8 payload (never subject to that wrapping) is the only place
        // the full URL — with client_id intact — actually appears.
        let truncated_visible = "https://claude.com/cai/oauth/authorize?code=t";
        let line = format!(
            "\u{1b}]8;;{AUTHORIZE_URL}\u{7}link text\u{1b}]8;;\u{7}If the browser didn't open, visit: {truncated_visible}"
        );
        assert_eq!(extract_url(&line), Some(AUTHORIZE_URL.to_string()));
    }
}

#[cfg(all(test, unix))]
mod spawn_login_pty_tests {
    use super::spawn_login_pty_unix;
    use std::collections::HashMap;
    use std::io::{BufRead, BufReader, Read};

    // Exercises the hand-rolled PTY spawn end to end: openpty → pre_exec
    // (signal reset, setsid, TIOCSCTTY, close_range fd hygiene) → exec →
    // read the child's stdout off the master. The unit-test binary does not
    // link libcef, so the interposed-close crash can't reproduce here; what
    // this pins is that the replacement spawn path is itself correct (a
    // regression guard for the openpty/dup/stdio wiring). The live
    // libcef-interposition proof is in the retro.
    #[test]
    fn spawns_program_on_pty_and_captures_stdout() {
        let env: HashMap<String, String> = HashMap::new();
        let (mut child, master) = spawn_login_pty_unix(
            "/bin/echo",
            &["hello-from-pty".to_string()],
            &env,
        )
        .expect("spawn on pty");

        let mut reader = BufReader::new(master.try_clone().expect("clone master"));
        let mut line = String::new();
        reader.read_line(&mut line).expect("read pty line");
        assert!(
            line.contains("hello-from-pty"),
            "expected child stdout on the pty, got {line:?}"
        );

        // Drain to EOF so the child isn't blocked on a full pty buffer, then reap.
        let mut rest = String::new();
        let _ = reader.read_to_string(&mut rest);
        let status = child.wait().expect("wait child");
        assert!(status.success(), "echo should exit 0, got {status:?}");
    }

    // fd hygiene: an inherited, explicitly non-CLOEXEC fd ≥ 3 must NOT be
    // visible in the exec'd child. Proves the pre_exec CLOEXEC marking
    // (close_range on this kernel, fcntl fallback elsewhere) actually
    // neutralizes leaked host fds — the reagent P2 concern that the old
    // outright-close fallback both leaked (ENOSYS) and clobbered std's
    // exec-error pipe. We open a pipe, clear its CLOEXEC flag so it WOULD
    // leak, then assert the child can't see that fd number.
    #[test]
    fn inherited_non_cloexec_fd_does_not_leak_into_child() {
        use std::os::fd::AsRawFd;

        // A pipe read end with CLOEXEC deliberately cleared → without our
        // hygiene it would be inherited across exec.
        let (rd, _wr) = nix_pipe();
        let leaked = rd.as_raw_fd();
        unsafe {
            let flags = libc::fcntl(leaked, libc::F_GETFD);
            assert!(flags >= 0);
            libc::fcntl(leaked, libc::F_SETFD, flags & !libc::FD_CLOEXEC);
        }

        let env: HashMap<String, String> = HashMap::new();
        let script = format!("test -e /proc/self/fd/{leaked} && echo LEAKED || echo CLEAN");
        let (mut child, master) =
            spawn_login_pty_unix("/bin/sh", &["-c".to_string(), script], &env)
                .expect("spawn on pty");

        let mut out = String::new();
        let _ = BufReader::new(master).read_to_string(&mut out);
        let _ = child.wait();
        assert!(
            out.contains("CLEAN") && !out.contains("LEAKED"),
            "fd {leaked} leaked into the child; hygiene failed. child said: {out:?}"
        );
    }

    // Minimal pipe(2) wrapper (avoids pulling nix into the test).
    fn nix_pipe() -> (std::fs::File, std::fs::File) {
        use std::os::fd::FromRawFd;
        let mut fds = [0i32; 2];
        let rc = unsafe { libc::pipe(fds.as_mut_ptr()) };
        assert_eq!(rc, 0, "pipe() failed");
        unsafe {
            (
                std::fs::File::from_raw_fd(fds[0]),
                std::fs::File::from_raw_fd(fds[1]),
            )
        }
    }

    // A child on its own session with the pty as controlling terminal must
    // see a TTY on stdin (proves setsid + TIOCSCTTY took effect — the reason
    // Claude's `auth login` runs its interactive flow instead of bailing).
    #[test]
    fn child_stdin_is_a_tty() {
        let env: HashMap<String, String> = HashMap::new();
        let (mut child, master) = spawn_login_pty_unix(
            "/bin/sh",
            &["-c".to_string(), "test -t 0 && echo IS_TTY || echo NO_TTY".to_string()],
            &env,
        )
        .expect("spawn on pty");

        let mut out = String::new();
        let _ = BufReader::new(master).read_to_string(&mut out);
        let _ = child.wait();
        assert!(out.contains("IS_TTY"), "child stdin should be a tty, got {out:?}");
    }
}
