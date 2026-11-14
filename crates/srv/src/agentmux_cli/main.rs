// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `agentmux`: run and manage the AgentMux server on this machine (a server with no desktop app,
//! whose UI is a browser on its own loopback address). SPEC_HEADLESS_SERVER_PACKAGE_AND_LOCAL_WEB_UI §4.1.
//!
//! The server runs on its own channel (`headless` unless `--channel` says otherwise), which gives it
//! its own data folder, lock and ports, so it can run beside a desktop AgentMux of the same person.
//! This command starts nothing of the server's own logic: `run` prepares the channel's data for this
//! version (`agentmux-srv migrate --prepare`) and becomes `agentmux-srv --headless`; `start` runs
//! `run` in the background; `stop` asks the server to shut down; `open` and `logout` are the sign-in
//! client of the server (`agentmux-srv open`, `agentmux-srv logout`), run for the server of this channel.
//!
//! Exit codes: 0 done, 1 the thing asked is not so (not running), 2 bad use, 3 could not do it.
//! Nothing here prints a key, a code or a token.

mod doctor;
mod process;
mod service;
mod settings;
mod setup;
mod state;

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use agentmux_common::win32::NoWindow;
use agentmux_common::{DataPaths, RuntimeMode};
use clap::{Args, Parser, Subcommand};

use settings::SecretStore;
use state::ServerState;

const VERSION: &str = env!("CARGO_PKG_VERSION");

/// What this command is called where the person runs it: `agentmux`, or `agentmux-server`, the name
/// the install always links and the one to use on a machine whose `agentmux` is the desktop app's.
fn invoked_name() -> &'static str {
    static NAME: std::sync::OnceLock<&'static str> = std::sync::OnceLock::new();
    NAME.get_or_init(|| {
        let argv0 = std::env::args_os().next().map(PathBuf::from);
        let name = argv0.as_deref().and_then(Path::file_stem).map(|n| n.to_string_lossy().into_owned());
        if name.as_deref() == Some("agentmux-server") { "agentmux-server" } else { "agentmux" }
    })
}

/// A text for the person, with every command of this program named the way they ran it.
pub(crate) fn say_as(text: &str) -> String {
    match invoked_name() {
        "agentmux" => text.to_string(),
        name => text.replace("`agentmux ", &format!("`{name} ")),
    }
}

/// A line to the person on stderr, led by the program's name.
macro_rules! warn {
    ($($arg:tt)*) => { eprintln!("{}: {}", invoked_name(), say_as(&format!($($arg)*))) };
}

/// A line to the person on stdout, with the commands it names under the name they ran.
macro_rules! tell {
    ($($arg:tt)*) => { println!("{}", say_as(&format!($($arg)*))) };
}
const DEFAULT_CHANNEL: &str = "headless";
/// The server's key file name inside its data folder.
const KEY_FILE_NAME: &str = "srv-auth-key";
/// How long `start` waits for the server to answer, which includes preparing the data of a new version.
const START_WAIT: Duration = Duration::from_secs(120);
/// How long `stop` waits for the server to leave after it was asked to.
const STOP_WAIT: Duration = Duration::from_secs(30);

#[derive(Parser, Debug)]
#[command(name = "agentmux", version, about = "Run and manage the AgentMux server on this machine")]
struct Cli {
    /// The server's channel: its own data, settings and agents, apart from a desktop AgentMux's.
    #[arg(long, global = true, default_value = DEFAULT_CHANNEL)]
    channel: String,
    #[command(subcommand)]
    command: Cmd,
}

#[derive(Args, Debug, Clone, Default)]
struct ServerArgs {
    /// The web port the browser opens (the websocket port is the next one). Default: the one `setup` kept, else 7380.
    #[arg(long, value_parser = clap::value_parser!(u16).range(1024..=65534))]
    port: Option<u16>,
    /// Where the server keeps secrets. Default: the one `setup` kept, else `file`.
    #[arg(long, value_enum)]
    secret_store: Option<SecretStore>,
    /// The address a browser reaches the server on when it is not the server's own: `http://127.0.0.1:<port>`.
    #[arg(long)]
    public_origin: Option<String>,
    /// The server program (default: `agentmux-srv` beside this command).
    #[arg(long, value_name = "FILE")]
    srv_bin: Option<PathBuf>,
    /// The built web UI (default: `web` beside the install's `bin` folder).
    #[arg(long, value_name = "DIR")]
    web_dir: Option<PathBuf>,
}

