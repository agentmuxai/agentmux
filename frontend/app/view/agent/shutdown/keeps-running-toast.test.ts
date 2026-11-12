// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { beforeEach, describe, expect, it, vi } from "vitest";

const h = vi.hoisted(() => ({
    shutdownAgent: vi.fn(),
    shutdownPreview: vi.fn(),
    openPaneModal: vi.fn(),
    openAgentInActiveTab: vi.fn(),
    /** Every push, in order, and the toasts on screen by id (a click removes its toast, as the real store does). */
    pushed: [] as any[],
    shown: new Map<string, any>(),
    actions: new Map<string, () => void>(),
    rows: [] as any[],
    blockMeta: {} as Record<string, Record<string, unknown>>,
}));

vi.mock("@/store/services", () => ({ ObjectService: { ShutdownAgent: h.shutdownAgent, ShutdownPreview: h.shutdownPreview } }));
vi.mock("@/app/store/pane-requests", () => ({ openPaneModal: h.openPaneModal }));
vi.mock("./shutdown-log", () => ({ beginShutdownLog: vi.fn(), endShutdownLog: vi.fn() }));
vi.mock("./shutdown-confirm", () => ({ ShutdownConfirm: () => null }));
vi.mock("@/app/store/flash-notifications", () => ({
    pushNotification: (n: any) => {
        h.pushed.push(n);
        h.shown.set(n.id, n);
    },
}));
vi.mock("@/app/notification/usenotification", () => ({ setNotificationAction: (key: string, fn: () => void) => h.actions.set(key, fn) }));
vi.mock("@/app/store/agent-runtime-live", () => ({
    openAgentInActiveTab: h.openAgentInActiveTab,
    sharedAgentRuntime: () => ({ rows: () => h.rows }),
}));
vi.mock("@/app/store/agent-parked", () => ({ isParkedMeta: (m: Record<string, unknown> | undefined) => m?.["agent:parked"] === true }));
vi.mock("@/store/mos", () => ({
    makeORef: (type: string, id: string) => `${type}:${id}`,
    getObjectValue: (oref: string) => ({ meta: h.blockMeta[oref.split(":")[1]] }),
}));
vi.mock("../commands/global/quit", () => ({ quitNoticeMessage: (s: { agent: string }) => `Conversation of ${s.agent} kept.` }));

import { noteKeepsRunning } from "./keeps-running-toast";

const idle = { working: false, claims: 0, crons_targeting: [] };
const quit = (agent: string, over = {}) => ({ status: "quit", agent, released_claims: 0, stopped_shells: 0, crons_targeting: [], ...over });

const agent = (id: string, name: string) => ({ agentId: id, name });
const parkedRow = (id: string, name: string, state = "idle") => {
    h.rows.push({ agent_id: id, name, block_id: `b-${id}`, state });
    h.blockMeta[`b-${id}`] = { "agent:parked": true };
};

const flush = async () => {
    for (let i = 0; i < 10; i++) await Promise.resolve();
    await new Promise((r) => setTimeout(r, 0));
};

/** The person clicks a button of the toast `id`: its handler runs, then the toast goes (usenotification.handleActionClick). */
async function click(id: string, label: string) {
    const face = h.shown.get(id);
    const action = face.actions.find((a: NotificationActionType) => a.label === label);
    expect(action, `a "${label}" button on "${face.title}"`).toBeTruthy();
    h.actions.get(action.actionKey)!();
    h.shown.delete(id);
    await flush();
}

const labels = (n: any) => (n.actions ?? []).map((a: NotificationActionType) => a.label);
const titles = () => h.pushed.map((n) => n.title);

beforeEach(() => {
    h.pushed.length = 0;
    h.shown.clear();
    h.actions.clear();
    h.rows.length = 0;
    for (const k of Object.keys(h.blockMeta)) delete h.blockMeta[k];
    h.shutdownAgent.mockReset();
    h.shutdownPreview.mockReset().mockResolvedValue(idle);
    h.openPaneModal.mockReset();
    h.openAgentInActiveTab.mockReset();
});

