// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * useAgentFailure — characterisation of every path the hook owns that the other test file does not
 * pin: the stored-failure seed (what it waits for, what it never raises), the reducer commands it
 * writes (source, reason, time), the countdown and recycle blocker, Take over and Move here, the
 * copy report and its label timing, the row's handlers, and the verdict plumbing. Written ahead of
 * a split of the hook, so each assertion is on what a caller of the hook can see: the row, the
 * dispatched commands and the callbacks.
 */

import { formatErrorReport } from "@/app/errors/error-report";
import type { AgentPaneEvent } from "@/app/store/agent-pane-state-store";
import { __resetListeners, fireEvent } from "@/app/store/agent-pane-state-store";
import type { PaneFailure } from "@/app/store/agent-pane-state/types";
import { recycleBlockers } from "@/app/store/recycle-blockers";
import { createRoot, createSignal, type Accessor } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { forgetRefusedSend, refusedSendOf, rememberRefusedSend } from "../failure/refused-send";

const hub = vi.hoisted(() => ({
    handlers: new Map<string, (e: unknown) => void>(),
    scopes: [] as { eventType: string; scope?: string }[],
    atomCalls: [] as [string, string][],
    persistedFailure: null as unknown,
    listeners: 0,
    blockers: 0,
}));

vi.mock("@/app/store/mps", () => {
    const muxEventSubscribe = vi.fn((sub: { eventType: string; scope?: string; handler: (e: unknown) => void }) => {
        hub.handlers.set(sub.eventType, sub.handler);
        hub.scopes.push({ eventType: sub.eventType, scope: sub.scope });
        return () => hub.handlers.delete(sub.eventType);
    });
    const onMuxEvent = (eventType: string, handler: (data: unknown, event: unknown) => void, scope?: string) =>
        muxEventSubscribe({
            eventType,
            scope,
            handler: (e) => {
                const ev = e as { data?: unknown } | null;
                if (ev?.data != null) handler(ev.data, ev);
            },
        });
    return { muxEventSubscribe, onMuxEvent };
});
vi.mock("@/app/store/mos", () => ({ makeORef: (a: string, b: string) => `${a}:${b}` }));
vi.mock("@/app/store/global", () => ({
    getBlockMetaKeyAtom: (blockId: string, key: string) => {
        hub.atomCalls.push([blockId, key]);
        return () => hub.persistedFailure;
    },
}));
const restoreRefusedSendMock = vi.hoisted(() => vi.fn());
vi.mock("../failure/refused-send-restore", () => ({ restoreRefusedSend: restoreRefusedSendMock }));
const setBlockMetaMock = vi.hoisted(() => vi.fn<(...a: unknown[]) => Promise<void>>(async () => {}));
vi.mock("@/app/store/block-meta", () => ({ setBlockMeta: setBlockMetaMock }));
const writeTextMock = vi.hoisted(() => vi.fn<(t: string) => Promise<void>>(async () => {}));
vi.mock("@/util/clipboard", () => ({ writeText: (text: string) => writeTextMock(text) }));
vi.mock("@/app/store/agent-pane-state-store", async (importOriginal) => {
    const real = await importOriginal<typeof import("@/app/store/agent-pane-state-store")>();
    return {
        ...real,
        addEventListener: (sink: Parameters<typeof real.addEventListener>[0]) => {
            hub.listeners++;
            const off = real.addEventListener(sink);
            return () => {
                hub.listeners--;
                off();
            };
        },
    };
});
vi.mock("@/app/store/recycle-blockers", async (importOriginal) => {
    const real = await importOriginal<typeof import("@/app/store/recycle-blockers")>();
    return {
        ...real,
        registerRecycleBlocker: (check: Parameters<typeof real.registerRecycleBlocker>[0]) => {
            hub.blockers++;
            const off = real.registerRecycleBlocker(check);
            return () => {
                hub.blockers--;
                off();
            };
        },
    };
});

import type { AgentFailure } from "@/types/rpc/AgentFailure";
import type { AccountState } from "../failure/failure-verdict";
import {
    jitteredBackoffSeconds,
    SEED_WAIT_MS,
    TAKEOVER_CONFIRM_MS,
    useAgentFailure,
    type UseAgentFailureOptions,
    type UseAgentFailureResult,
} from "./useAgentFailure";

const BLOCK_ID = "b";
const T0 = 1_000_000;

const fail = (code: AgentFailure["code"], extra: Partial<AgentFailure> = {}): AgentFailure => ({
    code,
    title: `T-${code}`,
    detail: `d-${code}`,
    retryable: false,
    ...extra,
});
const transient = () => fail("rate_limited", { title: "Throttled", detail: "429", retryable: true });

type Cmd = {
    type: string;
    source?: string;
    reason?: string;
    failure?: AgentFailure;
    at?: number;
    since?: number;
    turnAttempted?: boolean;
};

const fire = (data: unknown) => {
    const h = hub.handlers.get("agentfailure");
    if (!h) throw new Error("no agentfailure handler: onMount did not run");
    h({ data });
};
const flush = async () => {
    for (let i = 0; i < 8; i++) await Promise.resolve();
};

function mk(over: Partial<UseAgentFailureOptions> = {}) {
    const blockId = over.blockId ?? BLOCK_ID;
    const [failure, setFailure] = createSignal<PaneFailure | null>(null);
    const model = {
        blockId,
        disposed: false,
        dispatchPane: vi.fn((c: Cmd) => {
            if (c.type === "FailureObserved") {
                setFailure({
                    data: c.failure!,
                    at: c.at!,
                    source: c.source,
                    turnAttempted: c.turnAttempted,
                } as PaneFailure);
            } else if (c.type === "FailureCleared") {
                setFailure(null);
            }
            return [];
        }),
        dispatchDoc: vi.fn(() => []),
    };
    const cb = {
        onRetry: vi.fn(),
        onLoginAgain: vi.fn(),
        onLoginViaTerminal: vi.fn(),
        onOpenAccounts: vi.fn(),
        onNewSession: vi.fn(),
    };
    const ui = useAgentFailure({
        blockId,
        model: model as unknown as UseAgentFailureOptions["model"],
        failure,
        ...cb,
        ...over,
    });
    const calls = () => model.dispatchPane.mock.calls.map((c) => c[0] as Cmd);
    return {
        ui,
        model,
        failure,
        setFailure,
        ...cb,
        calls,
        seeds: () => calls().filter((c) => c.type === "FailureObserved" && c.source === "seed"),
        lives: () => calls().filter((c) => c.type === "FailureObserved" && c.source === "live"),
        cleared: () =>
            calls()
                .filter((c) => c.type === "FailureCleared")
                .map((c) => c.reason),
        external: (data: AgentFailure, extra: Partial<PaneFailure> = {}) =>
            setFailure({ data, at: Date.now(), ...extra }),
    };
}