#[derive(Subcommand, Debug)]
enum Cmd {
    /// Run the server in this terminal (what a service runs): prepare this version's data, then become the server.
    Run(ServerArgs),
    /// Start the server in the background.
    Start(ServerArgs),
    /// Ask the server to shut down.
    Stop {
        /// End it at once if it has not left 30 seconds after being asked.
        #[arg(long)]
        force: bool,
    },
    /// Stop, then start.
    Restart(ServerArgs),
    /// The first-run choices: port, where secrets live, a service that starts it at login; then start it and sign in.
    Setup(setup::SetupArgs),
    /// Check this machine for the server, and say what to do about each problem.
    Doctor {
        #[arg(long)]
        json: bool,
    },
    /// Is the server running, and where.
    Status {
        #[arg(long)]
        json: bool,
    },
    /// Sign a browser in: opens it at the server's address, then asks you to confirm in this terminal.
    Open {
        /// Print the one-time line for your own browser instead of opening one.
        #[arg(long)]
        print: bool,
        /// The address your browser reaches the server on, when it is not the server's own (the end of a tunnel).
        #[arg(long)]
        origin: Option<String>,
    },
    /// Sign every browser out.
    Logout,
    /// The server's log.
    Logs {
        /// How many of the last lines to show.
        #[arg(long, default_value_t = 50)]
        lines: usize,
        /// Keep showing new lines until interrupted.
        #[arg(short, long)]
        follow: bool,
    },
    /// The version.
    Version {
        #[arg(long)]
        json: bool,
    },
}

fn main() {
    let cli = Cli::parse();
    let code = dispatch(cli);
    std::process::exit(code);
}

fn dispatch(cli: Cli) -> i32 {
    if let Cmd::Version { json } = cli.command {
        return cmd_version(&cli.channel, json);
    }
    let paths = match context(&cli.channel) {
        Ok(p) => p,
        Err(e) => return fail(&e),
    };
    match cli.command {
        Cmd::Run(args) => cmd_run(&paths, &args),
        Cmd::Start(args) => cmd_start(&paths, &args),
        Cmd::Stop { force } => cmd_stop(&paths, force),
        Cmd::Restart(args) => {
            let args = restart_args(running(&paths), args);
            match cmd_stop(&paths, false) {
                0 => cmd_start(&paths, &args),
                code => code,
            }
        }
        Cmd::Setup(args) => cmd_setup(&paths, &args),
        Cmd::Doctor { json } => cmd_doctor(&paths, json),
        Cmd::Status { json } => cmd_status(&paths, json),
        Cmd::Open { print, origin } => cmd_client(&paths, "open", &client_args(print, origin)),
        Cmd::Logout => cmd_client(&paths, "logout", &[]),
        Cmd::Logs { lines, follow } => cmd_logs(&paths, lines, follow),
        Cmd::Version { .. } => unreachable!("answered above"),
    }
}

/// A restart keeps the port the running server was started on unless a flag says otherwise. The program is not kept:
/// a restart runs the installed one, which is the point of restarting after an install.
fn restart_args(was: Option<ServerState>, mut args: ServerArgs) -> ServerArgs {
    if let Some(was) = was {
        args.port = args.port.or(Some(was.web_port));
    }
    args
}

fn fail(message: &str) -> i32 {
    warn!("{message}");
    3
}

/// The channel's paths. The environment of the shell this runs in is not trusted to say anything about
/// the server: a terminal inside a desktop AgentMux carries that instance's variables, which would
/// point the server at the desktop's data. Only the test root override is kept.
fn context(channel: &str) -> Result<DataPaths, String> {
    for (key, _) in std::env::vars_os() {
        if key.to_string_lossy().starts_with("AGENTMUX_") && key != "AGENTMUX_HOME_OVERRIDE" {
            std::env::remove_var(key);
        }
    }
    std::env::set_var("AGENTMUX_CHANNEL", channel);
    DataPaths::resolve(VERSION, &RuntimeMode::Installed).map_err(|e| format!("channel {channel:?}: {e}"))
}

fn client_args(print: bool, origin: Option<String>) -> Vec<String> {
    let mut args = Vec::new();
    if print {
        args.push("--print".to_string());
    }
    if let Some(origin) = origin {
        args.extend(["--origin".to_string(), origin]);
    }
    args
}

/// This command's own folder, with links resolved: the install's `bin`.
fn exe_dir() -> Result<PathBuf, String> {
    let exe = std::env::current_exe().and_then(|p| p.canonicalize()).map_err(|e| format!("cannot find this command's own folder: {e}"))?;
    exe.parent().map(Path::to_path_buf).ok_or_else(|| "cannot find this command's own folder".to_string())
}

/// `existing` with `dir` in front of it (once), so the helpers installed beside the server are found first.
fn path_with_dir(dir: Option<&Path>, existing: Option<std::ffi::OsString>) -> std::ffi::OsString {
    let existing = existing.unwrap_or_default();
    // An empty entry would mean the current folder: it is dropped.
    let rest = std::env::split_paths(&existing).filter(|p| !p.as_os_str().is_empty() && Some(p.as_path()) != dir);
    std::env::join_paths(dir.map(Path::to_path_buf).into_iter().chain(rest)).unwrap_or(existing)
}

fn find_srv_in(bin_dir: &Path, flag: Option<&Path>) -> Result<PathBuf, String> {
    let path = flag.map(Path::to_path_buf).unwrap_or_else(|| bin_dir.join(if cfg!(windows) { "agentmux-srv.exe" } else { "agentmux-srv" }));
    // Absolute, so the folder put first on the server's PATH is a real one and never the current folder.
    match path.canonicalize() {
        Ok(abs) if abs.is_file() => Ok(abs),
        _ => Err(format!("the server program is not at {} (--srv-bin names another)", path.display())),
    }
}

