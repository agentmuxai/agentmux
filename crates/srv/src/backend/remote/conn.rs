// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A pane's connection, parsed from its `connection` block meta.
//!
//! - `local` (or empty): this machine.
//! - `wsl://<distro>`: a WSL distribution on this Windows machine.
//! - anything else: an SSH destination as the user writes it for `ssh`:
//!   `host`, `user@host`, `user@host:port`, or a `Host` alias from
//!   `~/.ssh/config`. The system `ssh` resolves it (spec §3.1).
//!
//! The SSH destination ends up in an `ssh` argv, so it is validated here: one
//! starting with `-` would be read by `ssh` as an option (`-oProxyCommand=...`
//! runs a local command), and whitespace or control characters have no place in
//! a destination. The launch also puts `--` before it; this is the first line.

/// Where a pane's shell runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnTarget {
    Local,
    /// A WSL distribution, by name.
    Wsl(String),
    /// An SSH destination, passed to the system `ssh` as written.
    Ssh(SshDest),
}

/// A validated SSH destination: `host`, `user@host` or `user@host:port`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SshDest {
    /// The destination as `ssh` takes it, without the port: `user@host` or `host`.
    pub destination: String,
    /// From a trailing `:port`, when given; otherwise ssh config decides.
    pub port: Option<u16>,
}

/// Prefix of a WSL connection name.
pub const WSL_PREFIX: &str = "wsl://";

/// Characters that only mean something to a shell, refused in an SSH
/// destination (see `parse_ssh`).
const SHELL_CHARS: &[char] = &[
    '`', '$', ';', '|', '&', '<', '>', '(', ')', '\'', '"', '\\', '*', '?', '!', '{', '}', '#', '~',
];

/// Longest connection name accepted: a hostname is at most 253 characters, plus
/// a user and a port.
const MAX_CONN_NAME_LEN: usize = 300;

impl ConnTarget {
    /// Parse a block's `connection` meta. `Err` names what is wrong, in words a
    /// pane can show.
    pub fn parse(name: &str) -> Result<ConnTarget, String> {
        let name = name.trim();
        if name.is_empty() || name == "local" {
            return Ok(ConnTarget::Local);
        }
        if name.len() > MAX_CONN_NAME_LEN {
            return Err("connection name is too long".to_string());
        }
        if name.chars().any(|c| c.is_whitespace() || c.is_control()) {
            return Err(
                "a connection name cannot contain spaces or control characters".to_string(),
            );
        }
        if let Some(distro) = name.strip_prefix(WSL_PREFIX) {
            return parse_wsl(distro).map(ConnTarget::Wsl);
        }
        parse_ssh(name).map(ConnTarget::Ssh)
    }

    /// The canonical connection name, as stored in block meta and used as the
    /// status key.
    pub fn name(&self) -> String {
        match self {
            ConnTarget::Local => "local".to_string(),
            ConnTarget::Wsl(d) => format!("{WSL_PREFIX}{d}"),
            ConnTarget::Ssh(s) => match s.port {
                Some(p) => format!("{}:{p}", s.destination),
                None => s.destination.clone(),
            },
        }
    }

    pub fn is_local(&self) -> bool {
        matches!(self, ConnTarget::Local)
    }
}

fn parse_wsl(distro: &str) -> Result<String, String> {
    // WSL distribution names are letters, digits, '.', '_' and '-' (wsl.exe
    // --install --name rules); anything else cannot be one.
    if distro.is_empty() {
        return Err("a WSL connection needs a distribution name, e.g. wsl://Ubuntu".to_string());
    }
    if !distro
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
    {
        return Err(format!("'{distro}' is not a WSL distribution name"));
    }
    if distro.starts_with('-') {
        return Err(format!("'{distro}' is not a WSL distribution name"));
    }
    Ok(distro.to_string())
}

fn parse_ssh(name: &str) -> Result<SshDest, String> {
    if name.starts_with('-') {
        return Err("a connection name cannot start with '-'".to_string());
    }
    // A trailing :port, unless the host part is a bracketed IPv6 literal without
    // a port, or a bare IPv6 address (several colons).
    let (destination, port) = split_port(name)?;
    let host = destination
        .rsplit_once('@')
        .map(|(_, h)| h)
        .unwrap_or(destination);
    if host.is_empty() {
        return Err("a connection needs a host".to_string());
    }
    if let Some((user, _)) = destination.rsplit_once('@') {
        if user.is_empty() {
            return Err("the user before '@' is empty".to_string());
        }
    }
    // Not a DNS-name whitelist: an ssh config `Host` alias may legitimately
    // contain '/' or '+' ("prod/web"), and OpenSSH resolves it (Codex P2 on
    // #4248). What is refused is shell syntax. The destination is passed as one
    // argv entry after `--`, so it never meets a local shell directly, but a
    // user's `ProxyCommand` expands it through `%h` into a shell command, and no
    // real host name or alias needs these characters.
    if destination.chars().any(|c| SHELL_CHARS.contains(&c)) {
        return Err(format!(
            "'{name}' is not a host name, user@host, or ssh config alias"
        ));
    }
    Ok(SshDest {
        destination: destination.to_string(),
        port,
    })
}

