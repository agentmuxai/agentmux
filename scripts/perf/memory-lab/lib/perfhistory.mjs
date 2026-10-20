// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The nightly performance check's history (SPEC_NIGHTLY_PERF_REGRESSION §4.6, §4.9): one JSON line a night in
// `perf/history.jsonl` on the `perf-data` branch, the Mermaid charts of the last 30 nights for the job summary, and the
// drift check. Pure functions; `nightly.mjs history` reads and writes the file. The line's field names are never
// renamed, only added: the nightly email reads the last line.

/** The measures the charts draw and the drift check watches, with the unit shown on the axis. */
export const MAIN_MEASURES = {
    app_mb: "MB",
    app_cpu_s: "s",
    srv_cpu_s: "s",
    round_ms: "ms",
    srv_written_mb: "MB",
};

export const CHART_NIGHTS = 30;
export const DRIFT_RECENT = 7;
export const DRIFT_BEFORE = 30;
export const DRIFT_PCT = 15;
// Fewer nights than this and a median says nothing: no drift is reported.
const MIN_RECENT = 5;
const MIN_BEFORE = 10;

const round = (x, places) => (x == null ? null : Math.round(x * 10 ** places) / 10 ** places);

export function median(values) {
    const v = values.filter((x) => x != null && Number.isFinite(x)).sort((x, y) => x - y);
    if (!v.length) return null;
    const mid = v.length >> 1;
    return v.length % 2 ? v[mid] : (v[mid - 1] + v[mid]) / 2;
}

/**
 * One night's history line (§4.9).
 *
 * - `plan`: `nightly.mjs plan`'s plan.json (`a` is null on a night with no earlier build, which gets no measures);
 * - `final`: the compare.json over every run of the night (`measures`, `flags`, the labels of the runs used in `head`);
 * - `initial`: the first matrix's compare.json, to tell whether the confirmation pair added runs;
 * - `versions`: `{a, b}`, the app versions the portables' file names carry (informational: nightlies share a version);
 * - `workload`: `{version, panes, rounds}` from a run's result.json.
 */
export function historyLine({ date, run, plan, versions = {}, workload, initial = null, final = null, issue = null }) {
    const build = (r, v) => (r ? { commit: String(r.sha).slice(0, 9), version: v ?? null, artifacts_run_id: r.id } : null);
    const hasMeasures = Boolean(plan?.a && final);
    const measures = {};
    if (hasMeasures) {
        for (const [name, m] of Object.entries(final.measures ?? {})) {
            if (m) measures[name] = { a: round(m.a, 2), b: round(m.b, 2), change_pct: round(m.change_pct, 1), flag: Boolean(m.flag) };
        }
    }
    const flags = hasMeasures ? (final.flags ?? []).filter((n) => measures[n]) : [];
    const runsPerSide = final?.head?.length ?? 0;
    // A confirmation pair finished when both sides gained runs over the first matrix (a lone extra B run is not a pair).
    const paired = runsPerSide > (initial?.head?.length ?? 0) && (final?.base?.length ?? 0) > (initial?.base?.length ?? 0);
    return {
        date,
        run_id: run.id,
        run_url: run.url,
        a: build(plan?.a, versions.a),
        b: build(plan?.b, versions.b),
        workload: { version: workload?.version ?? null, panes: workload?.panes ?? null, rounds: workload?.rounds ?? null, runs_per_side: runsPerSide },
        measures,
        flags,
        confirmed: flags.length > 0 && paired,
        issue,
    };
}

/** Whether a night's line is for the standard workload (what the nightly runs); a manual run with other flags is not recorded. */
export function isStandardWorkload(workload, standard = { panes: 12, rounds: 10 }) {
    return workload?.panes === standard.panes && workload?.rounds === standard.rounds;
}

/** The file's lines as objects; a line that does not parse is skipped, not fatal. */
export function parseHistory(text) {
    const lines = [];
    for (const raw of String(text ?? "").split(/\r?\n/)) {
        if (!raw.trim()) continue;
        try {
            lines.push(JSON.parse(raw));
        } catch {
            // a damaged line costs that night only
        }
    }
    return lines;
}