describe("the keeps-running toast", () => {
    it("offers Open and, to its right, Shutdown; Open still opens the first agent", async () => {
        parkedRow("q", "Quen");
        noteKeepsRunning([agent("q", "Quen")]);
        const toast = h.pushed[0];
        expect(toast.title).toBe("Quen keeps running without a pane");
        expect(toast.message).toBe("Closing keeps it running. Shut down ends it.");
        expect(labels(toast)).toEqual(["Open", "Shutdown"]);
        h.actions.get(toast.actions[0].actionKey)!();
        expect(h.openAgentInActiveTab).toHaveBeenCalledWith({ agent_id: "q", name: "Quen" });
    });

    it("shuts an idle agent down on one click: working with no buttons, then the result, which expires", async () => {
        parkedRow("q", "Quen");
        h.shutdownAgent.mockResolvedValue(quit("Quen"));
        noteKeepsRunning([agent("q", "Quen")]);
        await click("keeps-running:q", "Shutdown");

        expect(h.shutdownAgent).toHaveBeenCalledWith("b-q", false);
        expect(titles()).toEqual(["Quen keeps running without a pane", "Shutting down Quen…", "Shutting down Quen…", "Quen was shut down"]);
        const working = h.pushed[1];
        expect(working.actions).toBeUndefined();
        expect(working.expiration, "no expiry while it works").toBeUndefined();
        const done = h.pushed[3];
        expect(done.message).toBe("Conversation of Quen kept.");
        expect(done.actions).toBeUndefined();
        expect(done.expiration).toBeGreaterThan(Date.now() + 5000);
        expect(h.openPaneModal, "asked in the toast, never in a modal").not.toHaveBeenCalled();
        expect(h.shown.get("keeps-running:q").title, "the toast is never left closed with nothing said").toBe("Quen was shut down");
    });

    it("asks inside the toast when the agent is mid-turn, and Keep running leaves it be", async () => {
        parkedRow("q", "Quen");
        h.shutdownPreview.mockResolvedValue({ ...idle, working: true });
        noteKeepsRunning([agent("q", "Quen")]);
        await click("keeps-running:q", "Shutdown");

        const ask = h.shown.get("keeps-running:q");
        expect(ask.title).toBe("Shut down Quen?");
        expect(ask.message).toContain("middle of a turn");
        expect(labels(ask)).toEqual(["Shutdown", "Keep running"]);
        expect(h.shutdownAgent).not.toHaveBeenCalled();
        expect(h.openPaneModal).not.toHaveBeenCalled();

        await click("keeps-running:q", "Keep running");
        expect(h.shutdownAgent).not.toHaveBeenCalled();
        expect(h.shown.has("keeps-running:q")).toBe(false);
    });

    it("shuts a mid-turn agent down once the person confirms in the toast", async () => {
        parkedRow("q", "Quen");
        h.shutdownPreview.mockResolvedValue({ ...idle, working: true });
        h.shutdownAgent.mockResolvedValue(quit("Quen"));
        noteKeepsRunning([agent("q", "Quen")]);
        await click("keeps-running:q", "Shutdown");
        await click("keeps-running:q", "Shutdown");
        expect(h.shutdownAgent).toHaveBeenCalledTimes(1);
        expect(h.shown.get("keeps-running:q").title).toBe("Quen was shut down");
    });

    it("asks when it cannot tell what would be lost", async () => {
        parkedRow("q", "Quen");
        h.shutdownPreview.mockRejectedValue(new Error("no preview"));
        noteKeepsRunning([agent("q", "Quen")]);
        await click("keeps-running:q", "Shutdown");
        expect(labels(h.shown.get("keeps-running:q"))).toEqual(["Shutdown", "Keep running"]);
        expect(h.shutdownAgent).not.toHaveBeenCalled();
    });

    it("says why it failed, in words, with a Retry that tries again", async () => {
        parkedRow("q", "Quen");
        h.shutdownAgent.mockRejectedValueOnce(new Error("the agent did not stop in time")).mockResolvedValue(quit("Quen"));
        noteKeepsRunning([agent("q", "Quen")]);
        await click("keeps-running:q", "Shutdown");

        const failed = h.shown.get("keeps-running:q");
        expect(failed.title).toBe("Couldn't shut down Quen");
        expect(failed.message).toContain("the agent did not stop in time");
        expect(failed.type).toBe("error");
        expect(labels(failed)).toEqual(["Retry"]);
        expect(failed.expiration, "an error with a button stays until acted on").toBeUndefined();

        await click("keeps-running:q", "Retry");
        expect(h.shutdownAgent).toHaveBeenCalledTimes(2);
        expect(h.shown.get("keeps-running:q").title).toBe("Quen was shut down");
    });

    it("shuts several agents down together, counts them, and names the one that failed; Retry takes only that one", async () => {
        for (const [id, name] of [["a", "Ann"], ["b", "Bob"], ["c", "Cy"]]) parkedRow(id, name);
        h.shutdownAgent.mockImplementation(async (blockId: string) => {
            if (blockId === "b-b") throw new Error("busy");
            return quit(blockId === "b-a" ? "Ann" : "Cy");
        });
        noteKeepsRunning([agent("a", "Ann"), agent("b", "Bob"), agent("c", "Cy")]);
        const id = "keeps-running:a,b,c";
        expect(h.pushed[0].title).toBe("Ann, Bob and 1 more keep running without a pane");
        await click(id, "Shutdown");

        expect(h.pushed.some((n) => n.title === "Shutting down 3 agents…" && n.message === "0 of 3 done")).toBe(true);
        expect(h.pushed.some((n) => n.title === "Shutting down 3 agents…" && n.message === "1 of 3 done")).toBe(true);
        const failed = h.shown.get(id);
        expect(failed.title).toBe("Couldn't shut down Bob");
        expect(failed.message).toBe("Bob: busy. Ann, Cy were shut down.");

        h.shutdownAgent.mockClear().mockResolvedValue(quit("Bob"));
        await click(id, "Retry");
        expect(h.shutdownAgent.mock.calls.map((c) => c[0])).toEqual(["b-b"]);
        expect(h.shown.get(id).title).toBe("Bob was shut down");
    });

    it("says so instead of failing when the agent is already stopped or was opened again", async () => {
        parkedRow("q", "Quen", "stopped");
        noteKeepsRunning([agent("q", "Quen")]);
        await click("keeps-running:q", "Shutdown");
        expect(h.shown.get("keeps-running:q").title).toBe("Quen is already stopped.");
        expect(h.shutdownAgent).not.toHaveBeenCalled();

        h.pushed.length = 0;
        h.rows.length = 0;
        parkedRow("r", "Rae");
        h.blockMeta["b-r"] = {};
        noteKeepsRunning([agent("r", "Rae")]);
        await click("keeps-running:r", "Shutdown");
        expect(h.shown.get("keeps-running:r").title).toBe("Rae was opened again; shut it down from its pane.");
        expect(h.shutdownAgent).not.toHaveBeenCalled();
    });

    it("shuts down the ones still parked and says so about the one that is not", async () => {
        parkedRow("a", "Ann");
        parkedRow("b", "Bob", "stopped");
        h.shutdownAgent.mockResolvedValue(quit("Ann"));
        noteKeepsRunning([agent("a", "Ann"), agent("b", "Bob")]);
        await click("keeps-running:a,b", "Shutdown");
        expect(h.shutdownAgent.mock.calls.map((c) => c[0])).toEqual(["b-a"]);
        const done = h.shown.get("keeps-running:a,b");
        expect(done.title).toBe("Ann was shut down");
        expect(done.message).toContain("Bob is already stopped.");
    });
});
