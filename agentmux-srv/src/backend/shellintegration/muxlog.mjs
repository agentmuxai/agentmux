#!/usr/bin/env node
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// muxlog — discover, render, and follow AgentMux logs across every instance.
//
// Deployed by agentmux-srv next to the shell rcfiles (~/.agentmux/shell/muxlog.mjs);
// the bash/zsh/pwsh/fish `muxlog` functions delegate here. Run standalone with
// `node muxlog.mjs ...` (works from any subshell, unlike the shell function).
//
// Why a Node core instead of per-shell functions: log lines are structured NDJSON,
// they live in three different root trees (shared, dev/<branch>, channels/local-*),
// and the old version-pinned pointer routinely resolved a STALE instance's log.
// One tested implementation does discovery + JSON rendering + filtering uniformly.

import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { execFileSync } from "node:child_process";
import { pathToFileURL } from "node:url";

const HOME = os.homedir();
const AGENTMUX = path.join(HOME, ".agentmux");

// ─── Log discovery ────────────────────────────────────────────────────────────
// Logs can live in any of these roots (the shared dir alone is NOT enough — dev
// builds and per-build channels keep their host log in their own data dir):
//   ~/.agentmux/logs/                                  (shared: launcher, and
//                                                        srv when AGENTMUX_LOG_DIR
//                                                        isn't set — see below)
//   ~/.agentmux/dev/<branch>/<hash>/logs/              (task dev, keyed on branch)
//   ~/.agentmux/channels/<channel>/versions/<v>/logs/  (portable/per-build; both
//                                                        host AND srv as of
//                                                        agentmux-srv/src/bootstrap.rs
//                                                        honoring AGENTMUX_LOG_DIR —
//                                                        see REPORT_MUXSPECT_MUXLOG_
//                                                        CROSS_CHANNEL_INSPECTION_2026_08_22.md)
//
// The confirmed on-disk depth is `versions/<v>/logs` (ONE directory between
// version and logs — verified against a real running instance's own
// AGENTMUX_LOG_DIR). An earlier version of this glob assumed TWO directories
// (`versions/*/*/logs`), which never actually matches that shape — silently
// finding zero channel-build logs ever, on any platform, the entire time this
// source existed. Try both depths (cheap — glob() is a handful of readdirSync
// calls) so a differently-shaped install elsewhere still resolves, deduping by
// path so a hit on the 1-level pattern doesn't also list on the 2-level one.
function* logRoots() {
    yield { dir: path.join(AGENTMUX, "logs"), source: "shared" };
    for (const p of glob(path.join(AGENTMUX, "dev", "*", "*", "logs")))
        yield { dir: p, source: "dev:" + p.split(path.sep).at(-3) };
    const seen = new Set();
    for (const p of glob(path.join(AGENTMUX, "channels", "*", "versions", "*", "logs"))) {
        if (seen.has(p)) continue; seen.add(p);
        yield { dir: p, source: "channel:" + p.split(path.sep).at(-4) };
    }
    for (const p of glob(path.join(AGENTMUX, "channels", "*", "versions", "*", "*", "logs"))) {
        if (seen.has(p)) continue; seen.add(p);
        yield { dir: p, source: "channel:" + p.split(path.sep).at(-5) };
    }
}

// Tiny one-level-per-`*` glob (no deps). Only handles `*` path segments.
// Exported (pure, no I/O beyond readdirSync/existsSync — no network, no
// process.exit) for muxlog.test.mjs, which pins the depth-matching bug fixed
// in logRoots() below (an earlier version of the channels/ glob pattern had
// one wildcard segment too many and could never match a real install).
export function glob(pattern) {
    const parts = pattern.split(path.sep);
    let bases = [parts[0] === "" ? path.sep : parts[0]];
    for (let i = 1; i < parts.length; i++) {
        const seg = parts[i];
        const next = [];
        for (const base of bases) {
            if (seg.includes("*")) {
                const re = new RegExp("^" + seg.replace(/[.+?^${}()|[\]\\]/g, "\\$&").replace(/\*/g, ".*") + "$");
                let entries = [];
                try { entries = fs.readdirSync(base, { withFileTypes: true }); } catch { /* skip */ }
                for (const e of entries) if (re.test(e.name)) next.push(path.join(base, e.name));
            } else {
                next.push(path.join(base, seg));
            }
        }
        bases = next;
    }
    return bases.filter((p) => { try { return fs.existsSync(p); } catch { return false; } });
}

const TARGET_GLOB = {
    host: /^agentmux-host-v.*\.log(\..*)?$/,
    srv: /^agentmuxsrv-v.*\.log(\..*)?$/,
    launcher: /^agentmux-launcher\.log$/,
};

// Returns [{target, file, mtime, size, version, source}] across all roots, newest first.
function discover(target) {
    const out = [];
    const want = target === "fe" || target === "all" ? null : target;
    for (const { dir, source } of logRoots()) {
        let entries = [];
        try { entries = fs.readdirSync(dir); } catch { continue; }
        for (const name of entries) {
            for (const [t, re] of Object.entries(TARGET_GLOB)) {
                if (want && t !== want) continue;
                if (!re.test(name)) continue;
                const full = path.join(dir, name);
                let st; try { st = fs.statSync(full); } catch { continue; }
                const ver = (name.match(/v(\d+\.\d+\.\d+)/) || [])[1] || "?";
                out.push({ target: t, file: full, mtime: st.mtimeMs, size: st.size, version: ver, source });
            }
        }
    }
    return out.sort((a, b) => b.mtime - a.mtime);
}

