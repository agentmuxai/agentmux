// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Platform info commands for the CEF host.
// Ported from src-tauri/src/commands/platform.rs without Tauri dependencies.

use std::io::Read;
use std::sync::Arc;

use crate::state::AppState;

const SETTINGS_TEMPLATE: &str = include_str!("../../../settings-template.jsonc");

/// Get the current OS platform name.
pub fn get_platform() -> serde_json::Value {
    let platform = match std::env::consts::OS {
        "macos" => "darwin",
        "windows" => "win32",
        other => other,
    };
    serde_json::json!(platform)
}

/// Get the current user's username.
pub fn get_user_name() -> serde_json::Value {
    serde_json::json!(whoami::username())
}

/// Get the system hostname.
pub fn get_host_name() -> serde_json::Value {
    let hostname = whoami::fallible::hostname().unwrap_or_else(|_| "unknown".to_string());
    serde_json::json!(hostname)
}

/// Check if running in development mode — resolved from the runtime
/// `RuntimeMode` (launcher-injected env, or the host exe path).
pub fn get_is_dev() -> serde_json::Value {
    let mode = agentmux_common::RuntimeMode::from_env().or_else(|| {
        std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.to_path_buf()))
            .map(|d| agentmux_common::RuntimeMode::current(&d))
    });
    serde_json::json!(matches!(mode, Some(agentmux_common::RuntimeMode::Dev { .. })))
}

/// Get the app data directory path (version-specific).
pub fn get_data_dir(state: &Arc<AppState>) -> Result<serde_json::Value, String> {
    let dir = state.version_data_dir.lock();
    match dir.as_ref() {
        Some(d) => Ok(serde_json::json!(d)),
        None => Err("Data dir not initialized yet".to_string()),
    }
}

/// Get the app config directory path (version-specific).
pub fn get_config_dir(state: &Arc<AppState>) -> Result<serde_json::Value, String> {
    let dir = state.version_config_dir.lock();
    match dir.as_ref() {
        Some(d) => Ok(serde_json::json!(d)),
        None => Err("Config dir not initialized yet".to_string()),
    }
}

/// Get the AgentMux account-wide root (`~/.agentmux/`) — `user_home_dir`, set
/// from `paths.home_dir` (sidecar.rs; the same root in portable / installed /
/// override modes, not a per-channel or `<portable>/data` subdir). Used by the
/// frontend for per-agent paths (working dir, `GH_CONFIG_DIR`) and as the root
/// of the shared provider auth dir (`ensure_auth_dir`).
pub fn get_user_home_dir(state: &Arc<AppState>) -> Result<serde_json::Value, String> {
    let dir = state.user_home_dir.lock();
    match dir.as_ref() {
        Some(d) => Ok(serde_json::json!(d)),
        None => Err("User home dir not initialized yet".to_string()),
    }
}

/// Ensure a provider auth directory exists and return its absolute path.
/// The DEFAULT provider auth lives in the account-wide, version- and
/// channel-independent `~/.agentmux/shared/providers/<provider>/` — the
/// per-identity bundle override (identity_handlers) still wins for explicit
/// multi-account.
pub fn ensure_auth_dir(
    state: &Arc<AppState>,
    args: &serde_json::Value,
) -> Result<serde_json::Value, String> {
    let provider_id = args
        .get("provider_id")
        .or_else(|| args.get("providerId"))
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing provider_id".to_string())?;

    // Reject path traversal attempts in provider_id
    if provider_id.contains('/')
        || provider_id.contains('\\')
        || provider_id.contains("..")
        || provider_id.is_empty()
    {
        return Err(format!(
            "Invalid provider_id '{}': must not contain path separators or '..'",
            provider_id
        ));
    }

    // The DEFAULT provider auth/config dir lives under the account-wide, version-
    // and channel-independent shared root (`~/.agentmux/shared/providers/<provider>/`),
    // NOT the per-channel config dir. One login is shared across every instance /
    // channel / version — the structural fix for the per-channel validate-spin
    // regression (docs/retro/retro-provider-auth-isolation-regression-2026-06-05.md).
    // The per-identity bundle override (identity_handlers) still wins for explicit
    // multi-account. `user_home_dir` is the AgentMux root (`~/.agentmux/`).
    let home = state.user_home_dir.lock();
    let home = home
        .as_ref()
        .ok_or_else(|| "Home dir not initialized yet".to_string())?;

    let auth_dir = std::path::PathBuf::from(home)
        .join("shared")
        .join("providers")
        .join(provider_id);
    std::fs::create_dir_all(&auth_dir)
        .map_err(|e| format!("Failed to create auth dir for {}: {}", provider_id, e))?;

    Ok(serde_json::json!(auth_dir.to_string_lossy()))
}

