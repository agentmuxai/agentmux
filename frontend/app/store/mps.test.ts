// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Regression tests for the resubscribe payload (issue #3482).
 *
 * `makeMuxReSubCommand` rebuilds the whole `scopes` array from every live
 * subscriber and re-sends it on every subscribe. Without dedup, a caller that
 * repeatedly subscribes to one scope makes each message larger than the last —
 * live, that reached a 2.9 MB message re-sent ~13x/sec and crashed the app.
 */

import { beforeEach, describe, expect, it, vi } from "vitest";

const { sent } = vi.hoisted(() => ({ sent: [] as any[] }));

vi.mock("./ws", () => ({
    sendRawRpcMessage: (msg: any) => {
        sent.push(msg);
    },
}));

import { muxEventSubscribe } from "./mps";

/** The scopes of the most recently sent eventsub command. */
function lastScopes(): string[] {
    const last = sent[sent.length - 1];
    return last?.data?.scopes ?? [];
}

function lastCommand(): any {
    return sent[sent.length - 1];
}

beforeEach(() => {
    sent.length = 0;
});

describe("makeMuxReSubCommand scopes", () => {
    it("collapses repeated subscriptions to the same scope into one entry", () => {
        const scope = "block:86aa8882-603a-4dfa-8bed-db2b4c75f84c";
        const unsubs = [];
        for (let i = 0; i < 50; i++) {
            unsubs.push(
                muxEventSubscribe({ eventType: "test:dup", scope, handler: () => {} }),
            );
        }

        // Without dedup this array would hold 50 identical strings, and the
        // payload would have grown on every one of those 50 sends.
        expect(lastScopes()).toEqual([scope]);

        unsubs.forEach((u) => u());
    });

    it("keeps distinct scopes, so dedup does not narrow what is subscribed", () => {
        const a = "block:aaaa1111-0000-0000-0000-000000000000";
        const b = "block:bbbb2222-0000-0000-0000-000000000000";
        const u1 = muxEventSubscribe({ eventType: "test:distinct", scope: a, handler: () => {} });
        const u2 = muxEventSubscribe({ eventType: "test:distinct", scope: b, handler: () => {} });
        // A duplicate of one of them must not drop the other.
        const u3 = muxEventSubscribe({ eventType: "test:distinct", scope: a, handler: () => {} });

        expect(lastScopes().slice().sort()).toEqual([a, b].sort());

        u1();
        u2();
        u3();
    });

    it("still promotes a blank scope to allscopes", () => {
        const u1 = muxEventSubscribe({
            eventType: "test:allscopes",
            scope: "block:cccc3333-0000-0000-0000-000000000000",
            handler: () => {},
        });
        const u2 = muxEventSubscribe({ eventType: "test:allscopes", handler: () => {} });

        expect(lastCommand().data.allscopes).toBe(true);
        expect(lastScopes()).toEqual([]);

        u1();
        u2();
    });

    it("drops a scope from the payload once its last subscriber unsubscribes", () => {
        const a = "block:dddd4444-0000-0000-0000-000000000000";
        const b = "block:eeee5555-0000-0000-0000-000000000000";
        const u1 = muxEventSubscribe({ eventType: "test:unsub", scope: a, handler: () => {} });
        const u2 = muxEventSubscribe({ eventType: "test:unsub", scope: b, handler: () => {} });
        expect(lastScopes().slice().sort()).toEqual([a, b].sort());

        u2();
        expect(lastScopes()).toEqual([a]);

        u1();
    });

    it("warns once when subscribers for one event type run away", () => {
        const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
        const scope = "block:ffff6666-0000-0000-0000-000000000000";
        const unsubs = [];
        // The threshold is 200; go comfortably past it.
        for (let i = 0; i < 260; i++) {
            unsubs.push(
                muxEventSubscribe({ eventType: "test:leak", scope, handler: () => {} }),
            );
        }

        expect(warn).toHaveBeenCalledTimes(1);
        expect(String(warn.mock.calls[0][0])).toContain("test:leak");
        // The payload stayed bounded the whole time despite the leak.
        expect(lastScopes()).toEqual([scope]);

        unsubs.forEach((u) => u());
        warn.mockRestore();
    });
});