// ─── Rendering ────────────────────────────────────────────────────────────────
const LVL_COLOR = { ERROR: "\x1b[31m", WARN: "\x1b[33m", INFO: "\x1b[2m", DEBUG: "\x1b[2m" };
const RESET = "\x1b[0m";
const DIM = "\x1b[2m";

// An NDJSON log line → one compact human line (or null if filtered out).
function renderLine(raw, opt) {
    const t = raw.trim();
    if (!t) return null;
    let j;
    try {
        j = JSON.parse(t);
    } catch {
        // Non-JSON line (a panic backtrace, a raw println). It has no structured
        // fields, so any structured filter excludes it; a --grep must still match
        // its text. Otherwise pass it through verbatim so nothing is lost.
        if (opt.level || opt.target || opt.excludeTarget || opt.since) return null;
        if (opt.grep && !opt.grep.test(t)) return null;
        return t;
    }
    const fields = j.fields || {};
    const msg = fields.message ?? j.message ?? "";
    const target = j.target || "";
    const level = (j.level || "INFO").toUpperCase();

    // Default: drop agent-conversation transcript noise (the srv log is mostly this).
    if (!opt.all && /blockcontroller::subprocess|subprocess stdout → blockfile/.test(target + " " + msg)) return null;
    if (opt.level && !opt.level.includes(level.toLowerCase())) return null;
    if (opt.target && !target.includes(opt.target)) return null;
    if (opt.excludeTarget && target.includes(opt.excludeTarget)) return null;
    if (opt.grep && !opt.grep.test(String(msg))) return null;
    if (opt.since && j.timestamp && j.timestamp < opt.since) return null;

    if (opt.raw) return t;
    const ts = (j.timestamp || "").replace(/^.*T/, "").replace(/\..*$/, "") || "--:--:--";
    const c = LVL_COLOR[level] ?? "";
    let line = `${DIM}${ts}${RESET} ${c}${level.padEnd(5)}${RESET} ${DIM}${shortTarget(target)}${RESET}  ${msg}`;
    if (opt.verbose) {
        const extra = { ...fields }; delete extra.message;
        const keys = Object.keys(extra);
        if (keys.length) line += `  ${DIM}${JSON.stringify(extra)}${RESET}`;
    }
    return line;
}

function shortTarget(t) {
    // agentmux_cef::commands::backend → cef:backend ; agentmux_srv::server → srv:server
    return t.replace(/^agentmux_/, "").replace(/::/g, ":").replace(/:[^:]+:/g, ":");
}

// ─── Follow (tail -f over NDJSON) ─────────────────────────────────────────────
// Read for display. For a plain tail of a huge file we only need the end, so
// read the last 8 MB (the launcher log can reach hundreds of MB). When we have
// to scan/filter the whole history (grep, recipes, explicit filters) pass
// whole=true so matches aren't missed.
function readForDisplay(file, whole) {
    let size = 0; try { size = fs.statSync(file).size; } catch { return ""; }
    const WINDOW = 8 * 1024 * 1024;
    if (whole || size <= WINDOW) { try { return fs.readFileSync(file, "utf8"); } catch { return ""; } }
    const fd = fs.openSync(file, "r");
    const buf = Buffer.alloc(WINDOW);
    fs.readSync(fd, buf, 0, WINDOW, size - WINDOW);
    fs.closeSync(fd);
    const s = buf.toString("utf8");
    return s.slice(s.indexOf("\n") + 1); // drop the partial first line
}

// Filter FIRST, then keep the last n survivors — so `muxlog bridge`/`grep`
// returns the last n *matching* lines, not "the last n lines, if any match".
function printLastLines(file, n, opt, whole = false) {
    const rendered = [];
    for (const l of readForDisplay(file, whole).split("\n")) {
        const r = renderLine(l, opt); if (r != null) rendered.push(r);
    }
    const out = n > 0 ? rendered.slice(-n) : rendered;
    for (const r of out) process.stdout.write(r + "\n");
}

function follow(file, opt) {
    let size; try { size = fs.statSync(file).size; } catch { size = 0; }
    setInterval(() => {
        let st; try { st = fs.statSync(file); } catch { return; }
        if (st.size < size) size = 0;            // truncated/rotated
        if (st.size === size) return;
        const fd = fs.openSync(file, "r");
        const buf = Buffer.alloc(st.size - size);
        fs.readSync(fd, buf, 0, buf.length, size);
        fs.closeSync(fd);
        size = st.size;
        for (const l of buf.toString("utf8").split("\n")) {
            const r = renderLine(l, opt); if (r != null) process.stdout.write(r + "\n");
        }
    }, 400);
}

