// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `toolchain.vendor_install_info` / `toolchain.install_vendor_cli` — one click to run the
//! vendor's own installer for the agent CLIs AgentMux can't install into its own folder
//! (SPEC_ONE_CLICK_VENDOR_CLI_INSTALL_2026_10_20.md). It streams through the same
//! `install_chunk` events as `install.start` and the system-tool install, and `install.cancel`
//! stops it.
//!
//! **What runs comes from [`vendor_install_catalog`](crate::server::vendor_install_catalog),
//! chosen by provider id.** The RPC takes an id and nothing else: no command, URL, argument or
//! environment from the caller.
//!
//! **Only the user can start it:** before anything is downloaded or run, srv asks the person in
//! the approval window ([`ask_user`], served by the window host, which no pane process can reach
//! or answer). The panel's own button is not enough: every agent holds the key this RPC takes.
//! With no window to ask in, or no answer, nothing runs.
//!
//! **A script is downloaded first, and that file is what runs.** HTTPS only, every redirect's
//! host on the provider's list, a size cap, and its SHA-256 shown and logged. Nothing is pinned
//! (a vendor release would break it): the user is trusting the vendor, and AgentMux records what
//! that trust ran. It is argv only, never a pipe or a shell string; stdin is closed; AgentMux
//! never elevates; AgentMux's own secrets are taken out of the environment.

#[cfg(windows)]
use agentmux_common::win32::NoWindow;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use serde::Deserialize;
use serde_json::json;
use tokio::sync::oneshot;

use crate::backend::mps::Broker;
use crate::backend::rpc::engine::WshRpcEngine;
use crate::server::app_api::connections::{ask_user, UserAnswer};
use crate::server::install_handlers::{resolve_tool_path, InstallChunkEvent, InstallSessionRegistry, InstallStream};
use crate::server::system_install_handlers::ClaimGuard;
use crate::server::vendor_install_catalog::{url_allowed, vendor_install, Interpreter, Os, Recipe, VendorInstall};
use crate::server::AppState;

pub const COMMAND_TOOLCHAIN_VENDOR_INSTALL_INFO: &str = "toolchain.vendor_install_info";
pub const COMMAND_TOOLCHAIN_INSTALL_VENDOR_CLI: &str = "toolchain.install_vendor_cli";

/// The longest script AgentMux downloads. The five are under 60 KB.
const MAX_SCRIPT_BYTES: usize = 2 * 1024 * 1024;
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(60);
/// How long a finished installer's output is waited for. A helper it left running can hold a pipe open.
const DRAIN_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
#[serde(rename_all = "camelCase")]
struct VendorInstallReq {
    provider_id: String,
}

// What the panel shows for a provider: the vendor's command (for Copy) and where the script
// comes from. `available` is false for a provider with no installer AgentMux can run.
#[derive(Debug, serde::Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
#[serde(rename_all = "camelCase")]
struct VendorInstallInfo {
    available: bool,
    command: String,
    download_url: Option<String>,
}

// The install session to follow.
#[derive(Debug, serde::Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
#[serde(rename_all = "camelCase")]
struct VendorInstallStarted {
    session_id: String,
}

fn info_for(provider_id: &str, os: Os) -> VendorInstallInfo {
    match vendor_install(provider_id) {
        Some(v) => {
            let recipe = v.recipe(os);
            VendorInstallInfo { available: true, command: recipe.shown(), download_url: recipe.url().map(String::from) }
        }
        None => VendorInstallInfo { available: false, command: String::new(), download_url: None },
    }
}

/// The question the user is asked before anything runs: the vendor's command, what AgentMux does
/// with it, and that it installs outside AgentMux's isolation.
fn approval_question(entry: &VendorInstall, recipe: &Recipe) -> serde_json::Value {
    let name = entry.display;
    json!({
        "kind": "consent",
        "title": format!("Install {name}"),
        "message": format!(
            "AgentMux will run {name}'s own installer on this computer.\n\nThe vendor's command:\n{shown}\n\nWhat AgentMux does: {runs}.\n\nIt installs into your account and your PATH, outside AgentMux's isolation, and AgentMux can't check what the vendor's installer does. AgentMux shows the installer's SHA-256 and its output. It can't answer a password prompt: if the installer needs one, it stops.\n\nInstall it?",
            shown = recipe.shown(),
            runs = recipe.runs(),
        ),
        "ok_label": "Install",
        "cancel_label": "Cancel",
    })
}