const action = (ui: UseAgentFailureResult, name: string) =>
    (ui.row()?.actions ?? []).find((a) => a.label === name || a.title === name);
const click = (ui: UseAgentFailureResult, name: string) => {
    const a = action(ui, name);
    if (!a) throw new Error(`no action "${name}" in [${(ui.row()?.actions ?? []).map((x) => x.label ?? x.title)}]`);
    a.onClick!(undefined as unknown as MouseEvent);
};
const labels = (ui: UseAgentFailureResult) => (ui.row()?.actions ?? []).map((a) => a.label ?? a.title);

async function inRoot(fn: (dispose: () => void) => Promise<void>) {
    await createRoot(async (dispose) => {
        try {
            await fn(dispose);
        } finally {
            dispose();
        }
    });
}

beforeEach(() => {
    hub.handlers.clear();
    hub.scopes.length = 0;
    hub.atomCalls.length = 0;
    hub.persistedFailure = null;
    hub.listeners = 0;
    hub.blockers = 0;
    __resetListeners();
    vi.useFakeTimers();
    vi.setSystemTime(T0);
    // Jitter at its midpoint: the ladder lands on its nominal seconds.
    vi.spyOn(Math, "random").mockReturnValue(0.5);
    writeTextMock.mockReset();
    writeTextMock.mockResolvedValue(undefined);
    setBlockMetaMock.mockReset();
    setBlockMetaMock.mockResolvedValue(undefined);
    restoreRefusedSendMock.mockReset();
    forgetRefusedSend(BLOCK_ID);
});
afterEach(() => {
    vi.useRealTimers();
    vi.restoreAllMocks();
});

describe("jitteredBackoffSeconds and the constants", () => {
    it("spreads a rung by plus or minus 20 percent, rounded", () => {
        expect(jitteredBackoffSeconds(10, () => 0)).toBe(8);
        expect(jitteredBackoffSeconds(10, () => 0.5)).toBe(10);
        expect(jitteredBackoffSeconds(10, () => 1)).toBe(12);
        expect(jitteredBackoffSeconds(5, () => 0.4)).toBe(5); // 4.8 rounds up
        expect(jitteredBackoffSeconds(5, () => 0.1)).toBe(4); // 4.2 rounds down
    });

    it("never starts a countdown below one second", () => {
        expect(jitteredBackoffSeconds(1, () => 0.5)).toBe(1);
        expect(jitteredBackoffSeconds(0, () => 0.5)).toBe(1);
        expect(jitteredBackoffSeconds(1, () => 0)).toBe(1);
    });

    it("draws from Math.random when no source is given", () => {
        vi.spyOn(Math, "random").mockReturnValue(0);
        expect(jitteredBackoffSeconds(10)).toBe(8);
    });

    it("pins the confirm window and the seed wait", () => {
        expect(TAKEOVER_CONFIRM_MS).toBe(5000);
        expect(SEED_WAIT_MS).toBe(1500);
    });
});

describe("the stored failure seeded at mount", () => {
    it("reads agent:last_failure of its own block", async () => {
        await inRoot(async () => {
            mk({ blockId: "blk-7" });
            await flush();
            expect(hub.atomCalls).toEqual([["blk-7", "agent:last_failure"]]);
        });
    });

    it("raises it as a seed, at the time srv recorded, with the mount time as its floor", async () => {
        hub.persistedFailure = { ...fail("killed"), atMs: 12_345 };
        await inRoot(async () => {
            const { ui, seeds } = mk();
            await flush();
            expect(seeds()).toHaveLength(1);
            expect(seeds()[0]).toMatchObject({ type: "FailureObserved", source: "seed", at: 12_345, since: T0 });
            expect(seeds()[0].failure!.code).toBe("killed");
            expect(ui.row()!.title).toBe("T-killed");
        });
    });

    it.each([[0], [-5], [Infinity], ["12"], [undefined]])(
        "falls back to now when the stored time is %s",
        async (atMs) => {
            hub.persistedFailure = { ...fail("killed"), atMs };
            await inRoot(async () => {
                const { seeds } = mk();
                await flush();
                expect(seeds()[0].at).toBe(T0);
            });
        }
    );

    it("does not raise a stored usage limit whose reset has passed", async () => {
        hub.persistedFailure = { ...fail("usage_limit", { resetsAtMs: T0 - 1000 }), atMs: T0 - 5000 };
        await inRoot(async () => {
            const { ui, seeds } = mk();
            await flush();
            expect(seeds()).toHaveLength(0);
            expect(ui.row()).toBeNull();
        });
    });

    it("does not raise it over a failure that is already showing", async () => {
        hub.persistedFailure = fail("killed");
        await inRoot(async () => {
            const { ui, seeds, external } = mk();
            external(fail("network"));
            await flush();
            expect(seeds()).toHaveLength(0);
            expect(ui.row()!.title).toBe("T-network");
        });
    });

    it("raises a stored transient failure at once even while srv's state is unknown", async () => {
        hub.persistedFailure = transient();
        await inRoot(async () => {
            const { seeds } = mk({ liveAccount: () => "unknown" });
            await flush();
            expect(seeds()).toHaveLength(1);
        });
    });

    it("raises a stored sign-in failure at once when no account state is wired", async () => {
        hub.persistedFailure = fail("auth");
        await inRoot(async () => {
            const { seeds } = mk();
            await flush();
            expect(seeds()).toHaveLength(1);
        });
    });

    it("raises a stored sign-in failure at once when srv has already said", async () => {
        hub.persistedFailure = fail("auth");
        await inRoot(async () => {
            const { seeds } = mk({ liveAccount: () => "signed_out" });
            await flush();
            expect(seeds()).toHaveLength(1);
        });
    });

    it("waits exactly SEED_WAIT_MS for srv, then raises a stored sign-in failure", async () => {
        hub.persistedFailure = fail("auth");
        await inRoot(async () => {
            const { seeds } = mk({ liveAccount: () => "unknown" });
            await flush();
            vi.advanceTimersByTime(1499);
            expect(seeds()).toHaveLength(0);
            vi.advanceTimersByTime(1);
            expect(seeds()).toHaveLength(1);
        });
    });

    it("raises it once when srv answers, and not again when the wait runs out", async () => {
        hub.persistedFailure = fail("auth");
        await inRoot(async () => {
            const [account, setAccount] = createSignal<AccountState>("unknown");
            const { ui, seeds } = mk({ liveAccount: account });
            await flush();
            expect(seeds()).toHaveLength(0);
            setAccount("signed_out");
            expect(seeds()).toHaveLength(1);
            click(ui, "Dismiss");
            vi.advanceTimersByTime(SEED_WAIT_MS * 2);
            expect(seeds()).toHaveLength(1);
        });
    });

    it("a live failure that arrives during the wait cancels the stored one for good", async () => {
        hub.persistedFailure = fail("auth");
        await inRoot(async () => {
            const [account, setAccount] = createSignal<AccountState>("unknown");
            const { ui, seeds } = mk({ liveAccount: account });
            await flush();
            fire(fail("killed"));
            click(ui, "Dismiss");
            setAccount("signed_out");
            vi.advanceTimersByTime(SEED_WAIT_MS * 2);
            expect(seeds()).toHaveLength(0);
            expect(ui.row()).toBeNull();
        });
    });

    it("a failure that appears by other means during the wait is not overwritten by the stored one", async () => {
        hub.persistedFailure = fail("auth");
        await inRoot(async () => {
            const { ui, seeds, external } = mk({ liveAccount: () => "unknown" });
            await flush();
            external(fail("killed"));
            vi.advanceTimersByTime(SEED_WAIT_MS);
            expect(seeds()).toHaveLength(0);
            expect(ui.row()!.title).toBe("T-killed");
        });
    });

    it("drops the wait when the pane goes away", async () => {
        hub.persistedFailure = fail("auth");
        await inRoot(async (dispose) => {
            const { seeds } = mk({ liveAccount: () => "unknown" });
            await flush();
            dispose();
            vi.advanceTimersByTime(SEED_WAIT_MS * 2);
            expect(seeds()).toHaveLength(0);
        });
    });

    it("a turn that is not completed does not end the wait", async () => {
        hub.persistedFailure = fail("auth");
        await inRoot(async () => {
            const { seeds } = mk({ liveAccount: () => "unknown" });
            await flush();
            fireEvent(BLOCK_ID, {
                type: "turn-ended",
                outcome: "interrupted",
                statsMerged: false,
                stoppingCleared: false,
            } as unknown as AgentPaneEvent);
            vi.advanceTimersByTime(SEED_WAIT_MS);
            expect(seeds()).toHaveLength(1);
        });
    });

    it("a turn that starts in another pane does not end the wait", async () => {
        hub.persistedFailure = fail("auth");
        await inRoot(async () => {
            const { seeds } = mk({ liveAccount: () => "unknown" });
            await flush();
            fireEvent("other", { type: "turn-started", at: Date.now() } as AgentPaneEvent);
            vi.advanceTimersByTime(SEED_WAIT_MS);
            expect(seeds()).toHaveLength(1);
        });
    });

    it("shows a stored transient failure as the lapsed note, saying how long ago", async () => {
        hub.persistedFailure = { ...transient(), atMs: T0 - 10 * 60_000 };
        await inRoot(async () => {
            const { ui } = mk();
            await flush();
            expect(ui.verdict()).toBe("lapsed");
            expect(ui.row()!.title).toBe("Throttled · 10 min ago");
        });
    });
});