// ─── ls ───────────────────────────────────────────────────────────────────────
function listInstances() {
    const seen = new Set();
    const rows = [];
    for (const e of discover("host").concat(discover("srv"), discover("launcher"))) {
        if (seen.has(e.file)) continue; seen.add(e.file);
        rows.push(e);
    }
    rows.sort((a, b) => b.mtime - a.mtime);
    if (!rows.length) { console.log("No AgentMux logs found under ~/.agentmux."); return; }
    console.log(`${"TARGET".padEnd(9)}${"VERSION".padEnd(9)}${"SOURCE".padEnd(22)}${"AGE".padEnd(8)}${"SIZE".padEnd(8)}PATH`);
    for (const e of rows) {
        console.log(
            e.target.padEnd(9) + e.version.padEnd(9) + e.source.slice(0, 21).padEnd(22) +
            age(e.mtime).padEnd(8) + human(e.size).padEnd(8) + e.file,
        );
    }
}
function age(ms) {
    const s = Math.floor((Date.now() - ms) / 1000);
    if (s < 90) return s + "s";
    if (s < 5400) return Math.floor(s / 60) + "m";
    if (s < 129600) return Math.floor(s / 3600) + "h";
    return Math.floor(s / 86400) + "d";
}
function human(b) { return b < 1024 ? b + "B" : b < 1048576 ? (b / 1024).toFixed(0) + "K" : (b / 1048576).toFixed(1) + "M"; }

// ─── mem / doctor ───────────────────────────────────────────────────────────
// The 2026-06-16 OOM crash was driven by SYSTEM COMMIT exhaustion across several
// concurrent AgentMux instances (+ dev tooling), not physical RAM. This surfaces
// commit-free, the derived pressure level, and the live AgentMux footprint so
// the multi-instance cause is visible BEFORE the cliff.
// See SPEC_MEMORY_PRESSURE_SUPERVISION_2026_06_16 §5.G + Discussion #943.

// Keep in lockstep with the host thresholds (agentmux-cef/src/memory_pressure.rs).
const WARN_FLOOR_MB = 1024;
const CRITICAL_FLOOR_MB = 512;

function pressureLevel(freeMb) {
    if (freeMb < CRITICAL_FLOOR_MB) return "critical";
    if (freeMb < WARN_FLOOR_MB) return "warn";
    return "normal";
}

// Best-effort, never throws — a missing tool / odd platform degrades gracefully.
function run(file, args) {
    try {
        return execFileSync(file, args, { encoding: "utf8", stdio: ["ignore", "pipe", "ignore"], timeout: 8000 });
    } catch { return ""; }
}

// System COMMIT (page file + RAM) in MB — the OOM-relevant ceiling, NOT physical
// RAM. Returns { freeMb, totalMb } or null when the platform has no cheap figure.
function systemCommit() {
    const plat = os.platform();
    if (plat === "win32") {
        // Win32_OperatingSystem reports Free/Total VirtualMemory in KB = commit.
        const out = run("powershell", [
            "-NoProfile", "-Command",
            "$o=Get-CimInstance Win32_OperatingSystem; $o.FreeVirtualMemory; $o.TotalVirtualMemorySize",
        ]);
        const n = out.trim().split(/\s+/).map(Number).filter((x) => Number.isFinite(x));
        if (n.length >= 2) return { freeMb: Math.round(n[0] / 1024), totalMb: Math.round(n[1] / 1024) };
        return null;
    }
    if (plat === "linux") {
        try {
            const info = fs.readFileSync("/proc/meminfo", "utf8");
            const kb = (k) => { const m = info.match(new RegExp("^" + k + ":\\s+(\\d+)", "m")); return m ? +m[1] : null; };
            const limit = kb("CommitLimit"), committed = kb("Committed_AS");
            if (limit != null && committed != null) {
                return { freeMb: Math.round((limit - committed) / 1024), totalMb: Math.round(limit / 1024) };
            }
        } catch { /* fall through */ }
        return null;
    }
    return null; // macOS / other: no cheap commit figure
}

// Live AgentMux processes: [{ name, pid, mb }] sorted by memory desc.
function agentmuxProcesses() {
    const procs = [];
    if (os.platform() === "win32") {
        // CSV cols: "Image","PID","Session","Session#","Mem Usage"
        for (const line of run("tasklist", ["/FO", "CSV", "/NH"]).split(/\r?\n/)) {
            const c = line.split('","').map((s) => s.replace(/^"|"$/g, ""));
            if (c.length < 5 || !/agentmux/i.test(c[0])) continue;
            const kb = parseInt(c[4].replace(/[^\d]/g, ""), 10) || 0; // "12,345 K"
            procs.push({ name: c[0], pid: c[1], mb: Math.round(kb / 1024) });
        }
    } else {
        for (const line of run("ps", ["-eo", "comm,pid,rss"]).split(/\r?\n/).slice(1)) {
            const m = line.trim().match(/^(\S+)\s+(\d+)\s+(\d+)$/);
            if (!m || !/agentmux/i.test(m[1])) continue;
            procs.push({ name: m[1], pid: m[2], mb: Math.round(+m[3] / 1024) });
        }
    }
    return procs.sort((a, b) => b.mb - a.mb);
}

