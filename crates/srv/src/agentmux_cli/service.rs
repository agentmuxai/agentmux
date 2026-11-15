// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The server as a service of the person's own login: a systemd user unit on Linux, a launchd agent
//! on macOS. The unit runs `agentmux --channel <channel> run`, which reads the port and the secret
//! store from the channel's settings, so changing either is `agentmux setup` and a restart, never an
//! edit of a service file. Nothing here needs root: both live under the person's home.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use agentmux_common::win32::NoWindow;

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

/// The system folders a service's programs and the agents it starts are looked up in, after the package's own.
fn system_dirs(manager: Manager) -> &'static [&'static str] {
    match manager {
        Manager::Launchd => &["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin", "/bin", "/usr/sbin", "/sbin"],
        Manager::Systemd => &["/usr/local/bin", "/usr/bin", "/bin", "/usr/local/sbin", "/usr/sbin", "/sbin"],
    }
}

/// The folder of the package's own programs (`agentmux-srv`, `agentmux-mcp`, `agentmux-bashwrap`) that a service
/// puts first on its PATH. An install keeps a `current` link beside its version folders that an update moves, so
/// when the running version's `bin` is also `current/bin` that stable name is the one to write; otherwise the
/// folder this command resolves to.
fn package_bin_dir(exe: &Path) -> Option<PathBuf> {
    let resolved = exe.canonicalize().ok()?;
    let bin = resolved.parent()?.to_path_buf();
    let stable = bin.parent().and_then(Path::parent).map(|server| server.join("current").join("bin"));
    match stable {
        Some(stable) if stable.canonicalize().is_ok_and(|c| c == bin) => Some(stable),
        _ => Some(bin),
    }
}

/// The PATH a service runs with: the package's own folder first, then the system's, and nothing of the shell
/// that ran `setup` (its PATH can name another install's helpers, such as the desktop app's tools, ahead of this
/// package's, and changes with the shell).
pub fn service_path(manager: Manager, exe: &Path) -> String {
    let mut dirs: Vec<String> = package_bin_dir(exe).or_else(|| exe.parent().map(Path::to_path_buf)).into_iter().map(|d| d.to_string_lossy().into_owned()).collect();
    dirs.extend(system_dirs(manager).iter().map(|d| (*d).to_string()));
    dirs.join(":")
}

/// The text of the systemd user unit.
pub fn render_unit(exe: &Path, channel: &str, log: &Path, path_env: &str, home: &Path) -> String {
    format!(
        "[Unit]\n\
         Description=AgentMux server (channel {channel})\n\
         After=network.target\n\
         \n\
         [Service]\n\
         Type=simple\n\
         ExecStart={exe} --channel {channel_arg} run\n\
         Environment={path}\n\
         Environment={home_var}\n\
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
        home_var = systemd_quote(&format!("HOME={}", home.display())),
        log = log.display().to_string().replace('%', "%%"),
    )
}

