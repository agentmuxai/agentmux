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
    return { candidates, skipped: [], blockers, summary: "" };
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
        await screen.findByText("2 agents have an outdated id.");
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
});
