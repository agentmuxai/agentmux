// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** Settings › Agents › Outdated agent ids: the plan, Update them, stopping
 *  running panes first, the report and Undo. */

import { cleanup, fireEvent, render, screen, waitFor, within } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { ForkSlugBlocker } from "@/types/rpc/ForkSlugBlocker";
import type { ForkSlugCandidate } from "@/types/rpc/ForkSlugCandidate";
import type { ForkSlugPlan } from "@/types/rpc/ForkSlugPlan";

const migrate = vi.fn();
vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: { AgentsMigrateForkSlugsCommand: (...args: unknown[]) => migrate(...args) },
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));

import { ForkSlugMigration, planLine } from "./fork-slug-migration";

function cand(name: string, old_slug: string, new_slug: string, extra: Partial<ForkSlugCandidate> = {}): ForkSlugCandidate {
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

const THREE = [cand("Loap #2", "loap-loap-2", "loap-2"), cand("Naki #2", "naki-naki-2", "naki-2"), cand("Loap #3", "loap-loap-2-loap-3", "loap-3")];

function plan(candidates: ForkSlugCandidate[], blockers: ForkSlugBlocker[] = []): ForkSlugPlan {
    return { candidates, skipped: [], blockers, summary: "", duplicates: [] };
}

const RUNNING: ForkSlugBlocker = {
    kind: "running",
    message: "Loap #2 is running in stable (holds its lease): stop it first",
    clearable_by_stopping: true,
};
const OLD: ForkSlugBlocker = {
    kind: "old_instance",
    message: "An older AgentMux is running: channels/stable@0.58.1 (pid 4). Update or quit it first.",
    clearable_by_stopping: false,
};

const RUNNING_LOAP2 = [cand("Loap #2", "loap-loap-2", "loap-2", { running: [{ block_id: "b1", channel: "stable", how: "holds its lease" }] })];

const report = {
    journal: "/home/shared/backups/slug-migration-x/journal.jsonl",
    backup_dir: "/home/shared/backups/slug-migration-x",
    migrated: THREE.map((c) => ({ uid: c.uid, name: c.name, old_slug: c.old_slug, new_slug: c.new_slug, changes: [] })),
    pr_searches: ['gh-agent pr list --state open --search "agentmux:agent_id=loap-loap-2 in:body"'],
    notes: ["Relaunch each migrated agent."],
};

// An agent whose new id is taken, and the same agent once the user picked loap-3-2.
const REFUSED = cand("Loap #3", "loap-loap-2-loap-3", "loap-3", {
    uid: "uid-loap-3",
    refused: "loap-3 is taken: agent x has it",
    suggestion: "loap-3-2",
});
const PICKED = cand("Loap #3", "loap-loap-2-loap-3", "loap-3-2", { uid: "uid-loap-3" });
const PICK = [{ uid: "uid-loap-3", new_slug: "loap-3-2" }];

/** srv: `plans` answers each plan in turn (the last one repeats). */
function srv(plans: ForkSlugPlan[]) {
    let i = 0;
    migrate.mockImplementation(async (_client: unknown, data: { mode: string }) => {
        switch (data.mode) {
            case "plan":
                return { mode: "plan", plan: plans[Math.min(i++, plans.length - 1)], pending: [], wait_ms: 0, summary: "" };
            case "stop":
                return { mode: "stop", pending: [{ block_id: "b1", request_id: "r1" }], wait_ms: 0, summary: "" };
            case "apply":
                return { mode: "apply", report, pending: [], wait_ms: 0, summary: "" };
            case "rollback":
                return { mode: "rollback", report: { ...report, migrated: [] }, pending: [], wait_ms: 0, summary: "" };
        }
        throw new Error(`unexpected mode ${data.mode}`);
    });
}

const modes = () => migrate.mock.calls.map((c) => (c[1] as { mode: string }).mode);

describe("planLine", () => {
    it("counts the agents", () => {
        expect(planLine(plan(THREE))).toBe("3 agents have an outdated id.");
        expect(planLine(plan(THREE.slice(0, 1)))).toBe("1 agent has an outdated id.");
        expect(planLine(plan([]))).toBe("Every agent has an up-to-date id.");
    });

    it("says when agents can't be updated yet because their new id is taken", () => {
        expect(planLine(plan([THREE[0], REFUSED]))).toBe("2 agents have an outdated id. 1 can't be updated yet: pick another id below.");
        expect(planLine(plan([REFUSED]))).toBe("1 agent has an outdated id. It can't be updated yet: pick another id below.");
        expect(planLine(plan([REFUSED, { ...REFUSED, uid: "uid-b" }]))).toBe(
            "2 agents have an outdated id. None can be updated yet: pick another id below.",
        );
    });
});

describe("ForkSlugMigration", () => {
    beforeEach(() => {
        migrate.mockReset();
    });
    afterEach(() => cleanup());

    it("shows the plan and updates the agents, then the report and the PR searches", async () => {
        srv([plan(THREE), plan([])]);
        render(() => <ForkSlugMigration />);
        expect(await screen.findByText("3 agents have an outdated id.")).toBeInTheDocument();
        const list = screen.getByRole("list", { name: "Agents with an outdated id" });
        expect(within(list).getByText("loap-loap-2-loap-3")).toBeInTheDocument();
        fireEvent.click(screen.getByRole("button", { name: "Show where the old ids are stored" }));
        expect(within(list).getAllByText("K2")).toHaveLength(3);

        fireEvent.click(screen.getByRole("button", { name: "Update them" }));
        await screen.findByText(/agentmux:agent_id=loap-loap-2 in:body/);
        expect(await screen.findByText("Every agent has an up-to-date id.")).toBeInTheDocument();
        expect(modes()).toEqual(["plan", "apply", "plan"]);
        expect(screen.getByLabelText("Update report")).toHaveTextContent("Updated Naki #2");
    });

    it("lists agents that share an id, with the id proposed for the later one, and offers nothing to run", async () => {
        const holder = { name: "Claude", place: "the shared definitions" };
        const shared: ForkSlugPlan = {
            ...plan([]),
            duplicates: [
                {
                    slug: "claude-2",
                    holders: [
                        { ...holder, uid: "uid-a", created_at: 1 },
                        { ...holder, uid: "uid-b", created_at: 2, proposed: "claude-3" },
                    ],
                },
            ],
        };
        srv([shared]);
        render(() => <ForkSlugMigration />);
        await screen.findByText("Every agent has an up-to-date id.");
        const list = screen.getByRole("list", { name: "Agents that share an id" });
        expect(list).toHaveTextContent("claude-2");
        expect(list).toHaveTextContent("keeps it");
        expect(within(list).getByText("claude-3")).toBeInTheDocument();
        expect(screen.queryByRole("button", { name: "Update them" })).not.toBeInTheDocument();
    });

    it("stops the running panes first, through the override, then updates", async () => {
        const stopped = RUNNING_LOAP2.map((c) => ({ ...c, running: [] }));
        srv([plan(RUNNING_LOAP2, [RUNNING]), plan(RUNNING_LOAP2, [RUNNING]), plan(stopped), plan([])]);
        render(() => <ForkSlugMigration />);
        await screen.findByText("1 agent has an outdated id.");
        expect(screen.getByText(/stop it first/)).toBeInTheDocument();
        fireEvent.click(screen.getByRole("button", { name: "Update them" }));
        const dialog = await screen.findByRole("dialog");
        expect(dialog).toHaveTextContent("Stop 1 pane?");
        expect(dialog).toHaveTextContent("Loap #2 (stable)");
        fireEvent.click(within(dialog).getByRole("button", { name: "Stop and update" }));
        await waitFor(() => expect(modes()).toEqual(["plan", "plan", "stop", "plan", "apply", "plan"]));
    });

    it("offers no stop, and stops nothing, while an older AgentMux also blocks", async () => {
        srv([plan(RUNNING_LOAP2, [RUNNING, OLD])]);
        render(() => <ForkSlugMigration />);
        await screen.findByText("1 agent has an outdated id.");
        expect(screen.getByText(/An older AgentMux is running: channels\/stable@0.58.1/)).toBeInTheDocument();
        expect(screen.queryByText("Running: it is stopped first.")).not.toBeInTheDocument();
        const update = screen.getByRole("button", { name: "Update them" });
        expect(update).toBeDisabled();
        fireEvent.click(update);
        expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
        expect(modes()).toEqual(["plan"]);
    });

    it("reads the plan again before stopping, and stops nothing if a blocker it can't clear appeared", async () => {
        srv([plan(RUNNING_LOAP2, [RUNNING]), plan(RUNNING_LOAP2, [RUNNING, OLD])]);
        render(() => <ForkSlugMigration />);
        await screen.findByText("1 agent has an outdated id.");
        fireEvent.click(screen.getByRole("button", { name: "Update them" }));
        const dialog = await screen.findByRole("dialog");
        fireEvent.click(within(dialog).getByRole("button", { name: "Stop and update" }));
        await screen.findByText(/An older AgentMux is running/);
        expect(modes()).toEqual(["plan", "plan"]);
        expect(screen.getByRole("button", { name: "Update them" })).toBeDisabled();
    });

    it("can't start while something it can't clear blocks it, and says why an agent is refused", async () => {
        const refused = cand("Loap #3", "loap-loap-2-loap-3", "loap-3", { refused: "loap-3 is taken: agent x has it", suggestion: "loap-3-2" });
        srv([plan([THREE[0], refused], [OLD])]);
        render(() => <ForkSlugMigration />);
        await screen.findByText("2 agents have an outdated id. 1 can't be updated yet: pick another id below.");
        expect(screen.getByText(/the next free id is loap-3-2/)).toBeInTheDocument();
        expect(screen.getByText(/An older AgentMux is running/)).toBeInTheDocument();
        expect(screen.getByRole("button", { name: "Update them" })).toBeDisabled();
    });

    it("undoes a finished update from its journal", async () => {
        srv([plan(THREE), plan([]), plan(THREE)]);
        render(() => <ForkSlugMigration />);
        await screen.findByText("3 agents have an outdated id.");
        fireEvent.click(screen.getByRole("button", { name: "Update them" }));
        const undo = await screen.findByRole("button", { name: "Undo the update" });
        await waitFor(() => expect(undo).toBeEnabled());
        fireEvent.click(undo);
        const dialog = await screen.findByRole("dialog");
        fireEvent.click(within(dialog).getByRole("button", { name: "Undo" }));
        await waitFor(() => expect(modes()).toEqual(["plan", "apply", "plan", "rollback", "plan"]));
        expect(migrate.mock.calls[3][1]).toEqual({ mode: "rollback", journal: report.journal });
        expect(await screen.findByText("3 agents have an outdated id.")).toBeInTheDocument();
    });

    it("finishes or undoes a run that stopped partway, after the session that ran it is gone", async () => {
        const stopped: ForkSlugPlan = { ...plan([]), last_run: { journal: report.journal, finished: false, failed: "uid-loap-3: crashed" } };
        srv([stopped, plan([])]);
        render(() => <ForkSlugMigration />);
        expect(await screen.findByText("An update stopped before it finished: uid-loap-3: crashed")).toBeInTheDocument();
        expect(screen.queryByRole("button", { name: "Update them" })).not.toBeInTheDocument();
        expect(screen.getByRole("button", { name: "Undo the update" })).toBeEnabled();
        fireEvent.click(screen.getByRole("button", { name: "Finish the update" }));
        await waitFor(() => expect(modes()).toEqual(["plan", "apply", "plan"]));
    });

    it("undoes the last finished run from the plan's journal", async () => {
        srv([{ ...plan([]), last_run: { journal: report.journal, finished: true } }, plan([])]);
        render(() => <ForkSlugMigration />);
        await screen.findByText("Every agent has an up-to-date id.");
        fireEvent.click(screen.getByRole("button", { name: "Undo the update" }));
        fireEvent.click(within(await screen.findByRole("dialog")).getByRole("button", { name: "Undo" }));
        await waitFor(() => expect(modes()).toEqual(["plan", "rollback", "plan"]));
        expect(migrate.mock.calls[1][1]).toEqual({ mode: "rollback", journal: report.journal });
    });

    it("gives a refused agent the suggested id with one click, then clears the pick", async () => {
        srv([plan([REFUSED]), plan([PICKED]), plan([])]);
        render(() => <ForkSlugMigration />);
        await screen.findByText(/It can't be updated yet/);
        expect(screen.getByRole("button", { name: "Update them" })).toBeDisabled();
        fireEvent.click(screen.getByRole("button", { name: "Use loap-3-2" }));
        await waitFor(() => expect(modes()).toEqual(["plan", "plan", "apply", "plan"]));
        expect(migrate.mock.calls[1][1]).toEqual({ mode: "plan", choices: PICK });
        expect(migrate.mock.calls[2][1]).toEqual({ mode: "apply", choices: PICK });
        expect(migrate.mock.calls[3][1]).toEqual({ mode: "plan" });
        expect(await screen.findByText("Every agent has an up-to-date id.")).toBeInTheDocument();
    });

    it("says why a picked id can't be used, updates nothing and shows the plan as it was", async () => {
        const taken = { ...REFUSED, new_slug: "loap-3-2", refused: "loap-3-2 is taken: agent y has it" };
        srv([plan([REFUSED]), plan([taken]), plan([REFUSED])]);
        render(() => <ForkSlugMigration />);
        await screen.findByText(/It can't be updated yet/);
        fireEvent.click(screen.getByRole("button", { name: "Use loap-3-2" }));
        expect(await screen.findByRole("alert")).toHaveTextContent("loap-3-2 can't be used: loap-3-2 is taken: agent y has it");
        await waitFor(() => expect(modes()).toEqual(["plan", "plan", "plan"]));
        expect(migrate.mock.calls[2][1]).toEqual({ mode: "plan" });
        expect(screen.getByRole("button", { name: "Use loap-3-2" })).toBeEnabled();
    });

    it("takes an id of the user's own for a refused agent", async () => {
        srv([plan([{ ...REFUSED, suggestion: undefined }]), plan([{ ...PICKED, new_slug: "loap-three" }]), plan([])]);
        render(() => <ForkSlugMigration />);
        await screen.findByText(/It can't be updated yet/);
        expect(screen.queryByRole("button", { name: /^Use loap/ })).not.toBeInTheDocument();
        fireEvent.click(screen.getByRole("button", { name: "Choose another id" }));
        const input = screen.getByLabelText("New id for Loap #3");
        const use = screen.getByRole("button", { name: "Use this id" });
        expect(use).toBeDisabled();
        fireEvent.input(input, { target: { value: " loap-three " } });
        fireEvent.click(use);
        await waitFor(() => expect(modes()).toEqual(["plan", "plan", "apply", "plan"]));
        expect(migrate.mock.calls[1][1]).toEqual({ mode: "plan", choices: [{ uid: "uid-loap-3", new_slug: "loap-three" }] });
    });

    it("offers no pick while other agents can still be updated by the rule", async () => {
        srv([plan([THREE[0], REFUSED])]);
        render(() => <ForkSlugMigration />);
        await screen.findByText(/1 can't be updated yet/);
        expect(screen.getByText("Update the other agents first; then this one can take another id.")).toBeInTheDocument();
        expect(screen.getByRole("button", { name: "Use loap-3-2" })).toBeDisabled();
        expect(screen.getByRole("button", { name: "Choose another id" })).toBeDisabled();
    });

    it("stops the panes an agent runs in, with the pick, before it updates it", async () => {
        const running = { ...PICKED, running: [{ block_id: "b1", channel: "stable", how: "holds its lease" }] };
        const stopped = { ...PICKED, running: [] };
        srv([plan([REFUSED]), plan([running], [RUNNING]), plan([running], [RUNNING]), plan([stopped]), plan([])]);
        render(() => <ForkSlugMigration />);
        await screen.findByText(/It can't be updated yet/);
        fireEvent.click(screen.getByRole("button", { name: "Use loap-3-2" }));
        const dialog = await screen.findByRole("dialog");
        fireEvent.click(within(dialog).getByRole("button", { name: "Stop and update" }));
        await waitFor(() => expect(modes()).toEqual(["plan", "plan", "plan", "stop", "plan", "apply", "plan"]));
        expect(migrate.mock.calls[3][1]).toEqual({ mode: "stop", choices: PICK });
        expect(migrate.mock.calls[5][1]).toEqual({ mode: "apply", choices: PICK });
    });

    it("drops the pick when the stop is cancelled", async () => {
        const running = { ...PICKED, running: [{ block_id: "b1", channel: "stable", how: "holds its lease" }] };
        srv([plan([REFUSED]), plan([running], [RUNNING]), plan([REFUSED])]);
        render(() => <ForkSlugMigration />);
        await screen.findByText(/It can't be updated yet/);
        fireEvent.click(screen.getByRole("button", { name: "Use loap-3-2" }));
        const dialog = await screen.findByRole("dialog");
        fireEvent.click(within(dialog).getByRole("button", { name: "Cancel" }));
        await waitFor(() => expect(modes()).toEqual(["plan", "plan", "plan"]));
        expect(migrate.mock.calls[2][1]).toEqual({ mode: "plan" });
        expect(await screen.findByRole("button", { name: "Use loap-3-2" })).toBeEnabled();
    });
});