describe("a live failure", () => {
    it("subscribes to this block's agentfailure events", async () => {
        await inRoot(async () => {
            mk({ blockId: "blk-9" });
            await flush();
            expect(hub.scopes).toEqual([{ eventType: "agentfailure", scope: "block:blk-9" }]);
        });
    });

    it("is raised as a live failure at the time it arrives", async () => {
        await inRoot(async () => {
            const { lives } = mk();
            await flush();
            vi.advanceTimersByTime(777);
            fire(fail("killed"));
            expect(lives()).toHaveLength(1);
            expect(lives()[0]).toMatchObject({ source: "live", at: T0 + 777 });
        });
    });

    it("shows no row and no verdict before one arrives", async () => {
        await inRoot(async () => {
            const { ui } = mk();
            await flush();
            expect(ui.row()).toBeNull();
            expect(ui.verdict()).toBeNull();
        });
    });

    it("arrives collapsed, and a newer one collapses an open body", async () => {
        await inRoot(async () => {
            const { ui } = mk();
            await flush();
            fire(fail("killed"));
            expect(ui.row()!.expanded).toBe(false);
            expect(action(ui, "Details")).toBeDefined();
            click(ui, "Details");
            expect(ui.row()!.expanded).toBe(true);
            fire(fail("no_output"));
            expect(ui.row()!.expanded).toBe(false);
        });
    });

    it("a newer one clears the Copied label", async () => {
        await inRoot(async () => {
            const { ui } = mk();
            await flush();
            fire(fail("killed"));
            click(ui, "Copy error");
            await flush();
            expect(action(ui, "Copied")).toBeDefined();
            fire(fail("no_output"));
            expect(action(ui, "Copy error")).toBeDefined();
        });
    });

    it("a newer non-transient one cancels a running countdown", async () => {
        await inRoot(async () => {
            const { ui, onRetry } = mk();
            await flush();
            fire(transient());
            vi.advanceTimersByTime(2000);
            fire(fail("killed"));
            vi.advanceTimersByTime(300_000);
            expect(onRetry).not.toHaveBeenCalled();
            expect(labels(ui)).toContain("Retry");
        });
    });

    it("a non-transient one never arms a countdown", async () => {
        await inRoot(async () => {
            const { ui, onRetry } = mk();
            await flush();
            fire(fail("unknown_non_zero"));
            expect(recycleBlockers()).toEqual([]);
            vi.advanceTimersByTime(300_000);
            expect(onRetry).not.toHaveBeenCalled();
            expect(ui.row()).not.toBeNull();
        });
    });

    it("a transient one waiting for the API arms no countdown", async () => {
        await inRoot(async () => {
            const reconnect = {
                waitsForReconnect: vi.fn(() => true),
                reconnectHint: vi.fn(() => null),
                retry: vi.fn(() => false),
                retryBusy: vi.fn(() => false),
                failureDismissed: vi.fn(),
            };
            const { onRetry } = mk({ reconnect });
            await flush();
            fire(fail("network"));
            expect(reconnect.waitsForReconnect).toHaveBeenCalledWith("network");
            vi.advanceTimersByTime(300_000);
            expect(onRetry).not.toHaveBeenCalled();
        });
    });

    it("a newer one starts Take over from its first click again", async () => {
        await inRoot(async () => {
            const { ui } = mk({ onTakeOver: vi.fn(async () => {}) });
            await flush();
            fire(fail("live_elsewhere"));
            click(ui, "Take over");
            expect(action(ui, "Confirm take over")).toBeDefined();
            fire(fail("live_elsewhere"));
            expect(action(ui, "Take over")).toBeDefined();
        });
    });

    it("is judged by srv's account state only once that has changed since it arrived", async () => {
        hub.persistedFailure = fail("auth");
        await inRoot(async () => {
            const [account, setAccount] = createSignal<AccountState>("signed_out");
            const { ui } = mk({ liveAccount: account });
            await flush();
            expect(ui.row()).not.toBeNull();
            vi.advanceTimersByTime(10);
            setAccount("can_work");
            expect(ui.row()).toBeNull();
            vi.advanceTimersByTime(10);
            fire(fail("auth"));
            expect(ui.row()).not.toBeNull();
            expect(ui.verdict()).toBe("active");
        });
    });
});

