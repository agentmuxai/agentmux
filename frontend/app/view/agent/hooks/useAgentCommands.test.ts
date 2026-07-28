// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Pins the fix for a real, user-reported bug: the pane showed "Working…"
 * forever whenever `AgentInputCommand` itself failed synchronously (no
 * controller registered for the block — e.g. right after a backend
 * restart, before the pane is reopened; the identity spawn gate blocking
 * on a bad credential; any network-level rejection) — completely
 * independent of whatever happens deeper in the agent's own turn
 * lifecycle. `handleSendMessage` (agent-view.tsx) dispatches `TurnStart`
 * OPTIMISTICALLY before this RPC call ever runs; the catch block used to
 * only remove the pending "ghost" row (`PendingMessageRejected`) and never
 * reverted `turnPhase`, so it stayed stuck at Submitting/Streaming with no
 * path back to Idle. See
 * REPORT_LOGIN_PERSIST_FAILURE_AND_STUCK_WORKING_2026_07_27.md.
 */

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createRoot } from "solid-js";

const hub = vi.hoisted(() => ({
    agentInput: vi.fn(),
}));

vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: {
        AgentInputCommand: (...args: unknown[]) => hub.agentInput(...args),
        SetMetaCommand: vi.fn().mockResolvedValue(undefined),
    },
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));

import { useAgentCommands } from "./useAgentCommands";
import {
    registerPane,
    unregisterPane,
    type PaneRegistration,
} from "@/app/store/agent-pane-registration";
import { snapshot as paneSnapshot, __resetAllSlots as resetPaneStateSlots } from "@/app/store/agent-pane-state-store";
import { __resetAllSlots as resetDocSlots } from "@/app/store/agent-document-store";
import type { AgentPaneProjections } from "@/app/store/agent-pane-state-store";

function noopProjections(): AgentPaneProjections {
    return {
        streaming: () => {},
        sessionStats: () => {},
        sessionTotals: () => {},
        currentTool: () => {},
        turnTokens: () => {},
        pending: () => {},
        initPhase: () => {},
        turnPhase: () => {},
    };
}

function fullRegistration(): PaneRegistration {
    return {
        agentId: "agent-1",
        documentSetter: () => {},
        projections: noopProjections(),
    };
}

const BLOCK_ID = "block-send-fail";

beforeEach(() => {
    hub.agentInput.mockReset();
});
afterEach(() => {
    unregisterPane(BLOCK_ID);
    resetDocSlots();
    resetPaneStateSlots();
});

describe("useAgentCommands — turnPhase recovery on a failed send", () => {
    it("resets turnPhase to Idle when the idle-send AgentInputCommand rejects", async () => {
        hub.agentInput.mockRejectedValue(new Error("no controller for block"));
        const model = registerPane(BLOCK_ID, fullRegistration());
        // Move past InitPending — a fresh pane blocks sendMessage entirely
        // until its history fetch resolves. TurnStart also requires a
        // subscribed stream (state.lastEventMs != null) — mirrors what a
        // real mount does before the user can ever send.
        model.dispatchPane({ type: "InitReady", at: Date.now() }, "system");
        model.dispatchPane({ type: "StreamSubscribe", at: Date.now() }, "system");

        await createRoot(async (dispose) => {
            const commands = useAgentCommands({
                blockId: BLOCK_ID,
                model,
                block: () => undefined,
                provider: () => undefined,
                documentAtom: [() => [], () => {}] as any,
                log: () => {},
                setAuthUrl: () => {},
                canRetry: () => false,
                loginWaiting: () => false,
                setAuthNotice: () => {},
                notifyControllerHealthy: () => {},
                backToPicker: async () => {},
            });

            // Mirrors handleSendMessage's optimistic TurnStart before calling
            // commands.sendMessage — see agent-view.tsx.
            model.dispatchPane({ type: "TurnStart", at: Date.now() }, "user");
            expect(paneSnapshot(BLOCK_ID)?.turnPhase.kind).not.toBe("Idle");

            await commands.sendMessage("u there", false);

            expect(paneSnapshot(BLOCK_ID)?.turnPhase.kind).toBe("Idle");
            expect(paneSnapshot(BLOCK_ID)?.pending).toEqual([]);
            dispose();
        });
    });

    it("does NOT reset turnPhase when a held (queued-while-busy) message's flush rejects — a real turn is already in flight", async () => {
        const model = registerPane(BLOCK_ID, fullRegistration());
        model.dispatchPane({ type: "InitReady", at: Date.now() }, "system");
        model.dispatchPane({ type: "StreamSubscribe", at: Date.now() }, "system");

        await createRoot(async (dispose) => {
            const commands = useAgentCommands({
                blockId: BLOCK_ID,
                model,
                block: () => undefined,
                provider: () => undefined,
                documentAtom: [() => [], () => {}] as any,
                log: () => {},
                setAuthUrl: () => {},
                canRetry: () => false,
                loginWaiting: () => false,
                setAuthNotice: () => {},
                notifyControllerHealthy: () => {},
                backToPicker: async () => {},
            });

            // A real turn is genuinely active (e.g. the agent is streaming
            // the first message) — queue a second message behind it.
            hub.agentInput.mockResolvedValueOnce(undefined);
            model.dispatchPane({ type: "TurnStart", at: Date.now() }, "user");
            await commands.sendMessage("first message", false);
            await commands.sendMessage("held message", true);
            expect(commands.hasHeldMessages()).toBe(true);

            const busyPhase = paneSnapshot(BLOCK_ID)?.turnPhase.kind;
            expect(busyPhase).not.toBe("Idle");

            // The flush's own delivery fails — the ALREADY-active turn above
            // must not be cut short by this unrelated failure.
            hub.agentInput.mockRejectedValueOnce(new Error("transient send failure"));
            await commands.flushHeldMessages();

            expect(paneSnapshot(BLOCK_ID)?.turnPhase.kind).toBe(busyPhase);
            dispose();
        });
    });
});