fn find_web_in(bin_dir: &Path, flag: Option<&Path>) -> Result<PathBuf, String> {
    let candidates: Vec<PathBuf> = match flag {
        Some(dir) => vec![dir.to_path_buf()],
        None => vec![bin_dir.join("..").join("web"), bin_dir.join("web")],
    };
    candidates
        .iter()
        .find(|dir| dir.join("index.html").is_file())
        .map(|dir| dir.canonicalize().unwrap_or_else(|_| dir.clone()))
        .ok_or_else(|| format!("the built web UI (a folder with index.html) is not at {} (--web-dir names another)", candidates[0].display()))
}

/// The running server of this channel, by its record and its process.
fn running(paths: &DataPaths) -> Option<ServerState> {
    state::read(&paths.instance_dir).filter(|s| process::is_server(s.pid))
}

fn cmd_version(channel: &str, json: bool) -> i32 {
    let build = option_env!("BUILD_TIME").unwrap_or("dev");
    if json {
        println!("{}", serde_json::json!({ "version": VERSION, "channel": channel, "build": build }));
    } else {
        println!("{} {VERSION} (channel {channel})", invoked_name());
    }
    0
}

fn cmd_run(paths: &DataPaths, args: &ServerArgs) -> i32 {
    if !cfg!(unix) {
        return fail("the server package is for Linux and macOS in this release");
    }
    if let Err(e) = paths.ensure_dirs() {
        return fail(&format!("cannot create the channel's folders: {e}"));
    }
    if let Some(other) = running(paths).filter(|s| s.pid != std::process::id()) {
        warn!("the server is already running (pid {}) at {}", other.pid, other.url());
        return 1;
    }
    let bin_dir = match exe_dir() {
        Ok(d) => d,
        Err(e) => return fail(&e),
    };
    let (srv, web) = match (find_srv_in(&bin_dir, args.srv_bin.as_deref()), find_web_in(&bin_dir, args.web_dir.as_deref())) {
        (Ok(srv), Ok(web)) => (srv, web),
        (Err(e), _) | (_, Err(e)) => return fail(&e),
    };
    let settings = match settings::read(&paths.instance_dir) {
        Ok(s) => s,
        Err(e) => return fail(&e),
    };
    let port = args.port.unwrap_or(settings.port);
    let secret_store = args.secret_store.unwrap_or(settings.secret_store);
    let public_origin = args.public_origin.clone().or(settings.public_origin);
    if let Err(code) = prepare_data(paths, &srv) {
        return code;
    }
    let record = ServerState {
        pid: std::process::id(),
        version: VERSION.to_string(),
        channel: paths.channel.clone(),
        web_port: port,
        ws_port: port + 1,
        started_unix: agentmux_common::time::now_secs_u64(),
        key_file: paths.data_dir.join(KEY_FILE_NAME),
        data_dir: paths.data_dir.clone(),
        srv: srv.clone(),
    };
    if let Err(e) = state::write(&paths.instance_dir, &record) {
        return fail(&format!("cannot write the server's record: {e}"));
    }
    let mut cmd = Command::new(&srv);
    cmd.arg("--headless")
        .arg("--frontend-dir")
        .arg(&web)
        .args(["--web-port", &record.web_port.to_string(), "--ws-port", &record.ws_port.to_string()])
        .args(["--secret-store", secret_store.flag()]);
    if let Some(origin) = &public_origin {
        cmd.args(["--public-origin", origin]);
    }
    // The agents the server starts find their helpers (`agentmux-mcp`, `agentmux-bashwrap`) by name on the PATH.
    cmd.envs(paths.to_env_vars()).env("PATH", path_with_dir(srv.parent(), std::env::var_os("PATH"))).stdin(Stdio::null());
    become_server(cmd)
}

/// `agentmux-srv migrate --prepare` for this version's data: carries the previous version's data
/// forward and runs the pending migrations. It does nothing when there is nothing to do.
fn prepare_data(paths: &DataPaths, srv: &Path) -> Result<(), i32> {
    let log = |e: std::io::Error| fail(&format!("cannot open the server's log: {e}"));
    let out = state::open_log(&paths.instance_dir).map_err(log)?;
    let err = out.try_clone().map_err(log)?;
    let status = Command::new(srv)
        .arg("--wavedata")
        .arg(&paths.data_dir)
        .args(["migrate", "--prepare"])
        .envs(paths.to_env_vars())
        // Argument parsing asks for a key; the migration never uses it.
        .env("AGENTMUX_AUTH_KEY", "migrate-placeholder")
        .stdin(Stdio::null())
        .stdout(out)
        .stderr(err)
        .no_window()
        .status();
    match status {
        Ok(s) if s.success() => Ok(()),
        Ok(_) => Err(fail("preparing this version's data failed; nothing was started (see `agentmux logs`)")),
        Err(e) => Err(fail(&format!("cannot run {}: {e}", srv.display()))),
    }
}

/// Replace this process with the server, so the server keeps this process id and whoever supervises
/// this command (a service, `start`) supervises the server itself.
#[cfg(unix)]
fn become_server(mut cmd: Command) -> i32 {
    use std::os::unix::process::CommandExt;
    let e = cmd.exec();
    fail(&format!("cannot start the server: {e}"))
}

