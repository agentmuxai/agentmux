//! A container agent's login for a Claude account whose credentials are in the macOS Keychain.
//!
//! On Linux and Windows the bound account's `.credentials.json` is bind-mounted into the container. On macOS
//! Claude Code keeps that login in the Keychain, under `Claude Code-credentials-<first 8 hex of the sha256 of
//! the config dir>`, and never writes the file, so nothing was mounted and the container started signed out.
//!
//! Here srv reads that one Keychain entry and writes it to a staging file of its own, which is mounted like the
//! account's file would be. The Keychain stays the source of truth: when either side refreshes the token the
//! newer `expiresAt` wins, and a refresh made in the container is written back to the same entry, because refresh
//! tokens rotate and the host's login would otherwise stop working.
//!
//! The guards:
//! - only the bound account's own entry is read or written, by the service name computed from its config dir;
//!   nothing is searched or listed;
//! - the staging folder is `<AgentMux root>/container-credentials/<container>/<account>` (`0700`, the file `0600`), never
//!   in the account's dir or the agent's workspace, and it is deleted with the container;
//! - a refresh is written back only when the container's `.claude.json` names the same account as the host's
//!   and the staged `expiresAt` is newer than the Keychain's;
//! - each read and each write-back is one audit row (agent, account, direction), and no value is ever logged.

#![cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

use super::{ContainerManager, ContainerMountSpec, CLAUDE_CREDENTIALS_FILE};

/// Folder under the AgentMux root that holds one staging folder per container.
pub(crate) const STAGING_ROOT: &str = "container-credentials";
const META_FILE: &str = "meta.json";

/// The Keychain, as far as this module needs it. One entry is addressed by its service name.
pub(crate) trait Keychain {
    /// The entry's data, or `None` when there is no such entry.
    fn read(&self, service: &str) -> Result<Option<Zeroizing<String>>, String>;
    /// Replaces the data of the existing entry.
    fn write(&self, service: &str, value: &str) -> Result<(), String>;
}

/// The service name Claude Code uses on macOS for the login of `config_dir` (an absolute path, as
/// `CLAUDE_CONFIG_DIR` holds it).
pub(crate) fn service_name(config_dir: &str) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(config_dir.as_bytes());
    format!("Claude Code-credentials-{}", hex::encode(&digest[..4]))
}

/// Which way a login moved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Direction {
    KeychainToContainer,
    ContainerToKeychain,
}

/// One thing done, or refused, with a login: what the audit row says. Never carries a value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Moved {
    pub direction: Direction,
    /// The account's label (its email), or `unknown`.
    pub account: String,
    /// `Ok` when it was done; `Err` with the plain reason when it was refused or failed.
    pub outcome: Result<(), String>,
}

#[derive(Serialize, Deserialize)]
struct Meta {
    config_dir: String,
}

/// The expiry of a login in the Keychain's JSON, when that JSON holds a usable one.
fn expires_at(raw: &str) -> Option<i64> {
    let value: serde_json::Value = serde_json::from_str(raw).ok()?;
    let oauth = value.get("claudeAiOauth")?;
    for token in ["accessToken", "refreshToken"] {
        if oauth.get(token)?.as_str()?.is_empty() {
            return None;
        }
    }
    oauth.get("expiresAt")?.as_i64()
}

/// The furthest ahead a login's `expiresAt` may be and still be taken as a real refresh. Claude's access tokens
/// last hours, so a week is generous; a container can write its own `.credentials.json`, and without a limit a
/// far-future `expiresAt` would outrank every later refresh the host makes.
const MAX_EXPIRY_AHEAD_MS: i64 = 7 * 24 * 60 * 60 * 1000;

/// Whether `expires_at` (milliseconds since the epoch) is no further ahead than a real login's.
fn plausible_expiry(expires_at: i64) -> bool {
    expires_at <= agentmux_common::time::now_ms().saturating_add(MAX_EXPIRY_AHEAD_MS)
}

/// The account a Claude config dir (host or container) is signed in as: its id and its email.
fn account_in(claude_json: &str) -> (String, String) {
    let value: serde_json::Value = serde_json::from_str(claude_json).unwrap_or_default();
    let field = |name: &str| {
        value
            .pointer(&format!("/oauthAccount/{name}"))
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string()
    };
    (field("accountUuid"), field("emailAddress"))
}

fn host_account(config_dir: &str) -> (String, String) {
    let text =
        std::fs::read_to_string(Path::new(config_dir).join(".claude.json")).unwrap_or_default();
    account_in(&text)
}

