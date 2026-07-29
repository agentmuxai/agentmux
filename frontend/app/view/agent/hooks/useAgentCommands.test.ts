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
    dispatchSlashCommand: vi.fn(),
    setMeta: vi.fn(),
}));

vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: {
        AgentInputCommand: (...args: unknown[]) => hub.agentInput(...args),
        SetMetaCommand: (...args: unknown[]) => hub.setMeta(...args),
    },
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
// Only mocked where a test needs to control the slash-command outcome
// (or simulate a command calling ctx.clearAuthFailure()) — falls through to
// the real dispatcher (a "passthrough" outcome) whenever no test has set an
// implementation, so every other test's non-slash-command sends are
// unaffected.
vi.mock("../commands/dispatch", () => ({
    dispatchSlashCommand: (...args: unknown[]) => hub.dispatchSlashCommand(...args),
}));

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
    hub.dispatchSlashCommand.mockReset().mockResolvedValue({ kind: "passthrough" });
    hub.setMeta.mockReset().mockResolvedValue(undefined);
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
                forceControllerRefresh: async () => true,
                beginRecoveryFlow: () => {},
                endRecoveryFlow: () => {},
                isBackendTurnActive: () => false,
                isBackendTurnConfirmedIdle: () => true,
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
                forceControllerRefresh: async () => true,
                beginRecoveryFlow: () => {},
                endRecoveryFlow: () => {},
                isBackendTurnActive: () => false,
                isBackendTurnConfirmedIdle: () => true,
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
                forceControllerRefresh: async () => true,
                beginRecoveryFlow: () => {},
                endRecoveryFlow: () => {},
                isBackendTurnActive: () => false,
                isBackendTurnConfirmedIdle: () => true,
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
                forceControllerRefresh: async () => true,
                beginRecoveryFlow: () => {},
                endRecoveryFlow: () => {},
                isBackendTurnActive: () => false,
                isBackendTurnConfirmedIdle: () => true,
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

    it("rejects a message that is already known-bad at queue time IMMEDIATELY, without ever entering the held queue, and without touching the active turn", async () => {
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
        //
        // Rejects IMMEDIATELY rather than queueing-then-rejecting-at-flush:
        // flushHeldMessages would unconditionally reject this exact item
        // anyway, but only once some LATER trigger (a tool-call boundary or
        // turn-end) happens to run it — a tool-less or stuck turn might
        // never fire that trigger, leaving the message stuck in the "send
        // now" panel with no feedback indefinitely. codex P2 on PR #2338
        // (twenty-fourth re-review).
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
                forceControllerRefresh: async () => true,
                beginRecoveryFlow: () => {},
                endRecoveryFlow: () => {},
                isBackendTurnActive: () => false,
                isBackendTurnConfirmedIdle: () => true,
                backToPicker: async () => {},
            });

            // A turn is already active (independent of canRetry — see the
            // comment above).
            model.dispatchPane({ type: "TurnStart", at: Date.now() }, "user");
            const busyPhase = paneSnapshot(BLOCK_ID)?.turnPhase.kind;
            await commands.sendMessage("held while known-bad", true);

            // Rejected immediately — never entered the held queue at all.
            expect(commands.hasHeldMessages()).toBe(false);
            expect(hub.agentInput).not.toHaveBeenCalled();
            expect(paneSnapshot(BLOCK_ID)?.pending).toEqual([]);
            // The already-active turn must not be cut short by this
            // unrelated-to-it rejection.
            expect(paneSnapshot(BLOCK_ID)?.turnPhase.kind).toBe(busyPhase);
            dispose();
        });
    });

    it("still rejects a held message known-bad at queue time even if a later-FAILED recovery cleared loginWaiting() without setting canRetry() (codex P1 on PR #2338, sixteenth re-review)", async () => {
        // A prior version of this code re-checked LIVE canRetry()/
        // loginWaiting() at flush time to avoid over-rejecting a message
        // that recovered — but relogin()/useGlobalLogin()/loginViaTerminal()'s
        // default retryAfterLogin:true failure path clears loginWaiting()
        // and never sets canRetry() back to true, so both signals read
        // false after a recovery attempt FAILS too, not just when it
        // succeeds. The live re-check let a still-known-bad message through
        // in exactly that case. Must always reject once flagged bad at
        // queue time — never re-derive "is it fixed now" from these two
        // signals alone.
        //
        // Exercised via the blocked-deferred-refresh hold (idle-send path),
        // not the busy-path hold: codex P2 on PR #2338 (twenty-fourth
        // re-review) made the busy path reject a known-bad-at-queue-time
        // message IMMEDIATELY rather than queueing it — that scenario can
        // no longer reach heldQueue with authWasKnownBadAtQueueTime: true
        // at all, so the frozen-flag protection this test pins is only
        // still reachable via the OTHER push site (which intentionally does
        // NOT immediate-reject, since canRetry()/loginWaiting() being true
        // there can mean the SAME in-flight recovery this hold is already
        // waiting on).
        let loginWaitingNow = true;
        let backendConfirmedIdle = false;
        const model = registerPane(BLOCK_ID, fullRegistration());
        model.dispatchPane({ type: "InitReady", at: Date.now() }, "system");
        model.dispatchPane({ type: "StreamSubscribe", at: Date.now() }, "system");
        hub.dispatchSlashCommand.mockImplementation(async (_msg: string, _registry: unknown, ctx: { deferControllerRefreshUntilIdle: () => void }) => {
            ctx.deferControllerRefreshUntilIdle();
            return { kind: "handled" };
        });

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
                loginWaiting: () => loginWaitingNow,
                setAuthNotice: () => {},
                notifyControllerHealthy: () => {},
                forceControllerRefresh: async () => true,
                beginRecoveryFlow: () => {},
                endRecoveryFlow: () => {},
                isBackendTurnActive: () => false,
                isBackendTurnConfirmedIdle: () => backendConfirmedIdle,
                backToPicker: async () => {},
            });

            // /login succeeds mid-turn (defers instead of refreshing now) —
            // an UNRELATED deferred refresh is what blocks the send below.
            model.dispatchPane({ type: "TurnStart", at: Date.now() }, "user");
            await commands.sendMessage("/login", /* wasAlreadyWorking */ true);

            // A fresh idle send is held because the deferred refresh is
            // still blocked; loginWaiting() is ALSO true right now (a
            // separate, unrelated recovery attempt), captured into
            // authWasKnownBadAtQueueTime.
            await commands.sendMessage("held during a recovery attempt", /* wasAlreadyWorking */ false);
            expect(commands.hasHeldMessages()).toBe(true);

            // The unrelated recovery attempt FAILS: loginWaiting() clears,
            // but canRetry() was never set true (default
            // retryAfterLogin:true). The backend now confirms idle, so the
            // deferred refresh this hold was actually waiting on runs (and
            // succeeds) — isolating this test to the authWasKnownBadAtQueueTime
            // frozen-flag concern, not authFailureToPreserve (covered by
            // its own dedicated test).
            loginWaitingNow = false;
            backendConfirmedIdle = true;
            await commands.flushPendingControllerRefresh();
            await new Promise((resolve) => setTimeout(resolve, 0));

            expect(hub.agentInput).not.toHaveBeenCalled();
            expect(commands.hasHeldMessages()).toBe(false);
            dispose();
        });
    });

    it("rejects a held message queued while healthy if the SAME turn later fails with a live auth error before flush (reagent P1 on PR #2338, sixteenth re-review)", async () => {
        // authWasKnownBadAtQueueTime is false here — the turn was genuinely
        // healthy when this message was queued. But FailureObserved (a
        // mid-turn 401/403) ends the turn (Done) without touching
        // canRetry/loginWaiting, and deliverToBackend's own guard never
        // runs for a flushed item (initiatesTurn is always false) — so
        // without an independent live-failure check, this held message
        // would sail straight through to AgentInputCommand on the exact
        // credential that just failed.
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
                forceControllerRefresh: async () => true,
                beginRecoveryFlow: () => {},
                endRecoveryFlow: () => {},
                isBackendTurnActive: () => false,
                isBackendTurnConfirmedIdle: () => true,
                backToPicker: async () => {},
            });

            model.dispatchPane({ type: "TurnStart", at: Date.now() }, "user");
            await commands.sendMessage("held behind a healthy turn", true);
            expect(commands.hasHeldMessages()).toBe(true);

            // The turn this message was riding along with now fails with a
            // live auth error.
            model.dispatchPane(
                { type: "FailureObserved", failure: { code: "auth", title: "Not logged in", detail: "401", retryable: true }, at: Date.now() },
                "system",
            );

            await commands.flushHeldMessages();

            expect(hub.agentInput).not.toHaveBeenCalled();
            expect(commands.hasHeldMessages()).toBe(false);
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
                forceControllerRefresh: async () => true,
                beginRecoveryFlow: () => {},
                endRecoveryFlow: () => {},
                isBackendTurnActive: () => false,
                isBackendTurnConfirmedIdle: () => true,
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

    it("blocks a turn-initiating send while loginWaiting() is true even for a just-succeeded recovery's own auto-retry (no more trustedAfterRecovery bypass)", async () => {
        // Codex P1 on PR #2338 (fourteenth re-review): an earlier version
        // bypassed loginWaiting() here via a trustedAfterRecovery flag,
        // reasoning that a DIFFERENT overlapping recovery flow's own
        // uncertainty had no bearing on THIS flow's confirmed success. That
        // missed that relogin()/useGlobalLogin()/loginViaTerminal() all
        // unconditionally force-restart the controller once THEY finish —
        // killing the very turn the bypass just let start. The flag was
        // removed; this send must block like any other while a sibling
        // recovery flow is still active, and will succeed once
        // retryLastTurn fires again from whichever flow finishes last.
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
                forceControllerRefresh: async () => true,
                beginRecoveryFlow: () => {},
                endRecoveryFlow: () => {},
                isBackendTurnActive: () => false,
                isBackendTurnConfirmedIdle: () => true,
                backToPicker: async () => {},
            });

            model.dispatchPane({ type: "TurnStart", at: Date.now() }, "user");
            await commands.sendMessage("u there", false);

            expect(hub.agentInput).not.toHaveBeenCalled();
            expect(paneSnapshot(BLOCK_ID)?.turnPhase.kind).toBe("Idle");
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
                forceControllerRefresh: async () => true,
                beginRecoveryFlow: () => {},
                endRecoveryFlow: () => {},
                isBackendTurnActive: () => false,
                isBackendTurnConfirmedIdle: () => true,
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

