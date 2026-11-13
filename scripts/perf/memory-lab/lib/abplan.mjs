// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// An A/B run of the lab: the same flags on both sides, the runs interleaved (A B A B ...) so the machine's drift
// (temperature, background load, a disk filling) falls on both alike. The nightly performance check uses the same code.

import { join } from "node:path";

const SIDE_KEYS = ["portable", "srv", "srvCommit", "label"];

/** Parses `ab.mjs` arguments: its own options before a lone `--`, and the flags both sides get after it. */
export function parseAbArgs(argv) {
    const dash = argv.indexOf("--");
    const own = dash >= 0 ? argv.slice(0, dash) : argv;
    const common = dash >= 0 ? argv.slice(dash + 1) : [];
    const spec = { runs: 4, order: "ab", out: "ab-out", common, a: { env: [], setting: [] }, b: { env: [], setting: [] } };
    for (let i = 0; i < own.length; i++) {
        const raw = own[i];
        const value = () => {
            const v = own[++i];
            if (v === undefined) throw new Error(`${raw} needs a value`);
            return v;
        };
        const m = /^--(a|b)-(portable|srv|srv-commit|label|claude-version|env|setting)$/.exec(raw);
        if (m) {
            const [, side, what] = m;
            if (what === "env" || what === "setting") spec[side][what].push(value());
            else spec[side][what.replace(/-([a-z])/g, (_, c) => c.toUpperCase())] = value();
            continue;
        }
        switch (raw) {
            case "--portable":
                spec.portable = value();
                break;
            case "--runs":
                spec.runs = Number(value());
                break;
            case "--order":
                spec.order = value();
                break;
            case "--out":
                spec.out = value();
                break;
            case "--margins":
                spec.margins = value();
                break;
            default:
                throw new Error(`unknown argument ${raw} (flags for the lab itself go after a lone --)`);
        }
    }
    if (!Number.isInteger(spec.runs) || spec.runs < 1) throw new Error("--runs needs a whole number of 1 or more per side");
    if (!["ab", "ba"].includes(spec.order)) throw new Error('--order is "ab" (A first) or "ba" (B first)');
    for (const side of ["a", "b"]) spec[side].portable ??= spec.portable;
    if (!spec.a.portable || !spec.b.portable) throw new Error("give --portable <folder> for both sides, or --a-portable and --b-portable");
    for (const flag of ["--portable", "--label", "--out", "--srv", "--srv-commit", "--exe"]) {
        if (common.includes(flag)) throw new Error(`${flag} is set per side (or by ab.mjs), not in the flags both sides get`);
    }
    spec.a.label ??= "a";
    spec.b.label ??= "b";
    if (spec.a.label === spec.b.label) throw new Error("the two sides need different labels (--a-label, --b-label)");
    return spec;
}

/** The runs in the order they are made: A1 B1 A2 B2 ... (B first with order "ba"). Run labels are `<side label><n>`. */
export function planRuns(spec) {
    const sides = spec.order === "ba" ? ["b", "a"] : ["a", "b"];
    const plan = [];
    for (let n = 1; n <= spec.runs; n++) for (const side of sides) plan.push({ side, n, label: `${spec[side].label}${n}` });
    return plan;
}

/** The arguments of `run.mjs` for one planned run: the side's portable, srv and environment, then the flags both sides get. */
export function runArgs(run, spec) {
    const side = spec[run.side];
    const args = ["--portable", side.portable, "--label", run.label, "--out", spec.out];
    if (side.srv) args.push("--srv", side.srv);
    if (side.srvCommit) args.push("--srv-commit", side.srvCommit);
    if (side.claudeVersion) args.push("--claude-version", side.claudeVersion);
    for (const pair of side.env) args.push("--env", pair);
    for (const pair of side.setting) args.push("--setting", pair);
    return [...args, ...spec.common];
}

/** Which labels of each side finished (their result.json exists), and whether the comparison can be made. */
export function outcomeOf(plan, finished) {
    const done = { a: [], b: [] };
    for (const run of plan) if (finished.has(run.label)) done[run.side].push(run.label);
    return { done, comparable: done.a.length > 0 && done.b.length > 0 };
}

/** A side that has failed this many runs, none finished, is not run again: its remaining runs would fail the same way. */
export const GIVE_UP_AFTER = 2;

/** Whether `run` is left out because its side has failed `GIVE_UP_AFTER` runs and finished none. `ran` is the runs made so far, with `ok`. */
export function sideGivenUp(run, ran) {
    const own = ran.filter((r) => r.side === run.side);
    return own.length >= GIVE_UP_AFTER && own.every((r) => !r.ok);
}

/**
 * In words, why a side could not be measured, for the job summary: nothing when it finished at least one run.
 * `reasons` maps a run label to why it failed (the lab's own message); `skipped` are the labels not run.
 */
export function unrunnableSides(plan, outcome, reasons, skipped = [], commits = {}) {
    const lines = [];
    for (const side of ["a", "b"]) {
        if (outcome.done[side].length > 0) continue;
        const labels = plan.filter((r) => r.side === side).map((r) => r.label);
        const first = labels.map((l) => reasons[l]).find(Boolean);
        const tried = labels.filter((l) => !skipped.includes(l));
        const who = `${side.toUpperCase()}${commits[side] ? ` (build ${commits[side].slice(0, 9)})` : ""}`;
        const left = labels.length - tried.length;
        lines.push(`${who} did not run: ${first ?? "no run wrote a result"} (${tried.length} run${tried.length === 1 ? "" : "s"} tried${left ? `, ${left} not tried after those failed` : ""}).`);
    }
    return lines;
}

/** The arguments of `compare.mjs` for the finished runs: both sides' result files, the markdown and json outputs, and the margins file when given. */
export function compareArgs(spec, outcome) {
    const path = (label) => join(spec.out, label, "result.json");
    const args = ["--base", outcome.done.a.map(path).join(","), "--head", outcome.done.b.map(path).join(","), "--md", join(spec.out, "compare.md"), "--json", join(spec.out, "compare.json")];
    if (spec.margins) args.push("--margins", spec.margins);
    return args;
}
