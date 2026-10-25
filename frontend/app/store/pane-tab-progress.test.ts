// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { createEffect, createRoot, createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const h = vi.hoisted(() => ({
    metaSignal: null as null | [() => Record<string, unknown> | undefined, (m: Record<string, unknown> | undefined) => void],
    status: { is_agent_pane: true, turn_active: false } as { is_agent_pane?: boolean; turn_active?: boolean },
    listeners: [] as Array<{ scope: string; cb: (data: Record<string, unknown>) => void; unsub: ReturnType<typeof vi.fn> }>,
    getStatus: vi.fn(),
}));

vi.mock("@/app/store/global", () => ({
    MOS: {
        getMuxObjectAtom: () => () => ({ meta: h.metaSignal?.[0]() }),
        makeORef: (type: string, id: string) => `${type}:${id}`,
    },
}));
vi.mock("@/app/store/mps", () => ({
    onMuxEvent: (_event: string, cb: (data: Record<string, unknown>) => void, scope: string) => {
        const unsub = vi.fn();
        h.listeners.push({ scope, cb, unsub });
        return unsub;
    },
}));
vi.mock("@/app/store/mps-events", () => ({ WpsEvent: { ControllerStatus: "controllerstatus" } }));
vi.mock("@/app/store/services", () => ({ BlockService: { GetControllerStatus: (...a: unknown[]) => h.getStatus(...a) } }));

import { openProgressSubscriptions, progressOf, usePaneTabProgress } from "./pane-tab-progress";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "../../..");
type Row = {
    name: string;
    controller_type: string;
    turn_active: boolean;
    awaiting_user: boolean;
    failure: { code: string; resets_at_ms: number | null } | null;
    now_ms: number;
    expected: string;
};
const fixture: { rows: Row[] } = JSON.parse(readFileSync(join(ROOT, "test/fixtures/agent-state-decide.json"), "utf8"));

/** The controller types srv reports `is_agent_pane` for. */
const AGENT_CONTROLLERS = new Set(["persistent", "subprocess", "acp", "msp", "app-server"]);
const SHOWN = new Set(["working", "waiting", "limited", "signed_out"]);

function metaOf(row: Row): Record<string, unknown> {
    const meta: Record<string, unknown> = {};
    if (row.awaiting_user) meta["term:awaiting_user"] = true;
    if (row.failure) {
        meta["agent:last_failure"] = {
            code: row.failure.code,
            ...(row.failure.resets_at_ms != null ? { resetsAtMs: row.failure.resets_at_ms } : {}),
        };
    }
    return meta;
}

describe("the pill's progress follows srv's decide() rows", () => {
    it("has the shared rows", () => {
        expect(fixture.rows.length).toBeGreaterThanOrEqual(20);
    });

    for (const row of fixture.rows) {
        it(row.name, () => {
            const got = progressOf({
                isAgentPane: AGENT_CONTROLLERS.has(row.controller_type),
                turnActive: row.turn_active,
                meta: metaOf(row),
                now: row.now_ms,
            });
            // A pill shows working, waiting, limited and signed out; every other state shows nothing.
            expect(got?.state ?? null).toBe(SHOWN.has(row.expected) ? row.expected : null);
        });
    }
});

describe("the pill's words", () => {
    const now = new Date(2026, 9, 25, 12, 0).getTime();
    it("says each state in the Swarm chip's words", () => {
        const base = { isAgentPane: true, now };
        expect(progressOf({ ...base, turnActive: true, meta: {} })?.text).toBe("Working");
        expect(progressOf({ ...base, turnActive: true, meta: { "term:awaiting_user": true } })?.text).toBe("Waiting for you");
        expect(progressOf({ ...base, turnActive: false, meta: { "agent:last_failure": { code: "auth" } } })?.text).toBe("Signed out");
        const limited = progressOf({
            ...base,
            turnActive: false,
            meta: { "agent:last_failure": { code: "usage_limit", resetsAtMs: now + 3 * 3600_000 } },
        });
        expect(limited?.state).toBe("limited");
        expect(limited?.text).toMatch(/^Out of quota until /);
        expect(
            progressOf({ ...base, turnActive: false, meta: { "agent:last_failure": { code: "usage_limit" } } })?.text
        ).toBe("Out of quota");
    });
});