// Codex P1 on PR #2338 (ninth re-review): a live "auth" failure captured
// as authFailureToPreserve must survive a purely LOCAL command (bang or a
// slash command the dispatcher marks "handled") that never reaches
// deliverToBackend at all — otherwise it just vanishes (TurnStart already
// cleared state.failure to let the local command run), and the caller's
// NEXT normal send captures a null authFailureToPreserve with
// canRetry/loginWaiting both still false (mid-turn auth failures never
// touch either), so the guard lets it through to the still-known-bad
// credential.
describe("useAgentCommands — authFailureToPreserve survives a local command", () => {
    const authFailure: AgentFailure = { code: "auth", title: "Not logged in", detail: "401 from provider", retryable: true };

    const setup = () => {
        const model = registerPane(BLOCK_ID, fullRegistration());
        model.dispatchPane({ type: "InitReady", at: Date.now() }, "system");
        model.dispatchPane({ type: "StreamSubscribe", at: Date.now() }, "system");
        return model;
    };

    const makeOpts = (model: ReturnType<typeof registerPane>) => ({
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
        forceControllerRefresh: async () => true,
        beginRecoveryFlow: () => {},
        endRecoveryFlow: () => {},
        isBackendTurnActive: () => false,
        isBackendTurnConfirmedIdle: () => true,
        backToPicker: async () => {},
    });

    it("restores the failure banner after a bang command (!pwd) — bang commands never resolve auth", async () => {
        const model = setup();
        await createRoot(async (dispose) => {
            const commands = useAgentCommands(makeOpts(model));
            model.dispatchPane({ type: "TurnStart", at: Date.now() }, "user");
            await commands.sendMessage("!pwd", false, authFailure);

            expect(hub.agentInput).not.toHaveBeenCalled();
            expect(paneSnapshot(BLOCK_ID)?.failure?.data).toEqual(authFailure);
            dispose();
        });
    });

    it("restores the failure banner after an unrelated handled slash command (e.g. /help)", async () => {
        hub.dispatchSlashCommand.mockResolvedValue({ kind: "handled" });
        const model = setup();
        await createRoot(async (dispose) => {
            const commands = useAgentCommands(makeOpts(model));
            model.dispatchPane({ type: "TurnStart", at: Date.now() }, "user");
            await commands.sendMessage("/help", false, authFailure);

            expect(hub.agentInput).not.toHaveBeenCalled();
            expect(paneSnapshot(BLOCK_ID)?.failure?.data).toEqual(authFailure);
            dispose();
        });
    });

    it("does NOT restore the failure banner when the command itself resolves it (ctx.clearAuthFailure(), as /login's success path does)", async () => {
        hub.dispatchSlashCommand.mockImplementation(async (_msg: string, _registry: unknown, ctx: { clearAuthFailure: () => void }) => {
            ctx.clearAuthFailure();
            return { kind: "handled" };
        });
        const model = setup();
        await createRoot(async (dispose) => {
            const commands = useAgentCommands(makeOpts(model));
            model.dispatchPane({ type: "TurnStart", at: Date.now() }, "user");
            await commands.sendMessage("/login", false, authFailure);

            expect(hub.agentInput).not.toHaveBeenCalled();
            // Restoring the stale pre-command failure here would resurrect a
            // banner over a credential /login just fixed.
            expect(paneSnapshot(BLOCK_ID)?.failure).toBeNull();
            dispose();
        });
    });

    it("restores the failure banner when the slash-command dispatcher itself throws", async () => {
        hub.dispatchSlashCommand.mockRejectedValue(new Error("boom"));
        const model = setup();
        await createRoot(async (dispose) => {
            const commands = useAgentCommands(makeOpts(model));
            model.dispatchPane({ type: "TurnStart", at: Date.now() }, "user");
            await commands.sendMessage("/whatever", false, authFailure);

            expect(paneSnapshot(BLOCK_ID)?.failure?.data).toEqual(authFailure);
            dispose();
        });
    });

    // reagentx P1 on PR #2338 (thirty-fifth re-review): ctx.clearAuthFailure()
    // is named and documented as clearing an AUTH-specific failure, but used
    // to dispatch the same unconditional FailureCleared the reducer applies
    // regardless of code — an unrelated live failure (e.g. rate_limited)
    // that arrives independently (useAgentFailure.ts's own AgentFailure
    // subscription) while a /login attempt is still running would be
    // silently dismissed the moment /login succeeds, even though that
    // unrelated problem was never actually resolved.
    it("does NOT clear an unrelated LIVE non-auth failure when the command calls ctx.clearAuthFailure()", async () => {
        const rateLimited: AgentFailure = { code: "rate_limited", title: "Rate limited", detail: "429", retryable: true };
        hub.dispatchSlashCommand.mockImplementation(async (_msg: string, _registry: unknown, ctx: { clearAuthFailure: () => void }) => {
            // Simulates an UNRELATED failure arriving live, independently,
            // while this command (e.g. /login's OAuth poll) is still
            // running — mirrors useAgentFailure.ts's own AgentFailure
            // subscription dispatching FailureObserved for a completely
            // different reason.
            model.dispatchPane({ type: "FailureObserved", failure: rateLimited, at: Date.now() }, "system");
            ctx.clearAuthFailure();
            return { kind: "handled" };
        });
        const model = setup();
        await createRoot(async (dispose) => {
            const commands = useAgentCommands(makeOpts(model));
            model.dispatchPane({ type: "TurnStart", at: Date.now() }, "user");
            // No auth failure was showing at send time — mirrors the
            // ordinary /login-on-a-healthy-pane case.
            await commands.sendMessage("/login", false, null);

            expect(paneSnapshot(BLOCK_ID)?.failure?.data).toEqual(rateLimited);
            dispose();
        });
    });
});

