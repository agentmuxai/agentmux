// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Which container runtime this srv talks to (P11 of the container-parity work).
//!
//! `DOCKER_HOST` decides when it is set: a `unix://` socket, `tcp://` or `http://` (with TLS when
//! `DOCKER_TLS_VERIFY` is; without it only to this computer, because the daemon is handed the
//! account's credentials), `https://`, or Windows' `npipe://`. Anything else (`ssh://`) is named
//! as unsupported, and nothing else is tried: the person chose a runtime.
//!
//! When it is not set, the sockets a runtime leaves in the usual places are tried in order and
//! the first that answers a ping is used:
//!
//! - Linux: rootless Docker (`$XDG_RUNTIME_DIR/docker.sock`), rootless Podman
//!   (`$XDG_RUNTIME_DIR/podman/podman.sock`), Docker Desktop (`~/.docker/desktop/docker.sock`, and
//!   `~/.docker/run/docker.sock`), the system Docker (`/var/run/docker.sock`), the system Podman
//!   (`/run/podman/podman.sock`).
//! - macOS: Docker Desktop (`~/.docker/run/docker.sock`), OrbStack (`~/.orbstack/run/docker.sock`),
//!   Colima (`~/.colima/default/docker.sock`), Rancher Desktop (`~/.rd/docker.sock`), then the
//!   system path.
//! - Windows: the `docker_engine` named pipe, as before.
//!
//! A socket under the user's own directories must belong to the user: a planted socket there
//! would see every container request, environment variables included. The system paths are
//! root's by convention and are not checked.
//!
//! The answer is either the connection and a label for it ("Podman at unix:///…"), or a
//! [`Missing`] that says what was looked at and why each place failed.

use std::path::{Path, PathBuf};
use std::time::Duration;

use bollard::Docker;

use crate::backend::container::ContainerManager;

/// How long a socket gets to answer the ping.
const PING_TIMEOUT: Duration = Duration::from_secs(2);
/// bollard's request timeout, as `connect_with_local_defaults` sets it.
const API_TIMEOUT_SECS: u64 = 120;

/// What a socket belongs to, from where it was found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Docker,
    RootlessDocker,
    DockerDesktop,
    Podman,
    RootlessPodman,
    Colima,
    OrbStack,
    RancherDesktop,
    /// Named by `DOCKER_HOST`.
    Configured,
}

impl Kind {
    pub fn name(self) -> &'static str {
        match self {
            Kind::Docker => "Docker",
            Kind::RootlessDocker => "rootless Docker",
            Kind::DockerDesktop => "Docker Desktop",
            Kind::Podman => "Podman",
            Kind::RootlessPodman => "rootless Podman",
            Kind::Colima => "Colima",
            Kind::OrbStack => "OrbStack",
            Kind::RancherDesktop => "Rancher Desktop",
            Kind::Configured => "the runtime in DOCKER_HOST",
        }
    }
}

/// Where a runtime listens.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Endpoint {
    Unix(PathBuf),
    /// `host:port`, with TLS when `tls`.
    Tcp { host: String, tls: bool },
    /// A Windows named pipe, as `bollard` takes it.
    Pipe(String),
}

impl std::fmt::Display for Endpoint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Endpoint::Unix(p) => write!(f, "unix://{}", p.display()),
            Endpoint::Tcp { host, tls: true } => write!(f, "tcp://{host} (TLS)"),
            Endpoint::Tcp { host, tls: false } => write!(f, "tcp://{host}"),
            Endpoint::Pipe(p) => write!(f, "npipe://{p}"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Os {
    Linux,
    MacOs,
    Windows,
}

/// What detection reads from the machine, so tests can hand it a folder of fake sockets.
#[derive(Debug, Clone)]
pub struct Env {
    /// `DOCKER_HOST`; empty counts as unset.
    pub docker_host: Option<String>,
    /// `DOCKER_TLS_VERIFY` is set to something.
    pub tls_verify: bool,
    pub xdg_runtime_dir: Option<PathBuf>,
    pub home: Option<PathBuf>,
    /// Where `key.pem`, `cert.pem` and `ca.pem` are for a TLS connection: `DOCKER_CERT_PATH`,
    /// else `$DOCKER_CONFIG`, else `~/.docker`.
    pub cert_dir: Option<PathBuf>,
    pub os: Os,
    /// The effective user, for the ownership check on sockets under their own directories.
    pub uid: Option<u32>,
    /// `/`; the system sockets (`/var/run/docker.sock`, `/run/podman/podman.sock`) hang off it.
    pub root: PathBuf,
    pub ping_timeout: Duration,
}

impl Env {
    pub fn from_process() -> Self {
        let var = |k: &str| std::env::var_os(k).filter(|v| !v.is_empty());
        Self {
            docker_host: var("DOCKER_HOST").map(|v| v.to_string_lossy().trim().to_string()).filter(|v| !v.is_empty()),
            tls_verify: var("DOCKER_TLS_VERIFY").is_some(),
            xdg_runtime_dir: var("XDG_RUNTIME_DIR").map(PathBuf::from).filter(|p| p.is_absolute()),
            home: dirs::home_dir(),
            cert_dir: var("DOCKER_CERT_PATH")
                .or_else(|| var("DOCKER_CONFIG"))
                .map(PathBuf::from)
                .or_else(|| dirs::home_dir().map(|h| h.join(".docker"))),
            os: if cfg!(windows) {
                Os::Windows
            } else if cfg!(target_os = "macos") {
                Os::MacOs
            } else {
                Os::Linux
            },
            #[cfg(unix)]
            uid: Some(unsafe { libc::geteuid() }),
            #[cfg(not(unix))]
            uid: None,
            root: PathBuf::from("/"),
            ping_timeout: PING_TIMEOUT,
        }
    }
}

/// One place to try.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    pub kind: Kind,
    pub endpoint: Endpoint,
    /// Under the user's own directories, so the socket must be theirs.
    pub user_owned: bool,
}

