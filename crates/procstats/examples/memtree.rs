// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// memtree: sample the memory of one app's whole process tree, the way
// PLAN_PERFORMANCE_CLAIMS_VERIFICATION_2026_10_11.md §4.1 defines it.
//
//   cargo run --release -p agentmux-procstats --example memtree -- --list agentmux
//   cargo run --release -p agentmux-procstats --example memtree -- --pid 1234 --samples 60 --json out.jsonl
//
// `--list <name>` prints every top-level process whose name contains <name>
// (one per running copy of the app), so you can pick the right `--pid`.
// Sampling then walks everything under that process. Each process is "app"
// when its name contains the app token (default: the root's own name up to
// its first '-', ' ' or '.') and isn't a per-pane helper, otherwise "child"
// (shells, agent CLIs, language servers the app started). Totals are given
// for both. The metrics are each OS's own (`agentmux_procstats::MEMORY_METRIC`):
// Windows private working set and private commit, macOS phys_footprint,
// Linux anonymous resident + swap, plus PSS and USS from smaps_rollup.

use agentmux_procstats::{base_name, command_line_of, snapshot, ProcInfo, MEMORY_METRIC};
use std::collections::HashSet;
use std::io::Write;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const DEFAULT_PANE_HELPERS: &[&str] = &["agentmux-bashwrap", "agentmux-mcp"];

struct Args {
    list: Option<String>,
    pid: Option<u32>,
    app_token: Option<String>,
    pane_helpers: Vec<String>,
    samples: usize,
    interval: Duration,
    json: Option<String>,
}

fn parse_args() -> Result<Args, String> {
    let mut a = Args {
        list: None,
        pid: None,
        app_token: None,
        pane_helpers: DEFAULT_PANE_HELPERS.iter().map(|s| s.to_string()).collect(),
        samples: 60,
        interval: Duration::from_millis(1000),
        json: None,
    };
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        let mut val = || it.next().ok_or(format!("{arg} needs a value"));
        match arg.as_str() {
            "--list" => a.list = Some(val()?.to_lowercase()),
            "--pid" => a.pid = Some(val()?.parse().map_err(|e| format!("--pid: {e}"))?),
            "--app-token" => a.app_token = Some(val()?.to_lowercase()),
            "--pane-helpers" => {
                a.pane_helpers = val()?
                    .split(',')
                    .filter(|s| !s.is_empty())
                    .map(|s| s.to_lowercase())
                    .collect()
            }
            "--samples" => a.samples = val()?.parse().map_err(|e| format!("--samples: {e}"))?,
            "--interval-ms" => {
                a.interval = Duration::from_millis(
                    val()?.parse().map_err(|e| format!("--interval-ms: {e}"))?,
                )
            }
            "--json" => a.json = Some(val()?),
            "-h" | "--help" => return Err(String::new()),
            other => return Err(format!("unknown argument {other}")),
        }
    }
    if a.list.is_none() && a.pid.is_none() {
        return Err("give --list <name> or --pid <pid>".into());
    }
    Ok(a)
}

/// Linux: proportional and unique set size, from smaps_rollup. Bytes.
#[cfg(target_os = "linux")]
fn pss_uss(pid: u32) -> (Option<u64>, Option<u64>) {
    let Ok(text) = std::fs::read_to_string(format!("/proc/{pid}/smaps_rollup")) else {
        return (None, None);
    };
    let field = |name: &str| -> Option<u64> {
        text.lines()
            .find(|l| l.starts_with(name))
            .and_then(|l| l.split_whitespace().nth(1))
            .and_then(|kb| kb.parse::<u64>().ok())
            .map(|kb| kb * 1024)
    };
    let uss = match (field("Private_Clean:"), field("Private_Dirty:")) {
        (Some(c), Some(d)) => Some(c + d),
        _ => None,
    };
    (field("Pss:"), uss)
}

#[cfg(not(target_os = "linux"))]
fn pss_uss(_pid: u32) -> (Option<u64>, Option<u64>) {
    (None, None)
}

/// The process and everything under it, parent before child.
fn tree<'a>(all: &'a [ProcInfo], root: &'a ProcInfo) -> Vec<&'a ProcInfo> {
    let mut out = vec![root];
    let mut i = 0;
    while i < out.len() {
        let parent = out[i];
        for p in all.iter().filter(|p| p.is_child_of(parent)) {
            out.push(p);
        }
        i += 1;
    }
    out
}

fn default_token(name: &str) -> String {
    let lower = name.to_lowercase();
    lower
        .split(['-', ' ', '.', '_'])
        .next()
        .unwrap_or(&lower)
        .to_string()
}