fn split_port(name: &str) -> Result<(&str, Option<u16>), String> {
    // [v6] with no port, or [v6]:port.
    if name.ends_with(']') {
        return Ok((name, None));
    }
    if let Some(idx) = name.rfind("]:") {
        let port = parse_port(&name[idx + 2..])?;
        return Ok((&name[..idx + 1], Some(port)));
    }
    // Bare IPv6 (two or more colons, no brackets): no port can be given.
    if name.matches(':').count() >= 2 {
        return Ok((name, None));
    }
    match name.rsplit_once(':') {
        Some((dest, port)) => Ok((dest, Some(parse_port(port)?))),
        None => Ok((name, None)),
    }
}

fn parse_port(s: &str) -> Result<u16, String> {
    match s.parse::<u16>() {
        Ok(p) if p > 0 => Ok(p),
        _ => Err(format!("'{s}' is not a port number")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ssh(dest: &str, port: Option<u16>) -> ConnTarget {
        ConnTarget::Ssh(SshDest {
            destination: dest.to_string(),
            port,
        })
    }

    #[test]
    fn local_is_the_default() {
        for n in ["", "local", "  local  "] {
            assert_eq!(ConnTarget::parse(n), Ok(ConnTarget::Local), "{n:?}");
        }
    }

    #[test]
    fn wsl_names_parse() {
        assert_eq!(
            ConnTarget::parse("wsl://Ubuntu"),
            Ok(ConnTarget::Wsl("Ubuntu".into()))
        );
        assert_eq!(
            ConnTarget::parse("wsl://Ubuntu-24.04"),
            Ok(ConnTarget::Wsl("Ubuntu-24.04".into()))
        );
        assert!(ConnTarget::parse("wsl://").is_err());
        assert!(ConnTarget::parse("wsl://a/b").is_err());
        assert!(ConnTarget::parse("wsl://-d").is_err());
    }

    #[test]
    fn ssh_destinations_parse_as_written() {
        assert_eq!(ConnTarget::parse("area54"), Ok(ssh("area54", None)));
        assert_eq!(
            ConnTarget::parse("asaf@area54"),
            Ok(ssh("asaf@area54", None))
        );
        assert_eq!(
            ConnTarget::parse("asaf@area54:2222"),
            Ok(ssh("asaf@area54", Some(2222)))
        );
        assert_eq!(
            ConnTarget::parse("deploy@10.0.0.5"),
            Ok(ssh("deploy@10.0.0.5", None))
        );
        assert_eq!(
            ConnTarget::parse("[fe80::1]:22"),
            Ok(ssh("[fe80::1]", Some(22)))
        );
        assert_eq!(ConnTarget::parse("fe80::1"), Ok(ssh("fe80::1", None)));
        assert_eq!(
            ConnTarget::parse("user@fe80::1%eth0"),
            Ok(ssh("user@fe80::1%eth0", None))
        );
    }

    /// Codex P2 on #4248: an ssh config alias need not look like a DNS name.
    #[test]
    fn ssh_config_aliases_with_slashes_or_plus_are_accepted() {
        assert_eq!(ConnTarget::parse("prod/web"), Ok(ssh("prod/web", None)));
        assert_eq!(
            ConnTarget::parse("deploy@db+replica"),
            Ok(ssh("deploy@db+replica", None))
        );
    }

    /// The destination goes into an `ssh` argv: nothing that ssh could read as an
    /// option, and no shell syntax (a `ProxyCommand` expands it through `%h`).
    #[test]
    fn unsafe_or_malformed_names_are_refused() {
        for bad in [
            "-oProxyCommand=calc",
            "-p 22 host",
            "host;rm -rf ~",
            "host name",
            "host\nname",
            "user@",
            "@host",
            "host:0",
            "host:99999",
            "host:abc",
            "$(whoami)@host",
            "host`id`",
        ] {
            assert!(ConnTarget::parse(bad).is_err(), "{bad:?} should be refused");
        }
        assert!(ConnTarget::parse(&"a".repeat(MAX_CONN_NAME_LEN + 1)).is_err());
    }

    #[test]
    fn names_round_trip() {
        for n in [
            "local",
            "wsl://Ubuntu",
            "area54",
            "asaf@area54:2222",
            "[fe80::1]:22",
        ] {
            assert_eq!(ConnTarget::parse(n).unwrap().name(), n);
        }
        assert_eq!(ConnTarget::parse("").unwrap().name(), "local");
    }
}
