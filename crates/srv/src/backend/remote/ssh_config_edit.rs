// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Adding a host to the user's `~/.ssh/config` from Remotes, and finding a
//! host's `Host` line there (SPEC_REMOTES_PANE_2026_10_05.md §4.4).
//!
//! The file stays the one place hosts are defined, so every tool sees a host
//! added here. AgentMux only ever appends a new `Host` block at the end (an
//! `Include` and earlier `Host *` defaults still apply to it as before) and
//! never rewrites an existing entry. It refuses an alias the file already
//! names, keeps a copy of the file before its first write, and creates the
//! file owner-only when it is missing.
//!
//! Every value is checked before it is written: one word, no control
//! characters, no quotes, so a field can't end its line and start a directive
//! of its own (a `ProxyCommand`, say).

use std::path::{Path, PathBuf};

/// The copy kept of the config before AgentMux's first write.
pub const BACKUP_NAME: &str = "config.agentmux-backup";

/// What the Add remote form gives.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NewHost {
    pub alias: String,
    pub hostname: String,
    pub user: String,
    pub port: String,
    pub identity_file: String,
    pub proxy_jump: String,
}

/// One word for an ssh config line: no whitespace, quotes, `#` at the start,
/// or control characters.
fn word(field: &str, value: &str) -> Result<String, String> {
    let v = value.trim();
    if v.chars().any(|c| c.is_whitespace() || c.is_control() || c == '"' || c == '\'' || c == '\\') {
        return Err(format!("{field} can't contain spaces, quotes or backslashes"));
    }
    if v.starts_with('#') || v.starts_with('-') {
        return Err(format!("{field} can't start with {:?}", &v[..1]));
    }
    Ok(v.to_string())
}

/// The `Host` block for `host`, checked: an alias (required, a plain name)
/// and the optional settings, each on its own line.
pub fn host_block(host: &NewHost) -> Result<String, String> {
    let alias = word("The alias", &host.alias)?;
    if alias.is_empty() {
        return Err("a remote needs an alias".into());
    }
    if !super::ssh_config::is_plain_name(&alias) {
        return Err(format!("{alias:?} can't be an alias: use a plain name, without * ? or !"));
    }
    let mut block = format!("Host {alias}\n");
    fn line(block: &mut String, keyword: &str, field: &str, value: &str) -> Result<(), String> {
        let v = word(field, value)?;
        if !v.is_empty() {
            block.push_str(&format!("    {keyword} {v}\n"));
        }
        Ok(())
    }
    line(&mut block, "HostName", "The host name", &host.hostname)?;
    line(&mut block, "User", "The user", &host.user)?;
    let port = host.port.trim();
    if !port.is_empty() && port.parse::<u16>().map_or(true, |p| p == 0) {
        return Err(format!("{port:?} is not a port (1 to 65535)"));
    }
    line(&mut block, "Port", "The port", port)?;
    // A path with spaces is quoted; anything else that could break the line
    // is refused.
    let identity = host.identity_file.trim();
    if identity.chars().any(|c| c.is_control() || c == '"') {
        return Err("the identity file's path can't contain quotes".into());
    }
    // In a quoted value a trailing `\` would escape the closing quote and
    // leave the whole config unparseable; a key file never ends in one.
    if identity.ends_with('\\') {
        return Err("the identity file's path can't end in a backslash".into());
    }
    if !identity.is_empty() {
        if identity.contains(char::is_whitespace) {
            block.push_str(&format!("    IdentityFile \"{identity}\"\n"));
        } else {
            block.push_str(&format!("    IdentityFile {identity}\n"));
        }
    }
    line(&mut block, "ProxyJump", "The jump host", &host.proxy_jump)?;
    Ok(block)
}

/// Every name any `Host` line in the config (and what it includes) gives,
/// patterns included.
fn all_host_words(path: &Path, ssh_dir: &Path, depth: usize, out: &mut Vec<String>) {
    if depth > super::ssh_config::MAX_INCLUDE_DEPTH {
        return;
    }
    let Ok(bytes) = std::fs::read(path) else {
        return;
    };
    for line in String::from_utf8_lossy(&bytes).lines() {
        let Some((keyword, args)) = super::ssh_config::split_line(line) else {
            continue;
        };
        if keyword.eq_ignore_ascii_case("host") {
            out.extend(args);
        } else if keyword.eq_ignore_ascii_case("include") {
            for pattern in args {
                for file in super::ssh_config::expand_include(&pattern, ssh_dir) {
                    all_host_words(&file, ssh_dir, depth + 1, out);
                }
            }
        }
    }
}

