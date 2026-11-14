// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! What `agentmux setup` chose for this channel, kept in `agentmux-server.json` in the channel's folder
//! and read by `run` (so a service, which passes no arguments, starts the server the way the person
//! set it up). A flag on the command line wins over a value kept here; a missing file is the defaults.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

pub const SETTINGS_FILE: &str = "agentmux-server.json";
/// The web port when none was chosen; the websocket port is the next one.
pub const DEFAULT_PORT: u16 = 7380;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, clap::ValueEnum)]
#[serde(rename_all = "lowercase")]
pub enum SecretStore {
    /// Owner-only files under `~/.agentmux/shared/secrets/`, for a machine with no keychain.
    File,
    /// The operating system's keychain, as the desktop app uses.
    Keychain,
}

impl SecretStore {
    pub fn flag(self) -> &'static str {
        match self {
            SecretStore::File => "file",
            SecretStore::Keychain => "keychain",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub port: u16,
    pub secret_store: SecretStore,
    /// The address a browser reaches the server on when it is not the server's own (`ssh -L`):
    /// `http://127.0.0.1:<port>`.
    pub public_origin: Option<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Settings { port: DEFAULT_PORT, secret_store: SecretStore::File, public_origin: None }
    }
}

pub fn settings_path(channel_dir: &Path) -> PathBuf {
    channel_dir.join(SETTINGS_FILE)
}

/// The kept settings: the defaults when there is no file, an error when the file is there and does not read.
pub fn read(channel_dir: &Path) -> Result<Settings, String> {
    let path = settings_path(channel_dir);
    match std::fs::read_to_string(&path) {
        Ok(text) => {
            let settings: Settings = serde_json::from_str(&text).map_err(|e| format!("{} does not read ({e}); fix it or run `agentmux setup`", path.display()))?;
            check_port(settings.port).map_err(|e| format!("{}: {e}; fix it or run `agentmux setup`", path.display()))?;
            Ok(settings)
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Settings::default()),
        Err(e) => Err(format!("cannot read {}: {e}", path.display())),
    }
}

/// Write the settings, by a temporary file and a rename.
pub fn write(channel_dir: &Path, settings: &Settings) -> std::io::Result<()> {
    let path = settings_path(channel_dir);
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_string_pretty(settings).map_err(std::io::Error::other)?)?;
    std::fs::rename(&tmp, &path)
}

/// The port pair a start uses: the websocket port is the next one, so the web port can be at most 65534.
pub fn check_port(port: u16) -> Result<u16, String> {
    if (1024..=65534).contains(&port) { Ok(port) } else { Err(format!("port {port}: choose one from 1024 to 65534")) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_kept_port_that_cannot_be_used_is_refused_where_it_is_read() {
        let dir = tempfile::tempdir().unwrap();
        for bad in [0u16, 80, 65535] {
            std::fs::write(settings_path(dir.path()), format!("{{\"port\": {bad}, \"secret_store\": \"file\"}}")).unwrap();
            let err = read(dir.path()).unwrap_err();
            assert!(err.contains("agentmux setup"), "{err}");
        }
    }

    #[test]
    fn no_file_is_the_defaults_and_a_written_file_is_read_back() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(read(dir.path()).unwrap(), Settings::default());
        assert_eq!(Settings::default().port, 7380);
        let chosen = Settings { port: 7500, secret_store: SecretStore::Keychain, public_origin: Some("http://127.0.0.1:8080".into()) };
        write(dir.path(), &chosen).unwrap();
        assert_eq!(read(dir.path()).unwrap(), chosen);
        assert!(!dir.path().join("agentmux-server.json.tmp").exists());
    }

    #[test]
    fn a_file_with_only_some_choices_keeps_the_defaults_for_the_rest() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(settings_path(dir.path()), r#"{ "port": 7401 }"#).unwrap();
        assert_eq!(read(dir.path()).unwrap(), Settings { port: 7401, ..Settings::default() });
    }

    #[test]
    fn a_file_that_does_not_read_is_an_error_not_the_defaults() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(settings_path(dir.path()), "{ not json").unwrap();
        let e = read(dir.path()).unwrap_err();
        assert!(e.contains("agentmux-server.json") && e.contains("agentmux setup"), "{e}");
        std::fs::write(settings_path(dir.path()), r#"{ "secret_store": "vault" }"#).unwrap();
        assert!(read(dir.path()).is_err(), "an unknown store is refused");
    }

    #[test]
    fn the_port_pair_has_to_fit() {
        assert!(check_port(7380).is_ok() && check_port(1024).is_ok() && check_port(65534).is_ok());
        assert!(check_port(1023).is_err() && check_port(65535).is_err() && check_port(0).is_err());
    }

    #[test]
    fn the_store_flag_is_what_the_server_takes() {
        assert_eq!(SecretStore::File.flag(), "file");
        assert_eq!(SecretStore::Keychain.flag(), "keychain");
    }
}
