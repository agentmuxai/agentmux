// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `agentmux service install / uninstall / start / stop / status`: AgentMux Headless as a Windows service that starts when the
//! computer does (docs/specs/SPEC_WINDOWS_HEADLESS_AS_A_SERVICE_2026_11_20.md, D1 and D6). A person types these in a normal
//! terminal; what needs an administrator runs as a step of its own (`service apply`, hidden) that Windows asks the person to
//! allow. The step is the installed copy for everything but the install, and for the install this same program, read, hashed and
//! held first so that what runs elevated is what was checked (B.1; `winsvc_apply`).
//!
//! This file is the command line, what the person's own command does, and `status`. The decisions are in `winsvc_plan`, the
//! package in `winsvc_files`, the Windows calls in `winsvc_win` and the administrator step in `winsvc_apply`.

#![cfg_attr(not(windows), allow(dead_code))]

use clap::{Args, Subcommand, ValueEnum};

#[cfg(windows)]
use super::winsvc_plan as plan;

#[derive(Subcommand, Debug)]
pub(crate) enum ServiceCmd {
    /// Install AgentMux Headless as a Windows service: it starts when the computer does, before anyone signs in, and keeps
    /// running after you sign out. Windows asks you to allow it (an administrator step).
    Install(InstallArgs),
    /// Stop the service and remove it. Its program folder and data stay; `--purge` deletes the data too.
    Uninstall {
        /// Also delete the service's data folder: its agents' data, keys and the browsers' sign-ins.
        #[arg(long)]
        purge: bool,
        /// Do not ask before `--purge` deletes.
        #[arg(long)]
        yes: bool,
    },
    /// Start the installed service.
    Start,
    /// Stop the installed service.
    Stop,
    /// What the installed service is doing.
    Status {
        #[arg(long)]
        json: bool,
    },
    /// Be the Windows service: report to the service manager and run the server as `run` does.
    #[command(hide = true)]
    Run(crate::ServerArgs),
    /// The step of the others that needs an administrator (not a command to type).
    #[command(hide = true)]
    Apply(ApplyArgs),
}

#[derive(Args, Debug)]
pub(crate) struct InstallArgs {
    /// Install although this computer belongs to a domain (the service's account reaches the network as the computer).
    #[arg(long)]
    pub allow_domain_joined: bool,
    /// The web port the service listens on (the websocket port is the next one). Default 7380.
    #[arg(long, value_parser = clap::value_parser!(u16).range(1024..=65534))]
    pub port: Option<u16>,
}

/// What the administrator step is asked to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub(crate) enum Action {
    Install,
    Uninstall,
    Start,
    Stop,
}

#[derive(Args, Debug)]
pub(crate) struct ApplyArgs {
    #[arg(value_enum)]
    pub action: Action,
    /// The digest of the package the person's command checked (install).
    #[arg(long)]
    pub digest: Option<String>,
    /// The person who may read the server's key (install): their SID.
    #[arg(long)]
    pub reader: Option<String>,
    #[arg(long)]
    pub allow_domain_joined: bool,
    #[arg(long)]
    pub purge: bool,
    #[arg(long, value_parser = clap::value_parser!(u16).range(1024..=65534))]
    pub port: Option<u16>,
    /// Wait for Enter before the window closes, when something went wrong.
    #[arg(long)]
    pub pause: bool,
}

/// The arguments the person's command gives the administrator step, after the program's own name.
pub(crate) fn apply_command(channel: &str, a: &ApplyArgs) -> Vec<String> {
    let mut args: Vec<String> = ["--channel", channel, "service", "apply"].iter().map(|s| s.to_string()).collect();
    args.push(
        match a.action {
            Action::Install => "install",
            Action::Uninstall => "uninstall",
            Action::Start => "start",
            Action::Stop => "stop",
        }
        .to_string(),
    );
    for (flag, value) in [("--digest", &a.digest), ("--reader", &a.reader)] {
        if let Some(value) = value {
            args.extend([flag.to_string(), value.clone()]);
        }
    }
    if let Some(port) = a.port {
        args.extend(["--port".to_string(), port.to_string()]);
    }
    for (flag, on) in [("--allow-domain-joined", a.allow_domain_joined), ("--purge", a.purge), ("--pause", a.pause)] {
        if on {
            args.push(flag.to_string());
        }
    }
    args
}

/// `status` for a service in `state` (a word), as text.
pub(crate) fn status_text(name: &str, state: &str, pid: Option<u32>, account: &str, command: &str) -> String {
    let running = pid.map(|p| format!(", pid {p}")).unwrap_or_default();
    format!("service: {name} ({state}{running})\n  account: {account}\n  command: {command}")
}

