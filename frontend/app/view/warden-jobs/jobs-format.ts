// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Words for the Jobs section (docs/specs/SPEC_JOBS_AND_TEMPLATES_2026_10_24.md §3.3): a cron
// expression in words, a run's badge, a job's state, a cost. Whether a run failed is srv's call
// (`failed`, §3.4); this only says it.

import type { PillTone } from "@/app/element/ui";
import type { DroppedEvents } from "@/types/rpc/DroppedEvents";
import type { JobRunRow } from "@/types/rpc/JobRunRow";
import type { JobView } from "@/types/rpc/JobView";

// Standard cron numbering, as srv reads it (backend/cron/expr.rs): 0 and 7 are Sunday, 1 is Monday.
const DAYS = ["Sundays", "Mondays", "Tuesdays", "Wednesdays", "Thursdays", "Fridays", "Saturdays"];
const DAY_NAMES = ["SUN", "MON", "TUE", "WED", "THU", "FRI", "SAT"];

/** A day-of-week field with its names (`MON`, `mon-fri`) as standard numbers. */
function dowNumbers(dow: string): string {
    return dow.replace(/[a-z]{3}/gi, (name) => {
        const i = DAY_NAMES.indexOf(name.toUpperCase());
        return i < 0 ? name : String(i);
    });
}

const two = (n: number) => String(n).padStart(2, "0");
const isNum = (s: string) => /^\d+$/.test(s);

/** A 5-field cron expression, read in `zone`, in words for the common shapes ("every 15 min", "daily
 *  06:00 UTC", "Mondays 06:00 UTC"); anything else stays the expression. */
export function describeCron(expression: string, zone: string = "UTC"): string {
    const f = expression.trim().split(/\s+/);
    if (f.length !== 5) return expression;
    const [min, hour, dom, mon] = f;
    const dow = dowNumbers(f[4]);
    const anyDay = dom === "*" && mon === "*";
    if (anyDay && dow === "*" && hour === "*") {
        if (min === "*") return "every minute";
        const every = /^\*\/(\d+)$/.exec(min);
        if (every) return `every ${every[1]} min`;
        if (isNum(min)) return `hourly at :${two(Number(min))}`;
    }
    if (anyDay && dow === "*" && isNum(min)) {
        const every = /^\*\/(\d+)$/.exec(hour);
        if (every) return `every ${every[1]} h at :${two(Number(min))}`;
    }
    if (!isNum(min) || !isNum(hour)) return expression;
    const time = `${two(Number(hour))}:${two(Number(min))} ${zone}`;
    if (anyDay && dow === "*") return `daily ${time}`;
    if (anyDay && dow === "1-5") return `weekdays ${time}`;
    if (anyDay && /^\d(,\d)*$/.test(dow) && dow.split(",").every((d) => Number(d) <= 7)) {
        const days = dow.split(",").map((d) => DAYS[Number(d) % 7]);
        if (days.every(Boolean)) return `${days.join(", ")} ${time}`;
    }
    if (mon === "*" && dow === "*" && isNum(dom)) return `monthly on day ${Number(dom)} ${time}`;
    return expression;
}

/** A job's trigger in words: its cron expression in its zone, "every 2 h", "once, <date>", "by hand". */
export function describeTrigger(job: Pick<JobView, "source" | "trigger_kind" | "expression" | "tz" | "every_secs" | "once_at_ms">): string {
    if (job.source === "drone") return "by hand";
    // A cloud job without a schedule starts when its reporter says so.
    if (job.source === "cloud" && job.trigger_kind === "manual") return "when it reports";
    switch (job.trigger_kind) {
        case "every": {
            const s = job.every_secs ?? 0;
            if (s % 3600 === 0 && s >= 3600) return `every ${s / 3600} h`;
            return s % 60 === 0 ? `every ${s / 60} min` : `every ${s} s`;
        }
        case "once":
            return job.once_at_ms != null ? `once, ${new Date(job.once_at_ms).toLocaleString()}` : "once";
        case "manual":
            return "by hand";
        default:
            return describeCron(job.expression, job.tz || "UTC");
    }
}

/** "in 3 h", "in 12 min", "in 40 s", or "now" for a time just reached. */
export function inWords(thenMs: number, nowMs: number): string {
    const s = Math.round((thenMs - nowMs) / 1000);
    if (s <= 0) return "now";
    if (s < 60) return `in ${s} s`;
    const m = Math.round(s / 60);
    if (m < 120) return `in ${m} min`;
    const h = Math.round(m / 60);
    return h < 48 ? `in ${h} h` : `in ${Math.round(h / 24)} d`;
}

