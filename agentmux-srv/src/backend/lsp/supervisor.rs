// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// LspSupervisor — spawns language-server child processes and proxies
// LSP messages between the editor pane and the server.
//
// Design notes:
//   * One server per (workspace_root, language). Same workspace open in
//     two panes shares one server (refcounted).
//   * Backend is a dumb proxy: it frames messages on the wire and
//     forwards bodies. LSP semantics (initialize, didOpen, …) are
//     enforced on the frontend in LspClient.
//   * Server stdout is read on a tokio task; each framed message is
//     broadcast as a `lsp:message` event on the EventBus.
//   * Process is killed on supervisor drop (kill_on_drop=true).
//
// Spec: docs/specs/SPEC_EDITOR_LSP_AND_THEMES_2026-05-26.md (Tier 1).

use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;

use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStderr, ChildStdin, ChildStdout, Command};
use tokio::sync::Mutex;

use crate::backend::eventbus::{EventBus, WSEventType};

pub type ServerId = String;

pub struct StartArgs {
    pub language: String,
    pub workspace_root: PathBuf,
}

pub struct StartResult {
    pub server_id: ServerId,
    pub workspace_root: String,
}

#[derive(thiserror::Error, Debug)]
pub enum LspError {
    /// The configured server binary couldn't be found on PATH. The
    /// frontend uses this to render the install banner.
    #[error("server_binary_not_found:{language}")]
    BinaryNotFound { language: String },

    #[error("unsupported language: {0}")]
    UnsupportedLanguage(String),

    #[error("spawn failed: {0}")]
    SpawnFailed(String),

    #[error("server not running: {0}")]
    ServerNotFound(String),

    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

impl LspError {
    pub fn to_wire_string(&self) -> String {
        self.to_string()
    }
}

struct ServerHandle {
    stdin: Mutex<ChildStdin>,
    refcount: std::sync::atomic::AtomicUsize,
    _child: Mutex<Child>, // hold ownership; kill_on_drop fires on supervisor drop
}

pub struct LspSupervisor {
    servers: Mutex<HashMap<(String, String), Arc<ServerHandle>>>,
    event_bus: Arc<EventBus>,
}

impl LspSupervisor {
    pub fn new(event_bus: Arc<EventBus>) -> Self {
        Self {
            servers: Mutex::new(HashMap::new()),
            event_bus,
        }
    }

    /// Start (or attach to existing) server for `(workspace_root, language)`.
    pub async fn start(&self, args: StartArgs) -> Result<StartResult, LspError> {
        let lang = args.language.clone();
        let root_str = args.workspace_root.to_string_lossy().to_string();
        let key = (root_str.clone(), lang.clone());
        let server_id = make_server_id(&root_str, &lang);

        // Fast-path: existing server, bump refcount.
        {
            let servers = self.servers.lock().await;
            if let Some(handle) = servers.get(&key) {
                handle
                    .refcount
                    .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                return Ok(StartResult {
                    server_id,
                    workspace_root: root_str,
                });
            }
        }

        // Resolve binary on PATH. Phase 1 is opinionated about supported
        // languages — see `binary_for_language`.
        let binary = binary_for_language(&lang)
            .ok_or_else(|| LspError::UnsupportedLanguage(lang.clone()))?;
        let resolved = which::which(binary).map_err(|_| LspError::BinaryNotFound {
            language: lang.clone(),
        })?;

        tracing::info!(
            language = %lang,
            workspace = %root_str,
            binary = %resolved.display(),
            "LSP: spawning server"
        );

        let mut cmd = Command::new(&resolved);
        // typescript-language-server, pyright-langserver, gopls — all use --stdio
        // by convention. rust-analyzer reads from stdio without a flag (the
        // flag is ignored). Safe to pass on all.
        cmd.arg("--stdio")
            .current_dir(&args.workspace_root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);

        // On Windows: suppress console-window allocation. Spawned from the
        // windowless srv without CREATE_NO_WINDOW, each LSP server (ts-language-
        // server, pyright, gopls, rust-analyzer) opens a Windows Terminal window
        // (Win11 default-terminal handler). stdio is piped, so no console is
        // needed. See docs/retro/retro-windows-terminal-window-leak-2026-06-21.md.
        #[cfg(windows)]
        {
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }

        let mut child = cmd
            .spawn()
            .map_err(|e| LspError::SpawnFailed(e.to_string()))?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| LspError::SpawnFailed("no stdin".to_string()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| LspError::SpawnFailed("no stdout".to_string()))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| LspError::SpawnFailed("no stderr".to_string()))?;