describe("the countdown", () => {
    it("starts at the first rung and ticks down each second", async () => {
        await inRoot(async () => {
            const { ui } = mk();
            await flush();
            fire(transient());
            expect(action(ui, "Retry now (5s)")).toBeDefined();
            vi.advanceTimersByTime(1000);
            expect(action(ui, "Retry now (4s)")).toBeDefined();
            vi.advanceTimersByTime(1000);
            expect(action(ui, "Retry now (3s)")).toBeDefined();
        });
    });

    it("rounds the seconds left up", async () => {
        await inRoot(async () => {
            const { ui } = mk();
            await flush();
            fire(transient());
            vi.setSystemTime(Date.now() + 800);
            vi.advanceTimersByTime(1000);
            expect(action(ui, "Retry now (4s)")).toBeDefined();
        });
    });

    it("fires the retry when it reaches zero, and keeps counting toward the cap", async () => {
        await inRoot(async () => {
            const { ui, onRetry } = mk();
            await flush();
            fire(transient());
            vi.advanceTimersByTime(4999);
            expect(onRetry).not.toHaveBeenCalled();
            vi.advanceTimersByTime(1);
            expect(onRetry).toHaveBeenCalledTimes(1);
            expect(ui.row()).toBeNull();
            fire(transient());
            expect(action(ui, "Retry now (15s)")).toBeDefined();
        });
    });

    it("is a recycle blocker while it runs and not after", async () => {
        await inRoot(async () => {
            const { ui } = mk();
            await flush();
            expect(recycleBlockers()).toEqual([]);
            fire(transient());
            expect(recycleBlockers()).toEqual(["a failed turn is about to be retried"]);
            click(ui, "Dismiss");
            expect(recycleBlockers()).toEqual([]);
        });
    });

    it("leaves no countdown running once the ladder is spent", async () => {
        await inRoot(async () => {
            const { ui } = mk();
            await flush();
            for (const ms of [5000, 15000, 30000, 60000, 120000]) {
                fire(transient());
                vi.advanceTimersByTime(ms);
            }
            fire(transient());
            expect(ui.verdict()).toBe("lapsed");
            expect(recycleBlockers()).toEqual([]);
            expect(vi.getTimerCount()).toBe(0);
        });
    });

    it("stops when the pane goes away", async () => {
        await inRoot(async (dispose) => {
            const { onRetry } = mk();
            await flush();
            fire(transient());
            dispose();
            vi.advanceTimersByTime(300_000);
            expect(onRetry).not.toHaveBeenCalled();
        });
    });

    it("stops listening when the pane goes away", async () => {
        await inRoot(async (dispose) => {
            mk();
            await flush();
            expect(hub.handlers.has("agentfailure")).toBe(true);
            dispose();
            expect(hub.handlers.has("agentfailure")).toBe(false);
        });
    });
});

describe("Retry, Dismiss and what each writes", () => {
    it("Retry re-runs the turn, clears the row as retried, and leaves the next row fresh", async () => {
        await inRoot(async () => {
            const { ui, onRetry, cleared, external } = mk();
            await flush();
            fire(fail("killed"));
            click(ui, "Details");
            click(ui, "Retry");
            expect(onRetry).toHaveBeenCalledTimes(1);
            expect(cleared()).toEqual(["retried"]);
            external(fail("killed"));
            expect(ui.row()!.expanded).toBe(false);
            expect(action(ui, "Retry")!.disabled).toBeFalsy();
        });
    });

    it("Retry does nothing while the recovery's own retry is on its way", async () => {
        await inRoot(async () => {
            const reconnect = {
                waitsForReconnect: () => false,
                reconnectHint: () => null,
                retry: vi.fn(() => true),
                retryBusy: vi.fn(() => true),
                failureDismissed: vi.fn(),
            };
            const { ui, onRetry, cleared } = mk({ reconnect });
            await flush();
            fire(fail("killed"));
            expect(action(ui, "Retry")!.disabled).toBe(true);
            click(ui, "Retry");
            expect(reconnect.retry).not.toHaveBeenCalled();
            expect(onRetry).not.toHaveBeenCalled();
            expect(cleared()).toEqual([]);
        });
    });

    it("Retry goes through the recovery when it tracks the failed turn, and not also to onRetry", async () => {
        await inRoot(async () => {
            const reconnect = {
                waitsForReconnect: () => false,
                reconnectHint: () => null,
                retry: vi.fn(() => true),
                retryBusy: () => false,
                failureDismissed: vi.fn(),
            };
            const { ui, onRetry } = mk({ reconnect });
            await flush();
            fire(fail("killed"));
            click(ui, "Retry");
            expect(reconnect.retry).toHaveBeenCalledTimes(1);
            expect(onRetry).not.toHaveBeenCalled();
        });
    });

    it("Retry falls back to onRetry when the recovery does not take the turn", async () => {
        await inRoot(async () => {
            const reconnect = {
                waitsForReconnect: () => false,
                reconnectHint: () => null,
                retry: vi.fn(() => false),
                retryBusy: () => false,
                failureDismissed: vi.fn(),
            };
            const { ui, onRetry } = mk({ reconnect });
            await flush();
            fire(fail("killed"));
            click(ui, "Retry");
            expect(onRetry).toHaveBeenCalledTimes(1);
        });
    });

    it("Dismiss ends the episode, stops the countdown and tells the recovery before the row clears", async () => {
        await inRoot(async () => {
            let rowShownWhenTold: boolean | undefined;
            const reconnect = {
                waitsForReconnect: () => false,
                reconnectHint: () => null,
                retry: () => false,
                retryBusy: () => false,
                failureDismissed: vi.fn(() => {
                    rowShownWhenTold = ui.row() != null;
                }),
            };
            const { ui, onRetry, cleared } = mk({ reconnect });
            await flush();
            fire(transient());
            click(ui, "Dismiss");
            expect(reconnect.failureDismissed).toHaveBeenCalledTimes(1);
            expect(rowShownWhenTold).toBe(true);
            expect(cleared()).toEqual(["episode-ended"]);
            vi.advanceTimersByTime(300_000);
            expect(onRetry).not.toHaveBeenCalled();
            expect(ui.row()).toBeNull();
        });
    });

    it("Dismiss of a network failure forgets it in the block's meta, and swallows a failed write", async () => {
        const swallow = vi.fn();
        setBlockMetaMock.mockReturnValueOnce({ catch: swallow } as unknown as Promise<void>);
        await inRoot(async () => {
            const { ui } = mk();
            await flush();
            fire(fail("network"));
            click(ui, "Dismiss");
            await flush();
            expect(setBlockMetaMock).toHaveBeenCalledWith(BLOCK_ID, { "agent:last_failure": null });
            expect(swallow).toHaveBeenCalledTimes(1);
            expect(ui.row()).toBeNull();
        });
    });

    it("Dismiss of any other class leaves the block's meta alone", async () => {
        await inRoot(async () => {
            const { ui } = mk();
            await flush();
            for (const code of ["rate_limited", "auth", "usage_limit", "killed"] as const) {
                fire(fail(code));
                click(ui, "Dismiss");
            }
            expect(setBlockMetaMock).not.toHaveBeenCalled();
        });
    });

    it("Dismiss of a failure that is not about a refused send forgets the kept message", async () => {
        rememberRefusedSend(BLOCK_ID, { text: "hello", attachments: [] });
        await inRoot(async () => {
            const { ui } = mk();
            await flush();
            fire(fail("killed"));
            click(ui, "Dismiss");
            expect(refusedSendOf(BLOCK_ID)).toBeUndefined();
            expect(restoreRefusedSendMock).not.toHaveBeenCalled();
        });
    });

    it("Dismiss of an open-in-other-pane failure puts the refused message back instead", async () => {
        rememberRefusedSend(BLOCK_ID, { text: "hello", attachments: [] });
        await inRoot(async () => {
            const { ui } = mk();
            await flush();
            fire(fail("open_in_other_pane"));
            click(ui, "Dismiss");
            expect(restoreRefusedSendMock).toHaveBeenCalledWith(BLOCK_ID);
            expect(refusedSendOf(BLOCK_ID)).toBeDefined();
        });
    });

    it("a completed turn after the failure heals it as the end of an episode", async () => {
        await inRoot(async () => {
            const { ui, cleared } = mk();
            await flush();
            fire(fail("killed"));
            vi.advanceTimersByTime(5);
            fireEvent(BLOCK_ID, {
                type: "turn-ended",
                outcome: "completed",
                statsMerged: false,
                stoppingCleared: false,
            });
            expect(ui.row()).toBeNull();
            expect(cleared()).toEqual(["episode-ended"]);
        });
    });

    it("a turn that did not complete heals nothing", async () => {
        await inRoot(async () => {
            const { ui } = mk();
            await flush();
            fire(fail("killed"));
            vi.advanceTimersByTime(5);
            fireEvent(BLOCK_ID, {
                type: "turn-ended",
                outcome: "interrupted",
                statsMerged: false,
                stoppingCleared: false,
            } as unknown as AgentPaneEvent);
            expect(ui.row()).not.toBeNull();
        });
    });

    it("a turn that completes in another pane heals nothing", async () => {
        await inRoot(async () => {
            const { ui } = mk();
            await flush();
            fire(fail("killed"));
            vi.advanceTimersByTime(5);
            fireEvent("other", {
                type: "turn-ended",
                outcome: "completed",
                statsMerged: false,
                stoppingCleared: false,
            });
            expect(ui.row()).not.toBeNull();
        });
    });

    it("an armed Take over stays armed while the failure is replaced, and is dropped when it clears", async () => {
        await inRoot(async () => {
            const { ui, external, model } = mk({ onTakeOver: vi.fn(async () => {}) });
            await flush();
            external(fail("live_elsewhere"));
            click(ui, "Take over");
            expect(action(ui, "Confirm take over")).toBeDefined();
            external(fail("live_elsewhere", { title: "Again" }));
            expect(action(ui, "Confirm take over")).toBeDefined();
            model.dispatchPane({ type: "FailureCleared" });
            external(fail("live_elsewhere"));
            expect(action(ui, "Take over")).toBeDefined();
        });
    });
});

