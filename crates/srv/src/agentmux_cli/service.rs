// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The server as a service of the person's own login: a systemd user unit on Linux, a launchd agent
//! on macOS. The unit runs `agentmux --channel <channel> run`, which reads the port and the secret
//! store from the channel's settings, so changing either is `agentmux setup` and a restart, never an
//! edit of a service file. Nothing here needs root: both live under the person's home.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::DEFAULT_CHANNEL;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Manager {
    Systemd,
    Launchd,
}

/// The service manager of this machine, when the first package supports it.
pub fn manager() -> Option<Manager> {
    if cfg!(target_os = "linux") {
        Some(Manager::Systemd)
    } else if cfg!(target_os = "macos") {
        Some(Manager::Launchd)
    } else {
        None
    }
}

/// The unit's name, with the channel in it for any channel but the default.
pub fn unit_name(channel: &str) -> String {
    if channel == DEFAULT_CHANNEL { "agentmux.service".to_string() } else { format!("agentmux-{channel}.service") }
}

/// The launchd label, with the channel in it for any channel but the default.
pub fn label(channel: &str) -> String {
    if channel == DEFAULT_CHANNEL { "ai.agentmux.server".to_string() } else { format!("ai.agentmux.server.{channel}") }
}

/// Where the unit or the agent's file lives, under the person's home.
pub fn file_path(manager: Manager, user_home: &Path, channel: &str) -> PathBuf {
    match manager {
        Manager::Systemd => user_home.join(".config").join("systemd").join("user").join(unit_name(channel)),
        Manager::Launchd => user_home.join("Library").join("LaunchAgents").join(format!("{}.plist", label(channel))),
    }
}

/// A value inside a systemd double-quoted word: backslash, quote and the specifier mark are escaped.
fn systemd_quote(value: &str) -> String {
    let mut out = String::from("\"");
    for c in value.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '%' => out.push_str("%%"),
            '\n' | '\r' => out.push(' '),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// The text of the systemd user unit.
pub fn render_unit(exe: &Path, channel: &str, log: &Path, path_env: &str) -> String {
    format!(
        "[Unit]\n\
         Description=AgentMux server (channel {channel})\n\
         After=network.target\n\
         \n\
         [Service]\n\
         Type=simple\n\
         ExecStart={exe} --channel {channel_arg} run\n\
         Environment={path}\n\
         Restart=on-failure\n\
         RestartSec=3\n\
         TimeoutStopSec=30\n\
         StandardOutput=append:{log}\n\
         StandardError=append:{log}\n\
         \n\
         [Install]\n\
         WantedBy=default.target\n",
        exe = systemd_quote(&exe.to_string_lossy()),
        channel_arg = systemd_quote(channel),
        path = systemd_quote(&format!("PATH={path_env}")),
        log = log.display().to_string().replace('%', "%%"),
    )
}

fn xml_escape(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// The text of the launchd agent.
pub fn render_plist(exe: &Path, channel: &str, log: &Path, path_env: &str) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
         <plist version=\"1.0\">\n\
         <dict>\n\
         \t<key>Label</key><string>{label}</string>\n\
         \t<key>ProgramArguments</key>\n\
         \t<array>\n\
         \t\t<string>{exe}</string>\n\
         \t\t<string>--channel</string>\n\
         \t\t<string>{channel}</string>\n\
         \t\t<string>run</string>\n\
         \t</array>\n\
         \t<key>EnvironmentVariables</key>\n\
         \t<dict><key>PATH</key><string>{path}</string></dict>\n\
         \t<key>RunAtLoad</key><true/>\n\
         \t<key>KeepAlive</key>\n\
         \t<dict><key>SuccessfulExit</key><false/></dict>\n\
         \t<key>StandardOutPath</key><string>{log}</string>\n\
         \t<key>StandardErrorPath</key><string>{log}</string>\n\
         </dict>\n\
         </plist>\n",
        label = xml_escape(&label(channel)),
        exe = xml_escape(&exe.to_string_lossy()),
        channel = xml_escape(channel),
        path = xml_escape(path_env),
        log = xml_escape(&log.to_string_lossy()),
    )
}

/// Does the unit or agent's file name this command's own program? (The install may have moved.)
pub fn file_names_exe(text: &str, exe: &Path) -> bool {
    let exe = exe.to_string_lossy();
    text.contains(exe.as_ref()) || text.contains(&xml_escape(&exe)) || text.contains(&systemd_quote(&exe))
}