/// CEF/Electron subprocess role from `--type=`, else "main".
fn role(p: &ProcInfo) -> String {
    command_line_of(p.key())
        .and_then(|line| {
            line.split_whitespace().find_map(|w| {
                w.strip_prefix("--type=")
                    .map(|t| t.trim_matches('"').to_string())
            })
        })
        .unwrap_or_else(|| "main".into())
}

#[derive(Default, Clone, Copy)]
struct Totals {
    private: u64,
    resident: u64,
    commit: u64,
    pss: u64,
    uss: u64,
    procs: usize,
}

impl Totals {
    fn add(&mut self, r: &Row) {
        self.private += r.private.unwrap_or(0);
        self.resident += r.resident.unwrap_or(0);
        self.commit += r.commit.unwrap_or(0);
        self.pss += r.pss.unwrap_or(0);
        self.uss += r.uss.unwrap_or(0);
        self.procs += 1;
    }
    fn json(&self) -> String {
        format!(
            "{{\"procs\":{},\"private\":{},\"resident\":{},\"commit\":{},\"pss\":{},\"uss\":{}}}",
            self.procs, self.private, self.resident, self.commit, self.pss, self.uss
        )
    }
}

struct Row {
    pid: u32,
    name: String,
    role: String,
    app: bool,
    private: Option<u64>,
    resident: Option<u64>,
    commit: Option<u64>,
    pss: Option<u64>,
    uss: Option<u64>,
}

fn esc(s: &str) -> String {
    s.chars()
        .flat_map(|c| match c {
            '"' => vec!['\\', '"'],
            '\\' => vec!['\\', '\\'],
            c if (c as u32) < 0x20 => format!("\\u{:04x}", c as u32).chars().collect(),
            c => vec![c],
        })
        .collect()
}

fn opt(v: Option<u64>) -> String {
    v.map(|n| n.to_string()).unwrap_or_else(|| "null".into())
}

fn mb(v: u64) -> String {
    format!("{:.1}", v as f64 / 1_000_000.0)
}

fn median(mut v: Vec<u64>) -> u64 {
    v.sort_unstable();
    if v.is_empty() {
        0
    } else if v.len() % 2 == 1 {
        v[v.len() / 2]
    } else {
        (v[v.len() / 2 - 1] + v[v.len() / 2]) / 2
    }
}

fn list(needle: &str) -> std::io::Result<()> {
    let all = snapshot()?;
    // Per-pane helpers (found under a pane's agent, not the app) aren't roots.
    let matches = |p: &ProcInfo| {
        let lname = p.name.to_lowercase();
        lname.contains(needle) && !DEFAULT_PANE_HELPERS.iter().any(|h| lname.starts_with(h))
    };
    for root in all.iter().filter(|p| matches(p)) {
        let parent_matches = all.iter().any(|q| root.is_child_of(q) && matches(q));
        if parent_matches {
            continue;
        }
        let members = tree(&all, root);
        let line = command_line_of(root.key()).unwrap_or_default();
        println!(
            "pid {:>7}  started {}  {} processes  {}  {}",
            root.pid,
            root.started_at_ms
                .map(|ms| ms.to_string())
                .unwrap_or_else(|| "?".into()),
            members.len(),
            root.name,
            line.chars().take(160).collect::<String>()
        );
    }
    Ok(())
}

