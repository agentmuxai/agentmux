// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Settings › Agents › Outdated agent ids, the parts the card's own tests leave to chance: what the status
 * line says while each step runs, every error text, the wait after a stop, the re-read before and after
 * it, the pick of an id (typed, Enter, a second pick during the first), Undo, and what each control does
 * while something is under way. They pin the card so `ForkSlugMigration` can be split without changing it.
 */

import { cleanup, fireEvent, render, screen, waitFor, within } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { ForkSlugBlocker } from "@/types/rpc/ForkSlugBlocker";
import type { ForkSlugCandidate } from "@/types/rpc/ForkSlugCandidate";
import type { ForkSlugPlan } from "@/types/rpc/ForkSlugPlan";
import type { ForkSlugReport } from "@/types/rpc/ForkSlugReport";

const migrate = vi.fn();
vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: { AgentsMigrateForkSlugsCommand: (...args: unknown[]) => migrate(...args) },
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));

import { ForkSlugMigration, planLine, RECHECK_AFTER_MS, shouldRecheck } from "./fork-slug-migration";

function cand(
    name: string,
    old_slug: string,
    new_slug: string,
    extra: Partial<ForkSlugCandidate> = {}
): ForkSlugCandidate {
    return {
        uid: `uid-${new_slug}`,
        name,
        old_slug,
        new_slug,
        locations: [{ key: "K2", place: "definitions/x.json", value: old_slug }],
        running: [],
        ...extra,
    };
}

const A = cand("Loap #2", "loap-loap-2", "loap-2");
const B = cand("Naki #2", "naki-naki-2", "naki-2");
const REFUSED = cand("Loap #3", "loap-loap-2-loap-3", "loap-3", {
    uid: "uid-loap-3",
    refused: "loap-3 is taken: agent x has it",
    suggestion: "loap-3-2",
});
const PICKED = cand("Loap #3", "loap-loap-2-loap-3", "loap-3-2", { uid: "uid-loap-3" });
const PICK = [{ uid: "uid-loap-3", new_slug: "loap-3-2" }];

function plan(
    candidates: ForkSlugCandidate[],
    blockers: ForkSlugBlocker[] = [],
    extra: Partial<ForkSlugPlan> = {}
): ForkSlugPlan {
    return { candidates, skipped: [], blockers, summary: "", duplicates: [], ...extra };
}

const RUNNING_BLOCK: ForkSlugBlocker = { kind: "running", message: "Loap #2 is running", clearable_by_stopping: true };
const OLD: ForkSlugBlocker = {
    kind: "old_instance",
    message: "An older AgentMux is running.",
    clearable_by_stopping: false,
};
const RUN_A = cand("Loap #2", "loap-loap-2", "loap-2", {
    running: [{ block_id: "b1", channel: "stable", how: "lease" }],
});
const RUN_B = cand("Naki #2", "naki-naki-2", "naki-2", { running: [{ block_id: "b2", channel: "dev", how: "lease" }] });

const REPORT: ForkSlugReport = {
    journal: "/backups/run-1/journal.jsonl",
    backup_dir: "/backups/run-1",
    migrated: [{ uid: "uid-loap-2", name: "Loap #2", old_slug: "loap-loap-2", new_slug: "loap-2", changes: [] }],
    pr_searches: ["gh-agent pr list --search one", "gh-agent pr list --search two"],
    notes: ["Relaunch each migrated agent."],
};

/** A promise settled by hand. */
function gate<T = void>() {
    let resolve!: (v: T) => void;
    let reject!: (e: unknown) => void;
    const promise = new Promise<T>((res, rej) => {
        resolve = res;
        reject = rej;
    });
    return { promise, resolve, reject };
}

type Step<T> = T | Error | Promise<T> | (() => Promise<T>);
interface Script {
    plans: Array<Step<ForkSlugPlan>>;
    apply?: Step<ForkSlugReport>;
    stop?: Step<{ wait_ms: number }>;
    rollback?: Step<ForkSlugReport | undefined>;
}

/** srv: `plans` answers each plan in turn (the last repeats); the others answer their one mode. */
function srv(s: Script) {
    let i = 0;
    const settle = async <T,>(step: Step<T>): Promise<T> => {
        if (step instanceof Error) throw step;
        if (typeof step === "function") return (step as () => Promise<T>)();
        return step;
    };
    migrate.mockImplementation(async (_client: unknown, data: { mode: string }) => {
        switch (data.mode) {
            case "plan":
                return {
                    mode: "plan",
                    plan: await settle(s.plans[Math.min(i++, s.plans.length - 1)]),
                    pending: [],
                    wait_ms: 0,
                    summary: "",
                };
            case "stop": {
                const r = await settle(s.stop ?? { wait_ms: 0 });
                return { mode: "stop", pending: [], wait_ms: r.wait_ms, summary: "" };
            }
            case "apply":
                return { mode: "apply", report: await settle(s.apply ?? REPORT), pending: [], wait_ms: 0, summary: "" };
            case "rollback":
                return {
                    mode: "rollback",
                    report: await settle(s.rollback ?? { ...REPORT, migrated: [] }),
                    pending: [],
                    wait_ms: 0,
                    summary: "",
                };
        }
        throw new Error(`unexpected mode ${data.mode}`);
    });
}
const calls = () => migrate.mock.calls.map((c) => c[1] as { mode: string; journal?: string; choices?: unknown });
const modes = () => calls().map((c) => c.mode);
const count = (mode: string) => modes().filter((m) => m === mode).length;

const LOADED = "Checking agent ids…";
const btn = (name: string | RegExp) => screen.getByRole("button", { name });
const maybeBtn = (name: string | RegExp) => screen.queryByRole("button", { name });
const alert = () => screen.queryByRole("alert");
const tick = async () => {
    for (let i = 0; i < 12; i++) await Promise.resolve();
};

beforeEach(() => {
    migrate.mockReset();
});
afterEach(() => {
    cleanup();
    vi.useRealTimers();
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
});