/// Why `DOCKER_HOST` cannot be used, or the places to try. `Err` is the sentence for the person.
pub fn candidates(env: &Env) -> Result<Vec<Candidate>, String> {
    if let Some(host) = &env.docker_host {
        return configured(host, env.tls_verify).map(|c| vec![c]);
    }
    let unix = |kind, p: PathBuf, user_owned| Candidate { kind, endpoint: Endpoint::Unix(p), user_owned };
    let system_docker = unix(Kind::Docker, env.root.join("var/run/docker.sock"), false);
    let mut out = Vec::new();
    match env.os {
        Os::Windows => out.push(Candidate {
            kind: Kind::Docker,
            endpoint: Endpoint::Pipe("//./pipe/docker_engine".to_string()),
            user_owned: false,
        }),
        Os::Linux => {
            if let Some(xdg) = &env.xdg_runtime_dir {
                out.push(unix(Kind::RootlessDocker, xdg.join("docker.sock"), true));
                out.push(unix(Kind::RootlessPodman, xdg.join("podman/podman.sock"), true));
            }
            if let Some(home) = &env.home {
                // Docker Desktop for Linux keeps its socket in `desktop/`; `run/` is the macOS place.
                out.push(unix(Kind::DockerDesktop, home.join(".docker/desktop/docker.sock"), true));
                out.push(unix(Kind::DockerDesktop, home.join(".docker/run/docker.sock"), true));
            }
            out.push(system_docker);
            out.push(unix(Kind::Podman, env.root.join("run/podman/podman.sock"), false));
        }
        Os::MacOs => {
            if let Some(home) = &env.home {
                out.push(unix(Kind::DockerDesktop, home.join(".docker/run/docker.sock"), true));
                out.push(unix(Kind::OrbStack, home.join(".orbstack/run/docker.sock"), true));
                out.push(unix(Kind::Colima, home.join(".colima/default/docker.sock"), true));
                out.push(unix(Kind::RancherDesktop, home.join(".rd/docker.sock"), true));
            }
            out.push(system_docker);
        }
    }
    Ok(out)
}

/// `DOCKER_HOST` as a place to try.
fn configured(host: &str, tls_verify: bool) -> Result<Candidate, String> {
    let (scheme, rest) = host.split_once("://").ok_or_else(|| {
        format!("DOCKER_HOST is \"{host}\", which is not an address (expected unix://, tcp://, https:// or npipe://)")
    })?;
    let scheme = scheme.to_ascii_lowercase();
    if matches!(scheme.as_str(), "tcp" | "http" | "https") && host_of(rest.trim_end_matches('/')).is_none() {
        return Err(format!(
            "DOCKER_HOST is {host}, which is not a plain host and port (no user name, path or query), as in tcp://localhost:2375"
        ));
    }
    let endpoint = match scheme.as_str() {
        "unix" => Endpoint::Unix(PathBuf::from(rest)),
        "tcp" | "http" => {
            let host = rest.trim_end_matches('/').to_string();
            // The daemon is handed the account credential mount and the scoped srv token, so a
            // daemon across the network must be reached over TLS.
            if !tls_verify && !is_loopback(&host) {
                return Err(format!(
                    "DOCKER_HOST is {scheme}://{host}, an unencrypted connection to another computer, \
                     and a container agent's daemon is given the account's credentials. \
                     Use https:// or set DOCKER_TLS_VERIFY and DOCKER_CERT_PATH, or point DOCKER_HOST at localhost"
                ));
            }
            Endpoint::Tcp { host, tls: tls_verify }
        }
        "https" => Endpoint::Tcp { host: rest.trim_end_matches('/').to_string(), tls: true },
        "npipe" => Endpoint::Pipe(rest.to_string()),
        other => {
            return Err(format!(
                "DOCKER_HOST uses {other}://, which AgentMux can't connect through. \
                 Use a unix:// socket, tcp:// (with DOCKER_TLS_VERIFY and DOCKER_CERT_PATH for TLS) or https://"
            ))
        }
    };
    Ok(Candidate { kind: Kind::Configured, endpoint, user_owned: false })
}

/// The host of a plain `host`, `host:port`, `[v6]` or `[v6]:port`; `None` for anything else.
/// Nothing like user information (`a@b`), a path, a query or a space is accepted: the transport
/// would read such an address differently from a prefix check, so one that is not plain is refused.
fn host_of(host_port: &str) -> Option<&str> {
    if host_port.is_empty() || host_port.contains(|c: char| c.is_whitespace() || matches!(c, '@' | '/' | '?' | '#' | '\\')) {
        return None;
    }
    let port_ok = |p: &str| !p.is_empty() && p.len() <= 5 && p.bytes().all(|b| b.is_ascii_digit());
    if let Some(rest) = host_port.strip_prefix('[') {
        let (host, after) = rest.split_once(']')?;
        let ok = after.is_empty() || after.strip_prefix(':').is_some_and(port_ok);
        return (ok && !host.is_empty()).then_some(host);
    }
    match host_port.matches(':').count() {
        0 => Some(host_port),
        1 => {
            let (host, port) = host_port.split_once(':')?;
            (!host.is_empty() && port_ok(port)).then_some(host)
        }
        // A bare IPv6 address, which has no port.
        _ => Some(host_port),
    }
}

/// Whether `host_port` (`localhost`, `127.0.0.1:2375`, `[::1]:2375`) is this computer.
fn is_loopback(host_port: &str) -> bool {
    let Some(host) = host_of(host_port) else { return false };
    host.eq_ignore_ascii_case("localhost")
        || host.parse::<std::net::IpAddr>().is_ok_and(|ip| ip.to_canonical().is_loopback())
}

/// A place that did not work, and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tried {
    pub kind: Kind,
    pub endpoint: Endpoint,
    pub why: String,
}

/// Nothing answered.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Missing {
    pub tried: Vec<Tried>,
    /// `DOCKER_HOST` could not be used at all.
    pub unusable: Option<String>,
}