function memDoctor() {
    console.log("AgentMux memory doctor\n");

    const commit = systemCommit();
    if (commit) {
        const usedPct = (((commit.totalMb - commit.freeMb) / commit.totalMb) * 100).toFixed(1);
        const lvl = pressureLevel(commit.freeMb);
        const badge = lvl === "critical" ? "CRITICAL" : lvl === "warn" ? "WARN" : "ok";
        console.log(`  system commit : ${commit.freeMb} MB free / ${commit.totalMb} MB   (${usedPct}% used)   → ${badge}`);
        if (lvl === "critical") console.log("                  an OOM is imminent — close some windows or other apps NOW.");
        else if (lvl === "warn") console.log("                  getting tight — closing a window or another app will help.");
    } else {
        console.log("  system commit : (unavailable on this platform — commit-limit exhaustion is a Windows/Linux concern)");
    }

    const procs = agentmuxProcesses();
    // `agentmux-srv` is the backend sidecar — ~one per instance (a crash-monitor
    // child may add one more), so it's a far more robust instance proxy than the
    // launcher exe, which gets renamed per portable build.
    const backends = procs.filter((p) => /srv/i.test(p.name)).length;
    const totalMb = procs.reduce((s, p) => s + p.mb, 0);
    console.log(`\n  agentmux procs: ${procs.length} (≈${backends} backend${backends === 1 ? "" : "s"}), ${totalMb} MB total working set`);
    for (const p of procs.slice(0, 24)) {
        console.log(`    ${String(p.pid).padStart(6)}  ${String(p.mb).padStart(6)} MB  ${p.name}`);
    }
    if (procs.length > 24) console.log(`    … and ${procs.length - 24} more`);

    if (commit && pressureLevel(commit.freeMb) !== "normal") {
        console.log(`\n  AgentMux is using ${totalMb} MB of commit — closing a window or another running AgentMux is the most direct way to free it.`);
    }
}

// ─── CLI ──────────────────────────────────────────────────────────────────────
function parse(argv) {
    const opt = { n: 200, all: false, raw: false, verbose: false };
    const pos = [];
    for (let i = 0; i < argv.length; i++) {
        const a = argv[i];
        if (a === "-a" || a === "--all") opt.all = true;
        else if (a === "--raw") opt.raw = true;
        else if (a === "-v" || a === "--verbose") opt.verbose = true;
        else if (a === "-n") opt.n = parseInt(argv[++i], 10) || 200;
        else if (a === "-i" || a === "--instance") opt.instance = argv[++i];
        else if (a === "--level") { const v = argv[++i]; if (v) opt.level = v.toLowerCase().split(","); }
        else if (a === "--target") opt.target = argv[++i];
        else if (a === "--exclude-target") opt.excludeTarget = argv[++i];
        else if (a === "--since") opt.since = argv[++i];
        else if (a === "--grep") opt.grep = new RegExp(argv[++i], "i");
        else pos.push(a);
    }
    return { opt, pos };
}

// Case-insensitive across file path, source label, and version — the
// original inline filter only lowercased `.file`, so an `-i` matching only
// via `.source`/`.version` (e.g. `-i Dev:Main`) silently missed real
// candidates. Exported for muxlog.test.mjs.
export function filterByInstance(cands, needle) {
    const n = needle.toLowerCase();
    return cands.filter(
        (e) => e.file.toLowerCase().includes(n) || e.source.toLowerCase().includes(n) || e.version.toLowerCase().includes(n),
    );
}

// $AGENTMUX_CHANNEL's dev-mode format is `dev-<branch>[-<clone_id>]`
// (hyphen-joined — agentmux-common's resolve_channel_and_dir) but never
// appears as a literal substring of a dev candidate's `.source`
// (`"dev:" + branch`, colon) or `.file` path (`dev/<branch>/...`,
// slash-separated) — same words, different separators, so
// filterByInstance()'s plain substring check never matches (reagent P1 on
// PR #2741: the own-channel default in pickCandidate silently never fired
// for `task dev`).
//
// A first attempt fixed that by stripping ALL separators and doing a plain
// substring check on the flattened result — reagent P1 round 2 caught the
// real bug in that: flattening loses word boundaries, so needle
// "dev-phase-3" (normalized "devphase3") is a substring-PREFIX of an
// unrelated sibling's "dev-phase-3-repro" (normalized "devphase3repro"),
// false-positive matching a genuinely different instance whose branch name
// happens to start with the same characters. No amount of delimiter-padding
// fixes this on its own, since needle being a true PREFIX of haystack's
// token sequence still satisfies a boundary-aligned check trivially — the
// fix has to require the needle match a COMPLETE identifying segment (or
// exact concatenation of segments), never a partial/prefix one.
//
// So: normalize each candidate field into discrete tokens split on real
// structural boundaries (`/`/`\` for the file path, `:` for the source
// label's own prefix), and require EXACT equality against one of those
// tokens — or, for the file path's `dev/<branch>/<clone_id>/logs/...`
// shape specifically, exact equality against two CONSECUTIVE tokens joined
// (branch + clone_id) — never a substring/prefix match against a flattened
// blob. `-i` (filterByInstance) is untouched by any of this — a user
// typing `-i fix-shell` still gets plain, literal substring matching, same
// as always.
function normalizeToken(s) {
    return s.toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/^-+|-+$/g, "");
}