// Codex P2 on PR #2338 (tenth re-review): the auth guard originally ran
// ONLY once, before the runtime-args SetMetaCommand round-trip — a real
// async gap another recovery flow (or a mid-turn auth failure) can land in
// between that check and the actual AgentInputCommand send. canRetry()
// flipping true during that window must still block the send, not just be
// caught by the (already-passed) earlier check.
describe("useAgentCommands — re-checks the live auth guard immediately before the send", () => {
    const PROVIDER = { id: "claude", controllerType: "subprocess", launchArgs: [] } as any;

    it("blocks the send when canRetry() flips true during the SetMetaCommand round-trip", async () => {
        const model = registerPane(BLOCK_ID, fullRegistration());
        model.dispatchPane({ type: "InitReady", at: Date.now() }, "system");
        model.dispatchPane({ type: "StreamSubscribe", at: Date.now() }, "system");

        let canRetryNow = false;
        // Simulates a DIFFERENT recovery flow (or a mount-time auth_failed
        // classification) landing while this send's own SetMetaCommand RPC
        // is still in flight.
        hub.setMeta.mockImplementation(async () => {
            canRetryNow = true;
        });

        await createRoot(async (dispose) => {
            const commands = useAgentCommands({
                blockId: BLOCK_ID,
                model,
                block: () => undefined,
                provider: () => PROVIDER,
                documentAtom: [() => [], () => {}] as any,
                log: () => {},
                setAuthUrl: () => {},
                canRetry: () => canRetryNow,
                loginWaiting: () => false,
                setAuthNotice: () => {},
                notifyControllerHealthy: () => {},
                forceControllerRefresh: async () => true,
                beginRecoveryFlow: () => {},
                endRecoveryFlow: () => {},
                isBackendTurnActive: () => false,
                isBackendTurnConfirmedIdle: () => true,
                backToPicker: async () => {},
            });

            model.dispatchPane({ type: "TurnStart", at: Date.now() }, "user");
            await commands.sendMessage("u there", false);

            expect(hub.setMeta).toHaveBeenCalledOnce();
            // The stale (canRetry() was false) first check must not be the
            // last word — the second, live re-check right before the send
            // must catch it.
            expect(hub.agentInput).not.toHaveBeenCalled();
            expect(paneSnapshot(BLOCK_ID)?.turnPhase.kind).toBe("Idle");
            dispose();
        });
    });

    it("still sends normally when auth stays good across the round-trip", async () => {
        const model = registerPane(BLOCK_ID, fullRegistration());
        model.dispatchPane({ type: "InitReady", at: Date.now() }, "system");
        model.dispatchPane({ type: "StreamSubscribe", at: Date.now() }, "system");
        hub.agentInput.mockResolvedValueOnce(undefined);

        await createRoot(async (dispose) => {
            const commands = useAgentCommands({
                blockId: BLOCK_ID,
                model,
                block: () => undefined,
                provider: () => PROVIDER,
                documentAtom: [() => [], () => {}] as any,
                log: () => {},
                setAuthUrl: () => {},
                canRetry: () => false,
                loginWaiting: () => false,
                setAuthNotice: () => {},
                notifyControllerHealthy: () => {},
                forceControllerRefresh: async () => true,
                beginRecoveryFlow: () => {},
                endRecoveryFlow: () => {},
                isBackendTurnActive: () => false,
                isBackendTurnConfirmedIdle: () => true,
                backToPicker: async () => {},
            });

            model.dispatchPane({ type: "TurnStart", at: Date.now() }, "user");
            await commands.sendMessage("u there", false);

            expect(hub.agentInput).toHaveBeenCalledOnce();
            dispose();
        });
    });

    it("blocks the send when a NEW live auth failure arrives during the SetMetaCommand round-trip, even with no captured authFailureToPreserve (codex P2 on PR #2338, nineteenth re-review)", async () => {
        // No auth failure existed at the pre-TurnStart capture
        // (authFailureToPreserve stays null for the whole call) — but one
        // arrives mid-flight. canRetry()/loginWaiting() are NOT updated by
        // a mid-turn 401/403 either, so without a live re-read of
        // state.failure inside checkAuthGuard, nothing catches this.
        const model = registerPane(BLOCK_ID, fullRegistration());
        model.dispatchPane({ type: "InitReady", at: Date.now() }, "system");
        model.dispatchPane({ type: "StreamSubscribe", at: Date.now() }, "system");

        hub.setMeta.mockImplementation(async () => {
            model.dispatchPane(
                { type: "FailureObserved", failure: { code: "auth", title: "Not logged in", detail: "401", retryable: true }, at: Date.now() },
                "system",
            );
        });

        await createRoot(async (dispose) => {
            const commands = useAgentCommands({
                blockId: BLOCK_ID,
                model,
                block: () => undefined,
                provider: () => PROVIDER,
                documentAtom: [() => [], () => {}] as any,
                log: () => {},
                setAuthUrl: () => {},
                canRetry: () => false,
                loginWaiting: () => false,
                setAuthNotice: () => {},
                notifyControllerHealthy: () => {},
                forceControllerRefresh: async () => true,
                beginRecoveryFlow: () => {},
                endRecoveryFlow: () => {},
                isBackendTurnActive: () => false,
                isBackendTurnConfirmedIdle: () => true,
                backToPicker: async () => {},
            });

            model.dispatchPane({ type: "TurnStart", at: Date.now() }, "user");
            await commands.sendMessage("u there", false, /* authFailureToPreserve */ null);

            expect(hub.setMeta).toHaveBeenCalledOnce();
            expect(hub.agentInput).not.toHaveBeenCalled();
            dispose();
        });
    });
});

// Codex P1 on PR #2338 (eleventh re-review): a live paneSnapshot(...).turnPhase
// read inside buildCommandContext's isTurnActive would see the OPTIMISTIC
// TurnStart that handleSendMessage (agent-view.tsx) dispatches before ever
// calling sendMessage — even for the ordinary case of typing /login on a
// genuinely idle pane. That reports isTurnActive() === true unconditionally,
// permanently defeating login.ts's active-turn check for the most common
// path (an idle pane) and reproducing the exact stale-controller bug
// forceControllerRefresh was added to /login to fix.
describe("useAgentCommands — ctx.isTurnActive() reflects the pre-TurnStart snapshot", () => {
    it("reads false for a slash command sent while the pane was genuinely idle, even though the caller already dispatched an optimistic TurnStart", async () => {
        let observedIsTurnActive: boolean | undefined;
        hub.dispatchSlashCommand.mockImplementation(async (_msg: string, _registry: unknown, ctx: { isTurnActive: () => boolean }) => {
            observedIsTurnActive = ctx.isTurnActive();
            return { kind: "handled" };
        });
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
                forceControllerRefresh: async () => true,
                beginRecoveryFlow: () => {},
                endRecoveryFlow: () => {},
                isBackendTurnActive: () => false,
                isBackendTurnConfirmedIdle: () => true,
                backToPicker: async () => {},
            });

            // Mirrors handleSendMessage exactly: capture wasAlreadyWorking
            // BEFORE dispatching the optimistic TurnStart (pane was idle),
            // then dispatch it, then call sendMessage — same order as
            // agent-view.tsx's real call site.
            const wasAlreadyWorking = false;
            model.dispatchPane({ type: "TurnStart", at: Date.now() }, "user");
            await commands.sendMessage("/login", wasAlreadyWorking);

            expect(observedIsTurnActive).toBe(false);
            dispose();
        });
    });

    it("reads true for a slash command sent while a real turn was already streaming", async () => {
        let observedIsTurnActive: boolean | undefined;
        hub.dispatchSlashCommand.mockImplementation(async (_msg: string, _registry: unknown, ctx: { isTurnActive: () => boolean }) => {
            observedIsTurnActive = ctx.isTurnActive();
            return { kind: "handled" };
        });
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
                forceControllerRefresh: async () => true,
                beginRecoveryFlow: () => {},
                endRecoveryFlow: () => {},
                isBackendTurnActive: () => false,
                isBackendTurnConfirmedIdle: () => true,
                backToPicker: async () => {},
            });

            // A turn is already genuinely in flight (wasAlreadyWorking=true) —
            // handleSendMessage does NOT re-dispatch TurnStart in this case.
            model.dispatchPane({ type: "TurnStart", at: Date.now() }, "user");
            await commands.sendMessage("/login", /* wasAlreadyWorking */ true);

            expect(observedIsTurnActive).toBe(true);
            dispose();
        });
    });

    it("reads the LIVE (now-idle) state, not the frozen initial true, if the turn ends before the command checks isTurnActive() again", async () => {
        // Codex P1 on PR #2338 (fourteenth re-review): /login's own OAuth
        // poll can run for up to 5 minutes. If the turn that was active at
        // submission time ends DURING that wait, a frozen `true` would keep
        // reporting active long after the only edge that flushes a
        // deferred refresh (turn-just-ended) has already passed — stranding
        // the refresh forever.
        let firstCheck: boolean | undefined;
        let secondCheck: boolean | undefined;
        const model = registerPane(BLOCK_ID, fullRegistration());
        model.dispatchPane({ type: "InitReady", at: Date.now() }, "system");
        model.dispatchPane({ type: "StreamSubscribe", at: Date.now() }, "system");
        hub.dispatchSlashCommand.mockImplementation(async (_msg: string, _registry: unknown, ctx: { isTurnActive: () => boolean }) => {
            firstCheck = ctx.isTurnActive();
            // Simulates the ORIGINAL turn ending while /login's own poll is
            // still running — an independent event, unrelated to /login's
            // own (never re-dispatched, since wasAlreadyWorking was true)
            // TurnStart.
            model.dispatchPane({ type: "ReconcileTurnActive", at: Date.now(), active: false }, "system");
            secondCheck = ctx.isTurnActive();
            return { kind: "handled" };
        });

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
                forceControllerRefresh: async () => true,
                beginRecoveryFlow: () => {},
                endRecoveryFlow: () => {},
                isBackendTurnActive: () => false,
                isBackendTurnConfirmedIdle: () => true,
                backToPicker: async () => {},
            });

            model.dispatchPane({ type: "TurnStart", at: Date.now() }, "user");
            // Promote Submitting -> Streaming so the later ReconcileTurnActive
            // demotion (which only acts on a Streaming phase) has an effect.
            model.dispatchPane({ type: "StreamFlushObserved", addedCount: 1, at: Date.now() }, "system");
            await commands.sendMessage("/login", /* wasAlreadyWorking */ true);

            expect(firstCheck).toBe(true);
            expect(secondCheck).toBe(false);
            dispose();
        });
    });

    // reagent P1 on PR #2338 (twenty-eighth re-review): a premature
    // per-round session_end can demote LOCAL turnPhase to idle while the
    // backend has NEVER confirmed idle (isBackendTurnConfirmedIdle stays
    // false — the same "never confirmed either way" state as a pane that
    // mounts mid-turn before its first live event). The live-read branch
    // must not trust that local "idle" read as safe unless the backend has
    // POSITIVELY confirmed it — otherwise finalizeLoginSuccess
    // (login.ts) calls its immediate forceControllerRefresh path and kills
    // a turn that is, in reality, still genuinely active.
    it("still reads true when local turnPhase demotes to idle but the backend has NEVER confirmed idle either way", async () => {
        let observedIsTurnActive: boolean | undefined;
        hub.dispatchSlashCommand.mockImplementation(async (_msg: string, _registry: unknown, ctx: { isTurnActive: () => boolean }) => {
            // Simulates a premature session_end demoting local turnPhase to
            // idle DURING /login's own async work, before the backend has
            // ever confirmed idle.
            model.dispatchPane({ type: "ReconcileTurnActive", at: Date.now(), active: false }, "system");
            observedIsTurnActive = ctx.isTurnActive();
            return { kind: "handled" };
        });
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
                forceControllerRefresh: async () => true,
                beginRecoveryFlow: () => {},
                endRecoveryFlow: () => {},
                // Never confirmed active NOR confirmed idle — the "never
                // confirmed either way" state.
                isBackendTurnActive: () => false,
                isBackendTurnConfirmedIdle: () => false,
                backToPicker: async () => {},
            });

            // A turn is already genuinely in flight — wasAlreadyWorking=true,
            // so the live-read branch is exercised (not the frozen false one).
            model.dispatchPane({ type: "TurnStart", at: Date.now() }, "user");
            model.dispatchPane({ type: "StreamFlushObserved", addedCount: 1, at: Date.now() }, "system");
            await commands.sendMessage("/login", /* wasAlreadyWorking */ true);

            expect(observedIsTurnActive).toBe(true);
            dispose();
        });
    });
});