/// The staging folder of the login of `config_dir` for `container_name`, under the AgentMux `root`. It is named by
/// the account, so an agent bound to another account mounts another path: the container is then recreated (the
/// mount source changed) and can't keep a bind to the previous account's file.
pub(crate) fn staging_dir(root: &Path, container_name: &str, config_dir: &str) -> PathBuf {
    use sha2::{Digest, Sha256};
    let account = hex::encode(&Sha256::digest(config_dir.as_bytes())[..6]);
    root.join(STAGING_ROOT).join(container_name).join(account)
}

#[cfg(unix)]
fn private(path: &Path, mode: u32) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
}
#[cfg(not(unix))]
fn private(_: &Path, _: u32) -> std::io::Result<()> {
    Ok(())
}

fn create_private_dir(dir: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    // The account's folder and the two above it (the container's, and the shared root of all of them).
    for folder in dir.ancestors().take(3) {
        private(folder, 0o700)?;
    }
    Ok(())
}

/// Writes `value` to `file`: in place when it exists, so a container that has it bind-mounted keeps seeing the
/// same file; created `0600` when it does not.
fn put_file(file: &Path, value: &str) -> std::io::Result<()> {
    use std::io::Write as _;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    options.open(file)?.write_all(value.as_bytes())?;
    private(file, 0o600)
}

/// Puts the Keychain's login in the staging folder, with the note of which config dir it is for.
fn stage(dir: &Path, file: &Path, config_dir: &str, raw: &str) -> Result<(), String> {
    create_private_dir(dir).map_err(|e| format!("couldn't create the staging folder: {e}"))?;
    put_file(file, raw).map_err(|e| format!("couldn't write the staged login: {e}"))?;
    let meta = serde_json::to_string(&Meta {
        config_dir: config_dir.to_string(),
    })
    .unwrap_or_default();
    put_file(&dir.join(META_FILE), &meta)
        .map_err(|e| format!("couldn't write the staging note: {e}"))
}

/// [`sync_account`] for the account the agent is bound to now, after dealing with the accounts it was bound to
/// before: what an earlier account's login refreshed goes back to its own entry, and its staged login is deleted
/// unless a refresh is still pending (then it stays, unmounted). Each account is synced on its own, so this does
/// not recurse.
pub(crate) fn prepare(
    keychain: &dyn Keychain,
    root: &Path,
    container_name: &str,
    config_dir: &str,
    container_account: Option<&str>,
    moved: &mut dyn FnMut(Moved),
) -> Result<PathBuf, String> {
    for earlier in staged_config_dirs(root, container_name)
        .into_iter()
        .filter(|earlier| earlier != config_dir)
    {
        let _ = sync_account(
            keychain,
            root,
            container_name,
            &earlier,
            container_account,
            &mut *moved,
        );
        if refresh_pending(keychain, root, container_name, &earlier) {
            tracing::warn!(container = container_name, "container agent: keeping the staged login of an account the agent is no longer bound to, it holds a refresh the Keychain doesn't have yet");
        } else {
            remove_staging(root, container_name, &earlier);
        }
    }
    sync_account(
        keychain,
        root,
        container_name,
        config_dir,
        container_account,
        moved,
    )
}

