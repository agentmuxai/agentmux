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
//! - takes its auth key from `AGENTMUX_AUTH_KEY`, `--auth-key-file`, or
//!   generates one and writes it to `<instance runtime>/srv-auth-key` (0600) —
//!   the key is never printed, only the file's path;
//! - holds a per-channel lock so two headless servers can't share a data dir;
//! - turns the cloud subscriber off unless asked for (it reads the OS keychain,
//!   which a container usually doesn't have);
//! - is not tied to stdin or a parent process (`main` skips those watchers);
//!   SIGINT/SIGTERM still shut it down.
//!
//! It still binds loopback only (`--web-port` / `--ws-port` fix the ports).
//! Reaching it from another machine is a reverse proxy's job, not srv's.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use crate::backend::base::MuxLock;

/// Held for the process lifetime once acquired.
static LOCK: OnceLock<MuxLock> = OnceLock::new();

/// File name of the generated auth key under the instance runtime dir.
pub const AUTH_KEY_FILE_NAME: &str = "srv-auth-key";
/// Per-channel lock file under the instance runtime dir.
const LOCK_FILE_NAME: &str = "srv-headless.lock";

/// Was srv started headless (`--headless` or `AGENTMUX_HEADLESS=1`)?
pub fn requested() -> bool {
    requested_from(std::env::args(), std::env::var("AGENTMUX_HEADLESS").ok().as_deref())
}

fn requested_from(mut args: impl Iterator<Item = String>, env: Option<&str>) -> bool {
    args.any(|a| a == "--headless") || matches!(env, Some("1") | Some("true"))
}

/// The value after `flag` in argv (`--flag value` or `--flag=value`).
fn arg_value(args: &[String], flag: &str) -> Option<String> {
    let eq = format!("{flag}=");
    args.iter().enumerate().find_map(|(i, a)| {
        if a == flag {
            args.get(i + 1).cloned()
        } else {
            a.strip_prefix(&eq).map(str::to_string)
        }
    })
}

/// Prepare the environment the launcher would normally provide. Runs before
/// logging is initialized, so problems are returned for `main` to print.
/// Returns the path the auth key was written to, when it generated one.
pub fn prepare_env() -> Result<Option<PathBuf>, String> {
    let args: Vec<String> = std::env::args().collect();

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
    let runtime_dir = paths.instance_runtime_dir.clone();
    std::fs::create_dir_all(&runtime_dir)
        .map_err(|e| format!("cannot create {}: {e}", runtime_dir.display()))?;

    let lock = MuxLock::acquire_at(&runtime_dir.join(LOCK_FILE_NAME)).map_err(|_| {
        format!(
            "another headless agentmux-srv is already using {} (lock {})",
            paths.instance_dir.display(),
            runtime_dir.join(LOCK_FILE_NAME).display()
        )
    })?;
    let _ = LOCK.set(lock);

    let mut generated = None;
    if std::env::var("AGENTMUX_AUTH_KEY").map(|k| k.is_empty()).unwrap_or(true) {
        let key = match arg_value(&args, "--auth-key-file").or_else(|| std::env::var("AGENTMUX_AUTH_KEY_FILE").ok()) {
            Some(file) => read_key_file(Path::new(&file))?,
            None => {
                let (key, path) = write_generated_key(&runtime_dir)?;
                generated = Some(path);
                key
            }
        };
        std::env::set_var("AGENTMUX_AUTH_KEY", key);
    }

    for (flag, var) in [("--web-port", "AGENTMUX_SRV_WEB_PORT"), ("--ws-port", "AGENTMUX_SRV_WS_PORT")] {
        if let Some(port) = arg_value(&args, flag) {
            port.parse::<u16>().map_err(|_| format!("{flag}: not a port: {port:?}"))?;
            std::env::set_var(var, port);
        }
    }

    if std::env::var_os("AGENTMUX_DISABLE_CLOUD_SUBSCRIBER").is_none() {
        std::env::set_var("AGENTMUX_DISABLE_CLOUD_SUBSCRIBER", "1");
    }

    Ok(generated)
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
/// OS-chosen port), or the same loopback host with the fixed port in `var`.
pub fn loopback_bind_addr(var: &str) -> String {
    let default = crate::backend::lan_listeners::STARTUP_BIND_ADDR;
    match std::env::var(var).ok().and_then(|p| p.parse::<u16>().ok()) {
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

    fn args(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn headless_is_requested_by_flag_or_env() {
        assert!(requested_from(args(&["srv", "--headless"]).into_iter(), None));
        assert!(requested_from(args(&["srv"]).into_iter(), Some("1")));
        assert!(!requested_from(args(&["srv", "--wavedata", "x"]).into_iter(), None));
        assert!(!requested_from(args(&["srv"]).into_iter(), Some("0")));
    }

    #[test]
    fn arg_value_reads_both_forms() {
        let a = args(&["srv", "--headless", "--web-port", "8190", "--ws-port=8191"]);
        assert_eq!(arg_value(&a, "--web-port").as_deref(), Some("8190"));
        assert_eq!(arg_value(&a, "--ws-port").as_deref(), Some("8191"));
        assert_eq!(arg_value(&a, "--auth-key-file"), None);
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
    fn loopback_bind_addr_defaults_to_the_startup_address() {
        assert_eq!(
            loopback_bind_addr("AGENTMUX_TEST_UNSET_PORT_VAR"),
            crate::backend::lan_listeners::STARTUP_BIND_ADDR
        );
    }

    #[cfg(unix)]
    #[test]
    fn the_lock_admits_one_holder() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join(LOCK_FILE_NAME);
        let first = MuxLock::acquire_at(&p).unwrap();
        assert!(MuxLock::acquire_at(&p).is_err());
        drop(first);
        assert!(MuxLock::acquire_at(&p).is_ok());
    }
}
