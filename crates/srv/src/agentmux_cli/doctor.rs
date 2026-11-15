// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `agentmux doctor`: what is wrong on this machine for the server, and what to do about it. The
//! judging is a function of facts ([`evaluate`]); gathering them ([`gather`]) is the only part that
//! looks at the machine, so each rule has a test that needs no machine.

use std::path::{Path, PathBuf};

use agentmux_common::win32::NoWindow;
use serde::Serialize;

use crate::state::ServerState;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Level {
    Ok,
    Warn,
    Fail,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Check {
    pub name: &'static str,
    pub level: Level,
    pub detail: String,
    /// What to do, when there is something to do.
    pub fix: Option<String>,
}

fn check(name: &'static str, level: Level, detail: impl Into<String>, fix: Option<&str>) -> Check {
    Check { name, level, detail: detail.into(), fix: fix.map(str::to_string) }
}

/// Everything `evaluate` judges.
#[derive(Debug, Clone)]
pub struct Facts {
    pub os: &'static str,
    pub arch: &'static str,
    pub srv: Result<PathBuf, String>,
    pub web: Result<PathBuf, String>,
    pub folders: Result<(), String>,
    pub free_bytes: Option<u64>,
    /// The key file's permission bits, when it exists (Unix).
    pub key_mode: Option<u32>,
    pub port: u16,
    pub running: Option<ServerState>,
    pub answering: bool,
    /// Whether the port and the one after it can be bound now; `None` when it was not asked.
    pub port_free: Option<bool>,
    pub tools: Vec<(&'static str, bool)>,
    /// The shared libraries the agent tools link that are not on this machine (Linux; empty elsewhere).
    pub missing_libs: Vec<&'static str>,
    /// Whether the system's certificates are there for the server's https to trust (Linux; `None` elsewhere, where
    /// the system always has a store).
    pub certs_found: Option<bool>,
    pub installed_version: &'static str,
    /// What `node --version` says, when node is on the PATH and answers.
    pub node_version: Option<String>,
    /// The note a start left when another AgentMux's version held the shared folder.
    pub blocked: Option<crate::state::Blocked>,
    /// The service: whether its file exists, and whether it names this command's program.
    pub service_installed: bool,
    pub service_names_exe: Option<bool>,
}

const MIB: u64 = 1024 * 1024;

pub fn evaluate(f: &Facts) -> Vec<Check> {
    let mut out = Vec::new();
    let supported = matches!(f.os, "linux" | "macos") && matches!(f.arch, "x86_64" | "aarch64");
    out.push(if supported {
        check("platform", Level::Ok, format!("{} {}", f.os, f.arch), None)
    } else {
        check("platform", Level::Fail, format!("{} {} is not in the first package (Linux and macOS, x86_64 and arm64)", f.os, f.arch), Some("use a Linux or macOS machine, or the container image"))
    });
    for (what, found, flag) in [("server program", &f.srv, "--srv-bin"), ("web UI", &f.web, "--web-dir")] {
        out.push(match found {
            Ok(path) => check("install", Level::Ok, format!("{what}: {}", path.display()), None),
            Err(e) => check("install", Level::Fail, e.clone(), Some(&format!("reinstall AgentMux, or name it with {flag}"))),
        });
    }
    out.push(match &f.folders {
        Ok(()) => check("folders", Level::Ok, "the channel's folders are writable", None),
        Err(e) => check("folders", Level::Fail, e.clone(), Some("make the folder writable by this user (its owner and mode), then run this again")),
    });
    out.push(match f.free_bytes {
        None => check("disk", Level::Warn, "cannot read the free space", None),
        Some(b) if b < 200 * MIB => check("disk", Level::Fail, format!("{} MiB free", b / MIB), Some("free space: agent work and the server's databases are written here")),
        Some(b) if b < 1024 * MIB => check("disk", Level::Warn, format!("{} MiB free", b / MIB), Some("free some space soon")),
        Some(b) => check("disk", Level::Ok, format!("{:.1} GiB free", b as f64 / (1024.0 * MIB as f64)), None),
    });
    out.push(match f.key_mode {
        None => check("key file", Level::Ok, "none yet (the server makes it at its first start, owner-only)", None),
        Some(mode) if mode & 0o077 != 0 => check("key file", Level::Fail, format!("the key file is readable by others (mode {:o})", mode & 0o777), Some("chmod 600 the file named in `agentmux status`, then `agentmux restart`")),
        Some(_) => check("key file", Level::Ok, "owner-only", None),
    });
    out.push(match (&f.running, f.answering, f.port_free) {
        (Some(s), true, _) => check("port", Level::Ok, format!("the server answers on port {}", s.web_port), None),
        (Some(s), false, _) => check("port", Level::Warn, format!("the server (pid {}) is running but does not answer on port {} yet", s.pid, s.web_port), Some("wait a moment, then `agentmux logs`")),
        (None, _, Some(true)) => check("port", Level::Ok, format!("ports {} and {} are free", f.port, f.port + 1), None),
        (None, _, Some(false)) => check("port", Level::Fail, format!("port {} or {} is in use by another program", f.port, f.port + 1), Some("choose another with `agentmux setup --port <port>`")),
        (None, _, None) => check("port", Level::Warn, "not checked", None),
    });
    for (tool, found) in &f.tools {
        out.push(if *found {
            check("tools", Level::Ok, format!("{tool} found"), None)
        } else {
            check("tools", Level::Warn, format!("{tool} is not on the PATH"), Some("agent command-line tools are installed through npm: install Node.js and git with your package manager"))
        });
    }
    out.extend(node_version_check(f.node_version.as_deref()));
    match f.certs_found {
        Some(true) => out.push(check("certificates", Level::Ok, "the system's certificates are found", None)),
        Some(false) => out.push(check(
            "certificates",
            Level::Fail,
            agentmux_common::system_certs::NO_SYSTEM_CERTS,
            Some(agentmux_common::system_certs::NO_SYSTEM_CERTS_FIX),
        )),
        None => {}
    }
    if !f.missing_libs.is_empty() {
        out.push(check(
            "libraries",
            Level::Warn,
            format!("not found: {} (the agent tools link them)", f.missing_libs.join(", ")),
            Some("install them with your package manager (Debian and Ubuntu: `apt install libdbus-1-3 libxcb1`; Fedora: `dnf install dbus-libs libxcb`)"),
        ));
    }
    if let Some(b) = f.blocked.as_ref().filter(|_| f.running.is_none()) {
        // A warning, not a failure: this is the note of an earlier start, which may have cleared since (the other AgentMux
        // closed). `setup` stops on a failure before it starts the server, and the start is what finds out and clears it.
        out.push(check(
            "shared folder",
            Level::Warn,
            format!("the last start was refused: {} is using it, and this version's data needs preparing first", b.owner),
            Some(&format!("close {} and run `agentmux start`, or update one of the two so both are the same version", b.owner)),
        ));
    }
    if let Some(s) = &f.running {
        if s.version != f.installed_version {
            out.push(check("version", Level::Warn, format!("running {}, installed {}", s.version, f.installed_version), Some("`agentmux restart` runs the installed version")));
        }
    }
    out.push(match (f.service_installed, f.service_names_exe) {
        (false, _) => check("service", Level::Ok, "none: the server runs only when you start it", Some("`agentmux setup --service` starts it at every login")),
        (true, Some(false)) => check("service", Level::Warn, "the service runs a different install of this command", Some("`agentmux setup --service` points it at this one")),
        (true, _) => check("service", Level::Ok, "installed, and it runs this install", None),
    });
    out
}

/// The least Node.js any agent of the new-agent panel asks for: the lowest `minVersion` in
/// `frontend/app/view/agent/providers/catalog.ts` (a test keeps the two the same). Some agents ask for more.
pub const NODE_MIN: &str = "20.0.0";

/// `v22.1.0` or `22.1.0` as numbers; `None` for anything else.
fn parse_version(text: &str) -> Option<(u64, u64, u64)> {
    let mut parts = text.trim().trim_start_matches('v').split('.');
    let mut next = || parts.next()?.split(['-', '+']).next()?.parse::<u64>().ok();
    Some((next()?, next()?, next()?))
}

/// What the installed Node.js comes to against the one the panel asks for; nothing when it is not known.
fn node_version_check(found: Option<&str>) -> Option<Check> {
    let found = found?.trim();
    let have = parse_version(found)?;
    let need = parse_version(NODE_MIN)?;
    let shown = found.trim_start_matches('v');
    Some(if have >= need {
        check("node version", Level::Ok, format!("Node.js {shown}; the new-agent panel needs at least {NODE_MIN}, and some agents need a newer one"), None)
    } else {
        check(
            "node version",
            Level::Warn,
            format!("Node.js {shown} is older than the {NODE_MIN} that is the least any agent in the new-agent panel needs"),
            Some(&format!("install Node.js {NODE_MIN} or newer (nodejs.org, or your package manager's NodeSource repository), then run this again")),
        )
    })
}

/// `node --version`, when node is on the PATH and answers.
pub fn node_version() -> Option<String> {
    if !on_path("node") {
        return None;
    }
    let out = std::process::Command::new("node").arg("--version").no_window().stdin(std::process::Stdio::null()).output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).trim().to_string()).filter(|v| !v.is_empty())
}