/** Why a run was skipped, missed or stopped, in a few words (§3.1 `reason`). */
const REASON_TEXT: Record<string, string> = {
    paused: "paused",
    paused_all: "Pause all",
    app_not_running: "AgentMux was off",
    target_missing: "no target",
    target_stopped: "agent not running",
    superseded: "replaced by a newer fire",
    expired: "agent didn't come back in time",
    overlap: "previous run still going",
    group_busy: "waiting for its group",
    job_cap: "job's daily cap reached",
    account_cap: "account's daily cap reached",
    message_cap: "daily message limit reached",
    run_cap: "run cap reached",
    timeout: "timed out",
    no_report: "never reported",
};

export function reasonText(reason: string): string {
    return REASON_TEXT[reason] ?? reason;
}

/** An infrastructure status that isn't `ok`, in words (§3.1 `infra`). */
const INFRA_TEXT: Record<string, string> = {
    error: "error",
    timeout: "timed out",
    over_budget: "over budget",
    interrupted: "interrupted",
};

export interface RunBadge {
    label: string;
    tone: PillTone;
    /** Longer words for the tooltip. */
    detail: string;
    /** A skip: hollow in the strip. */
    hollow: boolean;
}

/** A run's badge: the outcome (ok, findings, failed, unknown) once it ran, else what kept it from
 *  running (missed, skipped, waiting, running). Red only when srv says it failed (§3.4). */
export function runBadge(run: JobRunRow): RunBadge {
    const why = run.reason ? reasonText(run.reason) : "";
    const withWhy = (s: string) => (why ? `${s}: ${why}` : s);
    const skips = run.skip_count > 1 ? ` (×${run.skip_count})` : "";
    switch (run.state) {
        case "skipped":
            return { label: "skipped", tone: "neutral", detail: withWhy("skipped") + skips, hollow: true };
        case "scheduled":
            return { label: "starting", tone: "neutral", detail: "starting", hollow: false };
        case "running":
            return { label: "running", tone: "accent", detail: "running", hollow: false };
        case "missed":
        case "lost":
            return { label: run.state, tone: "error", detail: withWhy(run.state), hollow: false };
        case "pending":
            if (run.failed) return { label: "missed", tone: "error", detail: withWhy("missed"), hollow: false };
            if (run.ended_at_ms != null) return { label: "replaced", tone: "neutral", detail: withWhy("not run"), hollow: true };
            return { label: "waiting", tone: "neutral", detail: withWhy("waiting for its agent"), hollow: false };
    }
    const infra = run.infra && run.infra !== "ok" ? (INFRA_TEXT[run.infra] ?? run.infra) : "";
    if (infra) return { label: "failed", tone: "error", detail: `failed: ${infra}`, hollow: false };
    switch (run.outcome) {
        case "ok":
            return { label: "ok", tone: "ok", detail: "ok", hollow: false };
        case "findings":
            return { label: "findings", tone: "warn", detail: "finished with findings", hollow: false };
        case "failed":
            return { label: "failed", tone: "error", detail: "the run reported it failed", hollow: false };
        default:
            return {
                label: "unknown",
                tone: run.failed ? "error" : "neutral",
                detail: "finished without saying how it went",
                hollow: false,
            };
    }
}

/** A job's state in words (§3.3 "State"). */
export function stateText(state: JobView["state"]): { label: string; tone: PillTone } {
    switch (state) {
        case "paused":
            return { label: "Paused", tone: "warn" };
        case "paused_all":
            return { label: "Paused by Pause all", tone: "warn" };
        case "ended":
            return { label: "Ended", tone: "neutral" };
        default:
            return { label: "Active", tone: "ok" };
    }
}

/** "$0.42", or "—" when no run reported a cost. */
export function costText(usd: number, known: boolean): string {
    if (!known) return "—";
    return `$${usd < 10 ? usd.toFixed(2) : usd.toFixed(0)}`;
}

/** How many jobs are failing: the title's "Jobs · N failing" (§3.3). */
export function failingCount(jobs: readonly JobView[] | null | undefined): number {
    return (jobs ?? []).filter((j) => j.failing).length;
}

/** Cloud events srv refused since it started, in words, or `""` when none were. */
export function droppedText(dropped: readonly DroppedEvents[] | null | undefined): string {
    const shown = (dropped ?? []).filter((d) => d.count > 0);
    const total = shown.reduce((n, d) => n + d.count, 0);
    if (total === 0) return "";
    const parts = shown.map((d) => `${d.count} ${d.reason}`);
    return `${total} cloud ${total === 1 ? "event was" : "events were"} dropped since AgentMux started (${parts.join(", ")}). They are never shown as runs.`;
}

/** The Kanban task a failed run went to (`links.kanban_task`, §3.4), if srv linked one. */
export function kanbanTask(run: JobRunRow): { id: string; num: number } | null {
    const t = (run.links as { kanban_task?: { id?: unknown; num?: unknown } }).kanban_task;
    return typeof t?.num === "number" && typeof t.id === "string" ? { id: t.id, num: t.num } : null;
}