impl ServiceCmd {
    /// Is this the service entry, which runs the server (and so needs the channel's data folders)?
    pub(crate) fn runs_the_server(&self) -> bool {
        matches!(self, ServiceCmd::Run(_))
    }
}

pub(crate) fn dispatch(command: ServiceCmd, paths: &crate::DataPaths) -> i32 {
    match command {
        ServiceCmd::Run(args) => super::winservice::command(paths, &args),
        other => manage(other, &paths.channel),
    }
}

/// Where the person's own terminal finds a running Windows service of this channel: its server program, its web port and its key
/// file (the one file of its data the person may read). `None` when there is no such service.
#[cfg(windows)]
pub(crate) fn service_target(channel: &str) -> Option<(std::path::PathBuf, u16, std::path::PathBuf)> {
    use super::winsvc_win::{self as win, Known};
    if !win::exists(&super::winservice::service_name(channel)) {
        return None;
    }
    let layout = plan::layout(&win::known_folder(Known::ProgramFiles).ok()?, &win::known_folder(Known::ProgramData).ok()?, channel, "0");
    let version = std::fs::canonicalize(&layout.current).ok()?.file_name()?.to_string_lossy().into_owned();
    let key = plan::key_file(&layout.data_root, channel, &version);
    let port = std::fs::read_to_string(plan::port_file(&key)).ok()?.trim().parse::<u16>().ok()?;
    Some((layout.current.join("bin").join("agentmux-srv.exe"), port, key))
}

#[cfg(not(windows))]
pub(crate) fn service_target(_channel: &str) -> Option<(std::path::PathBuf, u16, std::path::PathBuf)> {
    None
}

/// What `status` says when no server of the person's own is running but a Windows service of the channel is installed.
#[cfg(windows)]
pub(crate) fn installed_note(channel: &str) -> Option<String> {
    let name = super::winservice::service_name(channel);
    super::winsvc_win::exists(&name).then(|| format!("AgentMux is not running as you (channel {channel}), but the Windows service {name} is installed: `agentmux service status` says how it is, `agentmux open` signs a browser in to it."))
}

#[cfg(not(windows))]
pub(crate) fn installed_note(_channel: &str) -> Option<String> {
    None
}

#[cfg(not(windows))]
pub(crate) fn manage(_command: ServiceCmd, _channel: &str) -> i32 {
    crate::fail("a Windows service is for Windows. On Linux and macOS, `agentmux setup` starts AgentMux Headless at login")
}

#[cfg(windows)]
pub(crate) fn manage(command: ServiceCmd, channel: &str) -> i32 {
    if !super::winservice::channel_is_valid(channel) {
        return crate::fail(&format!("channel {channel:?} cannot be the name of a Windows service (letters, digits, - _ . only)"));
    }
    match command {
        ServiceCmd::Install(a) => front::install(channel, &a),
        ServiceCmd::Uninstall { purge, yes } => front::uninstall(channel, purge, yes),
        ServiceCmd::Start => front::relay_installed(channel, Action::Start, false),
        ServiceCmd::Stop => front::relay_installed(channel, Action::Stop, false),
        ServiceCmd::Status { json } => front::status(channel, json),
        ServiceCmd::Apply(a) => super::winsvc_apply::run(channel, &a),
        ServiceCmd::Run(_) => unreachable!("answered by dispatch"),
    }
}

#[cfg(windows)]
mod front {
    use std::io::IsTerminal;

    use agentmux_common::owner_only;
    use windows_service::service::ServiceAccess;

    use super::super::winsvc_apply as apply;
    use super::super::winsvc_files as files;
    use super::super::winsvc_plan as plan;
    use super::super::winsvc_win::{self as win, Known};
    use super::super::winservice;
    use super::{apply_command, status_text, Action, ApplyArgs, InstallArgs};

    fn nothing(action: Action) -> ApplyArgs {
        ApplyArgs { action, digest: None, reader: None, allow_domain_joined: false, purge: false, port: None, pause: false }
    }

    /// The program's own package: the folder above its `bin`.
    fn own_package() -> Result<std::path::PathBuf, String> {
        let exe = std::env::current_exe().and_then(|p| p.canonicalize()).map_err(|e| format!("cannot find this program: {e}"))?;
        exe.parent().and_then(std::path::Path::parent).map(std::path::Path::to_path_buf).ok_or_else(|| "cannot find the package this program is in".to_string())
    }