describe("the helpers", () => {
    it("planLine tells an unfinished update, with the reason when there is one", () => {
        expect(planLine(plan([A], [], { last_run: { journal: "j", finished: false, failed: "boom" } }))).toBe(
            "An update stopped before it finished: boom"
        );
        expect(planLine(plan([A], [], { last_run: { journal: "j", finished: false } }))).toBe(
            "An update stopped before it finished."
        );
        expect(planLine(plan([A], [], { last_run: { journal: "j", finished: true } }))).toBe(
            "1 agent has an outdated id."
        );
    });

    it("rechecks a plan 2 seconds old, and not one a moment younger", () => {
        expect(RECHECK_AFTER_MS).toBe(2000);
        expect(shouldRecheck("idle", 0, 1999)).toBe(false);
        expect(shouldRecheck("idle", 0, 2000)).toBe(true);
    });
});

describe("what the status line says", () => {
    it("is 'Checking agent ids…' while the first plan is on its way, then the plan's own line", async () => {
        const g = gate<ForkSlugPlan>();
        srv({ plans: [() => g.promise] });
        render(() => <ForkSlugMigration />);
        expect(screen.getByRole("status")).toHaveTextContent(LOADED);
        g.resolve(plan([A, B]));
        expect(await screen.findByText("2 agents have an outdated id.")).toBeInTheDocument();
    });

    it("says Updating… while the update runs, and keeps the buttons busy", async () => {
        const g = gate<ForkSlugReport>();
        srv({ plans: [plan([A]), plan([])], apply: () => g.promise });
        render(() => <ForkSlugMigration />);
        await screen.findByText("1 agent has an outdated id.");
        fireEvent.click(btn("Update them"));
        await waitFor(() => expect(screen.getByRole("status")).toHaveTextContent("Updating…"));
        expect(btn("Update them")).toHaveAttribute("aria-busy", "true");
        expect(btn("Update them")).toBeDisabled();
        g.resolve(REPORT);
        expect(await screen.findByText("Every agent has an up-to-date id.")).toBeInTheDocument();
    });

    it("says it is stopping the panes, with the modal gone, while the stop runs", async () => {
        const g = gate<{ wait_ms: number }>();
        srv({
            plans: [plan([RUN_A], [RUNNING_BLOCK]), plan([RUN_A], [RUNNING_BLOCK]), plan([])],
            stop: () => g.promise,
        });
        render(() => <ForkSlugMigration />);
        await screen.findByText("1 agent has an outdated id.");
        fireEvent.click(btn("Update them"));
        fireEvent.click(await screen.findByRole("button", { name: "Stop and update" }));
        await waitFor(() =>
            expect(screen.getByRole("status")).toHaveTextContent(
                "Stopping the panes. Each can still be kept running from its own pane for 30 s."
            )
        );
        expect(screen.queryByRole("button", { name: "Stop and update" })).not.toBeInTheDocument();
        expect(btn("Update them")).toHaveAttribute("aria-busy", "true");
        g.resolve({ wait_ms: 0 });
        await screen.findByText("Every agent has an up-to-date id.");
    });

    it("says Undoing the update… while the undo runs, with the modal gone", async () => {
        const g = gate<ForkSlugReport | undefined>();
        srv({ plans: [plan([], [], { last_run: { journal: "j1", finished: true } })], rollback: () => g.promise });
        render(() => <ForkSlugMigration />);
        fireEvent.click(await screen.findByRole("button", { name: "Undo the update" }));
        fireEvent.click(within(await screen.findByRole("dialog")).getByRole("button", { name: "Undo" }));
        await waitFor(() => expect(screen.getByRole("status")).toHaveTextContent("Undoing the update…"));
        expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
        expect(btn("Undo the update")).toBeDisabled();
        g.resolve(undefined);
    });

    it("keeps Update them and Finish the update off while the undo runs", async () => {
        const g = gate<ForkSlugReport | undefined>();
        srv({
            plans: [plan([A], [], { last_run: { journal: "j1", finished: false, failed: "step" } })],
            rollback: () => g.promise,
        });
        render(() => <ForkSlugMigration />);
        await screen.findByRole("button", { name: "Finish the update" });
        fireEvent.click(btn("Undo the update"));
        fireEvent.click(within(await screen.findByRole("dialog")).getByRole("button", { name: "Undo" }));
        await waitFor(() => expect(screen.getByRole("status")).toHaveTextContent("Undoing the update…"));
        expect(btn("Finish the update")).toBeDisabled();
        g.resolve(undefined);
    });

    it("keeps Update them off while the undo runs", async () => {
        const g = gate<ForkSlugReport | undefined>();
        srv({
            plans: [plan([A], [], { last_run: { journal: "j1", finished: true } })],
            rollback: () => g.promise,
        });
        render(() => <ForkSlugMigration />);
        await screen.findByRole("button", { name: "Update them" });
        fireEvent.click(btn("Undo the update"));
        fireEvent.click(within(await screen.findByRole("dialog")).getByRole("button", { name: "Undo" }));
        await waitFor(() => expect(screen.getByRole("status")).toHaveTextContent("Undoing the update…"));
        expect(btn("Update them")).toBeDisabled();
        g.resolve(undefined);
    });

    it("says Checking agent ids… while a picked id is checked", async () => {
        const g = gate<ForkSlugPlan>();
        srv({ plans: [plan([REFUSED]), () => g.promise, plan([REFUSED])] });
        render(() => <ForkSlugMigration />);
        await screen.findByText(/can't be updated yet/);
        fireEvent.click(btn("Use loap-3-2"));
        await waitFor(() => expect(screen.getByRole("status")).toHaveTextContent(LOADED));
        g.resolve(plan([REFUSED]));
        await waitFor(() => expect(screen.getByRole("status")).not.toHaveTextContent(LOADED));
    });
});

describe("errors", () => {
    it("a plan that cannot be read says so, with the reason or the text", async () => {
        srv({ plans: [new Error("down")] });
        render(() => <ForkSlugMigration />);
        expect(await screen.findByRole("alert")).toHaveTextContent("Couldn't check the agent ids: down");
        await waitFor(() => expect(screen.getByRole("status").textContent?.trim()).toBe(""));
        cleanup();
        migrate.mockRejectedValue("plain");
        render(() => <ForkSlugMigration />);
        expect(await screen.findByRole("alert")).toHaveTextContent("Couldn't check the agent ids: plain");
    });

    it("an update that fails says so and reads the plan again", async () => {
        srv({ plans: [plan([A]), plan([A])], apply: new Error("nope") });
        render(() => <ForkSlugMigration />);
        await screen.findByText("1 agent has an outdated id.");
        fireEvent.click(btn("Update them"));
        expect(await screen.findByRole("alert")).toHaveTextContent("Couldn't update: nope");
        await waitFor(() => expect(count("plan")).toBe(2));
        await waitFor(() => expect(btn("Update them")).toBeEnabled());
    });

    it("an update that stopped partway shows the reason and the report", async () => {
        srv({ plans: [plan([A]), plan([A])], apply: { ...REPORT, failed: "boom at step 3" } });
        render(() => <ForkSlugMigration />);
        await screen.findByText("1 agent has an outdated id.");
        fireEvent.click(btn("Update them"));
        expect(await screen.findByRole("alert")).toHaveTextContent("The update stopped: boom at step 3");
        expect(screen.getByLabelText("Update report")).toBeInTheDocument();
    });

    it("a finished update shows no error", async () => {
        srv({ plans: [plan([A]), plan([])] });
        render(() => <ForkSlugMigration />);
        await screen.findByText("1 agent has an outdated id.");
        fireEvent.click(btn("Update them"));
        await screen.findByLabelText("Update report");
        expect(alert()).toBeNull();
    });

    it("a stop that fails says so, applies nothing and leaves the card ready again", async () => {
        srv({ plans: [plan([RUN_A], [RUNNING_BLOCK])], stop: new Error("no override") });
        render(() => <ForkSlugMigration />);
        await screen.findByText("1 agent has an outdated id.");
        fireEvent.click(btn("Update them"));
        fireEvent.click(await screen.findByRole("button", { name: "Stop and update" }));
        expect(await screen.findByRole("alert")).toHaveTextContent("Couldn't stop the panes: no override");
        expect(count("apply")).toBe(0);
        await waitFor(() => expect(btn("Update them")).toBeEnabled());
        expect(screen.getByRole("status")).toHaveTextContent("1 agent has an outdated id.");
    });

    it("an undo that fails says so; one that works takes the report away", async () => {
        srv({
            plans: [plan([A]), plan([], [], { last_run: { journal: "j", finished: true } })],
            rollback: new Error("locked"),
        });
        render(() => <ForkSlugMigration />);
        await screen.findByText("1 agent has an outdated id.");
        fireEvent.click(btn("Update them"));
        await screen.findByLabelText("Update report");
        await waitFor(() => expect(btn("Undo the update")).toBeEnabled());
        fireEvent.click(btn("Undo the update"));
        fireEvent.click(within(await screen.findByRole("dialog")).getByRole("button", { name: "Undo" }));
        expect(await screen.findByRole("alert")).toHaveTextContent("Couldn't undo the update: locked");
        expect(screen.getByLabelText("Update report")).toBeInTheDocument();

        srv({ plans: [plan([], [], { last_run: { journal: "j", finished: true } })] });
        await waitFor(() => expect(btn("Undo the update")).toBeEnabled());
        fireEvent.click(btn("Undo the update"));
        fireEvent.click(within(await screen.findByRole("dialog")).getByRole("button", { name: "Undo" }));
        await waitFor(() => expect(screen.queryByLabelText("Update report")).not.toBeInTheDocument());
        expect(alert()).toBeNull();
    });

    it("an error is cleared by the next step", async () => {
        srv({ plans: [plan([A]), plan([A]), plan([])], apply: new Error("first") });
        render(() => <ForkSlugMigration />);
        await screen.findByText("1 agent has an outdated id.");
        fireEvent.click(btn("Update them"));
        await screen.findByRole("alert");
        await waitFor(() => expect(btn("Update them")).toBeEnabled());
        const g = gate<ForkSlugReport>();
        srv({ plans: [plan([A]), plan([])], apply: () => g.promise });
        fireEvent.click(btn("Update them"));
        await waitFor(() => expect(alert()).toBeNull());
        g.resolve(REPORT);
    });
});

describe("stopping the panes first", () => {
    async function openStop(s: Script) {
        srv(s);
        render(() => <ForkSlugMigration />);
        await screen.findByText(/outdated id/);
        fireEvent.click(btn("Update them"));
        return screen.findByRole("dialog");
    }

    it("lists the panes of the agents it would update, with their channel, and counts them", async () => {
        const dialog = await openStop({ plans: [plan([RUN_A, RUN_B, REFUSED], [RUNNING_BLOCK])] });
        expect(dialog).toHaveTextContent("Stop 2 panes?");
        expect(within(dialog).getByText("Loap #2 (stable)")).toBeInTheDocument();
        expect(within(dialog).getByText("Naki #2 (dev)")).toBeInTheDocument();
    });

    it("says 'pane' for one, and leaves out the channel when there is none", async () => {
        const noChannel = cand("Loap #2", "loap-loap-2", "loap-2", {
            running: [{ block_id: "b1", channel: "", how: "lease" }],
        });
        const dialog = await openStop({ plans: [plan([noChannel], [RUNNING_BLOCK])] });
        expect(dialog).toHaveTextContent("Stop 1 pane?");
        expect(within(dialog).getByText("Loap #2")).toBeInTheDocument();
    });

    it("leaves out the panes of an agent that cannot be updated yet", async () => {
        const refusedRunning = { ...REFUSED, running: [{ block_id: "b9", channel: "stable", how: "lease" }] };
        const dialog = await openStop({ plans: [plan([RUN_A, refusedRunning], [RUNNING_BLOCK])] });
        expect(within(dialog).queryByText(/Loap #3/)).not.toBeInTheDocument();
    });

    /** Holds back the waits the card asks for (the stop's override time), so a test can see their length. */
    function holdWaits() {
        const real = globalThis.setTimeout;
        const delays: number[] = [];
        const release: Array<() => void> = [];
        vi.spyOn(globalThis, "setTimeout").mockImplementation(((fn: () => void, ms?: number) => {
            if (ms === 1000 || ms === 5000 || ms === 6000) {
                delays.push(ms);
                release.push(fn);
                return 0 as never;
            }
            return real(fn, ms);
        }) as never);
        return { delays, release };
    }

    it("waits out the override and a second before it updates", async () => {
        srv({
            plans: [plan([RUN_A], [RUNNING_BLOCK]), plan([RUN_A], [RUNNING_BLOCK]), plan([A]), plan([])],
            stop: { wait_ms: 5000 },
        });
        render(() => <ForkSlugMigration />);
        await screen.findByText("1 agent has an outdated id.");
        fireEvent.click(btn("Update them"));
        const stop = await screen.findByRole("button", { name: "Stop and update" });
        const waits = holdWaits();
        fireEvent.click(stop);
        await waitFor(() => expect(waits.delays).toEqual([6000]), { timeout: 1500 });
        await tick();
        expect(count("apply")).toBe(0);
        waits.release[0]();
        await waitFor(() => expect(count("apply")).toBe(1));
    });

    it("does not wait when the panes need no override time", async () => {
        srv({
            plans: [plan([RUN_A], [RUNNING_BLOCK]), plan([RUN_A], [RUNNING_BLOCK]), plan([RUN_A]), plan([])],
            stop: { wait_ms: 0 },
        });
        render(() => <ForkSlugMigration />);
        await screen.findByText("1 agent has an outdated id.");
        fireEvent.click(btn("Update them"));
        const stop = await screen.findByRole("button", { name: "Stop and update" });
        const waits = holdWaits();
        fireEvent.click(stop);
        await waitFor(() => expect(count("apply")).toBe(1), { timeout: 1500 });
        expect(waits.delays).toEqual([]);
    });

    it("stops nothing when the plan read again has nothing left to update", async () => {
        srv({ plans: [plan([RUN_A], [RUNNING_BLOCK]), plan([])] });
        render(() => <ForkSlugMigration />);
        await screen.findByText("1 agent has an outdated id.");
        fireEvent.click(btn("Update them"));
        fireEvent.click(await screen.findByRole("button", { name: "Stop and update" }));
        await screen.findByText("Every agent has an up-to-date id.");
        expect(count("stop")).toBe(0);
        expect(count("apply")).toBe(0);
    });

    it("applies nothing when the plan read after the stop still has a blocker", async () => {
        srv({
            plans: [plan([RUN_A], [RUNNING_BLOCK]), plan([RUN_A], [RUNNING_BLOCK]), plan([RUN_A], [RUNNING_BLOCK])],
        });
        render(() => <ForkSlugMigration />);
        await screen.findByText("1 agent has an outdated id.");
        fireEvent.click(btn("Update them"));
        fireEvent.click(await screen.findByRole("button", { name: "Stop and update" }));
        await waitFor(() => expect(count("plan")).toBe(3));
        await tick();
        expect(count("stop")).toBe(1);
        expect(count("apply")).toBe(0);
        expect(screen.getByRole("status")).toHaveTextContent("1 agent has an outdated id.");
        expect(btn("Update them")).toBeEnabled();
    });

    it("applies nothing when the plan read after the stop has nothing left, and goes idle", async () => {
        srv({ plans: [plan([RUN_A], [RUNNING_BLOCK]), plan([RUN_A], [RUNNING_BLOCK]), plan([])] });
        render(() => <ForkSlugMigration />);
        await screen.findByText("1 agent has an outdated id.");
        fireEvent.click(btn("Update them"));
        fireEvent.click(await screen.findByRole("button", { name: "Stop and update" }));
        await screen.findByText("Every agent has an up-to-date id.");
        expect(count("apply")).toBe(0);
    });

    it("applies nothing when the plan cannot be read after the stop", async () => {
        srv({ plans: [plan([RUN_A], [RUNNING_BLOCK]), plan([RUN_A], [RUNNING_BLOCK]), new Error("gone")] });
        render(() => <ForkSlugMigration />);
        await screen.findByText("1 agent has an outdated id.");
        fireEvent.click(btn("Update them"));
        fireEvent.click(await screen.findByRole("button", { name: "Stop and update" }));
        expect(await screen.findByRole("alert")).toHaveTextContent("Couldn't check the agent ids: gone");
        expect(count("apply")).toBe(0);
        await waitFor(() => expect(screen.getByRole("status")).toHaveTextContent("1 agent has an outdated id."));
    });

    it("reads nothing as 'ready' when the plan cannot be read just before the stop", async () => {
        srv({ plans: [plan([RUN_A], [RUNNING_BLOCK]), new Error("gone")] });
        render(() => <ForkSlugMigration />);
        await screen.findByText("1 agent has an outdated id.");
        fireEvent.click(btn("Update them"));
        fireEvent.click(await screen.findByRole("button", { name: "Stop and update" }));
        expect(await screen.findByRole("alert")).toHaveTextContent("Couldn't check the agent ids: gone");
        await tick();
        expect(count("stop")).toBe(0);
        expect(count("apply")).toBe(0);
    });

    it("applies nothing after a stop that failed, even when the plan then has no blocker", async () => {
        srv({
            plans: [plan([RUN_A], [RUNNING_BLOCK]), plan([RUN_A], [RUNNING_BLOCK]), plan([A])],
            stop: new Error("no override"),
        });
        render(() => <ForkSlugMigration />);
        await screen.findByText("1 agent has an outdated id.");
        fireEvent.click(btn("Update them"));
        fireEvent.click(await screen.findByRole("button", { name: "Stop and update" }));
        await screen.findByRole("alert");
        await tick();
        expect(count("apply")).toBe(0);
        expect(count("plan")).toBe(2);
    });

    it("clears an earlier error when the stop starts", async () => {
        const g = gate<{ wait_ms: number }>();
        srv({
            plans: [plan([A]), plan([RUN_A], [RUNNING_BLOCK]), plan([RUN_A], [RUNNING_BLOCK]), plan([A])],
            apply: new Error("first try"),
            stop: () => g.promise,
        });
        render(() => <ForkSlugMigration />);
        await screen.findByText("1 agent has an outdated id.");
        fireEvent.click(btn("Update them"));
        expect(await screen.findByRole("alert")).toHaveTextContent("Couldn't update: first try");
        await waitFor(() => expect(btn("Update them")).toBeEnabled());
        fireEvent.click(btn("Update them"));
        fireEvent.click(await screen.findByRole("button", { name: "Stop and update" }));
        await waitFor(() => expect(count("stop")).toBe(1));
        expect(alert()).toBeNull();
        g.resolve({ wait_ms: 0 });
    });

    it("a cancelled stop with no pick reads nothing again", async () => {
        await openStop({ plans: [plan([RUN_A], [RUNNING_BLOCK])] });
        fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
        await tick();
        expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
        expect(count("plan")).toBe(1);
    });
});

describe("Update them and Finish the update", () => {
    it("Finish the update takes the place of Update them for a run that stopped, and resumes it", async () => {
        const g = gate<ForkSlugReport>();
        srv({
            plans: [plan([A], [], { last_run: { journal: "j", finished: false, failed: "step" } }), plan([])],
            apply: () => g.promise,
        });
        render(() => <ForkSlugMigration />);
        const finish = await screen.findByRole("button", { name: "Finish the update" });
        expect(maybeBtn("Update them")).toBeNull();
        expect(screen.getByRole("status")).toHaveTextContent("An update stopped before it finished: step");
        fireEvent.click(finish);
        await waitFor(() => expect(finish).toHaveAttribute("aria-busy", "true"));
        expect(finish).toBeDisabled();
        g.resolve(REPORT);
        await waitFor(() => expect(count("apply")).toBe(1));
    });

    it("a finished run offers Undo but not Finish the update", async () => {
        srv({ plans: [plan([A], [], { last_run: { journal: "j", finished: true } })] });
        render(() => <ForkSlugMigration />);
        await screen.findByRole("button", { name: "Update them" });
        expect(maybeBtn("Finish the update")).toBeNull();
        expect(btn("Undo the update")).toBeEnabled();
    });

    it("Update them is off when no agent can be updated yet, on when one can, and off while it works", async () => {
        srv({ plans: [plan([REFUSED])] });
        render(() => <ForkSlugMigration />);
        await screen.findByText(/can't be updated yet/);
        expect(btn("Update them")).toBeDisabled();
        cleanup();
        const g = gate<ForkSlugReport>();
        srv({ plans: [plan([A, REFUSED]), plan([])], apply: () => g.promise });
        render(() => <ForkSlugMigration />);
        await screen.findByText(/can't be updated yet/);
        expect(btn("Update them")).toBeEnabled();
        fireEvent.click(btn("Update them"));
        await waitFor(() => expect(btn("Update them")).toBeDisabled());
        g.resolve(REPORT);
    });

    it("Update them is off while something the stop can't clear is in the way", async () => {
        srv({ plans: [plan([A], [OLD])] });
        render(() => <ForkSlugMigration />);
        await screen.findByText("1 agent has an outdated id.");
        expect(btn("Update them")).toBeDisabled();
        expect(screen.getByRole("list", { name: "What has to change first" })).toHaveTextContent(
            "An older AgentMux is running."
        );
    });

    it("shows no list of what has to change first when nothing does", async () => {
        srv({ plans: [plan([A])] });
        render(() => <ForkSlugMigration />);
        await screen.findByText("1 agent has an outdated id.");
        expect(screen.queryByRole("list", { name: "What has to change first" })).not.toBeInTheDocument();
    });

    it("hides what has to change first while the card works", async () => {
        const g = gate<ForkSlugReport>();
        srv({
            plans: [plan([A], [{ ...RUNNING_BLOCK, clearable_by_stopping: true }]), plan([])],
            apply: () => g.promise,
        });
        render(() => <ForkSlugMigration />);
        await screen.findByRole("list", { name: "What has to change first" });
        fireEvent.click(btn("Update them"));
        await waitFor(() =>
            expect(screen.queryByRole("list", { name: "What has to change first" })).not.toBeInTheDocument()
        );
        g.resolve(REPORT);
    });
});

describe("the list", () => {
    it("shows each agent's old id, an arrow and the new one", async () => {
        srv({ plans: [plan([A])] });
        render(() => <ForkSlugMigration />);
        const list = await screen.findByRole("list", { name: "Agents with an outdated id" });
        expect(list.textContent).toContain("Loap #2 loap-loap-2 → loap-2");
    });

    it("notes that a running agent is stopped first, but not for a refused one or while a blocker can't be cleared", async () => {
        srv({
            plans: [
                plan(
                    [RUN_A, B, { ...REFUSED, running: [{ block_id: "b", channel: "stable", how: "l" }] }],
                    [RUNNING_BLOCK]
                ),
            ],
        });
        render(() => <ForkSlugMigration />);
        await screen.findByText(/can't be updated yet/);
        expect(screen.getAllByText("Running: it is stopped first.")).toHaveLength(1);
        cleanup();
        srv({ plans: [plan([RUN_A], [OLD])] });
        render(() => <ForkSlugMigration />);
        await screen.findByText("1 agent has an outdated id.");
        expect(screen.queryByText("Running: it is stopped first.")).not.toBeInTheDocument();
    });

    it("shows where the old ids are stored, with what each holds, and hides them again", async () => {
        srv({ plans: [plan([A])] });
        render(() => <ForkSlugMigration />);
        await screen.findByText("1 agent has an outdated id.");
        expect(document.querySelector(".setting-fork-slugs-where")).toBeNull();
        fireEvent.click(btn("Show where the old ids are stored"));
        const where = document.querySelector(".setting-fork-slugs-where")!;
        expect(where).toHaveTextContent("K2 loap-loap-2 definitions/x.json");
        fireEvent.click(btn("Hide where the old ids are stored"));
        expect(document.querySelector(".setting-fork-slugs-where")).toBeNull();
        expect(maybeBtn("Show where the old ids are stored")).not.toBeNull();
    });

    it("lists a holder after the first that has no proposal as needing a new id", async () => {
        const holder = { name: "Claude", place: "the shared definitions", created_at: 1 };
        srv({
            plans: [
                plan([], [], {
                    duplicates: [
                        {
                            slug: "claude-2",
                            holders: [
                                { ...holder, uid: "a" },
                                { ...holder, uid: "b", created_at: 2 },
                            ],
                        },
                    ],
                }),
            ],
        });
        render(() => <ForkSlugMigration />);
        const list = await screen.findByRole("list", { name: "Agents that share an id" });
        expect(list).toHaveTextContent("keeps it");
        expect(list).toHaveTextContent("needs a new id");
    });

    it("shows the report: each migrated agent old to new, the PR searches on separate lines, and the notes", async () => {
        srv({ plans: [plan([A]), plan([])] });
        render(() => <ForkSlugMigration />);
        await screen.findByText("1 agent has an outdated id.");
        fireEvent.click(btn("Update them"));
        const report = await screen.findByLabelText("Update report");
        expect(report.textContent).toContain("Updated Loap #2: loap-loap-2 → loap-2");
        expect(report.querySelector("pre")!.textContent).toBe(
            "gh-agent pr list --search one\ngh-agent pr list --search two"
        );
        expect(report).toHaveTextContent("Relaunch each migrated agent.");
    });

    it("shows no PR search when the report has none", async () => {
        srv({ plans: [plan([A]), plan([])], apply: { ...REPORT, pr_searches: [], notes: [] } });
        render(() => <ForkSlugMigration />);
        await screen.findByText("1 agent has an outdated id.");
        fireEvent.click(btn("Update them"));
        const report = await screen.findByLabelText("Update report");
        expect(report.querySelector("pre")).toBeNull();
    });
});

describe("picking an id", () => {
    async function refused() {
        srv({ plans: [plan([REFUSED]), plan([PICKED]), plan([])] });
        render(() => <ForkSlugMigration />);
        await screen.findByText(/can't be updated yet/);
    }
    const input = () => screen.getByRole("textbox", { name: "New id for Loap #3" });

    it("explains why an id is not available, and the next free one", async () => {
        srv({ plans: [plan([REFUSED])] });
        render(() => <ForkSlugMigration />);
        await screen.findByText(/can't be updated yet/);
        const note = document.querySelector(".setting-fork-slugs-list .setting-fork-slugs-note")!;
        expect(note.textContent).toContain("Not now: loap-3 is taken: agent x has it (the next free id is loap-3-2)");
        expect(note.textContent).not.toContain("Update the other agents first");
    });

    it("asks to update the others first only while others can still be updated", async () => {
        srv({ plans: [plan([A, REFUSED])] });
        render(() => <ForkSlugMigration />);
        await screen.findByText(/can't be updated yet/);
        expect(
            screen.getByText("Update the other agents first; then this one can take another id.")
        ).toBeInTheDocument();
        expect(btn("Use loap-3-2")).toBeDisabled();
        expect(btn("Choose another id")).toBeDisabled();
    });

    it("offers no pick while something that can't be cleared is in the way", async () => {
        srv({ plans: [plan([REFUSED], [OLD])] });
        render(() => <ForkSlugMigration />);
        await screen.findByText(/can't be updated yet/);
        expect(btn("Use loap-3-2")).toBeDisabled();
        expect(btn("Choose another id")).toBeDisabled();
    });

    it("offers no suggestion button when there is no suggestion", async () => {
        srv({ plans: [plan([{ ...REFUSED, suggestion: undefined }])] });
        render(() => <ForkSlugMigration />);
        await screen.findByText(/can't be updated yet/);
        expect(screen.queryByRole("button", { name: /^Use / })).toBeNull();
        expect(document.body.textContent).not.toContain("the next free id");
    });

    it("opens a field for an id of the user's own, and closes it on a second click", async () => {
        await refused();
        expect(screen.queryByRole("textbox")).toBeNull();
        fireEvent.click(btn("Choose another id"));
        expect(input()).toBeInTheDocument();
        fireEvent.click(btn("Choose another id"));
        expect(screen.queryByRole("textbox")).toBeNull();
    });

    it("takes the typed id, trimmed, on Enter, and only on Enter", async () => {
        await refused();
        fireEvent.click(btn("Choose another id"));
        fireEvent.input(input(), { target: { value: "  loap-3-2  " } });
        fireEvent.keyDown(input(), { key: "a" });
        fireEvent.keyDown(input(), { key: "Tab" });
        await tick();
        expect(count("plan")).toBe(1);
        fireEvent.keyDown(input(), { key: "Enter" });
        await waitFor(() => expect(calls().filter((c) => c.mode === "plan")[1].choices).toEqual(PICK));
    });

    it("does nothing on Enter with a blank id", async () => {
        await refused();
        fireEvent.click(btn("Choose another id"));
        fireEvent.input(input(), { target: { value: "   " } });
        fireEvent.keyDown(input(), { key: "Enter" });
        await tick();
        expect(count("plan")).toBe(1);
        expect(btn("Use this id")).toBeDisabled();
    });

    it("takes no second pick on Enter while the first is being checked", async () => {
        const g = gate<ForkSlugPlan>();
        srv({ plans: [plan([REFUSED]), () => g.promise, plan([])] });
        render(() => <ForkSlugMigration />);
        await screen.findByText(/can't be updated yet/);
        fireEvent.click(btn("Choose another id"));
        fireEvent.input(input(), { target: { value: "loap-3-2" } });
        fireEvent.keyDown(input(), { key: "Enter" });
        await waitFor(() => expect(count("plan")).toBe(2));
        fireEvent.keyDown(input(), { key: "Enter" });
        fireEvent.keyDown(input(), { key: "Enter" });
        await tick();
        expect(count("plan")).toBe(2);
        expect(btn("Use this id")).toBeDisabled();
        expect(btn("Use loap-3-2")).toBeDisabled();
        g.resolve(plan([PICKED]));
    });

    it("clears an earlier error when a pick starts", async () => {
        const g = gate<ForkSlugPlan>();
        srv({ plans: [plan([REFUSED]), plan([{ ...REFUSED, refused: "taken" }]), plan([REFUSED]), () => g.promise] });
        render(() => <ForkSlugMigration />);
        await screen.findByText(/can't be updated yet/);
        fireEvent.click(btn("Use loap-3-2"));
        expect(await screen.findByRole("alert")).toHaveTextContent("loap-3-2 can't be used: taken");
        await waitFor(() => expect(btn("Use loap-3-2")).toBeEnabled());
        fireEvent.click(btn("Use loap-3-2"));
        await waitFor(() => expect(alert()).toBeNull());
        g.resolve(plan([REFUSED]));
    });

    it("says why the id can't be used, with the reason srv gave, and asks srv again without the pick", async () => {
        srv({
            plans: [
                plan([REFUSED]),
                plan([{ ...REFUSED, refused: "loap-3-2 is taken: agent y has it" }]),
                plan([REFUSED]),
            ],
        });
        render(() => <ForkSlugMigration />);
        await screen.findByText(/can't be updated yet/);
        fireEvent.click(btn("Use loap-3-2"));
        expect(await screen.findByRole("alert")).toHaveTextContent(
            "loap-3-2 can't be used: loap-3-2 is taken: agent y has it"
        );
        const plans = calls().filter((c) => c.mode === "plan");
        expect(plans[1].choices).toEqual(PICK);
        expect(plans[2].choices).toBeUndefined();
        expect(count("apply")).toBe(0);
        await waitFor(() => expect(btn("Use loap-3-2")).toBeEnabled());
    });

    it("updates nothing, and shows no error, when the agent is gone from the plan checked with the pick", async () => {
        srv({ plans: [plan([REFUSED]), plan([]), plan([REFUSED])] });
        render(() => <ForkSlugMigration />);
        await screen.findByText(/can't be updated yet/);
        fireEvent.click(btn("Use loap-3-2"));
        await waitFor(() => expect(count("plan")).toBe(3));
        await tick();
        expect(alert()).toBeNull();
        expect(count("apply")).toBe(0);
        expect(calls().filter((c) => c.mode === "plan")[2].choices).toBeUndefined();
        await waitFor(() => expect(btn("Use loap-3-2")).toBeEnabled());
    });

    it("sends the pick with the update, then closes the field and forgets what was typed", async () => {
        srv({ plans: [plan([REFUSED]), plan([PICKED]), plan([REFUSED])] });
        render(() => <ForkSlugMigration />);
        await screen.findByText(/can't be updated yet/);
        fireEvent.click(btn("Choose another id"));
        fireEvent.input(input(), { target: { value: "loap-3-2" } });
        fireEvent.click(btn("Use this id"));
        await waitFor(() => expect(count("apply")).toBe(1));
        expect(calls().find((c) => c.mode === "apply")!.choices).toEqual(PICK);
        // the agent is refused again once the update is done: the field is closed, and empty when opened
        await waitFor(() => expect(count("plan")).toBe(3));
        await screen.findByText(/can't be updated yet/);
        expect(screen.queryByRole("textbox")).toBeNull();
        fireEvent.click(btn("Choose another id"));
        expect(input()).toHaveValue("");
    });

    it("keeps no pick after the update", async () => {
        srv({ plans: [plan([REFUSED]), plan([PICKED]), plan([PICKED]), plan([])] });
        render(() => <ForkSlugMigration />);
        await screen.findByText(/can't be updated yet/);
        fireEvent.click(btn("Use loap-3-2"));
        await waitFor(() => expect(count("apply")).toBe(1));
        await waitFor(() => expect(count("plan")).toBe(3));
        expect(calls().filter((c) => c.mode === "plan")[2].choices).toBeUndefined();
    });

    it("a pick for an agent that is running asks before it stops the panes", async () => {
        const pickedRunning = { ...PICKED, running: [{ block_id: "b1", channel: "stable", how: "lease" }] };
        srv({ plans: [plan([REFUSED]), plan([pickedRunning], [RUNNING_BLOCK])] });
        render(() => <ForkSlugMigration />);
        await screen.findByText(/can't be updated yet/);
        fireEvent.click(btn("Use loap-3-2"));
        expect(await screen.findByRole("dialog")).toHaveTextContent("Stop 1 pane?");
        expect(count("apply")).toBe(0);
    });
});

describe("Undo", () => {
    it("undoes the run of this session, not the plan's older one", async () => {
        srv({
            plans: [
                plan([A], [], { last_run: { journal: "old.jsonl", finished: true } }),
                plan([], [], { last_run: { journal: "old.jsonl", finished: true } }),
            ],
        });
        render(() => <ForkSlugMigration />);
        await screen.findByRole("button", { name: "Update them" });
        fireEvent.click(btn("Update them"));
        await screen.findByLabelText("Update report");
        await waitFor(() => expect(btn("Undo the update")).toBeEnabled());
        fireEvent.click(btn("Undo the update"));
        fireEvent.click(within(await screen.findByRole("dialog")).getByRole("button", { name: "Undo" }));
        await waitFor(() => expect(count("rollback")).toBe(1));
        expect(calls().find((c) => c.mode === "rollback")!.journal).toBe(REPORT.journal);
    });

    it("undoes the plan's run when the session has none", async () => {
        srv({ plans: [plan([], [], { last_run: { journal: "old.jsonl", finished: true } })] });
        render(() => <ForkSlugMigration />);
        fireEvent.click(await screen.findByRole("button", { name: "Undo the update" }));
        fireEvent.click(within(await screen.findByRole("dialog")).getByRole("button", { name: "Undo" }));
        await waitFor(() => expect(count("rollback")).toBe(1));
        expect(calls().find((c) => c.mode === "rollback")!.journal).toBe("old.jsonl");
    });

    it("offers no Undo without a run to undo", async () => {
        srv({ plans: [plan([A])] });
        render(() => <ForkSlugMigration />);
        await screen.findByText("1 agent has an outdated id.");
        expect(maybeBtn("Undo the update")).toBeNull();
    });

    it("Cancel closes the question and undoes nothing", async () => {
        srv({ plans: [plan([], [], { last_run: { journal: "j", finished: true } })] });
        render(() => <ForkSlugMigration />);
        fireEvent.click(await screen.findByRole("button", { name: "Undo the update" }));
        fireEvent.click(within(await screen.findByRole("dialog")).getByRole("button", { name: "Cancel" }));
        await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
        expect(count("rollback")).toBe(0);
    });

    it("does nothing when the run to undo is gone by the time it is confirmed", async () => {
        let now = 1_000_000;
        vi.spyOn(Date, "now").mockImplementation(() => now);
        srv({ plans: [plan([], [], { last_run: { journal: "j", finished: true } }), plan([])] });
        render(() => <ForkSlugMigration />);
        fireEvent.click(await screen.findByRole("button", { name: "Undo the update" }));
        const dialog = await screen.findByRole("dialog");
        now += RECHECK_AFTER_MS + 1;
        fireEvent(window, new Event("focus"));
        await waitFor(() => expect(count("plan")).toBe(2));
        await tick();
        fireEvent.click(within(dialog).getByRole("button", { name: "Undo" }));
        await tick();
        expect(count("rollback")).toBe(0);
        expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    });

    it("the Undo button is off while the card works", async () => {
        const g = gate<ForkSlugReport>();
        srv({
            plans: [plan([A], [], { last_run: { journal: "j", finished: true } }), plan([])],
            apply: () => g.promise,
        });
        render(() => <ForkSlugMigration />);
        await screen.findByRole("button", { name: "Update them" });
        expect(btn("Undo the update")).toBeEnabled();
        fireEvent.click(btn("Update them"));
        await waitFor(() => expect(btn("Undo the update")).toBeDisabled());
        g.resolve(REPORT);
    });
});

describe("coming back into view", () => {
    let now = 1_000_000;
    beforeEach(() => {
        now = 1_000_000;
        vi.spyOn(Date, "now").mockImplementation(() => now);
    });

    it("starts no second read when the first is still under way, however long ago the plan was read", async () => {
        const g = gate<ForkSlugPlan>();
        srv({ plans: [plan([A]), () => g.promise] });
        render(() => <ForkSlugMigration />);
        await screen.findByText("1 agent has an outdated id.");
        now += RECHECK_AFTER_MS + 1;
        fireEvent(window, new Event("focus"));
        await waitFor(() => expect(count("plan")).toBe(2));
        now += 10 * RECHECK_AFTER_MS;
        fireEvent(window, new Event("focus"));
        document.dispatchEvent(new Event("visibilitychange"));
        await tick();
        expect(count("plan")).toBe(2);
        g.resolve(plan([]));
    });

    it("reads again after the first read is done, once the plan is old enough", async () => {
        srv({ plans: [plan([A]), plan([A]), plan([])] });
        render(() => <ForkSlugMigration />);
        await screen.findByText("1 agent has an outdated id.");
        now += RECHECK_AFTER_MS + 1;
        fireEvent(window, new Event("focus"));
        await waitFor(() => expect(count("plan")).toBe(2));
        await tick();
        now += RECHECK_AFTER_MS + 1;
        fireEvent(window, new Event("focus"));
        await screen.findByText("Every agent has an up-to-date id.");
    });

    it("ignores the tab being hidden", async () => {
        srv({ plans: [plan([A]), plan([])] });
        render(() => <ForkSlugMigration />);
        await screen.findByText("1 agent has an outdated id.");
        now += RECHECK_AFTER_MS + 1;
        const state = vi.spyOn(document, "visibilityState", "get").mockReturnValue("hidden");
        document.dispatchEvent(new Event("visibilitychange"));
        await tick();
        expect(count("plan")).toBe(1);
        state.mockReturnValue("visible");
        document.dispatchEvent(new Event("visibilitychange"));
        await screen.findByText("Every agent has an up-to-date id.");
    });

    describe("when the card scrolls into view", () => {
        const seen: Array<{
            cb: (e: Array<{ isIntersecting: boolean }>) => void;
            observe: ReturnType<typeof vi.fn>;
            disconnect: ReturnType<typeof vi.fn>;
        }> = [];
        beforeEach(() => {
            seen.length = 0;
            vi.stubGlobal(
                "IntersectionObserver",
                class {
                    observe = vi.fn();
                    disconnect = vi.fn();
                    constructor(cb: (e: Array<{ isIntersecting: boolean }>) => void) {
                        seen.push({ cb, observe: this.observe, disconnect: this.disconnect });
                    }
                }
            );
        });

        it("watches the card itself and reads again when it is intersecting", async () => {
            srv({ plans: [plan([A]), plan([])] });
            render(() => <ForkSlugMigration />);
            await screen.findByText("1 agent has an outdated id.");
            expect(seen).toHaveLength(1);
            const card = document.getElementById("setting-agents.outdated_ids");
            expect(card).not.toBeNull();
            expect(seen[0].observe).toHaveBeenCalledWith(card);
            now += RECHECK_AFTER_MS + 1;
            seen[0].cb([{ isIntersecting: false }]);
            await tick();
            expect(count("plan")).toBe(1);
            seen[0].cb([{ isIntersecting: false }, { isIntersecting: true }]);
            await screen.findByText("Every agent has an up-to-date id.");
        });

        it("stops watching when the card goes away", async () => {
            srv({ plans: [plan([A])] });
            const { unmount } = render(() => <ForkSlugMigration />);
            await screen.findByText("1 agent has an outdated id.");
            unmount();
            expect(seen[0].disconnect).toHaveBeenCalledTimes(1);
        });
    });
});
