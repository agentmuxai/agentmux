// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! AgentMux Headless as a Windows service (docs/specs/SPEC_WINDOWS_HEADLESS_AS_A_SERVICE_2026_11_20.md).
//!
//! `agentmux service run` is the program the Service Control Manager (SCM) starts. SCM waits for it to report
//! "running" within about 30 seconds and kills a program that never talks to it, so a plain console program cannot be
//! a service: this module is the part that talks. It does nothing of its own: what runs is the same server `agentmux
//! run` runs, started on a thread, and what a stop does is the same graceful stop `agentmux stop` does. The program
//! knows nothing of this file's callers beyond two functions, so the CLI's own code stays where it is.
//!
//! The pure parts (the names, the account, the command line SCM is given, the status SCM is told) are here for the
//! installer and for tests; only [`run`] needs Windows.

use std::path::Path;

/// The service's name for a channel: the product's, with the channel in it for any channel but the default. SCM names
/// are one word (no spaces, no slashes), and the virtual account is made from the name.
pub fn service_name(channel: &str) -> String {
    if channel == crate::DEFAULT_CHANNEL { "AgentMuxHeadless".to_string() } else { format!("AgentMuxHeadless-{channel}") }
}

/// What the Services list shows.
pub fn display_name(channel: &str) -> String {
    if channel == crate::DEFAULT_CHANNEL { "AgentMux Headless".to_string() } else { format!("AgentMux Headless ({channel})") }
}

/// The virtual account SCM runs the service as: `NT SERVICE\<name>`. It has no password, its own identity for access
/// lists, and is never an administrator.
pub fn virtual_account(channel: &str) -> String {
    format!("NT SERVICE\\{}", service_name(channel))
}

/// A channel name that can be part of a service's name: letters, digits, `-`, `_` and `.`.
pub fn channel_is_valid(channel: &str) -> bool {
    !channel.is_empty() && channel.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

/// One argument as the Windows command line takes it: in double quotes when it has a space or a quote, a quote
/// escaped, and the backslashes before a closing quote doubled (a path that ends in `\` would otherwise escape it).
pub fn quote_arg(arg: &str) -> String {
    if !arg.is_empty() && !arg.chars().any(|c| matches!(c, ' ' | '\t' | '"')) {
        return arg.to_string();
    }
    let mut out = String::from("\"");
    let mut backslashes = 0usize;
    for c in arg.chars() {
        match c {
            '\\' => backslashes += 1,
            '"' => {
                out.push_str(&"\\".repeat(backslashes * 2 + 1));
                out.push('"');
                backslashes = 0;
            }
            c => {
                out.push_str(&"\\".repeat(backslashes));
                out.push(c);
                backslashes = 0;
            }
        }
    }
    out.push_str(&"\\".repeat(backslashes * 2));
    out.push('"');
    out
}

/// The command line SCM runs: `<program> --channel <channel> [--home <dir>] service run`. The program is the copy in the
/// folder only administrators can write; the data root, when it is not the default, is the one `--home` names.
pub fn command_line(program: &Path, channel: &str, home: Option<&Path>) -> String {
    let mut line = quote_arg(&program.to_string_lossy());
    line.push_str(" --channel ");
    line.push_str(&quote_arg(channel));
    if let Some(home) = home {
        line.push_str(" --home ");
        line.push_str(&quote_arg(&home.to_string_lossy()));
    }
    line.push_str(" service run");
    line
}

/// How long the service may take to start and to stop before SCM calls it stuck, in seconds.
pub const START_WAIT_SECS: u64 = 25;
pub const STOP_WAIT_SECS: u64 = 25;

/// What the service reports to SCM for the state of the server it hosts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    /// The server is being started.
    Starting,
    /// It is running and answering.
    Running,
    /// A stop was asked for and is being carried out.
    Stopping,
    /// The server ended; the code is what it exited with (0 is a clean stop).
    Stopped(i32),
}

/// What happens next, given what was seen: used by [`run`] and tested without SCM.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Next {
    /// Keep waiting.
    Wait,
    /// Report this.
    Report(Phase),
}

/// The decision of one look at the hosted server while it starts: `server_ended` is `Some(code)` when its thread has
/// returned, `answering` is whether it answers on its address, `waited` how long the start has taken so far.
pub fn while_starting(server_ended: Option<i32>, answering: bool, waited_secs: u64) -> Next {
    match server_ended {
        Some(code) => Next::Report(Phase::Stopped(if code == 0 { 1 } else { code })),
        None if answering => Next::Report(Phase::Running),
        // Past the wait SCM was told: report running anyway and let the server go on starting. A slow first start
        // (a migration of a large data folder) must not be the reason SCM kills a service that is working.
        None if waited_secs >= START_WAIT_SECS => Next::Report(Phase::Running),
        None => Next::Wait,
    }
}