impl Missing {
    /// What to tell the person: where it looked, what was wrong with each, and what to do.
    pub fn message(&self) -> String {
        if let Some(why) = &self.unusable {
            return format!("Container agents need Docker or Podman: {why}.");
        }
        let broken: Vec<String> = self
            .tried
            .iter()
            .filter(|t| !t.why.starts_with(ABSENT))
            .map(|t| format!("{} at {} ({})", t.kind.name(), t.endpoint, t.why))
            .collect();
        let absent: Vec<String> =
            self.tried.iter().filter(|t| t.why.starts_with(ABSENT)).map(|t| t.endpoint.to_string()).collect();
        let mut out = String::from("Container agents need Docker or Podman, and none answered.");
        if !broken.is_empty() {
            out.push_str(&format!(" Found but not working: {}.", broken.join("; ")));
        }
        if !absent.is_empty() {
            out.push_str(&format!(" Nothing at: {}.", absent.join(", ")));
        }
        let any = |needle: &str| self.tried.iter().any(|t| t.why.contains(needle));
        if self.tried.iter().any(|t| t.kind == Kind::Configured) {
            // The person chose it, so nothing else was tried.
            out.push_str(" DOCKER_HOST is set, so no other place was tried; unset it to look for Docker or Podman in the usual places.");
            return out;
        }
        if any("permission denied") {
            out.push_str(" Add your user to the docker group, or use rootless Docker or Podman.");
        } else if self.tried.iter().any(|t| matches!(t.kind, Kind::RootlessPodman) && t.why.starts_with(ABSENT)) {
            out.push_str(" For Podman, `systemctl --user enable --now podman.socket` starts its Docker-compatible socket.");
        } else {
            out.push_str(" Start Docker Desktop (or your runtime) and send your message again.");
        }
        out.push_str(" To use another socket, set DOCKER_HOST.");
        out
    }
}

/// `Tried::why` for a place with nothing in it, which the message lists apart from the broken ones.
const ABSENT: &str = "not found";

/// The runtime in use.
pub struct Connected {
    pub docker: Docker,
    pub kind: Kind,
    pub endpoint: Endpoint,
}

impl Connected {
    /// For the pane: `Podman at unix:///run/user/1000/podman/podman.sock`.
    pub fn label(&self) -> String {
        format!("{} at {}", self.kind.name(), self.endpoint)
    }
}

/// Where the runtime in use listens, for a CLI that must reach the same daemon as the manager does.
static IN_USE: std::sync::Mutex<Option<Endpoint>> = std::sync::Mutex::new(None);

/// `DOCKER_HOST` for the runtime in use, when it is a plain unix socket or TCP address; `None` when
/// none is connected yet, or the endpoint is a pipe or needs TLS files (the CLI's own default then applies).
pub fn docker_host_in_use() -> Option<String> {
    docker_host_of(IN_USE.lock().unwrap_or_else(|e| e.into_inner()).as_ref()?)
}

fn docker_host_of(endpoint: &Endpoint) -> Option<String> {
    match endpoint {
        Endpoint::Unix(path) => Some(format!("unix://{}", path.display())),
        Endpoint::Tcp { host, tls: false } => Some(format!("tcp://{host}")),
        _ => None,
    }
}

/// The first place that answers a ping, connected; or what was looked at.
pub async fn connect(env: &Env) -> Result<Connected, Missing> {
    let places = candidates(env).map_err(|why| Missing { tried: Vec::new(), unusable: Some(why) })?;
    let mut tried = Vec::new();
    for c in places {
        match try_candidate(&c, env).await {
            Ok(docker) => {
                *IN_USE.lock().unwrap_or_else(|e| e.into_inner()) = Some(c.endpoint.clone());
                return Ok(Connected { docker, kind: c.kind, endpoint: c.endpoint });
            }
            Err(why) => tried.push(Tried { kind: c.kind, endpoint: c.endpoint, why }),
        }
    }
    Err(Missing { tried, unusable: None })
}

/// Connects to `c` and pings it. `Err` is the reason in a few words.
async fn try_candidate(c: &Candidate, env: &Env) -> Result<Docker, String> {
    match &c.endpoint {
        Endpoint::Unix(path) => {
            check_socket(path, c.user_owned, env.uid)?;
            ping_unix(path, env.ping_timeout).await?;
            #[cfg(unix)]
            return Docker::connect_with_unix(&path.to_string_lossy(), API_TIMEOUT_SECS, bollard::API_DEFAULT_VERSION)
                .map_err(|e| e.to_string());
            // `check_socket` has already refused a unix socket here.
            #[cfg(not(unix))]
            Err("unix sockets are not available on this system".to_string())
        }
        // From here on the address is the one parsed above, never `DOCKER_HOST` read again by
        // bollard: what was checked is what is connected to.
        Endpoint::Tcp { host, tls } => {
            let addr = format!("tcp://{host}");
            let docker = if *tls {
                let dir = env.cert_dir.as_deref().ok_or_else(|| {
                    "TLS needs key.pem, cert.pem and ca.pem, and neither DOCKER_CERT_PATH nor ~/.docker is known".to_string()
                })?;
                Docker::connect_with_ssl(
                    &addr,
                    &dir.join("key.pem"),
                    &dir.join("cert.pem"),
                    &dir.join("ca.pem"),
                    API_TIMEOUT_SECS,
                    bollard::API_DEFAULT_VERSION,
                )
            } else {
                Docker::connect_with_http(&addr, API_TIMEOUT_SECS, bollard::API_DEFAULT_VERSION)
            }
            .map_err(|e| connection_error(&e.to_string()))?;
            ping_docker(docker, env.ping_timeout).await
        }
        Endpoint::Pipe(pipe) => {
            #[cfg(windows)]
            {
                let docker = Docker::connect_with_named_pipe(pipe, API_TIMEOUT_SECS, bollard::API_DEFAULT_VERSION)
                    .map_err(|e| connection_error(&e.to_string()))?;
                return ping_docker(docker, env.ping_timeout).await;
            }
            #[cfg(not(windows))]
            {
                let _ = pipe;
                Err("named pipes are only on Windows".to_string())
            }
        }
    }
}