export function matchesOwnChannel(cand, ownChannel) {
    const needle = normalizeToken(ownChannel);
    if (!needle) return false;

    // Source label: "dev:<branch>" / "channel:<name>" / "shared" — strip a
    // known "<word>:" prefix if present, exact-match what's left (with and
    // without a "dev-" prepended, since the channel string carries that
    // prefix but the source label's own identifying part doesn't).
    const sourceIdent = normalizeToken(cand.source.replace(/^[a-z]+:/i, ""));
    if (sourceIdent && (needle === sourceIdent || needle === `dev-${sourceIdent}`)) return true;

    // File path: split on real path separators only (never on '-' — a
    // branch/channel name's internal hyphens are part of ONE segment, not
    // segment boundaries). Check each segment alone, and each consecutive
    // pair joined (the dev-mode branch+clone_id shape), always as exact
    // equality, never substring.
    const segments = cand.file.split(/[\\/]/).map(normalizeToken).filter(Boolean);
    for (let i = 0; i < segments.length; i++) {
        const seg = segments[i];
        if (needle === seg || needle === `dev-${seg}`) return true;
        if (i + 1 < segments.length) {
            const pair = `${seg}-${segments[i + 1]}`;
            if (needle === pair || needle === `dev-${pair}`) return true;
        }
    }
    return false;
}

// Resolving "the" srv/host log used to mean "freshest across every instance
// on the machine" — with several instances at the same version routinely
// running at once (dev branches, portables, channels), that's frequently the
// WRONG instance, and nothing said so (see
// docs/reports/REPORT_MUXSPECT_MUXLOG_CROSS_CHANNEL_INSPECTION_2026_08_22.md
// §2.1: `swarm` resolved to a stale sibling version's log on the very first
// live repro of this). An explicit `-i` always wins, same as before. Absent
// that, prefer OUR OWN running instance — $AGENTMUX_CHANNEL is already in
// every agent pane's environment (same source resolveBlockId below already
// trusts for $AGENTMUX_BLOCKID), so a caller running from inside an agent
// pane gets ITS OWN instance's log by default instead of a same-version
// sibling's. This is a soft preference, not a hard requirement: falls
// through to the old "freshest overall" behavior when nothing matches
// (`launcher` has no per-channel log at all; an older srv build predating
// Ext 1's AGENTMUX_LOG_DIR fix still only writes to the shared root) rather
// than failing outright — degrading gracefully beats refusing to run.
//
// Pure (no I/O, no process.exit) — takes already-discovered candidates so
// muxlog.test.mjs can exercise the actual selection logic without touching
// the real filesystem/HOME. Returns `null` when nothing matches at all;
// `resolveFile` (below) owns turning that into the CLI's error+exit.
export function pickCandidate(cands, opt, ownChannel) {
    if (opt.instance) {
        const filtered = filterByInstance(cands, opt.instance);
        return filtered[0]?.file ?? null;
    }
    if (ownChannel) {
        const own = cands.filter((c) => matchesOwnChannel(c, ownChannel));
        if (own.length) return own[0].file;
    }
    return cands[0]?.file ?? null; // most recently active, no preference matched
}

function resolveFile(target, opt) {
    const cands = discover(target);
    const file = pickCandidate(cands, opt, process.env.AGENTMUX_CHANNEL);
    if (!file) {
        console.error(
            `muxlog: no ${target} log found${opt.instance ? ` matching '${opt.instance}'` : ""}. Try \`muxlog ls\`.`,
        );
        process.exit(1);
    }
    return file;
}

// ─── phases ───────────────────────────────────────────────────────────────────
// Unified turn-phase timeline for ONE agent pane, merged chronologically across
// the frontend's `[wave-turn]` transition log (host) and the backend's
// `[health] turn_active flip` log (srv) — the two separate files an
// investigator previously had to cross-reference by hand to answer "why did
// this pane show Working". See
// docs/specs/SPEC_AGENT_TURN_PHASE_TIMELINE_LOGGING_2026_08_18.md.
//
// Block-id matching differs by source: `[wave-turn]` embeds only a 7-char
// PANE PREFIX in its plain message text (`pane=abc1234`, from
// `blockId.slice(0, 7)` in agent-pane-state-store.ts — there is no structured
// field for it, since the frontend's console-log pipe just forwards a
// space-joined string). `[health] turn_active flip` instead carries the FULL
// block id as a structured tracing field (`fields.block_id`). Both are
// checked via a custom `matcher`, since `renderLine`'s generic `--grep`/
// `--target` only ever look at the message text, never at arbitrary
// structured fields like `block_id`.
//
// The recipe's own `matcher` (which pane/block this line is even about) is
// NON-NEGOTIABLE — always applied, never overridden — but every OTHER
// generic muxlog option the top-level help documents as applying "any
// position" (`--grep`, `--level`, `--target`, `--exclude-target`, `-a`)
// still needs to compose on top of it, the same way `swarm`/`auth`/`bridge`
// compose theirs via `renderLine`/`printLastLines` (reagent P2 on PR #2653 —
// `phases` originally bypassed all of these silently). A user's `--grep`
// ADDS an extra AND-filter here rather than replacing the recipe's own
// matcher (unlike `auth`'s `opt.grep || default`) — replacing it would
// defeat the entire point of `phases`, which is "only lines about this one
// block."
function collectPhaseLines(file, opt, matcher) {
    const out = [];
    for (const l of readForDisplay(file, true).split("\n")) {
        const t = l.trim();
        if (!t) continue;
        let j;
        try { j = JSON.parse(t); } catch { continue; }
        const fields = j.fields || {};
        const msg = String(fields.message ?? j.message ?? "");
        const target = j.target || "";
        const level = (j.level || "INFO").toUpperCase();
        if (!opt.all && /blockcontroller::subprocess|subprocess stdout → blockfile/.test(target + " " + msg)) continue;
        if (opt.level && !opt.level.includes(level.toLowerCase())) continue;
        if (opt.target && !target.includes(opt.target)) continue;
        if (opt.excludeTarget && target.includes(opt.excludeTarget)) continue;
        if (opt.grep && !opt.grep.test(msg)) continue;
        if (!matcher(msg, fields)) continue;
        if (opt.since && j.timestamp && j.timestamp < opt.since) continue;
        out.push({ ts: j.timestamp || "", msg, fields, raw: t });
    }
    return out;
}