#[cfg(windows)]
pub use imp::run;

#[cfg(windows)]
mod imp {
    use std::ffi::OsString;
    use std::sync::{mpsc, Mutex, OnceLock};
    use std::time::{Duration, Instant};

    use windows_service::service::{
        ServiceControl, ServiceControlAccept, ServiceExitCode, ServiceState, ServiceStatus, ServiceType,
    };
    use windows_service::service_control_handler::{self, ServiceControlHandlerResult};
    use windows_service::{define_windows_service, service_dispatcher};

    use super::{while_starting, Next, Phase, START_WAIT_SECS, STOP_WAIT_SECS};

    /// What the service hosts: the server's run (it blocks until the server ends, and gives its exit code), how to ask
    /// it to stop, and whether it answers yet. Set once by [`run`], before SCM calls the service's main.
    struct Host {
        name: String,
        serve: Box<dyn FnOnce() -> i32 + Send>,
        stop: Box<dyn Fn() + Send + Sync>,
        answering: Box<dyn Fn() -> bool + Send + Sync>,
    }

    static HOST: OnceLock<Mutex<Option<Host>>> = OnceLock::new();

    define_windows_service!(ffi_service_main, service_main);

    /// Hand this process to SCM as the service `name`. Returns the code the process should exit with: 0 after a stop SCM
    /// asked for, the server's own code when it ended by itself, 1 when SCM did not start this process as a service
    /// (it was run from a terminal: the service is not a command to type).
    pub fn run(
        name: &str,
        serve: impl FnOnce() -> i32 + Send + 'static,
        stop: impl Fn() + Send + Sync + 'static,
        answering: impl Fn() -> bool + Send + Sync + 'static,
    ) -> i32 {
        let host = Host { name: name.to_string(), serve: Box::new(serve), stop: Box::new(stop), answering: Box::new(answering) };
        *HOST.get_or_init(Default::default).lock().unwrap_or_else(|e| e.into_inner()) = Some(host);
        match service_dispatcher::start(name, ffi_service_main) {
            Ok(()) => 0,
            Err(e) => {
                eprintln!("agentmux service run: this is the program the Windows service manager starts, not a command to type: {e}");
                1
            }
        }
    }

    fn status(state: ServiceState, accept: ServiceControlAccept, exit: ServiceExitCode, wait: Duration) -> ServiceStatus {
        ServiceStatus {
            service_type: ServiceType::OWN_PROCESS,
            current_state: state,
            controls_accepted: accept,
            exit_code: exit,
            checkpoint: 0,
            wait_hint: wait,
            process_id: None,
        }
    }

