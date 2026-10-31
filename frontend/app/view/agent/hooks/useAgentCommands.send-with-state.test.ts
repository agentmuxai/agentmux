// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
// @vitest-environment jsdom
// sendWithState reads what TurnStart would clear before it starts the turn; escapeOnEmptyComposer
// delivers a held message or stops the turn, never both.

import { __resetAllSlots as resetDocSlots } from "@/app/store/agent-document-store";
import { registerPane, unregisterPane, type AgentPaneModel } from "@/app/store/agent-pane-registration";
import { __resetAllSlots as resetPaneStateSlots, snapshot as paneSnapshot } from "@/app/store/agent-pane-state-store";
import type { AgentFailure } from "@/types/rpc/AgentFailure";
import { createRoot } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { raiseFailure } from "../failure/failure-writer";

const hub = vi.hoisted(() => ({
    agentInput: vi.fn(),
    shellExec: vi.fn(),
    controllerInput: vi.fn(),
}));

vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: {
        AgentInputCommand: (...args: unknown[]) => hub.agentInput(...args),
        ShellExecCommand: (...args: unknown[]) => hub.shellExec(...args),
        ControllerInputCommand: (...args: unknown[]) => hub.controllerInput(...args),
        SetMetaCommand: vi.fn().mockResolvedValue(undefined),
    },
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));

import { useAgentCommands, type UseAgentCommands } from "./useAgentCommands";

const BLOCK_ID = "block-send-with-state";
const auth = { code: "auth", title: "Not signed in", detail: "", retryable: true } as AgentFailure;

function ready(): AgentPaneModel {
    const model = registerPane(BLOCK_ID, { agentId: "agent-1" });
    model.dispatchPane({ type: "InitReady", at: Date.now() }, "system");
    model.dispatchPane({ type: "StreamSubscribe", at: Date.now() }, "system");
    return model;
}

function withCommands(model: AgentPaneModel, run: (c: UseAgentCommands) => Promise<void>): Promise<void> {
    return createRoot(async (dispose) => {
        const commands = useAgentCommands({
            blockId: BLOCK_ID,
            model,
            block: () => undefined,
            provider: () => undefined,
            documentNodes: () => [],
            log: () => {},
            setAuthUrl: () => {},
            canRetry: () => false,
            loginWaiting: () => false,
            setAuthNotice: () => {},
            notifyControllerHealthy: () => {},
            forceControllerRefresh: async () => true,
            beginRecoveryFlow: () => {},
            endRecoveryFlow: () => {},
            isCancelled: () => false,
            resetCancelled: () => {},
            isBackendTurnActive: () => false,
            isBackendTurnConfirmedIdle: () => true,
            backToPicker: async () => {},
        });
        await run(commands);
        dispose();
    });
}

beforeEach(() => {
    hub.agentInput.mockReset().mockResolvedValue(undefined);
    hub.shellExec.mockReset().mockResolvedValue({ stdout: "", stderr: "", exit_code: 0 });
    hub.controllerInput.mockReset().mockResolvedValue(undefined);
});
afterEach(() => {
    unregisterPane(BLOCK_ID);
    resetDocSlots();
    resetPaneStateSlots();
});

describe("sendWithState", () => {
    it("starts the turn of an idle agent and sends the message", async () => {
        const model = ready();
        await withCommands(model, async (c) => {
            await c.sendWithState("hello");
            expect(hub.agentInput).toHaveBeenCalledTimes(1);
            expect(paneSnapshot(BLOCK_ID)?.turnPhase.kind).not.toBe("Idle");
        });
    });

    it("holds a message sent while a turn runs, without starting another", async () => {
        const model = ready();
        model.dispatchPane({ type: "TurnStart", at: Date.now(), content: "first" }, "user");
        const before = paneSnapshot(BLOCK_ID)?.turnPhase;
        await withCommands(model, async (c) => {
            await c.sendWithState("and another thing");
            expect(c.hasHeldMessages()).toBe(true);
            expect(hub.agentInput).not.toHaveBeenCalled();
            expect(paneSnapshot(BLOCK_ID)?.turnPhase).toEqual(before);
            expect(paneSnapshot(BLOCK_ID)?.pending.at(-1)?.enqueuedWhileBusy).toBe(true);
        });
    });

    it("opens the shell drawer for a !command, so its output is seen", async () => {
        const model = ready();
        await withCommands(model, async (c) => {
            expect(paneSnapshot(BLOCK_ID)?.detailsOpen).toBe(false);
            await c.sendWithState("!echo hi");
            expect(paneSnapshot(BLOCK_ID)?.detailsOpen).toBe(true);
            expect(hub.shellExec).toHaveBeenCalledTimes(1);
            expect(hub.agentInput).not.toHaveBeenCalled();
        });
    });

    it("puts the auth failure back when the send was a local command, which TurnStart had cleared", async () => {
        const model = ready();
        raiseFailure(model, { source: "live", failure: auth, at: Date.now() });
        expect(paneSnapshot(BLOCK_ID)?.failure?.data.code).toBe("auth");
        await withCommands(model, async (c) => {
            await c.sendWithState("!echo hi");
            expect(paneSnapshot(BLOCK_ID)?.failure?.data.code).toBe("auth");
        });
    });
});

describe("escapeOnEmptyComposer", () => {
    it("delivers a held message and leaves the turn running", async () => {
        const model = ready();
        model.dispatchPane({ type: "TurnStart", at: Date.now(), content: "first" }, "user");
        await withCommands(model, async (c) => {
            await c.sendWithState("steer");
            expect(c.hasHeldMessages()).toBe(true);
            c.escapeOnEmptyComposer();
            await vi.waitFor(() => expect(hub.agentInput).toHaveBeenCalledTimes(1));
            expect(hub.controllerInput).not.toHaveBeenCalled();
            expect(c.hasHeldMessages()).toBe(false);
        });
    });

    it("stops a running turn when nothing is held", async () => {
        const model = ready();
        model.dispatchPane({ type: "TurnStart", at: Date.now(), content: "first" }, "user");
        await withCommands(model, async (c) => {
            c.escapeOnEmptyComposer();
            expect(hub.controllerInput).toHaveBeenCalledTimes(1);
            expect(hub.agentInput).not.toHaveBeenCalled();
        });
    });

    it("does nothing on an idle pane", async () => {
        const model = ready();
        await withCommands(model, async (c) => {
            c.escapeOnEmptyComposer();
            expect(hub.controllerInput).not.toHaveBeenCalled();
        });
    });
});