describe("Take over and Move here", () => {
    const deferred = () => {
        let resolve!: () => void;
        let reject!: (e: unknown) => void;
        const promise = new Promise<void>((res, rej) => {
            resolve = res;
            reject = rej;
        });
        return { promise, resolve, reject };
    };

    it("disarms after exactly TAKEOVER_CONFIRM_MS", async () => {
        await inRoot(async () => {
            const { ui } = mk({ onTakeOver: vi.fn(async () => {}) });
            await flush();
            fire(fail("live_elsewhere"));
            click(ui, "Take over");
            vi.advanceTimersByTime(4999);
            expect(action(ui, "Confirm take over")).toBeDefined();
            vi.advanceTimersByTime(1);
            expect(action(ui, "Take over")).toBeDefined();
        });
    });

    it("a dropped arming does not cut a later one short", async () => {
        await inRoot(async () => {
            const { ui } = mk({ onTakeOver: vi.fn(async () => {}) });
            await flush();
            fire(fail("live_elsewhere"));
            click(ui, "Take over");
            vi.advanceTimersByTime(1000);
            fire(fail("live_elsewhere"));
            vi.advanceTimersByTime(2000);
            click(ui, "Take over");
            vi.advanceTimersByTime(2500);
            expect(action(ui, "Confirm take over")).toBeDefined();
        });
    });

    it("shows Taking over… and ignores further clicks while the request runs", async () => {
        const d = deferred();
        const onTakeOver = vi.fn(() => d.promise);
        await inRoot(async () => {
            const { ui } = mk({ onTakeOver });
            await flush();
            fire(fail("live_elsewhere"));
            click(ui, "Take over");
            click(ui, "Confirm take over");
            expect(onTakeOver).toHaveBeenCalledTimes(1);
            expect(action(ui, "Taking over…")!.disabled).toBe(true);
            click(ui, "Taking over…");
            expect(onTakeOver).toHaveBeenCalledTimes(1);
            d.reject(new Error("nope"));
            await flush();
            expect(action(ui, "Take over")).toBeDefined();
        });
    });

    it("on success ends the episode", async () => {
        await inRoot(async () => {
            const { ui, cleared } = mk({ onTakeOver: vi.fn(async () => {}) });
            await flush();
            fire(fail("live_elsewhere"));
            click(ui, "Take over");
            click(ui, "Confirm take over");
            await flush();
            expect(ui.row()).toBeNull();
            expect(cleared()).toEqual(["episode-ended"]);
        });
    });

    it("keeps the row on failure, saying why, as the row's own source and the time it failed", async () => {
        const d = deferred();
        await inRoot(async () => {
            const { ui, calls, external } = mk({ onTakeOver: vi.fn(() => d.promise) });
            await flush();
            external(fail("live_elsewhere", { title: "Elsewhere", detail: "other instance" }), { source: "seed" });
            click(ui, "Take over");
            click(ui, "Confirm take over");
            vi.advanceTimersByTime(321);
            d.reject(new Error("boom"));
            await flush();
            const last = calls().at(-1)!;
            expect(last).toMatchObject({ type: "FailureObserved", source: "seed", at: T0 + 321 });
            expect(last.failure).toMatchObject({
                code: "live_elsewhere",
                title: "Could not take over",
                detail: "boom",
            });
            expect(ui.row()!.title).toBe("Could not take over");
            expect(ui.row()!.meta).toContain("live_elsewhere");
            expect(action(ui, "Take over")).toBeDefined();
        });
    });

    it("says a thrown non-error as its text", async () => {
        const d = deferred();
        await inRoot(async () => {
            const { ui, external } = mk({ onTakeOver: vi.fn(() => d.promise) });
            await flush();
            external(fail("live_elsewhere"));
            click(ui, "Take over");
            click(ui, "Confirm take over");
            d.reject("plain words");
            await flush();
            expect(ui.row()!.detail).toBe("plain words");
        });
    });

    it("brings the row back when the resend inside cleared it", async () => {
        const d = deferred();
        await inRoot(async () => {
            const { ui, model, external } = mk({ onTakeOver: vi.fn(() => d.promise) });
            await flush();
            external(fail("live_elsewhere"));
            click(ui, "Take over");
            click(ui, "Confirm take over");
            model.dispatchPane({ type: "FailureCleared" });
            expect(ui.row()).toBeNull();
            d.reject(new Error("late"));
            await flush();
            expect(ui.row()!.title).toBe("Could not take over");
            expect(ui.row()!.detail).toBe("late");
        });
    });

    it("writes its message over whatever failure is showing when the request fails", async () => {
        const d = deferred();
        await inRoot(async () => {
            const { ui, external } = mk({ onTakeOver: vi.fn(() => d.promise) });
            await flush();
            external(fail("live_elsewhere"));
            click(ui, "Take over");
            click(ui, "Confirm take over");
            external(fail("killed"));
            d.reject(new Error("late"));
            await flush();
            expect(ui.row()!.meta).toContain("killed");
            expect(ui.row()!.title).toBe("Could not take over");
        });
    });

    it.each([
        [undefined, true],
        [true, true],
        [false, false],
    ])("passes turnAttempted %s on as %s", async (turnAttempted, expected) => {
        const onTakeOver = vi.fn<(t: boolean) => Promise<void>>(async () => {});
        await inRoot(async () => {
            const { ui, external } = mk({ onTakeOver });
            await flush();
            external(fail("live_elsewhere"), { turnAttempted });
            click(ui, "Take over");
            click(ui, "Confirm take over");
            expect(onTakeOver).toHaveBeenCalledWith(expected);
        });
    });

    it("offers no Take over without a handler", async () => {
        await inRoot(async () => {
            const { ui } = mk();
            await flush();
            fire(fail("live_elsewhere"));
            expect(labels(ui).some((l) => /take over/i.test(String(l)))).toBe(false);
        });
    });

    describe("open in another pane", () => {
        const held = () => fail("open_in_other_pane", { holderBlockId: "holder-1" });

        it("offers Go there with the holder, and Move here, each only with its handler", async () => {
            await inRoot(async () => {
                const onGoThere = vi.fn();
                const { ui } = mk({ onGoThere, onMoveHere: vi.fn(async () => {}) });
                await flush();
                fire(held());
                click(ui, "Go there");
                expect(onGoThere).toHaveBeenCalledWith("holder-1");
                expect(action(ui, "Move here")).toBeDefined();
            });
            await inRoot(async () => {
                const { ui } = mk({ onMoveHere: vi.fn(async () => {}) });
                await flush();
                fire(held());
                expect(action(ui, "Go there")).toBeUndefined();
                expect(action(ui, "Move here")).toBeDefined();
            });
            await inRoot(async () => {
                const { ui } = mk({ onGoThere: vi.fn() });
                await flush();
                fire(held());
                expect(action(ui, "Move here")).toBeUndefined();
                expect(action(ui, "Go there")).toBeDefined();
            });
        });

        it("offers no Go there when the holder is not known", async () => {
            await inRoot(async () => {
                const { ui } = mk({ onGoThere: vi.fn() });
                await flush();
                fire(fail("open_in_other_pane"));
                expect(action(ui, "Go there")).toBeUndefined();
            });
        });

        it("Move here takes two clicks, then runs the move", async () => {
            const onMoveHere = vi.fn(async () => {});
            await inRoot(async () => {
                const { ui } = mk({ onMoveHere });
                await flush();
                fire(held());
                click(ui, "Move here");
                expect(onMoveHere).not.toHaveBeenCalled();
                click(ui, "Confirm move here");
                expect(onMoveHere).toHaveBeenCalledTimes(1);
                await flush();
                expect(ui.row()).toBeNull();
            });
        });

        it("a failed move says so in its own words", async () => {
            await inRoot(async () => {
                const { ui } = mk({ onMoveHere: vi.fn(async () => Promise.reject(new Error("stuck"))) });
                await flush();
                fire(held());
                click(ui, "Move here");
                click(ui, "Confirm move here");
                await flush();
                expect(ui.row()!.title).toBe("Could not move it here");
                expect(ui.row()!.detail).toBe("stuck");
            });
        });

        it("says in the meta what was not sent", async () => {
            rememberRefusedSend(BLOCK_ID, { text: "please do the thing", attachments: [] });
            await inRoot(async () => {
                const { ui } = mk({ onMoveHere: vi.fn(async () => {}) });
                await flush();
                fire(held());
                expect(ui.row()!.meta).toContain('not sent: "please do the thing"');
            });
        });

        it("says nothing about a kept message on any other failure", async () => {
            rememberRefusedSend(BLOCK_ID, { text: "please do the thing", attachments: [] });
            await inRoot(async () => {
                const { ui } = mk();
                await flush();
                fire(fail("killed"));
                expect(ui.row()!.meta).not.toContain("not sent");
            });
        });
    });
});

