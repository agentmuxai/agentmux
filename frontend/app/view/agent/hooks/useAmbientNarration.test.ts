// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { createRoot } from "solid-js";
import { beforeEach, describe, expect, it, vi } from "vitest";

const handlers: Array<(ev: unknown) => void> = [];
const unsubs = vi.fn();

vi.mock("@/app/store/mps", () => ({
    muxEventSubscribe: (opts: { handler: (ev: unknown) => void }) => {
        handlers.push(opts.handler);
        return unsubs;
    },
}));

// Static import, not `await import(...)`: vitest hoists `vi.mock` above the
// import graph, so the mock above is already in place — and a top-level await
// is a type error under this tsconfig's module target (TS1378).
import type { AmbientNarrationNode } from "../types";
import { useAmbientNarration } from "./useAmbientNarration";

const emit = (data: unknown) => handlers.forEach((h) => h({ data }));

describe("useAmbientNarration", () => {
    beforeEach(() => {
        handlers.length = 0;
        unsubs.mockClear();
    });

    const run = (fn: (got: AmbientNarrationNode[]) => void) =>
        createRoot((dispose) => {
            const got: AmbientNarrationNode[] = [];
            useAmbientNarration("block-1", (n) => got.push(n));
            fn(got);
            dispose();
        });

    it("hands each narration from the broadcast to the callback as an ambient_narration node", () => {
        run((got) => {
            emit({ kind: "background_task", text: "Running task dev in the background." });
            expect(got).toHaveLength(1);
            expect(got[0].type).toBe("ambient_narration");
            expect(got[0].text).toBe("Running task dev in the background.");
            expect(got[0].kind).toBe("background_task");
            expect(got[0].timestamp).toBeGreaterThan(0);
        });
    });

    it("gives every narration a distinct id, even within the same millisecond", () => {
        // The document reducer dedups by id — a collision would silently drop the
        // second of two narrations that arrive back to back.
        run((got) => {
            for (let i = 0; i < 5; i++) emit({ kind: "background_task", text: `line ${i}` });
            expect(new Set(got.map((n) => n.id)).size).toBe(5);
        });
    });

    it("ignores an empty or whitespace-only line rather than rendering a blank row", () => {
        run((got) => {
            emit({ kind: "background_task", text: "   " });
            emit({ kind: "background_task", text: "" });
            expect(got).toEqual([]);
        });
    });

    it("ignores a malformed payload instead of throwing into the event handler", () => {
        // This runs inside a WS event dispatch — a throw here would take out
        // whatever else that dispatch was delivering.
        run((got) => {
            expect(() => {
                emit(undefined);
                emit({});
                emit({ text: 42 });
            }).not.toThrow();
            expect(got).toEqual([]);
        });
    });

    it("keeps every narration in order — retention is the document store's job, not the hook's", () => {
        run((got) => {
            for (let i = 0; i < 12; i++) emit({ kind: "background_task", text: `line ${i}` });
            expect(got).toHaveLength(12);
            expect(got.map((n) => n.text)).toEqual(Array.from({ length: 12 }, (_, i) => `line ${i}`));
        });
    });

    it("unsubscribes on dispose", () => {
        createRoot((dispose) => {
            useAmbientNarration("block-1", () => {});
            dispose();
        });
        expect(unsubs).toHaveBeenCalled();
    });
});