/// Is any check a failure?
pub fn failed(checks: &[Check]) -> bool {
    checks.iter().any(|c| c.level == Level::Fail)
}

/// The checks as lines for a terminal.
pub fn lines(checks: &[Check]) -> Vec<String> {
    checks
        .iter()
        .map(|c| {
            let tag = match c.level {
                Level::Ok => "ok  ",
                Level::Warn => "warn",
                Level::Fail => "FAIL",
            };
            match &c.fix {
                Some(fix) if c.level != Level::Ok => format!("{tag} {}: {}\n       -> {fix}", c.name, c.detail),
                _ => format!("{tag} {}: {}", c.name, c.detail),
            }
        })
        .collect()
}

/// Where the system keeps its certificates on Linux: bundle files and folders of certificate files (the places the
/// platform verifier reads).
const CERT_FILES: &[&str] = &[
    "/etc/ssl/certs/ca-certificates.crt",
    "/etc/pki/tls/certs/ca-bundle.crt",
    "/etc/ssl/ca-bundle.pem",
    "/etc/pki/tls/cacert.pem",
    "/etc/ssl/cert.pem",
    "/usr/local/share/certs/ca-root-nss.crt",
    "/etc/pki/ca-trust/extracted/pem/tls-ca-bundle.pem",
    "/etc/certs/ca-certificates.crt",
];
const CERT_DIRS: &[&str] = &["/etc/ssl/certs", "/etc/pki/tls/certs", "/usr/local/share/certs", "/etc/pki/ca-trust/extracted/pem/directory-hash", "/var/ssl/certs"];

