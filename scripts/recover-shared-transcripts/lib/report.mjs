// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// The report: report.json for tools, report.md for the person deciding what to do.

import { writeFileSync } from "node:fs";
import { join } from "node:path";

const mb = (n) => `${(n / (1024 * 1024)).toFixed(1)} MB`;

const ACTION_TEXT = {
    rebuild: "rebuild `output` from the per-channel copy",
    "drop-derived": "drop (the app rebuilds it)",
    "no-source": "NO per-channel copy found: left as it is",
    "source-shorter": "per-channel copy is SHORTER than the zone: left as it is (use --allow-shorter to rebuild anyway)",
    manual: "not touched: needs a look",
};

export function markdown(r) {
    const out = [];
    const line = (s = "") => out.push(s);
    line(`# Shared transcript store recovery (${r.mode})`);
    line();
    line(`Started ${r.startedAt}. Store: \`${r.source.db}\` (${mb(r.source.files.reduce((n, f) => n + f.size, 0))} with its WAL).`);
    line(`Work folder: \`${r.work}\`. Tools: sqlite3 ${r.tools.sqlite3}, node:sqlite ${r.tools.nodeSqlite}, node ${r.tools.node}.`);
    line();
    line("## Checks");
    const changed = r.allowRunning ? "changed while the script ran: AgentMux was running (--allow-running), so the copy may be inconsistent" : "NO, STOP AND LOOK";
    line(`- Originals untouched: **${r.originalsUnchanged === null ? "not checked yet" : r.originalsUnchanged ? "yes" : changed}** (size and modification time of the database, WAL and shm before and after).`);
    line(`- Copy: ${r.copy ? `${mb(r.copy.bytes)} in ${(r.copy.ms / 1000).toFixed(1)} s` : "skipped"}.`);
    line(`- Integrity of the copy: **${r.integrity ? (r.integrity.ok ? "ok" : "DAMAGED") : "not run"}**.`);
    if (r.integrity && !r.integrity.ok) {
        for (const l of r.integrity.lines.slice(0, 15)) line(`  - \`${l}\``);
        if (r.integrity.lines.length > 15) line(`  - ... ${r.integrity.lines.length - 15} more line(s) in report.json`);
    }
    if (r.recover) {
        line(`- \`.recover\`: ${r.recover.reused ? "reused " + r.recover.file : `${(r.recover.ms / 1000).toFixed(1)} s`}; integrity of the result: **${r.recoveredIntegrity ? (r.recoveredIntegrity.ok ? "ok" : "NOT OK") : "not checked"}**; \`lost_and_found\`: ${r.lostAndFound?.table ? `${r.lostAndFound.rows} row(s) it could not place` : "none"}.`);
    }
    if (r.userVersion) line(`- \`user_version\`: original ${r.userVersion.original}, recovered ${r.userVersion.recovered}${r.userVersion.fixed ? " (set back)" : ""}.`);
    line();
    line("## What is damaged");
    line(`${r.inventory.files} file(s) in ${r.inventory.zones} zone(s) checked; **${r.damage.length} damaged file(s)** in **${new Set(r.damage.map((d) => d.zone)).size} zone(s)**.`);
    if (r.damage.length) {
        line();
        line("| zone | file | what | recorded size | bytes in parts |");
        line("|---|---|---|---:|---:|");
        for (const d of r.damage) line(`| ${d.zone} | ${d.name} | ${d.kind}: ${d.detail} | ${d.size ?? "-"} | ${d.partsBytes} |`);
    }
    line();
    line("## Plan" + (r.mode === "apply" ? " (applied)" : " (nothing is changed in dry-run mode)"));
    if (!r.plan.length) line("Nothing to do.");
    for (const p of r.plan) {
        const src = p.source ? ` from \`${p.source.dataDir}\` (block ${p.source.blockOid}, ${mb(p.source.size)})` : "";
        const state = p.applied === undefined ? "" : p.applied ? " **done**" : ` **not done: ${p.error ?? "skipped"}**`;
        line(`- \`${p.zone}\` / \`${p.name}\`: ${ACTION_TEXT[p.action]}${src}.${p.note ? " " + p.note : ""}${state}`);
    }
    line();
    if (r.final) {
        line("## Result");
        line(`The rebuilt database is \`${r.final.file}\` (${mb(r.final.bytes)}); integrity: **${r.final.integrity.ok ? "ok" : "NOT OK"}**.`);
        line();
        line("To use it, with every AgentMux instance still stopped: move the old `filestore.db`, `filestore.db-wal` and `filestore.db-shm` aside (keep them), copy the rebuilt file in as `filestore.db` with no `-wal` or `-shm`, and start one instance first.");
    } else if (r.mode !== "apply") {
        line("Run again with `--apply --recovered <the recovered file>` to rebuild. The original store is never modified; the result is a new file in the work folder.");
    }
    return out.join("\n") + "\n";
}

export function writeReport(work, report) {
    writeFileSync(join(work, "report.json"), JSON.stringify(report, null, 2));
    writeFileSync(join(work, "report.md"), markdown(report));
}
