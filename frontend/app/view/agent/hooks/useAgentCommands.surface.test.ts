// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
// @vitest-environment jsdom

/**
 * What useAgentCommands hands to the rest of the pane, pinned before the hook is split into named
 * pieces: the context every slash command receives, the picker / help / `/btw` state, the
 * lost-delivery timer of an idle send, and the retry budget of a deferred controller refresh. The
 * auth guard, the held queue and the send paths are in useAgentCommands.test.ts.
 */

import { __resetAllSlots as resetDocSlots } from "@/app/store/agent-document-store";
import { registerPane, unregisterPane, type AgentPaneModel } from "@/app/store/agent-pane-registration";
import { __resetAllSlots as resetPaneStateSlots, snapshot as paneSnapshot } from "@/app/store/agent-pane-state-store";
import type { PaneFailure } from "@/app/store/agent-pane-state/types";
import type { AgentFailure } from "@/types/rpc/AgentFailure";
import { createRoot, createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { SlashCommandContext, SlashPickerSpec } from "../commands/types";
import type { TurnLedger } from "@/app/store/agent-pane-state/turn-ledger";
import { forgetRefusedSend, refusedSendOf } from "../failure/refused-send";
import type { ProviderDefinition } from "../providers";
import { useAgentCommands, type UseAgentCommands, type UseAgentCommandsOptions } from "./useAgentCommands";

const hub = vi.hoisted(() => ({
    agentInput: vi.fn(),
    dispatch: vi.fn(),
    shellExec: vi.fn(),
    controllerInput: vi.fn(),
    setMeta: vi.fn(),
    trail: vi.fn(),
}));

vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: {
        AgentInputCommand: (...args: unknown[]) => hub.agentInput(...args),
        ShellExecCommand: (...args: unknown[]) => hub.shellExec(...args),
        ControllerInputCommand: (...args: unknown[]) => hub.controllerInput(...args),
        SetMetaCommand: (...args: unknown[]) => hub.setMeta(...args),
    },
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("@/log/render-trail", async (original) => ({
    ...(await original<typeof import("@/log/render-trail")>()),
    trail: (...args: unknown[]) => hub.trail(...args),
}));
vi.mock("../commands/dispatch", () => ({
    dispatchSlashCommand: (...args: unknown[]) => hub.dispatch(...args),
}));

const BLOCK_ID = "block-surface";
const authFailure = { code: "auth", title: "Not signed in", detail: "", retryable: true } as AgentFailure;
const asPane = (turnAttempted = true): PaneFailure => ({ data: authFailure, at: 1, turnAttempted });

beforeEach(() => {
    hub.agentInput.mockReset().mockResolvedValue(undefined);
    hub.dispatch.mockReset().mockResolvedValue({ kind: "passthrough" });
    hub.shellExec.mockReset().mockResolvedValue({ stdout: "", stderr: "", exit_code: 0 });
    hub.controllerInput.mockReset().mockResolvedValue(undefined);
    hub.setMeta.mockReset().mockResolvedValue(undefined);
    hub.trail.mockReset();
});
afterEach(() => {
    vi.useRealTimers();
    unregisterPane(BLOCK_ID);
    resetDocSlots();
    resetPaneStateSlots();
});

function ready(initReady = true): AgentPaneModel {
    const model = registerPane(BLOCK_ID, { agentId: "agent-1" });
    if (initReady) model.dispatchPane({ type: "InitReady", at: Date.now() }, "system");
    model.dispatchPane({ type: "StreamSubscribe", at: Date.now() }, "system");
    return model;
}

/** Every option a test may want to watch is a spy; `over` replaces any of them. */
function mount(over: Partial<UseAgentCommandsOptions> = {}, initReady = true) {
    const model = ready(initReady);
    const spies = {
        log: vi.fn(),
        setAuthUrl: vi.fn(),
        setAuthNotice: vi.fn(),
        notifyControllerHealthy: vi.fn(),
        forceControllerRefresh: vi.fn(async () => true),
        beginRecoveryFlow: vi.fn(),
        endRecoveryFlow: vi.fn(),
        isCancelled: vi.fn(() => false),
        resetCancelled: vi.fn(),
        backToPicker: vi.fn(async () => {}),
    };
    const opts: UseAgentCommandsOptions = {
        blockId: BLOCK_ID,
        model,
        block: () => undefined,
        provider: () => undefined,
        documentNodes: () => [],
        canRetry: () => false,
        loginWaiting: () => false,
        isBackendTurnActive: () => false,
        isBackendTurnConfirmedIdle: () => true,
        ...spies,
        ...over,
    };
    const dispatched = vi.spyOn(model, "dispatchPane");
    return createRoot((dispose) => ({
        model,
        opts,
        spies,
        dispose,
        dispatched,
        commands: useAgentCommands(opts),
    }));
}

/** What the model was told, by event type. */
const events = (m: { dispatched: ReturnType<typeof vi.spyOn> }, type: string) =>
    m.dispatched.mock.calls.map((c) => c[0] as { type: string; id?: string }).filter((e) => e.type === type);

/** The context a slash command receives, captured by sending one. */
async function contextOf(commands: UseAgentCommands, wasAlreadyWorking = false): Promise<SlashCommandContext> {
    let captured: SlashCommandContext | undefined;
    hub.dispatch.mockImplementationOnce(async (_m: string, _r: unknown, ctx: SlashCommandContext) => {
        captured = ctx;
        return { kind: "handled" };
    });
    await commands.sendMessage("/probe", wasAlreadyWorking);
    return captured!;
}

/** Make the next slash command defer the controller refresh, as /login does mid-turn. */
async function deferRefresh(commands: UseAgentCommands): Promise<void> {
    hub.dispatch.mockImplementationOnce(async (_m: string, _r: unknown, ctx: SlashCommandContext) => {
        ctx.deferControllerRefreshUntilIdle();
        return { kind: "handled" };
    });
    await commands.sendMessage("/login", true);
}

describe("the context every slash command receives", () => {
    it("is built for this pane and hands each pane callback through unchanged", async () => {
        const block = () => ({ meta: { "cmd:cwd": "/work" } });
        const provider = () => ({ id: "codex" }) as ProviderDefinition;
        const documentNodes = () => [];
        const m = mount({ block, provider, documentNodes });
        const ctx = await contextOf(m.commands);

        expect(ctx.blockId).toBe(BLOCK_ID);
        expect(ctx.block).toBe(block);
        expect(ctx.provider).toBe(provider);
        expect(ctx.documentNodes).toBe(documentNodes);
        expect(ctx.log).toBe(m.spies.log);
        expect(ctx.setAuthUrl).toBe(m.spies.setAuthUrl);
        expect(ctx.notifyControllerHealthy).toBe(m.spies.notifyControllerHealthy);
        expect(ctx.forceControllerRefresh).toBe(m.spies.forceControllerRefresh);
        expect(ctx.beginRecoveryFlow).toBe(m.spies.beginRecoveryFlow);
        expect(ctx.endRecoveryFlow).toBe(m.spies.endRecoveryFlow);
        expect(ctx.isCancelled).toBe(m.spies.isCancelled);
        expect(ctx.resetCancelled).toBe(m.spies.resetCancelled);
    });

    it("quickFork is the pane's, and resolves false when the pane has none", async () => {
        const quickFork = vi.fn(async () => true);
        const withFork = await contextOf(mount({ quickFork }).commands);
        expect(await withFork.quickFork()).toBe(true);
        expect(quickFork).toHaveBeenCalledOnce();

        const without = await contextOf(mount().commands);
        expect(await without.quickFork()).toBe(false);
    });

    it("openPicker and openHelp are this hook's own", async () => {
        const m = mount();
        const ctx = await contextOf(m.commands);
        expect(m.commands.helpVisible()).toBe(false);
        ctx.openHelp();
        expect(m.commands.helpVisible()).toBe(true);
        const spec = { title: "Model" } as unknown as SlashPickerSpec;
        void ctx.openPicker(spec).catch(() => {});
        expect(m.commands.pickerSpec()).toBe(spec);
    });

    it("askSideQuestion is the hook's, so /btw opens the overlay", async () => {
        const ask = vi.fn(async () => ({ requestId: "r1" }));
        const m = mount({ askSideQuestion: ask });
        const ctx = await contextOf(m.commands);
        await ctx.askSideQuestion("why?");
        expect(ask).toHaveBeenCalledWith("why?");
        expect(m.commands.btwOverlay()).toMatchObject({ question: "why?", requestId: "r1" });
    });

    it("the registry follows the provider: a provider's own commands appear and go with it", () => {
        let setProvider!: (p: ProviderDefinition | undefined) => void;
        const m = createRoot((dispose) => {
            const [provider, set] = createSignal<ProviderDefinition | undefined>(undefined);
            setProvider = set;
            return { dispose, ...mount({ provider, block: () => ({ meta: { agentId: "agent-1" } }) }) };
        });
        const names = (list: { name: string }[]) => list.map((c) => c.name);

        expect(names(m.commands.availableCommands())).toContain("help");
        expect(names(m.commands.availableCommands())).not.toContain("btw");
        expect(names(m.commands.completions("bt"))).toEqual([]);

        setProvider({ id: "codex" } as ProviderDefinition);
        expect(names(m.commands.availableCommands())).toContain("btw");
        expect(names(m.commands.completions("bt"))).toEqual(["btw"]);

        setProvider(undefined);
        expect(names(m.commands.availableCommands())).not.toContain("btw");
    });
});

describe("the inline picker", () => {
    const spec = { title: "Model" } as unknown as SlashPickerSpec;
    const openOn = async (m: ReturnType<typeof mount>) => (await contextOf(m.commands)).openPicker;

    it("holds the spec while the choice is open, and resolving it hands the value back and closes", async () => {
        const m = mount();
        const openPicker = await openOn(m);
        expect(m.commands.pickerSpec()).toBeNull();
        const choice = openPicker(spec);
        expect(m.commands.pickerSpec()).toBe(spec);
        m.commands.resolvePicker("opus");
        await expect(choice).resolves.toBe("opus");
        expect(m.commands.pickerSpec()).toBeNull();
    });

    it("dismissing rejects the choice and closes", async () => {
        const m = mount();
        const openPicker = await openOn(m);
        const choice = openPicker(spec);
        m.commands.dismissPicker();
        await expect(choice).rejects.toBeUndefined();
        expect(m.commands.pickerSpec()).toBeNull();
    });

    it("opening a second picker dismisses the first and shows the new one", async () => {
        const m = mount();
        const openPicker = await openOn(m);
        const first = openPicker(spec);
        const secondSpec = { title: "Effort" } as unknown as SlashPickerSpec;
        const second = openPicker(secondSpec);
        await expect(first).rejects.toBeUndefined();
        expect(m.commands.pickerSpec()).toBe(secondSpec);
        m.commands.resolvePicker("high");
        await expect(second).resolves.toBe("high");
    });

    it("resolving or dismissing with nothing open does nothing", () => {
        const m = mount();
        expect(() => {
            m.commands.resolvePicker("x");
            m.commands.dismissPicker();
        }).not.toThrow();
        expect(m.commands.pickerSpec()).toBeNull();
    });
});

describe("the help panel", () => {
    it("opens on /help's request and closes on dismiss", async () => {
        const m = mount();
        const ctx = await contextOf(m.commands);
        expect(m.commands.helpVisible()).toBe(false);
        ctx.openHelp();
        expect(m.commands.helpVisible()).toBe(true);
        m.commands.closeHelp();
        expect(m.commands.helpVisible()).toBe(false);
    });
});

describe("the /btw overlay", () => {
    it("shows the question at once, before the request has answered", async () => {
        let answer!: (v: { requestId: string }) => void;
        const m = mount({ askSideQuestion: () => new Promise((r) => (answer = r)) });
        const pending = m.commands.askSideQuestion("why?");
        expect(m.commands.btwOverlay()).toEqual({ askId: expect.any(Number), question: "why?", requestId: null, error: null });
        answer({ requestId: "r1" });
        await pending;
        expect(m.commands.btwOverlay()).toMatchObject({ question: "why?", requestId: "r1", error: null });
    });

    it("records a failed request on the overlay and rethrows it", async () => {
        const m = mount({
            askSideQuestion: async () => {
                throw new Error("no backend");
            },
        });
        await expect(m.commands.askSideQuestion("why?")).rejects.toThrow("no backend");
        expect(m.commands.btwOverlay()).toMatchObject({ question: "why?", requestId: null, error: "no backend" });
    });

    it("records a failure that is not an Error by its text", async () => {
        const m = mount({
            askSideQuestion: async () => {
                throw "plain refusal";
            },
        });
        await expect(m.commands.askSideQuestion("why?")).rejects.toBe("plain refusal");
        expect(m.commands.btwOverlay()?.error).toBe("plain refusal");
    });

    it("answers with an empty request id when the pane gave no way to ask", async () => {
        const m = mount();
        await expect(m.commands.askSideQuestion("why?")).resolves.toEqual({ requestId: "" });
        expect(m.commands.btwOverlay()).toMatchObject({ requestId: "" });
    });

    it("closeBtw removes the overlay", async () => {
        const m = mount({ askSideQuestion: async () => ({ requestId: "r1" }) });
        await m.commands.askSideQuestion("why?");
        expect(m.commands.btwOverlay()).not.toBeNull();
        m.commands.closeBtw();
        expect(m.commands.btwOverlay()).toBeNull();
    });
});

describe("the lost-delivery timer of an idle send", () => {
    const send = async (m: ReturnType<typeof mount>) => {
        m.model.dispatchPane({ type: "TurnStart", at: Date.now() }, "user");
        await m.commands.sendMessage("hello", false);
    };

    it("expires the pending message 30 s after the send, not before", async () => {
        vi.useFakeTimers();
        const m = mount();
        await send(m);
        const queued = events(m, "PendingMessageQueued")[0] as { id: string };
        await vi.advanceTimersByTimeAsync(29_999);
        expect(events(m, "PendingMessageExpired")).toEqual([]);
        await vi.advanceTimersByTimeAsync(1);
        expect(events(m, "PendingMessageExpired")).toEqual([{ type: "PendingMessageExpired", id: queued.id }]);
    });

    it("is not armed for a message that was held and flushed", async () => {
        vi.useFakeTimers();
        const m = mount();
        m.model.dispatchPane({ type: "TurnStart", at: Date.now() }, "user");
        await m.commands.sendMessage("held", true);
        await m.commands.flushHeldMessages();
        expect(hub.agentInput).toHaveBeenCalledOnce();
        await vi.advanceTimersByTimeAsync(120_000);
        expect(events(m, "PendingMessageExpired")).toEqual([]);
    });

    it("is not armed when the send failed", async () => {
        vi.useFakeTimers();
        hub.agentInput.mockRejectedValue(new Error("no controller"));
        const m = mount();
        await send(m);
        await vi.advanceTimersByTimeAsync(120_000);
        expect(events(m, "PendingMessageExpired")).toEqual([]);
    });

    it("is cancelled when the pane goes away", async () => {
        vi.useFakeTimers();
        const m = mount();
        await send(m);
        m.dispose();
        await vi.advanceTimersByTimeAsync(120_000);
        expect(events(m, "PendingMessageExpired")).toEqual([]);
    });
});

describe("the deferred controller refresh", () => {
    const failing = () => vi.fn(async () => false);

    it("retries a failed refresh every 5 s, three times, then leaves it to the next send", async () => {
        vi.useFakeTimers();
        const forceControllerRefresh = failing();
        const m = mount({ forceControllerRefresh });
        await deferRefresh(m.commands);
        await m.commands.flushPendingControllerRefresh();
        expect(forceControllerRefresh).toHaveBeenCalledTimes(1);
        await vi.advanceTimersByTimeAsync(4_999);
        expect(forceControllerRefresh).toHaveBeenCalledTimes(1);
        await vi.advanceTimersByTimeAsync(1);
        expect(forceControllerRefresh).toHaveBeenCalledTimes(2);
        await vi.advanceTimersByTimeAsync(5_000);
        expect(forceControllerRefresh).toHaveBeenCalledTimes(3);
        await vi.advanceTimersByTimeAsync(5_000);
        expect(forceControllerRefresh).toHaveBeenCalledTimes(4);
        await vi.advanceTimersByTimeAsync(60_000);
        expect(forceControllerRefresh).toHaveBeenCalledTimes(4);
    });

    it("a fresh deferral gets the whole retry budget again", async () => {
        vi.useFakeTimers();
        const forceControllerRefresh = failing();
        const m = mount({ forceControllerRefresh });
        await deferRefresh(m.commands);
        await m.commands.flushPendingControllerRefresh();
        await vi.advanceTimersByTimeAsync(60_000);
        expect(forceControllerRefresh).toHaveBeenCalledTimes(4);

        await deferRefresh(m.commands);
        await m.commands.flushPendingControllerRefresh();
        expect(forceControllerRefresh).toHaveBeenCalledTimes(5);
        await vi.advanceTimersByTimeAsync(5_000);
        expect(forceControllerRefresh).toHaveBeenCalledTimes(6);
    });

    it("a second failure replaces the waiting retry instead of adding to it", async () => {
        vi.useFakeTimers();
        const forceControllerRefresh = failing();
        const m = mount({ forceControllerRefresh });
        await deferRefresh(m.commands);
        await m.commands.flushPendingControllerRefresh();
        await vi.advanceTimersByTimeAsync(1_000);
        await m.commands.flushPendingControllerRefresh();
        expect(forceControllerRefresh).toHaveBeenCalledTimes(2);
        // The first retry was due at 5 s; it was replaced by one due at 6 s.
        await vi.advanceTimersByTimeAsync(4_999);
        expect(forceControllerRefresh).toHaveBeenCalledTimes(2);
        await vi.advanceTimersByTimeAsync(1_000);
        expect(forceControllerRefresh).toHaveBeenCalledTimes(3);
    });

    it("a waiting retry is cancelled when the pane goes away", async () => {
        vi.useFakeTimers();
        const forceControllerRefresh = failing();
        const m = mount({ forceControllerRefresh });
        await deferRefresh(m.commands);
        await m.commands.flushPendingControllerRefresh();
        m.dispose();
        await vi.advanceTimersByTimeAsync(60_000);
        expect(forceControllerRefresh).toHaveBeenCalledTimes(1);
    });

    it("callers that arrive while a refresh runs share it and see its result", async () => {
        let finish!: (ok: boolean) => void;
        const forceControllerRefresh = vi.fn(() => new Promise<boolean>((r) => (finish = r)));
        const m = mount({ forceControllerRefresh });
        await deferRefresh(m.commands);
        const first = m.commands.flushPendingControllerRefresh();
        let secondDone = false;
        const second = m.commands.flushPendingControllerRefresh().then((ok) => {
            secondDone = true;
            return ok;
        });
        await Promise.resolve();
        expect(secondDone).toBe(false);
        finish(true);
        await expect(first).resolves.toBe(true);
        await expect(second).resolves.toBe(true);
        expect(forceControllerRefresh).toHaveBeenCalledOnce();
    });

    it("on success it clears the auth failure on screen", async () => {
        const m = mount();
        await deferRefresh(m.commands);
        m.model.dispatchPane({ type: "FailureObserved", failure: authFailure, at: Date.now() }, "system");
        expect(paneSnapshot(BLOCK_ID)?.failure?.data.code).toBe("auth");
        await m.commands.flushPendingControllerRefresh();
        expect(paneSnapshot(BLOCK_ID)?.failure).toBeNull();
        expect(m.spies.notifyControllerHealthy).toHaveBeenCalledOnce();
    });
});

describe("what sendMessage does before it sends", () => {
    const sendIdle = async (m: ReturnType<typeof mount>, text: string, ...rest: unknown[]) => {
        m.model.dispatchPane({ type: "TurnStart", at: Date.now() }, "user");
        // @ts-expect-error the rest arguments are passed through as given
        await m.commands.sendMessage(text, false, ...rest);
    };

    it("leaves a crash trace of the send and of the message it queued", async () => {
        const m = mount();
        await sendIdle(m, "hello");
        expect(hub.trail).toHaveBeenCalledWith("agent:send-message:enter", { blockId: BLOCK_ID, len: 5 });
        const queued = events(m, "PendingMessageQueued")[0] as { id: string };
        expect(hub.trail).toHaveBeenCalledWith("agent:dispatch:PendingMessageQueued", { messageId: queued.id });
    });

    it("reads a command through its leading and trailing blanks, and sends other text as typed", async () => {
        const m = mount();
        await sendIdle(m, "  /probe  ");
        expect(hub.dispatch.mock.calls[0][0]).toBe("/probe");

        hub.shellExec.mockClear();
        await sendIdle(m, "  !pwd  ");
        expect(hub.shellExec).toHaveBeenCalledOnce();
        expect(hub.shellExec.mock.calls[0][1]).toMatchObject({ command: "pwd" });

        await sendIdle(m, "  hi  ");
        expect(hub.agentInput).toHaveBeenCalledWith(expect.anything(), expect.objectContaining({ message: "  hi  " }));
        expect(events(m, "PendingMessageQueued").at(-1)).toMatchObject({ text: "  hi  " });
    });

    it("never treats a message with images as a shell command", async () => {
        const m = mount();
        await sendIdle(m, "!ls", null, [{ id: "a".repeat(64), name: "shot.png" }]);
        expect(hub.shellExec).not.toHaveBeenCalled();
        expect(hub.agentInput).toHaveBeenCalledWith(expect.anything(), expect.objectContaining({ message: "!ls" }));
    });

    it("puts the auth row back after a local command exactly as it was, as the pane's own doing", async () => {
        const m = mount();
        const before = Date.now();
        await sendIdle(m, "!pwd", asPane(false));
        const restored = m.dispatched.mock.calls.filter((c) => (c[0] as { type: string }).type === "FailureObserved");
        expect(restored).toHaveLength(1);
        expect(restored[0][0]).toMatchObject({ failure: authFailure, source: "command", turnAttempted: false });
        expect((restored[0][0] as { at: number }).at).toBeGreaterThanOrEqual(before);
        expect(restored[0][1]).toBe("system");
    });

    it("a slash command that ran while a turn was streaming leaves that turn alone", async () => {
        hub.dispatch.mockResolvedValue({ kind: "handled" });
        const m = mount();
        m.model.dispatchPane({ type: "TurnStart", at: Date.now() }, "user");
        await m.commands.sendMessage("/probe", true);
        expect(events(m, "TurnStartFailed")).toEqual([]);
        hub.dispatch.mockRejectedValue(new Error("boom"));
        await m.commands.sendMessage("/probe", true);
        expect(events(m, "TurnStartFailed")).toEqual([]);
    });

    it("a command that clears the auth failure really clears the live one, and the row is not put back", async () => {
        hub.dispatch.mockImplementationOnce(async (_m: string, _r: unknown, ctx: SlashCommandContext) => {
            ctx.clearAuthFailure();
            return { kind: "handled" };
        });
        const m = mount();
        m.model.dispatchPane({ type: "TurnStart", at: Date.now() }, "user");
        m.model.dispatchPane({ type: "FailureObserved", failure: authFailure, at: Date.now() }, "system");
        expect(paneSnapshot(BLOCK_ID)?.failure?.data.code).toBe("auth");
        await m.commands.sendMessage("/login", false, asPane());
        expect(paneSnapshot(BLOCK_ID)?.failure).toBeNull();
    });

    it("sends nothing while the history is still loading, and says so", async () => {
        const m = mount({}, false);
        await m.commands.sendMessage("too early", false);
        expect(hub.agentInput).not.toHaveBeenCalled();
        expect(events(m, "PendingMessageQueued")).toEqual([]);
        expect(m.spies.log).toHaveBeenCalledWith("send", "send blocked: history still loading", "warn");
    });
});

describe("the pending entry a send queues", () => {
    it("is queued as the person's doing, busy only when a turn was running", async () => {
        const m = mount();
        m.model.dispatchPane({ type: "TurnStart", at: Date.now() }, "user");
        await m.commands.sendMessage("idle one", false);
        await m.commands.sendMessage("busy one", true);
        const queued = m.dispatched.mock.calls.filter((c) => (c[0] as { type: string }).type === "PendingMessageQueued");
        expect(queued.map((c) => [(c[0] as { text: string }).text, (c[0] as { enqueuedWhileBusy: boolean }).enqueuedWhileBusy, c[1]])).toEqual([
            ["idle one", false, "user"],
            ["busy one", true, "user"],
        ]);
    });

    it("carries the system mark when the pane sent it on AgentMux's behalf", async () => {
        const m = mount();
        m.model.dispatchPane({ type: "TurnStart", at: Date.now() }, "user");
        await m.commands.sendMessage("# Session Context", false, null, [], true);
        await m.commands.sendMessage("typed", true);
        const queued = events(m, "PendingMessageQueued") as { text: string; system?: boolean }[];
        expect(queued.find((q) => q.text === "# Session Context")?.system).toBe(true);
        expect(queued.find((q) => q.text === "typed")).not.toHaveProperty("system");
    });

    it("asks the view to scroll on the next frame, and only when the view gave a way to", async () => {
        const frames: FrameRequestCallback[] = [];
        vi.stubGlobal("requestAnimationFrame", (cb: FrameRequestCallback) => frames.push(cb));
        try {
            const onSent = vi.fn();
            const m = mount({ onSent });
            m.model.dispatchPane({ type: "TurnStart", at: Date.now() }, "user");
            await m.commands.sendMessage("hello", false);
            expect(onSent).not.toHaveBeenCalled();
            expect(frames).toHaveLength(1);
            frames[0](0);
            expect(onSent).toHaveBeenCalledOnce();

            frames.length = 0;
            const bare = mount();
            bare.model.dispatchPane({ type: "TurnStart", at: Date.now() }, "user");
            await bare.commands.sendMessage("hello", false);
            expect(frames).toEqual([]);
        } finally {
            vi.unstubAllGlobals();
        }
    });
});

describe("a message typed while a turn runs", () => {
    it("is refused at once while a login attempt is unresolved, with the reason in the log", async () => {
        const m = mount({ loginWaiting: () => true });
        m.model.dispatchPane({ type: "TurnStart", at: Date.now() }, "user");
        await m.commands.sendMessage("held?", true);
        expect(m.commands.hasHeldMessages()).toBe(false);
        expect(m.spies.log).toHaveBeenCalledWith("auth", "message not sent — not logged in", "warn");
        expect(events(m, "PendingMessageRejected")).toHaveLength(1);
        expect(hub.agentInput).not.toHaveBeenCalled();
    });

    it("keeps no copy of an auth failure the caller passed: the live one decides at flush", async () => {
        const m = mount();
        m.model.dispatchPane({ type: "TurnStart", at: Date.now() }, "user");
        await m.commands.sendMessage("held", true, asPane());
        expect(m.commands.hasHeldMessages()).toBe(true);
        await m.commands.flushHeldMessages();
        expect(hub.agentInput).toHaveBeenCalledWith(expect.anything(), expect.objectContaining({ message: "held" }));
    });

    it("waits for the background-only flush to finish before sendMessage resolves", async () => {
        const m = mount();
        m.model.dispatchPane({ type: "TurnStart", at: Date.now() }, "user");
        m.model.dispatchPane({ type: "StreamFlushObserved", addedCount: 1, at: Date.now() }, "system");
        m.model.dispatchPane({ type: "AttachedTaskObserved", at: Date.now() }, "system");
        m.model.dispatchPane({ type: "ModelEndedTurn" }, "system");
        let accept!: () => void;
        hub.agentInput.mockReturnValue(new Promise<void>((r) => (accept = r)));
        let done = false;
        const sent = m.commands.sendMessage("now", true).then(() => (done = true));
        await new Promise((r) => setTimeout(r, 0));
        expect(hub.agentInput).toHaveBeenCalledOnce();
        expect(done).toBe(false);
        accept();
        await sent;
        expect(done).toBe(true);
    });

    it("is not flushed early while the view is showing a launch", async () => {
        const m = mount({ showingLaunchActivity: () => true });
        m.model.dispatchPane({ type: "TurnStart", at: Date.now() }, "user");
        m.model.dispatchPane({ type: "StreamFlushObserved", addedCount: 1, at: Date.now() }, "system");
        m.model.dispatchPane({ type: "AttachedTaskObserved", at: Date.now() }, "system");
        m.model.dispatchPane({ type: "ModelEndedTurn" }, "system");
        await m.commands.sendMessage("wait", true);
        expect(m.commands.hasHeldMessages()).toBe(true);
        expect(hub.agentInput).not.toHaveBeenCalled();
    });
});

const claude = {
    id: "claude",
    controllerType: "persistent",
    launchArgs: ["-p", "--output-format", "stream-json", "--verbose"],
    persistentLaunchArgs: ["--input-format", "stream-json", "--output-format", "stream-json", "--verbose"],
} as unknown as ProviderDefinition;

const ledger = (over: Partial<TurnLedger> = {}): TurnLedger => ({
    turnId: 1_000,
    seq: 1,
    origin: "user",
    trigger: { kind: "user", from: null, external: false },
    absorbed: [],
    startedAtMs: 1_000,
    passes: 1,
    active: true,
    inputs: 0,
    countedPasses: 0,
    outputTokens: 0,
    costUsd: 0,
    steps: 0,
    durationApiMs: 0,
    lastPassEndedAtMs: null,
    settleUntilMs: null,
    endedAtMs: null,
    end: null,
    ...over,
});

const started = (m: ReturnType<typeof mount>) => m.model.dispatchPane({ type: "TurnStart", at: Date.now() }, "user");
const failuresRaised = (m: ReturnType<typeof mount>) => events(m, "FailureObserved");

describe("the idle send's auth guard", () => {
    const notice = "Not logged in — run /login to sign in, then send again.";
    const waitingNotice = "Not logged in yet — wait for the login attempt to finish, then try again.";

    it("refuses before it touches the agent's args, and says why in the log and on screen", async () => {
        const m = mount({ canRetry: () => true, provider: () => claude });
        started(m);
        await m.commands.sendMessage("hi", false);
        expect(hub.setMeta).not.toHaveBeenCalled();
        expect(hub.agentInput).not.toHaveBeenCalled();
        expect(m.spies.log).toHaveBeenCalledWith("auth", "message not sent — not logged in", "warn");
        expect(m.spies.setAuthNotice).toHaveBeenCalledWith(notice);
        expect(events(m, "PendingMessageRejected")).toHaveLength(1);
        expect(events(m, "TurnStartFailed")).toHaveLength(1);
    });

    it("names the unfinished login attempt when one is what blocks the send", async () => {
        const m = mount({ loginWaiting: () => true });
        started(m);
        await m.commands.sendMessage("hi", false);
        expect(m.spies.setAuthNotice).toHaveBeenCalledWith(waitingNotice);
    });

    it("shows no notice when a failure row is already up, whether the caller captured it or it is live", async () => {
        const captured = mount({ canRetry: () => true });
        started(captured);
        await captured.commands.sendMessage("hi", false, asPane());
        expect(captured.spies.setAuthNotice).not.toHaveBeenCalled();
        captured.dispose();
        unregisterPane(BLOCK_ID);

        const live = mount({ canRetry: () => true });
        started(live);
        live.model.dispatchPane({ type: "FailureObserved", failure: authFailure, at: Date.now() }, "system");
        await live.commands.sendMessage("hi", false);
        expect(live.spies.setAuthNotice).not.toHaveBeenCalled();
        expect(hub.agentInput).not.toHaveBeenCalled();
    });

    it("a captured failure alone blocks the send, and so does a live one alone, but a send that starts no turn is not blocked", async () => {
        const captured = mount();
        started(captured);
        await captured.commands.sendMessage("hi", false, asPane());
        expect(hub.agentInput).not.toHaveBeenCalled();
        captured.dispose();
        unregisterPane(BLOCK_ID);

        const live = mount();
        started(live);
        live.model.dispatchPane({ type: "FailureObserved", failure: authFailure, at: Date.now() }, "system");
        await live.commands.sendMessage("hi", false);
        expect(hub.agentInput).not.toHaveBeenCalled();
        live.dispose();
        unregisterPane(BLOCK_ID);

        const held = mount({ canRetry: () => false });
        started(held);
        await held.commands.sendMessage("busy", true);
        held.spies.log.mockClear();
        held.model.dispatchPane({ type: "FailureObserved", failure: authFailure, at: Date.now() }, "system");
        await held.commands.flushHeldMessages();
        expect(held.spies.log).toHaveBeenCalledWith("auth", "held message not sent — not logged in", "warn");
    });
});

describe("the args written before each turn", () => {
    it("carry the runtime config kept in the block's meta", async () => {
        const m = mount({
            provider: () => claude,
            block: () => ({ meta: { agentMode: "host", "agent:runtime": { permissionMode: "plan" } } }),
        });
        started(m);
        await m.commands.sendMessage("hi", false);
        const written = hub.setMeta.mock.calls[0][1] as { meta: { "cmd:args": string[] } };
        const args = written.meta["cmd:args"];
        expect(args[args.indexOf("--permission-mode") + 1]).toBe("plan");
    });

    it("are written to this block, and a failed write is logged without stopping the send", async () => {
        hub.setMeta.mockRejectedValue(new Error("meta down"));
        const m = mount({ provider: () => claude });
        started(m);
        await m.commands.sendMessage("hi", false);
        expect(hub.setMeta.mock.calls[0][1]).toMatchObject({ oref: expect.stringContaining(BLOCK_ID) });
        expect(m.spies.log).toHaveBeenCalledWith("error", "Failed to update runtime args: Error: meta down", "error");
        expect(hub.agentInput).toHaveBeenCalledOnce();
    });
});

describe("delivering one message", () => {
    it("sends the text and its id to this block, and clears an earlier refusal kept for it", async () => {
        const m = mount();
        const { rememberRefusedSend } = await import("../failure/refused-send");
        rememberRefusedSend(BLOCK_ID, { text: "older", attachments: [] });
        started(m);
        await m.commands.sendMessage("hi", false);
        expect(hub.agentInput).toHaveBeenCalledWith(
            expect.anything(),
            expect.objectContaining({ blockid: BLOCK_ID, message: "hi", message_id: expect.stringMatching(/^user_/) }),
        );
        expect(refusedSendOf(BLOCK_ID)).toBeUndefined();
    });

    it("on a failure logs the reason and keeps the refused message with its images and mark", async () => {
        const images = [{ id: "a".repeat(64), name: "shot.png" }];
        hub.agentInput.mockRejectedValue(new Error("This conversation is already open in another pane (block 96a441d2)."));
        const m = mount();
        started(m);
        await m.commands.sendMessage("look", false, null, images, true);
        expect(m.spies.log).toHaveBeenCalledWith("error", expect.stringContaining("already open in another pane"), "error");
        expect(refusedSendOf(BLOCK_ID)).toEqual({ text: "look", attachments: images, system: true });
        forgetRefusedSend(BLOCK_ID);
    });

    it("on any other failure keeps nothing for Move here", async () => {
        hub.agentInput.mockRejectedValue(new Error("network down"));
        const m = mount();
        started(m);
        await m.commands.sendMessage("hi", false);
        expect(refusedSendOf(BLOCK_ID)).toBeUndefined();
    });
});

describe("flushing the held messages", () => {
    const hold = async (m: ReturnType<typeof mount>, ...texts: string[]) => {
        started(m);
        for (const t of texts) await m.commands.sendMessage(t, true);
    };
    const sentTexts = () => hub.agentInput.mock.calls.map((c) => (c[1] as { message: string }).message);

    it("delivers oldest first, marking each as flushing before it goes", async () => {
        const m = mount();
        await hold(m, "a", "b", "c");
        await m.commands.flushHeldMessages();
        expect(sentTexts()).toEqual(["a", "b", "c"]);
        const queued = events(m, "PendingMessageQueued") as { id: string }[];
        expect(events(m, "PendingMessageFlushStarted").map((e) => e.id)).toEqual(queued.map((q) => q.id));
    });

    it("runs one drain at a time: a second call while one is delivering waits for it", async () => {
        const m = mount();
        await hold(m, "a", "b");
        const accepts: (() => void)[] = [];
        hub.agentInput.mockImplementation(() => new Promise<void>((r) => accepts.push(r)));
        const first = m.commands.flushHeldMessages();
        const second = m.commands.flushHeldMessages();
        await new Promise((r) => setTimeout(r, 0));
        expect(sentTexts()).toEqual(["a"]);
        accepts[0]();
        await new Promise((r) => setTimeout(r, 0));
        expect(sentTexts()).toEqual(["a", "b"]);
        accepts[1]();
        await Promise.all([first, second]);
        expect(sentTexts()).toEqual(["a", "b"]);
    });

    it("asks to join the turn it was held during, unless that turn was stopped", async () => {
        const m = mount();
        m.model.dispatchPane({ type: "TurnObserved", ledger: ledger({ turnId: 1_000 }) }, "system");
        await hold(m, "joins");
        await m.commands.flushHeldMessages();
        expect(hub.agentInput.mock.calls[0][1]).toMatchObject({ message: "joins", joins_turn: 1_000 });
        m.dispose();
        unregisterPane(BLOCK_ID);

        for (const how of ["interrupting", "stopped", "completed"] as const) {
            hub.agentInput.mockClear();
            const s = mount();
            s.model.dispatchPane({ type: "TurnObserved", ledger: ledger({ turnId: 2_000 }) }, "system");
            await hold(s, how);
            if (how !== "completed") s.model.dispatchPane({ type: "RequestStop", at: Date.now() }, "user");
            if (how !== "interrupting") s.model.dispatchPane({ type: "TurnEnd", stats: null }, "system");
            await s.commands.flushHeldMessages();
            const sent = hub.agentInput.mock.calls[0][1] as Record<string, unknown>;
            if (how === "completed") expect(sent).toHaveProperty("joins_turn", 2_000);
            else expect(sent).not.toHaveProperty("joins_turn");
            s.dispose();
            unregisterPane(BLOCK_ID);
        }
    });

    it("joins turn 0 too, and joins nothing when srv has reported no turn", async () => {
        const zero = mount();
        zero.model.dispatchPane({ type: "TurnObserved", ledger: ledger({ turnId: 0 }) }, "system");
        await hold(zero, "zero");
        await zero.commands.flushHeldMessages();
        expect(hub.agentInput.mock.calls[0][1]).toHaveProperty("joins_turn", 0);
        zero.dispose();
        unregisterPane(BLOCK_ID);

        hub.agentInput.mockClear();
        const none = mount();
        await hold(none, "none");
        await none.commands.flushHeldMessages();
        expect(hub.agentInput.mock.calls[0][1]).not.toHaveProperty("joins_turn");
    });

    it("a message held by an idle send does not join: its own send starts the turn", async () => {
        const m = mount({ isBackendTurnConfirmedIdle: () => false });
        m.model.dispatchPane({ type: "TurnObserved", ledger: ledger() }, "system");
        await deferRefresh(m.commands);
        started(m);
        await m.commands.sendMessage("idle hold", false);
        expect(m.commands.hasHeldMessages()).toBe(true);
        await m.commands.flushHeldMessages();
        expect(hub.agentInput.mock.calls[0][1]).not.toHaveProperty("joins_turn");
    });

    describe("an idle-send hold whose captured auth failure outlived the refresh", () => {
        const holdWithCapturedFailure = async () => {
            const m = mount({ isBackendTurnConfirmedIdle: () => false });
            await deferRefresh(m.commands);
            started(m);
            await m.commands.sendMessage("idle hold", false, asPane(false));
            expect(m.commands.hasHeldMessages()).toBe(true);
            return m;
        };

        it("is refused at the flush, the turn it started is rolled back and the login row comes back", async () => {
            const m = await holdWithCapturedFailure();
            m.spies.log.mockClear();
            m.dispatched.mockClear();
            const before = Date.now();
            await m.commands.flushHeldMessages();
            expect(hub.agentInput).not.toHaveBeenCalled();
            expect(m.spies.log).toHaveBeenCalledWith("auth", "held message not sent — not logged in", "warn");
            expect(events(m, "TurnStartFailed")).toHaveLength(1);
            expect(events(m, "PendingMessageRejected")).toHaveLength(1);
            const raised = m.dispatched.mock.calls.filter((c) => (c[0] as { type: string }).type === "FailureObserved");
            expect(raised).toHaveLength(1);
            expect(raised[0][0]).toMatchObject({ failure: authFailure, turnAttempted: false, source: "command" });
            expect((raised[0][0] as { at: number }).at).toBeGreaterThanOrEqual(before);
            expect(raised[0][1]).toBe("system");
        });
    });
});

describe("recalling the latest held message", () => {
    it("takes its pending row off the conversation and hands back the text alone when it had no images", async () => {
        const m = mount();
        started(m);
        await m.commands.sendMessage("typed", true);
        const queued = events(m, "PendingMessageQueued")[0] as { id: string };
        const recalled = m.commands.recallLatestHeld();
        expect(recalled).toStrictEqual({ text: "typed" });
        expect(events(m, "PendingMessageRejected")).toEqual([{ type: "PendingMessageRejected", id: queued.id }]);
        expect(m.commands.hasHeldMessages()).toBe(false);
        expect(m.commands.recallLatestHeld()).toBeNull();
    });

    it("hands back the images it was held with", async () => {
        const images = [{ id: "a".repeat(64), name: "shot.png" }];
        const m = mount();
        started(m);
        await m.commands.sendMessage("look", true, null, images);
        expect(m.commands.recallLatestHeld()).toStrictEqual({ text: "look", attachments: images });
    });

    it("does not raise the login row again when a live auth failure already shows", async () => {
        const m = mount({ isBackendTurnConfirmedIdle: () => false });
        await deferRefresh(m.commands);
        started(m);
        await m.commands.sendMessage("idle hold", false, asPane());
        m.model.dispatchPane({ type: "FailureObserved", failure: authFailure, at: Date.now() }, "system");
        m.dispatched.mockClear();
        expect(m.commands.recallLatestHeld()).toStrictEqual({ text: "idle hold" });
        expect(failuresRaised(m)).toEqual([]);
    });
});

describe("back, and stopping the agent", () => {
    it("back waits for the model's backToPicker", async () => {
        let leave!: () => void;
        const backToPicker = vi.fn(() => new Promise<void>((r) => (leave = r)));
        const m = mount({ backToPicker });
        let done = false;
        const p = m.commands.back().then(() => (done = true));
        await Promise.resolve();
        expect(backToPicker).toHaveBeenCalledOnce();
        expect(done).toBe(false);
        leave();
        await p;
        expect(done).toBe(true);
    });

    it("stopping a running turn shows Stopping at once and sends SIGINT to this block", async () => {
        const m = mount();
        started(m);
        m.commands.stopAgent();
        expect(paneSnapshot(BLOCK_ID)?.turnPhase.kind).toBe("Interrupting");
        expect(m.dispatched.mock.calls.find((c) => (c[0] as { type: string }).type === "RequestStop")?.[1]).toBe("user");
        expect(hub.controllerInput).toHaveBeenCalledWith(expect.anything(), { blockid: BLOCK_ID, signame: "SIGINT" });
    });

    it("a refused signal is logged and the pane stops waiting for the stop", async () => {
        hub.controllerInput.mockRejectedValue(new Error("no process"));
        const m = mount();
        started(m);
        m.commands.stopAgent();
        await new Promise((r) => setTimeout(r, 0));
        expect(m.spies.log).toHaveBeenCalledWith("warn", "stop failed: no process", "warn");
        expect(events(m, "StopFailed")).toHaveLength(1);
    });
});

describe("sendWithState", () => {
    it("starts the turn with the message as its content, and passes the images and the system mark on", async () => {
        const images = [{ id: "a".repeat(64), name: "shot.png" }];
        const m = mount();
        await m.commands.sendWithState("# Context", images, true);
        const start = m.dispatched.mock.calls.find((c) => (c[0] as { type: string }).type === "TurnStart");
        expect(start?.[0]).toMatchObject({ content: "# Context" });
        expect(hub.agentInput).toHaveBeenCalledWith(
            expect.anything(),
            expect.objectContaining({ message: "# Context", attachments: images, system: true }),
        );
    });

    it("captures an auth failure that TurnStart clears, so a rejected send can put it back", async () => {
        const m = mount({ canRetry: () => true });
        m.model.dispatchPane({ type: "FailureObserved", failure: authFailure, at: Date.now() }, "system");
        await m.commands.sendWithState("hi");
        expect(hub.agentInput).not.toHaveBeenCalled();
        expect(paneSnapshot(BLOCK_ID)?.failure?.data.code).toBe("auth");
    });

    it("does not capture a failure of another kind: a local command leaves nothing to put back", async () => {
        const rate = { code: "rate_limited", title: "Rate limited", detail: "", retryable: true } as AgentFailure;
        const m = mount();
        m.model.dispatchPane({ type: "FailureObserved", failure: rate, at: Date.now() }, "system");
        m.dispatched.mockClear();
        await m.commands.sendWithState("!pwd");
        expect(failuresRaised(m)).toEqual([]);
    });
});

describe("a !command", () => {
    const run = async (text: string, over: Partial<UseAgentCommandsOptions> = {}) => {
        const m = mount(over);
        started(m);
        await m.commands.sendMessage(text, false);
        return m;
    };
    const logged = (m: ReturnType<typeof mount>) => m.spies.log.mock.calls;

    it("runs in the agent's working directory, under a long timeout, and logs what it ran", async () => {
        const m = await run("!ls -la", { block: () => ({ meta: { "cmd:cwd": "/work/repo" } }) });
        expect(hub.shellExec).toHaveBeenCalledWith(
            expect.anything(),
            { blockid: BLOCK_ID, command: "ls -la", working_dir: "/work/repo" },
            { timeout: 300_000 },
        );
        expect(logged(m)[0]).toEqual(["system", "$ ls -la"]);
    });

    it("runs in no directory when the agent has none", async () => {
        await run("!ls");
        expect(hub.shellExec.mock.calls[0][1]).toMatchObject({ working_dir: "" });
    });

    it("needs a command", async () => {
        const m = await run("!   ");
        expect(hub.shellExec).not.toHaveBeenCalled();
        expect(logged(m)).toEqual([["system", "!: command required", "warn"]]);
    });

    it("logs the output without its trailing blanks, errors as warnings, and a failing exit code", async () => {
        hub.shellExec.mockResolvedValue({ stdout: "out\n\n", stderr: "err\n", exit_code: 2 });
        const m = await run("!make");
        expect(logged(m)).toEqual([
            ["system", "$ make"],
            ["system", "out"],
            ["system", "err", "warn"],
            ["system", "exit 2", "warn"],
        ]);
    });

    it("logs nothing for empty output and a clean exit", async () => {
        const m = await run("!true");
        expect(logged(m)).toEqual([["system", "$ true"]]);
    });

    it("logs a failure to run it as a warning, whatever was thrown", async () => {
        hub.shellExec.mockRejectedValueOnce(new Error("rpc down"));
        const first = await run("!ls");
        expect(logged(first).at(-1)).toEqual(["system", "!: rpc down", "warn"]);
        first.dispose();
        unregisterPane(BLOCK_ID);

        hub.shellExec.mockRejectedValueOnce("plain");
        const second = await run("!ls");
        expect(logged(second).at(-1)).toEqual(["system", "!: plain", "warn"]);
    });
});