    pub fn install(channel: &str, a: &InstallArgs) -> i32 {
        if !a.allow_domain_joined && win::domain_joined() {
            return crate::fail(plan::DOMAIN_JOINED);
        }
        // Read and hold the package now, and say its digest to the administrator step, which reads it again and must find the same.
        let package = match own_package().and_then(|root| files::read_package(&root, true)) {
            Ok(p) => p,
            Err(e) => return crate::fail(&e),
        };
        let step = ApplyArgs { digest: Some(package.digest()), reader: owner_only::user_sid(), allow_domain_joined: a.allow_domain_joined, port: a.port, pause: true, ..nothing(Action::Install) };
        let code = if owner_only::is_elevated() {
            apply::run(channel, &ApplyArgs { pause: false, ..step })
        } else {
            let exe = match std::env::current_exe().and_then(|p| p.canonicalize()) {
                Ok(e) => e,
                Err(e) => return crate::fail(&format!("cannot find this program: {e}")),
            };
            elevated(channel, &exe, &step)
        };
        drop(package);
        code
    }

    pub fn uninstall(channel: &str, purge: bool, yes: bool) -> i32 {
        if !win::exists(&winservice::service_name(channel)) {
            println!("The service {} is not installed.", winservice::service_name(channel));
            return 1;
        }
        if purge && !yes && !confirm_purge(channel) {
            println!("Nothing was removed.");
            return 1;
        }
        relay_installed(channel, Action::Uninstall, purge)
    }

    /// Ask in the terminal before the service's data is deleted. No terminal, no answer: it is not deleted.
    fn confirm_purge(channel: &str) -> bool {
        let data = match (win::known_folder(Known::ProgramFiles), win::known_folder(Known::ProgramData)) {
            (Ok(f), Ok(d)) => plan::layout(&f, &d, channel, "0").data_root,
            _ => return false,
        };
        if !std::io::stdin().is_terminal() {
            eprintln!("agentmux: --purge deletes {}: give --yes to do it without being asked", data.display());
            return false;
        }
        println!("This deletes {}: the service's agents' data, its keys and its browsers' sign-ins. It cannot be undone.", data.display());
        print!("Delete it? [y/N] ");
        let _ = std::io::Write::flush(&mut std::io::stdout());
        let mut answer = String::new();
        let _ = std::io::stdin().read_line(&mut answer);
        matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes")
    }

    /// Run `action` as an administrator, by the installed copy (only administrators can change it).
    pub fn relay_installed(channel: &str, action: Action, purge: bool) -> i32 {
        let step = ApplyArgs { purge, pause: true, ..nothing(action) };
        if !win::exists(&winservice::service_name(channel)) {
            println!("The service {} is not installed.", winservice::service_name(channel));
            return 1;
        }
        let (files_dir, data_dir) = match (win::known_folder(Known::ProgramFiles), win::known_folder(Known::ProgramData)) {
            (Ok(f), Ok(d)) => (f, d),
            (Err(e), _) | (_, Err(e)) => return crate::fail(&e),
        };
        let exe = plan::layout(&files_dir, &data_dir, channel, "0").exe;
        let installed_here = std::env::current_exe().and_then(|p| p.canonicalize()).ok().zip(exe.parent().and_then(|p| p.canonicalize().ok())).is_some_and(|(me, dir)| me.starts_with(dir));
        if owner_only::is_elevated() && installed_here {
            return apply::run(channel, &ApplyArgs { pause: false, ..step });
        }
        if !exe.is_file() {
            return crate::fail(&format!("the installed copy {} is missing, and this does not run anything else as an administrator: {}", exe.display(), plan::NEEDS_ADMINISTRATOR));
        }
        elevated(channel, &exe, &step)
    }

    /// Run the administrator step by `exe`, and print what it wrote to its log.
    fn elevated(channel: &str, exe: &std::path::Path, step: &ApplyArgs) -> i32 {
        println!("Windows will ask you to allow an administrator step.");
        let code = match win::run_elevated(exe, &apply_command(channel, step)) {
            Ok(code) => code as i32,
            Err(e) => return crate::fail(&e),
        };
        if let Ok(text) = apply::log_path_of(channel).and_then(|log| std::fs::read_to_string(log).map_err(|e| e.to_string())) {
            print!("{text}");
        }
        code
    }