describe("the per-block subscription", () => {
    beforeEach(() => {
        vi.useFakeTimers();
        vi.setSystemTime(new Date(2026, 9, 25, 12, 0));
        h.listeners.length = 0;
        h.status = { is_agent_pane: true, turn_active: true };
        h.getStatus.mockImplementation(() => Promise.resolve(h.status));
        const [meta, setMeta] = createSignal<Record<string, unknown> | undefined>({});
        h.metaSignal = [meta, setMeta];
    });
    afterEach(() => vi.useRealTimers());

    it("is seeded from the controller status, follows events, and is shared and closed with its last user", async () => {
        const values: Array<string | undefined> = [];
        let disposeA!: () => void;
        let disposeB!: () => void;
        let a!: ReturnType<typeof usePaneTabProgress>;
        createRoot((d) => {
            disposeA = d;
            a = usePaneTabProgress("blk-1");
        });
        createRoot((d) => {
            disposeB = d;
            usePaneTabProgress("blk-1");
        });
        expect(openProgressSubscriptions()).toBe(1);
        expect(h.listeners).toHaveLength(1);
        expect(h.listeners[0].scope).toBe("block:blk-1");
        expect(h.getStatus).toHaveBeenCalledTimes(1);

        await vi.advanceTimersByTimeAsync(0);
        values.push(a()?.state);
        h.listeners[0].cb({ is_agent_pane: true }); // a turn-end event omits the false turn_active
        values.push(a()?.state);
        h.listeners[0].cb({ is_agent_pane: true, turn_active: true });
        values.push(a()?.state);
        h.listeners[0].cb({}); // a terminal pane's event
        values.push(a()?.state);
        expect(values).toEqual(["working", undefined, "working", undefined]);

        disposeA();
        expect(openProgressSubscriptions()).toBe(1);
        expect(h.listeners[0].unsub).not.toHaveBeenCalled();
        disposeB();
        expect(openProgressSubscriptions()).toBe(0);
        expect(h.listeners[0].unsub).toHaveBeenCalledTimes(1);
    });

    it("does not notify when a status event leaves the pill as it was", async () => {
        let runs = 0;
        let dispose!: () => void;
        createRoot((d) => {
            dispose = d;
            const progress = usePaneTabProgress("blk-2");
            createEffect(() => {
                progress();
                runs++;
            });
        });
        await vi.advanceTimersByTimeAsync(0); // the seed: working
        const afterSeed = runs;
        h.listeners[0].cb({ is_agent_pane: true, turn_active: true });
        h.listeners[0].cb({ is_agent_pane: true, turn_active: true });
        expect(runs).toBe(afterSeed);
        h.listeners[0].cb({ is_agent_pane: true });
        expect(runs).toBe(afterSeed + 1);
        dispose();
    });

    it("ends the limited line at the reset, with no event", async () => {
        h.status = { is_agent_pane: true, turn_active: false };
        const resetsAt = Date.now() + 60_000;
        h.metaSignal![1]({ "agent:last_failure": { code: "usage_limit", resetsAtMs: resetsAt } });
        let dispose!: () => void;
        let progress!: ReturnType<typeof usePaneTabProgress>;
        createRoot((d) => {
            dispose = d;
            progress = usePaneTabProgress("blk-3");
        });
        await vi.advanceTimersByTimeAsync(0);
        expect(progress()?.state).toBe("limited");
        await vi.advanceTimersByTimeAsync(59_000);
        expect(progress()?.state).toBe("limited");
        await vi.advanceTimersByTimeAsync(2_000);
        expect(progress()).toBeNull();
        dispose();
    });

    it("clears the signed-out line when the failure is cleared", async () => {
        h.status = { is_agent_pane: true, turn_active: false };
        h.metaSignal![1]({ "agent:last_failure": { code: "auth" } });
        let dispose!: () => void;
        let progress!: ReturnType<typeof usePaneTabProgress>;
        createRoot((d) => {
            dispose = d;
            progress = usePaneTabProgress("blk-4");
        });
        await vi.advanceTimersByTimeAsync(0);
        expect(progress()?.state).toBe("signed_out");
        h.metaSignal![1]({});
        expect(progress()).toBeNull();
        dispose();
    });
});