/// Brings the staged login of `container_name` in step with the Keychain entry of `config_dir`, and returns the
/// staging folder to mount (its `.credentials.json` is the login).
///
/// `container_account` is the account id the container's own `.claude.json` names, or `None` when it can't be
/// told (the container isn't running, or the file isn't there): a refresh made in the container is then not
/// written back, only the Keychain's newer login reaches the container.
///
/// `Err` when there is no login to give the container (no Keychain entry, or one that isn't a login), with the
/// reason in plain words; nothing is left staged in that case.
fn sync_account(
    keychain: &dyn Keychain,
    root: &Path,
    container_name: &str,
    config_dir: &str,
    container_account: Option<&str>,
    moved: &mut dyn FnMut(Moved),
) -> Result<PathBuf, String> {
    let dir = staging_dir(root, container_name, config_dir);
    let file = dir.join(CLAUDE_CREDENTIALS_FILE);
    let service = service_name(config_dir);
    let (host_uuid, label) = host_account(config_dir);
    let label = if label.is_empty() {
        "unknown".to_string()
    } else {
        label
    };

    let staged = std::fs::read_to_string(&file)
        .ok()
        .map(Zeroizing::new)
        .filter(|raw| expires_at(raw).is_some());
    let in_keychain = match keychain.read(&service) {
        Ok(entry) => entry.filter(|raw| expires_at(raw).is_some()),
        Err(e) => {
            let reason = format!("couldn't read the account's login from the Keychain: {e}");
            moved(Moved {
                direction: Direction::KeychainToContainer,
                account: label,
                outcome: Err(reason.clone()),
            });
            // A login that is already staged stays mounted and untouched, so the container is not changed under a
            // running agent and a refresh it made is not lost; it is synced at the next start that can read.
            return if staged.is_some() {
                Ok(dir)
            } else {
                Err(reason)
            };
        }
    };

    match (in_keychain, staged) {
        (None, None) => {
            let _ = std::fs::remove_dir_all(&dir);
            Err("this account has no login in the Keychain; sign in with it first".to_string())
        }
        // The Keychain entry is gone (signed out on the host) while a login is staged: leave both as they are and
        // never create the entry again from here.
        (None, Some(_)) => Ok(dir),
        (Some(kc), None) => {
            stage(&dir, &file, config_dir, &kc)?;
            moved(Moved {
                direction: Direction::KeychainToContainer,
                account: label,
                outcome: Ok(()),
            });
            Ok(dir)
        }
        (Some(kc), Some(staged)) => {
            let (kc_expiry, staged_expiry) = (
                expires_at(&kc).unwrap_or(0),
                expires_at(&staged).unwrap_or(0),
            );
            if kc_expiry >= staged_expiry && kc.as_str() == staged.as_str() {
                return Ok(dir);
            }
            if kc_expiry >= staged_expiry {
                stage(&dir, &file, config_dir, &kc)?;
                moved(Moved {
                    direction: Direction::KeychainToContainer,
                    account: label,
                    outcome: Ok(()),
                });
            } else {
                let outcome = if host_uuid.is_empty() {
                    Err("the host account's id isn't known, so the container's login can't be tied to it".to_string())
                } else if container_account != Some(host_uuid.as_str()) {
                    Err("the container isn't signed in as the bound account (or can't say), so its login stays there".to_string())
                } else if !plausible_expiry(staged_expiry) {
                    Err("the container's login expires further ahead than a real one does, so it isn't written to the Keychain".to_string())
                } else {
                    with_login_of(&kc, &staged)
                        .ok_or_else(|| {
                            "the Keychain entry or the staged login can't be read as JSON"
                                .to_string()
                        })
                        .and_then(|merged| write_verified(keychain, &service, &merged, &kc))
                };
                moved(Moved {
                    direction: Direction::ContainerToKeychain,
                    account: label,
                    outcome,
                });
            }
            Ok(dir)
        }
    }
}

/// Writes `value` to the entry and reads it back. When what is stored isn't `value`, the entry is put back to
/// `previous`, so a bad write never leaves the host signed out.
fn write_verified(
    keychain: &dyn Keychain,
    service: &str,
    value: &str,
    previous: &str,
) -> Result<(), String> {
    let wrote = keychain.write(service, value);
    let stored_ok = matches!(keychain.read(service), Ok(Some(raw)) if same_json(&raw, value));
    if wrote.is_ok() && stored_ok {
        return Ok(());
    }
    if !stored_ok {
        let _ = keychain.write(service, previous);
    }
    Err(match wrote {
        Err(e) => format!("couldn't write the refreshed login to the Keychain: {e}"),
        Ok(()) => "the Keychain didn't keep what was written, so the previous login was put back"
            .to_string(),
    })
}

/// Whether two JSON texts hold the same value, whatever their key order or spacing.
fn same_json(a: &str, b: &str) -> bool {
    match (
        serde_json::from_str::<serde_json::Value>(a),
        serde_json::from_str::<serde_json::Value>(b),
    ) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

/// The Keychain's JSON with only its `claudeAiOauth` replaced by the staged file's: whatever else the host keeps
/// in that entry stays as it is.
fn with_login_of(keychain_json: &str, staged_json: &str) -> Option<String> {
    let mut entry: serde_json::Value = serde_json::from_str(keychain_json).ok()?;
    let staged: serde_json::Value = serde_json::from_str(staged_json).ok()?;
    entry.as_object_mut()?.insert(
        "claudeAiOauth".to_string(),
        staged.get("claudeAiOauth")?.clone(),
    );
    serde_json::to_string(&entry).ok()
}

/// The config dirs that have a staged login for `container_name` (one per account it was bound to).
pub(crate) fn staged_config_dirs(root: &Path, container_name: &str) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(root.join(STAGING_ROOT).join(container_name)) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter_map(|entry| std::fs::read_to_string(entry.path().join(META_FILE)).ok())
        .filter_map(|text| serde_json::from_str::<Meta>(&text).ok())
        .map(|meta| meta.config_dir)
        .collect()
}