    pub fn status(channel: &str, json: bool) -> i32 {
        let name = winservice::service_name(channel);
        let service = match win::open(&name, ServiceAccess::QUERY_STATUS | ServiceAccess::QUERY_CONFIG) {
            Ok(s) => s,
            Err(_) => {
                println!("{}", if json { format!("{{\"service\":\"{name}\",\"installed\":false}}") } else { format!("service: {name} is not installed") });
                return 1;
            }
        };
        let (status, config) = match (service.query_status(), service.query_config()) {
            (Ok(s), Ok(c)) => (s, c),
            (Err(e), _) | (_, Err(e)) => return crate::fail(&format!("cannot ask the service how it is: {e}")),
        };
        let state = win::state_word(status.current_state);
        let account = config.account_name.map(|a| a.to_string_lossy().into_owned()).unwrap_or_default();
        let command = config.executable_path.to_string_lossy().into_owned();
        if json {
            println!("{}", serde_json::json!({ "service": name, "installed": true, "state": state, "pid": status.process_id, "account": account, "command": command }));
        } else {
            println!("{}", status_text(&name, state, status.process_id, &account, &command));
        }
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[derive(Parser, Debug)]
    struct Probe {
        #[arg(long, default_value = "headless")]
        channel: String,
        #[command(subcommand)]
        command: Wrap,
    }

    #[derive(Subcommand, Debug)]
    enum Wrap {
        Service {
            #[command(subcommand)]
            command: ServiceCmd,
        },
    }

    fn parse(args: &[&str]) -> ServiceCmd {
        let all: Vec<&str> = ["agentmux"].iter().chain(args.iter()).copied().collect();
        match Probe::try_parse_from(all).unwrap().command {
            Wrap::Service { command } => command,
        }
    }

    #[test]
    fn the_five_commands_and_the_two_hidden_ones_parse() {
        assert!(matches!(parse(&["service", "install"]), ServiceCmd::Install(InstallArgs { allow_domain_joined: false, port: None })));
        assert!(matches!(parse(&["service", "install", "--allow-domain-joined"]), ServiceCmd::Install(InstallArgs { allow_domain_joined: true, port: None })));
        assert!(matches!(parse(&["service", "install", "--port", "7500"]), ServiceCmd::Install(InstallArgs { port: Some(7500), .. })));
        assert!(Probe::try_parse_from(["agentmux", "service", "install", "--port", "80"]).is_err(), "a port below 1024 is refused");
        assert!(matches!(parse(&["service", "uninstall"]), ServiceCmd::Uninstall { purge: false, yes: false }));
        assert!(matches!(parse(&["service", "uninstall", "--purge", "--yes"]), ServiceCmd::Uninstall { purge: true, yes: true }));
        assert!(matches!(parse(&["service", "start"]), ServiceCmd::Start));
        assert!(matches!(parse(&["service", "stop"]), ServiceCmd::Stop));
        assert!(matches!(parse(&["service", "status", "--json"]), ServiceCmd::Status { json: true }));
        assert!(matches!(parse(&["service", "run"]), ServiceCmd::Run(_)));
        assert!(matches!(parse(&["service", "apply", "stop"]), ServiceCmd::Apply(ApplyArgs { action: Action::Stop, .. })));
    }

    #[test]
    fn what_the_person_s_command_gives_the_administrator_step_parses_back_to_the_same_step() {
        let sent = ApplyArgs {
            action: Action::Install,
            digest: Some("ab".repeat(32)),
            reader: Some("S-1-5-21-1-2-3-1001".into()),
            allow_domain_joined: true,
            purge: false,
            port: Some(7500),
            pause: true,
        };
        let args = apply_command("test", &sent);
        assert_eq!(&args[..5], ["--channel", "test", "service", "apply", "install"]);
        let probe = Probe::try_parse_from(std::iter::once("agentmux".to_string()).chain(args)).unwrap();
        assert_eq!(probe.channel, "test");
        let Wrap::Service { command: ServiceCmd::Apply(got) } = probe.command else { panic!("not the step") };
        assert_eq!((got.action, got.digest, got.reader, got.allow_domain_joined, got.purge, got.port, got.pause), (Action::Install, sent.digest, sent.reader, true, false, Some(7500), true));
        // A step with nothing but its action gives nothing else.
        let bare = ApplyArgs { action: Action::Uninstall, digest: None, reader: None, allow_domain_joined: false, purge: true, port: None, pause: false };
        assert_eq!(apply_command("headless", &bare), ["--channel", "headless", "service", "apply", "uninstall", "--purge"]);
    }

    #[test]
    fn the_status_says_the_state_the_process_the_account_and_the_command() {
        let said = status_text("AgentMuxHeadless", "running", Some(4242), "NT SERVICE\\AgentMuxHeadless", "\"C:\\p\\agentmux.exe\" service run");
        assert_eq!(said, "service: AgentMuxHeadless (running, pid 4242)\n  account: NT SERVICE\\AgentMuxHeadless\n  command: \"C:\\p\\agentmux.exe\" service run");
        assert!(status_text("AgentMuxHeadless", "stopped", None, "a", "c").starts_with("service: AgentMuxHeadless (stopped)\n"));
    }
}
