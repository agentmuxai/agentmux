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

#[cfg(windows)]
use agentmux_common::win32::NoWindow;
use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStderr, ChildStdin, ChildStdout, Command};
use tokio::sync::Mutex;

use crate::backend::eventbus::EventBus;

pub type ServerId = String;

pub struct StartArgs {
    pub language: String,
    pub workspace_root: PathBuf,
    /// The file the server is asked for: with the root, it decides whether a server runs on the host.
    pub file_path: PathBuf,
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

    /// The file, or its project folder, is one a container agent can write, or the project folder holds
    /// one: no server runs on the host for it, whatever the language. The frontend shows this as a state
    /// of its own.
    #[error("no_host_server_for_container_agent_folder")]
    ContainerAgentFolder,

    /// A document was opened by an address that is not a file's path: a server is given files only.
    #[error("not a file: {0}")]
    NotAFile(String),

    /// A message that would give a running server a folder was sent to it.
    #[error("not allowed: {0}")]
    NotAllowed(String),

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
    child: Mutex<Child>, // hold ownership; kill_on_drop fires when the last holder lets go
}

impl ServerHandle {
    /// Ends the process now. Taking the handle out of the map is not enough: a `send` that is writing to the
    /// server holds the handle too (and may wait on a server that reads nothing), and the child lives as long
    /// as any holder does.
    async fn end(&self) {
        let _ = self.child.lock().await.kill().await;
    }
}

pub struct LspSupervisor {
    servers: Mutex<HashMap<(String, String), Arc<ServerHandle>>>,
    event_bus: Arc<EventBus>,
}

/// Refuses a server for a file or a project folder that a container agent can write, and for a project
/// folder that holds one. The check reads the disk and the store, so it runs off the async workers.
async fn on_host_only(args: &StartArgs) -> Result<(), LspError> {
    let (file, root) = (args.file_path.clone(), args.workspace_root.clone());
    let on_host = tokio::task::spawn_blocking(move || super::workspace::served_on_host(&file, &root))
        .await
    .map_err(|e| LspError::SpawnFailed(format!("the folder check did not finish: {e}")))?;
    if on_host {
        Ok(())
    } else {
        Err(LspError::ContainerAgentFolder)
    }
}

/// How often a running server's project folder is looked at again.
const RECHECK_EVERY: Duration = Duration::from_secs(5);

/// The method of the message that tells the Editor a running server was stopped because its project folder
/// came to be one a container agent can write. It is sent as a message of the server, on the server's id, so
/// every client of that server hears it.
pub const NOT_ON_HOST_NOTICE: &str = "agentmux/notOnHost";

/// The messages the Editor's client sends a server are `initialize` (with the root srv gave it), `initialized`,
/// and `textDocument/didOpen`, `didChange` and `didClose`: none of them hands the server a folder or a file
/// other than by `didOpen`. A client that adds a folder to a running server is refused here.
const FOLDER_MESSAGES: [&str; 1] = ["workspace/didChangeWorkspaceFolders"];

/// Whether `message_json` is a message that gives a running server a folder ([`FOLDER_MESSAGES`]). A message
/// that does not name one of them is not parsed: a `didChange` carries the whole buffer on every keystroke.
fn adds_a_folder(message_json: &str) -> bool {
    if !FOLDER_MESSAGES.iter().any(|method| message_json.contains(method)) {
        return false;
    }
    serde_json::from_str::<serde_json::Value>(message_json)
        .ok()
        .and_then(|m| m.get("method").and_then(serde_json::Value::as_str).map(|m| FOLDER_MESSAGES.contains(&m)))
        .unwrap_or(false)
}