/// `Ok` only for an answered yes. A missing window, a closed one, a timeout and a no all refuse.
fn require_approval(answer: Result<UserAnswer, String>, name: &str) -> Result<(), String> {
    match answer {
        Ok(a) if a.answered && a.approve => Ok(()),
        Ok(a) if a.answered => Err(format!("toolchain.install_vendor_cli: the user declined to install {name}")),
        Ok(_) => Err(format!("toolchain.install_vendor_cli: the user did not approve installing {name} in time")),
        Err(e) => Err(format!("toolchain.install_vendor_cli: installing {name} needs the user's approval, and {e}")),
    }
}

/// The registry's claim for one provider's install. Its own namespace: a tool id never has a `:`.
fn claim_key(entry: &VendorInstall) -> String {
    format!("vendor:{}", entry.provider_id)
}

/// Where a run's lines and its end go: the `install:<session>` scope the panel follows.
#[derive(Clone)]
struct Emitter {
    broker: Arc<Broker>,
    session_id: String,
    scope: String,
}

impl Emitter {
    fn new(broker: Arc<Broker>, session_id: &str) -> Self {
        Self { broker, session_id: session_id.to_string(), scope: format!("install:{session_id}") }
    }

    fn line(&self, text: impl Into<String>, stream: InstallStream) {
        let event = InstallChunkEvent::line(&self.session_id, text.into(), stream);
        self.broker.publish_event::<crate::backend::mps_events::InstallChunk>(&event, vec![self.scope.clone()], 1024);
    }

    fn say(&self, text: impl Into<String>) {
        self.line(text, InstallStream::Stdout);
    }

    fn done(&self, ok: bool, error: Option<String>) {
        let event = InstallChunkEvent::done(&self.session_id, ok, json!(error));
        self.broker.publish_event::<crate::backend::mps_events::InstallChunk>(&event, vec![self.scope.clone()], 1024);
    }
}

/// One install run: which provider, and where its output goes.
struct Run {
    entry: &'static VendorInstall,
    recipe: &'static Recipe,
    out: Emitter,
}

/// What to spawn, ready: argv, the recipe's environment, and the downloaded file kept until the run ends.
struct Plan {
    program: String,
    args: Vec<String>,
    env: Vec<(&'static str, &'static str)>,
    _script: Option<tempfile::TempPath>,
}

#[derive(Debug)]
struct Fetched {
    bytes: Vec<u8>,
    host: String,
    sha256: String,
}

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    format!("{:x}", Sha256::digest(bytes))
}

/// Refuses an empty answer and a web page (a 404 or a sign-in page served as 200): neither is a script.
fn check_is_script(url: &str, bytes: &[u8]) -> Result<(), String> {
    if bytes.is_empty() {
        return Err(format!("download {url}: the site sent nothing"));
    }
    let head = String::from_utf8_lossy(&bytes[..bytes.len().min(256)]).trim_start().to_ascii_lowercase();
    if head.starts_with("<!doctype") || head.starts_with("<html") {
        return Err(format!("download {url}: the site sent a web page, not a script"));
    }
    Ok(())
}

/// Follows a redirect only to an HTTPS address on the provider's host list.
fn redirect_policy(allowed: &'static [&'static str]) -> reqwest::redirect::Policy {
    reqwest::redirect::Policy::custom(move |attempt| {
        if attempt.previous().len() >= 5 {
            return attempt.error("too many redirects");
        }
        if url_allowed(allowed, attempt.url()) {
            return attempt.follow();
        }
        let host = attempt.url().host_str().unwrap_or("?").to_string();
        attempt.error(format!("it redirected to {host}, which is not one of the vendor's addresses"))
    })
}