/// Append `host` to the config at `config` (Includes relative to `ssh_dir`).
/// Returns the block written.
pub fn add_host_in(config: &Path, ssh_dir: &Path, host: &NewHost) -> Result<String, String> {
    let block = host_block(host)?;
    let alias = host.alias.trim();
    let mut names = Vec::new();
    all_host_words(config, ssh_dir, 0, &mut names);
    if names.iter().any(|n| n == alias) {
        return Err(format!("your ssh config already has a host {alias:?}"));
    }
    std::fs::create_dir_all(ssh_dir).map_err(|e| format!("could not create {}: {e}", ssh_dir.display()))?;
    // As bytes, so anything in the file survives exactly. Only a missing file
    // counts as no config: a file that can't be read is left alone.
    let existing = match std::fs::read(config) {
        Ok(bytes) => Some(bytes),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(format!("could not read {} ({e}); nothing was changed", config.display())),
    };
    let backup = ssh_dir.join(BACKUP_NAME);
    if let Some(bytes) = &existing {
        if !backup.exists() {
            std::fs::write(&backup, bytes).map_err(|e| format!("could not keep a copy of your ssh config: {e}"))?;
            owner_only(&backup);
        }
    }
    // Appended, never rewritten: what is there stays byte for byte.
    let mut add = String::new();
    if let Some(bytes) = existing.as_deref().filter(|b| !b.is_empty()) {
        if !bytes.ends_with(b"\n") {
            add.push('\n');
        }
        add.push('\n');
    }
    add.push_str("# Added by AgentMux (Remotes)\n");
    add.push_str(&block);
    use std::io::Write;
    std::fs::OpenOptions::new()
        .append(true)
        .create_new(existing.is_none())
        .open(config)
        .and_then(|mut f| f.write_all(add.as_bytes()))
        .map_err(|e| format!("could not write {}: {e}", config.display()))?;
    if existing.is_none() {
        owner_only(config);
    }
    Ok(block)
}

/// Owner read and write only, as ssh wants its config (Unix; on Windows a new
/// file under the user's profile is already the user's).
fn owner_only(_path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(_path, std::fs::Permissions::from_mode(0o600));
    }
}

/// The file and 1-based line of the `Host` line naming `alias`, following
/// Includes.
pub fn locate_in(config: &Path, ssh_dir: &Path, alias: &str) -> Option<(PathBuf, usize)> {
    fn find(path: &Path, ssh_dir: &Path, alias: &str, depth: usize) -> Option<(PathBuf, usize)> {
        if depth > super::ssh_config::MAX_INCLUDE_DEPTH {
            return None;
        }
        let bytes = std::fs::read(path).ok()?;
        for (i, line) in String::from_utf8_lossy(&bytes).lines().enumerate() {
            let Some((keyword, args)) = super::ssh_config::split_line(line) else {
                continue;
            };
            if keyword.eq_ignore_ascii_case("host") && args.iter().any(|a| a == alias) {
                return Some((path.to_path_buf(), i + 1));
            }
            if keyword.eq_ignore_ascii_case("include") {
                for pattern in args {
                    for file in super::ssh_config::expand_include(&pattern, ssh_dir) {
                        if let Some(found) = find(&file, ssh_dir, alias, depth + 1) {
                            return Some(found);
                        }
                    }
                }
            }
        }
        None
    }
    find(config, ssh_dir, alias.trim(), 0)
}

