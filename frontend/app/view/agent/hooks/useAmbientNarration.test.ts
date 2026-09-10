// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { createRoot } from "solid-js";
import { beforeEach, describe, expect, it, vi } from "vitest";

const handlers: Array<(ev: unknown) => void> = [];
const unsubs = vi.fn();

vi.mock("@/app/store/wps", () => ({
    waveEventSubscribe: (opts: { handler: (ev: unknown) => void }) => {
        handlers.push(opts.handler);
        return unsubs;
    },
}));

// Static import, not `await import(...)`: vitest hoists `vi.mock` above the
// import graph, so the mock above is already in place — and a top-level await
// is a type error under this tsconfig's module target (TS1378).
import { useAmbientNarration } from "./useAmbientNarration";

const emit = (data: unknown) => handlers.forEach((h) => h({ data }));

describe("useAmbientNarration", () => {
    beforeEach(() => {
        handlers.length = 0;
        unsubs.mockClear();
    });

    it("collects narrations from the broadcast", () => {
        createRoot((dispose) => {
            const n = useAmbientNarration("block-1");
            expect(n()).toEqual([]);
            emit({ kind: "background_task", text: "Running task dev in the background." });
            expect(n()).toHaveLength(1);
            expect(n()[0].text).toBe("Running task dev in the background.");
            expect(n()[0].kind).toBe("background_task");
            dispose();
        });
    });

    it("ignores an empty or whitespace-only line rather than rendering a blank row", () => {
        createRoot((dispose) => {
            const n = useAmbientNarration("block-1");
            emit({ kind: "background_task", text: "   " });
            emit({ kind: "background_task", text: "" });
            expect(n()).toEqual([]);
            dispose();
        });
    });

    it("ignores a malformed payload instead of throwing into the event handler", () => {
        // This runs inside a WS event dispatch — a throw here would take out
        // whatever else that dispatch was delivering.
        createRoot((dispose) => {
            const n = useAmbientNarration("block-1");
            expect(() => {
                emit(undefined);
                emit({});
                emit({ text: 42 });
            }).not.toThrow();
            expect(n()).toEqual([]);
            dispose();
        });
    });

    it("retains only the most recent few", () => {
        // These are transient asides, not a log. An unbounded list on a
        // long-lived pane grows with nothing ever pruning it, and old entries
        // describe work that finished long ago.
        createRoot((dispose) => {
            const n = useAmbientNarration("block-1");
            for (let i = 0; i < 12; i++) emit({ kind: "background_task", text: `line ${i}` });
            expect(n()).toHaveLength(5);
            expect(n()[0].text).toBe("line 7");
            expect(n()[4].text).toBe("line 11");
            dispose();
        });
    });

    it("unsubscribes on dispose", () => {
        createRoot((dispose) => {
            useAmbientNarration("block-1");
            dispose();
        });
        expect(unsubs).toHaveBeenCalled();
    });
});