describe("Copy error", () => {
    const copied = async (data: AgentFailure, blockId = "abcdefghijkl") => {
        let text = "";
        writeTextMock.mockClear();
        await inRoot(async () => {
            const { ui } = mk({ blockId });
            await flush();
            fire(data);
            click(ui, data.stderrTail ? "Copy details" : "Copy error");
            await flush();
            text = writeTextMock.mock.calls.at(-1)![0];
        });
        return text;
    };

    it("reports the title, the class with signal, retryable, the detail and the stderr tail", async () => {
        const f = fail("unknown_non_zero", {
            title: "Run failed",
            detail: "it died",
            signal: 9,
            exitCode: 1,
            retryable: true,
            stderrTail: "tail text",
        });
        expect(await copied(f)).toBe(
            formatErrorReport({
                title: "Run failed",
                message: "unknown_non_zero · signal 9 · retryable",
                where: "Agent failure · pane abcdefgh",
                details: "it died\n\ntail text",
            })
        );
    });

    it("reports a zero exit code and a zero signal", async () => {
        expect(await copied(fail("killed", { exitCode: 0, detail: "" }))).toBe(
            formatErrorReport({
                title: "T-killed",
                message: "killed · exit 0",
                where: "Agent failure · pane abcdefgh",
                details: undefined,
            })
        );
        expect(await copied(fail("killed", { signal: 0, exitCode: 3, detail: "" }))).toBe(
            formatErrorReport({
                title: "T-killed",
                message: "killed · signal 0",
                where: "Agent failure · pane abcdefgh",
                details: undefined,
            })
        );
    });

    it("names an untitled failure, and leaves the details out when there are none", async () => {
        const text = await copied(fail("killed", { title: "", detail: "" }));
        expect(text).toBe(
            formatErrorReport({
                title: "Agent run failed",
                message: "killed",
                where: "Agent failure · pane abcdefgh",
                details: undefined,
            })
        );
        expect(text).not.toContain("Details:");
    });

    it("puts the detail alone, or the tail alone, when only one is there", async () => {
        expect(await copied(fail("killed", { detail: "only detail" }))).toContain("Details:\nonly detail");
        expect(await copied(fail("killed", { detail: "", stderrTail: "only tail" }))).toContain("Details:\nonly tail");
    });

    it("holds Copied for two seconds, and a second click restarts the clock", async () => {
        await inRoot(async () => {
            const { ui } = mk();
            await flush();
            fire(fail("killed"));
            click(ui, "Copy error");
            await flush();
            vi.advanceTimersByTime(1999);
            expect(action(ui, "Copied")).toBeDefined();
            vi.advanceTimersByTime(1);
            expect(action(ui, "Copy error")).toBeDefined();
            click(ui, "Copy error");
            await flush();
            vi.advanceTimersByTime(1500);
            click(ui, "Copied");
            await flush();
            vi.advanceTimersByTime(600);
            expect(action(ui, "Copied")).toBeDefined();
        });
    });

    it("shows Copy failed when the write fails, and goes back to Copy error after two seconds", async () => {
        writeTextMock.mockRejectedValue(new Error("no bridge"));
        document.execCommand = vi.fn().mockReturnValue(false);
        await inRoot(async () => {
            const { ui } = mk();
            await flush();
            fire(fail("killed"));
            click(ui, "Copy error");
            await flush();
            expect(action(ui, "Copy failed")).toBeDefined();
            vi.advanceTimersByTime(2000);
            expect(action(ui, "Copy error")).toBeDefined();
        });
    });

    it("a stale button copies nothing once the failure is gone", async () => {
        await inRoot(async () => {
            const { ui } = mk();
            await flush();
            fire(fail("killed"));
            const stale = action(ui, "Copy error")!;
            click(ui, "Dismiss");
            stale.onClick!(undefined as unknown as MouseEvent);
            await flush();
            expect(writeTextMock).not.toHaveBeenCalled();
        });
    });

    it("a late result does not leave a label on the next failure after the copy was reset", async () => {
        await inRoot(async () => {
            const { ui } = mk();
            await flush();
            fire(fail("killed"));
            click(ui, "Copy error");
            await flush();
            vi.advanceTimersByTime(500);
            fire(fail("no_output"));
            click(ui, "Copy error");
            await flush();
            vi.advanceTimersByTime(1600);
            expect(action(ui, "Copied")).toBeDefined();
        });
    });
});

