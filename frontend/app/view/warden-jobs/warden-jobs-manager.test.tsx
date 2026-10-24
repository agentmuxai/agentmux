// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { cleanup, fireEvent, render, screen, waitFor } from "@solidjs/testing-library";
import { afterEach, describe, expect, it, vi } from "vitest";

import type { JobsListResult } from "@/types/rpc/JobsListResult";
import type { JobView } from "@/types/rpc/JobView";

const rpc = vi.hoisted(() => ({ list: vi.fn(), runs: vi.fn(), act: vi.fn() }));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: {
        JobsListCommand: () => rpc.list(),
        JobsRunsCommand: (_c: unknown, d: unknown) => rpc.runs(d),
        JobsActCommand: (_c: unknown, d: unknown) => rpc.act(d),
    },
}));
const events = vi.hoisted(() => ({ handlers: new Map<string, (data: unknown) => void>() }));
vi.mock("@/app/store/mps", () => ({
    onMuxEvent: (type: string, handler: (data: unknown) => void) => {
        events.handlers.set(type, handler);
        return () => events.handlers.delete(type);
    },
}));
/** srv publishing `jobs:changed` for `jobId`. */
const jobsChanged = (jobId: string) => events.handlers.get("jobs:changed")?.({ job_id: jobId });
vi.mock("@/app/store/global", () => ({ openLink: vi.fn() }));
vi.mock("@/app/util/reveal-block", () => ({ revealBlock: vi.fn() }));
const warden = vi.hoisted(() => ({ open: vi.fn() }));
vi.mock("@/app/view/warden/open-warden", () => ({ openWarden: (section: string) => warden.open(section) }));

import { mkJob, mkRun, NOW } from "./jobs-fixtures";
import { mergeReload } from "./jobs-history";
import { WardenJobsManager } from "./warden-jobs-manager";

function listOf(jobs: JobView[], pausedAll = false): JobsListResult {
    return { jobs, paused_all: pausedAll ? { value: "1", set_at_ms: NOW - 60_000, set_by: "operator" } : null, now_ms: NOW };
}

afterEach(() => {
    cleanup();
    vi.clearAllMocks();
});

