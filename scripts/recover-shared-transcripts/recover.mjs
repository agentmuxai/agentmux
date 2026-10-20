#!/usr/bin/env node
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Offline recovery for the shared transcript store (~/.agentmux/shared/agents/
// transcripts/filestore.db). A development script, not shipped with the app. See
// README.md. The original store is only ever read (to copy it); everything else
// happens on copies in --work.

import { copyFileSync, existsSync, mkdirSync, openSync, readSync, closeSync, statSync } from "node:fs";
import { homedir } from "node:os";
import { join, resolve } from "node:path";
import { agentOfZone, DERIVED, findDamage, findNulHoles, lostAndFound } from "./lib/analyze.mjs";
import { checkWorkDir, copyNames, freeBytes, homeOf, locateStore, requiredBytes, runningInstances, workPath } from "./lib/preflight.mjs";
import { dropFile, outputTemplate, rebuildZone } from "./lib/rebuild.mjs";
import { writeReport } from "./lib/report.mjs";
import { atLeast, findSqlite3, integrityCheck, MIN_SQLITE, open, recover, sqliteVersionOfNode } from "./lib/sqlite.mjs";
import { findSources, readSource } from "./lib/sources.mjs";

const USAGE = `Usage: node scripts/recover-shared-transcripts/recover.mjs --work <folder> [options]

  --work <folder>       where the copies and the report go (needs about 2x the store's size free)
  --source <filestore.db>  the store (default: ~/.agentmux/shared/agents/transcripts/filestore.db)
  --home <folder>       the AgentMux home holding the per-channel stores (default: derived from --source)
  --sqlite3 <path>      the sqlite3 shell, ${MIN_SQLITE.join(".")} or newer (default: $SQLITE3, then PATH)
  --apply               rebuild the damaged zones in the recovered file (default: dry run, nothing is rebuilt)
  --recovered <file>    use an existing recovered database instead of copying and recovering again
  --skip-recover        dry run only: analyse the copy itself, without .recover (fast, for a store that still opens)
  --allow-shorter       rebuild a zone even when the per-channel copy is shorter than the zone
  --skip-content-scan   do not read every transcript part looking for zero-filled holes (NUL bytes)
  --allow-running       do not refuse when AgentMux processes are running (the copy may then be inconsistent)
`;

function parseArgs(argv) {
    const a = { apply: false, skipRecover: false, allowShorter: false, allowRunning: false, skipContentScan: false };
    const takes = { "--work": "work", "--source": "source", "--home": "home", "--sqlite3": "sqlite3", "--recovered": "recovered" };
    const flags = { "--apply": "apply", "--skip-recover": "skipRecover", "--allow-shorter": "allowShorter", "--allow-running": "allowRunning", "--skip-content-scan": "skipContentScan" };
    for (let i = 0; i < argv.length; i++) {
        if (takes[argv[i]]) a[takes[argv[i]]] = argv[++i];
        else if (flags[argv[i]]) a[flags[argv[i]]] = true;
        else throw new Error(`unknown argument ${argv[i]}\n\n${USAGE}`);
    }
    if (!a.work) throw new Error(`--work is required\n\n${USAGE}`);
    if (a.apply && a.skipRecover) throw new Error("--apply needs a recovered database: drop --skip-recover");
    return a;
}

const log = (s) => console.log(`${new Date().toISOString().slice(11, 19)} ${s}`);

/** `PRAGMA user_version` is a 4-byte integer at offset 60 of the database header. */
function headerUserVersion(file) {
    const fd = openSync(file, "r");
    try {
        const buf = Buffer.alloc(4);
        readSync(fd, buf, 0, 4, 60);
        return buf.readUInt32BE(0);
    } finally {
        closeSync(fd);
    }
}

const fingerprint = (store) => store.files.map((f) => (existsSync(f.path) ? `${statSync(f.path).size}:${statSync(f.path).mtimeMs}` : "absent"));

export async function main(argv) {
    const args = parseArgs(argv);
    const source = args.source ?? join(homedir(), ".agentmux", "shared", "agents", "transcripts", "filestore.db");
    const store = locateStore(source);
    const home = resolve(args.home ?? homeOf(store.db));
    const work = resolve(args.work);
    checkWorkDir(work, store);

    const running = runningInstances();
    if (running.lines.length && !args.allowRunning) {
        console.error(`AgentMux is running; a copy taken now can be inconsistent. Quit every instance first (or pass --allow-running):\n${running.lines.slice(0, 5).join("\n")}`);
        return 2;
    }
    const cli = findSqlite3(args.sqlite3);
    if (!cli.version || !atLeast(cli.version, MIN_SQLITE)) {
        console.error(`the sqlite3 shell is ${cli.text}; ${MIN_SQLITE.join(".")} or newer is needed (older ones carry the WAL-reset bug)`);
        return 2;
    }
    const need = requiredBytes(store);
    if (!args.recovered && freeBytes(work) < need) {
        console.error(`not enough room under ${work}: ${(freeBytes(work) / 2 ** 30).toFixed(1)} GB free, about ${(need / 2 ** 30).toFixed(1)} GB needed`);
        return 2;
    }
    mkdirSync(work, { recursive: true });

    const before = fingerprint(store);
    const report = {
        mode: args.apply ? "apply" : "dry run",
        startedAt: new Date().toISOString(),
        source: { db: store.db, files: store.files },
        work,
        home,
        tools: { sqlite3: cli.text.split(" ")[0], nodeSqlite: sqliteVersionOfNode(), node: process.version },
        allowRunning: args.allowRunning,
        originalsUnchanged: null,
        copy: null,
        integrity: null,
        recover: null,
        recoveredIntegrity: null,
        lostAndFound: null,
        userVersion: null,
        inventory: { files: 0, zones: 0 },
        damage: [],
        plan: [],
        final: null,
    };

    // 1-3. Copy, check, recover.
    let working;
    if (args.recovered) {
        working = resolve(args.recovered);
        report.recover = { reused: true, file: working, ms: 0 };
        log(`using the recovered database ${working}`);
    } else {
        const started = Date.now();
        let bytes = 0;
        for (const f of copyNames(store)) {
            log(`copying ${f.name}`);
            copyFileSync(f.from, workPath(work, f.name));
            bytes += statSync(workPath(work, f.name)).size;
        }
        report.copy = { ms: Date.now() - started, bytes };
        const copy = workPath(work, "filestore.db");
        log("integrity_check on the copy (this can take a while)");
        report.integrity = integrityCheck(cli, copy);
        log(`the copy is ${report.integrity.ok ? "intact" : "damaged"} (${report.integrity.lines.length} line(s))`);
        if (args.skipRecover) {
            working = copy;
            report.recover = { reused: false, file: copy, ms: 0, skipped: true };
        } else {
            working = workPath(work, "recovered.db");
            if (existsSync(working)) throw new Error(`${working} already exists; remove it or pass --recovered`);
            log("sqlite3 .recover into recovered.db (this can take a long time)");
            const r = await recover(cli, copy, working);
            report.recover = { reused: false, file: working, ms: r.ms, stderr: r.stderr.slice(0, 2000) };
            report.recoveredIntegrity = integrityCheck(cli, working);
            log(`recovered database integrity: ${report.recoveredIntegrity.ok ? "ok" : "NOT ok"}`);
        }
    }

    // The recovered file can lose the header's schema version; the original's is the truth.
    const original = headerUserVersion(store.db);
    {
        const db = open(working, { write: args.apply });
        let recovered = Number(db.prepare("PRAGMA user_version").get().user_version);
        let fixed = false;
        if (recovered !== original && args.apply) {
            db.exec(`PRAGMA user_version = ${original}`);
            recovered = original;
            fixed = true;
        }
        report.userVersion = { original, recovered, fixed };
        db.close();
    }

    // 4. Find the damage.
    const db = open(working, { write: args.apply });
    try {
        report.lostAndFound = lostAndFound(db);
        const found = findDamage(db);
        report.damage = found.damage;
        if (!args.skipContentScan) {
            log("looking for zero-filled holes in the transcripts");
            for (const hole of findNulHoles(db)) {
                const known = found.damage.find((d) => d.zone === hole.zone && d.name === hole.name);
                const detail = `${hole.parts} part(s) hold NUL bytes, from part ${hole.first}`;
                if (known) known.detail += `; ${detail}`;
                else {
                    const f = db.prepare("SELECT size FROM db_wave_file WHERE zoneid = ? AND name = ?").get(hole.zone, hole.name);
                    const bytes = Number(db.prepare("SELECT COALESCE(SUM(LENGTH(data)), 0) AS n FROM db_file_data WHERE zoneid = ? AND name = ?").get(hole.zone, hole.name).n);
                    found.damage.push({ zone: hole.zone, name: hole.name, kind: "nul-bytes", detail, size: f ? Number(f.size) : null, partsBytes: bytes, prefixBytes: 0 });
                }
            }
        }
        report.inventory = { files: found.files, zones: found.zones };
        log(`${found.files} file(s) in ${found.zones} zone(s); ${found.damage.length} damaged`);

        // 5. Plan: which damaged files can be rebuilt from which per-channel copy.
        const wanted = new Set(found.damage.map((d) => (d.name === "output" ? agentOfZone(d.zone) : null)).filter(Boolean));
        const sources = wanted.size ? findSources(home, wanted) : { dataDirsScanned: 0, byAgent: new Map() };
        if (wanted.size) log(`looked for ${wanted.size} agent(s)' transcripts in ${sources.dataDirsScanned} channel store(s)`);
        const rebuilt = new Set();
        for (const d of found.damage) {
            const item = { zone: d.zone, name: d.name, kind: d.kind };
            const defId = d.name === "output" ? agentOfZone(d.zone) : null;
            if (defId) {
                const best = sources.byAgent.get(defId)?.best ?? null;
                const zoneBytes = Math.max(d.size ?? 0, d.partsBytes);
                item.source = best;
                if (!best) item.action = "no-source";
                else if (best.size < zoneBytes && !args.allowShorter) {
                    item.action = "source-shorter";
                    item.note = `Copy ${best.size} bytes, zone ${zoneBytes}.`;
                } else {
                    item.action = "rebuild";
                    if (best.size < zoneBytes) item.note = `Copy is ${zoneBytes - best.size} bytes shorter than the zone (--allow-shorter).`;
                    rebuilt.add(d.zone);
                }
            } else if (DERIVED.has(d.name)) item.action = "drop-derived";
            else item.action = "manual";
            report.plan.push(item);
        }
        // A derived file of a zone that is being rebuilt goes with it.
        for (const p of report.plan) if (p.action === "drop-derived" && rebuilt.has(p.zone)) p.note = "Dropped with the rebuild of this zone's output.";

        // 6. Apply.
        if (args.apply) {
            const template = outputTemplate(db);
            const now = Date.now();
            for (const p of report.plan) {
                try {
                    if (p.action === "rebuild") {
                        log(`rebuilding ${p.zone}`);
                        rebuildZone(db, p.zone, readSource(p.source), now, template);
                        p.applied = true;
                    } else if (p.action === "drop-derived") {
                        if (!rebuilt.has(p.zone)) dropFile(db, p.zone, p.name);
                        p.applied = true;
                    }
                } catch (e) {
                    p.applied = false;
                    p.error = String(e.message ?? e);
                }
            }
            report.remainingDamage = findDamage(db).damage;
        }
    } finally {
        db.close();
    }
    if (args.apply) {
        log("integrity_check on the rebuilt database");
        report.final = { file: working, bytes: statSync(working).size, integrity: integrityCheck(cli, working) };
    }

    report.originalsUnchanged = JSON.stringify(before) === JSON.stringify(fingerprint(store));
    writeReport(work, report);
    log(`report: ${join(work, "report.md")}`);
    // With --allow-running the instances writing to the store are expected to change it.
    return report.originalsUnchanged || args.allowRunning ? 0 : 1;
}

if (import.meta.url === `file://${process.argv[1]}` || process.argv[1]?.endsWith("recover.mjs")) {
    main(process.argv.slice(2)).then(
        (code) => process.exit(code),
        (e) => {
            console.error(e.message ?? e);
            process.exit(1);
        },
    );
}
