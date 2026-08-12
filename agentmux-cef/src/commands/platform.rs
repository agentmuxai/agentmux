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

/// Check if THIS build is a `task dev` build — resolved from the host exe
/// PATH (`is_dev_self`), NOT `AGENTMUX_RUNTIME_MODE`. A running dev AgentMux
/// leaks that env into descendant processes, which would otherwise flip a
/// packaged build to "DEV" (the status-bar badge).
pub fn get_is_dev() -> serde_json::Value {
    serde_json::json!(agentmux_common::is_dev_self())
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
///
/// Security: refuses keys whose name suggests a secret (cloud keys, API
/// tokens, passwords, the AgentMux auth key, etc.). `get_env` is reachable by
/// any IPC caller holding the bearer token — including agent processes and
/// browser-pane code — so an unrestricted reader is a direct
/// credential-exfiltration path. The legitimate frontend never depends on this
/// command in the CEF host (the `getEnv` shim returns "" and resolves real
/// values from window globals), so the denylist is transparent to the app.
/// See reports security sweep 2026-06-12 (get-env-unrestricted).
pub fn get_env(args: &serde_json::Value) -> serde_json::Value {
    let key = args
        .get("key")
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    if is_sensitive_env_key(key) {
        tracing::warn!(key = %key, "get_env: refused to expose a secret-looking env var");
        return serde_json::Value::Null;
    }
    match std::env::var(key) {
        Ok(val) => serde_json::json!(val),
        Err(_) => serde_json::Value::Null,
    }
}

/// True if an env-var name looks like it holds a secret and must not be
/// exposed over the IPC `get_env` command. Substring match on the
/// upper-cased key so prefixed/suffixed variants (AWS_SECRET_ACCESS_KEY,
/// GITHUB_TOKEN, AGENTMUX_AUTH_KEY, …) are all caught.
fn is_sensitive_env_key(key: &str) -> bool {
    const NEEDLES: [&str; 7] = [
        "KEY", "SECRET", "TOKEN", "PASSWORD", "PASSWD", "CREDENTIAL", "AUTH",
    ];
    let upper = key.to_ascii_uppercase();
    NEEDLES.iter().any(|needle| upper.contains(needle))
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
/// The host runs from `<portable>/runtime/` and the marker is packaged INTO
/// `runtime/`, so it sits right next to this exe (the `exe_dir` candidate). We
/// also check one level up (the extract root) for robustness against older
/// portables that wrote the marker at the root.
fn read_build_label() -> Option<String> {
    let exe = std::env::current_exe().ok()?;
    let exe_dir = exe.parent()?;
    let candidates = [
        Some(exe_dir.join("agentmux-portable.marker")),
        exe_dir.parent().map(|p| p.join("agentmux-portable.marker")),
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

const BUILD_CHANNEL_DEFAULT: &str = match option_env!("AGENTMUX_BUILD_CHANNEL_DEFAULT") {
    Some(s) => s,
    None => "stable",
};

/// Get details for the About modal.
pub fn get_about_modal_details(state: &Arc<AppState>) -> serde_json::Value {
    let version = env!("CARGO_PKG_VERSION");
    let endpoints = state.backend_endpoints.lock();
    let channel = agentmux_common::DataPaths::from_env()
        .map(|p| p.channel)
        .unwrap_or_else(|| BUILD_CHANNEL_DEFAULT.to_string());

    serde_json::json!({
        "version": version,
        // Exact local-build label matching the portable's folder/ZIP name
        // (null for released / installed / dev builds). Lets the user tie
        // a running instance back to the artifact on disk.
        "buildLabel": read_build_label(),
        "gitHash": env!("AGENTMUX_GIT_HASH"),
        "buildTime": env!("AGENTMUX_BUILD_TIME").parse::<i64>().unwrap_or(0),
        "channel": channel,
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
    let debug_port = *state.debug_port.lock();
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
        "hostType": "host",
        "pid": pid,
        "ports": {
            "ipc": format!("127.0.0.1:{}", ipc_port),
            "web": endpoints.web_endpoint,
            "ws": endpoints.ws_endpoint,
            "devtools": format!("127.0.0.1:{}", debug_port),
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

// --- Settings helpers (ported from src-tauri/src/commands/platform.rs) ---

pub(crate) fn read_settings_jsonc(path: &std::path::Path) -> serde_json::Map<String, serde_json::Value> {
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

/// Open a URL in the system's default browser (IPC command wrapper).
pub fn open_external(args: &serde_json::Value) -> Result<serde_json::Value, String> {
    let url = args
        .get("url")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing url".to_string())?;
    open_url_in_default_browser(url)?;
    Ok(serde_json::Value::Null)
}

/// Open a URL in the system's default browser.
///
/// Shared by the `open_external` IPC command and by `on_before_popup`'s
/// external-link routing (`target="_blank"` / `window.open` from the app UI).
/// Validates the scheme first — defends against command injection and unexpected
/// protocol handlers.
pub fn open_url_in_default_browser(url: &str) -> Result<(), String> {
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

    Ok(())
}

/// True if `url` is an http(s) URL whose host is NOT this app's own loopback
/// origin — i.e. a link to the outside world (github.com, docs.agentmux.ai, …).
///
/// The app UI is always served from `http://127.0.0.1:<ipc_port>` (or
/// `http://localhost:<vite_port>` in dev), so any other host is external.
/// `on_before_popup` uses this to decide whether a `target="_blank"` link
/// should open in the system browser (external) or navigate in-app (internal /
/// browser-pane). Non-http schemes return false — they are not ours to route.
pub fn is_external_http_url(url: &str) -> bool {
    let lower = url.trim().to_ascii_lowercase();
    let rest = match lower
        .strip_prefix("http://")
        .or_else(|| lower.strip_prefix("https://"))
    {
        Some(r) => r,
        None => return false,
    };
    // authority = everything before the first '/', '?' or '#'
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    // drop any userinfo ("user:pass@host") then any ":port" suffix
    let host = authority.rsplit('@').next().unwrap_or(authority);
    let host = host.split(':').next().unwrap_or(host);
    !matches!(host, "127.0.0.1" | "localhost" | "0.0.0.0" | "")
}

/// Schemes a browser pane is allowed to *navigate* to. Everything web-ish
/// (pages, inline content, devtools, websockets) plus the loopback app origin.
/// Anything else is a non-web protocol whose navigation Chromium would, by
/// default, hand to the OS shell (`ShellExecute` on Windows) — which can launch
/// an OS-registered handler, and if that handler is elevated, raise a **UAC**
/// prompt. `on_before_browse` blocks navigations to disallowed schemes for
/// browser panes so embedded web content can never reach an OS protocol handler
/// (see docs/reports/REPORT_BROWSER_PANE_GOOGLE_LOGIN_INSTANCE_EXIT_AND_UAC_2026_08_11.md).
const PANE_ALLOWED_NAV_SCHEMES: &[&str] = &[
    "http", "https", "about", "data", "blob", "ws", "wss", "devtools",
    "chrome-devtools", "chrome",
];

/// The scheme of `url` (lowercased) if it has one — the run of
/// `[a-z0-9+.-]` before the first `:`, per RFC 3986. Returns `None` for a
/// scheme-relative or relative URL (no valid scheme → resolves against the
/// current http(s) origin, always safe).
fn url_scheme(url: &str) -> Option<String> {
    let trimmed = url.trim();
    let colon = trimmed.find(':')?;
    let scheme = &trimmed[..colon];
    if scheme.is_empty() {
        return None;
    }
    let mut chars = scheme.chars();
    // First char must be a letter; the rest letters/digits/+/-/. (RFC 3986).
    let first_ok = chars.next().map(|c| c.is_ascii_alphabetic()).unwrap_or(false);
    let rest_ok = scheme
        .chars()
        .skip(1)
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'));
    if first_ok && rest_ok {
        Some(scheme.to_ascii_lowercase())
    } else {
        None
    }
}

/// True if a browser pane must NOT be allowed to navigate to `url` because its
/// scheme is a non-web external protocol that Chromium would hand to the OS
/// shell. A relative/scheme-less URL, or one of `PANE_ALLOWED_NAV_SCHEMES`, is
/// allowed (returns false).
pub fn is_disallowed_pane_nav_scheme(url: &str) -> bool {
    match url_scheme(url) {
        None => false, // relative / scheme-less — resolves against current origin
        Some(scheme) => !PANE_ALLOWED_NAV_SCHEMES.contains(&scheme.as_str()),
    }
}

#[cfg(test)]
mod external_url_tests {
    use super::{is_disallowed_pane_nav_scheme, is_external_http_url};

    #[test]
    fn external_sites_are_external() {
        assert!(is_external_http_url("https://github.com/agentmuxai/agentmux/issues/new"));
        assert!(is_external_http_url("https://docs.agentmux.ai/config"));
        assert!(is_external_http_url("http://example.com:8080/x?y=1#z"));
        assert!(is_external_http_url("https://user@evil.com/path"));
    }

    #[test]
    fn loopback_app_origin_is_internal() {
        assert!(!is_external_http_url("http://127.0.0.1:54469/"));
        assert!(!is_external_http_url("http://localhost:5173/?windowLabel=window-x"));
        assert!(!is_external_http_url("http://127.0.0.1:1/agentmux/browser/foo"));
    }

    #[test]
    fn non_http_schemes_are_not_routed() {
        assert!(!is_external_http_url("about:blank"));
        assert!(!is_external_http_url("data:text/html,hi"));
        assert!(!is_external_http_url("blob:abc"));
        assert!(!is_external_http_url("vscode://file/x"));
    }

    #[test]
    fn web_schemes_are_allowed_pane_nav() {
        for u in [
            "https://claude.ai/login",
            "http://127.0.0.1:5173/",
            "about:blank",
            "data:text/html,hi",
            "blob:https://x/abc",
            "devtools://devtools/bundled/x.html",
            "/relative/path",
            "//scheme-relative/path",
            "?just=query",
        ] {
            assert!(!is_disallowed_pane_nav_scheme(u), "should allow: {u}");
        }
    }

    #[test]
    fn non_web_external_schemes_are_blocked_pane_nav() {
        // These are the OS-handoff schemes that can raise a UAC prompt.
        for u in [
            "ms-cxh://x",
            "microsoft-edge://x",
            "tel:+15551234",
            "mailto:a@b.com",
            "vscode://file/x",
            "steam://run/1",
            "callto:foo",
            "custom-installer://elevate",
        ] {
            assert!(is_disallowed_pane_nav_scheme(u), "should block: {u}");
        }
    }
}

/// Open the system file manager at the given path.
/// Directories are opened directly; files are revealed (selected) in their
/// parent directory.
///
/// | Platform | Directory        | File                        |
/// |----------|------------------|-----------------------------|
/// | Windows  | `explorer <dir>` | `explorer /select,<file>`   |
/// | macOS    | `open <dir>`     | `open -R <file>`            |
/// | Linux    | `xdg-open <dir>` | `xdg-open <parent dir>`     |
pub fn reveal_in_file_explorer(args: &serde_json::Value) -> Result<serde_json::Value, String> {
    let file_path = args
        .get("filePath")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing filePath".to_string())?;

    #[cfg(target_os = "windows")]
    {
        // Convert forward slashes (from JS normalisation) back to backslashes.
        let native = file_path.replace('/', "\\");
        let is_dir = std::path::Path::new(&native).is_dir();
        let arg = if is_dir {
            // Open the directory itself.
            native.clone()
        } else {
            // /select,<path> must be a single argument — the comma delimits the
            // switch from the path. Reveals the file in its parent directory.
            format!("/select,{}", native)
        };
        let _ = std::process::Command::new("explorer.exe")
            .arg(arg)
            .spawn()
            .map_err(|e| format!("Failed to open in Explorer: {}", e))?;
    }
    #[cfg(target_os = "macos")]
    {
        let is_dir = std::path::Path::new(file_path).is_dir();
        if is_dir {
            let _ = std::process::Command::new("open")
                .arg(file_path)
                .spawn()
                .map_err(|e| format!("Failed to open directory in Finder: {}", e))?;
        } else {
            let _ = std::process::Command::new("open")
                .args(["-R", file_path])
                .spawn()
                .map_err(|e| format!("Failed to reveal file in Finder: {}", e))?;
        }
    }
    #[cfg(target_os = "linux")]
    {
        let path = std::path::Path::new(file_path);
        let open_path = if path.is_dir() {
            file_path
        } else {
            path.parent().and_then(|p| p.to_str()).unwrap_or(file_path)
        };
        let _ = std::process::Command::new("xdg-open")
            .arg(open_path)
            .spawn()
            .map_err(|e| format!("Failed to open path: {}", e))?;
    }

    Ok(serde_json::Value::Null)
}

/// Extensions the Media pane can display. Kept in sync by hand with
/// `IMAGE_EXTENSIONS`/`VIDEO_EXTENSIONS`/`AUDIO_EXTENSIONS` in
/// `frontend/app/view/media/media.tsx` — there's no shared-constant
/// mechanism across the Rust host / TS frontend boundary, so a change to
/// one list needs the same change here. Deliberately no `mkv`: Chromium's
/// `<video>` element doesn't reliably accept the Matroska container for
/// direct playback regardless of the codec inside, so listing it here
/// would let a user pick a file that then fails to render.
const MEDIA_IMAGE_EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "gif", "webp"];
const MEDIA_VIDEO_EXTENSIONS: &[&str] = &["webm", "mp4", "mov"];
const MEDIA_AUDIO_EXTENSIONS: &[&str] = &["wav"];

/// Show a native "open file" dialog, filtered to the Media pane's supported
/// image/video/audio types, and return the chosen path (or `null` if the
/// user cancelled). Used by the Media pane (SPEC_MEDIA_PANE_2026_07_26.md)
/// so pointing it at a clip doesn't require typing/pasting an absolute path.
///
/// `rfd::FileDialog::pick_file` blocks the calling thread until the user
/// responds — wrapped in `spawn_blocking` so it doesn't stall a shared
/// Tokio worker, matching `get_window_position`'s precedent in `ipc.rs`.
pub async fn show_open_file_dialog(
    _args: &serde_json::Value,
) -> Result<serde_json::Value, String> {
    let path = tokio::task::spawn_blocking(|| {
        rfd::FileDialog::new()
            .add_filter(
                "Supported media",
                &[MEDIA_IMAGE_EXTENSIONS, MEDIA_VIDEO_EXTENSIONS, MEDIA_AUDIO_EXTENSIONS].concat(),
            )
            .add_filter("Images", MEDIA_IMAGE_EXTENSIONS)
            .add_filter("Videos", MEDIA_VIDEO_EXTENSIONS)
            .add_filter("Audio", MEDIA_AUDIO_EXTENSIONS)
            .pick_file()
    })
    .await
    .map_err(|e| format!("show_open_file_dialog: task join error: {e}"))?;
    Ok(match path {
        Some(p) => serde_json::json!(p.to_string_lossy()),
        None => serde_json::Value::Null,
    })
}

/// File picker for Armory Bundle Format (`.abf`) files — Phase 3 of
/// docs/specs/SPEC_ABF_IMPORT_UI_PHASE3_2026_08_02.md §4 Step 1.
/// `show_open_file_dialog` can't be reused: it takes no filter argument and
/// its own filter list is hard-coded to image/video/audio extensions, with
/// no generic filter mechanism elsewhere in this module — this mirrors its
/// shape with an `.abf` filter instead. The filter is advisory only (a
/// non-`.abf` file picked here still just fails `unzip_bundle_import`'s
/// "not a valid zip archive" check server-side, same as today).
pub async fn show_open_bundle_dialog(
    _args: &serde_json::Value,
) -> Result<serde_json::Value, String> {
    let path = tokio::task::spawn_blocking(|| {
        rfd::FileDialog::new()
            .add_filter("Armory Bundle", &["abf"])
            .pick_file()
    })
    .await
    .map_err(|e| format!("show_open_bundle_dialog: task join error: {e}"))?;
    Ok(match path {
        Some(p) => serde_json::json!(p.to_string_lossy()),
        None => serde_json::Value::Null,
    })
}

fn extract_commented_setting_key(line: &str) -> Option<&str> {
    let trimmed = line.trim_start();
    let rest = trimmed.strip_prefix("//")?;
    let rest = rest.trim_start();
    let rest = rest.strip_prefix('"')?;
    let end = rest.find('"')?;
    Some(&rest[..end])
}