describe("the row hands each action to its caller", () => {
    it("Details toggles the body", async () => {
        await inRoot(async () => {
            const { ui } = mk();
            await flush();
            fire(fail("killed"));
            click(ui, "Details");
            expect(ui.row()!.expanded).toBe(true);
            expect(action(ui, "Hide details")).toBeDefined();
            click(ui, "Hide details");
            expect(ui.row()!.expanded).toBe(false);
        });
    });

    it("Login Again and Login via terminal pass turnAttempted on, defaulting to true", async () => {
        for (const [turnAttempted, expected] of [
            [undefined, true],
            [false, false],
        ] as const) {
            await inRoot(async () => {
                const { ui, external, onLoginAgain, onLoginViaTerminal } = mk();
                await flush();
                external(fail("auth"), { turnAttempted });
                click(ui, expected ? "Login Again" : "Log in");
                click(ui, "Login via terminal");
                expect(onLoginAgain).toHaveBeenCalledWith(expected);
                expect(onLoginViaTerminal).toHaveBeenCalledWith(expected);
            });
        }
    });

    it("Connectors → Accounts opens the accounts", async () => {
        await inRoot(async () => {
            const { ui, onOpenAccounts } = mk();
            await flush();
            fire(fail("auth"));
            click(ui, "Connectors → Accounts");
            expect(onOpenAccounts).toHaveBeenCalledTimes(1);
        });
    });

    it("offers a bind in place of Connectors when accounts could be bound, and runs the handler", async () => {
        await inRoot(async () => {
            const onBindAccount = vi.fn();
            const { ui } = mk({ bindCandidates: () => [{ id: "a1", name: "Work" }], onBindAccount });
            await flush();
            fire(fail("auth"));
            expect(action(ui, "Connectors → Accounts")).toBeUndefined();
            const ev = {} as MouseEvent;
            action(ui, "Use Work")!.onClick!(ev);
            expect(onBindAccount).toHaveBeenCalledWith(ev);
        });
    });

    it("a bind with no handler wired does nothing", async () => {
        await inRoot(async () => {
            const { ui } = mk({ bindCandidates: () => [{ id: "a1", name: "Work" }] });
            await flush();
            fire(fail("auth"));
            expect(() => click(ui, "Use Work")).not.toThrow();
        });
    });

    it("New session starts a fresh session", async () => {
        await inRoot(async () => {
            const { ui, onNewSession } = mk();
            await flush();
            fire(fail("context_exceeded"));
            click(ui, "New session");
            expect(onNewSession).toHaveBeenCalledTimes(1);
        });
    });

    it("adds the recovery's hint to the meta", async () => {
        await inRoot(async () => {
            const reconnect = {
                waitsForReconnect: () => true,
                reconnectHint: () => "retries when the API is back",
                retry: () => false,
                retryBusy: () => false,
                failureDismissed: vi.fn(),
            };
            const { ui } = mk({ reconnect });
            await flush();
            fire(fail("network"));
            expect(ui.row()!.meta).toContain("retries when the API is back");
        });
    });
});