/// Are there system certificates on this Linux machine? `None` elsewhere.
pub fn system_certs_found() -> Option<bool> {
    cfg!(target_os = "linux").then(|| certs_present(std::env::var_os("SSL_CERT_FILE"), std::env::var_os("SSL_CERT_DIR"), CERT_FILES, CERT_DIRS))
}

/// The environment's bundle or folder counts first, then the usual places; a folder counts when it holds something.
fn certs_present(env_file: Option<std::ffi::OsString>, env_dir: Option<std::ffi::OsString>, files: &[&str], dirs: &[&str]) -> bool {
    let is_file = |p: &Path| std::fs::metadata(p).is_ok_and(|m| m.is_file() && m.len() > 0);
    let has_entries = |p: &Path| std::fs::read_dir(p).is_ok_and(|mut d| d.next().is_some());
    env_file.is_some_and(|f| is_file(Path::new(&f)))
        || env_dir.is_some_and(|d| std::env::split_paths(&d).any(|p| has_entries(&p)))
        || files.iter().any(|f| is_file(Path::new(f)))
        || dirs.iter().any(|d| has_entries(Path::new(d)))
}

/// The shared libraries `agentmux-mcp` links that cannot be found in the usual library folders (Linux only).
pub fn missing_libs() -> Vec<&'static str> {
    if !cfg!(target_os = "linux") {
        return Vec::new();
    }
    let mut dirs: Vec<PathBuf> = ["/lib", "/lib64", "/usr/lib", "/usr/lib64", "/usr/local/lib"].iter().map(PathBuf::from).collect();
    for base in ["/lib", "/usr/lib"] {
        if let Ok(entries) = std::fs::read_dir(base) {
            dirs.extend(entries.flatten().map(|e| e.path()).filter(|p| p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.ends_with("-linux-gnu"))));
        }
    }
    ["libdbus-1.so.3", "libxcb.so.1"].into_iter().filter(|lib| !dirs.iter().any(|d| d.join(lib).exists())).collect()
}

/// Is `name` an executable file in a folder of the PATH?
pub fn on_path(name: &str) -> bool {
    let Some(path) = std::env::var_os("PATH") else { return false };
    let suffixes: &[&str] = if cfg!(windows) { &[".exe", ".cmd", ".bat", ""] } else { &[""] };
    std::env::split_paths(&path).any(|dir| suffixes.iter().any(|s| dir.join(format!("{name}{s}")).is_file()))
}

