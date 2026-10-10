// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Tower: what each of AgentMux's own processes is — "GPU", "Renderer",
//! "Network service", "Server", "Launcher" — so the AgentMux task's rows say
//! more than one executable name repeated a dozen times.
//!
//! Every CEF subprocess runs the host executable, so its name can't tell them
//! apart (on Windows every one reads "AgentMux"); its command line can, with
//! `--type=` and `--utility-sub-type=`. A command line can hold secrets, so it
//! is read here only to pick a label and never leaves this module. On macOS the
//! helper apps are already named per type ("AgentMux Helper (GPU)"), which is
//! the fallback when a command line can't be read.
//!
//! The window host and the launcher carry no `--type`; they are told apart by
//! the tree: the host is the parent of CEF subprocesses, the launcher an
//! ancestor of this backend.

use std::collections::{HashMap, HashSet};

use agentmux_procstats::{ProcInfo, ProcKey};

/// What a process's own command line (or, failing that, its name) says.
#[derive(Debug, Clone, PartialEq)]
enum Kind {
    /// A CEF subprocess: its type.
    Cef(String),
    /// srv itself in another role (it re-runs its own binary).
    Srv(&'static str),
    /// No `--type`: the window host or the launcher, or neither.
    Plain,
    /// The OS wouldn't give the command line, and the name says nothing:
    /// read again on later samples, up to [`READ_ATTEMPTS`] in all, since a
    /// read can fail on a process that is still starting.
    Unreadable(u8),
}

/// How many times a command line that couldn't be read is tried. A process
/// the OS never lets us read (another user's) is given up on after that.
const READ_ATTEMPTS: u8 = 5;

/// Remembers each process's [`Kind`], so a command line is read once per
/// process rather than once per sample. Forgets processes that are gone.
#[derive(Debug, Default)]
pub struct Describer {
    kinds: HashMap<ProcKey, Kind>,
}

impl Describer {
    /// A label for each of `members` (indices into `snap`) that has one.
    /// `cmdline` reads a process's command line (`agentmux_procstats::command_line_of`).
    pub fn describe(
        &mut self,
        snap: &[ProcInfo],
        members: &[usize],
        own_pid: u32,
        cmdline: &dyn Fn(ProcKey) -> Option<String>,
    ) -> HashMap<usize, String> {
        let present: HashSet<ProcKey> = members.iter().map(|&i| snap[i].key()).collect();
        self.kinds.retain(|k, _| present.contains(k));
        for &i in members {
            let p = &snap[i];
            if p.pid == own_pid {
                continue;
            }
            let attempts = match self.kinds.get(&p.key()) {
                None => 0,
                Some(Kind::Unreadable(n)) if *n < READ_ATTEMPTS => *n,
                Some(_) => continue,
            };
            let kind = match kind_of(p, cmdline(p.key()).as_deref()) {
                Kind::Unreadable(_) => Kind::Unreadable(attempts + 1),
                known => known,
            };
            self.kinds.insert(p.key(), kind);
        }

        let by_pid: HashMap<u32, usize> = members.iter().map(|&i| (snap[i].pid, i)).collect();
        let parent_of = |i: usize| {
            let p = &snap[i];
            p.ppid.and_then(|pp| by_pid.get(&pp).copied()).filter(|&q| p.is_child_of(&snap[q]))
        };
        // The window host: whatever CEF subprocesses hang off.
        let hosts: HashSet<usize> = members
            .iter()
            .filter(|&&i| matches!(self.kinds.get(&snap[i].key()), Some(Kind::Cef(_))))
            .filter_map(|&i| parent_of(i))
            .collect();
        // The launcher: this backend's AgentMux ancestors.
        let mut ancestors = HashSet::new();
        let mut at = by_pid.get(&own_pid).copied();
        for _ in 0..16 {
            let Some(parent) = at.and_then(parent_of) else { break };
            ancestors.insert(parent);
            at = Some(parent);
        }

        let mut out = HashMap::new();
        for &i in members {
            let p = &snap[i];
            let label = if p.pid == own_pid {
                Some("Server".to_string())
            } else {
                match self.kinds.get(&p.key()) {
                    Some(Kind::Cef(t)) => Some(t.clone()),
                    Some(Kind::Srv(role)) => Some(role.to_string()),
                    _ if hosts.contains(&i) => Some("Main process".to_string()),
                    _ if ancestors.contains(&i) => Some("Launcher".to_string()),
                    _ => None,
                }
            };
            if let Some(label) = label {
                out.insert(i, label);
            }
        }
        out
    }
}

fn kind_of(p: &ProcInfo, cmdline: Option<&str>) -> Kind {
    let Some(cmd) = cmdline else {
        return mac_helper_type(&p.name).map_or(Kind::Unreadable(1), Kind::Cef);
    };
    let name = p.name.to_ascii_lowercase();
    // The AgentMux group holds every unclaimed descendant of the launcher, not
    // only AgentMux's binaries, so a `--type=` switch counts only on one of
    // ours (the host exe, "agentmux-cef", or a macOS "AgentMux Helper").
    if name.starts_with("agentmux") {
        if let Some(ty) = switch(cmd, "type") {
            return Kind::Cef(cef_label(ty, switch(cmd, "utility-sub-type")));
        }
    }
    if name.starts_with("agentmux-srv") {
        return Kind::Srv(if has_arg(cmd, "__extract") {
            "Document parser"
        } else if has_arg(cmd, "--crash-monitor") {
            "Crash monitor"
        } else {
            "Server"
        });
    }
    Kind::Plain
}

fn args(cmd: &str) -> impl Iterator<Item = &str> {
    cmd.split_whitespace().map(|a| a.trim_matches('"'))
}

/// `--name=value` → `value`. Chromium writes its switches in that form, and
/// none of the values read here contain spaces.
fn switch<'a>(cmd: &'a str, name: &str) -> Option<&'a str> {
    let key = format!("--{name}=");
    args(cmd).find_map(|a| a.strip_prefix(key.as_str()))
}