#[cfg(not(unix))]
fn become_server(_cmd: Command) -> i32 {
    fail("the server package is for Linux and macOS in this release")
}

fn cmd_start(paths: &DataPaths, args: &ServerArgs) -> i32 {
    if !cfg!(unix) {
        return fail("the server package is for Linux and macOS in this release");
    }
    if let Err(e) = paths.ensure_dirs() {
        return fail(&format!("cannot create the channel's folders: {e}"));
    }
    if let Some(s) = running(paths) {
        println!("AgentMux is already running: {}", s.url());
        return 0;
    }
    if let Some(home) = user_home().ok().filter(|h| service::installed(h, &paths.channel)) {
        return match service::control(&home, &paths.channel, "start") {
            Ok(()) => wait_for_server(paths, None),
            Err(e) => fail(&e),
        };
    }
    let exe = match std::env::current_exe() {
        Ok(e) => e,
        Err(e) => return fail(&format!("cannot find this command: {e}")),
    };
    let log = |e: std::io::Error| fail(&format!("cannot open the server's log: {e}"));
    let out = match state::open_log(&paths.instance_dir) {
        Ok(f) => f,
        Err(e) => return log(e),
    };
    let err = match out.try_clone() {
        Ok(f) => f,
        Err(e) => return log(e),
    };
    let mut cmd = Command::new(exe);
    cmd.args(["--channel", &paths.channel, "run"]);
    if let Some(port) = args.port {
        cmd.args(["--port", &port.to_string()]);
    }
    if let Some(store) = args.secret_store {
        cmd.args(["--secret-store", store.flag()]);
    }
    if let Some(origin) = &args.public_origin {
        cmd.args(["--public-origin", origin]);
    }
    if let Some(srv) = &args.srv_bin {
        cmd.arg("--srv-bin").arg(srv);
    }
    if let Some(web) = &args.web_dir {
        cmd.arg("--web-dir").arg(web);
    }
    cmd.stdin(Stdio::null()).stdout(out).stderr(err).no_window();
    process::detach(&mut cmd);
    let child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => return fail(&format!("cannot start the server: {e}")),
    };
    wait_for_server(paths, Some(child))
}

/// Wait until the server this start made answers. With a child of ours, only the record of that
/// child counts (a record left by an earlier server names another process); a service's start has
/// no child, and any record of a live server that answers is its.
fn wait_for_server(paths: &DataPaths, mut child: Option<std::process::Child>) -> i32 {
    let begun = Instant::now();
    loop {
        let mine = state::read(&paths.instance_dir).filter(|s| match &child {
            Some(c) => s.pid == c.id(),
            None => process::is_server(s.pid),
        });
        if let Some(s) = mine {
            if process::health_ok(s.web_port) {
                println!("AgentMux is running: {}", s.url());
                tell!("Sign in with: `agentmux open`");
                return 0;
            }
        }
        if let Some(Ok(Some(status))) = child.as_mut().map(std::process::Child::try_wait) {
            warn!("the server stopped while starting ({status}). The end of its log:");
            show_log_tail(paths, 15);
            return 3;
        }
        if begun.elapsed() > START_WAIT {
            warn!("the server has not answered after {} seconds; it is still starting. See `agentmux logs`.", START_WAIT.as_secs());
            return 3;
        }
        std::thread::sleep(Duration::from_millis(300));
    }
}

fn show_log_tail(paths: &DataPaths, lines: usize) {
    if let Ok(text) = std::fs::read_to_string(state::log_path(&paths.instance_dir)) {
        for line in state::last_lines(&text, lines) {
            eprintln!("  {line}");
        }
    }
}

fn cmd_stop(paths: &DataPaths, force: bool) -> i32 {
    let Some(record) = state::read(&paths.instance_dir) else {
        println!("AgentMux is not running.");
        return 0;
    };
    if !process::is_server(record.pid) {
        // The record outlived its server (a crash, a reboot), and the id may be another program's now: nothing is signalled.
        state::remove(&paths.instance_dir);
        println!("AgentMux is not running (cleared a record left by a server that is gone).");
        return 0;
    }
    // A service restarts a server that exits non-zero, so a service's server is stopped through the service.
    let stopped = match user_home().ok().filter(|h| service::installed(h, &paths.channel)) {
        Some(home) => service::control(&home, &paths.channel, "stop"),
        None if process::terminate(record.pid) => Ok(()),
        None => Err(format!("could not signal the server (pid {})", record.pid)),
    };
    if let Err(e) = stopped {
        return fail(&e);
    }
    if !wait_gone(record.pid, STOP_WAIT) {
        if !force {
            warn!("the server has not left after {} seconds. `agentmux stop --force` ends it at once.", STOP_WAIT.as_secs());
            return 3;
        }
        process::kill(record.pid);
        if !wait_gone(record.pid, Duration::from_secs(5)) {
            return fail("the server did not end");
        }
    }
    state::remove(&paths.instance_dir);
    println!("AgentMux stopped.");
    0
}