/// Whether the staged login of `config_dir` holds a refresh that is not in the Keychain yet: it expires later
/// than the Keychain's, or the Keychain can't be read to tell. Deleting the staged login then would lose a
/// rotated token.
pub(crate) fn refresh_pending(
    keychain: &dyn Keychain,
    root: &Path,
    container_name: &str,
    config_dir: &str,
) -> bool {
    let staged = std::fs::read_to_string(
        staging_dir(root, container_name, config_dir).join(CLAUDE_CREDENTIALS_FILE),
    )
    .ok()
    .map(Zeroizing::new)
    .and_then(|raw| expires_at(&raw));
    let Some(staged) = staged.filter(|&expiry| plausible_expiry(expiry)) else {
        return false;
    };
    match keychain.read(&service_name(config_dir)) {
        Err(_) => true,
        Ok(None) => false,
        Ok(Some(raw)) => expires_at(&raw).is_none_or(|kc| staged > kc),
    }
}

/// Deletes the staged login of `config_dir` for `container_name` (and the container's folder when it is empty).
pub(crate) fn remove_staging(root: &Path, container_name: &str, config_dir: &str) {
    let dir = staging_dir(root, container_name, config_dir);
    if dir.exists() {
        if let Err(e) = std::fs::remove_dir_all(&dir) {
            tracing::warn!(container = container_name, error = %e, "container agent: couldn't delete the staged login");
        }
    }
    let _ = std::fs::remove_dir(root.join(STAGING_ROOT).join(container_name));
}

/// The account id in the `.claude.json` that a Docker archive download holds (a tar with that one file).
fn account_in_archive(tar_bytes: &[u8]) -> Option<String> {
    use std::io::Read as _;
    const LIMIT: u64 = 4 * 1024 * 1024;
    let mut archive = tar::Archive::new(tar_bytes);
    let mut file = archive.entries().ok()?.next()?.ok()?;
    let mut text = String::new();
    file.by_ref().take(LIMIT).read_to_string(&mut text).ok()?;
    Some(account_in(&text).0).filter(|id| !id.is_empty())
}

/// What the pane says when a container agent's Claude login can't be given to its container on macOS.
pub(super) fn no_login_message(reason: &str) -> String {
    format!(
        "This agent's Claude account can't sign in inside its container: {reason}. Sign in with the account under \
         Connectors > Accounts (or bind an account that uses an API key), then send your message again."
    )
}

/// The real Keychain, through `/usr/bin/security`: the entries Claude Code writes allow that tool to read them
/// without a prompt.
#[cfg(target_os = "macos")]
pub(crate) struct SecurityCli;

#[cfg(target_os = "macos")]
impl SecurityCli {
    const LIMIT: std::time::Duration = std::time::Duration::from_secs(10);

    fn user() -> Result<String, String> {
        std::env::var("USER")
            .ok()
            .filter(|u| !u.is_empty())
            .ok_or_else(|| "no USER in the environment".to_string())
    }

    /// Runs `security` with `args` (and `stdin` when given), waits at most [`Self::LIMIT`], and returns the exit
    /// code and stdout. Polls rather than spending a thread on the wait.
    fn run(args: &[&str], stdin: Option<&str>) -> Result<(i32, Zeroizing<String>), String> {
        use std::io::{Read as _, Write as _};
        use std::process::{Command, Stdio};
        let mut command = Command::new("/usr/bin/security");
        command
            .args(args)
            .stdin(if stdin.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        // The tool needs nothing of this instance's identity.
        crate::backend::pane_env::sanitize_external_std_command(&mut command);
        let mut child = command.spawn().map_err(|e| e.to_string())?;
        if let (Some(text), Some(mut pipe)) = (stdin, child.stdin.take()) {
            pipe.write_all(text.as_bytes()).map_err(|e| e.to_string())?;
        }
        let deadline = std::time::Instant::now() + Self::LIMIT;
        let status = loop {
            if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
                break status;
            }
            if std::time::Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                return Err("the Keychain didn't answer in time".to_string());
            }
            std::thread::sleep(std::time::Duration::from_millis(25));
        };
        let mut out = Zeroizing::new(String::new());
        if let Some(mut pipe) = child.stdout.take() {
            let _ = pipe.read_to_string(&mut out);
        }
        Ok((status.code().unwrap_or(-1), out))
    }
}

