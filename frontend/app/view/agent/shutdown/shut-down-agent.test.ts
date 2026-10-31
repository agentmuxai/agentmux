// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { beforeEach, describe, expect, it, vi } from "vitest";

const shutdownAgent = vi.hoisted(() => vi.fn());
const shutdownPreview = vi.hoisted(() => vi.fn());
const openModal = vi.hoisted(() => vi.fn());
const pushNotification = vi.hoisted(() => vi.fn());
const beginShutdownLog = vi.hoisted(() => vi.fn());
const endShutdownLog = vi.hoisted(() => vi.fn());

vi.mock("@/store/services", () => ({ ObjectService: { ShutdownAgent: shutdownAgent, ShutdownPreview: shutdownPreview } }));
vi.mock("@/app/store/modalmodel", () => ({ openModal }));
vi.mock("@/app/store/flash-notifications", () => ({ pushNotification }));
vi.mock("./shutdown-log", () => ({ beginShutdownLog, endShutdownLog }));
vi.mock("./shutdown-confirm", () => ({ ShutdownConfirm: () => null }));

import { needsConfirm, requestShutdown, shutdownConfirmText, type ShutdownPreview } from "./shut-down-agent";

const idle: ShutdownPreview = { working: false, claims: 0, crons_targeting: [] };
const summary = (over = {}) => ({
    status: "quit",
    agent: "Korp",
    released_claims: 0,
    stopped_shells: 0,
    crons_targeting: [],
    ...over,
});

describe("needsConfirm", () => {
    it("is false for an idle agent with nothing claimed and no cron", () => {
        expect(needsConfirm(idle)).toBe(false);
    });

    it("is true mid-turn, with a claim, or with a cron targeting the agent", () => {
        expect(needsConfirm({ ...idle, working: true })).toBe(true);
        expect(needsConfirm({ ...idle, claims: 1 })).toBe(true);
        expect(needsConfirm({ ...idle, crons_targeting: ["nightly"] })).toBe(true);
    });
});

describe("shutdownConfirmText", () => {
    it("names what ends, and says the conversation is kept", () => {
        const t = shutdownConfirmText("Korp", { working: true, claims: 2, crons_targeting: ["nightly"] });
        expect(t.title).toBe("Shut down Korp?");
        expect(t.description).toContain("middle of a turn");
        expect(t.description).toContain("2 work items it has claimed go back to the pool");
        expect(t.description).toContain("1 cron job targets it (nightly); it is kept");
        expect(t.description).toContain("The conversation is kept.");
    });
});

describe("requestShutdown", () => {
    beforeEach(() => {
        for (const m of [shutdownAgent, shutdownPreview, openModal, pushNotification, beginShutdownLog, endShutdownLog]) m.mockReset();
    });

    it("shuts an idle agent down on the spot, with its background tasks kept, and says so", async () => {
        shutdownPreview.mockResolvedValue(idle);
        shutdownAgent.mockResolvedValue(summary());
        await requestShutdown("blk-1", "Korp");
        expect(openModal).not.toHaveBeenCalled();
        expect(shutdownAgent).toHaveBeenCalledWith("blk-1", false);
        expect(beginShutdownLog).toHaveBeenCalledWith("blk-1");
        expect(pushNotification).toHaveBeenCalledWith(expect.objectContaining({ title: "Korp shut down", type: "info" }));
    });

    it("asks first when the agent is mid-turn, and shuts down only on confirm, with the box's answer", async () => {
        shutdownPreview.mockResolvedValue({ ...idle, working: true });
        await requestShutdown("blk-1", "Korp");
        expect(shutdownAgent).not.toHaveBeenCalled();
        expect(openModal).toHaveBeenCalledTimes(1);
        const props = openModal.mock.calls[0][1];
        expect(props.title).toBe("Shut down Korp?");
        shutdownAgent.mockResolvedValue(summary());
        props.onConfirm(true);
        await vi.waitFor(() => expect(shutdownAgent).toHaveBeenCalledWith("blk-1", true));
    });

    it("asks when it cannot tell what would end", async () => {
        shutdownPreview.mockRejectedValue(new Error("down"));
        await requestShutdown("blk-1", "Korp");
        expect(openModal).toHaveBeenCalledTimes(1);
        expect(shutdownAgent).not.toHaveBeenCalled();
    });

    it("reports a failure and clears the shutdown log", async () => {
        shutdownPreview.mockResolvedValue(idle);
        shutdownAgent.mockRejectedValue(new Error("QuitAgent: block not found: blk-1"));
        await requestShutdown("blk-1", "Korp");
        expect(endShutdownLog).toHaveBeenCalledWith("blk-1");
        expect(pushNotification).toHaveBeenCalledWith(
            expect.objectContaining({ title: "Couldn't shut down Korp", type: "error" })
        );
    });

    it("shows no second notice for a shutdown already under way", async () => {
        shutdownPreview.mockResolvedValue(idle);
        shutdownAgent.mockResolvedValue(summary({ status: "already_quitting" }));
        await requestShutdown("blk-1", "Korp");
        expect(pushNotification).not.toHaveBeenCalled();
    });
});