const PHASE_SOURCE_COLOR = { fe: "\x1b[36m", srv: "\x1b[35m" }; // cyan / magenta

// [health] turn_active flip's own useful payload lives in structured fields
// (agentmux-srv/src/backend/blockcontroller/health.rs: `active`, `was_active`,
// `exit_code`), NOT in the message text — which is the same static string
// "[health] turn_active flip" every time. Surfacing only `entry.msg` (as an
// earlier version of this recipe did) made every non-`--raw` srv line in the
// timeline render identically regardless of whether the turn became active
// or inactive — the exact fact the merged timeline exists to expose (reagent
// P1 on PR #2653). Always shown for srv lines (not gated behind --verbose,
// unlike the generic renderLine path) since it's the entire reason srv lines
// are in this recipe's timeline at all.
function extraFieldsSuffix(fields) {
    const shown = { ...fields };
    delete shown.message;
    delete shown.block_id; // redundant — already implied by the block filter
    const keys = Object.keys(shown);
    return keys.length ? `  ${DIM}${keys.map((k) => `${k}=${shown[k]}`).join(" ")}${RESET}` : "";
}

function renderPhaseLine(entry, opt) {
    if (opt.raw) return entry.raw;
    const ts = (entry.ts || "").replace(/^.*T/, "").replace(/\..*$/, "") || "--:--:--";
    const c = PHASE_SOURCE_COLOR[entry.source] ?? "";
    const suffix = entry.source === "srv" ? extraFieldsSuffix(entry.fields) : "";
    return `${DIM}${ts}${RESET} ${c}${entry.source.padEnd(3)}${RESET}  ${entry.msg}${suffix}`;
}

function resolveBlockId(pos) {
    const id = pos[1] || process.env.AGENTMUX_BLOCKID;
    if (!id) {
        console.error(
            "muxlog: `phases` needs a block id — pass one explicitly (`muxlog phases <block-id>`) " +
            "or run this from inside an agent's own shell, where $AGENTMUX_BLOCKID is already set.",
        );
        process.exit(1);
    }
    return id;
}

// Whether `file` actually contains at least one line matching BOTH `tag`
// (e.g. "[wave-turn]") and `needle` (a pane prefix or full block id).
// Content presence is the ground truth for "is this the right log for this
// pane" — filename metadata (source/version) is only ever a search-order
// hint below, never sufficient on its own (reagent P1 x2 on PR #2653; see
// resolvePhaseFiles's own doc comment for the two concrete failure modes
// that motivated this).
function fileContainsPane(file, tag, needle) {
    let content = "";
    try { content = fs.readFileSync(file, "utf8"); } catch { return false; }
    return content.includes(tag) && content.includes(needle);
}