        // Spawn the stdout reader. Each framed message becomes an
        // `lsp:message` WSEvent broadcast for the frontend to dispatch.
        let event_bus = self.event_bus.clone();
        let server_id_for_stdout = server_id.clone();
        tokio::spawn(async move {
            read_lsp_messages(stdout, server_id_for_stdout, event_bus).await;
        });

        // Drain stderr — a chatty server (rust-analyzer, tsserver on errors)
        // can fill the pipe buffer and stall its own stdin/stdout. Log each
        // line via tracing so the supervisor surface remains observable.
        let server_id_for_stderr = server_id.clone();
        tokio::spawn(async move {
            drain_lsp_stderr(stderr, server_id_for_stderr).await;
        });

        let handle = Arc::new(ServerHandle {
            stdin: Mutex::new(stdin),
            refcount: std::sync::atomic::AtomicUsize::new(1),
            _child: Mutex::new(child),
        });

        // Re-check under the second lock — a concurrent start() for the same
        // (workspace, language) could have raced past the fast-path check
        // above. If so, drop our just-spawned child (kill_on_drop reaps it)
        // and bump the existing entry's refcount instead.
        let mut servers = self.servers.lock().await;
        if let Some(existing) = servers.get(&key) {
            existing
                .refcount
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            drop(servers);
            drop(handle); // releases our spawned child via kill_on_drop
            return Ok(StartResult {
                server_id,
                workspace_root: root_str,
            });
        }
        servers.insert(key, handle);

        Ok(StartResult {
            server_id,
            workspace_root: root_str,
        })
    }

    /// Forward an LSP message (pre-serialized JSON) to the server's stdin.
    pub async fn send(&self, server_id: &str, message_json: &str) -> Result<(), LspError> {
        let key = parse_server_id(server_id)?;
        let handle = {
            let servers = self.servers.lock().await;
            servers
                .get(&key)
                .cloned()
                .ok_or_else(|| LspError::ServerNotFound(server_id.to_string()))?
        };
        let mut stdin = handle.stdin.lock().await;
        let body = message_json.as_bytes();
        // LSP header: Content-Length: <bytes>\r\n\r\n
        let header = format!("Content-Length: {}\r\n\r\n", body.len());
        stdin.write_all(header.as_bytes()).await?;
        stdin.write_all(body).await?;
        stdin.flush().await?;
        Ok(())
    }

    /// Decrement refcount. When it reaches zero, drop the handle so the
    /// child process exits via `kill_on_drop`. Phase 1 does not yet
    /// implement the 60s idle grace from the spec (deferred — see
    /// supervisor.rs Open Questions follow-up).
    pub async fn stop(&self, server_id: &str) -> Result<(), LspError> {
        let key = parse_server_id(server_id)?;
        let mut servers = self.servers.lock().await;
        if let Some(handle) = servers.get(&key) {
            let prev = handle
                .refcount
                .fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
            if prev <= 1 {
                tracing::info!(server_id = %server_id, "LSP: shutting down (refcount=0)");
                servers.remove(&key);
                // Drop fires kill_on_drop → process exits.
            }
        }
        Ok(())
    }
}

fn make_server_id(workspace_root: &str, language: &str) -> ServerId {
    // Use a sentinel separator unlikely to appear in either component.
    format!("lsp://{language}::{workspace_root}")
}

fn parse_server_id(server_id: &str) -> Result<(String, String), LspError> {
    let stripped = server_id
        .strip_prefix("lsp://")
        .ok_or_else(|| LspError::ServerNotFound(server_id.to_string()))?;
    let (lang, root) = stripped
        .split_once("::")
        .ok_or_else(|| LspError::ServerNotFound(server_id.to_string()))?;
    Ok((root.to_string(), lang.to_string()))
}

/// Phase 1 binary table — the cross-platform `--stdio`-mode binary name
/// for each supported language. Extended in Phase 3.
fn binary_for_language(language: &str) -> Option<&'static str> {
    Some(match language {
        "typescript" | "javascript" => "typescript-language-server",
        // Phase 3 candidates — pre-listed so the discovery path works
        // for follow-up phases without code churn.
        "rust" => "rust-analyzer",
        "python" => "pyright-langserver",
        "go" => "gopls",
        "c" | "cpp" => "clangd",
        _ => return None,
    })
}