fn xml_escape(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// The text of the launchd agent.
pub fn render_plist(exe: &Path, channel: &str, log: &Path, path_env: &str, home: &Path) -> String {
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
         \t<dict>\n\
         \t\t<key>HOME</key><string>{home}</string>\n\
         \t\t<key>PATH</key><string>{path}</string>\n\
         \t</dict>\n\
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
        home = xml_escape(&home.to_string_lossy()),
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
    let out = Command::new(program).args(args).stdin(Stdio::null()).no_window().output().map_err(|e| format!("cannot run {program}: {e}"))?;
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
pub fn install(user_home: &Path, exe: &Path, channel: &str, log: &Path) -> Result<PathBuf, String> {
    let manager = manager().ok_or_else(|| "a service is for Linux and macOS in this release".to_string())?;
    let path = file_path(manager, user_home, channel);
    let path_env = service_path(manager, exe);
    let text = match manager {
        Manager::Systemd => render_unit(exe, channel, log, &path_env, user_home),
        Manager::Launchd => render_plist(exe, channel, log, &path_env, user_home),
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

/// Stop the service, take it out of the login, remove its file. The file's path when there was a service to
/// remove, `None` when there was none.
pub fn remove(user_home: &Path, channel: &str) -> Result<Option<PathBuf>, String> {
    let Some(manager) = manager() else { return Ok(None) };
    let path = file_path(manager, user_home, channel);
    if !path.is_file() {
        return Ok(None);
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
    Ok(Some(path))
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
        let text = render_unit(Path::new("/home/u/.agentmux/server/0.60.26/bin/agentmux"), "headless", Path::new("/home/u/.agentmux/channels/headless/server.log"), "/usr/local/bin:/usr/bin", Path::new("/home/u"));
        assert!(text.contains("ExecStart=\"/home/u/.agentmux/server/0.60.26/bin/agentmux\" --channel \"headless\" run\n"), "{text}");
        assert!(text.contains("Environment=\"PATH=/usr/local/bin:/usr/bin\"\n"));
        assert!(text.contains("Environment=\"HOME=/home/u\"\n"), "{text}");
        assert!(text.contains("Restart=on-failure\n") && text.contains("WantedBy=default.target\n"));
        assert!(text.contains("StandardOutput=append:/home/u/.agentmux/channels/headless/server.log\n"));
        assert!(!text.contains("User=") && !text.contains("sudo"), "a user unit needs no user and no root");
    }

    #[test]
    fn a_path_with_a_space_a_quote_or_a_percent_cannot_break_the_unit() {
        let text = render_unit(Path::new("/home/my user/100%/agentmux"), "headless", Path::new("/l/x%y.log"), "/a b:/c\"d", Path::new("/home/my user"));
        assert!(text.contains("ExecStart=\"/home/my user/100%%/agentmux\" "), "{text}");
        assert!(text.contains("Environment=\"PATH=/a b:/c\\\"d\"\n"), "{text}");
        assert!(text.contains("append:/l/x%%y.log\n"), "{text}");
        assert!(!text.contains("\n\n\n"), "no stray line");
        let hostile = render_unit(Path::new("/x/agentmux"), "headless", Path::new("/l"), "/a\nExecStart=/bin/evil", Path::new("/h\nExecStart=/bin/evil2"));
        let settings = hostile.lines().filter(|l| l.starts_with("ExecStart=")).count();
        assert_eq!(settings, 1, "a newline in the PATH starts no second setting: {hostile}");
    }

    #[test]
    fn the_agent_restarts_after_a_crash_but_not_after_a_clean_stop() {
        let text = render_plist(Path::new("/Users/u/.agentmux/server/bin/agentmux"), "headless", Path::new("/Users/u/.agentmux/channels/headless/server.log"), "/opt/homebrew/bin:/usr/bin", Path::new("/Users/u"));
        assert!(text.contains("<key>Label</key><string>ai.agentmux.server</string>"));
        assert!(text.contains("<string>/Users/u/.agentmux/server/bin/agentmux</string>\n\t\t<string>--channel</string>\n\t\t<string>headless</string>\n\t\t<string>run</string>"), "{text}");
        assert!(text.contains("<key>SuccessfulExit</key><false/>") && text.contains("<key>RunAtLoad</key><true/>"));
        assert!(text.starts_with("<?xml") && text.trim_end().ends_with("</plist>"));
    }

    #[test]
    fn the_agent_file_escapes_what_xml_would_read() {
        let text = render_plist(Path::new("/Users/a&b/<x>/agentmux"), "headless", Path::new("/l"), "/p\"q", Path::new("/Users/a&b"));
        assert!(text.contains("/Users/a&amp;b/&lt;x&gt;/agentmux") && text.contains("/p&quot;q"), "{text}");
        assert!(!text.contains("a&b"));
    }

    #[test]
    fn the_agent_gets_the_home_it_was_installed_for_and_the_unit_too() {
        let plist = render_plist(Path::new("/Users/u/.agentmux/server/bin/agentmux"), "headless", Path::new("/l"), "/p", Path::new("/Users/u"));
        assert!(plist.contains("<key>HOME</key><string>/Users/u</string>") && plist.contains("<key>PATH</key><string>/p</string>"), "{plist}");
        let env_start = plist.find("<key>EnvironmentVariables</key>").unwrap();
        let env_end = env_start + plist[env_start..].find("</dict>").unwrap();
        assert!(plist[env_start..env_end].contains("HOME") && plist[env_start..env_end].contains("PATH"), "both live in EnvironmentVariables: {plist}");
        let unit = render_unit(Path::new("/h/agentmux"), "headless", Path::new("/l"), "/p", Path::new("/home/my user"));
        assert!(unit.contains("Environment=\"HOME=/home/my user\"\n"), "{unit}");
        let hostile = render_unit(Path::new("/h/agentmux"), "headless", Path::new("/l"), "/p", Path::new("/h\nExecStart=/bin/evil"));
        assert_eq!(hostile.lines().filter(|l| l.starts_with("ExecStart=")).count(), 1, "a newline in the home starts no second setting: {hostile}");
    }

    #[cfg(unix)]
    #[test]
    fn the_service_path_is_the_packages_own_folder_then_the_systems_and_nothing_of_the_shell() {
        let root = tempfile::tempdir().unwrap();
        let server = root.path().canonicalize().unwrap().join("server");
        let bin = server.join("0.60.28").join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::write(bin.join("agentmux"), "").unwrap();
        // A link as `install.sh` makes it, and one from the person's own bin folder that points into the package.
        std::os::unix::fs::symlink("0.60.28", server.join("current")).unwrap();
        let local = root.path().join(".local").join("bin");
        std::fs::create_dir_all(&local).unwrap();
        std::os::unix::fs::symlink(server.join("current").join("bin").join("agentmux"), local.join("agentmux")).unwrap();

        for manager in [Manager::Launchd, Manager::Systemd] {
            let by_link = service_path(manager, &local.join("agentmux"));
            let dirs: Vec<&str> = by_link.split(':').collect();
            assert_eq!(dirs[0], server.join("current").join("bin").to_string_lossy(), "the stable name, so an update does not strand the service: {by_link}");
            assert_eq!(&dirs[1..], system_dirs(manager), "then the system's folders, in order: {by_link}");
            assert!(dirs.iter().all(|d| !d.is_empty() && d.starts_with('/')), "no empty or relative entry means the current folder: {by_link}");
            assert!(!by_link.contains("AgentMux.app") && !by_link.contains(".local"), "{by_link}");
        }
        // Without a `current` link the folder this command resolves to is used.
        std::fs::remove_file(server.join("current")).unwrap();
        let bare = service_path(Manager::Launchd, &bin.join("agentmux"));
        assert!(bare.starts_with(&format!("{}:", bin.display())), "{bare}");
        // A program that is not there still gets its own folder first.
        assert!(service_path(Manager::Systemd, Path::new("/nowhere/bin/agentmux")).starts_with("/nowhere/bin:/usr/local/bin:"));
    }

    #[test]
    fn removing_a_service_that_is_not_there_says_so_by_removing_nothing() {
        let home = tempfile::tempdir().unwrap();
        assert_eq!(remove(home.path(), "headless").unwrap(), None);
    }

    #[test]
    fn a_file_names_the_program_it_runs_in_either_form() {
        let exe = Path::new("/home/my user/agentmux");
        assert!(file_names_exe(&render_unit(exe, "headless", Path::new("/l"), "/p", Path::new("/h")), exe));
        assert!(file_names_exe(&render_plist(exe, "headless", Path::new("/l"), "/p", Path::new("/h")), exe));
        assert!(!file_names_exe(&render_unit(Path::new("/old/agentmux"), "headless", Path::new("/l"), "/p", Path::new("/h")), exe));
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    #[test]
    fn where_there_is_no_service_there_is_no_install() {
        assert!(manager().is_none());
        assert!(install(Path::new("/h"), Path::new("/x"), "headless", Path::new("/l")).is_err());
        assert!(control(Path::new("/h"), "headless", "start").is_err());
        assert!(!installed(Path::new("/h"), "headless"));
    }
}
