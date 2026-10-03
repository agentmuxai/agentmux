// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! agentmux-remote: AgentMux's helper on an SSH host
//! (SPEC_REMOTE_TERMINALS_AND_DURABLE_SESSIONS_2026_10_02.md §6, §7).
//!
//! - `version`: its version and protocol, for the handshake.
//! - `daemon`: the per-user session daemon (`daemon.rs`).
//! - `attach --session ID [--offset N] [--cols C] [--rows R]`: attach to a
//!   durable session, starting the daemon and the session as needed.
//! - `end --session ID`, `list`: end a session; list them.
//! - `serve --stdio`: file operations for srv over stdio (`serve.rs`), on
//!   any platform.
//!
//! It holds no credentials and opens no network port: it talks only to its
//! own SSH channel and a per-user Unix socket. Its files live in
//! `~/.agentmux-remote` (or `$AGENTMUX_REMOTE_HOME`).

use agentmux_remote::frame;

#[cfg(unix)]
mod attach;
#[cfg(unix)]
mod daemon;
#[cfg(unix)]
mod pty;
use agentmux_remote::serve;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    std::process::exit(run(&args));
}

#[cfg(unix)]
fn flag<'a>(args: &'a [String], name: &str) -> Option<&'a str> {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1))
        .map(String::as_str)
}

#[cfg(unix)]
fn base_dir() -> std::path::PathBuf {
    if let Some(dir) = std::env::var_os("AGENTMUX_REMOTE_HOME").filter(|d| !d.is_empty()) {
        return dir.into();
    }
    let home = std::env::var_os("HOME").unwrap_or_else(|| ".".into());
    std::path::PathBuf::from(home).join(".agentmux-remote")
}

fn run(args: &[String]) -> i32 {
    match args.first().map(String::as_str) {
        Some("version") => {
            println!(
                "agentmux-remote {} protocol {}",
                env!("CARGO_PKG_VERSION"),
                frame::PROTOCOL
            );
            0
        }
        Some("serve") if args.get(1).map(String::as_str) == Some("--stdio") => {
            let (mut stdin, mut stdout) = (std::io::stdin().lock(), std::io::stdout().lock());
            match serve::run(&serve::home_dir(), &mut stdin, &mut stdout) {
                Ok(()) => 0,
                Err(e) => {
                    eprintln!("agentmux-remote serve: {e}");
                    1
                }
            }
        }
        #[cfg(unix)]
        Some("daemon") => match daemon::run(&base_dir()) {
            Ok(()) => 0,
            Err(e) => {
                eprintln!("agentmux-remote daemon: {e}");
                1
            }
        },
        #[cfg(unix)]
        Some("attach") => {
            let Some(session) = flag(args, "--session").filter(|s| daemon::valid_session_id(s))
            else {
                eprintln!(
                    "agentmux-remote attach: --session <id> is required (letters, digits, - and _)"
                );
                return 2;
            };
            // A size that does not fit falls back to the default, never wraps.
            let size = |name: &str, default: u16| {
                flag(args, name)
                    .and_then(|v| v.parse::<u16>().ok())
                    .unwrap_or(default)
            };
            let offset = match flag(args, "--offset").map(str::parse::<u64>) {
                None => 0,
                Some(Ok(o)) => o,
                Some(Err(_)) => {
                    eprintln!("agentmux-remote attach: --offset must be a number");
                    return 2;
                }
            };
            let (cols, rows) = (size("--cols", 80), size("--rows", 24));
            attach::run(&base_dir(), session, offset, cols, rows)
        }
        #[cfg(unix)]
        Some("end") => match flag(args, "--session").filter(|s| daemon::valid_session_id(s)) {
            Some(session) => attach::control(&base_dir(), &format!("END {session}")),
            None => 2,
        },
        #[cfg(unix)]
        Some("list") => attach::control(&base_dir(), "LIST"),
        #[cfg(not(unix))]
        Some("daemon" | "attach" | "end" | "list") => {
            eprintln!("agentmux-remote: durable sessions need a Linux or macOS host");
            1
        }
        _ => {
            eprintln!("usage: agentmux-remote version | serve --stdio | daemon | attach --session ID [--offset N] [--cols C] [--rows R] | end --session ID | list");
            2
        }
    }
}
