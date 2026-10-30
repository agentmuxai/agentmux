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
                } else {
                    with_login_of(&kc, &staged)
                        .ok_or_else(|| {
                            "the Keychain entry or the staged login can't be read as JSON"
                                .to_string()
                        })
                        .and_then(|merged| {
                            keychain.write(&service, &merged).map_err(|e| {
                                format!("couldn't write the refreshed login to the Keychain: {e}")
                            })
                        })
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
    let Some(staged) = staged else { return false };
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
        // Through `security -i` so the login is read from stdin and never shows in a process listing; hex, so no
        // quoting of the JSON is needed.
        let command = Zeroizing::new(format!(
            "add-generic-password -U -s \"{service}\" -a \"{user}\" -X \"{}\"\n",
            hex::encode(value)
        ));
        let (code, _) = Self::run(&["-i"], Some(&command))?;
        if code == 0 {
            Ok(())
        } else {
            Err(format!("security exited with {code}"))
        }
    }
}

impl ContainerManager {
    /// Stages the Keychain login of the account whose config dir is `config_dir` for the container about to be
    /// created, and returns the folder to mount as the account's credentials; `None` when there is no login to
    /// give (the reason is logged and the container starts signed out, as before). Does nothing off macOS.
    pub(crate) async fn stage_keychain_login(
        &self,
        container_name: &str,
        config_dir: &str,
    ) -> Option<String> {
        #[cfg(target_os = "macos")]
        {
            self.sync_keychain_login(container_name, config_dir, true)
                .await
                .map(|dir| dir.to_string_lossy().into_owned())
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = (container_name, config_dir);
            None
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
    ) -> Option<PathBuf> {
        let root = agentmux_common::data_paths::agentmux_root().ok()?;
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
        .ok()?;
        for row in &rows {
            audit(container_name, row);
        }
        match result {
            Ok(dir) => Some(dir),
            Err(reason) => {
                tracing::warn!(container = container_name, %reason, "container agent: no Keychain login to give the container; it starts signed out");
                None
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
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::collections::HashMap;

    struct Fake {
        entries: RefCell<HashMap<String, String>>,
        touched: RefCell<Vec<String>>,
        unreadable: std::cell::Cell<bool>,
    }
    impl Fake {
        fn with(service: &str, raw: Option<String>) -> Self {
            let fake = Fake {
                entries: RefCell::new(HashMap::new()),
                touched: RefCell::new(Vec::new()),
                unreadable: std::cell::Cell::new(false),
            };
            if let Some(raw) = raw {
                fake.entries.borrow_mut().insert(service.to_string(), raw);
            }
            fake
        }
    }
    impl Keychain for Fake {
        fn read(&self, service: &str) -> Result<Option<Zeroizing<String>>, String> {
            self.touched.borrow_mut().push(format!("read {service}"));
            if self.unreadable.get() {
                return Err("locked".to_string());
            }
            Ok(self
                .entries
                .borrow()
                .get(service)
                .cloned()
                .map(Zeroizing::new))
        }
        fn write(&self, service: &str, value: &str) -> Result<(), String> {
            self.touched.borrow_mut().push(format!("write {service}"));
            self.entries
                .borrow_mut()
                .insert(service.to_string(), value.to_string());
            Ok(())
        }
    }

    const ACCOUNT: &str = "acct-1111";

    /// The same JSON whatever order its keys were written in.
    fn json(text: &str) -> serde_json::Value {
        serde_json::from_str(text).unwrap()
    }

    fn login(expires: i64, tag: &str) -> String {
        format!(
            r#"{{"claudeAiOauth":{{"accessToken":"a-{tag}","refreshToken":"r-{tag}","expiresAt":{expires}}}}}"#
        )
    }

    /// A host config dir signed in as `ACCOUNT`, and an AgentMux root, both temporary.
    fn setup() -> (tempfile::TempDir, String) {
        let tmp = tempfile::tempdir().unwrap();
        let config = tmp.path().join("account");
        std::fs::create_dir_all(&config).unwrap();
        std::fs::write(config.join(".claude.json"), format!(r#"{{"oauthAccount":{{"accountUuid":"{ACCOUNT}","emailAddress":"me@example.com"}}}}"#)).unwrap();
        let dir = config.to_string_lossy().into_owned();
        (tmp, dir)
    }

    fn run(
        kc: &Fake,
        tmp: &tempfile::TempDir,
        config: &str,
        container_account: Option<&str>,
    ) -> (Result<PathBuf, String>, Vec<Moved>) {
        let mut moved = Vec::new();
        let result = prepare(
            kc,
            &tmp.path().join("root"),
            "agentmux-c1",
            config,
            container_account,
            &mut |m| moved.push(m),
        );
        (result, moved)
    }

    #[test]
    fn the_service_name_is_claudes_for_that_config_dir() {
        // A placeholder path: the name is a hash of the whole path, so any path has one.
        let dir = "/Users/example/.agentmux/shared/identities/00000000-0000-0000-0000-000000000000/claude";
        assert_eq!(service_name(dir), "Claude Code-credentials-c6fb88e5");
    }

    #[test]
    fn the_first_start_stages_the_keychain_login_privately_and_only_reads_that_entry() {
        let (tmp, config) = setup();
        let service = service_name(&config);
        let kc = Fake::with(&service, Some(login(100, "kc")));
        let (result, moved) = run(&kc, &tmp, &config, None);
        let dir = result.unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.join(".credentials.json")).unwrap(),
            login(100, "kc")
        );
        assert_eq!(
            *kc.touched.borrow(),
            vec![format!("read {service}")],
            "only the bound account's own entry is touched"
        );
        assert_eq!(
            moved,
            vec![Moved {
                direction: Direction::KeychainToContainer,
                account: "me@example.com".into(),
                outcome: Ok(())
            }]
        );
        assert!(
            dir.starts_with(tmp.path().join("root").join(STAGING_ROOT))
                && !dir.starts_with(&config)
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode(&dir), 0o700);
            assert_eq!(mode(&dir.join(".credentials.json")), 0o600);
        }
    }

    #[test]
    fn no_login_in_the_keychain_stages_nothing_and_says_so() {
        let (tmp, config) = setup();
        let kc = Fake::with(&service_name(&config), None);
        let (result, moved) = run(&kc, &tmp, &config, None);
        assert!(result.unwrap_err().contains("no login in the Keychain"));
        assert!(moved.is_empty());
        assert!(!staging_dir(&tmp.path().join("root"), "agentmux-c1", &config).exists());
        let kc = Fake::with(&service_name(&config), Some("not a login".into()));
        assert!(run(&kc, &tmp, &config, None).0.is_err());
    }

    #[test]
    fn a_login_the_host_refreshed_reaches_the_staged_file_in_place() {
        let (tmp, config) = setup();
        let service = service_name(&config);
        let kc = Fake::with(&service, Some(login(100, "old")));
        let dir = run(&kc, &tmp, &config, None).0.unwrap();
        let file = dir.join(".credentials.json");
        #[cfg(unix)]
        let inode = std::os::unix::fs::MetadataExt::ino(&std::fs::metadata(&file).unwrap());
        kc.entries.borrow_mut().insert(service, login(200, "new"));
        let (result, _) = run(&kc, &tmp, &config, Some(ACCOUNT));
        assert_eq!(result.unwrap(), dir);
        assert_eq!(std::fs::read_to_string(&file).unwrap(), login(200, "new"));
        #[cfg(unix)]
        assert_eq!(
            std::os::unix::fs::MetadataExt::ino(&std::fs::metadata(&file).unwrap()),
            inode,
            "the mounted file stays the same file"
        );
    }

    #[test]
    fn a_login_the_container_refreshed_is_written_back_for_the_same_account() {
        let (tmp, config) = setup();
        let service = service_name(&config);
        let kc = Fake::with(&service, Some(login(100, "old")));
        let dir = run(&kc, &tmp, &config, None).0.unwrap();
        std::fs::write(dir.join(".credentials.json"), login(300, "container")).unwrap();
        let (_, moved) = run(&kc, &tmp, &config, Some(ACCOUNT));
        assert_eq!(
            json(&kc.entries.borrow()[&service]),
            json(&login(300, "container"))
        );
        assert_eq!(
            moved,
            vec![Moved {
                direction: Direction::ContainerToKeychain,
                account: "me@example.com".into(),
                outcome: Ok(())
            }]
        );
        assert_eq!(
            kc.touched
                .borrow()
                .iter()
                .filter(|t| t.starts_with("write"))
                .count(),
            1
        );
    }

    #[test]
    fn a_login_for_another_account_or_an_unknown_one_is_not_written_back() {
        for container_account in [Some("someone-else"), None] {
            let (tmp, config) = setup();
            let service = service_name(&config);
            let kc = Fake::with(&service, Some(login(100, "old")));
            let dir = run(&kc, &tmp, &config, None).0.unwrap();
            std::fs::write(dir.join(".credentials.json"), login(300, "other")).unwrap();
            let (_, moved) = run(&kc, &tmp, &config, container_account);
            assert_eq!(
                kc.entries.borrow()[&service],
                login(100, "old"),
                "the Keychain entry is untouched"
            );
            assert!(!kc.touched.borrow().iter().any(|t| t.starts_with("write")));
            assert!(
                matches!(
                    moved.as_slice(),
                    [Moved {
                        direction: Direction::ContainerToKeychain,
                        outcome: Err(_),
                        ..
                    }]
                ),
                "{moved:?}"
            );
        }
    }

    #[test]
    fn a_staged_login_that_is_older_than_the_keychains_is_replaced_not_written_back() {
        let (tmp, config) = setup();
        let service = service_name(&config);
        let kc = Fake::with(&service, Some(login(500, "kc")));
        let dir = run(&kc, &tmp, &config, None).0.unwrap();
        std::fs::write(dir.join(".credentials.json"), login(400, "stale")).unwrap();
        let _ = run(&kc, &tmp, &config, Some(ACCOUNT));
        assert_eq!(kc.entries.borrow()[&service], login(500, "kc"));
        assert_eq!(
            std::fs::read_to_string(dir.join(".credentials.json")).unwrap(),
            login(500, "kc")
        );
        assert!(!kc.touched.borrow().iter().any(|t| t.starts_with("write")));
    }

    #[test]
    fn a_host_account_with_no_known_id_never_gets_a_write_back() {
        let (tmp, config) = setup();
        std::fs::remove_file(Path::new(&config).join(".claude.json")).unwrap();
        let service = service_name(&config);
        let kc = Fake::with(&service, Some(login(100, "old")));
        let dir = run(&kc, &tmp, &config, None).0.unwrap();
        std::fs::write(dir.join(".credentials.json"), login(300, "c")).unwrap();
        let (_, moved) = run(&kc, &tmp, &config, Some("")); // an empty id never matches
        assert!(!kc.touched.borrow().iter().any(|t| t.starts_with("write")));
        assert!(matches!(
            moved.as_slice(),
            [Moved {
                outcome: Err(_),
                ..
            }]
        ));
    }

    #[test]
    fn a_signed_out_keychain_entry_is_not_created_again_from_the_staged_login() {
        let (tmp, config) = setup();
        let service = service_name(&config);
        let kc = Fake::with(&service, Some(login(100, "old")));
        let dir = run(&kc, &tmp, &config, None).0.unwrap();
        std::fs::write(dir.join(".credentials.json"), login(300, "c")).unwrap();
        kc.entries.borrow_mut().clear();
        assert_eq!(run(&kc, &tmp, &config, Some(ACCOUNT)).0.unwrap(), dir);
        assert!(!kc.touched.borrow().iter().any(|t| t.starts_with("write")));
    }

    #[test]
    fn removing_the_container_deletes_the_staged_login_and_its_note() {
        let (tmp, config) = setup();
        let kc = Fake::with(&service_name(&config), Some(login(100, "kc")));
        let root = tmp.path().join("root");
        run(&kc, &tmp, &config, None).0.unwrap();
        assert_eq!(
            staged_config_dirs(&root, "agentmux-c1"),
            vec![config.clone()]
        );
        remove_staging(&root, "agentmux-c1", &config);
        assert!(!staging_dir(&root, "agentmux-c1", &config).exists());
        assert!(staged_config_dirs(&root, "agentmux-c1").is_empty());
        assert!(
            !root.join(STAGING_ROOT).join("agentmux-c1").exists(),
            "the container's folder goes with its last login"
        );
        remove_staging(&root, "agentmux-c1", &config); // already gone: fine
    }

    /// Against the real login keychain, with an entry of its own that this test creates and deletes (never a
    /// Claude entry): `cargo test -p agentmux-srv -- --ignored keychain_credentials_live`
    #[cfg(target_os = "macos")]
    #[test]
    #[ignore = "uses the real login keychain"]
    fn keychain_credentials_live_round_trip_through_security_cli() {
        let service = format!(
            "agentmux-test-container-login-{}",
            uuid::Uuid::new_v4().simple()
        );
        let user = std::env::var("USER").unwrap();
        let first = login(100, "first");
        let added = std::process::Command::new("/usr/bin/security")
            .args([
                "add-generic-password",
                "-s",
                &service,
                "-a",
                &user,
                "-w",
                &first,
            ])
            .status()
            .unwrap();
        assert!(added.success());
        let keychain = SecurityCli;
        let read = keychain.read(&service).map(|v| v.map(|z| z.to_string()));
        let second = login(200, "second \"quoted\" \\ and spaces");
        let written = keychain.write(&service, &second);
        let read_again = keychain.read(&service).map(|v| v.map(|z| z.to_string()));
        let missing = keychain
            .read(&format!("{service}-absent"))
            .map(|v| v.map(|z| z.to_string()));
        let _ = std::process::Command::new("/usr/bin/security")
            .args(["delete-generic-password", "-s", &service, "-a", &user])
            .status();
        assert_eq!(read, Ok(Some(first)));
        assert_eq!(written, Ok(()));
        assert_eq!(
            read_again,
            Ok(Some(second)),
            "an update keeps the text exactly"
        );
        assert_eq!(missing, Ok(None));
    }

    /// The whole flow against the real login keychain, for a throwaway config dir (so a throwaway service name):
    /// stage, a refresh in the container written back, a refresh on the host brought in.
    #[cfg(target_os = "macos")]
    #[test]
    #[ignore = "uses the real login keychain"]
    fn keychain_credentials_live_prepare_flow() {
        let (tmp, config) = setup();
        let service = service_name(&config);
        let user = std::env::var("USER").unwrap();
        let security = |args: &[&str]| {
            std::process::Command::new("/usr/bin/security")
                .args(args)
                .status()
                .unwrap()
                .success()
        };
        assert!(security(&[
            "add-generic-password",
            "-s",
            &service,
            "-a",
            &user,
            "-w",
            &login(100, "host")
        ]));
        let root = tmp.path().join("root");
        let mut moved = Vec::new();
        let outcome = (|| -> Result<(), String> {
            let dir = prepare(
                &SecurityCli,
                &root,
                "agentmux-live",
                &config,
                None,
                &mut |m| moved.push(m),
            )?;
            let file = dir.join(".credentials.json");
            assert_eq!(std::fs::read_to_string(&file).unwrap(), login(100, "host"));
            std::fs::write(&file, login(300, "container")).unwrap();
            prepare(
                &SecurityCli,
                &root,
                "agentmux-live",
                &config,
                Some(ACCOUNT),
                &mut |m| moved.push(m),
            )?;
            assert_eq!(
                SecurityCli.read(&service)?.map(|z| json(&z)),
                Some(json(&login(300, "container")))
            );
            assert!(security(&[
                "add-generic-password",
                "-U",
                "-s",
                &service,
                "-a",
                &user,
                "-w",
                &login(500, "host-again")
            ]));
            prepare(
                &SecurityCli,
                &root,
                "agentmux-live",
                &config,
                Some(ACCOUNT),
                &mut |m| moved.push(m),
            )?;
            assert_eq!(
                std::fs::read_to_string(&file).unwrap(),
                login(500, "host-again")
            );
            Ok(())
        })();
        let _ = security(&["delete-generic-password", "-s", &service, "-a", &user]);
        outcome.unwrap();
        let directions: Vec<_> = moved
            .iter()
            .map(|m| (m.direction, m.outcome.is_ok()))
            .collect();
        assert_eq!(
            directions,
            vec![
                (Direction::KeychainToContainer, true),
                (Direction::ContainerToKeychain, true),
                (Direction::KeychainToContainer, true)
            ]
        );
    }

    #[test]
    fn the_containers_account_is_read_from_the_archive_docker_returns() {
        let tar_of = |name: &str, body: &str| {
            let mut builder = tar::Builder::new(Vec::new());
            let mut header = tar::Header::new_gnu();
            header.set_size(body.len() as u64);
            header.set_mode(0o600);
            header.set_cksum();
            builder
                .append_data(&mut header, name, body.as_bytes())
                .unwrap();
            builder.into_inner().unwrap()
        };
        let with = tar_of(
            ".claude.json",
            r#"{"oauthAccount":{"accountUuid":"acct-1111"},"other":1}"#,
        );
        assert_eq!(account_in_archive(&with).as_deref(), Some("acct-1111"));
        assert_eq!(account_in_archive(&tar_of(".claude.json", "{}")), None);
        assert_eq!(
            account_in_archive(&tar_of(".claude.json", "not json")),
            None
        );
        assert_eq!(account_in_archive(b""), None);
    }

    #[test]
    fn an_unreadable_keychain_keeps_the_staged_login_and_deletes_nothing() {
        let (tmp, config) = setup();
        let kc = Fake::with(&service_name(&config), Some(login(100, "old")));
        let dir = run(&kc, &tmp, &config, None).0.unwrap();
        std::fs::write(dir.join(".credentials.json"), login(300, "container")).unwrap();
        kc.unreadable.set(true);
        let (result, moved) = run(&kc, &tmp, &config, Some(ACCOUNT));
        assert_eq!(
            result.unwrap(),
            dir,
            "the mount does not change under a running agent"
        );
        assert_eq!(
            std::fs::read_to_string(dir.join(".credentials.json")).unwrap(),
            login(300, "container")
        );
        assert!(
            matches!(
                moved.as_slice(),
                [Moved {
                    outcome: Err(_),
                    ..
                }]
            ),
            "{moved:?}"
        );
        assert!(!kc.touched.borrow().iter().any(|t| t.starts_with("write")));
        // With nothing staged, an unreadable Keychain is an error the pane can show.
        let (tmp2, config2) = setup();
        let kc2 = Fake::with(&service_name(&config2), Some(login(100, "x")));
        kc2.unreadable.set(true);
        assert!(run(&kc2, &tmp2, &config2, None).0.is_err());
    }

    #[test]
    fn a_staged_refresh_the_keychain_lacks_is_pending_and_is_not_deleted_with_the_container() {
        let (tmp, config) = setup();
        let root = tmp.path().join("root");
        let service = service_name(&config);
        let kc = Fake::with(&service, Some(login(100, "old")));
        assert!(
            !refresh_pending(&kc, &root, "agentmux-c1", &config),
            "nothing staged"
        );
        let dir = run(&kc, &tmp, &config, None).0.unwrap();
        assert!(
            !refresh_pending(&kc, &root, "agentmux-c1", &config),
            "staged equals the Keychain's"
        );
        std::fs::write(dir.join(".credentials.json"), login(300, "container")).unwrap();
        assert!(
            refresh_pending(&kc, &root, "agentmux-c1", &config),
            "the container refreshed and nothing was written back"
        );
        kc.unreadable.set(true);
        assert!(
            refresh_pending(&kc, &root, "agentmux-c1", &config),
            "can't tell: keep it"
        );
        kc.unreadable.set(false);
        kc.entries
            .borrow_mut()
            .insert(service, login(300, "container"));
        assert!(
            !refresh_pending(&kc, &root, "agentmux-c1", &config),
            "written back: safe to delete"
        );
    }

    #[test]
    fn rebinding_the_agent_to_another_account_never_mounts_the_old_accounts_login() {
        let (tmp, config_a) = setup();
        let config_b = tmp.path().join("account-b");
        std::fs::create_dir_all(&config_b).unwrap();
        std::fs::write(
            config_b.join(".claude.json"),
            r#"{"oauthAccount":{"accountUuid":"acct-2222","emailAddress":"b@example.com"}}"#,
        )
        .unwrap();
        let config_b = config_b.to_string_lossy().into_owned();
        let root = tmp.path().join("root");
        let (service_a, service_b) = (service_name(&config_a), service_name(&config_b));
        let kc = Fake::with(&service_a, Some(login(100, "a")));
        let dir_a = run(&kc, &tmp, &config_a, None).0.unwrap();
        std::fs::write(dir_a.join(".credentials.json"), login(300, "a-refreshed")).unwrap();
        assert_ne!(
            staging_dir(&root, "agentmux-c1", &config_a),
            staging_dir(&root, "agentmux-c1", &config_b),
            "another account mounts another path, so the container is recreated"
        );

        // B has no login yet: nothing of A's is offered for it, and A's refresh went back to A's entry.
        let (result, _) = run(&kc, &tmp, &config_b, Some(ACCOUNT));
        assert!(
            result.is_err(),
            "B has no login and A's is not offered in its place"
        );
        assert_eq!(
            json(&kc.entries.borrow()[&service_a]),
            json(&login(300, "a-refreshed"))
        );
        assert!(
            !dir_a.exists(),
            "A's refresh is in the Keychain, so its staged login is deleted"
        );

        // B gets a login: the staged file is B's, at B's own path.
        kc.entries.borrow_mut().insert(service_b, login(50, "b"));
        let dir_b = run(&kc, &tmp, &config_b, None).0.unwrap();
        assert_ne!(dir_b, dir_a);
        assert_eq!(
            std::fs::read_to_string(dir_b.join(".credentials.json")).unwrap(),
            login(50, "b")
        );
        assert_eq!(staged_config_dirs(&root, "agentmux-c1"), vec![config_b]);
    }

    #[test]
    fn an_old_accounts_pending_refresh_is_kept_when_the_agent_is_rebound() {
        let (tmp, config_a) = setup();
        let config_b = tmp.path().join("account-b").to_string_lossy().into_owned();
        std::fs::create_dir_all(&config_b).unwrap();
        let service_a = service_name(&config_a);
        let kc = Fake::with(&service_a, Some(login(100, "a")));
        kc.entries
            .borrow_mut()
            .insert(service_name(&config_b), login(50, "b"));
        let dir_a = run(&kc, &tmp, &config_a, None).0.unwrap();
        std::fs::write(dir_a.join(".credentials.json"), login(300, "a-refreshed")).unwrap();
        // The container's account can't be told, so A's refresh can't be written back yet.
        let dir_b = run(&kc, &tmp, &config_b, None).0.unwrap();
        assert_ne!(dir_a, dir_b);
        assert_eq!(
            kc.entries.borrow()[&service_a],
            login(100, "a"),
            "not written back"
        );
        assert_eq!(
            std::fs::read_to_string(dir_a.join(".credentials.json")).unwrap(),
            login(300, "a-refreshed"),
            "kept, not deleted"
        );
    }

    #[test]
    fn syncing_again_while_two_accounts_are_staged_terminates_and_keeps_the_pending_one() {
        let (tmp, config_a) = setup();
        let config_b = tmp.path().join("account-b").to_string_lossy().into_owned();
        std::fs::create_dir_all(&config_b).unwrap();
        let kc = Fake::with(&service_name(&config_a), Some(login(100, "a")));
        kc.entries
            .borrow_mut()
            .insert(service_name(&config_b), login(50, "b"));
        let dir_a = run(&kc, &tmp, &config_a, None).0.unwrap();
        std::fs::write(dir_a.join(".credentials.json"), login(300, "a-refreshed")).unwrap();
        let dir_b = run(&kc, &tmp, &config_b, None).0.unwrap();
        let root = tmp.path().join("root");
        assert_eq!(
            staged_config_dirs(&root, "agentmux-c1").len(),
            2,
            "A stays staged, B is mounted"
        );
        // Every turn start, and the sync before a removal, run again over both.
        for _ in 0..3 {
            assert_eq!(run(&kc, &tmp, &config_b, None).0.unwrap(), dir_b);
            let mut moved = Vec::new();
            for dir in staged_config_dirs(&root, "agentmux-c1") {
                let _ = sync_account(&kc, &root, "agentmux-c1", &dir, None, &mut |m| {
                    moved.push(m)
                });
            }
        }
        assert_eq!(staged_config_dirs(&root, "agentmux-c1").len(), 2);
        assert_eq!(
            std::fs::read_to_string(dir_a.join(".credentials.json")).unwrap(),
            login(300, "a-refreshed")
        );
    }

    #[test]
    fn a_write_back_replaces_only_the_login_and_keeps_the_rest_of_the_entry() {
        let (tmp, config) = setup();
        let service = service_name(&config);
        let with_extra = r#"{"claudeAiOauth":{"accessToken":"a","refreshToken":"r","expiresAt":100},"mcpOAuth":{"server":{"token":"keep"}}}"#;
        let kc = Fake::with(&service, Some(with_extra.to_string()));
        let dir = run(&kc, &tmp, &config, None).0.unwrap();
        std::fs::write(dir.join(".credentials.json"), login(300, "container")).unwrap();
        let _ = run(&kc, &tmp, &config, Some(ACCOUNT));
        let written: serde_json::Value =
            serde_json::from_str(&kc.entries.borrow()[&service]).unwrap();
        assert_eq!(written["claudeAiOauth"]["accessToken"], "a-container");
        assert_eq!(written["claudeAiOauth"]["expiresAt"], 300);
        assert_eq!(written["mcpOAuth"]["server"]["token"], "keep");
        assert_eq!(with_login_of("not json", &login(1, "x")), None);
        assert_eq!(
            with_login_of(with_extra, "{}"),
            None,
            "a staged file with no login is never written"
        );
    }
}
