// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Headless mode: srv started directly — a container entrypoint, a server, CI —
//! with no launcher or desktop host. docs/specs/SPEC_SRV_HEADLESS_MODE_2026_09_26.md.
//!
//! Normally the launcher prepares srv's world before spawning it: the data-path
//! env (`DataPaths::to_env_vars`), a per-launch auth key, and a stdin pipe and
//! parent process whose end means "shut down". Headless srv does that itself:
//!
//! - resolves and exports the data-path env when it isn't already set;
//! - takes srv's data-dir lock early (the same lock every srv takes), so no two
//!   servers — headless or launcher-started — share a set of databases;
//! - takes its auth key from `AGENTMUX_AUTH_KEY`, `--auth-key-file`, or
//!   generates one and writes it to `<data dir>/srv-auth-key` (0600) — the key
//!   is never printed, only the file's path;
//! - turns the cloud subscriber off unless asked for (it reads the OS keychain,
//!   which a container usually doesn't have);
//! - is not tied to stdin or a parent process (`main` skips those watchers);
//!   SIGINT/SIGTERM still shut it down.
//!
//! It still binds loopback only (`--web-port` / `--ws-port` fix the ports).
//! Reaching it from another machine is a reverse proxy's job, not srv's.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use crate::config::CliArgs;


/// Set once `prepare_env` succeeded: this process is the headless server.
static ACTIVE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// `--web-port` / `--ws-port`, held in-process (never in the env, so a
/// launcher-spawned srv or an agent can't inherit them — ReAgent P1 on #3893).
static PORTS: OnceLock<(Option<u16>, Option<u16>)> = OnceLock::new();

/// Which startup listener a bind address is for.
#[derive(Clone, Copy)]
pub enum Listener {
    Web,
    Ws,
}

/// Is this process running headless (after `prepare_env`)?
pub fn active() -> bool {
    ACTIVE.load(std::sync::atomic::Ordering::SeqCst)
}

/// File name of the generated auth key, beside the databases.
pub const AUTH_KEY_FILE_NAME: &str = "srv-auth-key";

/// Is this the headless server starting (`--headless` or `AGENTMUX_HEADLESS=1`)?
/// Never for a subcommand such as `migrate`: it needs none of the headless
/// setup, and its lock would refuse a `migrate --verify` run beside a running
/// headless server (ReAgent P1 on #3893).
pub fn requested() -> bool {
    let args: Vec<OsString> = std::env::args_os().collect();
    requested_from(&args, std::env::var("AGENTMUX_HEADLESS").ok().as_deref())
}

fn requested_from(args: &[OsString], env: Option<&str>) -> bool {
    let asked = args.iter().any(|a| a == "--headless") || matches!(env, Some("1") | Some("true"));
    // Parse with the same CliArgs `load_config` uses. A subcommand, or
    // --help/--version (which clap reports as an Err), is not the server, so
    // no setup and no lock (Codex P2 on #3893). Any other argv clap rejects is
    // left for `load_config` to report.
    use clap::error::ErrorKind;
    let not_the_server = match <CliArgs as clap::Parser>::try_parse_from(args) {
        Ok(a) => a.command.is_some(),
        Err(e) => matches!(
            e.kind(),
            ErrorKind::DisplayHelp | ErrorKind::DisplayVersion | ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand
        ),
    };
    asked && !not_the_server
}

