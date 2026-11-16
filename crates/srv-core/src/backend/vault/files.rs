// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Files that hold a Vault value for one command: `Terminal` `run` with
//! `secret_files` (SPEC_VAULT_GENERATE_CAPTURE_AND_BROWSER_SIGNINS_2026_10_28.md §4.3),
//! for tools that read a key only from a file (`--key-file`,
//! `GOOGLE_APPLICATION_CREDENTIALS`).
//!
//! Each command gets a fresh folder of its own under `<data home>/vault-files/`,
//! readable by the user alone (0700 and 0600 on Unix; on Windows the folder is
//! inside the user's profile, which only the user, SYSTEM and Administrators can
//! read). [`SecretDir`] overwrites and removes it when it is dropped, that is when
//! the command's runner ends. A folder left by a crash is removed when srv next
//! starts ([`init`]).

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

static ROOT: OnceLock<PathBuf> = OnceLock::new();

const FOLDER: &str = "vault-files";

/// Sets the folder's place and removes whatever a crashed srv left there.
pub fn init(data_home: &Path) {
    let root = data_home.join(FOLDER);
    if root.exists() {
        if let Err(e) = remove(&root) {
            tracing::warn!(error = %e, "vault: couldn't remove the secret files a previous run left");
        }
    }
    let _ = ROOT.set(root);
}

fn root() -> PathBuf {
    ROOT.get().cloned().unwrap_or_else(|| std::env::temp_dir().join(format!("agentmux-{FOLDER}")))
}

/// Overwrites every file under `dir` with zeros, then removes the folder.
fn remove(dir: &Path) -> std::io::Result<()> {
    if let Ok(entries) = std::fs::read_dir(dir) {
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                let _ = remove(&p);
            } else if let Ok(meta) = e.metadata() {
                if let Ok(mut f) = std::fs::OpenOptions::new().write(true).open(&p) {
                    let _ = f.write_all(&vec![0u8; meta.len() as usize]);
                    let _ = f.sync_all();
                }
            }
        }
    }
    std::fs::remove_dir_all(dir)
}

fn private_dir(path: &Path) -> std::io::Result<()> {
    let mut b = std::fs::DirBuilder::new();
    b.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        b.mode(0o700);
    }
    b.create(path)
}

/// One command's folder. Dropping it removes its files.
#[derive(Debug)]
pub struct SecretDir {
    path: PathBuf,
}

impl SecretDir {
    pub fn create() -> Result<Self, String> {
        let root = root();
        private_dir(&root).map_err(|e| format!("couldn't make the folder for secret files: {e}"))?;
        let path = root.join(uuid::Uuid::new_v4().to_string());
        private_dir(&path).map_err(|e| format!("couldn't make the folder for secret files: {e}"))?;
        Ok(SecretDir { path })
    }

    /// Writes `value` to a new file named `name`, readable by the user alone, and
    /// returns its path.
    pub fn write(&self, name: &str, value: &str) -> Result<PathBuf, String> {
        let file = self.path.join(name);
        let mut o = std::fs::OpenOptions::new();
        o.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            o.mode(0o600);
        }
        let mut f = o.open(&file).map_err(|e| format!("couldn't write the secret file for {name}: {e}"))?;
        f.write_all(value.as_bytes()).and_then(|_| f.sync_all()).map_err(|e| format!("couldn't write the secret file for {name}: {e}"))?;
        Ok(file)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for SecretDir {
    fn drop(&mut self) {
        if let Err(e) = remove(&self.path) {
            tracing::warn!(error = %e, "vault: couldn't remove a command's secret files; they go when srv next starts");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_folder_holds_its_files_until_dropped() {
        let dir = SecretDir::create().unwrap();
        let file = dir.write("SERVICE_KEY", "not-a-real-key-123").unwrap();
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "not-a-real-key-123");
        assert!(dir.write("SERVICE_KEY", "again").is_err(), "a name is written once");
        let folder = dir.path().to_path_buf();
        drop(dir);
        assert!(!folder.exists(), "the folder goes with the command");
    }

    #[cfg(unix)]
    #[test]
    fn files_and_folders_are_the_users_alone() {
        use std::os::unix::fs::PermissionsExt;
        let dir = SecretDir::create().unwrap();
        let file = dir.write("K", "value-value").unwrap();
        assert_eq!(std::fs::metadata(dir.path()).unwrap().permissions().mode() & 0o777, 0o700);
        assert_eq!(std::fs::metadata(&file).unwrap().permissions().mode() & 0o777, 0o600);
    }

    #[test]
    fn init_removes_what_a_crash_left() {
        let home = std::env::temp_dir().join(format!("agentmux-files-test-{}", uuid::Uuid::new_v4()));
        let left = home.join(FOLDER).join("old");
        std::fs::create_dir_all(&left).unwrap();
        std::fs::write(left.join("K"), "left over").unwrap();
        init(&home);
        assert!(!home.join(FOLDER).exists());
        let _ = std::fs::remove_dir_all(&home);
    }
}