    fn service_main(_args: Vec<OsString>) {
        let Some(host) = HOST.get().and_then(|h| h.lock().unwrap_or_else(|e| e.into_inner()).take()) else { return };
        let (stop_tx, stop_rx) = mpsc::channel::<()>();
        let handler = move |control| match control {
            ServiceControl::Stop | ServiceControl::Shutdown => {
                let _ = stop_tx.send(());
                ServiceControlHandlerResult::NoError
            }
            ServiceControl::Interrogate => ServiceControlHandlerResult::NoError,
            _ => ServiceControlHandlerResult::NotImplemented,
        };
        let Ok(handle) = service_control_handler::register(&host.name, handler) else { return };
        let none = ServiceControlAccept::empty();
        let _ = handle.set_service_status(status(ServiceState::StartPending, none, ServiceExitCode::Win32(0), Duration::from_secs(START_WAIT_SECS + 5)));

        let (done_tx, done_rx) = mpsc::channel::<i32>();
        let serve = host.serve;
        std::thread::spawn(move || {
            let _ = done_tx.send(serve());
        });

        // Starting: look every half second until the server answers, ends, or the wait SCM was told is up.
        let started = Instant::now();
        let mut ended: Option<i32> = None;
        loop {
            if let Ok(code) = done_rx.try_recv() {
                ended = Some(code);
            }
            match while_starting(ended, (host.answering)(), started.elapsed().as_secs()) {
                Next::Wait => std::thread::sleep(Duration::from_millis(500)),
                Next::Report(Phase::Running) => {
                    let _ = handle.set_service_status(status(
                        ServiceState::Running,
                        ServiceControlAccept::STOP | ServiceControlAccept::SHUTDOWN,
                        ServiceExitCode::Win32(0),
                        Duration::default(),
                    ));
                    break;
                }
                Next::Report(Phase::Stopped(code)) => {
                    let _ = handle.set_service_status(status(ServiceState::Stopped, none, ServiceExitCode::ServiceSpecific(code as u32), Duration::default()));
                    return;
                }
                Next::Report(_) => break,
            }
        }

        // Running: until SCM asks for a stop or the server ends by itself.
        let code = loop {
            if let Ok(code) = done_rx.try_recv() {
                break code;
            }
            match stop_rx.recv_timeout(Duration::from_millis(500)) {
                Ok(()) => {
                    let _ = handle.set_service_status(status(ServiceState::StopPending, none, ServiceExitCode::Win32(0), Duration::from_secs(STOP_WAIT_SECS + 5)));
                    (host.stop)();
                    break done_rx.recv_timeout(Duration::from_secs(STOP_WAIT_SECS)).unwrap_or(0);
                }
                Err(mpsc::RecvTimeoutError::Timeout) => continue,
                Err(mpsc::RecvTimeoutError::Disconnected) => break done_rx.recv_timeout(Duration::from_secs(STOP_WAIT_SECS)).unwrap_or(0),
            }
        };
        let exit = if code == 0 { ServiceExitCode::Win32(0) } else { ServiceExitCode::ServiceSpecific(code as u32) };
        let _ = handle.set_service_status(status(ServiceState::Stopped, none, exit, Duration::default()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn the_service_is_named_from_the_channel_and_runs_as_its_own_virtual_account() {
        assert_eq!(service_name("headless"), "AgentMuxHeadless");
        assert_eq!(service_name("test"), "AgentMuxHeadless-test");
        assert_eq!(display_name("headless"), "AgentMux Headless");
        assert_eq!(virtual_account("headless"), "NT SERVICE\\AgentMuxHeadless");
        assert_eq!(virtual_account("test"), "NT SERVICE\\AgentMuxHeadless-test");
    }

    #[test]
    fn a_channel_that_cannot_be_part_of_a_service_name_is_refused() {
        for ok in ["headless", "a-b_c.d", "X1"] {
            assert!(channel_is_valid(ok), "{ok}");
        }
        for bad in ["", "a b", "a/b", "a\\b", "a\"b", "né"] {
            assert!(!channel_is_valid(bad), "{bad:?}");
        }
    }

    #[test]
    fn an_argument_is_quoted_as_the_windows_command_line_reads_it() {
        assert_eq!(quote_arg("headless"), "headless");
        assert_eq!(quote_arg(""), "\"\"");
        assert_eq!(quote_arg("C:\\Program Files\\AgentMux"), "\"C:\\Program Files\\AgentMux\"");
        // A trailing backslash would escape the closing quote: it is doubled.
        assert_eq!(quote_arg("C:\\Program Data\\"), "\"C:\\Program Data\\\\\"");
        assert_eq!(quote_arg("say \"hi\""), "\"say \\\"hi\\\"\"");
        // Backslashes before a quote are doubled, and the quote escaped.
        assert_eq!(quote_arg("a \\\"b"), "\"a \\\\\\\"b\"");
        // Backslashes elsewhere are left as they are.
        assert_eq!(quote_arg("a b\\c"), "\"a b\\c\"");
    }

    #[test]
    fn the_command_line_names_the_program_the_channel_and_the_data_root_once() {
        let program = PathBuf::from("C:\\Program Files\\AgentMux\\Headless\\current\\bin\\agentmux.exe");
        assert_eq!(
            command_line(&program, "headless", None),
            "\"C:\\Program Files\\AgentMux\\Headless\\current\\bin\\agentmux.exe\" --channel headless service run"
        );
        let home = PathBuf::from("C:\\ProgramData\\AgentMux\\Headless");
        assert_eq!(
            command_line(&program, "headless", Some(&home)),
            "\"C:\\Program Files\\AgentMux\\Headless\\current\\bin\\agentmux.exe\" --channel headless --home C:\\ProgramData\\AgentMux\\Headless service run"
        );
    }

    #[test]
    fn the_service_reports_running_when_the_server_answers_and_stopped_when_it_ends() {
        assert_eq!(while_starting(None, false, 1), Next::Wait);
        assert_eq!(while_starting(None, true, 1), Next::Report(Phase::Running));
        // A server that ended before it answered is a failed start, never a clean 0.
        assert_eq!(while_starting(Some(0), false, 2), Next::Report(Phase::Stopped(1)));
        assert_eq!(while_starting(Some(3), false, 2), Next::Report(Phase::Stopped(3)));
        // A slow start is not a reason for SCM to kill it.
        assert_eq!(while_starting(None, false, START_WAIT_SECS), Next::Report(Phase::Running));
    }
}