/// The user's ssh dir and config.
pub fn user_paths() -> Option<(PathBuf, PathBuf)> {
    let ssh_dir = dirs::home_dir()?.join(".ssh");
    Some((ssh_dir.join("config"), ssh_dir))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn host(alias: &str) -> NewHost {
        NewHost { alias: alias.into(), hostname: "10.0.0.5".into(), user: "asaf".into(), ..Default::default() }
    }

    #[test]
    fn a_block_has_only_the_given_settings() {
        let full = NewHost {
            alias: "db1".into(),
            hostname: "db1.internal".into(),
            user: "admin".into(),
            port: "2222".into(),
            identity_file: "~/.ssh/my key".into(),
            proxy_jump: "bastion".into(),
        };
        assert_eq!(
            host_block(&full).unwrap(),
            "Host db1\n    HostName db1.internal\n    User admin\n    Port 2222\n    IdentityFile \"~/.ssh/my key\"\n    ProxyJump bastion\n"
        );
        assert_eq!(host_block(&NewHost { alias: "box".into(), ..Default::default() }).unwrap(), "Host box\n");
    }

    #[test]
    fn a_field_cant_start_a_directive_of_its_own() {
        for bad in [
            NewHost { hostname: "x\n    ProxyCommand calc".into(), ..host("a") },
            NewHost { user: "a b".into(), ..host("a") },
            NewHost { proxy_jump: "-oProxyCommand=calc".into(), ..host("a") },
            NewHost { identity_file: "k\"\nProxyCommand x".into(), ..host("a") },
            NewHost { identity_file: r"C:\Users\John Doe\.ssh\".into(), ..host("a") },
            NewHost { port: "0".into(), ..host("a") },
            NewHost { port: "ssh".into(), ..host("a") },
            host(""),
            host("*"),
            host("web*"),
        ] {
            assert!(host_block(&bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn adding_appends_keeps_a_copy_and_refuses_a_name_the_file_has() {
        let dir = tempfile::tempdir().unwrap();
        let config = dir.path().join("config");
        std::fs::write(&config, "Host *\n    ServerAliveInterval 30\nInclude extra\n").unwrap();
        std::fs::write(dir.path().join("extra"), "Host nas\n").unwrap();
        add_host_in(&config, dir.path(), &host("db1")).unwrap();
        let text = std::fs::read_to_string(&config).unwrap();
        assert!(text.starts_with("Host *\n"), "the old content is untouched: {text}");
        assert!(text.ends_with("# Added by AgentMux (Remotes)\nHost db1\n    HostName 10.0.0.5\n    User asaf\n"), "{text}");
        let backup = std::fs::read_to_string(dir.path().join(BACKUP_NAME)).unwrap();
        assert_eq!(backup, "Host *\n    ServerAliveInterval 30\nInclude extra\n");
        // Again: refused, whether the name is in the file or an Include.
        assert!(add_host_in(&config, dir.path(), &host("db1")).is_err());
        assert!(add_host_in(&config, dir.path(), &host("nas")).is_err());
        // A second add keeps the first copy.
        add_host_in(&config, dir.path(), &host("web")).unwrap();
        assert_eq!(std::fs::read_to_string(dir.path().join(BACKUP_NAME)).unwrap(), backup);
        assert_eq!(locate_in(&config, dir.path(), "web").map(|(_, l)| l), Some(text.lines().count() + 3));
        assert_eq!(locate_in(&config, dir.path(), "nas"), Some((dir.path().join("extra"), 1)));
        assert_eq!(locate_in(&config, dir.path(), "nope"), None);
    }

    /// Bytes that aren't UTF-8 (a Latin-1 comment) survive exactly, and the
    /// copy has them too.
    #[test]
    fn a_config_that_isnt_utf8_is_kept_byte_for_byte() {
        let dir = tempfile::tempdir().unwrap();
        let config = dir.path().join("config");
        let original: &[u8] = b"# caf\xe9\nHost nas\n";
        std::fs::write(&config, original).unwrap();
        assert!(add_host_in(&config, dir.path(), &host("nas")).is_err(), "nas is still seen");
        add_host_in(&config, dir.path(), &host("box")).unwrap();
        let after = std::fs::read(&config).unwrap();
        assert!(after.starts_with(original), "{:?}", String::from_utf8_lossy(&after));
        assert!(after.ends_with(b"Host box\n    HostName 10.0.0.5\n    User asaf\n"));
        assert_eq!(std::fs::read(dir.path().join(BACKUP_NAME)).unwrap(), original);
    }

    /// A config that exists but can't be read is left alone.
    #[test]
    fn an_unreadable_config_is_left_alone() {
        let dir = tempfile::tempdir().unwrap();
        let config = dir.path().join("config");
        std::fs::create_dir(&config).unwrap();
        let err = add_host_in(&config, dir.path(), &host("box")).unwrap_err();
        assert!(err.contains("nothing was changed"), "{err}");
        assert!(config.is_dir());
        assert!(!dir.path().join(BACKUP_NAME).exists());
    }

    #[test]
    fn a_missing_config_is_created() {
        let dir = tempfile::tempdir().unwrap();
        let ssh_dir = dir.path().join(".ssh");
        let config = ssh_dir.join("config");
        add_host_in(&config, &ssh_dir, &host("box")).unwrap();
        assert_eq!(
            std::fs::read_to_string(&config).unwrap(),
            "# Added by AgentMux (Remotes)\nHost box\n    HostName 10.0.0.5\n    User asaf\n"
        );
        assert!(!ssh_dir.join(BACKUP_NAME).exists(), "nothing to keep a copy of");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&config).unwrap().permissions().mode() & 0o777, 0o600);
        }
    }
}