describe("WardenJobsManager", () => {
    it("shows one row per job: trigger and next run, last run, recent runs, cost and state, and counts the failing", async () => {
        const failedRun = mkRun({ id: "r2", infra: "error", failed: true, started_at_ms: NOW - 120_000, scheduled_for_ms: NOW - 120_000 });
        const skipped = mkRun({ id: "r3", state: "skipped", reason: "paused", started_at_ms: NOW - 60_000 });
        rpc.list.mockResolvedValue(
            listOf([
                mkJob({ recent: [skipped, failedRun, mkRun()], failing: true, cost_today_usd: 0.5, cost_week_usd: 1.25, cost_known: true }),
                mkJob({ id: "j2", name: "Weekly report", expression: "0 6 * * 1", state: "paused", next_run_ms: null }),
            ])
        );
        const onFailingCount = vi.fn();
        render(() => <WardenJobsManager visible onFailingCount={onFailingCount} />);
        await waitFor(() => expect(screen.getByText("Nightly build")).toBeInTheDocument());
        const row = screen.getByText("Nightly build").closest("tr")!;
        expect(row.textContent).toContain("daily 06:00 UTC");
        expect(row.textContent).toContain("in 3 h");
        expect(row.textContent).toContain("skipped");
        expect(row.textContent).toContain("$0.50");
        expect(row.textContent).toContain("$1.25");
        expect(row.dataset.failing).toBe("true");
        const marks = row.querySelectorAll(".warden-jobs-mark");
        expect(marks.length).toBe(3);
        expect((marks[1] as HTMLElement).dataset.tone).toBe("error");
        expect((marks[2] as HTMLElement).dataset.hollow).toBe("true");
        const paused = screen.getByText("Weekly report").closest("tr")!;
        expect(paused.textContent).toContain("Mondays 06:00 UTC");
        expect(paused.textContent).toContain("Paused");
        expect(paused.textContent).toContain("never run");
        expect(paused.textContent).toContain("—");
        await waitFor(() => expect(onFailingCount).toHaveBeenLastCalledWith(1));
    });

    it("opens a job's runs on click, and acts on it", async () => {
        rpc.list.mockResolvedValue(listOf([mkJob({ owner_block_id: "b-owner" })]));
        rpc.runs.mockResolvedValue({
            runs: [mkRun({ summary: "built fine <b>bold</b>", cost_usd: 0.12, cost_known: true, account_id: "acct-1" }), mkRun({ id: "r0", state: "missed", reason: "target_stopped", failed: true })],
        });
        rpc.act.mockResolvedValue({ ok: true, run_id: "r9", outcome: "delivered", error: null });
        render(() => <WardenJobsManager visible />);
        await waitFor(() => expect(screen.getByText("Nightly build")).toBeInTheDocument());
        fireEvent.click(screen.getByText("Nightly build"));
        await waitFor(() => expect(screen.getByText("built fine <b>bold</b>")).toBeInTheDocument());
        expect(rpc.runs).toHaveBeenCalledWith({ job_id: "j1", limit: 50 });
        expect(screen.getByText("agent not running")).toBeInTheDocument();
        expect(screen.getByText("$0.12")).toBeInTheDocument();
        expect(screen.getByRole("button", { name: /Open owner/ })).toBeInTheDocument();

        fireEvent.click(screen.getByRole("button", { name: /Run now/ }));
        await waitFor(() => expect(rpc.act).toHaveBeenCalledWith({ action: "run_now", job_id: "j1" }));
        await waitFor(() => expect(screen.getByText("Started.")).toBeInTheDocument());

        fireEvent.click(screen.getByRole("button", { name: /^Pause$/ }));
        await waitFor(() => expect(rpc.act).toHaveBeenCalledWith({ action: "pause", job_id: "j1" }));

        // Delete asks once more, and says the runs stay.
        const del = screen.getByRole("button", { name: /^Delete$/ }) as HTMLButtonElement;
        await waitFor(() => expect(del.disabled).toBe(false));
        fireEvent.click(del);
        expect(rpc.act).not.toHaveBeenCalledWith({ action: "delete", job_id: "j1" });
        fireEvent.click(screen.getByRole("button", { name: /Delete it, keep its runs/ }));
        await waitFor(() => expect(rpc.act).toHaveBeenCalledWith({ action: "delete", job_id: "j1" }));
    });

    it("links a failing job's incident and its failed runs to the Kanban task", async () => {
        const incident = {
            id: "i1", job_id: "j1", job_source: "cron", job_name: "Nightly build", work_id: "w1", task_num: 12, owner_uid: "uid-b",
            first_run_id: "r0", last_run_id: "r0", failures: 2, opened_at_ms: NOW - 60_000, last_failed_at_ms: NOW, ended_at_ms: null,
            end_reason: "", ended_by_run: "",
        };
        rpc.list.mockResolvedValue(listOf([mkJob({ failing: true, incident })]));
        rpc.runs.mockResolvedValue({
            runs: [mkRun({ id: "r0", state: "missed", reason: "target_stopped", failed: true, links: { kanban_task: { id: "w1", num: 12 } } }), mkRun()],
        });
        render(() => <WardenJobsManager visible />);
        await waitFor(() => expect(screen.getByText("Nightly build")).toBeInTheDocument());
        fireEvent.click(screen.getByText("Nightly build"));
        await waitFor(() => expect(screen.getByRole("button", { name: /^Kanban #12$/ })).toBeInTheDocument());
        expect(screen.getAllByRole("button", { name: /Kanban #12/ }).length).toBe(2);
        fireEvent.click(screen.getByRole("button", { name: /Failing: Kanban #12 · 2 in a row/ }));
        expect(warden.open).toHaveBeenCalledWith("kanban");
    });

    it("reloads an open history when srv says its job changed, so an older run's end shows", async () => {
        rpc.list.mockResolvedValue(listOf([mkJob(), mkJob({ id: "j2", name: "Other job" })]));
        const older = mkRun({ id: "r-old", state: "running", infra: "", outcome: "unknown", started_at_ms: NOW - 120_000, ended_at_ms: null });
        const newer = mkRun({ id: "r-new", state: "running", infra: "", outcome: "unknown", started_at_ms: NOW - 60_000, ended_at_ms: null });
        rpc.runs.mockResolvedValue({ runs: [newer, older] });
        render(() => <WardenJobsManager visible />);
        await waitFor(() => expect(screen.getByText("Nightly build")).toBeInTheDocument());
        fireEvent.click(screen.getByText("Nightly build"));
        await waitFor(() => expect(screen.getAllByText("running").length).toBeGreaterThanOrEqual(2));
        const asked = rpc.runs.mock.calls.length;

        // Another job changing doesn't reload this one.
        jobsChanged("j2");
        await Promise.resolve();
        expect(rpc.runs.mock.calls.length).toBe(asked);

        // The older run ends after the newer one started: its row updates.
        rpc.runs.mockResolvedValue({ runs: [newer, { ...older, state: "finished", infra: "ok", outcome: "ok", summary: "older one done", ended_at_ms: NOW }] });
        jobsChanged("j1");
        await waitFor(() => expect(screen.getByText("older one done")).toBeInTheDocument());
        expect(rpc.runs).toHaveBeenLastCalledWith({ job_id: "j1", limit: 50 });
    });

    it("drops an answer for a job that is no longer open", async () => {
        rpc.list.mockResolvedValue(listOf([mkJob({ id: "a", name: "Job A" }), mkJob({ id: "b", name: "Job B" })]));
        let answerA: (v: unknown) => void = () => {};
        rpc.runs.mockImplementation((d: { job_id: string }) =>
            d.job_id === "a"
                ? new Promise((resolve) => (answerA = resolve))
                : Promise.resolve({ runs: [mkRun({ id: "rb", job_id: "b", summary: "B's run" })] })
        );
        render(() => <WardenJobsManager visible />);
        await waitFor(() => expect(screen.getByText("Job A")).toBeInTheDocument());
        fireEvent.click(screen.getByText("Job A"));
        fireEvent.click(screen.getByText("Job B"));
        await waitFor(() => expect(screen.getByText("B's run")).toBeInTheDocument());
        answerA({ runs: [mkRun({ id: "ra", job_id: "a", summary: "A's run" })] });
        await new Promise((r) => setTimeout(r, 0));
        expect(screen.queryByText("A's run")).toBeNull();
        expect(screen.getByText("B's run")).toBeInTheDocument();
    });

    it("keeps the older pages already loaded when it reloads", () => {
        const at = (id: string, t: number) => mkRun({ id, started_at_ms: t });
        const shown = [at("r5", 5), at("r4", 4), at("r3", 3), at("r2", 2), at("r1", 1)];
        const fresh = [at("r6", 6), { ...at("r5", 5), summary: "now done" }, at("r4", 4)];
        expect(mergeReload(fresh, shown).map((r) => r.id)).toEqual(["r6", "r5", "r4", "r3", "r2", "r1"]);
        expect(mergeReload(fresh, shown)[1].summary).toBe("now done");
        expect(mergeReload([], shown)).toEqual([]);
    });

    it("with Pause all on, says so, turns Run now off, and offers Resume all", async () => {
        rpc.list.mockResolvedValue(listOf([mkJob({ state: "paused_all" })], true));
        rpc.runs.mockResolvedValue({ runs: [] });
        rpc.act.mockResolvedValue({ ok: true, run_id: null, outcome: null, error: null });
        render(() => <WardenJobsManager visible />);
        await waitFor(() => expect(screen.getByRole("status").textContent).toContain("Every job is paused"));
        expect(screen.getByText("Paused by Pause all")).toBeInTheDocument();
        fireEvent.click(screen.getByText("Nightly build"));
        await waitFor(() => expect(screen.getByText("No runs yet.")).toBeInTheDocument());
        expect((screen.getByRole("button", { name: /Run now/ }) as HTMLButtonElement).disabled).toBe(true);
        fireEvent.click(screen.getByRole("button", { name: /Resume all jobs/ }));
        await waitFor(() => expect(rpc.act).toHaveBeenCalledWith({ action: "resume_all" }));
    });

    it("pauses every job from the toolbar", async () => {
        rpc.list.mockResolvedValue(listOf([]));
        rpc.act.mockResolvedValue({ ok: true, run_id: null, outcome: null, error: null });
        render(() => <WardenJobsManager visible />);
        await waitFor(() => expect(screen.getByText(/No jobs yet/)).toBeInTheDocument());
        fireEvent.click(screen.getByRole("button", { name: /Pause all jobs/ }));
        await waitFor(() => expect(rpc.act).toHaveBeenCalledWith({ action: "pause_all" }));
    });

    it("shows a Drone graph read-only, and doesn't ask while hidden", async () => {
        rpc.list.mockResolvedValue(listOf([mkJob({ id: "d1", source: "drone", name: "Graph", expression: "", owner: "", actions: false, next_run_ms: null, recent: [mkRun()] })]));
        rpc.runs.mockResolvedValue({ runs: [mkRun()] });
        const { unmount } = render(() => <WardenJobsManager visible />);
        await waitFor(() => expect(screen.getByText("Graph")).toBeInTheDocument());
        expect(screen.getByText("by hand")).toBeInTheDocument();
        fireEvent.click(screen.getByText("Graph"));
        await waitFor(() => expect(rpc.runs).toHaveBeenCalled());
        expect(screen.queryByRole("button", { name: /Run now/ })).toBeNull();
        unmount();
        vi.clearAllMocks();
        render(() => <WardenJobsManager visible={false} />);
        await Promise.resolve();
        expect(rpc.list).not.toHaveBeenCalled();
    });
});