/// Get an environment variable value.
pub fn get_env(args: &serde_json::Value) -> serde_json::Value {
    let key = args
        .get("key")
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    match std::env::var(key) {
        Ok(val) => serde_json::json!(val),
        Err(_) => serde_json::Value::Null,
    }
}

/// The ephemeral build label of a local portable, read from the
/// `agentmux-portable.marker` file the packaging script writes next to
/// the launcher (`scripts/package-portable.sh`). Format on disk:
/// `AgentMux portable build <label>\n`, e.g.
/// `AgentMux portable build 0.39.2+g9dd2d78.dirty.20260528T2203.21046`.
///
/// Read from the MARKER (not baked via `option_env!`) on purpose: the
/// label changes every build, and the marker is rewritten on every
/// `task package`, so reading it at runtime is always accurate and
/// costs no recompile — whereas a compile-time bake would go stale any
/// time `agentmux-cef` itself wasn't rebuilt (e.g. a frontend-only
/// rebuild). Released / installed / dev builds have no marker → returns
/// `None` and the UI falls back to the plain version + git hash.
///
/// The host runs from `<portable>/runtime/`, so the marker sits one
/// level up; we also check the exe dir itself for layout robustness.
fn read_build_label() -> Option<String> {
    let exe = std::env::current_exe().ok()?;
    let exe_dir = exe.parent()?;
    let candidates = [
        exe_dir.parent().map(|p| p.join("agentmux-portable.marker")),
        Some(exe_dir.join("agentmux-portable.marker")),
    ];
    for cand in candidates.into_iter().flatten() {
        if let Ok(contents) = std::fs::read_to_string(&cand) {
            if let Some(label) = contents.trim().strip_prefix("AgentMux portable build ") {
                let label = label.trim();
                if !label.is_empty() {
                    return Some(label.to_string());
                }
            }
        }
    }
    None
}

/// Get details for the About modal.
pub fn get_about_modal_details(state: &Arc<AppState>) -> serde_json::Value {
    let version = env!("CARGO_PKG_VERSION");
    let endpoints = state.backend_endpoints.lock();

    serde_json::json!({
        "version": version,
        // Exact local-build label matching the portable's folder/ZIP name
        // (null for released / installed / dev builds). Lets the user tie
        // a running instance back to the artifact on disk.
        "buildLabel": read_build_label(),
        "gitHash": env!("AGENTMUX_GIT_HASH"),
        "buildTime": env!("AGENTMUX_BUILD_TIME").parse::<i64>().unwrap_or(0),
        "platform": match std::env::consts::OS {
            "macos" => "darwin",
            "windows" => "win32",
            other => other,
        },
        "arch": std::env::consts::ARCH,
        "backendEndpoints": {
            "ws": endpoints.ws_endpoint,
            "web": endpoints.web_endpoint,
        }
    })
}

/// Get comprehensive host info for the hostname popover.
pub fn get_host_info(state: &Arc<AppState>) -> serde_json::Value {
    let version = env!("CARGO_PKG_VERSION");
    let endpoints = state.backend_endpoints.lock();
    let ipc_port = *state.ipc_port.lock();
    let data_dir = state.version_data_dir.lock().clone().unwrap_or_default();
    let pid = std::process::id();

    // Resolve primary local IP
    let local_ip = local_ip_address().unwrap_or_else(|| "127.0.0.1".to_string());

    let os_info = format!("{} {}",
        match std::env::consts::OS {
            "windows" => "Windows",
            "macos" => "macOS",
            "linux" => "Linux",
            other => other,
        },
        std::env::consts::ARCH
    );

    serde_json::json!({
        "hostname": whoami::fallible::hostname().unwrap_or_else(|_| "unknown".to_string()),
        "os": os_info,
        "localIp": local_ip,
        "instanceId": format!("v{}", version),
        "version": version,
        "dataDir": data_dir,
        "hostType": "CEF 148",
        "pid": pid,
        "ports": {
            "ipc": format!("127.0.0.1:{}", ipc_port),
            "web": endpoints.web_endpoint,
            "ws": endpoints.ws_endpoint,
            "devtools": "127.0.0.1:9222",
        }
    })
}