// Codex P1 on PR #2338 (thirteenth re-review): /login deferring its
// controller restart while a turn is active (ctx.deferControllerRefreshUntilIdle)
// is only safe if something actually runs that deferred restart once the
// turn ends — otherwise a persistent controller (which stays alive across
// MANY turns) is stranded on the stale credential forever, indistinguishable
// from the bug this whole mechanism exists to fix.
describe("useAgentCommands — flushPendingControllerRefresh", () => {
    const setup = () => {
        const model = registerPane(BLOCK_ID, fullRegistration());
        model.dispatchPane({ type: "InitReady", at: Date.now() }, "system");
        model.dispatchPane({ type: "StreamSubscribe", at: Date.now() }, "system");
        return model;
    };

    it("is a no-op when nothing was deferred", async () => {
        const model = setup();
        const forceControllerRefresh = vi.fn(async () => true);
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
                forceControllerRefresh,
                beginRecoveryFlow: () => {},
                endRecoveryFlow: () => {},
                isBackendTurnActive: () => false,
                isBackendTurnConfirmedIdle: () => true,
                backToPicker: async () => {},
            });

            await commands.flushPendingControllerRefresh();

            expect(forceControllerRefresh).not.toHaveBeenCalled();
            dispose();
        });
    });

    it("runs the deferred refresh exactly once, and clears the fast-fail guards only on success", async () => {
        hub.dispatchSlashCommand.mockImplementation(async (_msg: string, _registry: unknown, ctx: { deferControllerRefreshUntilIdle: () => void }) => {
            ctx.deferControllerRefreshUntilIdle();
            return { kind: "handled" };
        });
        const model = setup();
        const forceControllerRefresh = vi.fn(async () => true);
        const notifyControllerHealthy = vi.fn();

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
                notifyControllerHealthy,
                forceControllerRefresh,
                beginRecoveryFlow: () => {},
                endRecoveryFlow: () => {},
                isBackendTurnActive: () => false,
                isBackendTurnConfirmedIdle: () => true,
                backToPicker: async () => {},
            });

            // Simulates /login succeeding mid-turn: it defers instead of
            // refreshing immediately.
            model.dispatchPane({ type: "TurnStart", at: Date.now() }, "user");
            await commands.sendMessage("/login", /* wasAlreadyWorking */ true);
            expect(forceControllerRefresh).not.toHaveBeenCalled();

            // The turn ending is what should trigger the deferred refresh.
            await commands.flushPendingControllerRefresh();
            expect(forceControllerRefresh).toHaveBeenCalledOnce();
            expect(notifyControllerHealthy).toHaveBeenCalledOnce();
            expect(paneSnapshot(BLOCK_ID)?.failure).toBeNull();

            // A second turn-end (or any later call) must not re-run it —
            // already consumed.
            await commands.flushPendingControllerRefresh();
            expect(forceControllerRefresh).toHaveBeenCalledOnce();

            dispose();
        });
    });

    // reagentx P1 on PR #2338 (thirty-fifth re-review): the success branch
    // dispatched an unconditional FailureCleared, which the reducer applies
    // regardless of data.code — so a deferred /login refresh succeeding
    // would silently wipe an UNRELATED live failure (e.g. rate_limited)
    // that arrived on this pane while the refresh was still deferred,
    // even though that unrelated problem was never actually resolved.
    it("does NOT clear an unrelated LIVE non-auth failure when the deferred refresh succeeds", async () => {
        hub.dispatchSlashCommand.mockImplementation(async (_msg: string, _registry: unknown, ctx: { deferControllerRefreshUntilIdle: () => void }) => {
            ctx.deferControllerRefreshUntilIdle();
            return { kind: "handled" };
        });
        const model = setup();
        const forceControllerRefresh = vi.fn(async () => true);
        const notifyControllerHealthy = vi.fn();
        const rateLimited: AgentFailure = { code: "rate_limited", title: "Rate limited", detail: "429", retryable: true };

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
                notifyControllerHealthy,
                forceControllerRefresh,
                beginRecoveryFlow: () => {},
                endRecoveryFlow: () => {},
                isBackendTurnActive: () => false,
                isBackendTurnConfirmedIdle: () => true,
                backToPicker: async () => {},
            });

            model.dispatchPane({ type: "TurnStart", at: Date.now() }, "user");
            await commands.sendMessage("/login", /* wasAlreadyWorking */ true);

            // An UNRELATED failure arrives live while the refresh is still
            // deferred — mirrors useAgentFailure.ts's own AgentFailure
            // subscription firing independently of this /login attempt.
            model.dispatchPane({ type: "FailureObserved", failure: rateLimited, at: Date.now() }, "system");

            await commands.flushPendingControllerRefresh();
            expect(forceControllerRefresh).toHaveBeenCalledOnce();
            expect(notifyControllerHealthy).toHaveBeenCalledOnce();
            expect(paneSnapshot(BLOCK_ID)?.failure?.data).toEqual(rateLimited);
            dispose();
        });
    });

    it("does NOT clear the fast-fail guards when the deferred refresh itself fails", async () => {
        hub.dispatchSlashCommand.mockImplementation(async (_msg: string, _registry: unknown, ctx: { deferControllerRefreshUntilIdle: () => void }) => {
            ctx.deferControllerRefreshUntilIdle();
            return { kind: "handled" };
        });
        const model = setup();
        const forceControllerRefresh = vi.fn(async () => false);
        const notifyControllerHealthy = vi.fn();

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
                notifyControllerHealthy,
                forceControllerRefresh,
                beginRecoveryFlow: () => {},
                endRecoveryFlow: () => {},
                isBackendTurnActive: () => false,
                isBackendTurnConfirmedIdle: () => true,
                backToPicker: async () => {},
            });

            model.dispatchPane({ type: "TurnStart", at: Date.now() }, "user");
            await commands.sendMessage("/login", /* wasAlreadyWorking */ true);
            await commands.flushPendingControllerRefresh();

            expect(forceControllerRefresh).toHaveBeenCalledOnce();
            expect(notifyControllerHealthy).not.toHaveBeenCalled();

            // reagent P1 on PR #2338 (twenty-sixth re-review): asserting
            // notifyControllerHealthy wasn't called is not enough —
            // forceControllerRefresh's own failure path never sets
            // canRetry()/loginWaiting()/state.failure either, so nothing
            // else was left blocking unless flushPendingControllerRefresh
            // itself re-arms controllerRefreshPendingUntilIdle on failure.
            // A subsequent fresh idle send must still be BLOCKED (held),
            // not delivered to the still-stale controller.
            await commands.sendMessage("fresh after failed refresh", /* wasAlreadyWorking */ false);
            expect(hub.agentInput).not.toHaveBeenCalled();
            expect(commands.hasHeldMessages()).toBe(true);
            // The retry this send triggered also failed (mock always
            // returns false) — called a second time.
            expect(forceControllerRefresh).toHaveBeenCalledTimes(2);
            dispose();
        });
    });

    // codex P2 on PR #2338 (twenty-seventh re-review): re-arming
    // controllerRefreshPendingUntilIdle on failure isn't enough on its
    // own — nothing re-checks a plain closure variable just because it
    // changed. The turn-just-ended edge and the reactive turnIdle effect
    // that triggered THIS attempt have already fired; without a scheduled
    // retry, a held message would sit indefinitely unless the user
    // happens to send another message or a new turn starts.
    it("automatically retries a failed deferred refresh after a delay, with no external trigger", async () => {
        hub.dispatchSlashCommand.mockImplementation(async (_msg: string, _registry: unknown, ctx: { deferControllerRefreshUntilIdle: () => void }) => {
            ctx.deferControllerRefreshUntilIdle();
            return { kind: "handled" };
        });
        const model = registerPane(BLOCK_ID, fullRegistration());
        model.dispatchPane({ type: "InitReady", at: Date.now() }, "system");
        model.dispatchPane({ type: "StreamSubscribe", at: Date.now() }, "system");
        // Fails on the first attempt, succeeds on the automatic retry.
        let attempt = 0;
        const forceControllerRefresh = vi.fn(async () => {
            attempt += 1;
            return attempt >= 2;
        });
        const notifyControllerHealthy = vi.fn();

        vi.useFakeTimers();
        try {
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
                    notifyControllerHealthy,
                    forceControllerRefresh,
                    beginRecoveryFlow: () => {},
                    endRecoveryFlow: () => {},
                    isBackendTurnActive: () => false,
                    isBackendTurnConfirmedIdle: () => true,
                    backToPicker: async () => {},
                });

                model.dispatchPane({ type: "TurnStart", at: Date.now() }, "user");
                await commands.sendMessage("/login", /* wasAlreadyWorking */ true);
                await commands.flushPendingControllerRefresh();

                expect(forceControllerRefresh).toHaveBeenCalledOnce();
                expect(notifyControllerHealthy).not.toHaveBeenCalled();

                // No external trigger — just time passing.
                await vi.advanceTimersByTimeAsync(5000);

                expect(forceControllerRefresh).toHaveBeenCalledTimes(2);
                expect(notifyControllerHealthy).toHaveBeenCalledOnce();
                dispose();
            });
        } finally {
            vi.useRealTimers();
        }
    });

    // codex P2 on PR #2338 (thirty-fourth re-review): onCleanup only runs
    // ONCE, at dispose time. If a forceControllerRefresh() RPC is still in
    // flight when the pane disposes, no retry timer exists yet for
    // onCleanup to clear. Without isDisposed, a failure resolving AFTER
    // disposal would schedule a brand-new timer with nothing left to ever
    // clean it up.
    it("does not schedule a retry if the pane disposes while the deferred refresh's own RPC is still in flight", async () => {
        hub.dispatchSlashCommand.mockImplementation(async (_msg: string, _registry: unknown, ctx: { deferControllerRefreshUntilIdle: () => void }) => {
            ctx.deferControllerRefreshUntilIdle();
            return { kind: "handled" };
        });
        const model = registerPane(BLOCK_ID, fullRegistration());
        model.dispatchPane({ type: "InitReady", at: Date.now() }, "system");
        model.dispatchPane({ type: "StreamSubscribe", at: Date.now() }, "system");

        let resolveRefresh: ((v: boolean) => void) | undefined;
        const forceControllerRefresh = vi.fn(
            () => new Promise<boolean>((resolve) => { resolveRefresh = resolve; }),
        );

        vi.useFakeTimers();
        try {
            let disposeFn: (() => void) | undefined;
            let flushPromise: Promise<boolean> | undefined;
            await createRoot(async (dispose) => {
                disposeFn = dispose;
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
                    forceControllerRefresh,
                    beginRecoveryFlow: () => {},
                    endRecoveryFlow: () => {},
                    isBackendTurnActive: () => false,
                    isBackendTurnConfirmedIdle: () => true,
                    backToPicker: async () => {},
                });

                model.dispatchPane({ type: "TurnStart", at: Date.now() }, "user");
                await commands.sendMessage("/login", /* wasAlreadyWorking */ true);

                // Kick off the flush — its own RPC is still unresolved.
                flushPromise = commands.flushPendingControllerRefresh();
            });

            // Dispose WHILE the RPC is still in flight — onCleanup runs now,
            // with no retry timer yet to clear.
            disposeFn?.();

            // The RPC now resolves as a FAILURE, after disposal.
            resolveRefresh?.(false);
            await flushPromise;

            // No retry may be scheduled — advancing well past the retry
            // delay must not trigger a second forceControllerRefresh call.
            await vi.advanceTimersByTimeAsync(10000);
            expect(forceControllerRefresh).toHaveBeenCalledOnce();
        } finally {
            vi.useRealTimers();
        }
    });
});

