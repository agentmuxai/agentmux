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

const HOME = os.homedir();
const AGENTMUX = path.join(HOME, ".agentmux");

// ─── Log discovery ────────────────────────────────────────────────────────────
// Logs can live in any of these roots (the shared dir alone is NOT enough — dev
// builds and per-build channels keep their host log in their own data dir):
//   ~/.agentmux/logs/                                  (shared: srv, launcher, some host)
//   ~/.agentmux/dev/<branch>/<hash>/logs/              (task dev, keyed on branch)
//   ~/.agentmux/channels/local-*/versions/<v>/.../logs/(portable/per-build)
function* logRoots() {
    yield { dir: path.join(AGENTMUX, "logs"), source: "shared" };
    for (const p of glob(path.join(AGENTMUX, "dev", "*", "*", "logs")))
        yield { dir: p, source: "dev:" + p.split(path.sep).at(-3) };
    for (const p of glob(path.join(AGENTMUX, "channels", "*", "versions", "*", "*", "logs")))
        yield { dir: p, source: "channel:" + p.split(path.sep).at(-5) };
}

// Tiny one-level-per-`*` glob (no deps). Only handles `*` path segments.
function glob(pattern) {
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
    try { j = JSON.parse(t); } catch { return opt.raw ? t : t; } // non-JSON: pass through
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
    const all = discover("all").concat(discover("host"), discover("srv"), discover("launcher"));
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
        else if (a === "--level") opt.level = argv[++i].toLowerCase().split(",");
        else if (a === "--target") opt.target = argv[++i];
        else if (a === "--exclude-target") opt.excludeTarget = argv[++i];
        else if (a === "--since") opt.since = argv[++i];
        else if (a === "--grep") opt.grep = new RegExp(argv[++i], "i");
        else pos.push(a);
    }
    return { opt, pos };
}

function resolveFile(target, opt) {
    let cands = discover(target);
    if (opt.instance) cands = cands.filter((e) => e.file.toLowerCase().includes(opt.instance.toLowerCase()) || e.source.includes(opt.instance) || e.version.includes(opt.instance));
    if (!cands.length) {
        console.error(`muxlog: no ${target} log found${opt.instance ? ` matching '${opt.instance}'` : ""}. Try \`muxlog ls\`.`);
        process.exit(1);
    }
    return cands[0].file; // most recently active
}

const HELP = `muxlog — AgentMux log viewer

  muxlog [host|srv|launcher|fe|all] [tail|cat|grep <re>]   default: host tail (follow)
  muxlog ls                          list every instance's logs (newest first)
  muxlog errors                      ERROR/WARN across host+srv (active instance)
  muxlog bridge                      startup-handshake trace (debug reconnect loops)

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

    if (cmd === "errors") {
        opt.level = ["error", "warn"];
        for (const tgt of ["host", "srv"]) {
            const f = discover(tgt).filter((e) => !opt.instance || e.file.includes(opt.instance))[0];
            if (f) { console.log(`\n=== ${tgt}: ${f.file} ===`); printLastLines(f.file, opt.n, opt, true); }
        }
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

    const targets = ["host", "srv", "launcher", "fe", "all"];
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

main();
