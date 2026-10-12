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
// Sampling then walks everything under that process. A process is "child"
// when it is, or runs under, a pane root: a shell, a console host or a
// per-pane helper (`--pane-roots` changes the list). Everything else the app
// started, its own helpers and language servers included, is "app". Totals
// are given for both. The metrics are each OS's own
// (`agentmux_procstats::MEMORY_METRIC`): Windows private working set and
// private commit, macOS phys_footprint, Linux anonymous resident + swap,
// plus PSS and USS from smaps_rollup.
//
// A total is null when the OS doesn't provide the metric, and null and
// listed in "partial" when some processes had it and others didn't (one
// exited mid-read), so a total is never silently short. On macOS the
// per-process footprints are summed, which can count a region shared between
// processes more than once; the run ends with `footprint`'s own total over the
// same processes, which counts each once.

use agentmux_procstats::{base_name, command_line_of, snapshot, ProcInfo, ProcKey, MEMORY_METRIC};
use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Executable stems that start a pane's own subtree: agent CLIs (AgentMux
/// starts them without a shell on Windows), shells, console hosts and
/// AgentMux's per-pane helpers.
const DEFAULT_PANE_ROOTS: &[&str] = &[
    "claude",
    "codex",
    "gemini",
    "kimi",
    "opencode",
    "qwen",
    "aider",
    "goose",
    "amp",
    "copilot",
    "cursor-agent",
    "agentmux-bashwrap",
    "agentmux-mcp",
    "bash",
    "zsh",
    "sh",
    "dash",
    "fish",
    "nu",
    "pwsh",
    "powershell",
    "cmd",
    "conhost",
    "openconsole",
    "tmux",
    "login",
];

const METRICS: [&str; 5] = ["private", "resident", "commit", "pss", "uss"];

struct Args {
    list: Option<String>,
    pid: Option<u32>,
    pane_roots: Vec<String>,
    samples: usize,
    interval: Duration,
    json: Option<String>,
}