// Codex P1 on PR #2338 (fifteenth re-review): /login succeeding mid-turn
// (deferring its controller restart) and flushHeldMessages draining the
// "send now" queue are triggered by the SAME turn-just-ended moment via
// TWO independent signals (a live controllerstatus event calling
// flushPendingControllerRefresh vs. a reactive turnPhaseAtom effect calling
// flushHeldMessages) — either can fire first, and neither originally
// awaited the other. A held message could reach AgentInputCommand while
// ControllerResyncCommand was still stopping/replacing the controller.
describe("useAgentCommands — flushHeldMessages serializes behind a pending/in-flight controller refresh", () => {
    it("does not send a held message until the deferred controller refresh (triggered by a DIFFERENT, concurrent caller) actually completes", async () => {
        hub.dispatchSlashCommand.mockImplementation(async (_msg: string, _registry: unknown, ctx: { deferControllerRefreshUntilIdle: () => void }) => {
            ctx.deferControllerRefreshUntilIdle();
            return { kind: "handled" };
        });
        const model = registerPane(BLOCK_ID, fullRegistration());
        model.dispatchPane({ type: "InitReady", at: Date.now() }, "system");
        model.dispatchPane({ type: "StreamSubscribe", at: Date.now() }, "system");

        let resolveRefresh: (() => void) | undefined;
        const forceControllerRefresh = vi.fn(
            () => new Promise<boolean>((resolve) => { resolveRefresh = () => resolve(true); }),
        );

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
                forceControllerRefresh,
                beginRecoveryFlow: () => {},
                endRecoveryFlow: () => {},
                isBackendTurnActive: () => false,
                isBackendTurnConfirmedIdle: () => true,
                backToPicker: async () => {},
            });

            // /login succeeds mid-turn (defers instead of refreshing now).
            model.dispatchPane({ type: "TurnStart", at: Date.now() }, "user");
            await commands.sendMessage("/login", /* wasAlreadyWorking */ true);

            // A second message got held behind the same active turn — its
            // authWasKnownBadAtQueueTime is false (auth was fine when it
            // was queued), so deliverToBackend's guard is bypassed for it
            // (initiatesTurn=false) once flushed.
            await commands.sendMessage("hi", /* wasAlreadyWorking */ true);
            expect(commands.hasHeldMessages()).toBe(true);

            // Simulates the turn ending: turnPhase actually transitions to
            // idle (flushHeldMessages' own internal refresh-await is gated
            // on this being live-idle — see the "does NOT run... mid-turn"
            // test below), the live controllerstatus path fires
            // flushPendingControllerRefresh WITHOUT awaiting it (mirrors
            // agent-view.tsx's trackTurnJustEnded), and the reactive
            // turnPhaseAtom effect independently calls flushHeldMessages
            // around the same moment.
            model.dispatchPane({ type: "StreamFlushObserved", addedCount: 1, at: Date.now() }, "system");
            model.dispatchPane({ type: "ReconcileTurnActive", at: Date.now(), active: false }, "system");
            const refreshPromise = commands.flushPendingControllerRefresh();
            const flushPromise = commands.flushHeldMessages();

            // The refresh RPC hasn't resolved yet — the held message must
            // not have been sent.
            expect(forceControllerRefresh).toHaveBeenCalledOnce();
            expect(hub.agentInput).not.toHaveBeenCalled();

            resolveRefresh?.();
            await refreshPromise;
            await flushPromise;

            expect(hub.agentInput).toHaveBeenCalledWith(
                expect.anything(),
                expect.objectContaining({ message: "hi" }),
            );
            dispose();
        });
    });

    it("does NOT run the deferred refresh when flushHeldMessages is triggered mid-turn (e.g. a new-tool-call boundary or Esc-to-steer) — only once the pane is actually idle", async () => {
        // Codex P1 on PR #2338 (seventeenth re-review): flushHeldMessages
        // also runs at a mid-turn tool-call boundary (agent-view.tsx's
        // reactive effect fires on newToolCall || turnIdle, not turnIdle
        // alone) and from the Esc-to-steer handler — both while a turn can
        // still be genuinely active. Running the deferred refresh
        // unconditionally there would force-restart the controller
        // mid-turn, exactly what deferring it was meant to prevent.
        hub.dispatchSlashCommand.mockImplementation(async (_msg: string, _registry: unknown, ctx: { deferControllerRefreshUntilIdle: () => void }) => {
            ctx.deferControllerRefreshUntilIdle();
            return { kind: "handled" };
        });
        const model = registerPane(BLOCK_ID, fullRegistration());
        model.dispatchPane({ type: "InitReady", at: Date.now() }, "system");
        model.dispatchPane({ type: "StreamSubscribe", at: Date.now() }, "system");
        const forceControllerRefresh = vi.fn(async () => true);

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
                forceControllerRefresh,
                beginRecoveryFlow: () => {},
                endRecoveryFlow: () => {},
                isBackendTurnActive: () => false,
                isBackendTurnConfirmedIdle: () => true,
                backToPicker: async () => {},
            });

            // /login succeeds mid-turn (defers instead of refreshing now).
            model.dispatchPane({ type: "TurnStart", at: Date.now() }, "user");
            await commands.sendMessage("/login", /* wasAlreadyWorking */ true);

            // A second message is held behind the same active turn.
            await commands.sendMessage("hi", /* wasAlreadyWorking */ true);
            expect(commands.hasHeldMessages()).toBe(true);

            // Promote to Streaming — the pane is genuinely still working —
            // then flush as if triggered by a mid-turn tool-call boundary.
            model.dispatchPane({ type: "StreamFlushObserved", addedCount: 1, at: Date.now() }, "system");
            await commands.flushHeldMessages();

            expect(forceControllerRefresh).not.toHaveBeenCalled();
            expect(hub.agentInput).toHaveBeenCalledWith(
                expect.anything(),
                expect.objectContaining({ message: "hi" }),
            );

            // Now the turn genuinely ends — a later flush (or the
            // reactive-effect trigger it mirrors) must run the deferred
            // refresh.
            model.dispatchPane({ type: "ReconcileTurnActive", at: Date.now(), active: false }, "system");
            await commands.flushPendingControllerRefresh();

            expect(forceControllerRefresh).toHaveBeenCalledOnce();
            dispose();
        });
    });

    // Codex P1 on PR #2338 (twenty-second re-review): flushHeldMessages's own
    // internal flushPendingControllerRefresh() call no-ops (without clearing
    // the flag) when the backend hasn't positively confirmed idle yet —
    // exactly the premature-session_end divergence that gate exists for. The
    // OLD behavior proceeded to drain the queue regardless, delivering
    // straight to a controller that might still hold the stale pre-refresh
    // credential.
    it("does NOT drain the held queue while local turnPhase reads idle but the backend hasn't confirmed idle yet — drains later once the refresh actually completes", async () => {
        hub.dispatchSlashCommand.mockImplementation(async (_msg: string, _registry: unknown, ctx: { deferControllerRefreshUntilIdle: () => void }) => {
            ctx.deferControllerRefreshUntilIdle();
            return { kind: "handled" };
        });
        const model = registerPane(BLOCK_ID, fullRegistration());
        model.dispatchPane({ type: "InitReady", at: Date.now() }, "system");
        model.dispatchPane({ type: "StreamSubscribe", at: Date.now() }, "system");
        const forceControllerRefresh = vi.fn(async () => true);
        let backendConfirmedIdle = false;

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
                forceControllerRefresh,
                beginRecoveryFlow: () => {},
                endRecoveryFlow: () => {},
                isBackendTurnActive: () => false,
                isBackendTurnConfirmedIdle: () => backendConfirmedIdle,
                backToPicker: async () => {},
            });

            // /login succeeds mid-turn (defers instead of refreshing now).
            model.dispatchPane({ type: "TurnStart", at: Date.now() }, "user");
            await commands.sendMessage("/login", /* wasAlreadyWorking */ true);

            // A second message is held behind the same active turn.
            await commands.sendMessage("hi", /* wasAlreadyWorking */ true);
            expect(commands.hasHeldMessages()).toBe(true);

            // Local turnPhase reaches idle, but the backend has NOT
            // positively confirmed idle (the premature session_end
            // divergence) — flushHeldMessages must not drain yet.
            model.dispatchPane({ type: "StreamFlushObserved", addedCount: 1, at: Date.now() }, "system");
            model.dispatchPane({ type: "ReconcileTurnActive", at: Date.now(), active: false }, "system");
            await commands.flushHeldMessages();

            expect(forceControllerRefresh).not.toHaveBeenCalled();
            expect(hub.agentInput).not.toHaveBeenCalled();
            expect(commands.hasHeldMessages()).toBe(true);

            // The backend now positively confirms idle — a later trigger
            // completes the refresh, which must itself drain the queue it
            // left stranded. The drain it triggers is fire-and-forget
            // (flushPendingControllerRefresh's callers must not block on a
            // full queue drain), so give its internal chain a tick to run.
            backendConfirmedIdle = true;
            await commands.flushPendingControllerRefresh();
            await new Promise((resolve) => setTimeout(resolve, 0));

            expect(forceControllerRefresh).toHaveBeenCalledOnce();
            expect(hub.agentInput).toHaveBeenCalledWith(
                expect.anything(),
                expect.objectContaining({ message: "hi" }),
            );
            expect(commands.hasHeldMessages()).toBe(false);
            dispose();
        });
    });
});