/// Read framed LSP messages from the server's stdout. Each message is
/// broadcast as an `lsp:message` event with `{ server_id, message }`.
/// Loop exits on EOF (server died) — the supervisor's child handle
/// then surfaces the exit via Drop on the next `stop` cycle.
async fn read_lsp_messages(stdout: ChildStdout, server_id: ServerId, event_bus: Arc<EventBus>) {
    let mut reader = BufReader::new(stdout);
    loop {
        let mut content_length: usize = 0;
        // Read headers until blank line.
        loop {
            let mut line = String::new();
            match reader.read_line(&mut line).await {
                Ok(0) => {
                    tracing::info!(server_id = %server_id, "LSP: stdout EOF (server exited)");
                    return;
                }
                Ok(_) => {}
                Err(e) => {
                    tracing::warn!(server_id = %server_id, error = %e, "LSP: stdout read error");
                    return;
                }
            }
            let trimmed = line.trim_end_matches(['\r', '\n']);
            if trimmed.is_empty() {
                break;
            }
            if let Some(value) = trimmed.strip_prefix("Content-Length:") {
                content_length = value.trim().parse().unwrap_or(0);
            }
            // Other headers (e.g. Content-Type) are ignored — LSP only
            // requires Content-Length.
        }
        if content_length == 0 {
            continue;
        }
        let mut buf = vec![0u8; content_length];
        if reader.read_exact(&mut buf).await.is_err() {
            tracing::warn!(server_id = %server_id, "LSP: body read failed");
            return;
        }
        let body = match std::str::from_utf8(&buf) {
            Ok(s) => s,
            Err(_) => {
                tracing::warn!(server_id = %server_id, "LSP: non-UTF8 body");
                continue;
            }
        };
        // Parse to a serde_json::Value so the frontend gets structured JSON,
        // not a string-of-JSON.
        let message = match serde_json::from_str::<serde_json::Value>(body) {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!(server_id = %server_id, error = %e, "LSP: invalid JSON body");
                continue;
            }
        };
        event_bus.broadcast_event(&WSEventType {
            eventtype: "lsp:message".to_string(),
            oref: String::new(),
            data: Some(serde_json::json!({
                "server_id": server_id,
                "message": message,
            })),
        });
    }
}

/// Drain the server's stderr line-by-line, logging each line via tracing.
/// Without this the OS pipe buffer fills up on chatty servers (rust-analyzer
/// during indexing, tsserver on startup errors) and the server's own writes
/// block, which can stall stdin/stdout handling.
async fn drain_lsp_stderr(stderr: ChildStderr, server_id: ServerId) {
    let mut reader = BufReader::new(stderr);
    let mut line = String::new();
    loop {
        line.clear();
        match reader.read_line(&mut line).await {
            Ok(0) => return, // EOF
            Ok(_) => {
                tracing::debug!(
                    server_id = %server_id,
                    stderr = %line.trim_end_matches(['\r', '\n']),
                    "LSP stderr"
                );
            }
            Err(e) => {
                tracing::warn!(server_id = %server_id, error = %e, "LSP: stderr read error");
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn server_id_roundtrip() {
        let id = make_server_id("/Users/asaf/project", "typescript");
        let (root, lang) = parse_server_id(&id).unwrap();
        assert_eq!(root, "/Users/asaf/project");
        assert_eq!(lang, "typescript");
    }

    #[test]
    fn server_id_with_double_colon_in_root() {
        // Edge case — Windows paths can contain colons (`C:`); make sure
        // our separator doesn't trip them.
        let id = make_server_id("C:\\Users\\asaf\\project", "rust");
        let (root, lang) = parse_server_id(&id).unwrap();
        assert_eq!(root, "C:\\Users\\asaf\\project");
        assert_eq!(lang, "rust");
    }

    #[test]
    fn binary_lookup_table() {
        assert_eq!(binary_for_language("typescript"), Some("typescript-language-server"));
        assert_eq!(binary_for_language("javascript"), Some("typescript-language-server"));
        assert_eq!(binary_for_language("rust"), Some("rust-analyzer"));
        assert_eq!(binary_for_language("python"), Some("pyright-langserver"));
        assert_eq!(binary_for_language("go"), Some("gopls"));
        assert_eq!(binary_for_language("c"), Some("clangd"));
        assert_eq!(binary_for_language("cpp"), Some("clangd"));
        assert_eq!(binary_for_language("brainfuck"), None);
    }
}