/// The longest line `security -i` takes. Past about 4 KB it runs the first part as a command and then fails on the
/// rest, which has already replaced the entry with a cut-off value.
#[cfg(target_os = "macos")]
const MAX_SECURITY_LINE: usize = 3900;

/// How `security` is asked to replace the data of the entry `service` / `user`: the arguments, and the line for
/// its stdin when there is one.
#[cfg(target_os = "macos")]
struct WriteCall {
    args: Vec<String>,
    stdin: Option<Zeroizing<String>>,
}

/// The call that replaces the entry's data. Through `security -i` while the line fits, so the login is read from
/// stdin and never shows in a process listing; hex, so no quoting of the JSON is needed. A larger entry (one that
/// keeps many MCP tokens) goes as the `-X` argument instead, which has no such limit: for a few milliseconds the
/// login is then in the arguments of a process only the same user can list (the operator treats same-user processes as trusted).
#[cfg(target_os = "macos")]
fn write_call(service: &str, user: &str, value: &str) -> WriteCall {
    let hex = Zeroizing::new(hex::encode(value));
    let line = Zeroizing::new(format!(
        "add-generic-password -U -s \"{service}\" -a \"{user}\" -X \"{}\"\n",
        hex.as_str()
    ));
    if line.len() <= MAX_SECURITY_LINE {
        return WriteCall {
            args: vec!["-i".to_string()],
            stdin: Some(line),
        };
    }
    WriteCall {
        args: [
            "add-generic-password",
            "-U",
            "-s",
            service,
            "-a",
            user,
            "-X",
            hex.as_str(),
        ]
        .map(String::from)
        .to_vec(),
        stdin: None,
    }
}

#[cfg(target_os = "macos")]
impl Keychain for SecurityCli {
    fn read(&self, service: &str) -> Result<Option<Zeroizing<String>>, String> {
        let user = Self::user()?;
        let (code, out) = Self::run(
            &["find-generic-password", "-s", service, "-a", &user, "-w"],
            None,
        )?;
        match code {
            0 => Ok(Some(Zeroizing::new(out.trim_end_matches('\n').to_string()))),
            // errSecItemNotFound
            44 => Ok(None),
            other => Err(format!("security exited with {other}")),
        }
    }

    fn write(&self, service: &str, value: &str) -> Result<(), String> {
        let user = Self::user()?;
        let call = write_call(service, &user, value);
        let args: Vec<&str> = call.args.iter().map(String::as_str).collect();
        let (code, _) = Self::run(&args, call.stdin.as_deref().map(String::as_str))?;
        if code == 0 {
            Ok(())
        } else {
            Err(format!("security exited with {code}"))
        }
    }
}

impl ContainerManager {
    /// Stages the Keychain login of the account whose config dir is `config_dir` for the container about to be
    /// created, and returns the folder to mount as the account's credentials; `Err` with the reason in plain words
    /// when there is no login to give. Only called on macOS, where a config dir has a `keychain_config_dir`.
    pub(crate) async fn stage_keychain_login(
        &self,
        container_name: &str,
        config_dir: &str,
    ) -> Result<String, String> {
        #[cfg(target_os = "macos")]
        {
            self.sync_keychain_login(container_name, config_dir, true)
                .await
                .map(|dir| dir.to_string_lossy().into_owned())
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = (container_name, config_dir);
            Err("a Keychain login only exists on macOS".to_string())
        }
    }

    /// After a drift removal, which is followed at once by a create from the same mount spec: the folder `remove`
    /// deleted is made again, since Docker refuses a bind mount whose source is missing.
    pub(crate) async fn restage_keychain_login(
        &self,
        container_name: &str,
        spec: &ContainerMountSpec,
    ) {
        if let Some(config_dir) = spec.keychain_config_dir.as_deref() {
            let _ = self.stage_keychain_login(container_name, config_dir).await;
        }
    }

