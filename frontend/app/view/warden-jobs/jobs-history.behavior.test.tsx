// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * One job's history in the Jobs section, pinned as a whole ahead of splitting it into a state object and view parts:
 * each scenario mounts the real component with stand-ins for the cloud bits and the dry-run result, drives its props
 * (the job, the clock, the change counters) as signals, and logs the history as an indented tree and, in order, every
 * call it makes (the runs it asks for, the actions, the dry run, the buttons' callbacks). The logs are snapshots
 * recorded on the unsplit component; the split must leave them byte for byte. (warden-jobs-manager.test.tsx asserts
 * what the person sees through the manager; this pins the markup and the calls.)
 */

import { cleanup, fireEvent, render } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { JobRunRow } from "@/types/rpc/JobRunRow";
import type { JobView } from "@/types/rpc/JobView";
import type { JobsActRequest } from "@/types/rpc/JobsActRequest";
import type { JobsActResult } from "@/types/rpc/JobsActResult";
import type { JobsDryRunResult } from "@/types/rpc/JobsDryRunResult";
import type { JobsRunsResult } from "@/types/rpc/JobsRunsResult";

const h = vi.hoisted(() => ({
    log: [] as string[],
    vaultOn: true,
    agents: undefined as { id: string; name: string }[] | undefined,
}));

vi.mock("@/app/store/global", () => ({ openLink: (url: string) => (h.log.push(`openLink ${url}`), Promise.resolve()) }));
vi.mock("@/app/util/reveal-block", () => ({
    revealBlock: (id: string, opts: unknown) => (h.log.push(`revealBlock ${id} ${JSON.stringify(opts)}`), Promise.resolve()),
}));
vi.mock("@/app/view/warden/open-warden", () => ({
    openWarden: (section: string) => (h.log.push(`openWarden ${section}`), Promise.resolve()),
}));
vi.mock("@/app/view/vault/vault-store", () => ({
    vaultAvailable: () => h.vaultOn,
    vaultEntries: () => [{ id: "v-1", name: "Repo token" }],
}));
vi.mock("@/app/view/agent/use-agent-list", () => ({ useAgentList: () => [() => h.agents, () => null] }));
// The local format of a date is the machine's; the snapshots must not be.
vi.spyOn(Date.prototype, "toLocaleString").mockImplementation(function (this: Date) {
    return `L${this.getTime()}`;
});
vi.mock("./cloud-job-bits", () => ({
    CloudFacts: (props: { cloud: { reporter: string } }) => <span data-stand="CloudFacts">{props.cloud.reporter}</span>,
    CloudRunNow: (props: { cloud: { reporter: string } }) => <span data-stand="CloudRunNow">{props.cloud.reporter}</span>,
}));
vi.mock("./job-dry-run", () => ({
    JobDryRun: (props: { result: { name: string; ok: boolean }; vault: unknown; nameOf: (id: string) => string }) => (
        <div data-stand="JobDryRun" data-vault={JSON.stringify(props.vault)}>
            {props.result.name}|{props.result.ok ? "ok" : "no"}|{props.nameOf("a-1")}|{props.nameOf("zz")}
        </div>
    ),
}));

import { JobHistory } from "./jobs-history";
import { mkDryRun, mkJob, mkRun, NOW } from "./jobs-fixtures";
import { RUNS_MAX, RUNS_PAGE, type JobChange } from "./jobs-model";

const T = h.log;
const log = (line: string) => T.push(line);
const snapshot = (name: string) => expect(T.join("\n") + "\n").toMatchFileSnapshot(`./__snapshots__/jobs-history.behavior.${name}.txt`);

const ATTRS = /^(class|role|aria-.*|data-.*|title|disabled|type|href)$/;
/** The history as an indented tree: tags, classes, the attributes that carry state, and text. */
function dump(node: Node, depth = 0, out: string[] = []): string[] {
    const pad = "  ".repeat(depth);
    if (node.nodeType === Node.TEXT_NODE) {
        const t = (node.textContent ?? "").trim().replace(/\s+/g, " ");
        if (t) out.push(`${pad}"${t}"`);
        return out;
    }
    if (node.nodeType !== Node.ELEMENT_NODE) return out;
    const el = node as HTMLElement;
    if (el.tagName === "svg" || el.tagName === "STYLE") return out;
    const attrs = [...el.attributes].filter((a) => ATTRS.test(a.name)).map((a) => `${a.name}=${JSON.stringify(a.value)}`);
    out.push(`${pad}<${el.tagName.toLowerCase()}${attrs.length ? " " + attrs.join(" ") : ""}>`);
    el.childNodes.forEach((c) => dump(c, depth + 1, out));
    return out;
}

let host: HTMLElement;
const look = (label: string) => {
    log(`-- ${label}`);
    log(dump(host).join("\n"));
};
const flush = async () => {
    for (let i = 0; i < 8; i++) await new Promise((r) => setTimeout(r, 0));
};
const gate = <V,>() => {
    let open!: (v: V) => void;
    let fail!: (e: unknown) => void;
    const promise = new Promise<V>((res, rej) => ((open = res), (fail = rej)));
    return { promise, open, fail };
};

/** `n` runs of a job, newest first, a minute apart. */
const rowsOf = (jobId: string, n: number, over: Partial<JobRunRow> = {}): JobRunRow[] =>
    Array.from({ length: n }, (_, i) => mkRun({ id: `${jobId}-r${i}`, job_id: jobId, started_at_ms: NOW - (i + 1) * 60_000, scheduled_for_ms: NOW - (i + 1) * 60_000, ...over }));

const ok = (over: Partial<JobsActResult> = {}): JobsActResult => ({ ok: true, run_id: "x", outcome: "delivered", error: null, ...over });

/** Mounts the history of `job` with every prop a signal and every callback logged. */
function mount(initial: { job?: JobView; pausedAll?: boolean; rows?: Record<string, JobRunRow[]> } = {}) {
    const [job, setJob] = createSignal<JobView>(initial.job ?? mkJob());
    const [now, setNow] = createSignal(NOW);
    const [pausedAll, setPausedAll] = createSignal(initial.pausedAll ?? false);
    const [changed, setChanged] = createSignal<JobChange>({ jobId: "", seq: 0 });
    const [polled, setPolled] = createSignal(0);
    const rows = initial.rows ?? { j1: rowsOf("j1", 3) };
    const calls = {
        loadRuns: (async (jobId: string, before?: number, limit?: number): Promise<JobsRunsResult> => {
            const all = (rows[jobId] ?? []).filter((r) => before == null || r.started_at_ms <= before);
            return { runs: all.slice(0, limit ?? all.length) };
        }) as (jobId: string, before?: number, limit?: number) => Promise<JobsRunsResult>,
        act: (async () => ok()) as (req: JobsActRequest) => Promise<JobsActResult>,
        dryRun: (async () => mkDryRun()) as (jobId: string) => Promise<JobsDryRunResult>,
    };
    const view = render(() => (
        <JobHistory
            job={job()}
            now={now()}
            pausedAll={pausedAll()}
            loadRuns={(jobId, before, limit) => {
                log(`loadRuns ${jobId} before=${before ?? "-"} limit=${limit ?? "-"}`);
                return calls.loadRuns(jobId, before, limit);
            }}
            changed={changed()}
            polled={polled()}
            act={(req) => {
                log(`act ${JSON.stringify(req)}`);
                return calls.act(req);
            }}
            dryRun={(jobId) => {
                log(`dryRun ${jobId}`);
                return calls.dryRun(jobId);
            }}
            onEdit={() => log("onEdit")}
            onClone={() => log("onClone")}
            onReviewUpdate={() => log("onReviewUpdate")}
            onClose={() => log("onClose")}
        />
    ));
    host = view.container;
    return { calls, rows, setJob, setNow, setPausedAll, setChanged, setPolled };
}

const button = (text: string) => [...host.querySelectorAll("button")].find((b) => b.textContent?.trim() === text)!;
const press = async (text: string) => {
    fireEvent.click(button(text));
    await flush();
};

beforeEach(() => {
    T.length = 0;
    h.vaultOn = true;
    h.agents = [{ id: "a-1", name: "Builder" }];
});
afterEach(() => {
    cleanup();
});

describe("JobHistory, pinned", () => {
    it("lays out a job, its header and a mixed list of runs", async () => {
        const t = NOW;
        const rows = [
            mkRun({ id: "m1", started_at_ms: t - 60_000, scheduled_for_ms: t - 60_000, trigger: "manual", summary: "Did it", cost_known: true, cost_usd: 0.42, account_id: "acct-1" }),
            mkRun({ id: "m2", started_at_ms: t - 120_000, scheduled_for_ms: null, trigger: "catch_up", state: "running", ended_at_ms: null, provider: "claude" }),
            mkRun({ id: "m3", started_at_ms: t - 180_000, state: "skipped", reason: "paused", skip_count: 4, ended_at_ms: null }),
            mkRun({ id: "m4", started_at_ms: t - 240_000, infra: "timeout", failed: true, reason: "run_cap", cost_known: true, cost_usd: 1.5, cost_partial: true }),
            mkRun({ id: "m5", started_at_ms: t - 300_000, outcome: "findings", summary: "<b>not markup</b>" }),
            mkRun({ id: "m6", started_at_ms: t - 360_000, state: "missed", failed: true, ended_at_ms: null }),
        ];
        mount({ job: mkJob({ owner_block_id: "blk-1" }), rows: { j1: rows } });
        await flush();
        look("mixed runs");
        await snapshot("layout");
    });

    it("says there are no runs yet, and leaves out an owner and a target that are not set", async () => {
        mount({ job: mkJob({ owner: "", target: "", state: "paused" }), rows: { j1: [] } });
        await flush();
        look("no runs, paused, no owner");
        mount({ job: mkJob({ state: "ended", actions: false }), rows: { j1: [] } });
        await flush();
        look("ended, no actions");
        await snapshot("empty");
    });

    it("shows a cloud job's facts and run-now note, and none of the actions", async () => {
        const cloud = { reporter: "nightly-bot", reporter_pinned: true, grace_minutes: 30, max_run_minutes: 120, run_now_link: null, run_now_ask_owner: null, approved_at_ms: 0 };
        mount({ job: mkJob({ id: "c1", source: "cloud", actions: false, cloud }), rows: { c1: rowsOf("c1", 1) } });
        await flush();
        look("cloud job");
        await snapshot("cloud");
    });

    it("offers the failing incident, a template's update and the owner, each calling its callback", async () => {
        const incident = { id: "i1", task_num: 77, failures: 3 } as unknown as JobView["incident"];
        const update = { version: "1.1.0", approved: false } as any;
        const m = mount({ job: mkJob({ incident, owner_block_id: "blk-1", template: { id: "t", version: "1.0.0", update } }) });
        await flush();
        look("incident and update");
        await press("Failing: Kanban #77 · 3 in a row");
        await press("Review 1.1.0");
        await press("Open owner");
        await press("Edit");
        await press("Clone");
        await press("Close");
        m.setJob(mkJob({ incident, template: { id: "t", version: "1.0.0", update: { version: "1.1.0", approved: true } as any } }));
        await flush();
        look("approved update");
        await press("Update to 1.1.0");
        m.setJob(mkJob({ incident: { id: "i1", task_num: null, failures: 3 } as unknown as JobView["incident"], template: { id: "t", version: "1.0.0", update: null } }));
        await flush();
        look("incident without a task, no update");
        await snapshot("header-buttons");
    });

    it("links a run to its transcript, its incident task, its drone run and its reported links", async () => {
        const long = "A very long label of a link that goes on";
        const run = mkRun({
            id: "l1",
            started_at_ms: NOW - 60_000,
            links: {
                transcript: { block_id: "blk-9" },
                kanban_task: { id: "w1", num: 12 },
                drone_run_id: "drone-5",
                reported: ["https://example.com/a", { label: "Report", url: "https://example.com/b" }, { url: "https://example.com/c" }, { label: long, url: "https://example.com/d" }, "http://insecure.example.com", { label: "bad" }, 7, null],
            },
        });
        mount({ rows: { j1: [run, mkRun({ id: "l2", started_at_ms: NOW - 120_000, links: { reported: "no", transcript: { block_id: 5 }, drone_run_id: "" } })] } });
        await flush();
        look("links");
        await press("Agent");
        await press("Kanban #12");
        await press("Report");
        await press(`${long.slice(0, 23)}…`);
        await snapshot("links");
    });

    it("runs a job now and reports whether it started, and the failure when the call throws", async () => {
        const m = mount();
        await flush();
        await press("Run now");
        look("started");
        m.calls.act = async () => ok({ ok: false, error: "no agent", outcome: "failed" });
        await press("Run now");
        look("not run: error");
        m.calls.act = async () => ok({ ok: false, error: null, outcome: null });
        await press("Run now");
        look("not run: unknown");
        m.calls.act = async () => {
            throw new Error("srv is down");
        };
        await press("Run now");
        look("thrown");
        await snapshot("run-now");
    });

    it("disables the actions while one is running and when everything is paused", async () => {
        const g = gate<JobsActResult>();
        const m = mount();
        await flush();
        m.calls.act = () => g.promise;
        fireEvent.click(button("Run now"));
        await flush();
        look("run now in flight");
        g.open(ok());
        await flush();
        look("run now done");
        m.setPausedAll(true);
        await flush();
        look("everything paused");
        await snapshot("busy");
    });

    it("pauses an active job and resumes a paused one", async () => {
        const m = mount();
        await flush();
        await press("Pause");
        m.setJob(mkJob({ state: "paused" }));
        await flush();
        look("paused job");
        await press("Resume");
        m.setJob(mkJob({ state: "ended" }));
        await flush();
        look("ended job");
        await snapshot("pause-resume");
    });

    it("asks before deleting, takes it back, and closes the history once the job is gone", async () => {
        const m = mount();
        await flush();
        await press("Delete");
        look("confirm");
        await press("Keep");
        look("kept");
        await press("Delete");
        await press("Delete it, keep its runs");
        look("deleted");
        await press("Delete");
        m.calls.act = async () => {
            throw new Error("locked");
        };
        await press("Delete it, keep its runs");
        look("delete failed");
        await snapshot("delete");
    });

    it("dry-runs a job and shows the result with the vault and the agents' names", async () => {
        const m = mount();
        await flush();
        const g = gate<JobsDryRunResult>();
        m.calls.dryRun = () => g.promise;
        fireEvent.click(button("Dry run"));
        await flush();
        look("dry run in flight");
        g.open(mkDryRun({ name: "Weekly" }));
        await flush();
        look("dry run result");
        h.vaultOn = false;
        m.calls.dryRun = async () => mkDryRun({ name: "Second", ok: false, error: "bad" });
        await press("Dry run");
        look("vault off, refused");
        m.calls.dryRun = async () => {
            throw new Error("no srv");
        };
        await press("Dry run");
        look("dry run thrown");
        await snapshot("dry-run");
    });

    it("loads the first page, then the older ones, without the rows it already has", async () => {
        const m = mount({ rows: { j1: rowsOf("j1", 70) } });
        await flush();
        look("first page");
        await press("Older runs");
        look("second page");
        log(`rows: ${host.querySelectorAll("tbody tr").length}`);
        m.rows.j1 = rowsOf("j1", 3);
        await snapshot("paging");
    });

    it("hides the older button when the page comes back short, and says why a load failed", async () => {
        const full = rowsOf("j1", RUNS_PAGE);
        const m = mount({ rows: { j1: full } });
        await flush();
        look("a full page shows the button");
        m.calls.loadRuns = async () => {
            throw new Error("rpc broke");
        };
        await press("Older runs");
        look("older failed");
        await snapshot("load-failures");
    });

    it("clears the confirm and the dry run when another job is opened, and drops an answer for the old one", async () => {
        const slow = gate<JobsRunsResult>();
        const m = mount({ rows: { j1: rowsOf("j1", 2), j2: rowsOf("j2", 1) } });
        await flush();
        await press("Delete");
        await press("Dry run");
        look("j1 with confirm and dry run");
        const first = m.calls.loadRuns;
        m.calls.loadRuns = (jobId, before, limit) => (jobId === "j2" ? slow.promise : first(jobId, before, limit));
        m.setJob(mkJob({ id: "j2", name: "Other job" }));
        await flush();
        look("j2 loading");
        m.setJob(mkJob({ id: "j3", name: "Third" }));
        await flush();
        slow.open({ runs: rowsOf("j2", 1) });
        await flush();
        look("j3 after the stale j2 answer");
        await snapshot("switch-job");
    });

    it("reloads the runs when its own job, or every job, changed, and not for another", async () => {
        const m = mount({ rows: { j1: rowsOf("j1", 3) } });
        await flush();
        m.setChanged({ jobId: "other", seq: 1 });
        await flush();
        m.setChanged({ jobId: "j1", seq: 2 });
        await flush();
        m.setChanged({ jobId: "", seq: 3 });
        await flush();
        m.rows.j1 = rowsOf("j1", 4);
        m.setChanged({ jobId: "j1", seq: 4 });
        await flush();
        log(`rows: ${host.querySelectorAll("tbody tr").length}`);
        await snapshot("changed");
    });

    it("reloads as deep as the rows shown go, and keeps the older rows below the fresh ones", async () => {
        const m = mount({ rows: { j1: rowsOf("j1", 120) } });
        await flush();
        await press("Older runs");
        await press("Older runs");
        log(`rows: ${host.querySelectorAll("tbody tr").length}`);
        m.setPolled(1);
        await flush();
        log(`rows after poll: ${host.querySelectorAll("tbody tr").length}`);
        m.rows.j1 = [mkRun({ id: "j1-new", started_at_ms: NOW - 10_000 }), ...rowsOf("j1", 120)];
        m.setChanged({ jobId: "j1", seq: 1 });
        await flush();
        log(`rows after change: ${host.querySelectorAll("tbody tr").length} first=${host.querySelector("tbody tr td")?.textContent}`);
        log(`cap ${RUNS_MAX}`);
        await snapshot("depth");
    });

    it("drops a reload that a newer one overtook, and one that failed", async () => {
        const slow = gate<JobsRunsResult>();
        const m = mount({ rows: { j1: rowsOf("j1", 2) } });
        await flush();
        const normal = m.calls.loadRuns;
        m.calls.loadRuns = () => slow.promise;
        m.setPolled(1);
        await flush();
        m.calls.loadRuns = async (jobId, before, limit) => normal(jobId, before, limit);
        m.rows.j1 = rowsOf("j1", 5);
        m.setPolled(2);
        await flush();
        log(`rows after the newer reload: ${host.querySelectorAll("tbody tr").length}`);
        slow.open({ runs: rowsOf("j1", 1) });
        await flush();
        log(`rows after the overtaken one: ${host.querySelectorAll("tbody tr").length}`);
        m.calls.loadRuns = async () => {
            throw new Error("rpc broke");
        };
        m.setPolled(3);
        await flush();
        look("reload failed");
        await snapshot("overtaken");
    });

    it("moves the clock on every row when the time passes", async () => {
        const m = mount();
        await flush();
        look("now");
        m.setNow(NOW + 2 * 86_400_000);
        await flush();
        look("two days later");
        await snapshot("clock");
    });
});