/// `docker` if it answers a ping in time.
async fn ping_docker(docker: Docker, timeout: Duration) -> Result<Docker, String> {
    match tokio::time::timeout(timeout, docker.ping()).await {
        Ok(Ok(_)) => Ok(docker),
        Ok(Err(e)) => Err(connection_error(&e.to_string())),
        Err(_) => Err(format!("no answer within {}s", timeout.as_secs_f32().ceil())),
    }
}

/// bollard's text for a connection that could not be made is hyper's internals; say it plainly.
fn connection_error(text: &str) -> String {
    if text.contains("(Connect)") || text.to_ascii_lowercase().contains("connection refused") {
        "could not connect: is the daemon running and reachable?".to_string()
    } else {
        text.to_string()
    }
}

/// Whether `path` is a socket the user may talk to.
#[cfg(unix)]
fn check_socket(path: &Path, user_owned: bool, uid: Option<u32>) -> Result<(), String> {
    use std::os::unix::fs::{FileTypeExt, MetadataExt};
    let meta = std::fs::metadata(path).map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => ABSENT.to_string(),
        std::io::ErrorKind::PermissionDenied => "permission denied".to_string(),
        _ => e.to_string(),
    })?;
    if !meta.file_type().is_socket() {
        return Err("not a socket".to_string());
    }
    if let (true, Some(uid)) = (user_owned, uid) {
        if meta.uid() != uid {
            return Err(format!("owned by another user (uid {}), not you", meta.uid()));
        }
    }
    Ok(())
}

#[cfg(not(unix))]
fn check_socket(_path: &Path, _user_owned: bool, _uid: Option<u32>) -> Result<(), String> {
    Err("unix sockets are not available on this system".to_string())
}

/// `GET /_ping` over a unix socket; any `200` is an answer (Docker and Podman both give one).
#[cfg(unix)]
async fn ping_unix(path: &Path, timeout: Duration) -> Result<(), String> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let exchange = async {
        let mut s = tokio::net::UnixStream::connect(path).await.map_err(connect_error)?;
        s.write_all(b"GET /_ping HTTP/1.1\r\nHost: docker\r\nConnection: close\r\n\r\n")
            .await
            .map_err(|e| e.to_string())?;
        // The status line is enough; a runtime that keeps the connection open is still answering.
        let mut head = Vec::new();
        let mut chunk = [0u8; 256];
        while !head.windows(2).any(|w| w == b"\r\n") && head.len() < 1024 {
            let n = s.read(&mut chunk).await.map_err(|e| e.to_string())?;
            if n == 0 {
                break;
            }
            head.extend_from_slice(&chunk[..n]);
        }
        status_of(&head)
    };
    match tokio::time::timeout(timeout, exchange).await {
        Ok(r) => r,
        Err(_) => Err(format!("accepted the connection but gave no answer within {}s", timeout.as_secs_f32().ceil())),
    }
}

#[cfg(not(unix))]
async fn ping_unix(_path: &Path, _timeout: Duration) -> Result<(), String> {
    Err("unix sockets are not available on this system".to_string())
}

#[cfg(unix)]
fn connect_error(e: std::io::Error) -> String {
    match e.kind() {
        std::io::ErrorKind::PermissionDenied => "permission denied".to_string(),
        std::io::ErrorKind::ConnectionRefused => "connection refused: nothing is listening, so start the service".to_string(),
        std::io::ErrorKind::NotFound => ABSENT.to_string(),
        _ => e.to_string(),
    }
}

/// `Ok` for an HTTP `200` status line.
#[cfg(unix)]
fn status_of(head: &[u8]) -> Result<(), String> {
    let line = String::from_utf8_lossy(head);
    let line = line.lines().next().unwrap_or("").trim();
    let mut parts = line.split_whitespace();
    match (parts.next(), parts.next()) {
        (Some(v), Some("200")) if v.starts_with("HTTP/1.") => Ok(()),
        (Some(v), Some(code)) if v.starts_with("HTTP/") => Err(format!("answered HTTP {code}, not a container runtime's ping")),
        _ => Err("did not answer like a container runtime".to_string()),
    }
}

/// State of the container-runtime slot held by [`ContainerRuntimeHandle`].
enum RuntimeSlot {
    /// Test/no-docker-expected fixtures — `get()` never attempts a real
    /// connect, always reports unavailable.
    #[cfg(test)]
    Disabled,
    /// No manager yet: never found a runtime, or the last one stopped answering. `get()` looks
    /// again ([`ContainerManager::connect_detected`]) while in this state, at most once per
    /// [`RETRY_DETECT_AFTER`].
    Empty,
    Connected(ContainerManager),
}

/// How long a failed detection stands before the next call looks again.
const RETRY_DETECT_AFTER: std::time::Duration = std::time::Duration::from_secs(3);

/// Self-healing holder for the container runtime connection.
///
/// A plain `Option<ContainerManager>` fixed at process boot means a Docker
/// daemon that starts (or a socket/named-pipe that appears) AFTER AgentMux
/// launched is never picked up without an app restart — every consumer
/// (the availability probe AND the actual container-launch code paths)
/// would see a permanent `None`. This type retries on demand instead, so
/// "start Docker Desktop while AgentMux is already running" is picked up
/// within one call, everywhere `container_manager` is read.
///
/// See docs/retro/RETRO_DOCKER_DETECTION_DIVERGENCE_2026_07_04.md.
pub struct ContainerRuntimeHandle {
    slot: tokio::sync::RwLock<RuntimeSlot>,
    /// When the last detection failed and what it said, so a pane asking again within
    /// [`RETRY_DETECT_AFTER`] gets the same answer without another round of pings.
    last_miss: std::sync::Mutex<Option<(std::time::Instant, String)>>,
    /// Dev-proxy registry to attach to every `ContainerManager` this handle
    /// ever produces — including across reconnects, each of which creates
    /// a fresh manager instance with its own unset `attach_dev_proxy_registry`
    /// state. `None` until `set_dev_proxy_registry` is called (bootstrap
    /// wires this in once both this handle and the registry exist); tests
    /// using `disabled()` never set it, and `get()` never even constructs
    /// a manager in that case. See
    /// docs/specs/SPEC_NATIVE_CONTAINER_DEV_PROXY_2026_09_19.md.
    dev_proxy: std::sync::RwLock<Option<crate::backend::dev_proxy::DevProxyRegistry>>,
}