fn wait_gone(pid: u32, limit: Duration) -> bool {
    let begun = Instant::now();
    while process::is_server(pid) {
        if begun.elapsed() > limit {
            return false;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    true
}

fn cmd_status(paths: &DataPaths, json: bool) -> i32 {
    let record = running(paths);
    let healthy = record.as_ref().is_some_and(|s| process::health_ok(s.web_port));
    let uptime = record.as_ref().map(|s| agentmux_common::time::now_secs_u64().saturating_sub(s.started_unix));
    if json {
        println!(
            "{}",
            serde_json::json!({
                "running": record.is_some(),
                "answering": healthy,
                "channel": paths.channel,
                "pid": record.as_ref().map(|s| s.pid),
                "version": record.as_ref().map(|s| s.version.clone()),
                "installed_version": VERSION,
                "url": record.as_ref().map(ServerState::url),
                "uptime_secs": uptime,
                "data_dir": record.as_ref().map(|s| s.data_dir.clone()),
                "service": user_home().is_ok_and(|h| service::installed(&h, &paths.channel)),
            })
        );
    } else if let Some(s) = &record {
        println!("AgentMux {} is {} (pid {}, channel {}, up {})", s.version, if healthy { "running" } else { "running but not answering yet" }, s.pid, s.channel, human_duration(uptime.unwrap_or(0)));
        println!("  open:  {}", s.url());
        println!("  data:  {}", s.data_dir.display());
        if s.version != VERSION {
            tell!("  installed version is {VERSION}: `agentmux restart` runs it");
        }
    } else {
        tell!("AgentMux is not running (channel {}). Start it with `agentmux start`.", paths.channel);
    }
    if healthy { 0 } else { 1 }
}

fn human_duration(secs: u64) -> String {
    match secs {
        s if s < 90 => format!("{s}s"),
        s if s < 90 * 60 => format!("{}m", s / 60),
        s if s < 48 * 3600 => format!("{}h", s / 3600),
        s => format!("{}d", s / 86400),
    }
}

fn user_home() -> Result<PathBuf, String> {
    dirs::home_dir().ok_or_else(|| "cannot find your home folder".to_string())
}

/// The path a service should run this command by: the one it was started as, when that is a link
/// to this program (an install's `current` link survives an update), else its resolved path.
fn stable_exe() -> Result<PathBuf, String> {
    let resolved = std::env::current_exe().and_then(|p| p.canonicalize()).map_err(|e| format!("cannot find this command: {e}"))?;
    let as_run = std::env::args_os().next().map(PathBuf::from).and_then(|argv0| {
        if argv0.is_absolute() {
            Some(argv0)
        } else if argv0.components().count() == 1 {
            // Run by name: the entry of the PATH that holds it.
            std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()).map(|dir| dir.join(&argv0)).find(|p| p.is_file())
        } else {
            None
        }
    });
    Ok(as_run.filter(|p| p.canonicalize().is_ok_and(|c| c == resolved)).unwrap_or(resolved))
}

/// What the doctor judges; `srv_flag` and `web_dir_flag` are the places the person named, when they did.
fn facts(paths: &DataPaths, port: u16, srv_flag: Option<&Path>, web_dir_flag: Option<&Path>) -> doctor::Facts {
    let bin_dir = exe_dir();
    let find = |f: fn(&Path, Option<&Path>) -> Result<PathBuf, String>, flag: Option<&Path>| bin_dir.clone().and_then(|d| f(&d, flag));
    let record = running(paths);
    let key_file = record.as_ref().map(|s| s.key_file.clone()).unwrap_or_else(|| paths.data_dir.join(KEY_FILE_NAME));
    let exe = stable_exe().ok();
    let service_file = user_home().ok().and_then(|h| service::manager().map(|m| service::file_path(m, &h, &paths.channel)));
    let service_text = service_file.as_ref().and_then(|f| std::fs::read_to_string(f).ok());
    doctor::Facts {
        os: std::env::consts::OS,
        arch: std::env::consts::ARCH,
        srv: find(find_srv_in, srv_flag),
        web: find(find_web_in, web_dir_flag),
        folders: paths.ensure_dirs().map_err(|e| format!("cannot create the channel's folders: {e}")).and_then(|()| doctor::writable(&paths.instance_dir)),
        free_bytes: agentmux_common::disk_space::volume_free_total_bytes(&paths.instance_dir).map(|(free, _)| free),
        key_mode: doctor::mode_of(&key_file),
        port,
        answering: record.as_ref().is_some_and(|s| process::health_ok(s.web_port)),
        port_free: record.is_none().then(|| doctor::ports_free(port)),
        running: record,
        tools: ["node", "npm", "git"].into_iter().map(|t| (t, doctor::on_path(t))).collect(),
        installed_version: VERSION,
        service_installed: service_text.is_some(),
        service_names_exe: service_text.zip(exe).map(|(text, exe)| service::file_names_exe(&text, &exe)),
    }
}

fn cmd_doctor(paths: &DataPaths, json: bool) -> i32 {
    let port = match settings::read(&paths.instance_dir) {
        Ok(s) => s.port,
        Err(e) => return fail(&e),
    };
    let checks = doctor::evaluate(&facts(paths, port, None, None));
    if json {
        println!("{}", serde_json::json!({ "ok": !doctor::failed(&checks), "checks": checks }));
    } else {
        for line in doctor::lines(&checks) {
            println!("{line}");
        }
    }
    if doctor::failed(&checks) { 1 } else { 0 }
}

fn cmd_setup(paths: &DataPaths, args: &setup::SetupArgs) -> i32 {
    let current = match settings::read(&paths.instance_dir) {
        Ok(s) => s,
        Err(e) => return fail(&e),
    };
    let checks = doctor::evaluate(&facts(paths, args.port.unwrap_or(current.port), args.srv_bin.as_deref(), args.web_dir.as_deref()));
    for line in doctor::lines(&checks) {
        println!("{line}");
    }
    if doctor::failed(&checks) {
        warn!("fix the failures above, then run `agentmux setup` again.");
        return 1;
    }
    let plan = match setup::plan(args, &current, service::manager().is_some(), &mut setup::Terminal) {
        Ok(p) => p,
        Err(e) => {
            warn!("setup: {e}");
            return 2;
        }
    };
    // The server stops first, so the new choices and the service take effect at its next start.
    if plan.start {
        let code = cmd_stop(paths, false);
        if code != 0 {
            return code;
        }
    }
    if let Err(e) = settings::write(&paths.instance_dir, &plan.settings) {
        return fail(&format!("cannot write the settings: {e}"));
    }
    println!("Port {}, secrets in the {} store.", plan.settings.port, plan.settings.secret_store.flag());
    let home = match user_home() {
        Ok(h) => h,
        Err(e) => return fail(&e),
    };
    if plan.service {
        let installed = stable_exe().and_then(|exe| service::install(&home, &exe, &paths.channel, &state::log_path(&paths.instance_dir), &std::env::var("PATH").unwrap_or_default()));
        match installed {
            Ok(file) => {
                println!("The service is installed ({}): AgentMux starts at every login.", file.display());
                if cfg!(target_os = "linux") {
                    println!("To keep it running with no one logged in: loginctl enable-linger $USER");
                }
            }
            Err(e) => return fail(&e),
        }
    } else if let Err(e) = service::remove(&home, &paths.channel) {
        return fail(&e);
    }
    if plan.start {
        let start = setup_server_args(args);
        let code = cmd_start(paths, &start);
        if code != 0 {
            return code;
        }
        if plan.open {
            return cmd_client(paths, "open", &[]);
        }
    }
    tell!("Sign in with `agentmux open`. Models sign in where their command-line tools do: run the tool (for example `claude`) in a terminal pane once.");
    println!("Agents, tasks and Global Memory of this server live in {}.", paths.instance_dir.display());
    0
}

/// The flags of `setup` that name where the server's parts are.
fn setup_server_args(args: &setup::SetupArgs) -> ServerArgs {
    ServerArgs { srv_bin: args.srv_bin.clone(), web_dir: args.web_dir.clone(), ..ServerArgs::default() }
}

/// `agentmux-srv open` or `logout`, run for the server of this channel with the same terminal.
fn cmd_client(paths: &DataPaths, name: &str, extra: &[String]) -> i32 {
    let Some(record) = running(paths) else {
        warn!("the server is not running. Start it with `agentmux start`.");
        return 1;
    };
    let srv = if record.srv.is_file() { record.srv.clone() } else { exe_dir().and_then(|d| find_srv_in(&d, None)).unwrap_or(record.srv.clone()) };
    let status = Command::new(&srv)
        .arg(name)
        .args(["--port", &record.web_port.to_string(), "--key-file"])
        .arg(&record.key_file)
        .args(extra)
        .status();
    match status {
        Ok(s) => s.code().unwrap_or(3),
        Err(e) => fail(&format!("cannot run {}: {e}", srv.display())),
    }
}

fn cmd_logs(paths: &DataPaths, lines: usize, follow: bool) -> i32 {
    use std::io::{Read, Seek, SeekFrom, Write};
    let path = state::log_path(&paths.instance_dir);
    let Ok(text) = std::fs::read_to_string(&path) else {
        warn!("no log yet at {}", path.display());
        return 1;
    };
    let mut out = std::io::stdout();
    for line in state::last_lines(&text, lines) {
        if writeln!(out, "{line}").is_err() {
            return 0;
        }
    }
    if !follow {
        return 0;
    }
    let mut offset = text.len() as u64;
    loop {
        std::thread::sleep(Duration::from_millis(500));
        let Ok(mut file) = std::fs::File::open(&path) else { continue };
        let len = file.metadata().map(|m| m.len()).unwrap_or(0);
        // A log moved aside and started over is read from its start.
        if len < offset {
            offset = 0;
        }
        if len == offset || file.seek(SeekFrom::Start(offset)).is_err() {
            continue;
        }
        let mut more = String::new();
        if file.take(len - offset).read_to_string(&mut more).is_err() {
            continue;
        }
        offset = len;
        if write!(out, "{more}").and_then(|()| out.flush()).is_err() {
            return 0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    static ENV: Mutex<()> = Mutex::new(());

    fn parse(args: &[&str]) -> Result<Cli, clap::Error> {
        Cli::try_parse_from([&["agentmux"], args].concat())
    }

    #[test]
    fn the_default_channel_is_headless_and_the_default_port_is_fixed() {
        let cli = parse(&["start"]).unwrap();
        assert_eq!(cli.channel, "headless");
        let Cmd::Start(args) = cli.command else { panic!("start") };
        assert_eq!(args.port, None, "the port is the one setup kept, else the default");
        assert_eq!(settings::DEFAULT_PORT, 7380);
    }

    #[test]
    fn a_port_outside_the_unprivileged_range_is_refused() {
        for bad in ["0", "80", "1023", "65535", "70000", "x"] {
            assert!(parse(&["start", "--port", bad]).is_err(), "{bad}");
        }
        for ok in ["1024", "7380", "65534"] {
            assert!(parse(&["start", "--port", ok]).is_ok(), "{ok}");
        }
    }

    #[test]
    fn the_channel_flag_works_before_or_after_the_command() {
        assert_eq!(parse(&["--channel", "work", "status"]).unwrap().channel, "work");
        assert_eq!(parse(&["status", "--channel", "work"]).unwrap().channel, "work");
    }

    #[test]
    fn every_command_of_the_package_parses() {
        for args in [
            vec!["run"],
            vec!["start", "--port", "7400"],
            vec!["stop", "--force"],
            vec!["restart"],
            vec!["status", "--json"],
            vec!["doctor", "--json"],
            vec!["setup", "--yes", "--port", "7380", "--secret-store", "file", "--service", "--no-open"],
            vec!["run", "--secret-store", "keychain", "--public-origin", "http://127.0.0.1:8080"],
            vec!["open", "--print"],
            vec!["open", "--origin", "http://127.0.0.1:8080"],
            vec!["logout"],
            vec!["logs", "--lines", "5", "-f"],
            vec!["version", "--json"],
        ] {
            assert!(parse(&args).is_ok(), "{args:?}");
        }
        assert!(parse(&[]).is_err(), "a command is required");
        assert!(parse(&["nonsense"]).is_err());
    }

    /// The server's channel is its own, whatever the shell this runs in carries: a terminal inside a
    /// desktop AgentMux has that instance's channel and folders in its environment.
    #[test]
    fn the_channel_folders_ignore_the_environment_of_the_shell() {
        let _lock = ENV.lock().unwrap_or_else(|e| e.into_inner());
        let home = tempfile::tempdir().unwrap();
        std::env::set_var("AGENTMUX_HOME_OVERRIDE", home.path());
        std::env::set_var("AGENTMUX_CHANNEL", "stable");
        std::env::set_var("AGENTMUX_DATA_DIR", "/the/desktops/data");
        std::env::set_var("AGENTMUX_AUTH_KEY", "a key of the desktop's");
        let paths = context("headless").unwrap();
        assert_eq!(paths.channel, "headless");
        assert_eq!(paths.instance_dir, home.path().join("channels").join("headless"));
        assert!(paths.data_dir.starts_with(home.path().join("channels").join("headless").join("versions")));
        assert!(std::env::var_os("AGENTMUX_DATA_DIR").is_none(), "the desktop's variables are dropped");
        assert!(std::env::var_os("AGENTMUX_AUTH_KEY").is_none());
        assert_eq!(context("work").unwrap().instance_dir, home.path().join("channels").join("work"));
        std::env::remove_var("AGENTMUX_HOME_OVERRIDE");
        std::env::remove_var("AGENTMUX_CHANNEL");
    }

    #[test]
    fn a_channel_name_that_would_leave_its_folder_is_refused() {
        let _lock = ENV.lock().unwrap_or_else(|e| e.into_inner());
        let home = tempfile::tempdir().unwrap();
        std::env::set_var("AGENTMUX_HOME_OVERRIDE", home.path());
        for bad in ["..", "a/b", "shared", "channels"] {
            assert!(context(bad).is_err(), "{bad}");
        }
        std::env::remove_var("AGENTMUX_HOME_OVERRIDE");
        std::env::remove_var("AGENTMUX_CHANNEL");
    }

    #[test]
    fn the_program_and_the_web_ui_are_found_beside_the_command_or_named() {
        let root = tempfile::tempdir().unwrap();
        let bin = root.path().join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        assert!(find_srv_in(&bin, None).is_err());
        assert!(find_web_in(&bin, None).is_err());
        let srv_name = if cfg!(windows) { "agentmux-srv.exe" } else { "agentmux-srv" };
        std::fs::write(bin.join(srv_name), b"").unwrap();
        std::fs::create_dir_all(root.path().join("web")).unwrap();
        std::fs::write(root.path().join("web").join("index.html"), b"<html>").unwrap();
        assert_eq!(find_srv_in(&bin, None).unwrap(), bin.join(srv_name).canonicalize().unwrap());
        assert!(find_web_in(&bin, None).is_ok(), "web is beside bin");
        let elsewhere = tempfile::tempdir().unwrap();
        assert!(find_web_in(&bin, Some(elsewhere.path())).is_err(), "a named folder without index.html is refused");
        assert!(find_srv_in(&bin, Some(&elsewhere.path().join("missing"))).is_err());
    }

    #[test]
    fn a_relative_program_or_web_folder_is_made_absolute() {
        let _lock = ENV.lock().unwrap_or_else(|e| e.into_inner());
        let root = tempfile::tempdir().unwrap();
        let srv_name = if cfg!(windows) { "agentmux-srv.exe" } else { "agentmux-srv" };
        std::fs::write(root.path().join(srv_name), b"").unwrap();
        std::fs::create_dir_all(root.path().join("web")).unwrap();
        std::fs::write(root.path().join("web").join("index.html"), b"<html>").unwrap();
        let before = std::env::current_dir().unwrap();
        std::env::set_current_dir(root.path()).unwrap();
        let srv = find_srv_in(Path::new("/nonexistent-bin"), Some(Path::new(srv_name)));
        let web = find_web_in(Path::new("/nonexistent-bin"), Some(Path::new("web")));
        std::env::set_current_dir(before).unwrap();
        assert!(srv.unwrap().is_absolute(), "a bare name is not looked up on PATH or left relative");
        assert!(web.unwrap().is_absolute());
    }

    #[test]
    fn a_restart_keeps_the_port_the_server_was_started_on_and_runs_the_installed_program() {
        let args = ServerArgs::default;
        let kept = restart_args(Some(record(1)), args());
        assert_eq!(kept.port, Some(record(1).web_port));
        assert!(kept.srv_bin.is_none(), "the program is the installed one, not the old version's folder");
        assert_eq!(restart_args(None, args()).port, None, "nothing running: the saved settings decide");
        let flagged = restart_args(Some(record(1)), ServerArgs { port: Some(7500), ..args() });
        assert_eq!(flagged.port, Some(7500), "a flag wins");
    }

    fn with_channel<T>(f: impl FnOnce(&DataPaths) -> T) -> T {
        let _lock = ENV.lock().unwrap_or_else(|e| e.into_inner());
        let home = tempfile::tempdir().unwrap();
        std::env::set_var("AGENTMUX_HOME_OVERRIDE", home.path());
        let paths = context("headless").unwrap();
        paths.ensure_dirs().unwrap();
        let out = f(&paths);
        std::env::remove_var("AGENTMUX_HOME_OVERRIDE");
        std::env::remove_var("AGENTMUX_CHANNEL");
        out
    }

    fn record(pid: u32) -> ServerState {
        ServerState {
            pid,
            version: VERSION.into(),
            channel: "headless".into(),
            web_port: 7380,
            ws_port: 7381,
            started_unix: agentmux_common::time::now_secs_u64(),
            key_file: "/k".into(),
            data_dir: "/d".into(),
            srv: "/s/agentmux-srv".into(),
        }
    }

    /// A record whose process is not the server is cleared and never signalled: this test process has
    /// the record's id, and is not `agentmux-srv`.
    #[test]
    fn stop_clears_a_record_whose_process_is_not_the_server() {
        with_channel(|paths| {
            state::write(&paths.instance_dir, &record(std::process::id())).unwrap();
            assert_eq!(cmd_stop(paths, false), 0);
            assert!(state::read(&paths.instance_dir).is_none());
            assert_eq!(cmd_stop(paths, false), 0, "stopping what is not running is done");
        });
    }

    #[test]
    fn status_says_not_running_without_a_server_and_exits_one() {
        with_channel(|paths| {
            assert_eq!(cmd_status(paths, false), 1);
            state::write(&paths.instance_dir, &record(std::process::id())).unwrap();
            assert_eq!(cmd_status(paths, true), 1, "a record of a process that is not the server is not a running server");
        });
    }

    #[test]
    fn open_and_logout_need_a_running_server() {
        with_channel(|paths| {
            assert_eq!(cmd_client(paths, "open", &[]), 1);
            assert_eq!(cmd_client(paths, "logout", &[]), 1);
        });
    }

    #[test]
    fn the_servers_own_folder_leads_the_path_once() {
        let sep = if cfg!(windows) { ";" } else { ":" };
        let existing = |dirs: &[&str]| Some(std::ffi::OsString::from(dirs.join(sep)));
        let got = path_with_dir(Some(Path::new("/i/bin")), existing(&["/usr/bin", "/i/bin", "/bin"]));
        assert_eq!(got.to_string_lossy(), ["/i/bin", "/usr/bin", "/bin"].join(sep));
        assert_eq!(path_with_dir(Some(Path::new("/i/bin")), None).to_string_lossy(), "/i/bin");
        assert_eq!(path_with_dir(None, existing(&["/usr/bin"])).to_string_lossy(), "/usr/bin");
    }

    #[test]
    fn durations_are_short() {
        assert_eq!(human_duration(5), "5s");
        assert_eq!(human_duration(600), "10m");
        assert_eq!(human_duration(3 * 3600), "3h");
        assert_eq!(human_duration(5 * 86400), "5d");
    }

    #[test]
    fn the_sign_in_client_gets_only_what_it_was_asked_to_carry() {
        assert!(client_args(false, None).is_empty());
        assert_eq!(client_args(true, Some("http://127.0.0.1:8080".into())), vec!["--print", "--origin", "http://127.0.0.1:8080"]);
    }
}