fn main() {
    let args = match parse_args() {
        Ok(a) => a,
        Err(e) => {
            if !e.is_empty() {
                eprintln!("memtree: {e}");
            }
            eprintln!("usage: memtree --list <name> | --pid <pid> [--app-token <t>] [--pane-helpers a,b] [--samples 60] [--interval-ms 1000] [--json out.jsonl]");
            std::process::exit(2);
        }
    };
    if let Some(needle) = &args.list {
        if let Err(e) = list(needle) {
            eprintln!("memtree: {e}");
            std::process::exit(1);
        }
        return;
    }
    let root_pid = args.pid.expect("checked in parse_args");
    let mut json = args.json.as_ref().map(|p| {
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(p)
            .unwrap_or_else(|e| {
                eprintln!("memtree: {p}: {e}");
                std::process::exit(1);
            })
    });

    let mut roles = std::collections::HashMap::new();
    let mut app_series: Vec<Totals> = Vec::new();
    let mut child_series: Vec<Totals> = Vec::new();
    let mut last_rows: Vec<Row> = Vec::new();
    let mut root_key = None;

    for i in 0..args.samples {
        if i > 0 {
            std::thread::sleep(args.interval);
        }
        let all = match snapshot() {
            Ok(a) => a,
            Err(e) => {
                eprintln!("memtree: snapshot failed: {e}");
                continue;
            }
        };
        let Some(root) = all
            .iter()
            .find(|p| p.pid == root_pid && root_key.is_none_or(|k| p.key() == k))
        else {
            eprintln!("memtree: process {root_pid} is gone");
            break;
        };
        root_key = Some(root.key());
        let token = args
            .app_token
            .clone()
            .unwrap_or_else(|| default_token(&root.name));
        let helpers: HashSet<&str> = args.pane_helpers.iter().map(|s| s.as_str()).collect();
        let mut rows = Vec::new();
        for p in tree(&all, root) {
            let lname = p.name.to_lowercase();
            let stem = lname.trim_end_matches(".exe");
            let helper = helpers.iter().any(|h| stem.starts_with(h));
            let (pss, uss) = pss_uss(p.pid);
            let role = roles.entry(p.key()).or_insert_with(|| role(p)).clone();
            rows.push(Row {
                pid: p.pid,
                name: base_name(&p.name).to_string(),
                role,
                app: lname.contains(&token) && !helper,
                private: p.mem_private,
                resident: p.mem_resident,
                commit: p.mem_commit,
                pss,
                uss,
            });
        }
        let (mut app, mut child) = (Totals::default(), Totals::default());
        for r in &rows {
            if r.app {
                app.add(r)
            } else {
                child.add(r)
            }
        }
        if let Some(f) = json.as_mut() {
            let ts = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_millis())
                .unwrap_or(0);
            let procs: Vec<String> = rows
                .iter()
                .map(|r| {
                    format!(
                        "{{\"pid\":{},\"name\":\"{}\",\"role\":\"{}\",\"app\":{},\"private\":{},\"resident\":{},\"commit\":{},\"pss\":{},\"uss\":{}}}",
                        r.pid, esc(&r.name), esc(&r.role), r.app, opt(r.private), opt(r.resident), opt(r.commit), opt(r.pss), opt(r.uss)
                    )
                })
                .collect();
            let _ = writeln!(
                f,
                "{{\"ts_ms\":{ts},\"os\":\"{}\",\"metric\":\"{}\",\"root\":{root_pid},\"app\":{},\"child\":{},\"procs\":[{}]}}",
                std::env::consts::OS,
                MEMORY_METRIC,
                app.json(),
                child.json(),
                procs.join(",")
            );
        }
        app_series.push(app);
        child_series.push(child);
        last_rows = rows;
        eprint!(
            "\rsample {}/{}  app private {} MB",
            i + 1,
            args.samples,
            mb(app.private)
        );
    }
    eprintln!();
    if app_series.is_empty() {
        std::process::exit(1);
    }

    println!("Last sample, per process (MB; private = {MEMORY_METRIC}):");
    println!(
        "{:>7}  {:<5}  {:<14} {:>9} {:>9} {:>9} {:>9} {:>9}  name",
        "pid", "kind", "role", "private", "resident", "commit", "pss", "uss"
    );
    let show = |v: Option<u64>| v.map(mb).unwrap_or_else(|| "-".into());
    for r in &last_rows {
        println!(
            "{:>7}  {:<5}  {:<14} {:>9} {:>9} {:>9} {:>9} {:>9}  {}",
            r.pid,
            if r.app { "app" } else { "child" },
            r.role.chars().take(14).collect::<String>(),
            show(r.private),
            show(r.resident),
            show(r.commit),
            show(r.pss),
            show(r.uss),
            r.name
        );
    }
    println!();
    println!(
        "Totals over {} samples (MB): median [min, max]",
        app_series.len()
    );
    // Commit is Windows-only and PSS/USS Linux-only: leave out what this OS lacks.
    let has_commit = last_rows.iter().any(|r| r.commit.is_some());
    let has_pss = last_rows.iter().any(|r| r.pss.is_some());
    for (label, series) in [("app", &app_series), ("child", &child_series)] {
        let stat = |f: fn(&Totals) -> u64| {
            let v: Vec<u64> = series.iter().map(f).collect();
            let (lo, hi) = (*v.iter().min().unwrap_or(&0), *v.iter().max().unwrap_or(&0));
            format!("{} [{}, {}]", mb(median(v)), mb(lo), mb(hi))
        };
        let mut line = format!(
            "  {label:<5} procs {:>3}  private {}  resident {}",
            series.last().map(|t| t.procs).unwrap_or(0),
            stat(|t| t.private),
            stat(|t| t.resident),
        );
        if has_commit {
            line += &format!("  commit {}", stat(|t| t.commit));
        }
        if has_pss {
            line += &format!("  pss {}  uss {}", stat(|t| t.pss), stat(|t| t.uss));
        }
        println!("{line}");
    }
}