impl ContainerRuntimeHandle {
    /// Boot-time constructor. The slot starts `Empty`: finding the runtime pings sockets, which
    /// is async, so the first `get()` (the availability check spawned at boot makes it) runs
    /// [`ContainerManager::connect_detected`]. A failed look leaves the slot `Empty`, so
    /// `get()`/`is_available()` look again later instead of staying stuck for the process lifetime.
    pub fn connect_at_startup() -> Self {
        Self {
            slot: tokio::sync::RwLock::new(RuntimeSlot::Empty),
            last_miss: std::sync::Mutex::new(None),
            dev_proxy: std::sync::RwLock::new(None),
        }
    }

    /// Test-only constructor: permanently reports unavailable and never
    /// attempts a real connect, keeping host-only unit tests hermetic and
    /// deterministic regardless of whether the test box has Docker.
    #[cfg(test)]
    pub fn disabled() -> Self {
        Self {
            slot: tokio::sync::RwLock::new(RuntimeSlot::Disabled),
            last_miss: std::sync::Mutex::new(None),
            dev_proxy: std::sync::RwLock::new(None),
        }
    }

    /// Test-only: a handle that already holds `mgr`.
    #[cfg(test)]
    fn connected_for_test(mgr: ContainerManager) -> Self {
        Self {
            slot: tokio::sync::RwLock::new(RuntimeSlot::Connected(mgr)),
            last_miss: std::sync::Mutex::new(None),
            dev_proxy: std::sync::RwLock::new(None),
        }
    }

    /// Wire a dev-proxy registry into this handle so every
    /// `ContainerManager` it produces — now and after any future Docker
    /// reconnect — has `stop`/`remove` clear that agent's routes. Call
    /// once at bootstrap, after both this handle and the registry exist
    /// (see `bootstrap::build_app_state`).
    pub fn set_dev_proxy_registry(&self, registry: crate::backend::dev_proxy::DevProxyRegistry) {
        if let Ok(mut slot) = self.dev_proxy.write() {
            *slot = Some(registry.clone());
        }
        // Attach immediately to a manager that already exists (none at boot, where the slot is
        // still `Empty`), not just future ones: `get()` attaches the stored registry itself.
        if let Ok(slot) = self.slot.try_read() {
            if let RuntimeSlot::Connected(mgr) = &*slot {
                mgr.attach_dev_proxy_registry(registry);
            }
        }
    }

    /// Returns a connected manager, looking for the runtime
    /// ([`ContainerManager::connect_detected`]) if the slot is currently `Empty`. Cheap on the
    /// happy path (a read-lock plus a cheap `Arc`-backed clone); only takes the write lock and
    /// looks again when we don't already have a manager, and not within [`RETRY_DETECT_AFTER`]
    /// of a look that found none.
    pub async fn get(&self) -> Option<ContainerManager> {
        {
            let slot = self.slot.read().await;
            match &*slot {
                RuntimeSlot::Connected(mgr) => return Some(mgr.clone()),
                #[cfg(test)]
                RuntimeSlot::Disabled => return None,
                RuntimeSlot::Empty => {}
            }
        }
        let mut slot = self.slot.write().await;
        // Re-check under the write lock — another caller may have already
        // connected between our read-unlock and this write-lock.
        if let RuntimeSlot::Connected(mgr) = &*slot {
            return Some(mgr.clone());
        }
        #[cfg(test)]
        if matches!(&*slot, RuntimeSlot::Disabled) {
            return None;
        }
        if let Some((at, _)) = &*self.last_miss.lock().unwrap() {
            if at.elapsed() < RETRY_DETECT_AFTER {
                return None;
            }
        }
        match ContainerManager::connect_detected().await {
            Ok(mgr) => {
                // A reconnect makes a brand-new `ContainerManagerInner`
                // (fresh `OnceLock`s) — re-attach the registry here so
                // `stop`/`remove` cleanup survives a Docker daemon
                // restart, not just the initial connect.
                if let Ok(guard) = self.dev_proxy.read() {
                    if let Some(registry) = guard.as_ref() {
                        mgr.attach_dev_proxy_registry(registry.clone());
                    }
                }
                *self.last_miss.lock().unwrap() = None;
                *slot = RuntimeSlot::Connected(mgr.clone());
                Some(mgr)
            }
            Err(e) => {
                *self.last_miss.lock().unwrap() = Some((std::time::Instant::now(), e.user_message()));
                None
            }
        }
    }

    /// [`get`](Self::get), or the plain-language reason there is no runtime
    /// (where it looked and why each place failed), for the pane.
    pub async fn get_or_explain(&self) -> Result<crate::backend::container::ContainerManager, String> {
        match self.get().await {
            Some(mgr) => Ok(mgr),
            None => Err(self
                .missing_message()
                .unwrap_or_else(|| "Docker not available on this host; cannot start container agent".to_string())),
        }
    }

    /// The runtime in use (`Podman at unix:///…`), if one is connected.
    pub async fn runtime_label(&self) -> Option<String> {
        match &*self.slot.read().await {
            RuntimeSlot::Connected(mgr) => mgr.runtime_label().map(str::to_string),
            _ => None,
        }
    }

    /// What was missing the last time looking for a runtime found none: where it looked and why
    /// each place failed, for the pane. `None` while one is connected or before the first look.
    pub fn missing_message(&self) -> Option<String> {
        self.last_miss.lock().unwrap().as_ref().map(|(_, m)| m.clone())
    }