/// Is a service of this channel installed for this person?
pub fn installed(user_home: &Path, channel: &str) -> bool {
    manager().is_some_and(|m| file_path(m, user_home, channel).is_file())
}

fn run_quiet(program: &str, args: &[&str]) -> Result<(), String> {
    let out = Command::new(program).args(args).stdin(Stdio::null()).output().map_err(|e| format!("cannot run {program}: {e}"))?;
    if out.status.success() {
        return Ok(());
    }
    let said = String::from_utf8_lossy(&out.stderr);
    Err(format!("`{program} {}` failed: {}", args.join(" "), said.trim()))
}

#[cfg(unix)]
fn uid() -> u32 {
    // SAFETY: getuid has no failure case and touches no memory.
    unsafe { libc::getuid() }
}

#[cfg(not(unix))]
fn uid() -> u32 {
    0
}

/// Write the service's file and have the manager run it from now on and at every login.
pub fn install(user_home: &Path, exe: &Path, channel: &str, log: &Path, path_env: &str) -> Result<PathBuf, String> {
    let manager = manager().ok_or_else(|| "a service is for Linux and macOS in this release".to_string())?;
    let path = file_path(manager, user_home, channel);
    let text = match manager {
        Manager::Systemd => render_unit(exe, channel, log, path_env),
        Manager::Launchd => render_plist(exe, channel, log, path_env),
    };
    std::fs::create_dir_all(path.parent().unwrap_or(user_home)).map_err(|e| format!("cannot create {}: {e}", path.parent().unwrap_or(user_home).display()))?;
    std::fs::write(&path, text).map_err(|e| format!("cannot write {}: {e}", path.display()))?;
    match manager {
        Manager::Systemd => {
            run_quiet("systemctl", &["--user", "daemon-reload"])?;
            run_quiet("systemctl", &["--user", "enable", &unit_name(channel)])?;
        }
        Manager::Launchd => {
            let domain = format!("gui/{}", uid());
            // A copy loaded before is replaced, not refused.
            let _ = run_quiet("launchctl", &["bootout", &format!("{domain}/{}", label(channel))]);
            run_quiet("launchctl", &["bootstrap", &domain, &path.to_string_lossy()])?;
        }
    }
    Ok(path)
}

/// Stop the service, take it out of the login, remove its file.
pub fn remove(user_home: &Path, channel: &str) -> Result<(), String> {
    let Some(manager) = manager() else { return Ok(()) };
    let path = file_path(manager, user_home, channel);
    if !path.is_file() {
        return Ok(());
    }
    match manager {
        Manager::Systemd => {
            let _ = run_quiet("systemctl", &["--user", "disable", "--now", &unit_name(channel)]);
        }
        Manager::Launchd => {
            let _ = run_quiet("launchctl", &["bootout", &format!("gui/{}/{}", uid(), label(channel))]);
        }
    }
    std::fs::remove_file(&path).map_err(|e| format!("cannot remove {}: {e}", path.display()))?;
    if manager == Manager::Systemd {
        let _ = run_quiet("systemctl", &["--user", "daemon-reload"]);
    }
    Ok(())
}