// reagent P1 on PR #2338 (seventeenth re-review): a turn can end via the
// independent session_end -> TurnEnd stream path (useTurnLifecycle.ts's
// finalizeTurn), which is NOT synchronized with the controllerstatus event
// stream that drives the deferred-refresh flush elsewhere. A fresh idle
// send landing right after that TurnEnd but before the lagging
// controllerstatus event would otherwise pass every guard (they're all
// already clear) and reach AgentInputCommand while the deferred
// ControllerResyncCommand still hasn't run.
describe("useAgentCommands — idle sendMessage runs the deferred controller refresh before AgentInputCommand", () => {
    it("awaits a pending/in-flight deferred refresh before delivering a fresh idle send", async () => {
        hub.dispatchSlashCommand.mockImplementation(async (_msg: string, _registry: unknown, ctx: { deferControllerRefreshUntilIdle: () => void }) => {
            ctx.deferControllerRefreshUntilIdle();
            return { kind: "handled" };
        });
        const model = registerPane(BLOCK_ID, fullRegistration());
        model.dispatchPane({ type: "InitReady", at: Date.now() }, "system");
        model.dispatchPane({ type: "StreamSubscribe", at: Date.now() }, "system");

        let resolveRefresh: (() => void) | undefined;
        const forceControllerRefresh = vi.fn(
            () => new Promise<boolean>((resolve) => { resolveRefresh = () => resolve(true); }),
        );

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
                forceControllerRefresh,
                beginRecoveryFlow: () => {},
                endRecoveryFlow: () => {},
                isBackendTurnActive: () => false,
                isBackendTurnConfirmedIdle: () => true,
                backToPicker: async () => {},
            });

            // /login succeeds mid-turn (defers instead of refreshing now).
            model.dispatchPane({ type: "TurnStart", at: Date.now() }, "user");
            await commands.sendMessage("/login", /* wasAlreadyWorking */ true);
            expect(forceControllerRefresh).not.toHaveBeenCalled();

            // A fresh message is sent as a genuinely idle send — the exact
            // TurnEnd-raced-ahead-of-controllerstatus scenario.
            const sendPromise = commands.sendMessage("fresh message", /* wasAlreadyWorking */ false);

            // The refresh RPC hasn't resolved yet — AgentInputCommand must
            // not have fired.
            expect(forceControllerRefresh).toHaveBeenCalledOnce();
            expect(hub.agentInput).not.toHaveBeenCalled();

            resolveRefresh?.();
            await sendPromise;

            expect(hub.agentInput).toHaveBeenCalledWith(
                expect.anything(),
                expect.objectContaining({ message: "fresh message" }),
            );
            dispose();
        });
    });

    it("does NOT reject the send on a stale authFailureToPreserve snapshot when the deferred refresh it just ran resolved it (reagent P1 on PR #2338, eighteenth re-review)", async () => {
        // authFailureToPreserve is captured by the CALLER (handleSendMessage
        // in agent-view.tsx) before TurnStart — and before sendMessage ever
        // runs flushPendingControllerRefresh. If that refresh (deferred by
        // an earlier /login success) resolves successfully here, it proves
        // the controller is now confirmed on the fresh credential — the
        // caller's already-captured "there was a live auth failure" snapshot
        // is now stale and must not be trusted to reject this send.
        hub.dispatchSlashCommand.mockImplementation(async (_msg: string, _registry: unknown, ctx: { deferControllerRefreshUntilIdle: () => void }) => {
            ctx.deferControllerRefreshUntilIdle();
            return { kind: "handled" };
        });
        const model = registerPane(BLOCK_ID, fullRegistration());
        model.dispatchPane({ type: "InitReady", at: Date.now() }, "system");
        model.dispatchPane({ type: "StreamSubscribe", at: Date.now() }, "system");
        const forceControllerRefresh = vi.fn(async () => true);

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
                forceControllerRefresh,
                beginRecoveryFlow: () => {},
                endRecoveryFlow: () => {},
                isBackendTurnActive: () => false,
                isBackendTurnConfirmedIdle: () => true,
                backToPicker: async () => {},
            });

            // /login succeeds mid-turn (defers instead of refreshing now).
            model.dispatchPane({ type: "TurnStart", at: Date.now() }, "user");
            await commands.sendMessage("/login", /* wasAlreadyWorking */ true);

            // A fresh idle send captures a (now-stale-by-the-time-it-matters)
            // live auth failure — mirrors handleSendMessage's own capture
            // right before dispatching TurnStart.
            const staleAuthFailure: AgentFailure = { code: "auth", title: "Not logged in", detail: "401", retryable: true };
            await commands.sendMessage("fresh message", /* wasAlreadyWorking */ false, staleAuthFailure);

            expect(forceControllerRefresh).toHaveBeenCalledOnce();
            // Must NOT have been fast-failed on the stale snapshot.
            expect(hub.agentInput).toHaveBeenCalledWith(
                expect.anything(),
                expect.objectContaining({ message: "fresh message" }),
            );
            expect(paneSnapshot(BLOCK_ID)?.failure).toBeNull();
            dispose();
        });
    });

    // Codex P1 on PR #2338 (twenty-second re-review): flushPendingControllerRefresh
    // returns `false` for two different reasons — "nothing was ever deferred"
    // (safe to send) and "a refresh IS deferred but isBackendTurnConfirmedIdle()
    // isn't true yet, so it deliberately left the flag pending" (NOT safe —
    // the controller may still hold the stale pre-refresh credential). The
    // idle-send path must distinguish these and hold the message in the
    // latter case instead of delivering straight through.
    it("holds (does not deliver) a fresh idle send while a deferred refresh is still blocked on backend idle confirmation, then delivers it once the refresh completes", async () => {
        hub.dispatchSlashCommand.mockImplementation(async (_msg: string, _registry: unknown, ctx: { deferControllerRefreshUntilIdle: () => void }) => {
            ctx.deferControllerRefreshUntilIdle();
            return { kind: "handled" };
        });
        const model = registerPane(BLOCK_ID, fullRegistration());
        model.dispatchPane({ type: "InitReady", at: Date.now() }, "system");
        model.dispatchPane({ type: "StreamSubscribe", at: Date.now() }, "system");
        const forceControllerRefresh = vi.fn(async () => true);
        let backendConfirmedIdle = false;

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
                forceControllerRefresh,
                beginRecoveryFlow: () => {},
                endRecoveryFlow: () => {},
                isBackendTurnActive: () => false,
                isBackendTurnConfirmedIdle: () => backendConfirmedIdle,
                backToPicker: async () => {},
            });

            // /login succeeds mid-turn (defers instead of refreshing now).
            model.dispatchPane({ type: "TurnStart", at: Date.now() }, "user");
            await commands.sendMessage("/login", /* wasAlreadyWorking */ true);

            // A fresh idle send arrives — local turnPhase reads idle (the
            // premature session_end scenario), but the backend hasn't
            // positively confirmed idle yet. Must NOT reach AgentInputCommand.
            await commands.sendMessage("fresh message", /* wasAlreadyWorking */ false);
            expect(forceControllerRefresh).not.toHaveBeenCalled();
            expect(hub.agentInput).not.toHaveBeenCalled();
            expect(commands.hasHeldMessages()).toBe(true);

            // The backend now positively confirms idle — a later trigger
            // (mirroring trackTurnJustEnded's edge) flushes the refresh,
            // which must itself drain the message it stranded.
            backendConfirmedIdle = true;
            await commands.flushPendingControllerRefresh();

            expect(forceControllerRefresh).toHaveBeenCalledOnce();
            expect(hub.agentInput).toHaveBeenCalledWith(
                expect.anything(),
                expect.objectContaining({ message: "fresh message" }),
            );
            expect(commands.hasHeldMessages()).toBe(false);
            dispose();
        });
    });

    // reagentx P1 on PR #2338 (twenty-third re-review): when THIS fresh
    // idle send is the one that resolves a pending deferred refresh,
    // flushPendingControllerRefresh's success path fires flushHeldMessages()
    // in the background to drain any PRE-EXISTING held items — but the
    // caller (sendMessage) used to fall straight through to its own direct
    // deliverToBackend call without waiting for that drain, so the two
    // AgentInputCommand deliveries could interleave with no ordering
    // guarantee, violating the file's documented FIFO submission-order
    // invariant.
    it("delivers a pre-existing held message before this fresh send's own message when THIS send is the one that resolves the deferred refresh (FIFO order)", async () => {
        hub.dispatchSlashCommand.mockImplementation(async (_msg: string, _registry: unknown, ctx: { deferControllerRefreshUntilIdle: () => void }) => {
            ctx.deferControllerRefreshUntilIdle();
            return { kind: "handled" };
        });
        const model = registerPane(BLOCK_ID, fullRegistration());
        model.dispatchPane({ type: "InitReady", at: Date.now() }, "system");
        model.dispatchPane({ type: "StreamSubscribe", at: Date.now() }, "system");
        const forceControllerRefresh = vi.fn(async () => true);

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
                forceControllerRefresh,
                beginRecoveryFlow: () => {},
                endRecoveryFlow: () => {},
                isBackendTurnActive: () => false,
                isBackendTurnConfirmedIdle: () => true,
                backToPicker: async () => {},
            });

            // /login succeeds mid-turn (defers instead of refreshing now).
            model.dispatchPane({ type: "TurnStart", at: Date.now() }, "user");
            await commands.sendMessage("/login", /* wasAlreadyWorking */ true);

            // An OLDER message gets held behind the same active turn.
            await commands.sendMessage("old message", /* wasAlreadyWorking */ true);
            expect(commands.hasHeldMessages()).toBe(true);

            // The turn ends; local turnPhase reaches idle.
            model.dispatchPane({ type: "StreamFlushObserved", addedCount: 1, at: Date.now() }, "system");
            model.dispatchPane({ type: "ReconcileTurnActive", at: Date.now(), active: false }, "system");

            // A fresh idle send arrives and is itself the call that resolves
            // the deferred refresh (isBackendTurnConfirmedIdle is true).
            await commands.sendMessage("new message", /* wasAlreadyWorking */ false);

            expect(forceControllerRefresh).toHaveBeenCalledOnce();
            expect(commands.hasHeldMessages()).toBe(false);
            expect(hub.agentInput).toHaveBeenCalledTimes(2);
            // FIFO: the pre-existing held message must be delivered BEFORE
            // this send's own message, not interleaved/reordered.
            expect(hub.agentInput.mock.calls[0][1]).toEqual(
                expect.objectContaining({ message: "old message" }),
            );
            expect(hub.agentInput.mock.calls[1][1]).toEqual(
                expect.objectContaining({ message: "new message" }),
            );
            dispose();
        });
    });

    // codex P2 on PR #2338 (twenty-third re-review): a message held because
    // its deferred refresh was still blocked (not the busy-path hold, which
    // never has its live failure cleared by TurnStart) must carry its
    // captured authFailureToPreserve — otherwise a SUBSEQUENT FAILED
    // deferred refresh (leaving the controller on the still-bad credential)
    // is invisible to flushHeldMessages: authWasKnownBadAtQueueTime is false
    // (a mid-turn auth failure never sets canRetry/loginWaiting) and the
    // live check finds nothing (TurnStart already cleared it at queue time).
    it("rejects a held message (queued while a deferred refresh was blocked) whose captured authFailureToPreserve predates a since-FAILED refresh, and restores its banner", async () => {
        hub.dispatchSlashCommand.mockImplementation(async (_msg: string, _registry: unknown, ctx: { deferControllerRefreshUntilIdle: () => void }) => {
            ctx.deferControllerRefreshUntilIdle();
            return { kind: "handled" };
        });
        const model = registerPane(BLOCK_ID, fullRegistration());
        model.dispatchPane({ type: "InitReady", at: Date.now() }, "system");
        model.dispatchPane({ type: "StreamSubscribe", at: Date.now() }, "system");
        // The deferred refresh itself FAILS.
        const forceControllerRefresh = vi.fn(async () => false);
        let backendConfirmedIdle = false;

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
                forceControllerRefresh,
                beginRecoveryFlow: () => {},
                endRecoveryFlow: () => {},
                isBackendTurnActive: () => false,
                isBackendTurnConfirmedIdle: () => backendConfirmedIdle,
                backToPicker: async () => {},
            });

            // /login succeeds mid-turn (defers instead of refreshing now).
            model.dispatchPane({ type: "TurnStart", at: Date.now() }, "user");
            await commands.sendMessage("/login", /* wasAlreadyWorking */ true);

            // A fresh idle send captures a live auth failure right before
            // TurnStart clears it — mirrors handleSendMessage's own
            // capture. The refresh is still blocked, so this gets held.
            const capturedFailure: AgentFailure = { code: "auth", title: "Not logged in", detail: "401", retryable: true };
            await commands.sendMessage("fresh message", /* wasAlreadyWorking */ false, capturedFailure);
            expect(commands.hasHeldMessages()).toBe(true);
            expect(hub.agentInput).not.toHaveBeenCalled();

            // The backend confirms idle — the deferred refresh runs but
            // FAILS. flushPendingControllerRefresh's own auto-drain only
            // fires on SUCCESS, so — mirroring agent-view.tsx's reactive
            // turnIdle effect, which independently calls flushHeldMessages()
            // whenever hasHeldMessages() is true regardless of the refresh
            // outcome — a separate flushHeldMessages() call is what
            // actually rejects the now-known-bad item in production.
            backendConfirmedIdle = true;
            await commands.flushPendingControllerRefresh();
            await commands.flushHeldMessages();

            expect(forceControllerRefresh).toHaveBeenCalledOnce();
            // Must NOT have been delivered to the still-bad controller.
            expect(hub.agentInput).not.toHaveBeenCalled();
            expect(commands.hasHeldMessages()).toBe(false);
            // The recovery banner must be restored, not silently dropped.
            expect(paneSnapshot(BLOCK_ID)?.failure?.data.code).toBe("auth");
            dispose();
        });
    });

    // reagent P2 on PR #2338 (twenty-ninth re-review): flushHeldMessages's
    // rejection path restores authFailureToPreserve into the failure
    // banner, but recallLatestHeld (the ArrowUp un-queue path) popped the
    // item and returned it to the composer WITHOUT that same restoration —
    // a message held because it captured a live auth failure at queue
    // time silently lost its recovery banner on recall, with nothing left
    // to bring it back.
    it("restores the failure banner when recalling (ArrowUp) a held message that captured a live auth failure at queue time", async () => {
        hub.dispatchSlashCommand.mockImplementation(async (_msg: string, _registry: unknown, ctx: { deferControllerRefreshUntilIdle: () => void }) => {
            ctx.deferControllerRefreshUntilIdle();
            return { kind: "handled" };
        });
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
                forceControllerRefresh: async () => false,
                beginRecoveryFlow: () => {},
                endRecoveryFlow: () => {},
                isBackendTurnActive: () => false,
                isBackendTurnConfirmedIdle: () => false,
                backToPicker: async () => {},
            });

            // /login succeeds mid-turn (defers instead of refreshing now).
            model.dispatchPane({ type: "TurnStart", at: Date.now() }, "user");
            await commands.sendMessage("/login", /* wasAlreadyWorking */ true);

            // A fresh idle send captures a live auth failure right before
            // TurnStart clears it. The refresh is still blocked, so this
            // gets held.
            const capturedFailure: AgentFailure = { code: "auth", title: "Not logged in", detail: "401", retryable: true };
            await commands.sendMessage("held message", /* wasAlreadyWorking */ false, capturedFailure);
            expect(commands.hasHeldMessages()).toBe(true);

            // The user presses ArrowUp to recall the message un-sent —
            // BEFORE any flush ever runs.
            const recalled = commands.recallLatestHeld();

            expect(recalled?.text).toBe("held message");
            expect(commands.hasHeldMessages()).toBe(false);
            // The recovery banner must be restored, not silently dropped.
            expect(paneSnapshot(BLOCK_ID)?.failure?.data.code).toBe("auth");
            dispose();
        });
    });
});