    /// True iff a manager is available AND its daemon answers a live ping
    /// right now. Never trusts a cached `Connected` slot alone — the
    /// daemon can go down again after a successful connect, and
    /// `check_available` is a real `docker.ping()`, not a cached flag.
    pub async fn is_available(&self) -> bool {
        match self.get().await {
            Some(mgr) => {
                if mgr.check_available().await.is_ok() {
                    return true;
                }
                // The runtime stopped answering: forget it, so the next call looks again and can
                // land on another one (Docker stopped, Podman running).
                let mut slot = self.slot.write().await;
                if matches!(&*slot, RuntimeSlot::Connected(_)) {
                    *slot = RuntimeSlot::Empty;
                    // Say so in `missing_message()`, backdated so the next call still looks again.
                    let said = match mgr.runtime_label() {
                        Some(label) => format!("Container agents need Docker or Podman, and {label} stopped answering. Start it again, or set DOCKER_HOST to another socket."),
                        None => "Container agents need Docker or Podman, and the runtime stopped answering. Start it again, or set DOCKER_HOST to another socket.".to_string(),
                    };
                    let at = std::time::Instant::now().checked_sub(RETRY_DETECT_AFTER).unwrap_or_else(std::time::Instant::now);
                    *self.last_miss.lock().unwrap() = Some((at, said));
                }
                false
            }
            None => false,
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::UnixListener;

    fn env(dir: &Path, os: Os) -> Env {
        Env {
            docker_host: None,
            tls_verify: false,
            xdg_runtime_dir: Some(dir.join("run-user")),
            home: Some(dir.join("home")),
            cert_dir: None,
            os,
            uid: Some(unsafe { libc::geteuid() }),
            root: dir.join("root"),
            ping_timeout: Duration::from_millis(300),
        }
    }

    fn at(dir: &Path, rel: &str) -> PathBuf {
        let p = dir.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        p
    }

    /// A runtime that answers every request with `status`.
    fn serve(path: PathBuf, status: &'static str) -> tokio::task::JoinHandle<()> {
        let listener = UnixListener::bind(path).unwrap();
        tokio::spawn(async move {
            while let Ok((mut s, _)) = listener.accept().await {
                tokio::spawn(async move {
                    let mut buf = [0u8; 512];
                    let _ = s.read(&mut buf).await;
                    let _ = s.write_all(format!("HTTP/1.1 {status}\r\nContent-Length: 2\r\n\r\nOK").as_bytes()).await;
                });
            }
        })
    }

    /// A socket file whose service has gone: the listener closes, the file stays.
    fn stale(path: PathBuf) {
        drop(std::os::unix::net::UnixListener::bind(path).unwrap());
    }

    fn kinds(env: &Env) -> Vec<Kind> {
        candidates(env).unwrap().into_iter().map(|c| c.kind).collect()
    }

    #[test]
    fn linux_tries_rootless_first_then_desktop_then_the_system_sockets() {
        let e = env(Path::new("/t"), Os::Linux);
        assert_eq!(
            candidates(&e).unwrap().into_iter().map(|c| c.endpoint.to_string()).collect::<Vec<_>>(),
            [
                "unix:///t/run-user/docker.sock",
                "unix:///t/run-user/podman/podman.sock",
                "unix:///t/home/.docker/desktop/docker.sock",
                "unix:///t/home/.docker/run/docker.sock",
                "unix:///t/root/var/run/docker.sock",
                "unix:///t/root/run/podman/podman.sock",
            ]
        );
        assert_eq!(
            kinds(&e),
            [
                Kind::RootlessDocker,
                Kind::RootlessPodman,
                Kind::DockerDesktop,
                Kind::DockerDesktop,
                Kind::Docker,
                Kind::Podman
            ]
        );
        // Without a runtime dir there is no rootless to look for.
        let e = Env { xdg_runtime_dir: None, ..e };
        assert_eq!(kinds(&e), [Kind::DockerDesktop, Kind::DockerDesktop, Kind::Docker, Kind::Podman]);
    }

    #[test]
    fn macos_tries_desktop_orbstack_colima_and_rancher() {
        let e = env(Path::new("/t"), Os::MacOs);
        assert_eq!(
            kinds(&e),
            [Kind::DockerDesktop, Kind::OrbStack, Kind::Colima, Kind::RancherDesktop, Kind::Docker]
        );
        assert!(candidates(&e).unwrap()[2].endpoint.to_string().ends_with("/.colima/default/docker.sock"));
    }

    #[test]
    fn windows_keeps_the_named_pipe() {
        let c = candidates(&env(Path::new("/t"), Os::Windows)).unwrap();
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].endpoint, Endpoint::Pipe("//./pipe/docker_engine".to_string()));
    }

    /// Plain tcp:// or http:// only to this computer: the daemon is handed the account's
    /// credentials and the scoped srv token.
    #[test]
    fn an_unencrypted_docker_host_is_for_this_computer_only() {
        let with = |h: &str, tls: bool| Env {
            docker_host: Some(h.to_string()),
            tls_verify: tls,
            ..env(Path::new("/t"), Os::Linux)
        };
        for local in ["tcp://localhost:2375", "http://LOCALHOST:2375", "tcp://127.0.0.1:2375", "tcp://127.8.9.10", "tcp://[::1]:2375", "tcp://[::ffff:127.0.0.1]:2375", "tcp://::1"] {
            assert!(candidates(&with(local, false)).is_ok(), "{local} is this computer");
        }
        for remote in ["tcp://203.0.113.5:2375", "http://dock.example:2375", "tcp://198.51.100.2", "tcp://[2001:db8::1]:2375", "tcp://localhost.example.com:2375", "tcp://127.0.0.1.example.com:2375"] {
            let why = candidates(&with(remote, false)).unwrap_err();
            assert!(why.contains("unencrypted") && why.contains("https://") && why.contains("DOCKER_TLS_VERIFY"), "{remote}: {why}");
            assert!(!why.contains("  ") && !why.contains('\n'), "the sentence is one line, single-spaced: {why}");
        }
        // The address is read as the transport reads it: what comes before an `@` is user
        // information, not the host.
        for tricky in [
            "tcp://[::1]@203.0.113.5:2375",
            "tcp://127.0.0.1@203.0.113.5:2375",
            "tcp://localhost@dock.example",
            "tcp://127.0.0.1:2375/../",
            "tcp://127.0.0.1:2375?x=1",
            "tcp://127.0.0.1:",
            "tcp://127.0.0.1:99999999",
            "tcp://127.0.0.1 :2375",
            "tcp://",
        ] {
            assert!(candidates(&with(tricky, false)).is_err(), "{tricky} must be refused");
            assert!(candidates(&with(tricky, true)).is_err(), "{tricky} must be refused with TLS too");
        }
        // With TLS, or over https://, any host is fine.
        assert!(candidates(&with("tcp://203.0.113.5:2376", true)).is_ok());
        assert!(candidates(&with("https://dock.example:2376", false)).is_ok());
    }