/** The file's text with `line` added; a line of the same run (a rerun of the workflow) is replaced, not repeated. */
export function appendHistory(text, line) {
    const kept = parseHistory(text).filter((l) => l.run_id !== line.run_id);
    kept.push(line);
    return kept.map((l) => JSON.stringify(l)).join("\n") + "\n";
}

/** The nights that can be compared with each other: one per date (the last), with measures, on the newest line's workload. */
function comparable(lines) {
    const withData = lines.filter((l) => l.a && l.measures && Object.keys(l.measures).length);
    const last = withData[withData.length - 1];
    if (!last) return [];
    const same = (l) => ["version", "panes", "rounds"].every((k) => l.workload?.[k] === last.workload?.[k]);
    const byDate = new Map();
    for (const l of withData.filter(same)) byDate.set(l.date, l);
    return [...byDate.values()].sort((x, y) => String(x.date).localeCompare(String(y.date)));
}

/**
 * The slow drift (§4.6): the main measures whose median over the last 7 nights is more than 15% above their median over the
 * 30 nights before, using each night's B figure. Needs at least 5 recent and 10 earlier nights. Returns
 * `[{ name, recent, before, changePct }]`.
 */
export function driftOf(lines, { recent = DRIFT_RECENT, before = DRIFT_BEFORE, pct = DRIFT_PCT } = {}) {
    const nights = comparable(lines);
    const last = nights.slice(-recent);
    const earlier = nights.slice(-recent - before, -recent);
    if (last.length < MIN_RECENT || earlier.length < MIN_BEFORE) return [];
    const out = [];
    for (const name of Object.keys(MAIN_MEASURES)) {
        const r = median(last.map((l) => l.measures[name]?.b));
        const e = median(earlier.map((l) => l.measures[name]?.b));
        if (r == null || !e) continue;
        const changePct = round(((r - e) / e) * 100, 1);
        if (changePct > pct) out.push({ name, recent: round(r, 2), before: round(e, 2), changePct });
    }
    return out;
}

/**
 * Which drifting measures are new tonight: those in tonight's drift that were not in last night's. A drift is said once, when it
 * starts, not every night it lasts. `prior` is the history without tonight's line.
 */
export function driftOnset(prior, tonight) {
    const already = new Set(driftOf(prior).map((d) => d.name));
    const now = driftOf([...prior, tonight]);
    return { current: now, onset: now.filter((d) => !already.has(d.name)) };
}

const label = (date) => String(date).slice(5);

/** One Mermaid `xychart-beta` of a measure over the last nights (B's figure each night); "" when fewer than two nights have it. */
export function chartOf(lines, name, nights = CHART_NIGHTS) {
    const points = comparable(lines)
        .slice(-nights)
        .map((l) => ({ date: l.date, value: l.measures[name]?.b }))
        .filter((p) => p.value != null);
    if (points.length < 2) return "";
    const values = points.map((p) => p.value);
    const lo = Math.min(...values);
    const hi = Math.max(...values);
    const pad = (hi - lo) * 0.2 || hi * 0.05 || 1;
    const min = Math.max(0, Math.floor(lo - pad));
    const max = Math.ceil(hi + pad);
    return [
        "```mermaid",
        "xychart-beta",
        `    title "${name} (${MAIN_MEASURES[name] ?? ""}), B each night, last ${points.length} nights"`,
        `    x-axis [${points.map((p) => `"${label(p.date)}"`).join(", ")}]`,
        `    y-axis "${MAIN_MEASURES[name] ?? name}" ${min} --> ${max}`,
        `    line [${values.map((v) => round(v, 2)).join(", ")}]`,
        "```",
    ].join("\n");
}

/** The job summary's trend section: the charts of the main measures, then the drift if there is one. */
export function trendMarkdown(lines, drift = []) {
    const parts = ["## Trend (the nightly history)"];
    const charts = Object.keys(MAIN_MEASURES).map((n) => chartOf(lines, n)).filter(Boolean);
    parts.push(charts.length ? charts.join("\n\n") : "Not enough nights with a comparison yet for a chart (it needs two).");
    if (drift.length) {
        parts.push(
            `Slow drift: the last ${DRIFT_RECENT} nights' median is more than ${DRIFT_PCT}% above the ${DRIFT_BEFORE} nights before for ${drift.map((d) => `${d.name} (+${d.changePct}%)`).join(", ")}.`
        );
    }
    return parts.join("\n\n");
}