/// Get the primary non-loopback IPv4 address.
fn local_ip_address() -> Option<String> {
    // Connect a UDP socket to an external address to determine the local IP
    // (doesn't actually send data — just resolves the route)
    let socket = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.connect("8.8.8.8:80").ok()?;
    let addr = socket.local_addr().ok()?;
    Some(addr.ip().to_string())
}

/// Get the documentation site URL.
pub fn get_docsite_url(state: &Arc<AppState>) -> serde_json::Value {
    let endpoints = state.backend_endpoints.lock();
    if !endpoints.web_endpoint.is_empty() {
        serde_json::json!(format!("http://{}/docsite/", endpoints.web_endpoint))
    } else {
        serde_json::json!("https://docs.agentmux.ai")
    }
}

/// Open a file in the best available code editor.
pub fn open_in_editor(args: &serde_json::Value) -> Result<serde_json::Value, String> {
    let path = args
        .get("path")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing path".to_string())?;

    #[cfg(target_os = "windows")]
    {
        // Use explorer.exe directly instead of cmd /C start to avoid shell injection.
        std::process::Command::new("explorer")
            .arg(path)
            .spawn()
            .map_err(|e| format!("Failed to open file: {}", e))?;
        return Ok(serde_json::Value::Null);
    }

    #[cfg(not(target_os = "windows"))]
    {
        let cli_editors = ["code", "cursor", "zed", "subl", "atom"];
        for editor in &cli_editors {
            if std::process::Command::new(editor).arg(path).spawn().is_ok() {
                return Ok(serde_json::Value::Null);
            }
        }
    }

    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(path)
            .spawn()
            .map_err(|e| e.to_string())?;
        return Ok(serde_json::Value::Null);
    }
    #[cfg(target_os = "linux")]
    {
        std::process::Command::new("xdg-open")
            .arg(path)
            .spawn()
            .map_err(|e| e.to_string())?;
        return Ok(serde_json::Value::Null);
    }

    #[allow(unreachable_code)]
    Ok(serde_json::Value::Null)
}