    #[test]
    fn the_cli_is_pointed_at_a_unix_socket_or_plain_tcp_and_left_alone_otherwise() {
        let sock = Endpoint::Unix(PathBuf::from("/run/user/1000/podman/podman.sock"));
        assert_eq!(docker_host_of(&sock).as_deref(), Some("unix:///run/user/1000/podman/podman.sock"));
        assert_eq!(docker_host_of(&Endpoint::Tcp { host: "10.0.0.5:2375".into(), tls: false }).as_deref(), Some("tcp://10.0.0.5:2375"));
        assert_eq!(docker_host_of(&Endpoint::Tcp { host: "h:2376".into(), tls: true }), None, "TLS needs files the CLI finds itself");
        assert_eq!(docker_host_of(&Endpoint::Pipe("//./pipe/docker_engine".into())), None);
    }

    #[test]
    fn docker_host_decides_alone() {
        let with = |h: &str, tls: bool| Env {
            docker_host: Some(h.to_string()),
            tls_verify: tls,
            ..env(Path::new("/t"), Os::Linux)
        };
        let one = |h: &str, tls: bool| {
            let c = candidates(&with(h, tls)).unwrap();
            assert_eq!(c.len(), 1, "no probing beside an explicit DOCKER_HOST: {h}");
            c[0].endpoint.clone()
        };
        assert_eq!(one("unix:///run/x.sock", false), Endpoint::Unix("/run/x.sock".into()));
        assert_eq!(one("tcp://127.0.0.1:2375", false), Endpoint::Tcp { host: "127.0.0.1:2375".into(), tls: false });
        assert_eq!(one("tcp://203.0.113.5:2376", true), Endpoint::Tcp { host: "203.0.113.5:2376".into(), tls: true });
        assert_eq!(one("https://dock.example:2376/", false), Endpoint::Tcp { host: "dock.example:2376".into(), tls: true });
        assert_eq!(one("npipe:////./pipe/docker_engine", false), Endpoint::Pipe("//./pipe/docker_engine".into()));
        assert_eq!(
            candidates(&with("ssh://me@box", false)).unwrap_err().contains("ssh://"),
            true,
            "an unsupported scheme is named"
        );
        assert!(candidates(&with("/var/run/docker.sock", false)).unwrap_err().contains("not an address"));
    }

    #[tokio::test]
    async fn the_first_socket_that_answers_wins_and_stale_ones_are_skipped() {
        let t = tempfile::TempDir::new().unwrap();
        let e = env(t.path(), Os::Linux);
        stale(at(t.path(), "run-user/docker.sock"));
        let podman = serve(at(t.path(), "run-user/podman/podman.sock"), "200 OK");
        // A later socket would answer too, and is not reached.
        let system = serve(at(t.path(), "root/var/run/docker.sock"), "200 OK");

        let got = connect(&e).await.ok().expect("podman answers");
        assert_eq!(got.kind, Kind::RootlessPodman);
        assert!(got.label().starts_with("rootless Podman at unix://"), "{}", got.label());
        assert!(got.label().ends_with("/run-user/podman/podman.sock"));
        podman.abort();
        system.abort();
    }

    /// A tcp:// DOCKER_HOST is connected to at the address that was parsed, whatever the
    /// process environment says.
    #[tokio::test]
    async fn a_tcp_docker_host_is_connected_at_the_parsed_address() {
        use tokio::net::TcpListener;
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = tokio::spawn(async move {
            while let Ok((mut s, _)) = listener.accept().await {
                tokio::spawn(async move {
                    let mut buf = [0u8; 1024];
                    let _ = s.read(&mut buf).await;
                    let _ = s.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nOK").await;
                });
            }
        });
        let t = tempfile::TempDir::new().unwrap();
        let e = Env { docker_host: Some(format!("tcp://127.0.0.1:{port}")), ..env(t.path(), Os::Linux) };
        let got = connect(&e).await.ok().expect("the tcp runtime answers");
        assert_eq!(got.kind, Kind::Configured);
        assert_eq!(got.endpoint, Endpoint::Tcp { host: format!("127.0.0.1:{port}"), tls: false });
        server.abort();
    }

    /// Docker Desktop for Linux keeps its socket in `~/.docker/desktop/`, not `run/`.
    #[tokio::test]
    async fn docker_desktop_on_linux_is_found_at_its_own_path() {
        let t = tempfile::TempDir::new().unwrap();
        let desktop = serve(at(t.path(), "home/.docker/desktop/docker.sock"), "200 OK");
        let got = connect(&env(t.path(), Os::Linux)).await.ok().expect("Docker Desktop answers");
        assert_eq!(got.kind, Kind::DockerDesktop);
        assert!(got.label().ends_with("/home/.docker/desktop/docker.sock"), "{}", got.label());
        desktop.abort();
    }