fn has_arg(cmd: &str, arg: &str) -> bool {
    args(cmd).any(|a| a == arg)
}

fn cef_label(ty: &str, utility_sub_type: Option<&str>) -> String {
    match ty {
        "renderer" => "Renderer".to_string(),
        "gpu-process" => "GPU".to_string(),
        "utility" => utility_label(utility_sub_type),
        "crashpad-handler" => "Crash reporter".to_string(),
        "zygote" => "Zygote".to_string(),
        "ppapi" | "ppapi-broker" => "Plugin".to_string(),
        // A fixed label: never text taken from a command line.
        _ => "Subprocess".to_string(),
    }
}

/// `network.mojom.NetworkService` → "Network service".
fn utility_label(sub_type: Option<&str>) -> String {
    let Some(service) = sub_type.and_then(|s| s.split('.').next()).filter(|s| !s.is_empty()) else {
        return "Utility".to_string();
    };
    match service {
        "network" => "Network service".to_string(),
        "storage" => "Storage service".to_string(),
        "audio" => "Audio service".to_string(),
        "video_capture" => "Video capture service".to_string(),
        "data_decoder" => "Data decoder".to_string(),
        // A fixed label: never text taken from a command line.
        _ => "Utility".to_string(),
    }
}

/// macOS helper apps are named per type: "AgentMux Helper (GPU)" → "GPU". The
/// plain "AgentMux Helper" runs utility processes.
fn mac_helper_type(name: &str) -> Option<String> {
    let rest = name.strip_prefix("AgentMux Helper")?;
    let rest = rest.trim();
    if rest.is_empty() {
        return Some("Utility".to_string());
    }
    let ty = rest.strip_prefix('(')?.strip_suffix(')')?;
    Some(match ty {
        "Renderer" | "Alloy" => "Renderer".to_string(),
        other => other.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(pid: u32, ppid: u32, name: &str) -> ProcInfo {
        ProcInfo {
            pid,
            ppid: Some(ppid).filter(|&x| x != 0),
            start_key: pid as u64 * 10,
            started_at_ms: Some(pid as u64 * 10),
            name: name.to_string(),
            cpu_ns: Some(0),
            mem_private: Some(1000),
            mem_resident: Some(2000),
            mem_commit: None,
        }
    }

    /// A Windows portable, as measured: the launcher starts the host and srv;
    /// the host's CEF subprocesses hang off it; srv has a crash monitor.
    fn windows_tree() -> (Vec<ProcInfo>, HashMap<u32, &'static str>) {
        let snap = vec![
            p(10, 1, "agentmux.exe"),
            p(20, 10, "agentmux-0.59.18.exe"),
            p(21, 20, "agentmux-0.59.18.exe"),
            p(22, 20, "agentmux-0.59.18.exe"),
            p(23, 20, "agentmux-0.59.18.exe"),
            p(24, 20, "agentmux-0.59.18.exe"),
            p(30, 10, "agentmux-srv-0.59.18-windows.x64.exe"),
            p(31, 30, "agentmux-srv-0.59.18-windows.x64.exe"),
            p(32, 30, "agentmux-srv-0.59.18-windows.x64.exe"),
        ];
        let cmd = HashMap::from([
            (10, r#""C:\AgentMux\agentmux.exe""#),
            (20, r#""C:\AgentMux\runtime\agentmux-0.59.18.exe" --some-flag"#),
            (21, r#""C:\AgentMux\runtime\agentmux-0.59.18.exe" --type=gpu-process --gpu-preferences=abc"#),
            (22, r#""C:\AgentMux\runtime\agentmux-0.59.18.exe" --type=renderer --renderer-client-id=4"#),
            (23, r#""C:\AgentMux\runtime\agentmux-0.59.18.exe" --type=utility --utility-sub-type=network.mojom.NetworkService"#),
            (24, r#""C:\AgentMux\runtime\agentmux-0.59.18.exe" --type=utility --utility-sub-type=video_capture.mojom.VideoCaptureService"#),
            (31, r#""C:\AgentMux\runtime\agentmux-srv-0.59.18-windows.x64.exe" --crash-monitor"#),
            (32, r#""C:\AgentMux\runtime\agentmux-srv-0.59.18-windows.x64.exe" __extract --kind pdf"#),
        ]);
        (snap, cmd)
    }

    fn labels(snap: &[ProcInfo], cmd: &HashMap<u32, &str>, own_pid: u32) -> HashMap<u32, String> {
        let members: Vec<usize> = (0..snap.len()).collect();
        let read = |k: ProcKey| cmd.get(&k.pid).map(|s| s.to_string());
        Describer::default()
            .describe(snap, &members, own_pid, &read)
            .into_iter()
            .map(|(i, l)| (snap[i].pid, l))
            .collect()
    }

    #[test]
    fn labels_every_process_of_a_windows_install() {
        let (snap, cmd) = windows_tree();
        let got = labels(&snap, &cmd, 30);
        let want: HashMap<u32, String> = [
            (10, "Launcher"),
            (20, "Main process"),
            (21, "GPU"),
            (22, "Renderer"),
            (23, "Network service"),
            (24, "Video capture service"),
            (30, "Server"),
            (31, "Crash monitor"),
            (32, "Document parser"),
        ]
        .into_iter()
        .map(|(pid, l)| (pid, l.to_string()))
        .collect();
        assert_eq!(got, want);
    }

    #[test]
    fn a_macos_helper_is_labelled_by_its_name_when_its_command_line_is_unreadable() {
        let snap = vec![
            p(10, 1, "agentmux-launcher"),
            p(20, 10, "agentmux-cef"),
            p(21, 20, "AgentMux Helper (GPU)"),
            p(22, 20, "AgentMux Helper (Renderer)"),
            p(23, 20, "AgentMux Helper"),
            p(30, 10, "agentmux-srv-0.59.18-darwin.arm64"),
        ];
        let got = labels(&snap, &HashMap::new(), 30);
        assert_eq!(got.get(&21).map(String::as_str), Some("GPU"));
        assert_eq!(got.get(&22).map(String::as_str), Some("Renderer"));
        assert_eq!(got.get(&23).map(String::as_str), Some("Utility"));
        // The helpers make 20 the window host even without command lines.
        assert_eq!(got.get(&20).map(String::as_str), Some("Main process"));
        assert_eq!(got.get(&10).map(String::as_str), Some("Launcher"));
        assert_eq!(got.get(&30).map(String::as_str), Some("Server"));
    }

    #[test]
    fn a_process_that_is_neither_host_nor_launcher_gets_no_label() {
        let snap = vec![p(10, 1, "agentmux.exe"), p(30, 10, "agentmux-srv.exe"), p(40, 30, "agentmux-helper-thing")];
        let cmd = HashMap::from([(10, "agentmux.exe"), (40, "agentmux-helper-thing --x")]);
        let got = labels(&snap, &cmd, 30);
        assert_eq!(got.get(&40), None);
        assert_eq!(got.get(&10).map(String::as_str), Some("Launcher"));
    }

    #[test]
    fn reads_each_command_line_once_and_forgets_exited_processes() {
        let (snap, cmd) = windows_tree();
        let members: Vec<usize> = (0..snap.len()).collect();
        let reads = std::cell::Cell::new(0);
        let read = |k: ProcKey| {
            reads.set(reads.get() + 1);
            cmd.get(&k.pid).map(|s| s.to_string())
        };
        let mut d = Describer::default();
        d.describe(&snap, &members, 30, &read);
        let first = reads.get();
        assert_eq!(first, snap.len() - 1, "every process but srv itself, once");
        d.describe(&snap, &members, 30, &read);
        assert_eq!(reads.get(), first, "a second sample reads nothing");
        d.describe(&snap, &members[..2], 30, &read);
        assert_eq!(d.kinds.len(), 2, "processes no longer present are forgotten");
    }

    #[test]
    fn a_command_line_that_could_not_be_read_is_tried_again_a_few_times() {
        let (snap, cmd) = windows_tree();
        let members: Vec<usize> = (0..snap.len()).collect();
        // The GPU process (21) can't be read at first, then can.
        let readable = std::cell::Cell::new(false);
        let reads_of_21 = std::cell::Cell::new(0);
        let read = |k: ProcKey| {
            if k.pid == 21 {
                reads_of_21.set(reads_of_21.get() + 1);
                if !readable.get() {
                    return None;
                }
            }
            cmd.get(&k.pid).map(|s| s.to_string())
        };
        let mut d = Describer::default();
        let first = d.describe(&snap, &members, 30, &read);
        assert_eq!(first.get(&2), None, "unreadable at first: no label yet");
        readable.set(true);
        let second = d.describe(&snap, &members, 30, &read);
        assert_eq!(second.get(&2).map(String::as_str), Some("GPU"));
        assert_eq!(reads_of_21.get(), 2);
        d.describe(&snap, &members, 30, &read);
        assert_eq!(reads_of_21.get(), 2, "read once it's known");
    }

    #[test]
    fn a_process_that_can_never_be_read_is_given_up_on() {
        let snap = vec![p(10, 1, "agentmux.exe"), p(30, 10, "agentmux-srv.exe"), p(40, 30, "agentmux-thing")];
        let members: Vec<usize> = (0..snap.len()).collect();
        let reads = std::cell::Cell::new(0);
        let read = |k: ProcKey| {
            if k.pid == 40 {
                reads.set(reads.get() + 1);
            }
            None
        };
        let mut d = Describer::default();
        for _ in 0..10 {
            d.describe(&snap, &members, 30, &read);
        }
        assert_eq!(reads.get(), READ_ATTEMPTS as usize);
    }

    #[test]
    fn utility_sub_types() {
        assert_eq!(utility_label(Some("storage.mojom.StorageService")), "Storage service");
        assert_eq!(utility_label(Some("audio.mojom.AudioService")), "Audio service");
        assert_eq!(utility_label(Some("proxy_resolver.mojom.ProxyResolverFactory")), "Utility");
        assert_eq!(utility_label(None), "Utility");
    }

    #[test]
    fn other_cef_types() {
        assert_eq!(cef_label("crashpad-handler", None), "Crash reporter");
        assert_eq!(cef_label("zygote", None), "Zygote");
        assert_eq!(cef_label("broker", None), "Subprocess");
    }

    #[test]
    fn a_type_switch_on_a_process_that_is_not_ours_means_nothing() {
        // Some tool the user ran from an AgentMux terminal that wasn't claimed
        // by a task, with a Chromium-style switch of its own.
        let snap = vec![
            p(10, 1, "agentmux.exe"),
            p(30, 10, "agentmux-srv.exe"),
            p(40, 30, "electron.exe"),
        ];
        let cmd = HashMap::from([(10, "agentmux.exe"), (40, "electron.exe --type=renderer")]);
        let got = labels(&snap, &cmd, 30);
        assert_eq!(got.get(&40), None);
        // And it doesn't make its parent look like the window host.
        assert_eq!(got.get(&30).map(String::as_str), Some("Server"));
    }
}