describe("the verdict", () => {
    const usageLimit = (resetsAtMs: number) => fail("usage_limit", { resetsAtMs });

    it("a usage limit is healed one second after its reset passes, with no event to say so", async () => {
        await inRoot(async () => {
            const { ui, external } = mk();
            await flush();
            external(usageLimit(T0 + 30_000));
            expect(ui.verdict()).toBe("active");
            vi.advanceTimersByTime(30_999);
            expect(ui.row()).not.toBeNull();
            vi.advanceTimersByTime(1);
            expect(ui.row()).toBeNull();
        });
    });

    it("a healed failure leaves as the end of an episode", async () => {
        await inRoot(async () => {
            const { external, cleared } = mk();
            await flush();
            external(usageLimit(T0 + 1000));
            vi.advanceTimersByTime(2000);
            expect(cleared()).toEqual(["episode-ended"]);
        });
    });

    it("schedules a look only for a reset that is ahead and can be timed", async () => {
        const timers = async (resetIn: number | undefined) => {
            let count = -1;
            await inRoot(async () => {
                const { external } = mk();
                await flush();
                external(fail("usage_limit", { resetsAtMs: resetIn === undefined ? undefined : T0 + resetIn }));
                count = vi.getTimerCount();
            });
            return count;
        };
        expect(await timers(undefined)).toBe(0);
        expect(await timers(2 ** 31 - 1)).toBe(1);
        expect(await timers(2 ** 31)).toBe(0);
        expect(await timers(2 ** 30 + 10)).toBe(1);
    });

    it("drops the look at the old reset when the failure is replaced", async () => {
        await inRoot(async () => {
            const { external } = mk();
            await flush();
            external(usageLimit(T0 + 100_000));
            external(usageLimit(T0 + 200_000));
            expect(vi.getTimerCount()).toBe(1);
        });
    });

    it("judges a stored failure that arrives later by the time it was recorded, not the pane's start", async () => {
        await inRoot(async () => {
            const { ui, external } = mk({ liveAccount: () => "can_work" });
            await flush();
            external(fail("auth"), { at: 1 });
            expect(ui.verdict()).toBe("active");
        });
    });

    it("a failure that arrived in this pane is not judged by an account state that never changed", async () => {
        await inRoot(async () => {
            const { ui, external } = mk({ liveAccount: () => "can_work" });
            await flush();
            external(fail("auth"));
            expect(ui.verdict()).toBe("active");
            expect(ui.row()).not.toBeNull();
        });
    });

    it("a transient failure shown by other means is active until the ladder is spent", async () => {
        await inRoot(async () => {
            const { ui, external } = mk();
            await flush();
            external(transient());
            expect(ui.verdict()).toBe("active");
        });
    });

    it("follows the account once it changes after the failure arrived", async () => {
        await inRoot(async () => {
            const [account, setAccount] = createSignal<AccountState>("signed_out");
            const { ui, external } = mk({ liveAccount: account as Accessor<AccountState> });
            await flush();
            vi.advanceTimersByTime(10);
            external(fail("auth"));
            expect(ui.verdict()).toBe("active");
            vi.advanceTimersByTime(10);
            setAccount("can_work");
            expect(ui.row()).toBeNull();
        });
    });

    it("an account change before the failure arrived does not heal it", async () => {
        await inRoot(async () => {
            const [account, setAccount] = createSignal<AccountState>("signed_out");
            const { ui, external } = mk({ liveAccount: account as Accessor<AccountState> });
            await flush();
            setAccount("can_work");
            vi.advanceTimersByTime(10);
            external(fail("auth"));
            expect(ui.verdict()).toBe("active");
        });
    });
});

describe("what a pane starts with and gives back", () => {
    it("a stored failure shows as a collapsed row with every action ready", async () => {
        hub.persistedFailure = fail("live_elsewhere");
        await inRoot(async () => {
            const { ui } = mk({ onTakeOver: vi.fn(async () => {}) });
            await flush();
            const row = ui.row()!;
            expect(row.expanded).toBe(false);
            expect(action(ui, "Details")).toBeDefined();
            expect(action(ui, "Take over")!.disabled).toBe(false);
            expect(action(ui, "Copy error")).toBeDefined();
        });
        hub.persistedFailure = fail("killed");
        await inRoot(async () => {
            const { ui } = mk();
            await flush();
            expect(action(ui, "Retry")!.disabled).toBeFalsy();
        });
    });

    it("gives back its listeners, recycle blocker and timers when the pane goes away", async () => {
        await inRoot(async (dispose) => {
            const { ui } = mk({ onTakeOver: vi.fn(async () => {}) });
            await flush();
            expect(hub.listeners).toBe(1);
            expect(hub.blockers).toBe(1);
            fire(fail("live_elsewhere"));
            click(ui, "Take over");
            click(ui, "Copy error");
            await flush();
            expect(vi.getTimerCount()).toBe(2);
            dispose();
            expect(hub.listeners).toBe(0);
            expect(hub.blockers).toBe(0);
            expect(vi.getTimerCount()).toBe(0);
        });
    });

    it("a Retry leaves no Copied label on the next row", async () => {
        await inRoot(async () => {
            const { ui, external } = mk();
            await flush();
            fire(fail("killed"));
            click(ui, "Copy error");
            await flush();
            click(ui, "Retry");
            external(fail("killed"));
            expect(action(ui, "Copy error")).toBeDefined();
        });
    });

    it("a finished take over leaves the next row ready to be taken over", async () => {
        await inRoot(async () => {
            const { ui, external } = mk({ onTakeOver: vi.fn(async () => {}) });
            await flush();
            fire(fail("live_elsewhere"));
            click(ui, "Take over");
            click(ui, "Confirm take over");
            await flush();
            external(fail("live_elsewhere"));
            expect(action(ui, "Take over")!.disabled).toBe(false);
        });
    });
});

describe("the verdict, further", () => {
    it("follows every change of the account, not only the first", async () => {
        await inRoot(async () => {
            const [account, setAccount] = createSignal<AccountState>("can_work");
            const { ui, external } = mk({ liveAccount: account as Accessor<AccountState> });
            await flush();
            vi.advanceTimersByTime(10);
            setAccount("signed_out");
            vi.advanceTimersByTime(10);
            external(fail("auth"));
            vi.advanceTimersByTime(10);
            setAccount("can_work");
            expect(ui.row()).toBeNull();
        });
    });

    it("an account change in the same millisecond as the failure does not heal it", async () => {
        await inRoot(async () => {
            const [account, setAccount] = createSignal<AccountState>("signed_out");
            const { ui, external } = mk({ liveAccount: account as Accessor<AccountState> });
            await flush();
            external(fail("auth"));
            setAccount("can_work");
            expect(ui.verdict()).toBe("active");
        });
    });

    it("times no look at a reset that is not ahead of a failure it cannot end", async () => {
        const timers = async (resetsAtMs: number) => {
            let count = -1;
            await inRoot(async () => {
                const { external } = mk();
                await flush();
                external(fail("killed", { resetsAtMs }));
                count = vi.getTimerCount();
            });
            return count;
        };
        expect(await timers(T0)).toBe(0);
        expect(await timers(T0 - 5000)).toBe(0);
        expect(await timers(T0 + 5000)).toBe(1);
    });
});