/// Prepare the environment the launcher would normally provide. Runs before
/// logging is initialized, so problems are returned for `main` to print.
/// Returns the path the auth key was written to, when it generated one.
pub fn prepare_env() -> Result<Option<PathBuf>, String> {
    // The same parser `load_config` uses, over `args_os`, so path flags that
    // aren't valid UTF-8 arrive intact (Codex P2 on #3893).
    let cli = <CliArgs as clap::Parser>::try_parse_from(std::env::args_os()).map_err(|e| e.to_string())?;

    let paths = match agentmux_common::DataPaths::from_env() {
        Some(p) => p,
        None => {
            let mode = agentmux_common::RuntimeMode::from_env().unwrap_or(agentmux_common::RuntimeMode::Installed);
            let p = agentmux_common::DataPaths::resolve(env!("CARGO_PKG_VERSION"), &mode)?;
            p.ensure_dirs()?;
            for (k, v) in p.to_env_vars() {
                std::env::set_var(k, v);
            }
            p
        }
    };
    // Every path is now explicit in the env, as the launcher leaves it. Drop the
    // root override so srv resolves its stores the launcher's way — from the
    // exported data dir — rather than at the override root, which would put
    // every version's databases in one place (Codex P1 on #3893).
    std::env::remove_var("AGENTMUX_HOME_OVERRIDE");

    // Lock the directory the databases actually live in — `--wavedata` when
    // given (Config makes it the data home), else the resolved data dir — with
    // the same lock every srv takes (`base::acquire_data_dir_lock`), and before
    // writing anything there, so a second server can't clobber the first's key
    // file (Codex P1/P2 on #3893).
    let data_dir = effective_data_dir(cli.wavedata.as_deref(), &paths.data_dir);
    std::fs::create_dir_all(&data_dir).map_err(|e| format!("cannot create {}: {e}", data_dir.display()))?;
    crate::backend::base::acquire_data_dir_lock(&data_dir)?;

    let mut generated = None;
    if std::env::var("AGENTMUX_AUTH_KEY").map(|k| k.is_empty()).unwrap_or(true) {
        let key_file = cli.auth_key_file.clone().or_else(|| std::env::var_os("AGENTMUX_AUTH_KEY_FILE").map(PathBuf::from));
        let key = match key_file {
            Some(file) => read_key_file(&file)?,
            None => {
                let (key, path) = write_generated_key(&data_dir)?;
                generated = Some(path);
                key
            }
        };
        std::env::set_var("AGENTMUX_AUTH_KEY", key);
    }

    let _ = PORTS.set((cli.web_port, cli.ws_port));

    if std::env::var_os("AGENTMUX_DISABLE_CLOUD_SUBSCRIBER").is_none() {
        std::env::set_var("AGENTMUX_DISABLE_CLOUD_SUBSCRIBER", "1");
    }

    ACTIVE.store(true, std::sync::atomic::Ordering::SeqCst);
    Ok(generated)
}

/// The directory srv will open its databases in: `--wavedata` if given (the
/// same precedence `Config::from_env_and_args` applies), else `resolved`.
fn effective_data_dir(wavedata: Option<&Path>, resolved: &Path) -> PathBuf {
    wavedata
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(resolved)
        .to_path_buf()
}

/// Read a key from a file the operator provided; surrounding whitespace is ignored.
fn read_key_file(path: &Path) -> Result<String, String> {
    let key = std::fs::read_to_string(path)
        .map_err(|e| format!("--auth-key-file {}: {e}", path.display()))?
        .trim()
        .to_string();
    if key.is_empty() {
        return Err(format!("--auth-key-file {}: empty", path.display()));
    }
    Ok(key)
}

/// Generate a fresh key and write it to `<dir>/srv-auth-key`, readable by the
/// owner only. A new key every start, like a launcher-spawned srv.
fn write_generated_key(dir: &Path) -> Result<(String, PathBuf), String> {
    let key = format!("{}{}", uuid::Uuid::new_v4().simple(), uuid::Uuid::new_v4().simple());
    let path = dir.join(AUTH_KEY_FILE_NAME);
    let _ = std::fs::remove_file(&path);
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    use std::io::Write;
    opts.open(&path)
        .and_then(|mut f| f.write_all(key.as_bytes()))
        .map_err(|e| format!("cannot write the auth key to {}: {e}", path.display()))?;
    Ok((key, path))
}

/// The address a startup listener binds: `STARTUP_BIND_ADDR` (loopback,
/// OS-chosen port) — or, headless only, the same loopback host with the port
/// `--web-port` / `--ws-port` gave. A non-headless srv always gets the default.
pub fn startup_bind_addr(listener: Listener) -> String {
    let fixed = PORTS.get().and_then(|(web, ws)| match listener {
        Listener::Web => *web,
        Listener::Ws => *ws,
    });
    bind_addr_with(fixed)
}