describe("useAgentCommands — fast-fail when the pane is already known-unauthenticated (retro-send-while-unauthenticated-2026-07-28.md)", () => {
    it("never calls AgentInputCommand when canRetry() is true, and reverts turnPhase immediately", async () => {
        const model = registerPane(BLOCK_ID, fullRegistration());
        model.dispatchPane({ type: "InitReady", at: Date.now() }, "system");
        model.dispatchPane({ type: "StreamSubscribe", at: Date.now() }, "system");
        const setAuthNotice = vi.fn();

        await createRoot(async (dispose) => {
            const commands = useAgentCommands({
                blockId: BLOCK_ID,
                model,
                block: () => undefined,
                provider: () => undefined,
                documentAtom: [() => [], () => {}] as any,
                log: () => {},
                setAuthUrl: () => {},
                // The pane is already showing the mount-time "Log in" bar —
                // this is the exact known-bad state this fix targets.
                canRetry: () => true,
                loginWaiting: () => false,
                setAuthNotice,
                notifyControllerHealthy: () => {},
                backToPicker: async () => {},
            });

            // Mirrors handleSendMessage's optimistic TurnStart before calling
            // commands.sendMessage — see agent-view.tsx.
            model.dispatchPane({ type: "TurnStart", at: Date.now() }, "user");
            expect(paneSnapshot(BLOCK_ID)?.turnPhase.kind).not.toBe("Idle");

            await commands.sendMessage("u there", false);

            // The whole point: no CLI spawn was ever attempted for a pane the
            // UI already knows is logged out.
            expect(hub.agentInput).not.toHaveBeenCalled();
            expect(paneSnapshot(BLOCK_ID)?.turnPhase.kind).toBe("Idle");
            expect(paneSnapshot(BLOCK_ID)?.pending).toEqual([]);
            expect(setAuthNotice).toHaveBeenCalledWith(expect.stringContaining("logged in"));
            dispose();
        });
    });

    it("does not fast-fail a held message flush when canRetry() only became true after the turn was already running", async () => {
        const model = registerPane(BLOCK_ID, fullRegistration());
        model.dispatchPane({ type: "InitReady", at: Date.now() }, "system");
        model.dispatchPane({ type: "StreamSubscribe", at: Date.now() }, "system");
        let retry = false;

        await createRoot(async (dispose) => {
            const commands = useAgentCommands({
                blockId: BLOCK_ID,
                model,
                block: () => undefined,
                provider: () => undefined,
                documentAtom: [() => [], () => {}] as any,
                log: () => {},
                setAuthUrl: () => {},
                canRetry: () => retry,
                loginWaiting: () => false,
                setAuthNotice: () => {},
                notifyControllerHealthy: () => {},
                backToPicker: async () => {},
            });

            // A real turn starts while still authenticated.
            hub.agentInput.mockResolvedValueOnce(undefined);
            model.dispatchPane({ type: "TurnStart", at: Date.now() }, "user");
            await commands.sendMessage("first message", false);
            await commands.sendMessage("held message", true);
            expect(commands.hasHeldMessages()).toBe(true);
            const busyPhase = paneSnapshot(BLOCK_ID)?.turnPhase.kind;

            // Auth expires mid-turn (e.g. a 401 flips canRetry — unrelated to
            // this specific held flush, which is for the SAME already-running
            // turn). initiatesTurn is false for a flush, so even if this path
            // were hit it must not cut the active turn short.
            retry = true;
            hub.agentInput.mockClear();
            hub.agentInput.mockResolvedValueOnce(undefined);
            await commands.flushHeldMessages();

            // The held message must actually be attempted, not silently
            // dropped by the fast-fail guard — a prior version of this guard
            // applied unconditionally and ejected the queued message here
            // without ever calling AgentInputCommand (Codex P1 on PR #2338).
            expect(hub.agentInput).toHaveBeenCalledWith(
                expect.anything(),
                expect.objectContaining({ message: "held message" }),
            );
            expect(paneSnapshot(BLOCK_ID)?.turnPhase.kind).toBe(busyPhase);
            dispose();
        });
    });

    it("rejects a held message that was already known-bad at queue time, without touching the active turn", async () => {
        // deliverToBackend's guard is deliberately gated on initiatesTurn
        // (always false for a flush), so it never sees a flushed item at
        // all — nothing else checks whether THIS specific message was
        // queued while the pane already knew it was logged out. A
        // controller reporting an active turn (wasAlreadyWorking=true)
        // while canRetry()/loginWaiting() is ALSO true is a real, reachable
        // combination (these are tracked by independent signals with no
        // invariant enforcing "active turn implies good auth") — e.g. a
        // reopened pane whose backend turn is genuinely still streaming
        // while its OWN mount-time auth check independently reports
        // auth_failed. Codex P2 on PR #2338 (sixth re-review).
        const model = registerPane(BLOCK_ID, fullRegistration());
        model.dispatchPane({ type: "InitReady", at: Date.now() }, "system");
        model.dispatchPane({ type: "StreamSubscribe", at: Date.now() }, "system");

        await createRoot(async (dispose) => {
            const commands = useAgentCommands({
                blockId: BLOCK_ID,
                model,
                block: () => undefined,
                provider: () => undefined,
                documentAtom: [() => [], () => {}] as any,
                log: () => {},
                setAuthUrl: () => {},
                canRetry: () => true,
                loginWaiting: () => false,
                setAuthNotice: () => {},
                notifyControllerHealthy: () => {},
                backToPicker: async () => {},
            });

            // A turn is already active (independent of canRetry — see the
            // comment above), so this send takes the held/queued path.
            model.dispatchPane({ type: "TurnStart", at: Date.now() }, "user");
            await commands.sendMessage("held while known-bad", true);
            expect(commands.hasHeldMessages()).toBe(true);
            const busyPhase = paneSnapshot(BLOCK_ID)?.turnPhase.kind;

            await commands.flushHeldMessages();

            expect(hub.agentInput).not.toHaveBeenCalled();
            expect(paneSnapshot(BLOCK_ID)?.pending).toEqual([]);
            // The already-active turn must not be cut short by this
            // unrelated-to-it rejection.
            expect(paneSnapshot(BLOCK_ID)?.turnPhase.kind).toBe(busyPhase);
            dispose();
        });
    });

    it("still fast-fails a turn-initiating send while loginWaiting() is true, even though canRetry() already flipped false", async () => {
        // Mirrors relogin()'s own sequencing: canRetry is cleared the
        // INSTANT the mount-time "Log in" button is clicked — well before
        // the OAuth attempt (up to 5 minutes) actually resolves. Codex P1
        // (re-review of PR #2338): canRetry() alone left that whole window
        // unguarded.
        const model = registerPane(BLOCK_ID, fullRegistration());
        model.dispatchPane({ type: "InitReady", at: Date.now() }, "system");
        model.dispatchPane({ type: "StreamSubscribe", at: Date.now() }, "system");
        const setAuthNotice = vi.fn();

        await createRoot(async (dispose) => {
            const commands = useAgentCommands({
                blockId: BLOCK_ID,
                model,
                block: () => undefined,
                provider: () => undefined,
                documentAtom: [() => [], () => {}] as any,
                log: () => {},
                setAuthUrl: () => {},
                canRetry: () => false,
                loginWaiting: () => true,
                setAuthNotice,
                notifyControllerHealthy: () => {},
                backToPicker: async () => {},
            });

            model.dispatchPane({ type: "TurnStart", at: Date.now() }, "user");
            await commands.sendMessage("u there", false);

            expect(hub.agentInput).not.toHaveBeenCalled();
            expect(paneSnapshot(BLOCK_ID)?.turnPhase.kind).toBe("Idle");
            expect(setAuthNotice).toHaveBeenCalledWith(expect.stringContaining("Not logged in"));
            dispose();
        });
    });

    it("trustedAfterRecovery bypasses loginWaiting() specifically, for the auto-retry after a recovery flow's own confirmed success", async () => {
        // loginWaiting is a shared counter across relogin()/useGlobalLogin()/
        // loginViaTerminal() (useAgentControllerStatus.ts) — it can still
        // read true here because a DIFFERENT, unrelated recovery flow is
        // overlapping with the one whose onRecovered callback triggered
        // this exact resend. That other flow's own uncertainty has no
        // bearing on whether THIS confirmed-good credential is safe to
        // retry on. Codex P2 on PR #2338 (fifth re-review).
        const model = registerPane(BLOCK_ID, fullRegistration());
        model.dispatchPane({ type: "InitReady", at: Date.now() }, "system");
        model.dispatchPane({ type: "StreamSubscribe", at: Date.now() }, "system");

        await createRoot(async (dispose) => {
            const commands = useAgentCommands({
                blockId: BLOCK_ID,
                model,
                block: () => undefined,
                provider: () => undefined,
                documentAtom: [() => [], () => {}] as any,
                log: () => {},
                setAuthUrl: () => {},
                canRetry: () => false,
                loginWaiting: () => true,
                setAuthNotice: () => {},
                notifyControllerHealthy: () => {},
                backToPicker: async () => {},
            });

            hub.agentInput.mockResolvedValueOnce(undefined);
            model.dispatchPane({ type: "TurnStart", at: Date.now() }, "user");
            await commands.sendMessage("u there", false, null, /* trustedAfterRecovery */ true);

            expect(hub.agentInput).toHaveBeenCalledWith(
                expect.anything(),
                expect.objectContaining({ message: "u there" }),
            );
            dispose();
        });
    });

    it("fast-fails a turn-initiating send when the caller captured a live auth failure, and restores the failure banner instead of a generic notice", async () => {
        // Neither canRetry nor loginWaiting reflects a mid-turn 401/403 —
        // that's the separate failure-banner mechanism (state.failure with
        // data.code === "auth"). agent-view.tsx's handleSendMessage captures
        // this BEFORE dispatching TurnStart (which unconditionally clears
        // state.failure) and passes it through as sendMessage's third arg.
        // Codex P1 on PR #2338 (second re-review). Re-dispatching the SAME
        // failure (rather than a generic authNotice referencing a "Log in"
        // button that isn't shown here) preserves the "Login Again"/"Use
        // existing login" actions the banner already offers (Codex P1,
        // third re-review).
        const model = registerPane(BLOCK_ID, fullRegistration());
        model.dispatchPane({ type: "InitReady", at: Date.now() }, "system");
        model.dispatchPane({ type: "StreamSubscribe", at: Date.now() }, "system");
        const setAuthNotice = vi.fn();
        const authFailure: AgentFailure = { code: "auth", title: "Not logged in", detail: "401 from provider", retryable: true };

        await createRoot(async (dispose) => {
            const commands = useAgentCommands({
                blockId: BLOCK_ID,
                model,
                block: () => undefined,
                provider: () => undefined,
                documentAtom: [() => [], () => {}] as any,
                log: () => {},
                setAuthUrl: () => {},
                canRetry: () => false,
                loginWaiting: () => false,
                setAuthNotice,
                notifyControllerHealthy: () => {},
                backToPicker: async () => {},
            });

            model.dispatchPane({ type: "TurnStart", at: Date.now() }, "user");
            await commands.sendMessage("u there", false, authFailure);

            expect(hub.agentInput).not.toHaveBeenCalled();
            expect(paneSnapshot(BLOCK_ID)?.turnPhase.kind).toBe("Idle");
            expect(setAuthNotice).not.toHaveBeenCalled();
            expect(paneSnapshot(BLOCK_ID)?.failure?.data).toEqual(authFailure);
            dispose();
        });
    });

});
