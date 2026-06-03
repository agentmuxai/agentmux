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

/// Get the user home directory used by the frontend for per-agent paths
/// (working dir, `GH_CONFIG_DIR`, etc.).
///
/// Portable returns `<portable>/data`; installed returns `~/.agentmux`;
/// `AGENTMUX_DATA_HOME`, if set at launch, overrides both.
/// See `docs/specs/portable-agent-working-dirs.md`.
pub fn get_user_home_dir(state: &Arc<AppState>) -> Result<serde_json::Value, String> {
    let dir = state.user_home_dir.lock();
    match dir.as_ref() {
        Some(d) => Ok(serde_json::json!(d)),
        None => Err("User home dir not initialized yet".to_string()),
    }
}

/// Ensure a provider auth directory exists and return its absolute path.
/// Auth dirs are version-isolated under the version-specific config dir.
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

    let config_dir = state.version_config_dir.lock();
    let config_dir = config_dir
        .as_ref()
        .ok_or_else(|| "Config dir not initialized yet".to_string())?;

    let auth_dir = std::path::PathBuf::from(config_dir)
        .join("auth")
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
    /// AsyncWrite via tokio. Used by Claude, Codex, Gemini, Copilot,
    /// Kimi — anything that doesn't strictly require a TTY for its
    /// auth subcommand.
    Pipe(tokio::process::ChildStdin),
    /// PTY writer — `portable_pty` master writer. Sync `std::io::Write`.
    /// Used by providers whose auth subcommand bails on `isatty()==0`
    /// (currently OpenClaw's `openclaw models auth login`).
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

    if requires_tty {
        return run_cli_login_pty(state, cli_path, login_args, auth_env).await;
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

    // Capture the OAuth URL from stdout/stderr. The CLI prints it within the
    // first few hundred ms after spawn. We read until we find "https://..."
    // or time out after 2s.
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();

    let auth_url: Option<String> = tokio::time::timeout(
        std::time::Duration::from_secs(2),
        async {
            use tokio::io::AsyncBufReadExt;
            let mut combined = Vec::new();
            if let Some(s) = stdout {
                let mut lines = tokio::io::BufReader::new(s).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    if let Some(url) = extract_url(&line) {
                        return Some(url);
                    }
                    combined.push(line);
                    if combined.len() > 20 { break; }
                }
            }
            if let Some(s) = stderr {
                let mut lines = tokio::io::BufReader::new(s).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    if let Some(url) = extract_url(&line) {
                        return Some(url);
                    }
                    if combined.len() > 40 { break; }
                    combined.push(line);
                }
            }
            None
        },
    ).await.unwrap_or(None);

    if let Some(ref url) = auth_url {
        tracing::info!(url = %url, "run_cli_login: captured auth URL");
    } else {
        tracing::warn!("run_cli_login: no auth URL captured within 2s");
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
        }
        // Clear the stored stdin handle once the process is done.
        *state_for_cleanup.cli_login_stdin.lock() = None;
    });

    Ok(serde_json::json!({ "auth_url": auth_url }))
}

/// PTY-backed variant of run_cli_login. Used for providers whose auth
/// subcommand requires an interactive TTY (currently OpenClaw —
/// `openclaw models auth login --provider <id>` exits immediately with
/// "requires an interactive TTY" when stdin is a pipe).
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
        let mut found: Option<String> = None;
        let mut line = String::new();
        loop {
            line.clear();
            match reader.read_line(&mut line) {
                Ok(0) => break, // EOF
                Ok(_) => {
                    if let Some(u) = extract_url(&line) {
                        found = Some(u);
                        break;
                    }
                }
                Err(e) => {
                    tracing::warn!(error = %e, "run_cli_login_pty: read error");
                    break;
                }
            }
        }
        let _ = url_tx.send(found);
        // Reader is dropped here. Master keeps living in the wait task
        // below.
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
        match child.wait() {
            Ok(status) => tracing::info!(
                exit_code = ?status.exit_code(),
                "run_cli_login_pty: child exited"
            ),
            Err(e) => tracing::warn!(
                error = %e,
                "run_cli_login_pty: child wait error"
            ),
        }
        // pair drops here, after child.wait() returns
        drop(pair);
        *state_for_cleanup.cli_login_stdin.lock() = None;
        *state_for_cleanup.cli_login_pty_pid.lock() = None;
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

    // Only allow safe URL schemes
    if !url.starts_with("http://") && !url.starts_with("https://") && !url.starts_with("devtools://") {
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

fn extract_commented_setting_key(line: &str) -> Option<&str> {
    let trimmed = line.trim_start();
    let rest = trimmed.strip_prefix("//")?;
    let rest = rest.trim_start();
    let rest = rest.strip_prefix('"')?;
    let end = rest.find('"')?;
    Some(&rest[..end])
}