// Codex P1 on PR #2338 (nineteenth re-review): a premature per-round
// session_end can transiently demote the frontend's own turnPhase to
// "Done" even while the backend controller genuinely still reports
// turn_active: true (the same divergence useControllerStatusEvents.ts's
// didTurnJustEnd is deliberately independent of turnPhase to avoid).
// isTurnActive() must trust the authoritative backend signal, not just
// turnPhase, or /login's deferred-refresh check would force-restart a
// controller that's still genuinely working.
describe("useAgentCommands — isTurnActive() trusts the authoritative backend signal over a possibly-stale turnPhase", () => {
    it("reads true when isBackendTurnActive() says active even though turnPhase currently reads Idle", async () => {
        let observedIsTurnActive: boolean | undefined;
        hub.dispatchSlashCommand.mockImplementation(async (_msg: string, _registry: unknown, ctx: { isTurnActive: () => boolean }) => {
            observedIsTurnActive = ctx.isTurnActive();
            return { kind: "handled" };
        });
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
                forceControllerRefresh: async () => true,
                beginRecoveryFlow: () => {},
                endRecoveryFlow: () => {},
                // The backend controller is still genuinely running a turn.
                isBackendTurnActive: () => true,
                isBackendTurnConfirmedIdle: () => false,
                backToPicker: async () => {},
            });

            // Simulates the premature per-round session_end: turnPhase
            // transitions all the way back to Idle even though the turn
            // (per isBackendTurnActive above) never actually ended.
            model.dispatchPane({ type: "TurnStart", at: Date.now() }, "user");
            model.dispatchPane({ type: "StreamFlushObserved", addedCount: 1, at: Date.now() }, "system");
            model.dispatchPane({ type: "ReconcileTurnActive", at: Date.now(), active: false }, "system");
            expect(paneSnapshot(BLOCK_ID)?.turnPhase.kind).toBe("Idle");

            await commands.sendMessage("/login", /* wasAlreadyWorking */ true);

            expect(observedIsTurnActive).toBe(true);
            dispose();
        });
    });

    it("reads true when isBackendTurnActive() says active even for the FROZEN wasAlreadyWorking===false branch (codex P1 on PR #2338, twentieth re-review)", async () => {
        // A premature per-round session_end can ALSO make handleSendMessage
        // itself capture wasAlreadyWorking === false (turnPhase already
        // read Done/Idle at capture time) even though the backend was
        // never actually idle — not just corrupt a later live read (the
        // scenario the previous test covers). Freezing false for this
        // branch is still correct for its OWN reason (an optimistic
        // TurnStart corrupting a LIVE read), but isBackendTurnActive()
        // can't suffer that corruption — it's fed only by real backend
        // events — so it must still be consulted even when
        // wasAlreadyWorking itself is false.
        let observedIsTurnActive: boolean | undefined;
        hub.dispatchSlashCommand.mockImplementation(async (_msg: string, _registry: unknown, ctx: { isTurnActive: () => boolean }) => {
            observedIsTurnActive = ctx.isTurnActive();
            return { kind: "handled" };
        });
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
                forceControllerRefresh: async () => true,
                beginRecoveryFlow: () => {},
                endRecoveryFlow: () => {},
                isBackendTurnActive: () => true,
                isBackendTurnConfirmedIdle: () => false,
                backToPicker: async () => {},
            });

            // handleSendMessage captured wasAlreadyWorking=false (turnPhase
            // already showed idle at that moment).
            await commands.sendMessage("/login", /* wasAlreadyWorking */ false);

            expect(observedIsTurnActive).toBe(true);
            dispose();
        });
    });
});