/// `start`, `stop` or `restart` through the service manager. On macOS a stop unloads the agent (a
/// SIGTERM would be answered by `KeepAlive` starting it again if the server exits non-zero), and a
/// start loads it again; the file stays, so the next login loads it.
pub fn control(user_home: &Path, channel: &str, action: &str) -> Result<(), String> {
    match manager() {
        Some(Manager::Systemd) => run_quiet("systemctl", &["--user", action, &unit_name(channel)]),
        Some(Manager::Launchd) => {
            let domain = format!("gui/{}", uid());
            let target = format!("{domain}/{}", label(channel));
            let path = file_path(Manager::Launchd, user_home, channel);
            let stop = || {
                // Not loaded is the state a stop asks for.
                let _ = run_quiet("launchctl", &["bootout", &target]);
            };
            let start = || {
                // Loaded already is fine: the kick below starts it if it is not running.
                let _ = run_quiet("launchctl", &["bootstrap", &domain, &path.to_string_lossy()]);
                run_quiet("launchctl", &["kickstart", &target])
            };
            match action {
                "stop" => {
                    stop();
                    Ok(())
                }
                "restart" => {
                    stop();
                    start()
                }
                _ => start(),
            }
        }
        None => Err("a service is for Linux and macOS in this release".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_channel_has_the_plain_names_and_another_has_its_own() {
        assert_eq!(unit_name("headless"), "agentmux.service");
        assert_eq!(unit_name("work"), "agentmux-work.service");
        assert_eq!(label("headless"), "ai.agentmux.server");
        assert_eq!(label("work"), "ai.agentmux.server.work");
        let home = Path::new("/home/u");
        assert_eq!(file_path(Manager::Systemd, home, "headless"), home.join(".config/systemd/user/agentmux.service"));
        assert_eq!(file_path(Manager::Launchd, home, "work"), home.join("Library/LaunchAgents/ai.agentmux.server.work.plist"));
    }

    #[test]
    fn the_unit_runs_the_command_for_its_channel_and_restarts_on_failure_only() {
        let text = render_unit(Path::new("/home/u/.agentmux/server/0.60.26/bin/agentmux"), "headless", Path::new("/home/u/.agentmux/channels/headless/server.log"), "/usr/local/bin:/usr/bin");
        assert!(text.contains("ExecStart=\"/home/u/.agentmux/server/0.60.26/bin/agentmux\" --channel \"headless\" run\n"), "{text}");
        assert!(text.contains("Environment=\"PATH=/usr/local/bin:/usr/bin\"\n"));
        assert!(text.contains("Restart=on-failure\n") && text.contains("WantedBy=default.target\n"));
        assert!(text.contains("StandardOutput=append:/home/u/.agentmux/channels/headless/server.log\n"));
        assert!(!text.contains("User=") && !text.contains("sudo"), "a user unit needs no user and no root");
    }

    #[test]
    fn a_path_with_a_space_a_quote_or_a_percent_cannot_break_the_unit() {
        let text = render_unit(Path::new("/home/my user/100%/agentmux"), "headless", Path::new("/l/x%y.log"), "/a b:/c\"d");
        assert!(text.contains("ExecStart=\"/home/my user/100%%/agentmux\" "), "{text}");
        assert!(text.contains("Environment=\"PATH=/a b:/c\\\"d\"\n"), "{text}");
        assert!(text.contains("append:/l/x%%y.log\n"), "{text}");
        assert!(!text.contains("\n\n\n"), "no stray line");
        let hostile = render_unit(Path::new("/x/agentmux"), "headless", Path::new("/l"), "/a\nExecStart=/bin/evil");
        let settings = hostile.lines().filter(|l| l.starts_with("ExecStart=")).count();
        assert_eq!(settings, 1, "a newline in the PATH starts no second setting: {hostile}");
    }

    #[test]
    fn the_agent_restarts_after_a_crash_but_not_after_a_clean_stop() {
        let text = render_plist(Path::new("/Users/u/.agentmux/server/bin/agentmux"), "headless", Path::new("/Users/u/.agentmux/channels/headless/server.log"), "/opt/homebrew/bin:/usr/bin");
        assert!(text.contains("<key>Label</key><string>ai.agentmux.server</string>"));
        assert!(text.contains("<string>/Users/u/.agentmux/server/bin/agentmux</string>\n\t\t<string>--channel</string>\n\t\t<string>headless</string>\n\t\t<string>run</string>"), "{text}");
        assert!(text.contains("<key>SuccessfulExit</key><false/>") && text.contains("<key>RunAtLoad</key><true/>"));
        assert!(text.starts_with("<?xml") && text.trim_end().ends_with("</plist>"));
    }

    #[test]
    fn the_agent_file_escapes_what_xml_would_read() {
        let text = render_plist(Path::new("/Users/a&b/<x>/agentmux"), "headless", Path::new("/l"), "/p\"q");
        assert!(text.contains("/Users/a&amp;b/&lt;x&gt;/agentmux") && text.contains("/p&quot;q"), "{text}");
        assert!(!text.contains("a&b"));
    }

    #[test]
    fn a_file_names_the_program_it_runs_in_either_form() {
        let exe = Path::new("/home/my user/agentmux");
        assert!(file_names_exe(&render_unit(exe, "headless", Path::new("/l"), "/p"), exe));
        assert!(file_names_exe(&render_plist(exe, "headless", Path::new("/l"), "/p"), exe));
        assert!(!file_names_exe(&render_unit(Path::new("/old/agentmux"), "headless", Path::new("/l"), "/p"), exe));
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    #[test]
    fn where_there_is_no_service_there_is_no_install() {
        assert!(manager().is_none());
        assert!(install(Path::new("/h"), Path::new("/x"), "headless", Path::new("/l"), "/p").is_err());
        assert!(control(Path::new("/h"), "headless", "start").is_err());
        assert!(!installed(Path::new("/h"), "headless"));
    }
}