    /// Before a container is removed: writes a login it refreshed back to the Keychain, while the container is
    /// still there to ask which account it is signed in as. Best effort; it never fails the removal.
    pub(crate) async fn write_back_keychain_login(&self, container_name: &str) {
        #[cfg(target_os = "macos")]
        if let Ok(root) = agentmux_common::data_paths::agentmux_root() {
            for config_dir in staged_config_dirs(&root, container_name) {
                let _ = self
                    .sync_keychain_login(container_name, &config_dir, false)
                    .await;
            }
        }
        #[cfg(not(target_os = "macos"))]
        let _ = container_name;
    }

    /// After a container is removed: deletes its staged logins, except one that holds a refresh that did not reach
    /// the Keychain (it stays until a later sync can write it back).
    pub(crate) async fn drop_staged_keychain_login(&self, container_name: &str) {
        let Ok(root) = agentmux_common::data_paths::agentmux_root() else {
            return;
        };
        for config_dir in staged_config_dirs(&root, container_name) {
            #[cfg(target_os = "macos")]
            {
                let (root_for_check, name, dir) =
                    (root.clone(), container_name.to_string(), config_dir.clone());
                let pending = tokio::task::spawn_blocking(move || {
                    refresh_pending(&SecurityCli, &root_for_check, &name, &dir)
                })
                .await
                .unwrap_or(true);
                if pending {
                    tracing::warn!(container = container_name, "container agent: keeping the staged login, it holds a refresh the Keychain doesn't have yet");
                    continue;
                }
            }
            remove_staging(&root, container_name, &config_dir);
        }
    }

    /// The account id the container's own `.claude.json` names, or `None` when there is no container or file. Read
    /// through Docker's archive API, which works for a stopped container too: a container is stopped before it is
    /// removed, and its login must be written back then.
    #[cfg(target_os = "macos")]
    async fn container_account(&self, container_name: &str) -> Option<String> {
        use bollard::container::DownloadFromContainerOptions;
        use futures_util::StreamExt as _;
        const LIMIT: usize = 4 * 1024 * 1024;
        let path = format!("{}/.claude.json", super::CONTAINER_CLAUDE_DIR);
        let mut stream = self
            .inner
            .docker
            .download_from_container(container_name, Some(DownloadFromContainerOptions { path }));
        let mut tar_bytes = Vec::new();
        while let Some(chunk) = stream.next().await {
            tar_bytes.extend_from_slice(&chunk.ok()?);
            if tar_bytes.len() > LIMIT {
                return None;
            }
        }
        account_in_archive(&tar_bytes)
    }

    #[cfg(target_os = "macos")]
    async fn sync_keychain_login(
        &self,
        container_name: &str,
        config_dir: &str,
        rebind_cleanup: bool,
    ) -> Result<PathBuf, String> {
        let root = agentmux_common::data_paths::agentmux_root()
            .map_err(|e| format!("AgentMux's data folder isn't available: {e}"))?;
        let account = self.container_account(container_name).await;
        let (name, dir) = (container_name.to_string(), config_dir.to_string());
        let (result, rows) = tokio::task::spawn_blocking(move || {
            let mut rows = Vec::new();
            let sync = if rebind_cleanup {
                prepare
            } else {
                sync_account
            };
            let result = sync(
                &SecurityCli,
                &root,
                &name,
                &dir,
                account.as_deref(),
                &mut |m| rows.push(m),
            );
            (result, rows)
        })
        .await
        .map_err(|e| format!("reading the Keychain was interrupted: {e}"))?;
        for row in &rows {
            audit(container_name, row);
        }
        match result {
            Ok(dir) => Ok(dir),
            Err(reason) => {
                tracing::warn!(container = container_name, %reason, "container agent: no Keychain login to give the container");
                Err(reason)
            }
        }
    }
}

/// One audit row for a login that moved (or was refused), in the same trail as the fleet actions. The row names the
/// container (which carries the agent's id) and the account's label; never a value.
#[cfg(target_os = "macos")]
fn audit(container_name: &str, moved: &Moved) {
    let (action, how) = match moved.direction {
        Direction::KeychainToContainer => ("container_credential_read", "Keychain to container"),
        Direction::ContainerToKeychain => {
            ("container_credential_writeback", "container to Keychain")
        }
    };
    let reason = format!("account {}: {how}", moved.account);
    let error = moved.outcome.as_ref().err().map(String::as_str);
    crate::backend::reactive::handler::get_global_handler().log_fleet_action_audit(
        Some("agentmux"),
        container_name,
        "",
        action,
        moved.outcome.is_ok(),
        error,
        &uuid::Uuid::new_v4().to_string(),
        Some(&reason),
    );
}

#[cfg(test)]
mod tests;
