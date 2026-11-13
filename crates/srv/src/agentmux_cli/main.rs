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

mod process;
mod state;

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use agentmux_common::{DataPaths, RuntimeMode};
use clap::{Args, Parser, Subcommand};

use state::ServerState;

const VERSION: &str = env!("CARGO_PKG_VERSION");
const DEFAULT_CHANNEL: &str = "headless";
/// The server's web port when none is given; the websocket port is the next one.
const DEFAULT_PORT: u16 = 7380;
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
    /// The web port the browser opens (the websocket port is the next one).
    #[arg(long, default_value_t = DEFAULT_PORT, value_parser = clap::value_parser!(u16).range(1024..=65534))]
    port: u16,
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
        Cmd::Restart(args) => match cmd_stop(&paths, false) {
            0 => cmd_start(&paths, &args),
            code => code,
        },
        Cmd::Status { json } => cmd_status(&paths, json),
        Cmd::Open { print, origin } => cmd_client(&paths, "open", &client_args(print, origin)),
        Cmd::Logout => cmd_client(&paths, "logout", &[]),
        Cmd::Logs { lines, follow } => cmd_logs(&paths, lines, follow),
        Cmd::Version { .. } => unreachable!("answered above"),
    }
}

fn fail(message: &str) -> i32 {
    eprintln!("agentmux: {message}");
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

fn find_srv_in(bin_dir: &Path, flag: Option<&Path>) -> Result<PathBuf, String> {
    let path = flag.map(Path::to_path_buf).unwrap_or_else(|| bin_dir.join(if cfg!(windows) { "agentmux-srv.exe" } else { "agentmux-srv" }));
    if path.is_file() { Ok(path) } else { Err(format!("the server program is not at {} (--srv-bin names another)", path.display())) }
}

fn find_web_in(bin_dir: &Path, flag: Option<&Path>) -> Result<PathBuf, String> {
    let candidates: Vec<PathBuf> = match flag {
        Some(dir) => vec![dir.to_path_buf()],
        None => vec![bin_dir.join("..").join("web"), bin_dir.join("web")],
    };
    candidates
        .iter()
        .find(|dir| dir.join("index.html").is_file())
        .cloned()
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
        println!("agentmux {VERSION} (channel {channel})");
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
        eprintln!("agentmux: the server is already running (pid {}) at {}", other.pid, other.url());
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
    if let Err(code) = prepare_data(paths, &srv) {
        return code;
    }
    let record = ServerState {
        pid: std::process::id(),
        version: VERSION.to_string(),
        channel: paths.channel.clone(),
        web_port: args.port,
        ws_port: args.port + 1,
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
        .envs(paths.to_env_vars())
        .stdin(Stdio::null());
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
    cmd.args(["--channel", &paths.channel, "run", "--port", &args.port.to_string()]);
    if let Some(srv) = &args.srv_bin {
        cmd.arg("--srv-bin").arg(srv);
    }
    if let Some(web) = &args.web_dir {
        cmd.arg("--web-dir").arg(web);
    }
    cmd.stdin(Stdio::null()).stdout(out).stderr(err);
    process::detach(&mut cmd);
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => return fail(&format!("cannot start the server: {e}")),
    };
    let begun = Instant::now();
    loop {
        // The record of THIS start: a record left by an earlier server names another process.
        if let Some(s) = state::read(&paths.instance_dir).filter(|s| s.pid == child.id()) {
            if process::health_ok(s.web_port) {
                println!("AgentMux is running: {}", s.url());
                println!("Sign in with: agentmux open");
                return 0;
            }
        }
        if let Ok(Some(status)) = child.try_wait() {
            eprintln!("agentmux: the server stopped while starting ({status}). The end of its log:");
            show_log_tail(paths, 15);
            return 3;
        }
        if begun.elapsed() > START_WAIT {
            eprintln!("agentmux: the server has not answered after {} seconds; it is still starting. See `agentmux logs`.", START_WAIT.as_secs());
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
    if !process::terminate(record.pid) {
        return fail(&format!("could not signal the server (pid {})", record.pid));
    }
    if !wait_gone(record.pid, STOP_WAIT) {
        if !force {
            eprintln!("agentmux: the server has not left after {} seconds. `agentmux stop --force` ends it at once.", STOP_WAIT.as_secs());
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
            })
        );
    } else if let Some(s) = &record {
        println!("AgentMux {} is {} (pid {}, channel {}, up {})", s.version, if healthy { "running" } else { "running but not answering yet" }, s.pid, s.channel, human_duration(uptime.unwrap_or(0)));
        println!("  open:  {}", s.url());
        println!("  data:  {}", s.data_dir.display());
        if s.version != VERSION {
            println!("  installed version is {VERSION}: `agentmux restart` runs it");
        }
    } else {
        println!("AgentMux is not running (channel {}). Start it with `agentmux start`.", paths.channel);
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

/// `agentmux-srv open` or `logout`, run for the server of this channel with the same terminal.
fn cmd_client(paths: &DataPaths, name: &str, extra: &[String]) -> i32 {
    let Some(record) = running(paths) else {
        eprintln!("agentmux: the server is not running. Start it with `agentmux start`.");
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
        eprintln!("agentmux: no log yet at {}", path.display());
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
        assert_eq!(args.port, 7380);
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
        assert_eq!(find_srv_in(&bin, None).unwrap(), bin.join(srv_name));
        assert!(find_web_in(&bin, None).is_ok(), "web is beside bin");
        let elsewhere = tempfile::tempdir().unwrap();
        assert!(find_web_in(&bin, Some(elsewhere.path())).is_err(), "a named folder without index.html is refused");
        assert!(find_srv_in(&bin, Some(&elsewhere.path().join("missing"))).is_err());
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