/// Can the port and the one after it be bound on the loopback address now?
pub fn ports_free(port: u16) -> bool {
    let Some(next) = port.checked_add(1) else { return false };
    [port, next].iter().all(|p| std::net::TcpListener::bind(("127.0.0.1", *p)).is_ok())
}

/// Can this folder be written? A file is made and removed.
pub fn writable(dir: &Path) -> Result<(), String> {
    let probe = dir.join(".agentmux-write-test");
    std::fs::write(&probe, b"").map_err(|e| format!("cannot write in {}: {e}", dir.display()))?;
    let _ = std::fs::remove_file(&probe);
    Ok(())
}

/// The permission bits of a file (Unix); `None` when there is no such file or no bits to read.
pub fn mode_of(path: &Path) -> Option<u32> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(path).ok().map(|m| m.permissions().mode())
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn good() -> Facts {
        Facts {
            os: "linux",
            arch: "x86_64",
            srv: Ok("/i/bin/agentmux-srv".into()),
            web: Ok("/i/web".into()),
            folders: Ok(()),
            free_bytes: Some(50 * 1024 * MIB),
            key_mode: Some(0o100600),
            port: 7380,
            running: None,
            answering: false,
            port_free: Some(true),
            tools: vec![("node", true), ("npm", true), ("git", true)],
            missing_libs: vec![],
            certs_found: Some(true),
            installed_version: "0.60.26",
            node_version: None,
            blocked: None,
            service_installed: false,
            service_names_exe: None,
        }
    }

    fn level_of(checks: &[Check], name: &str) -> Vec<Level> {
        checks.iter().filter(|c| c.name == name).map(|c| c.level).collect()
    }

    fn running(version: &str) -> ServerState {
        ServerState { pid: 9, version: version.into(), channel: "headless".into(), web_port: 7380, ws_port: 7381, started_unix: 0, key_file: "/k".into(), data_dir: "/d".into(), srv: "/s".into() }
    }

    #[test]
    fn a_good_machine_has_nothing_to_fix() {
        let checks = evaluate(&good());
        assert!(!failed(&checks));
        assert!(checks.iter().all(|c| c.level == Level::Ok), "{checks:?}");
    }

    #[test]
    fn only_linux_and_macos_on_x86_64_or_arm64_are_supported() {
        for (os, arch, ok) in [("linux", "x86_64", true), ("linux", "aarch64", true), ("macos", "aarch64", true), ("macos", "x86_64", true), ("windows", "x86_64", false), ("linux", "riscv64", false), ("freebsd", "x86_64", false)] {
            let facts = Facts { os, arch, ..good() };
            assert_eq!(level_of(&evaluate(&facts), "platform"), vec![if ok { Level::Ok } else { Level::Fail }], "{os} {arch}");
        }
    }

    #[test]
    fn a_missing_program_or_web_ui_is_a_failure_with_a_way_out() {
        let facts = Facts { srv: Err("the server program is not at /i/bin".into()), web: Err("no web".into()), ..good() };
        let checks = evaluate(&facts);
        assert!(failed(&checks));
        let fixes: Vec<_> = checks.iter().filter(|c| c.level == Level::Fail).filter_map(|c| c.fix.clone()).collect();
        assert!(fixes.iter().any(|f| f.contains("--srv-bin")) && fixes.iter().any(|f| f.contains("--web-dir")), "{fixes:?}");
    }

    #[test]
    fn disk_warns_under_a_gibibyte_and_fails_under_two_hundred_mebibytes() {
        for (free, want) in [(Some(5 * 1024 * MIB), Level::Ok), (Some(1024 * MIB), Level::Ok), (Some(1023 * MIB), Level::Warn), (Some(200 * MIB), Level::Warn), (Some(199 * MIB), Level::Fail), (None, Level::Warn)] {
            let facts = Facts { free_bytes: free, ..good() };
            assert_eq!(level_of(&evaluate(&facts), "disk"), vec![want], "{free:?}");
        }
    }

    #[test]
    fn a_key_file_others_can_read_is_a_failure() {
        for (mode, want) in [(None, Level::Ok), (Some(0o100600), Level::Ok), (Some(0o100400), Level::Ok), (Some(0o100640), Level::Fail), (Some(0o100604), Level::Fail), (Some(0o100666), Level::Fail)] {
            let facts = Facts { key_mode: mode, ..good() };
            assert_eq!(level_of(&evaluate(&facts), "key file"), vec![want], "{mode:?}");
        }
    }

    #[test]
    fn the_port_is_judged_by_whether_the_server_runs() {
        let busy = Facts { port_free: Some(false), ..good() };
        assert_eq!(level_of(&evaluate(&busy), "port"), vec![Level::Fail]);
        let ours = Facts { running: Some(running("0.60.26")), answering: true, port_free: Some(false), ..good() };
        assert_eq!(level_of(&evaluate(&ours), "port"), vec![Level::Ok], "its own port in use is not a problem");
        let stuck = Facts { running: Some(running("0.60.26")), answering: false, ..good() };
        assert_eq!(level_of(&evaluate(&stuck), "port"), vec![Level::Warn]);
    }

    #[test]
    fn a_machine_with_no_certificates_fails_and_names_the_package() {
        let mut f = good();
        f.certs_found = Some(false);
        let checks = evaluate(&f);
        assert_eq!(level_of(&checks, "certificates"), vec![Level::Fail]);
        assert!(failed(&checks));
        assert!(lines(&checks).iter().any(|l| l.contains("ca-certificates") && l.contains("SSL_CERT_FILE")));
        f.certs_found = None;
        assert!(level_of(&evaluate(&f), "certificates").is_empty(), "macOS and Windows have their own store: nothing is said");
    }

    #[test]
    fn certificates_are_found_by_the_environment_a_bundle_or_a_folder_with_something_in_it() {
        let dir = tempfile::tempdir().unwrap();
        let bundle = dir.path().join("bundle.pem");
        let empty_bundle = dir.path().join("empty.pem");
        let folder = dir.path().join("certs");
        let empty_folder = dir.path().join("none");
        std::fs::write(&bundle, b"-----BEGIN CERTIFICATE-----").unwrap();
        std::fs::write(&empty_bundle, b"").unwrap();
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join("a.pem"), b"x").unwrap();
        std::fs::create_dir_all(&empty_folder).unwrap();
        let (b, e, d, n) = (bundle.to_str().unwrap(), empty_bundle.to_str().unwrap(), folder.to_str().unwrap(), empty_folder.to_str().unwrap());
        assert!(!certs_present(None, None, &[e, "/nonexistent/x"], &[n, "/nonexistent"]), "an empty bundle and an empty folder are no store");
        assert!(certs_present(None, None, &[b], &[]));
        assert!(certs_present(None, None, &[], &[d]));
        assert!(certs_present(Some(b.into()), None, &[], &[]), "SSL_CERT_FILE");
        assert!(certs_present(None, Some(d.into()), &[], &[]), "SSL_CERT_DIR");
        assert!(!certs_present(Some("/nonexistent".into()), None, &[], &[]));
    }

    #[test]
    fn a_missing_library_is_a_warning_that_names_the_packages() {
        let mut f = good();
        f.missing_libs = vec!["libdbus-1.so.3"];
        let checks = evaluate(&f);
        assert_eq!(level_of(&checks, "libraries"), vec![Level::Warn]);
        assert!(!failed(&checks));
        assert!(lines(&checks).iter().any(|l| l.contains("libdbus-1.so.3") && l.contains("libdbus-1-3")));
        assert!(level_of(&evaluate(&good()), "libraries").is_empty(), "nothing missing, nothing said");
    }

    #[test]
    fn a_missing_tool_is_a_warning_not_a_failure() {
        let facts = Facts { tools: vec![("node", false), ("npm", false), ("git", true)], ..good() };
        let checks = evaluate(&facts);
        assert!(!failed(&checks));
        assert_eq!(level_of(&checks, "tools"), vec![Level::Warn, Level::Warn, Level::Ok]);
    }

    #[test]
    fn a_server_older_than_the_install_is_said_so() {
        let facts = Facts { running: Some(running("0.60.20")), answering: true, ..good() };
        assert_eq!(level_of(&evaluate(&facts), "version"), vec![Level::Warn]);
        let same = Facts { running: Some(running("0.60.26")), answering: true, ..good() };
        assert!(level_of(&evaluate(&same), "version").is_empty());
    }

    #[test]
    fn a_start_refused_beside_another_version_is_a_warning_that_says_what_to_do_and_never_stops_setup() {
        let blocked = crate::state::Blocked { owner: "AgentMux 0.60.27 (stable)".into(), reason: "29 migration(s) pending".into(), version: "0.60.28".into(), since_unix: 1 };
        let facts = Facts { blocked: Some(blocked.clone()), ..good() };
        let checks = evaluate(&facts);
        assert_eq!(level_of(&checks, "shared folder"), vec![Level::Warn]);
        // `setup` refuses to go on after a failure; the note is the last start's, and the next start clears or renews it.
        assert!(!failed(&checks), "a remembered refusal must not stop setup from retrying");
        let text = lines(&checks).join("\n");
        assert!(text.contains("the last start was refused: AgentMux 0.60.27 (stable) is using it") && text.contains("-> close AgentMux 0.60.27 (stable) and run `agentmux start`"), "{text}");
        // A server that is running again has no refusal to report.
        let up = Facts { blocked: Some(blocked), running: Some(running("0.60.26")), answering: true, ..good() };
        assert!(level_of(&evaluate(&up), "shared folder").is_empty());
    }

    #[test]
    fn a_node_older_than_the_panel_needs_is_a_warning_with_the_same_number() {
        let old = Facts { node_version: Some("v18.20.4".into()), ..good() };
        let checks = evaluate(&old);
        assert_eq!(level_of(&checks, "node version"), vec![Level::Warn]);
        let text = lines(&checks).join("\n");
        assert!(text.contains("Node.js 18.20.4 is older than the 20.0.0 that is the least any agent in the new-agent panel needs") && text.contains("install Node.js 20.0.0 or newer"), "{text}");
        for fine in ["v20.0.0", "v22.1.0", "v24.9.0", "23.0.0-nightly"] {
            let ok = Facts { node_version: Some(fine.into()), ..good() };
            assert_eq!(level_of(&evaluate(&ok), "node version"), vec![Level::Ok], "{fine}");
        }
        let none = Facts { node_version: Some("not a version".into()), ..good() };
        assert!(level_of(&evaluate(&none), "node version").is_empty(), "an answer that is no version says nothing");
        assert!(level_of(&evaluate(&good()), "node version").is_empty(), "no node, no version line");
    }

    #[test]
    fn the_minimum_node_is_the_one_the_agent_panel_names() {
        let catalog = std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../frontend/app/view/agent/providers/catalog.ts")).unwrap();
        let mut lowest: Option<(u64, u64, u64)> = None;
        for part in catalog.split("minVersion: \"").skip(1) {
            let version = part.split('"').next().unwrap_or("");
            if let Some(v) = parse_version(version) {
                lowest = Some(lowest.map_or(v, |l| l.min(v)));
            }
        }
        assert_eq!(lowest, parse_version(NODE_MIN), "the lowest Node.js catalog.ts asks for is no longer {NODE_MIN}: change NODE_MIN in doctor.rs with it");
    }

    #[test]
    fn a_service_of_another_install_is_a_warning() {
        let facts = Facts { service_installed: true, service_names_exe: Some(false), ..good() };
        assert_eq!(level_of(&evaluate(&facts), "service"), vec![Level::Warn]);
        let fine = Facts { service_installed: true, service_names_exe: Some(true), ..good() };
        assert_eq!(level_of(&evaluate(&fine), "service"), vec![Level::Ok]);
    }

    #[test]
    fn the_lines_show_a_way_out_only_where_there_is_something_to_fix() {
        let facts = Facts { free_bytes: Some(10 * MIB), ..good() };
        let text = lines(&evaluate(&facts)).join("\n");
        assert!(text.contains("FAIL disk: 10 MiB free\n       -> free space"), "{text}");
        assert!(text.contains("ok   platform: linux x86_64\n"), "{text}");
        assert!(!text.contains("ok   service: none: the server runs only when you start it\n       ->"), "an ok line carries no arrow: {text}");
    }

    #[test]
    fn the_machine_probes_answer_for_a_real_folder_and_a_taken_port() {
        let dir = tempfile::tempdir().unwrap();
        assert!(writable(dir.path()).is_ok());
        assert!(writable(&dir.path().join("missing")).is_err());
        assert!(!dir.path().join(".agentmux-write-test").exists());
        let held = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = held.local_addr().unwrap().port();
        assert!(!ports_free(port), "a port in use");
        drop(held);
        assert!(!on_path("agentmux-no-such-program-xyz"));
    }
}