/// Downloads the script at `url`. Errors start with `download `, which the panel reads as a network failure.
async fn fetch_script(url: &str, allowed: &'static [&'static str]) -> Result<Fetched, String> {
    let start = reqwest::Url::parse(url).map_err(|e| format!("download {url}: {e}"))?;
    if !url_allowed(allowed, &start) {
        return Err(format!("download {url}: not an address AgentMux may fetch an installer from"));
    }
    let client = reqwest::Client::builder()
        .redirect(redirect_policy(allowed))
        .timeout(DOWNLOAD_TIMEOUT)
        .user_agent(concat!("AgentMux/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|e| format!("download {url}: {e}"))?;
    let mut resp = client.get(start).send().await.map_err(|e| format!("download {url}: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("download {url}: the site answered {}", resp.status()));
    }
    let host = resp.url().host_str().unwrap_or("").to_string();
    let mut bytes = Vec::new();
    while let Some(chunk) = resp.chunk().await.map_err(|e| format!("download {url}: {e}"))? {
        if bytes.len() + chunk.len() > MAX_SCRIPT_BYTES {
            return Err(format!("download {url}: larger than {} MB, which is not an installer script", MAX_SCRIPT_BYTES / (1024 * 1024)));
        }
        bytes.extend_from_slice(&chunk);
    }
    check_is_script(url, &bytes)?;
    let sha256 = sha256_hex(&bytes);
    Ok(Fetched { bytes, host, sha256 })
}

/// What goes on disk for `interpreter`. Windows PowerShell 5.1 reads a `.ps1` with no byte-order
/// mark in the ANSI code page, so a script with an em dash or a curly quote in a string would not
/// parse the way `irm | iex` (which decodes the text properly) runs it: it gets the UTF-8 mark,
/// unless it has one already. The hash shown is of what the vendor sent, not of this.
fn script_on_disk(bytes: &[u8], interpreter: Interpreter) -> Vec<u8> {
    const MARKS: [&[u8]; 3] = [b"\xEF\xBB\xBF", b"\xFF\xFE", b"\xFE\xFF"];
    if interpreter != Interpreter::PowerShell || MARKS.iter().any(|m| bytes.starts_with(m)) {
        return bytes.to_vec();
    }
    let mut out = Vec::with_capacity(bytes.len() + 3);
    out.extend_from_slice(MARKS[0]);
    out.extend_from_slice(bytes);
    out
}

/// Saves the script where only this user can read it; removed when the returned path is dropped.
fn write_script(bytes: &[u8], interpreter: Interpreter) -> Result<tempfile::TempPath, String> {
    use std::io::Write;
    let bytes = script_on_disk(bytes, interpreter);
    let suffix = format!(".{}", interpreter.extension());
    let mut file = tempfile::Builder::new()
        .prefix("agentmux-vendor-install-")
        .suffix(&suffix)
        .tempfile()
        .map_err(|e| format!("couldn't save the installer to run it: {e}"))?;
    file.write_all(&bytes).and_then(|()| file.flush()).map_err(|e| format!("couldn't save the installer to run it: {e}"))?;
    Ok(file.into_temp_path())
}

async fn prepare_script(
    run: &Run,
    url: &'static str,
    interpreter: Interpreter,
    env: &'static [(&'static str, &'static str)],
) -> Result<Plan, String> {
    run.out.say(format!("Downloading {url}"));
    let fetched = fetch_script(url, run.entry.allowed_hosts).await?;
    run.out.say(format!("Downloaded {} bytes from {}", fetched.bytes.len(), fetched.host));
    run.out.say(format!("SHA-256 {}", fetched.sha256));
    tracing::info!(
        provider = run.entry.provider_id,
        url,
        host = %fetched.host,
        size = fetched.bytes.len(),
        sha256 = %fetched.sha256,
        "toolchain.install_vendor_cli: downloaded the installer"
    );
    let script = write_script(&fetched.bytes, interpreter)?;
    let (program, lead) = interpreter.program_and_args();
    let mut args: Vec<String> = lead.iter().map(|a| (*a).to_string()).collect();
    args.push(script.to_string_lossy().into_owned());
    Ok(Plan { program: program.to_string(), args, env: env.to_vec(), _script: Some(script) })
}

/// The Python `pip install --user` runs under: the Windows launcher first (the `python.exe` the
/// Store ships before Python is installed only opens the Store), then `python3` and `python`.
async fn find_python() -> Option<String> {
    let candidates: &[&str] = if cfg!(windows) { &["py", "python"] } else { &["python3", "python"] };
    for candidate in candidates {
        if let Some(path) = resolve_tool_path(candidate).await {
            return Some(path);
        }
    }
    None
}

async fn prepare_pip(package: &'static str) -> Result<Plan, String> {
    let python = find_python()
        .await
        .ok_or_else(|| "spawn python: Python isn't installed, or isn't on PATH. Install it from the Toolchain first.".to_string())?;
    let args = ["-m", "pip", "install", "--user", package].map(String::from).to_vec();
    Ok(Plan { program: python, args, env: Vec::new(), _script: None })
}

async fn prepare(run: &Run) -> Result<Plan, String> {
    match run.recipe {
        Recipe::Script { url, interpreter, env, .. } => prepare_script(run, *url, *interpreter, *env).await,
        Recipe::Pip { package } => prepare_pip(*package).await,
    }
}

/// How the installer's process ended.
enum Wait {
    Exited(std::io::Result<std::process::ExitStatus>),
    TimedOut,
    Cancelled,
}

fn forward<R>(reader: R, out: Emitter, stream: InstallStream) -> tokio::task::JoinHandle<()>
where
    R: tokio::io::AsyncRead + Unpin + Send + 'static,
{
    tokio::spawn(async move {
        use tokio::io::{AsyncBufReadExt, BufReader};
        let mut lines = BufReader::new(reader).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            out.line(line, stream);
        }
    })
}

async fn drain(tasks: [tokio::task::JoinHandle<()>; 2]) {
    for task in tasks {
        let _ = tokio::time::timeout(DRAIN_TIMEOUT, task).await;
    }
}

fn program_name(program: &str) -> String {
    std::path::Path::new(program).file_stem().map_or_else(|| program.to_string(), |s| s.to_string_lossy().into_owned())
}

/// Ends the installer and everything it started: its process group on Unix, `taskkill /T` on Windows.
async fn end_tree(child: &mut tokio::process::Child) {
    if let Some(pid) = child.id() {
        let _ = agentmux_common::process::force_kill_tree(pid);
    }
    let _ = child.kill().await;
}

/// Spawns `plan` (argv, stdin closed, AgentMux's secrets out of the environment, no elevation),
/// streams its output, and ends it on cancel or when the recipe's time is up.
async fn run_child(run: &Run, plan: Plan, cancel: &mut oneshot::Receiver<()>) -> Result<(), String> {
    run.out.say(format!("$ {} {}", plan.program, plan.args.join(" ")));
    let mut cmd = tokio::process::Command::new(&plan.program);
    crate::backend::pane_env::sanitize_external_command(&mut cmd);
    cmd.args(&plan.args)
        .envs(plan.env.iter().copied())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    crate::backend::process_tracker::registry::own_process_group(&mut cmd);
    #[cfg(windows)]
    cmd.no_window();
    let mut child = agentmux_common::process::KillGroupOnDrop(cmd.spawn().map_err(|e| format!("spawn {}: {e}", plan.program))?);
    let output = [
        forward(child.stdout.take().expect("piped"), run.out.clone(), InstallStream::Stdout),
        forward(child.stderr.take().expect("piped"), run.out.clone(), InstallStream::Stderr),
    ];
    let limit = Duration::from_secs(run.entry.timeout_secs);
    let waited = tokio::select! {
        biased;
        _ = &mut *cancel => Wait::Cancelled,
        r = tokio::time::timeout(limit, child.wait()) => match r {
            Ok(status) => Wait::Exited(status),
            Err(_) => Wait::TimedOut,
        },
    };
    let ended = match waited {
        Wait::Exited(Ok(status)) if status.success() => Ok(()),
        Wait::Exited(Ok(status)) => Err(format!("{} exited {:?}", program_name(&plan.program), status.code())),
        Wait::Exited(Err(e)) => Err(format!("wait: {e}")),
        Wait::TimedOut => {
            end_tree(&mut child).await;
            Err(format!("timed out after {} minutes", run.entry.timeout_secs / 60))
        }
        Wait::Cancelled => {
            end_tree(&mut child).await;
            Err("cancelled".to_string())
        }
    };
    drain(output).await;
    ended
}

/// After a clean exit: whether this srv can see the CLI, which its own PATH may not until it restarts.
async fn report_found(run: &Run) {
    let id = run.entry.provider_id;
    let Some(cli) = crate::backend::providers::get_provider(id).map(|p| p.cli_command) else { return };
    match crate::server::cli_handlers::resolve_provider_cli_on_path(id, cli).await {
        Some(path) => run.out.say(format!("Found {} at {path}", run.entry.display)),
        None => run.out.say(format!(
            "Note: the installer finished successfully, but AgentMux's current session doesn't see \"{cli}\" yet — restart AgentMux to pick up the updated PATH."
        )),
    }
}

async fn perform(run: &Run, cancel: &mut oneshot::Receiver<()>) -> Result<(), String> {
    run.out.say(format!("$ {}", run.recipe.shown()));
    if run.recipe.url().is_some() {
        run.out.say("AgentMux downloads that script first, shows its hash, and runs the file it downloaded.");
    }
    let plan = tokio::select! {
        biased;
        _ = &mut *cancel => return Err("cancelled".to_string()),
        plan = prepare(run) => plan?,
    };
    run_child(run, plan, cancel).await?;
    report_found(run).await;
    Ok(())
}

fn spawn_run(run: Run, registry: Arc<InstallSessionRegistry>, claim_key: String, mut cancel_rx: oneshot::Receiver<()>) {
    tokio::spawn(async move {
        match perform(&run, &mut cancel_rx).await {
            Ok(()) => run.out.done(true, None),
            Err(e) => run.out.done(false, Some(e)),
        }
        registry.drop_session(&run.out.session_id);
        registry.release_system_tool(&claim_key);
    });
}

async fn start_install(
    app: &AppState,
    registry: Arc<InstallSessionRegistry>,
    broker: Arc<Broker>,
    provider_id: &str,
) -> Result<VendorInstallStarted, String> {
    let entry = vendor_install(provider_id)
        .ok_or_else(|| format!("toolchain.install_vendor_cli: {provider_id:?} has no vendor installer AgentMux can run"))?;
    let recipe = entry.recipe(Os::current());
    let key = claim_key(entry);
    if !registry.try_claim_system_tool(&key) {
        return Err(format!("toolchain.install_vendor_cli: {} is already being installed in another session", entry.display));
    }
    // Held while the user is asked, so a second request is refused at once instead of showing a
    // second window. Let go unless the install starts.
    let mut claim = ClaimGuard { registry: registry.clone(), key: key.clone(), armed: true };
    // Before anything is downloaded or run: the user's yes, asked by srv itself.
    require_approval(ask_user(app, "", approval_question(entry, recipe)).await, entry.display)?;
    claim.armed = false;
    let session_id = format!("vendorinstall-{}", uuid::Uuid::new_v4());
    tracing::info!(
        session_id = %session_id,
        provider = entry.provider_id,
        command = %recipe.runs(),
        "toolchain.install_vendor_cli"
    );
    let (cancel_tx, cancel_rx) = oneshot::channel();
    registry.insert(session_id.clone(), cancel_tx);
    let run = Run { entry, recipe, out: Emitter::new(broker, &session_id) };
    spawn_run(run, registry, key, cancel_rx);
    Ok(VendorInstallStarted { session_id })
}

pub fn register_vendor_install_handlers(engine: &Arc<WshRpcEngine>, state: &AppState) {
    engine.register_typed(COMMAND_TOOLCHAIN_VENDOR_INSTALL_INFO, move |req: VendorInstallReq, _ctx| async move {
        Ok(info_for(&req.provider_id, Os::current()))
    });

    let registry = state.install_sessions.clone();
    let broker = state.broker.clone();
    let app = state.clone();
    engine.register_typed(COMMAND_TOOLCHAIN_INSTALL_VENDOR_CLI, move |req: VendorInstallReq, _ctx| {
        let registry = registry.clone();
        let broker = broker.clone();
        let app = app.clone();
        async move { start_install(&app, registry, broker, &req.provider_id).await }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::vendor_install_catalog::VENDOR_INSTALLS;

    fn answer(answered: bool, approve: bool) -> Result<UserAnswer, String> {
        Ok(UserAnswer { answered, approve, ..Default::default() })
    }

    fn grok() -> &'static VendorInstall {
        vendor_install("grok").unwrap()
    }

    fn plan(program: &str, args: &[&str]) -> Plan {
        Plan { program: program.to_string(), args: args.iter().map(|a| (*a).to_string()).collect(), env: Vec::new(), _script: None }
    }

    fn run_of(state: &AppState) -> Run {
        let entry = grok();
        Run { entry, recipe: entry.recipe(Os::current()), out: Emitter::new(state.broker.clone(), "vendorinstall-test") }
    }

    #[test]
    fn the_panel_gets_the_vendors_command_for_every_provider_on_both_systems() {
        for v in VENDOR_INSTALLS {
            for os in [Os::Windows, Os::Unix] {
                let info = info_for(v.provider_id, os);
                assert!(info.available && !info.command.is_empty(), "{} {os:?}", v.provider_id);
                assert_eq!(info.download_url.is_some(), matches!(v.recipe(os), Recipe::Script { .. }), "{} {os:?}", v.provider_id);
            }
        }
        assert_eq!(info_for("grok", Os::Unix).command, "curl -fsSL https://x.ai/cli/install.sh | bash");
    }

    #[test]
    fn a_provider_that_has_no_vendor_installer_is_not_available() {
        for id in ["claude", "codex", "goose", "", "../grok", "grok "] {
            let info = info_for(id, Os::Unix);
            assert!(!info.available && info.command.is_empty() && info.download_url.is_none(), "{id:?}");
        }
    }

    #[test]
    fn the_question_names_the_vendors_command_what_runs_and_the_missing_isolation() {
        let q = approval_question(grok(), grok().recipe(Os::Unix));
        assert_eq!(q["kind"], "consent");
        assert_eq!(q["title"], "Install Grok Build");
        let message = q["message"].as_str().unwrap();
        assert!(message.contains("install.sh | bash"), "{message}");
        assert!(message.contains("then run that file with: bash"), "{message}");
        assert!(message.contains("outside AgentMux's isolation"), "{message}");
        assert!(message.contains("SHA-256"), "{message}");
        let pip = approval_question(vendor_install("kimi").unwrap(), vendor_install("kimi").unwrap().recipe(Os::Unix));
        assert!(pip["message"].as_str().unwrap().contains("python -m pip install --user kimi-cli"));
    }

    #[test]
    fn only_an_answered_yes_approves() {
        assert!(require_approval(answer(true, true), "Grok Build").is_ok());
        assert!(require_approval(answer(true, false), "Grok Build").unwrap_err().contains("declined"));
        assert!(require_approval(answer(false, true), "Grok Build").unwrap_err().contains("in time"));
        assert!(require_approval(answer(false, false), "Grok Build").is_err());
        assert!(require_approval(Err("no AgentMux window is connected to ask the user in".into()), "Grok Build")
            .unwrap_err()
            .contains("needs the user's approval"));
    }

    /// A caller that reaches the RPC with no window to ask in gets nothing started, and the claim is let go.
    #[tokio::test]
    async fn with_no_window_to_ask_in_nothing_starts_and_the_claim_is_released() {
        let state = crate::server::tests::test_state();
        assert!(state.host_ipc.lock().await.is_none());
        let err = start_install(&state, state.install_sessions.clone(), state.broker.clone(), "grok").await.unwrap_err();
        assert!(err.contains("no AgentMux window is connected"), "{err}");
        assert!(state.install_sessions.try_claim_system_tool(&claim_key(grok())), "the refusal let go of the claim");
    }

    #[tokio::test]
    async fn an_unknown_provider_is_refused_before_anything_is_claimed_or_asked() {
        let state = crate::server::tests::test_state();
        let err = start_install(&state, state.install_sessions.clone(), state.broker.clone(), "claude").await.unwrap_err();
        assert!(err.contains("no vendor installer"), "{err}");
    }

    #[tokio::test]
    async fn a_second_request_while_one_is_pending_is_refused() {
        let state = crate::server::tests::test_state();
        assert!(state.install_sessions.try_claim_system_tool(&claim_key(grok())));
        let err = start_install(&state, state.install_sessions.clone(), state.broker.clone(), "grok").await.unwrap_err();
        assert!(err.contains("already being installed"), "{err}");
    }

    #[test]
    fn the_claim_key_cannot_be_a_system_tool_id() {
        // `is_safe_tool_id` allows letters, digits, `_` and `-`: never a `:`.
        for v in VENDOR_INSTALLS {
            assert!(claim_key(v).contains(':'));
        }
    }

    #[test]
    fn the_hash_is_the_standard_sha256() {
        assert_eq!(sha256_hex(b"abc"), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
    }

    #[test]
    fn a_web_page_or_nothing_is_not_a_script() {
        assert!(check_is_script("u", b"#!/bin/sh\necho hi\n").is_ok());
        assert!(check_is_script("u", b"# a comment\r\nWrite-Host hi").is_ok());
        assert!(check_is_script("u", b"").unwrap_err().starts_with("download u:"));
        assert!(check_is_script("u", b"  \n<!DOCTYPE html><title>404</title>").is_err());
        assert!(check_is_script("u", b"<HTML><body>sign in").is_err());
    }

    #[tokio::test]
    async fn an_http_or_foreign_address_is_refused_before_any_request() {
        let err = fetch_script("http://x.ai/cli/install.sh", &["x.ai"]).await.unwrap_err();
        assert!(err.starts_with("download "), "{err}");
        let err = fetch_script("https://evil.example/install.sh", &["x.ai"]).await.unwrap_err();
        assert!(err.contains("not an address"), "{err}");
        assert!(fetch_script("not a url", &["x.ai"]).await.is_err());
    }

    #[test]
    fn a_powershell_script_gets_a_utf8_mark_unless_it_has_one() {
        let plain = "Write-Host \"a — b\"".as_bytes();
        let marked = script_on_disk(plain, Interpreter::PowerShell);
        assert_eq!(&marked[..3], b"\xEF\xBB\xBF");
        assert_eq!(&marked[3..], plain);
        // Already marked (UTF-8 or UTF-16): left as the vendor sent it.
        assert_eq!(script_on_disk(&marked, Interpreter::PowerShell), marked);
        assert_eq!(script_on_disk(b"\xFF\xFEh\0i\0", Interpreter::PowerShell), b"\xFF\xFEh\0i\0");
        // A shell script is never touched: a mark before `#!` would stop it being one.
        assert_eq!(script_on_disk(b"#!/bin/sh\n", Interpreter::Bash), b"#!/bin/sh\n");
        assert_eq!(script_on_disk(b"#!/bin/sh\n", Interpreter::Sh), b"#!/bin/sh\n");
    }

    #[test]
    fn the_script_is_saved_with_its_extension_and_removed_with_its_path() {
        let path = write_script(b"echo hi\n", Interpreter::PowerShell).unwrap();
        let kept = path.to_path_buf();
        assert_eq!(kept.extension().and_then(|e| e.to_str()), Some("ps1"));
        assert_eq!(std::fs::read(&kept).unwrap(), b"\xEF\xBB\xBFecho hi\n", "a PowerShell file starts with the UTF-8 mark");
        drop(path);
        assert!(!kept.exists(), "the downloaded file is gone when the run ends");
    }

    #[test]
    fn the_program_in_a_failure_line_is_its_bare_name() {
        assert_eq!(program_name("bash"), "bash");
        assert_eq!(program_name("/usr/bin/python3"), "python3");
        assert_eq!(program_name("C:\\Program Files\\Python\\py.exe"), if cfg!(windows) { "py" } else { "C:\\Program Files\\Python\\py" });
    }

    #[tokio::test]
    async fn a_missing_program_is_a_spawn_failure_the_panel_can_read() {
        let state = crate::server::tests::test_state();
        let (_tx, mut rx) = oneshot::channel();
        let err = run_child(&run_of(&state), plan("agentmux-no-such-installer", &[]), &mut rx).await.unwrap_err();
        assert!(err.starts_with("spawn agentmux-no-such-installer:"), "{err}");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_failing_installer_reports_its_exit_code() {
        let state = crate::server::tests::test_state();
        let (_tx, mut rx) = oneshot::channel();
        let err = run_child(&run_of(&state), plan("sh", &["-c", "echo out; echo err >&2; exit 3"]), &mut rx).await.unwrap_err();
        assert_eq!(err, "sh exited Some(3)");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_clean_installer_succeeds() {
        let state = crate::server::tests::test_state();
        let (_tx, mut rx) = oneshot::channel();
        run_child(&run_of(&state), plan("sh", &["-c", "exit 0"]), &mut rx).await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn cancel_stops_the_installer_and_what_it_started() {
        let state = crate::server::tests::test_state();
        let (tx, mut rx) = oneshot::channel();
        tx.send(()).unwrap();
        let began = std::time::Instant::now();
        let err = run_child(&run_of(&state), plan("sh", &["-c", "sleep 30 & sleep 30"]), &mut rx).await.unwrap_err();
        assert_eq!(err, "cancelled");
        assert!(began.elapsed() < Duration::from_secs(20), "cancel did not wait for the installer");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn the_installer_gets_the_recipes_environment() {
        let state = crate::server::tests::test_state();
        let (_tx, mut rx) = oneshot::channel();
        let mut p = plan("sh", &["-c", r#"test "$CONFIGURE" = false"#]);
        p.env = vec![("CONFIGURE", "false")];
        run_child(&run_of(&state), p, &mut rx).await.unwrap();
    }
}