/// The file a `textDocument/didOpen` message opens, which is what a server is given to serve: `None` for any
/// other message, `Some(None)` for a document whose address is not a file's path. A message that does not name
/// `textDocument/didOpen` is not parsed.
fn opened_document(message_json: &str) -> Option<Option<PathBuf>> {
    if !message_json.contains("textDocument/didOpen") {
        return None;
    }
    let message: serde_json::Value = serde_json::from_str(message_json).ok()?;
    if message.get("method")?.as_str()? != "textDocument/didOpen" {
        return None;
    }
    let uri = message.pointer("/params/textDocument/uri").and_then(serde_json::Value::as_str);
    Some(uri.and_then(|uri| url::Url::parse(uri).ok()).and_then(|url| url.to_file_path().ok()))
}

impl LspSupervisor {
    pub fn new(event_bus: Arc<EventBus>) -> Self {
        Self {
            servers: Mutex::new(HashMap::new()),
            event_bus,
        }
    }

    /// Counts one more user of the server running for `key`; false when there is none.
    async fn attach(&self, key: &(String, String)) -> bool {
        let servers = self.servers.lock().await;
        let running = servers.get(key);
        if let Some(handle) = running {
            handle.refcount.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
        running.is_some()
    }

    /// Start (or attach to existing) server for `(workspace_root, language)`.
    pub async fn start(&self, args: StartArgs) -> Result<StartResult, LspError> {
        let lang = args.language.clone();
        let root_str = args.workspace_root.to_string_lossy().to_string();
        let key = (root_str.clone(), lang.clone());
        let server_id = make_server_id(&root_str, &lang);

        // The answer is about the folder, so it comes first: before a running server is attached to and
        // before the language is looked at.
        on_host_only(&args).await?;
        if self.attach(&key).await {
            return Ok(StartResult { server_id, workspace_root: root_str });
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
        // A long-lived third-party language server. Not ours, not a helper, so it
        // gets the strict policy — no instance identity, no API endpoint.
        crate::backend::pane_env::sanitize_external_command(&mut cmd);
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
            cmd.no_window();
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
            child: Mutex::new(child),
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
        // A server is given a file only by `didOpen`, and a running server is shared by every file under its
        // root: the file is asked about here, as `start` asks about the first one, so a file that is a link
        // into a container agent's folder is refused whichever client opens it.
        if adds_a_folder(message_json) {
            return Err(LspError::NotAllowed(server_id.to_string()));
        }
        if let Some(opened) = opened_document(message_json) {
            let file = opened.ok_or_else(|| LspError::NotAFile(server_id.to_string()))?;
            let root = PathBuf::from(&key.0);
            let on_host = tokio::task::spawn_blocking(move || super::workspace::served_on_host(&file, &root))
                .await
                .map_err(|e| LspError::SpawnFailed(format!("the folder check did not finish: {e}")))?;
            if !on_host {
                return Err(LspError::ContainerAgentFolder);
            }
        }
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
                let ended = servers.remove(&key);
                drop(servers);
                if let Some(handle) = ended {
                    handle.end().await;
                }
            }
        }
        Ok(())
    }
}

impl LspSupervisor {
    /// Stops every running server whose project folder is no longer one the host serves: a container agent
    /// came to be able to write the folder, or a folder beneath it, after the server started. Each client of
    /// a stopped server is told on the server's id ([`NOT_ON_HOST_NOTICE`]). Returns the ids stopped.
    pub async fn stop_where_not_on_host(&self) -> Vec<ServerId> {
        let keys: Vec<(String, String)> = self.servers.lock().await.keys().cloned().collect();
        let mut stopped = Vec::new();
        for key in keys {
            let root = PathBuf::from(&key.0);
            // Where it can't be told, the server is not kept.
            let on_host = tokio::task::spawn_blocking(move || super::workspace::served_on_host(&root, &root)).await.unwrap_or(false);
            let removed = if on_host { None } else { self.servers.lock().await.remove(&key) };
            let Some(handle) = removed else { continue };
            handle.end().await;
            let server_id = make_server_id(&key.0, &key.1);
            tracing::warn!(server_id = %server_id, "LSP: stopped, its project folder is now one a container agent can write");
            let notice = serde_json::json!({ "jsonrpc": "2.0", "method": NOT_ON_HOST_NOTICE, "params": {} });
            let event = crate::backend::mps_events::LspMessageEvent { server_id: server_id.clone(), message: notice };
            self.event_bus.broadcast_event(&EventBus::typed_ws_event::<crate::backend::mps_events::LspMessage>(&event, String::new()));
            stopped.push(server_id);
        }
        stopped
    }