/// Ensure settings.json exists in the config directory with the latest template.
pub fn ensure_settings_file(state: &Arc<AppState>) -> Result<serde_json::Value, String> {
    let config_dir_str = state
        .version_config_dir
        .lock()
        .clone()
        .ok_or_else(|| "Config dir not initialized yet".to_string())?;
    let config_dir = std::path::PathBuf::from(&config_dir_str);

    std::fs::create_dir_all(&config_dir)
        .map_err(|e| format!("Failed to create config dir: {}", e))?;

    let settings_path = config_dir.join("settings.json");

    // Read existing user values (strips JSONC comments, parses JSON)
    let existing = read_settings_jsonc(&settings_path);

    // Merge user values into fresh template
    let merged = merge_into_template(SETTINGS_TEMPLATE, &existing);
    std::fs::write(&settings_path, &merged)
        .map_err(|e| format!("Failed to write settings.json: {}", e))?;

    Ok(serde_json::json!(settings_path.to_string_lossy()))
}

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
        tracing::info!(url = %url, "run_cli_login: captured auth URL");
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
        }
    });

    Ok(serde_json::json!({ "auth_url": auth_url }))
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
/// CRITICAL ConPTY lifetime contract on Windows: the PtyPair (master +
/// slave) MUST stay alive across child.wait(). Same hazard pattern
/// agentmux-bashwrap navigates. The blocking wait task takes ownership
/// of the pair so the destructor runs after the child reaps.
async fn run_cli_login_pty(
    state: Arc<AppState>,
    cli_path: String,
    login_args: Vec<String>,
    auth_env: std::collections::HashMap<String, String>,
    generation: u64,
) -> Result<serde_json::Value, String> {
    use portable_pty::{native_pty_system, CommandBuilder, PtySize};

    let pty_system = native_pty_system();
    let pair = pty_system
        .openpty(PtySize {
            rows: 24,
            cols: 80,
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|e| format!("openpty for {cli_path}: {e}"))?;

    let mut cmd = CommandBuilder::new(&cli_path);
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

    // Capture the child PID before moving the child into the wait
    // task — cancel_cli_login needs it to kill the subprocess
    // platform-side, since aborting the spawn_blocking wait does not
    // propagate to the child.
    let child_pid = child.process_id();
    if let Some(pid) = child_pid {
        *state.cli_login_pty_pid.lock() = Some(pid);
    }

    let reader = pair
        .master
        .try_clone_reader()
        .map_err(|e| format!("PTY try_clone_reader: {e}"))?;
    let writer = pair
        .master
        .take_writer()
        .map_err(|e| format!("PTY take_writer: {e}"))?;

    tracing::info!(cli = %cli_path, pid = ?child_pid, "run_cli_login: spawned (PTY), waiting for OAuth URL");

    // Store the PTY writer so set_provider_auth can deliver an OAuth
    // code via stdin (some flows prompt the user to paste a code).
    {
        let mut stored = state.cli_login_stdin.lock();
        *stored = Some(CliLoginStdin::Pty(writer));
    }

    // Synchronously read from the master in a blocking task, scanning
    // each line for an OAuth URL. portable_pty's reader is sync.
    // The 15 s cap is enforced async-side via tokio::time::timeout —
    // BufRead::read_line itself blocks indefinitely without per-read
    // timeout support, so a child that pauses before its first line
    // (or sits at a prompt with no newline) would wedge `url_rx.await`
    // without it. When the timeout fires we return auth_url=None to
    // the frontend and let the wait task below reap the child whenever
    // it finishes naturally.
    let (url_tx, url_rx) = tokio::sync::oneshot::channel::<Option<String>>();
    tokio::task::spawn_blocking(move || {
        use std::io::BufRead;
        let mut reader = std::io::BufReader::new(reader);
        // Wrap the oneshot in an Option so we send the URL exactly once and then
        // keep reading. Before the URL: scan for it. After: keep draining and LOG
        // every line — the CLI's `Paste code here >` prompt and, crucially, its
        // response to the code delivered via set_provider_auth (success vs. an
        // "invalid code" / error). Without this the host discarded that output,
        // so a failed login was a black box. Draining also keeps the CLI from
        // blocking on a full PTY output buffer.
        let mut url_tx = Some(url_tx);
        let mut line = String::new();
        loop {
            line.clear();
            match reader.read_line(&mut line) {
                Ok(0) => break, // EOF
                Ok(_) => {
                    if let Some(tx) = url_tx.take() {
                        if let Some(u) = extract_url(&line) {
                            let _ = tx.send(Some(u));
                        } else {
                            url_tx = Some(tx); // not the URL line yet
                        }
                    } else {
                        let t = line.trim_end();
                        if !t.trim().is_empty() {
                            tracing::info!(target: "login_pty", "[login-pty] {}", t);
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
        std::time::Duration::from_secs(15),
        url_rx,
    )
    .await
    {
        Ok(Ok(u)) => u,
        Ok(Err(_)) | Err(_) => None,
    };
    if let Some(ref url) = auth_url {
        tracing::info!(url = %url, "run_cli_login_pty: captured auth URL");
    } else {
        tracing::warn!("run_cli_login_pty: no auth URL captured within 15s");
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
            match child.try_wait() {
                Ok(Some(status)) => {
                    tracing::info!(
                        exit_code = ?status.exit_code(),
                        "run_cli_login_pty: child exited"
                    );
                    break;
                }
                Ok(None) => {
                    if std::time::Instant::now() >= deadline {
                        tracing::warn!("run_cli_login_pty: login timed out, killing child");
                        let _ = child.kill();
                        let _ = child.wait();
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
        // pair drops here, after the child reaps (ConPTY lifetime contract).
        drop(pair);
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
        }
    });

    Ok(serde_json::json!({ "auth_url": auth_url }))
}

/// Extract an OAuth URL from a line of CLI output.
/// Strips ANSI escape sequences and looks for `https://...` substrings.
fn extract_url(line: &str) -> Option<String> {
    // Strip ANSI escapes. Two families matter here:
    //   * CSI  — `ESC [ … <final 0x40..=0x7e>` (colors, cursor moves)
    //   * OSC  — `ESC ] … (BEL | ST)` — notably OSC-8 hyperlinks, which the
    //     Claude CLI emits. OSC-8 embeds the URL in the sequence params AND
    //     repeats it as visible link text, so a naive pass that only knew CSI
    //     left the raw `]8;;https://…<BEL>` in place and captured the URL
    //     twice (doubled), producing a broken link. We discard the OSC
    //     sequence but stash any URI it carried as a fallback.
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

    // Prefer the visible (de-escaped) text; fall back to any OSC-8 URI.
    pick(&clean).or_else(|| osc_uris.iter().find_map(|u| pick(u)))
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

// --- Settings helpers (ported from src-tauri/src/commands/platform.rs) ---

fn read_settings_jsonc(path: &std::path::Path) -> serde_json::Map<String, serde_json::Value> {
    if !path.exists() {
        return serde_json::Map::new();
    }
    match std::fs::read_to_string(path) {
        Ok(content) => {
            let stripped = json_comments::StripComments::new(content.as_bytes());
            let mut json_bytes = Vec::new();
            std::io::BufReader::new(stripped)
                .read_to_end(&mut json_bytes)
                .unwrap_or_default();
            let json_str = strip_trailing_commas(&String::from_utf8_lossy(&json_bytes));
            match serde_json::from_str::<serde_json::Value>(&json_str) {
                Ok(serde_json::Value::Object(map)) => map,
                _ => serde_json::Map::new(),
            }
        }
        Err(_) => serde_json::Map::new(),
    }
}

fn strip_trailing_commas(input: &str) -> String {
    let mut result = String::with_capacity(input.len());
    let mut in_string = false;
    let mut last_comma_pos: Option<usize> = None;

    for ch in input.chars() {
        if in_string {
            result.push(ch);
            if ch == '"' {
                let backslashes = result[..result.len() - 1]
                    .chars()
                    .rev()
                    .take_while(|&c| c == '\\')
                    .count();
                if backslashes % 2 == 0 {
                    in_string = false;
                }
            }
            continue;
        }
        match ch {
            '"' => {
                in_string = true;
                last_comma_pos = None;
                result.push(ch);
            }
            ',' => {
                last_comma_pos = Some(result.len());
                result.push(ch);
            }
            '}' | ']' => {
                if let Some(pos) = last_comma_pos {
                    result.replace_range(pos..pos + 1, " ");
                }
                last_comma_pos = None;
                result.push(ch);
            }
            _ if ch.is_whitespace() => {
                result.push(ch);
            }
            _ => {
                last_comma_pos = None;
                result.push(ch);
            }
        }
    }
    result
}

fn merge_into_template(
    template: &str,
    user_settings: &serde_json::Map<String, serde_json::Value>,
) -> String {
    if user_settings.is_empty() {
        return template.to_string();
    }

    let mut remaining: std::collections::HashMap<&str, &serde_json::Value> =
        user_settings.iter().map(|(k, v)| (k.as_str(), v)).collect();
    let mut lines: Vec<String> = Vec::new();

    for line in template.lines() {
        if let Some(key) = extract_commented_setting_key(line) {
            if let Some(value) = remaining.remove(key) {
                let indent: String = line.chars().take_while(|c| c.is_whitespace()).collect();
                let val_str = serde_json::to_string(value).unwrap_or_default();
                lines.push(format!("{}\"{}\": {},", indent, key, val_str));
                continue;
            }
        }
        lines.push(line.to_string());
    }

    if !remaining.is_empty() {
        if let Some(brace_pos) = lines.iter().rposition(|l| l.trim() == "}") {
            let mut extra: Vec<String> = Vec::new();
            extra.push(String::new());
            extra.push("    // -- User Overrides --".to_string());
            let mut sorted_keys: Vec<&&str> = remaining.keys().collect();
            sorted_keys.sort();
            for key in sorted_keys {
                let value = remaining[*key];
                let val_str = serde_json::to_string(value).unwrap_or_default();
                extra.push(format!("    \"{}\": {},", key, val_str));
            }
            for (i, line) in extra.into_iter().enumerate() {
                lines.insert(brace_pos + i, line);
            }
        }
    }

    let mut result = lines.join("\n");
    if !result.ends_with('\n') {
        result.push('\n');
    }
    result
}

/// Open a URL in the system's default browser.
pub fn open_external(args: &serde_json::Value) -> Result<serde_json::Value, String> {
    let url = args
        .get("url")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing url".to_string())?;

    // Allow safe URL schemes. vscode:// is included so that file-path links
    // with a :line suffix can open the file at the correct line in VS Code.
    let allowed = url.starts_with("http://")
        || url.starts_with("https://")
        || url.starts_with("devtools://")
        || url.starts_with("vscode://");
    if !allowed {
        return Err(format!("Refusing to open URL with unsupported scheme: {}", url));
    }

    #[cfg(target_os = "windows")]
    {
        // Use rundll32 url.dll,FileProtocolHandler instead of explorer.exe or
        // cmd /C start. Explorer is a file manager — when it is already running
        // (always the case on Windows), passing a URL to a second explorer
        // instance is unreliable and sometimes opens a file-manager window.
        // cmd.exe interprets & and | in URLs as command separators (injection).
        // url.dll,FileProtocolHandler is the Windows built-in URL dispatcher:
        // it reads HKCR\https\shell\open\command and always opens the default
        // browser, handling any printable characters in the URL safely.
        let _ = std::process::Command::new("rundll32.exe")
            .args(["url.dll,FileProtocolHandler", url])
            .spawn()
            .map_err(|e| format!("Failed to open URL: {}", e))?;
    }
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("open")
            .arg(url)
            .spawn()
            .map_err(|e| format!("Failed to open URL: {}", e))?;
    }
    #[cfg(target_os = "linux")]
    {
        let _ = std::process::Command::new("xdg-open")
            .arg(url)
            .spawn()
            .map_err(|e| format!("Failed to open URL: {}", e))?;
    }

    Ok(serde_json::Value::Null)
}

/// Open the system file manager with the given file selected.
/// On Windows: `explorer /select,<path>`.
/// On macOS:   `open -R <path>`.
/// On Linux:   opens the parent directory via `xdg-open` (no cross-desktop
///             "select file" standard exists).
pub fn reveal_in_file_explorer(args: &serde_json::Value) -> Result<serde_json::Value, String> {
    let file_path = args
        .get("filePath")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing filePath".to_string())?;

    #[cfg(target_os = "windows")]
    {
        // Convert forward slashes (from JS normalisation) back to backslashes.
        let native = file_path.replace('/', "\\");
        // /select,<path> must be a single argument — the comma is the delimiter
        // between the switch and the path, not a shell separator.
        let _ = std::process::Command::new("explorer.exe")
            .arg(format!("/select,{}", native))
            .spawn()
            .map_err(|e| format!("Failed to reveal in Explorer: {}", e))?;
    }
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("open")
            .args(["-R", file_path])
            .spawn()
            .map_err(|e| format!("Failed to reveal in Finder: {}", e))?;
    }
    #[cfg(target_os = "linux")]
    {
        let parent = std::path::Path::new(file_path)
            .parent()
            .and_then(|p| p.to_str())
            .unwrap_or(file_path);
        let _ = std::process::Command::new("xdg-open")
            .arg(parent)
            .spawn()
            .map_err(|e| format!("Failed to open parent directory: {}", e))?;
    }

    Ok(serde_json::Value::Null)
}

fn extract_commented_setting_key(line: &str) -> Option<&str> {
    let trimmed = line.trim_start();
    let rest = trimmed.strip_prefix("//")?;
    let rest = rest.trim_start();
    let rest = rest.strip_prefix('"')?;
    let end = rest.find('"')?;
    Some(&rest[..end])
}
