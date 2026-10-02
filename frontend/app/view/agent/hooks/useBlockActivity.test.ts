// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * What a session end does to the block meta the Swarm row reads
 * (store/swarm-line.ts): the live title is kept as `term:restored_summary`, and
 * everything that belonged to the finished session is cleared. ReAgent P2 on
 * #4234: `term:last_prompt` was left behind, so a fresh session opened under the
 * old session's last message.
 */

import { createRoot } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const hub = vi.hoisted(() => ({
    updates: [] as Array<Record<string, unknown>>,
    blockMeta: {} as Record<string, unknown>,
    handlers: [] as Array<{ eventType: string; handler: (e: unknown) => void }>,
}));

vi.mock("@/app/store/mps", () => ({
    muxEventSubscribe: (sub: { eventType: string; handler: (e: unknown) => void }) => {
        hub.handlers.push(sub);
        return () => {};
    },
}));
vi.mock("@/app/store/services", () => ({
    ObjectService: {
        UpdateObjectMeta: (_oref: string, meta: Record<string, unknown>) => {
            hub.updates.push(meta);
            return Promise.resolve();
        },
    },
}));
vi.mock("@/app/store/global", () => ({
    MOS: { getMuxObjectAtom: () => () => ({ meta: hub.blockMeta }) },
}));

import { META_AWAITING_USER, META_LAST_PROMPT, META_RESTORED } from "@/app/store/swarm-line";
import { useBlockActivity } from "./useBlockActivity";

function endSession() {
    let dispose = () => {};
    createRoot((d) => {
        dispose = d;
        useBlockActivity({ blockId: "b1" });
    });
    const status = hub.handlers.find((h) => h.eventType === "controllerstatus");
    status!.handler({ data: { shellprocstatus: "done" } });
    dispose();
}

beforeEach(() => {
    hub.updates.length = 0;
    hub.handlers.length = 0;
    hub.blockMeta = {};
});
afterEach(() => vi.clearAllMocks());

describe("useBlockActivity at session end", () => {
    it("keeps the finished session's title as the restored one and clears the live one", () => {
        hub.blockMeta = { "term:ambient_summary": "Set up CI for the docs site" };
        endSession();
        expect(hub.updates).toHaveLength(1);
        expect(hub.updates[0]).toMatchObject({
            "term:ambient_summary": null,
            "term:osc_title": null,
            [META_RESTORED]: "Set up CI for the docs site",
        });
    });

    it("clears the finished session's last message and waiting flag", () => {
        hub.blockMeta = {
            "term:ambient_summary": "Set up CI for the docs site",
            [META_LAST_PROMPT]: "Please fix the login redirect",
            [META_AWAITING_USER]: true,
        };
        endSession();
        expect(hub.updates[0]).toMatchObject({ [META_LAST_PROMPT]: null, [META_AWAITING_USER]: null });
    });

    it("does not overwrite the restored title with a placeholder or nothing", () => {
        for (const ended of ["(none yet)", "", undefined]) {
            hub.updates.length = 0;
            hub.handlers.length = 0;
            hub.blockMeta = { "term:ambient_summary": ended, [META_RESTORED]: "Older real title" };
            endSession();
            expect(META_RESTORED in hub.updates[0], `ended=${String(ended)}`).toBe(false);
        }
    });
});