fn parse_args() -> Result<Args, String> {
    let mut a = Args {
        list: None,
        pid: None,
        pane_roots: DEFAULT_PANE_ROOTS.iter().map(|s| s.to_string()).collect(),
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
            "--pane-roots" => {
                a.pane_roots = val()?
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

/// The executable's name, lowercased, without `.exe`.
fn stem(name: &str) -> String {
    let lower = base_name(name).to_lowercase();
    lower.strip_suffix(".exe").unwrap_or(&lower).to_string()
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

/// One metric summed over a set of processes, with how many had it.
#[derive(Default, Clone, Copy)]
struct Sum {
    total: u64,
    have: usize,
    missing: usize,
}

impl Sum {
    fn add(&mut self, v: Option<u64>) {
        match v {
            Some(v) => {
                self.total += v;
                self.have += 1;
            }
            None => self.missing += 1,
        }
    }
    /// The total, only when every process had the metric: `None` when the
    /// OS doesn't provide it, or when some processes lacked it (partial).
    fn value(&self) -> Option<u64> {
        (self.have > 0 && self.missing == 0).then_some(self.total)
    }
    fn partial(&self) -> bool {
        self.have > 0 && self.missing > 0
    }
}

#[derive(Default, Clone, Copy)]
struct Totals {
    sums: [Sum; 5],
    procs: usize,
}

impl Totals {
    fn add(&mut self, r: &Row) {
        let values = [r.private, r.resident, r.commit, r.pss, r.uss];
        for (sum, v) in self.sums.iter_mut().zip(values) {
            sum.add(v);
        }
        self.procs += 1;
    }
    fn json(&self) -> String {
        let fields: Vec<String> = METRICS
            .iter()
            .zip(&self.sums)
            .map(|(name, sum)| format!("\"{name}\":{}", opt(sum.value())))
            .collect();
        let partial: Vec<String> = METRICS
            .iter()
            .zip(&self.sums)
            .filter(|(_, sum)| sum.partial())
            .map(|(name, _)| format!("\"{name}\""))
            .collect();
        format!(
            "{{\"procs\":{},{},\"partial\":[{}]}}",
            self.procs,
            fields.join(","),
            partial.join(",")
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

fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

fn list(needle: &str) -> std::io::Result<()> {
    let all = snapshot()?;
    // Pane roots (a shell or a per-pane helper whose name matches) aren't
    // copies of the app.
    let matches = |p: &ProcInfo| {
        p.name.to_lowercase().contains(needle)
            && !DEFAULT_PANE_ROOTS.contains(&stem(&p.name).as_str())
    };
    for root in all.iter().filter(|p| matches(p)) {
        if all.iter().any(|q| root.is_child_of(q) && matches(q)) {
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

/// The run's raw-data file: one run per file, so an existing file is
/// refused rather than mixed with this run's samples. It starts with a line
/// saying what the run was.
fn open_json(path: &str, args: &Args, root_pid: u32) -> std::fs::File {
    let fail = |e: std::io::Error| -> ! {
        eprintln!("memtree: {path}: {e} (use a new file per run)");
        std::process::exit(1);
    };
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .unwrap_or_else(|e| fail(e));
    let roots: Vec<String> = args
        .pane_roots
        .iter()
        .map(|r| format!("\"{}\"", esc(r)))
        .collect();
    writeln!(
        f,
        "{{\"run\":{{\"started_ms\":{},\"os\":\"{}\",\"metric\":\"{}\",\"root\":{root_pid},\"samples\":{},\"interval_ms\":{},\"pane_roots\":[{}]}}}}",
        now_ms(),
        std::env::consts::OS,
        MEMORY_METRIC,
        args.samples,
        args.interval.as_millis(),
        roots.join(",")
    )
    .unwrap_or_else(|e| fail(e));
    f
}

fn main() {
    let args = match parse_args() {
        Ok(a) => a,
        Err(e) => {
            if !e.is_empty() {
                eprintln!("memtree: {e}");
            }
            eprintln!("usage: memtree --list <name> | --pid <pid> [--pane-roots a,b] [--samples 60] [--interval-ms 1000] [--json out.jsonl]");
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
    let mut json = args.json.as_deref().map(|p| open_json(p, &args, root_pid));
    let pane_roots: HashSet<&str> = args.pane_roots.iter().map(|s| s.as_str()).collect();

    let mut roles = HashMap::new();
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
        // A process is a pane's when it is a pane root or runs under one.
        // `tree` lists parents before children, so a parent's verdict is in.
        let mut in_pane: HashSet<ProcKey> = HashSet::new();
        let mut rows = Vec::new();
        for p in tree(&all, root) {
            let parent_in_pane = all
                .iter()
                .any(|q| in_pane.contains(&q.key()) && p.is_child_of(q));
            let pane = p.pid != root.pid
                && (parent_in_pane || pane_roots.contains(stem(&p.name).as_str()));
            if pane {
                in_pane.insert(p.key());
            }
            let (pss, uss) = pss_uss(p.pid);
            let role = roles.entry(p.key()).or_insert_with(|| role(p)).clone();
            rows.push(Row {
                pid: p.pid,
                name: base_name(&p.name).to_string(),
                role,
                app: !pane,
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
            let procs: Vec<String> = rows
                .iter()
                .map(|r| {
                    format!(
                        "{{\"pid\":{},\"name\":\"{}\",\"role\":\"{}\",\"app\":{},\"private\":{},\"resident\":{},\"commit\":{},\"pss\":{},\"uss\":{}}}",
                        r.pid, esc(&r.name), esc(&r.role), r.app, opt(r.private), opt(r.resident), opt(r.commit), opt(r.pss), opt(r.uss)
                    )
                })
                .collect();
            // The .jsonl is the run's raw data: a failed write ends the run
            // rather than leaving a summary over a missing or cut-short file.
            if let Err(e) = writeln!(
                f,
                "{{\"ts_ms\":{},\"app\":{},\"child\":{},\"procs\":[{}]}}",
                now_ms(),
                app.json(),
                child.json(),
                procs.join(",")
            )
            .and_then(|_| f.flush())
            {
                eprintln!(
                    "\nmemtree: writing {}: {e}",
                    args.json.as_deref().unwrap_or("?")
                );
                std::process::exit(1);
            }
        }
        app_series.push(app);
        child_series.push(child);
        last_rows = rows;
        eprint!(
            "\rsample {}/{}  app private {} MB",
            i + 1,
            args.samples,
            mb(app.sums[0].total)
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
    // A metric this OS doesn't provide (commit off Windows, PSS/USS off
    // Linux) has no samples and is left out. A partial sample (a process
    // lacked the metric) is left out of the median and counted.
    for (label, series) in [("app", &app_series), ("child", &child_series)] {
        let last = series.last().copied().unwrap_or_default();
        let mut line = format!("  {label:<5} procs {:>3}", last.procs);
        for (i, name) in METRICS.iter().enumerate() {
            let v: Vec<u64> = series.iter().filter_map(|t| t.sums[i].value()).collect();
            let partial = series.iter().filter(|t| t.sums[i].partial()).count();
            let (Some(&lo), Some(&hi)) = (v.iter().min(), v.iter().max()) else {
                if partial > 0 {
                    line += &format!("  {name} partial in all {partial} samples");
                }
                continue;
            };
            line += &format!("  {name} {} [{}, {}]", mb(median(v)), mb(lo), mb(hi));
            if partial > 0 {
                line += &format!(" ({partial} partial samples left out)");
            }
        }
        println!("{line}");
    }
    footprint_cross_check(&last_rows);
}

/// macOS: `footprint`'s own total over the app processes of the last
/// sample. It counts a region shared between processes once, where the
/// per-process sum above can count it in each.
#[cfg(target_os = "macos")]
fn footprint_cross_check(rows: &[Row]) {
    let app: Vec<&Row> = rows.iter().filter(|r| r.app).collect();
    let mut cmd = std::process::Command::new("footprint");
    for r in &app {
        cmd.arg("-p").arg(r.pid.to_string());
    }
    println!();
    match cmd.output() {
        Ok(out) if out.status.success() => {
            let text = String::from_utf8_lossy(&out.stdout);
            let lines: Vec<&str> = text.lines().collect();
            println!(
                "footprint over the {} app processes (last 25 lines):",
                app.len()
            );
            for l in &lines[lines.len().saturating_sub(25)..] {
                println!("  {l}");
            }
        }
        Ok(out) => println!(
            "footprint failed ({}): {}",
            out.status,
            String::from_utf8_lossy(&out.stderr).trim()
        ),
        Err(e) => println!("footprint not run: {e}"),
    }
}

#[cfg(not(target_os = "macos"))]
fn footprint_cross_check(_rows: &[Row]) {}
