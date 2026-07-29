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
                forceControllerRefresh: async () => true,
                beginRecoveryFlow: () => {},
                endRecoveryFlow: () => {},
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
        let loginWaitingNow = true;
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
                loginWaiting: () => loginWaitingNow,
                setAuthNotice: () => {},
                notifyControllerHealthy: () => {},
                forceControllerRefresh: async () => true,
                beginRecoveryFlow: () => {},
                endRecoveryFlow: () => {},
                backToPicker: async () => {},
            });

            model.dispatchPane({ type: "TurnStart", at: Date.now() }, "user");
            await commands.sendMessage("held during a recovery attempt", true);
            expect(commands.hasHeldMessages()).toBe(true);

            // The recovery attempt FAILS: loginWaiting() clears, but
            // canRetry() was never set true (default retryAfterLogin:true).
            loginWaitingNow = false;

            await commands.flushHeldMessages();

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
                backToPicker: async () => {},
            });

            model.dispatchPane({ type: "TurnStart", at: Date.now() }, "user");
            await commands.sendMessage("u there", false);

            expect(hub.agentInput).toHaveBeenCalledOnce();
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
                backToPicker: async () => {},
            });

            model.dispatchPane({ type: "TurnStart", at: Date.now() }, "user");
            await commands.sendMessage("/login", /* wasAlreadyWorking */ true);
            await commands.flushPendingControllerRefresh();

            expect(forceControllerRefresh).toHaveBeenCalledOnce();
            expect(notifyControllerHealthy).not.toHaveBeenCalled();
            dispose();
        });
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
});