    #[tokio::test]
    async fn nothing_answering_says_what_each_place_was() {
        let t = tempfile::TempDir::new().unwrap();
        let e = env(t.path(), Os::Linux);
        stale(at(t.path(), "run-user/docker.sock"));
        std::fs::write(at(t.path(), "root/var/run/docker.sock"), b"not a socket").unwrap();
        let bad = serve(at(t.path(), "home/.docker/desktop/docker.sock"), "500 Internal Server Error");

        let Err(missing) = connect(&e).await else { panic!("nothing should answer") };
        let why = |k: Kind| missing.tried.iter().find(|t| t.kind == k).unwrap().why.clone();
        assert!(why(Kind::RootlessDocker).contains("connection refused"), "{}", why(Kind::RootlessDocker));
        assert_eq!(why(Kind::Docker), "not a socket");
        assert!(why(Kind::DockerDesktop).contains("HTTP 500"), "{}", why(Kind::DockerDesktop));
        assert!(why(Kind::Podman).starts_with(ABSENT));

        let m = missing.message();
        assert!(m.contains("none answered"), "{m}");
        assert!(m.contains("rootless Docker at unix://") && m.contains("connection refused"), "{m}");
        assert!(m.contains("Nothing at:") && m.contains("/root/run/podman/podman.sock"), "{m}");
        assert!(m.contains("systemctl --user enable --now podman.socket"), "{m}");
        assert!(m.contains("DOCKER_HOST"), "{m}");
        bad.abort();
    }

    #[tokio::test]
    async fn a_socket_under_the_users_directories_must_be_theirs() {
        let t = tempfile::TempDir::new().unwrap();
        let server = serve(at(t.path(), "run-user/docker.sock"), "200 OK");
        let mine = env(t.path(), Os::Linux);
        assert!(connect(&mine).await.is_ok());

        // The same socket, for a process that is someone else.
        let other = Env { uid: Some(mine.uid.unwrap().wrapping_add(1)), ..mine };
        let Err(missing) = connect(&other).await else { panic!("another user's socket must not be used") };
        let t0 = &missing.tried[0];
        assert_eq!(t0.kind, Kind::RootlessDocker);
        assert!(t0.why.starts_with("owned by another user"), "{}", t0.why);
        server.abort();
    }

    #[tokio::test]
    async fn a_system_socket_is_used_whoever_owns_it() {
        let t = tempfile::TempDir::new().unwrap();
        let server = serve(at(t.path(), "root/var/run/docker.sock"), "200 OK");
        let e = Env { uid: Some(unsafe { libc::geteuid() }.wrapping_add(1)), ..env(t.path(), Os::Linux) };
        assert_eq!(connect(&e).await.ok().expect("the system socket answers").kind, Kind::Docker);
        server.abort();
    }

    #[tokio::test]
    async fn a_socket_that_accepts_but_never_answers_is_given_up_on() {
        let t = tempfile::TempDir::new().unwrap();
        let path = at(t.path(), "run-user/docker.sock");
        let listener = UnixListener::bind(&path).unwrap();
        let hold = tokio::spawn(async move {
            let mut held = Vec::new();
            while let Ok((s, _)) = listener.accept().await {
                held.push(s);
            }
        });
        let Err(missing) = connect(&env(t.path(), Os::Linux)).await else { panic!("no answer expected") };
        assert!(missing.tried[0].why.contains("gave no answer"), "{}", missing.tried[0].why);
        hold.abort();
    }

    #[tokio::test]
    async fn a_docker_host_that_does_not_answer_is_not_second_guessed() {
        let t = tempfile::TempDir::new().unwrap();
        // A working socket sits in the usual place; DOCKER_HOST points elsewhere and wins.
        let server = serve(at(t.path(), "run-user/docker.sock"), "200 OK");
        let e = Env { docker_host: Some("unix:///nonexistent.sock".into()), ..env(t.path(), Os::Linux) };
        let Err(missing) = connect(&e).await else { panic!("DOCKER_HOST alone decides") };
        assert_eq!(missing.tried.len(), 1);
        let m = missing.message();
        assert!(m.contains("/nonexistent.sock") && m.contains("DOCKER_HOST is set"), "{m}");
        assert!(!m.contains("To use another socket"), "{m}");
        server.abort();
    }

    /// A runtime that stops answering is forgotten, and the next availability answer says so.
    #[tokio::test]
    async fn a_runtime_that_stops_answering_is_forgotten_with_a_reason() {
        let t = tempfile::TempDir::new().unwrap();
        let path = at(t.path(), "run-user/docker.sock");
        let server = serve(path.clone(), "200 OK");
        let docker = Docker::connect_with_unix(&path.to_string_lossy(), 5, bollard::API_DEFAULT_VERSION).unwrap();
        let mgr = ContainerManager::with_docker(docker, Some(format!("rootless Docker at unix://{}", path.display())));
        let handle = ContainerRuntimeHandle::connected_for_test(mgr);
        assert!(handle.is_available().await, "answers while the service is up");
        assert!(handle.runtime_label().await.unwrap().starts_with("rootless Docker at"));

        server.abort();
        let _ = server.await; // the listener drops with the task
        assert!(!handle.is_available().await, "stopped");
        assert!(handle.runtime_label().await.is_none(), "forgotten");
        let said = handle.missing_message().expect("says why");
        assert!(said.contains("rootless Docker at") && said.contains("stopped answering"), "{said}");
    }

    #[test]
    fn hypers_connect_error_is_said_plainly() {
        assert!(connection_error("Error in the hyper legacy client: client error (Connect)").starts_with("could not connect"));
        assert_eq!(connection_error("something else"), "something else");
    }

    #[tokio::test]
    async fn an_unusable_docker_host_is_the_whole_answer() {
        let e = Env { docker_host: Some("ssh://me@box".into()), ..env(Path::new("/t"), Os::Linux) };
        let Err(missing) = connect(&e).await else { panic!("ssh is not supported") };
        assert!(missing.tried.is_empty());
        assert!(missing.message().starts_with("Container agents need Docker or Podman: DOCKER_HOST uses ssh://"));
    }

    /// Prints what detection makes of this machine. Run by hand:
    /// `cargo test -p agentmux-srv --bins -- --ignored --nocapture itest_what_this_machine_has`
    #[tokio::test]
    #[ignore = "reads this machine's real sockets"]
    async fn itest_what_this_machine_has() {
        match connect(&Env::from_process()).await {
            Ok(found) => {
                let version = found.docker.version().await.map(|v| v.version.unwrap_or_default());
                println!("runtime: {} (daemon version: {version:?})", found.label());
            }
            Err(missing) => println!("no runtime: {}", missing.message()),
        }
    }
}