// Resolve the ONE host+srv log pair that actually belongs to `blockId`'s
// running instance. Two correlation bugs, both caught live by reagent
// review while this recipe was under review, motivate doing this by
// CONTENT rather than by filename metadata:
//
//   1. Picking `hostCands[0]` (globally most-recently-active host log,
//      respecting only `-i`) without checking it actually contains the
//      requested block id: with several instances running, the caller's
//      OWN pane (the common case — default $AGENTMUX_BLOCKID) can easily
//      NOT be in whichever instance happens to be most recently active
//      overall, silently resolving to the wrong instance's host log (and,
//      via that wrong host, the wrong srv pairing too) and reporting "no
//      lines found" instead of the real timeline — defeating the "zero
//      setup self-audit" the recipe exists to provide.
//   2. Correlating srv→host by `(source, version)` filename metadata alone:
//      `source` for a dev build is `"dev:" + <branch>` only — it drops the
//      `<hash>` build-directory segment, so two retained dev builds of the
//      SAME branch at the SAME version are indistinguishable by this key,
//      and the mtime-sorted first match can be an unrelated build's srv
//      log. (`source` alone, with no version check at all, has an even
//      coarser version of the same problem: the shared root holds several
//      version-tagged pairs at once.)
//
// Fix: content presence is the ground truth for both. `source`/`version`
// are used ONLY to order candidates cheaply (try the most-likely pairing
// first) before falling back to a full scan — never as the sole decision.
function resolvePhaseFiles(pos, opt) {
    const blockId = resolveBlockId(pos);
    const prefix = `pane=${blockId.slice(0, 7)}`;

    let hostCands = discover("host");
    // Case-insensitive filterByInstance (see its own doc comment) — the
    // content-verification scan just below is the real correctness backstop
    // for `phases`, so this is only ever an ordering hint, same as before.
    if (opt.instance) hostCands = filterByInstance(hostCands, opt.instance);
    if (!hostCands.length) {
        console.error(`muxlog: no host log found${opt.instance ? ` matching '${opt.instance}'` : ""}. Try \`muxlog ls\`.`);
        process.exit(1);
    }
    // Scan newest-first for the first host log that actually contains this
    // pane. Falls back to the most-recent candidate (old behavior) only if
    // NONE contain it — the timeline query below still runs and reports a
    // clear "no lines found" naming the (real, if wrong) file it checked,
    // rather than crashing outright.
    const hostEntry = hostCands.find((e) => fileContainsPane(e.file, "[wave-turn]", prefix)) ?? hostCands[0];

    const srvCands = discover("srv");
    const bySourceVersion = srvCands.filter((e) => e.source === hostEntry.source && e.version === hostEntry.version);
    const bySource = srvCands.filter((e) => e.source === hostEntry.source);
    const srvEntry =
        bySourceVersion.find((e) => fileContainsPane(e.file, "[health]", blockId)) ??
        bySource.find((e) => fileContainsPane(e.file, "[health]", blockId)) ??
        srvCands.find((e) => fileContainsPane(e.file, "[health]", blockId)) ??
        // No srv log anywhere contains this block id yet — not necessarily
        // an error (e.g. the pane opened but no turn has run since srv last
        // rotated). Fall back to the best-guess pairing so the timeline
        // still shows whatever fe-side lines exist.
        bySourceVersion[0] ?? bySource[0] ?? srvCands[0];
    if (!srvEntry) {
        console.error("muxlog: no srv log found. Try `muxlog ls`.");
        process.exit(1);
    }

    return { blockId, prefix, hostFile: hostEntry.file, srvFile: srvEntry.file };
}

function phasesTimeline(pos, opt) {
    const { blockId, prefix, hostFile, srvFile } = resolvePhaseFiles(pos, opt);

    const feLines = collectPhaseLines(hostFile, opt, (msg) => msg.includes("[wave-turn]") && msg.includes(prefix))
        .map((e) => ({ ...e, source: "fe" }));
    const srvLines = collectPhaseLines(srvFile, opt, (msg, fields) => msg.includes("[health]") && fields.block_id === blockId)
        .map((e) => ({ ...e, source: "srv" }));

    const merged = [...feLines, ...srvLines].sort((a, b) => (a.ts < b.ts ? -1 : a.ts > b.ts ? 1 : 0));
    if (!merged.length) {
        console.error(
            `muxlog phases: no [wave-turn]/[health] lines found for block ${blockId} in\n` +
            `  host: ${hostFile}\n  srv:  ${srvFile}`,
        );
        process.exit(1);
    }

    console.log(`=== phases: ${blockId} ===`);
    console.log(`    host: ${hostFile}`);
    console.log(`    srv:  ${srvFile}\n`);
    const tail = opt.n > 0 ? merged.slice(-opt.n) : merged;
    for (const e of tail) console.log(renderPhaseLine(e, opt));
}

const HELP = `muxlog — AgentMux log viewer

  muxlog [host|srv|launcher|fe|all] [tail|cat|grep <re>]   default: host tail (follow)
  muxlog ls                          list every instance's logs (newest first)
  muxlog mem                         system commit-free + pressure + live AgentMux procs
  muxlog errors                      ERROR/WARN across host+srv (active instance)
  muxlog bridge                      startup-handshake trace (debug reconnect loops)
  muxlog swarm                       subagent/swarm lifecycle trace (spawn/name/status, debug duplicate groups)
  muxlog auth                        provider auth/identity trace (login/OAuth wiring/unlink/account removal, credstate snapshots)
  muxlog phases [<block-id>]         merged turn-phase timeline (host [wave-turn] + srv [health]) for one pane — defaults to $AGENTMUX_BLOCKID

Options (any position):
  -i <substr>   pick the instance whose log path/branch/version matches <substr>
  -n <N>        history lines before following (default 200)
  -a            include agent-transcript noise (excluded by default)
  --grep <re>   filter on the message field only (not the whole JSON line)
  --level a,b   only these levels (error,warn,info,debug)
  --target <s>  only log lines whose target contains <s>
  --since <ts>  only lines at/after ISO <ts> (e.g. 2026-06-15T23:30)
  --raw         emit the original NDJSON   --verbose  include structured fields`;