    /// Looks at the running servers' folders again every few seconds ([`Self::stop_where_not_on_host`]) until
    /// the supervisor is dropped. Needs a running tokio runtime.
    pub fn spawn_recheck(self: &Arc<Self>) {
        let supervisor = Arc::downgrade(self);
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(RECHECK_EVERY);
            loop {
                tick.tick().await;
                let Some(supervisor) = supervisor.upgrade() else { return };
                supervisor.stop_where_not_on_host().await;
            }
        });
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
        let event = crate::backend::mps_events::LspMessageEvent { server_id: server_id.to_string(), message };
        event_bus.broadcast_event(&EventBus::typed_ws_event::<crate::backend::mps_events::LspMessage>(&event, String::new()));
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

    fn start_args(language: &str, root: &std::path::Path) -> StartArgs {
        StartArgs { language: language.to_string(), workspace_root: root.to_path_buf(), file_path: root.join("a.ts") }
    }

    #[tokio::test]
    async fn a_container_agents_folder_gets_no_server_on_the_host_in_any_language() {
        let tmp = tempfile::tempdir().unwrap();
        let (ws, own) = (tmp.path().join("ws"), tmp.path().join("own"));
        std::fs::create_dir_all(&ws).unwrap();
        std::fs::create_dir_all(&own).unwrap();
        crate::backend::fs_ops::container_folders::add_for_test(&ws, &format!("lsp-{}", uuid::Uuid::new_v4().simple()), "/workspace");
        let supervisor = LspSupervisor::new(Arc::new(EventBus::new()));

        for language in ["typescript", "javascript", "rust", "python", "go", "c", "cpp", "no-such-language"] {
            let refused = supervisor.start(start_args(language, &ws)).await;
            assert!(matches!(refused, Err(LspError::ContainerAgentFolder)), "{language}: {:?}", refused.err());
        }
        assert_eq!(LspError::ContainerAgentFolder.to_wire_string(), "no_host_server_for_container_agent_folder");
        assert!(supervisor.servers.lock().await.is_empty());

        // The person's own folder passes the check and goes on to the language.
        let own_folder = supervisor.start(start_args("no-such-language", &own)).await;
        assert!(matches!(own_folder, Err(LspError::UnsupportedLanguage(_))), "{:?}", own_folder.err());
    }

    /// A child that stays up and reads nothing: it stands for a language server that is running.
    fn quiet_child() -> Child {
        #[cfg(windows)]
        let mut cmd = {
            let mut cmd = Command::new("ping");
            cmd.args(["-n", "120", "127.0.0.1"]);
            cmd.no_window();
            cmd
        };
        #[cfg(not(windows))]
        let mut cmd = {
            let mut cmd = Command::new("sleep");
            cmd.arg("120");
            cmd
        };
        cmd.stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::null()).kill_on_drop(true);
        cmd.spawn().unwrap()
    }

    /// A server for `root` that is already running, as `start` leaves one.
    async fn running(supervisor: &LspSupervisor, root: &std::path::Path, language: &str) -> (ServerId, Arc<ServerHandle>) {
        let mut child = quiet_child();
        let stdin = child.stdin.take().unwrap();
        let handle = Arc::new(ServerHandle { stdin: Mutex::new(stdin), refcount: std::sync::atomic::AtomicUsize::new(1), child: Mutex::new(child) });
        let root = root.to_string_lossy().into_owned();
        supervisor.servers.lock().await.insert((root.clone(), language.to_string()), handle.clone());
        (make_server_id(&root, language), handle)
    }

    fn a_project(root: &std::path::Path) {
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(root.join("package.json"), "{}").unwrap();
        std::fs::write(root.join("src").join("a.ts"), "").unwrap();
    }

    fn did_open(file: &std::path::Path) -> String {
        let uri = url::Url::from_file_path(file).unwrap().to_string();
        serde_json::json!({ "jsonrpc": "2.0", "method": "textDocument/didOpen", "params": { "textDocument": { "uri": uri, "languageId": "typescript", "version": 1, "text": "" } } }).to_string()
    }

    fn link_file(to: &std::path::Path, at: &std::path::Path) -> bool {
        #[cfg(unix)]
        return std::os::unix::fs::symlink(to, at).is_ok();
        #[cfg(windows)]
        return std::os::windows::fs::symlink_file(to, at).is_ok();
    }

    #[tokio::test]
    async fn a_file_that_resolves_into_a_container_agents_folder_is_not_opened_on_a_running_server() {
        let tmp = tempfile::tempdir().unwrap();
        let (ws, own) = (tmp.path().join("ws"), tmp.path().join("own"));
        a_project(&ws);
        a_project(&own);
        crate::backend::fs_ops::container_folders::add_for_test(&ws, &format!("lsp-{}", uuid::Uuid::new_v4().simple()), "/workspace");
        let supervisor = LspSupervisor::new(Arc::new(EventBus::new()));
        let (id, _handle) = running(&supervisor, &own, "typescript").await;

        // A plain file of the project: the running server takes it.
        assert!(supervisor.send(&id, &did_open(&own.join("src").join("a.ts"))).await.is_ok());
        // Its other messages carry no file to ask about.
        assert!(supervisor.send(&id, r#"{"jsonrpc":"2.0","method":"initialized","params":{}}"#).await.is_ok());

        // A file under the project's path that is a link into the agent's folder: refused, and the answer is
        // the one `start` gives, which the Editor shows as the folder being a container agent's.
        let linked = own.join("src").join("linked.ts");
        if link_file(&ws.join("src").join("a.ts"), &linked) {
            let refused = supervisor.send(&id, &did_open(&linked)).await;
            assert!(matches!(refused, Err(LspError::ContainerAgentFolder)), "{:?}", refused.err());
            assert!(supervisor.send(&id, &did_open(&own.join("src").join("a.ts"))).await.is_ok(), "the plain file is still served");
        } else {
            eprintln!("no link could be made here: the linked file is not tried");
        }
        // A file written as a path inside the agent's folder is refused too, whatever the server's root.
        let refused = supervisor.send(&id, &did_open(&ws.join("src").join("a.ts"))).await;
        assert!(matches!(refused, Err(LspError::ContainerAgentFolder)), "{:?}", refused.err());
        // A message that gives the server a folder is refused, whatever the folder.
        let adds = r#"{"jsonrpc":"2.0","method":"workspace/didChangeWorkspaceFolders","params":{"event":{"added":[{"uri":"file:///x","name":"x"}],"removed":[]}}}"#;
        assert!(matches!(supervisor.send(&id, adds).await, Err(LspError::NotAllowed(_))));
        // A document that is no file is not given to a server.
        let not_a_file = r#"{"jsonrpc":"2.0","method":"textDocument/didOpen","params":{"textDocument":{"uri":"untitled:one","languageId":"typescript","version":1,"text":""}}}"#;
        assert!(matches!(supervisor.send(&id, not_a_file).await, Err(LspError::NotAFile(_))));
    }

    fn did_open_uri(uri: &str) -> String {
        format!(r#"{{"jsonrpc":"2.0","method":"textDocument/didOpen","params":{{"textDocument":{{"uri":"{uri}","languageId":"typescript","version":1,"text":""}}}}}}"#)
    }

    #[test]
    fn a_network_path_is_the_file_it_names_on_windows_and_no_file_elsewhere() {
        let opened = opened_document(&did_open_uri("file://fileserver/share/src/a%20b.ts")).expect("a didOpen");
        if cfg!(windows) {
            assert_eq!(opened, Some(PathBuf::from(r"\\fileserver\share\src\a b.ts")));
        } else {
            assert_eq!(opened, None, "a host in a file address is no path here");
        }
        let plain = opened_document(&did_open_uri("file:///proj/src/a.ts")).expect("a didOpen");
        assert!(plain.is_some() || cfg!(windows), "a path's address still names its file");
    }

    #[tokio::test]
    async fn the_last_client_to_stop_ends_the_process_even_while_a_send_holds_the_handle() {
        let tmp = tempfile::tempdir().unwrap();
        let own = tmp.path().join("own");
        a_project(&own);
        let supervisor = LspSupervisor::new(Arc::new(EventBus::new()));
        let (id, held) = running(&supervisor, &own, "typescript").await;
        supervisor.stop(&id).await.unwrap();
        assert!(supervisor.servers.lock().await.is_empty());
        assert!(held.child.lock().await.try_wait().unwrap().is_some(), "the process has ended");
    }

    #[tokio::test]
    async fn a_running_server_is_stopped_when_its_folder_comes_to_hold_a_container_agents_folder() {
        let tmp = tempfile::tempdir().unwrap();
        let (a, b) = (tmp.path().join("a"), tmp.path().join("b"));
        a_project(&a);
        a_project(&b);
        let bus = Arc::new(EventBus::new());
        let mut heard = bus.register_ws("lsp-recheck", "lsp-recheck-tab");
        let supervisor = LspSupervisor::new(bus);
        // A holder of a's handle that outlives the stop, as a `send` that is writing to the server does.
        let (id_a, held_a) = running(&supervisor, &a, "typescript").await;
        let (id_b, held_b) = running(&supervisor, &b, "typescript").await;

        assert!(supervisor.stop_where_not_on_host().await.is_empty(), "nothing has changed");
        assert_eq!(supervisor.servers.lock().await.len(), 2);

        // A container agent comes to be able to write a folder beneath a's root, and one beside b.
        crate::backend::fs_ops::container_folders::add_for_test(&a.join("src"), &format!("lsp-{}", uuid::Uuid::new_v4().simple()), "/workspace");
        crate::backend::fs_ops::container_folders::add_for_test(&tmp.path().join("beside"), &format!("lsp-{}", uuid::Uuid::new_v4().simple()), "/workspace");
        assert_eq!(supervisor.stop_where_not_on_host().await, vec![id_a.clone()]);
        let left: Vec<String> = supervisor.servers.lock().await.keys().map(|(root, _)| root.clone()).collect();
        assert_eq!(left, vec![b.to_string_lossy().into_owned()], "the one beside it keeps running");
        assert!(held_a.child.lock().await.try_wait().unwrap().is_some(), "the stopped server's process has ended, though a holder of its handle is still there");
        assert!(held_b.child.lock().await.try_wait().unwrap().is_none(), "the one beside it is still running");
        assert!(supervisor.send(&id_a, &did_open(&a.join("src").join("a.ts"))).await.is_err(), "a stopped server takes nothing");
        assert!(supervisor.send(&id_b, &did_open(&b.join("src").join("a.ts"))).await.is_ok());

        // Its clients are told, on its id, as the server's own message.
        let frame = heard.priority.try_recv().expect("the clients of the stopped server are told").to_string();
        assert!(frame.contains("lsp:message") && frame.contains(NOT_ON_HOST_NOTICE), "{frame}");
        assert!(heard.priority.try_recv().is_err(), "once, and only for the one stopped");
        assert!(supervisor.stop_where_not_on_host().await.is_empty(), "and not again");
    }
}
