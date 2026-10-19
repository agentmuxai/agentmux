// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// An A/B run of the lab: the same flags on both sides, the runs interleaved (A B A B ...) so the machine's drift
// (temperature, background load, a disk filling) falls on both alike. The nightly performance check uses the same code.

const SIDE_KEYS = ["portable", "srv", "srvCommit", "label"];

/** Parses `ab.mjs` arguments: its own options before a lone `--`, and the flags both sides get after it. */
export function parseAbArgs(argv) {
    const dash = argv.indexOf("--");
    const own = dash >= 0 ? argv.slice(0, dash) : argv;
    const common = dash >= 0 ? argv.slice(dash + 1) : [];
    const spec = { runs: 4, order: "ab", out: "ab-out", common, a: { env: [] }, b: { env: [] } };
    for (let i = 0; i < own.length; i++) {
        const raw = own[i];
        const value = () => {
            const v = own[++i];
            if (v === undefined) throw new Error(`${raw} needs a value`);
            return v;
        };
        const m = /^--(a|b)-(portable|srv|srv-commit|label|env)$/.exec(raw);
        if (m) {
            const [, side, what] = m;
            if (what === "env") spec[side].env.push(value());
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
    for (const pair of side.env) args.push("--env", pair);
    return [...args, ...spec.common];
}

/** Which labels of each side finished (their result.json exists), and whether the comparison can be made. */
export function outcomeOf(plan, finished) {
    const done = { a: [], b: [] };
    for (const run of plan) if (finished.has(run.label)) done[run.side].push(run.label);
    return { done, comparable: done.a.length > 0 && done.b.length > 0 };
}