// Codex P1 on PR #2338 (twentieth re-review): a refresh can be correctly
// deferred while a turn is active, then get FLUSHED before an authoritative
// turn_active: false arrives — a premature frontend Done/Idle transition
// triggers either the held-message flush or a fresh idle send, both of
// which call flushPendingControllerRefresh based on turnPhase-derived
// state. Checking isBackendTurnActive() centrally, inside
// flushPendingControllerRefresh itself, closes this for every caller at
// once instead of requiring each one to re-derive it.
describe("useAgentCommands — flushPendingControllerRefresh leaves the flag pending while the backend is still active", () => {
    it("does NOT run the refresh (and does not consume the pending flag) while isBackendTurnActive() is still true", async () => {
        hub.dispatchSlashCommand.mockImplementation(async (_msg: string, _registry: unknown, ctx: { deferControllerRefreshUntilIdle: () => void }) => {
            ctx.deferControllerRefreshUntilIdle();
            return { kind: "handled" };
        });
        const model = registerPane(BLOCK_ID, fullRegistration());
        model.dispatchPane({ type: "InitReady", at: Date.now() }, "system");
        model.dispatchPane({ type: "StreamSubscribe", at: Date.now() }, "system");
        const forceControllerRefresh = vi.fn(async () => true);
        let backendActive = true;

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
                forceControllerRefresh,
                beginRecoveryFlow: () => {},
                endRecoveryFlow: () => {},
                isBackendTurnActive: () => backendActive,
                isBackendTurnConfirmedIdle: () => !backendActive,
                backToPicker: async () => {},
            });

            // /login succeeds mid-turn (defers instead of refreshing now).
            model.dispatchPane({ type: "TurnStart", at: Date.now() }, "user");
            await commands.sendMessage("/login", /* wasAlreadyWorking */ true);

            // Attempt to flush while the backend still confirms activity —
            // e.g. triggered by a premature frontend Done/Idle transition.
            await commands.flushPendingControllerRefresh();
            expect(forceControllerRefresh).not.toHaveBeenCalled();

            // The flag must still be pending — a LATER, genuine idle
            // confirmation must still be able to run it.
            backendActive = false;
            await commands.flushPendingControllerRefresh();
            expect(forceControllerRefresh).toHaveBeenCalledOnce();
            dispose();
        });
    });

    // reagent P1 on PR #2338 (twenty-first re-review): isBackendTurnActive()
    // is `wasTurnActive === true` — for a pane that mounts mid-turn, before
    // the first live controllerstatus event arrives, that reads `false` even
    // though the backend is NOT confirmed idle either (wasTurnActive is
    // still `undefined`). flushPendingControllerRefresh's destructive
    // force-restart must lean on POSITIVE idle confirmation
    // (isBackendTurnConfirmedIdle), not merely the absence of a positive
    // active signal — otherwise this unconfirmed state would incorrectly
    // flush and could kill a genuinely-active turn.
    it("does NOT run the refresh while the backend state is UNCONFIRMED — isBackendTurnActive() false does not imply isBackendTurnConfirmedIdle()", async () => {
        hub.dispatchSlashCommand.mockImplementation(async (_msg: string, _registry: unknown, ctx: { deferControllerRefreshUntilIdle: () => void }) => {
            ctx.deferControllerRefreshUntilIdle();
            return { kind: "handled" };
        });
        const model = registerPane(BLOCK_ID, fullRegistration());
        model.dispatchPane({ type: "InitReady", at: Date.now() }, "system");
        model.dispatchPane({ type: "StreamSubscribe", at: Date.now() }, "system");
        const forceControllerRefresh = vi.fn(async () => true);

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
                forceControllerRefresh,
                beginRecoveryFlow: () => {},
                endRecoveryFlow: () => {},
                // Neither confirmed active nor confirmed idle — the
                // mount-mid-turn-before-first-event state.
                isBackendTurnActive: () => false,
                isBackendTurnConfirmedIdle: () => false,
                backToPicker: async () => {},
            });

            model.dispatchPane({ type: "TurnStart", at: Date.now() }, "user");
            await commands.sendMessage("/login", /* wasAlreadyWorking */ true);

            await commands.flushPendingControllerRefresh();
            expect(forceControllerRefresh).not.toHaveBeenCalled();
            dispose();
        });
    });
});