function main() {
    const { opt, pos } = parse(process.argv.slice(2));
    const cmd = pos[0] || "host";

    if (cmd === "help" || cmd === "-h" || cmd === "--help") { console.log(HELP); return; }
    if (cmd === "ls") { listInstances(); return; }
    if (cmd === "mem" || cmd === "doctor") { memDoctor(); return; }

    if (cmd === "errors") {
        opt.level = ["error", "warn"];
        // Was its own ad-hoc `.file`-only filter with no own-channel default
        // at all — routed through the same pickCandidate() every other
        // recipe uses so `errors` gets the same "prefer my own instance"
        // behavior instead of "freshest anywhere" (see resolveFile's doc
        // comment above).
        for (const tgt of ["host", "srv"]) {
            const file = pickCandidate(discover(tgt), opt, process.env.AGENTMUX_CHANNEL);
            if (file) { console.log(`\n=== ${tgt}: ${file} ===`); printLastLines(file, opt.n, opt, true); }
        }
        return;
    }
    if (cmd === "swarm") {
        // Subagent/swarm lifecycle trace: spawn, display_name resolution
        // (subagent.GenerateName), status transitions (including
        // reconcile_stale_subagents' active->abandoned pass), and the
        // parent_block_id a subagent is bound to on backfill — the fields a
        // NAME-based grouping/dedup bug in the Swarm view needs, without
        // wading through the (usually much larger) agent-transcript log.
        // All emitted srv-side from subagent_watcher.rs's tracing target
        // (`agentmux_srv::backend::subagent_watcher`, rendered as
        // `srv:subagent_watcher` by shortTarget) — there is no host-side
        // subagent logging to combine in (checked: agentmux-cef has none).
        opt.target = opt.target || "subagent_watcher";
        const f = resolveFile("srv", opt);
        console.log(`=== swarm trace: ${f} ===`);
        printLastLines(f, opt.n, opt, true);
        return;
    }
    if (cmd === "auth") {
        // Provider auth / identity lifecycle trace. Unlike `swarm` (one module
        // → one tracing target), auth events span several srv modules —
        // server/identity_handlers.rs (auth.start/spawn/poll/cancel/submit*,
        // "auth success (direct-account)" persistence, OAuth config-dir
        // wiring), server/cli_handlers.rs (CheckCliAuth, "claude auth:"
        // credential seeding), identity/auth_session.rs (cancel_session,
        // session timeout), identity/resolver.rs ("oauth probe"), and the
        // logout side in server/app_api/identity.rs + agent_handlers/
        // identity.rs ("identity.unlink:", "identity.delete:"), plus the
        // layer-3 spawn gate in identity/resolver.rs
        // ("identity.spawn.blocked:", "identity.spawn.ambient:"), and the
        // login/logout-round credential-state diagnostics in
        // server/cli_handlers.rs ("auth.credstate:", a redacted
        // token-fingerprint snapshot of the checked dir) + the post-removal
        // verify in identity/cleanup.rs ("identity.delete: ... STILL PRESENT
        // after remove_dir_all") — so filter on the MESSAGE vocabulary, not a
        // target (opt.grep matches the message field only, exactly what we
        // want). A user --grep overrides the recipe's regex;
        // --target/--level/--since/-n still combine.
        opt.grep = opt.grep || /\bauth\.\w+|auth success|auth session|cancel_session|claude auth|CheckCliAuth|OAuth config dir|oauth probe|identity_upsert|identity\.(unlink|delete|self\.|account|spawn)|account\.oauth|keychain delete/i;
        const f = resolveFile("srv", opt);
        console.log(`=== auth trace: ${f} ===`);
        printLastLines(f, opt.n, opt, true);
        return;
    }
    if (cmd === "bridge") {
        // The startup handshake — correlate cred injection with bridge-init outcome.
        opt.grep = /Loading URL|Injected IPC|backend.?ready|window\.api|Bootstrap|setupCefApi|reconnect|on_load_end/i;
        opt.all = true;
        const f = resolveFile("host", opt);
        console.log(`=== bridge trace: ${f} ===`);
        printLastLines(f, opt.n, opt, true);
        return;
    }
    if (cmd === "phases") {
        // One-shot merged timeline, not a live follow — see phasesTimeline's
        // own doc comment for why host+srv need correlated instance
        // resolution and custom (not renderLine's generic) filtering.
        phasesTimeline(pos, opt);
        return;
    }

    const targets = ["host", "srv", "launcher", "fe", "all"];
    const validActions = ["tail", "cat", "grep"];
    // A first token that is neither a known recipe (help/ls/mem/errors/swarm/
    // auth/bridge/phases — all handled above and returned), a log target,
    // nor a bare action is an unknown command. Without this guard it silently falls
    // through to `follow(host)` below and tails the host log forever with no
    // filter — which is exactly how a typo, or `muxlog auth` run against a
    // build predating the auth recipe, appears to "hang". Fail fast instead.
    if (!targets.includes(cmd) && !validActions.includes(cmd)) {
        console.error(`muxlog: unknown command '${cmd}'. Run \`muxlog help\` for usage.`);
        process.exit(1);
    }
    const target = targets.includes(cmd) ? cmd : "host";
    const action = (targets.includes(cmd) ? pos[1] : pos[0]) || "tail";
    if (action === "grep") { const re = pos[pos.indexOf("grep") + 1]; if (re) opt.grep = new RegExp(re, "i"); }
    if (target === "fe") opt.grep = opt.grep || /\[fe\]/;

    const file = resolveFile(target === "fe" ? "host" : target === "all" ? "host" : target, opt);

    if (action === "cat" || action === "grep") { printLastLines(file, 0, opt, true); return; }
    // default: tail -f (print last -n lines, then follow)
    printLastLines(file, opt.n, opt);
    follow(file, opt);
}

// Only run when executed directly (`node muxlog.mjs ...`) — importing this
// module (muxlog.test.mjs imports `glob`) must not trigger a live tail loop /
// `process.exit`. Same pattern as muxspect.mjs.
if (import.meta.url === pathToFileURL(process.argv[1] ?? "").href) {
    main();
}