fn bind_addr_with(port: Option<u16>) -> String {
    let default = crate::backend::lan_listeners::STARTUP_BIND_ADDR;
    match port {
        Some(port) => {
            let host = default.rsplit_once(':').map(|(h, _)| h).unwrap_or(default);
            format!("{host}:{port}")
        }
        None => default.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(v: &[&str]) -> Vec<OsString> {
        v.iter().map(OsString::from).collect()
    }

    fn cli(v: &[&str]) -> CliArgs {
        <CliArgs as clap::Parser>::try_parse_from(args(v)).unwrap()
    }

    #[test]
    fn headless_is_requested_by_flag_or_env() {
        assert!(requested_from(&args(&["srv", "--headless"]), None));
        assert!(requested_from(&args(&["srv"]), Some("1")));
        assert!(!requested_from(&args(&["srv", "--wavedata", "x"]), None));
        assert!(!requested_from(&args(&["srv"]), Some("0")));
    }

    /// A subcommand, --help or --version never runs the headless setup (or
    /// takes its lock), even with AGENTMUX_HEADLESS=1 set for the whole env.
    #[test]
    fn a_subcommand_or_help_is_never_headless() {
        assert!(!requested_from(&args(&["srv", "migrate", "--verify"]), Some("1")));
        assert!(!requested_from(&args(&["srv", "--headless", "migrate", "--list"]), None));
        assert!(!requested_from(&args(&["srv", "--headless", "--help"]), None));
        assert!(!requested_from(&args(&["srv", "--help"]), Some("1")));
    }

    /// The lock goes where the databases go: --wavedata wins, as in Config.
    #[test]
    fn the_lock_follows_wavedata() {
        let resolved = Path::new("/home/u/.agentmux/channels/stable/versions/1/data");
        let dir = |v: &[&str]| effective_data_dir(cli(v).wavedata.as_deref(), resolved);
        assert_eq!(dir(&["srv", "--headless"]), resolved);
        assert_eq!(dir(&["srv", "--headless", "--wavedata", "/shared"]), Path::new("/shared"));
        assert_eq!(dir(&["srv", "--wavedata=/shared"]), Path::new("/shared"));
    }

    /// A `--wavedata` that isn't valid UTF-8 is locked byte for byte, the same
    /// directory Config then opens the stores in.
    #[cfg(unix)]
    #[test]
    fn a_non_utf8_wavedata_is_kept() {
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt;
        let raw = OsStr::from_bytes(b"/data/caf\xe9");
        let a = vec![OsString::from("srv"), OsString::from("--headless"), OsString::from("--wavedata"), raw.to_os_string()];
        assert!(requested_from(&a, None));
        let parsed = <CliArgs as clap::Parser>::try_parse_from(a).unwrap();
        assert_eq!(effective_data_dir(parsed.wavedata.as_deref(), Path::new("/x")).as_os_str(), raw);
    }

    #[test]
    fn flags_parse_in_both_forms() {
        let c = cli(&["srv", "--headless", "--web-port", "8190", "--ws-port=8191"]);
        assert_eq!((c.web_port, c.ws_port), (Some(8190), Some(8191)));
        assert_eq!(c.auth_key_file, None);
        assert!(<CliArgs as clap::Parser>::try_parse_from(args(&["srv", "--web-port", "x"])).is_err());
    }

    #[test]
    fn generated_key_is_written_owner_only_and_fresh_each_time() {
        let dir = tempfile::tempdir().unwrap();
        let (k1, p1) = write_generated_key(dir.path()).unwrap();
        assert_eq!(std::fs::read_to_string(&p1).unwrap(), k1);
        assert_eq!(k1.len(), 64);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&p1).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }
        let (k2, _) = write_generated_key(dir.path()).unwrap();
        assert_ne!(k1, k2);
        assert_eq!(std::fs::read_to_string(&p1).unwrap(), k2);
    }

    #[test]
    fn key_file_is_trimmed_and_must_not_be_empty() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("k");
        std::fs::write(&p, "  secret-key\n").unwrap();
        assert_eq!(read_key_file(&p).unwrap(), "secret-key");
        std::fs::write(&p, "\n").unwrap();
        assert!(read_key_file(&p).is_err());
        assert!(read_key_file(&dir.path().join("missing")).is_err());
    }

    #[test]
    fn bind_addr_is_the_startup_address_or_its_host_with_a_fixed_port() {
        let default = crate::backend::lan_listeners::STARTUP_BIND_ADDR;
        assert_eq!(bind_addr_with(None), default);
        assert_eq!(bind_addr_with(Some(8190)), "127.0.0.1:8190");
    }

    /// Only `prepare_env` sets the fixed ports, and only in-process: a srv
    /// that isn't headless binds the default whatever its env says.
    #[test]
    fn a_non_headless_srv_ignores_port_env() {
        std::env::set_var("AGENTMUX_SRV_WEB_PORT", "8190");
        assert_eq!(
            startup_bind_addr(Listener::Web),
            crate::backend::lan_listeners::STARTUP_BIND_ADDR
        );
        std::env::remove_var("AGENTMUX_SRV_WEB_PORT");
    }

    #[cfg(unix)]
    #[test]
    fn the_data_dir_lock_is_reentrant_here_and_exclusive_otherwise() {
        use crate::backend::base::{acquire_data_dir_lock, MuxLock, SRV_DATA_LOCK_FILE};
        let dir = tempfile::tempdir().unwrap();
        acquire_data_dir_lock(dir.path()).unwrap();
        // Headless takes it early, bootstrap again later: fine in one process.
        acquire_data_dir_lock(dir.path()).unwrap();
        // Anyone else is refused while it's held.
        assert!(MuxLock::acquire_at(&dir.path().join(SRV_DATA_LOCK_FILE)).is_err());
    }
}
