// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it, vi } from "vitest";

vi.mock("@/app/store/global", () => ({ atoms: {}, MOS: {} }));
vi.mock("@/app/store/mps", () => ({ onMuxEvent: vi.fn() }));
vi.mock("@/app/store/mps-events", () => ({ WpsEvent: {} }));
vi.mock("@/app/store/rpc-api", () => ({ RpcApi: {} }));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));

import { answerAsk, type MaskBridgeDeps } from "./capture-mask-bridge";
import { blackedOutPane, CaptureWatch, measure, PRIVATE_PANELS, type MaskSource, type Measured } from "./capture-mask";
import type { WindowCaptureMask } from "@/types/rpc/WindowCaptureMask";

const r = (x: number, y: number, w: number, h: number) => ({ x, y, w, h });

function source(over: Partial<MaskSource> = {}, metas: Record<string, Record<string, unknown>> = {}): MaskSource {
    return {
        width: 1000,
        height: 600,
        scale: 1.25,
        panes: () => [
            { blockId: "vault", rect: r(0, 0, 400, 600) },
            { blockId: "agent", rect: r(400, 0, 600, 600) },
            { blockId: "hidden-memory", rect: r(0, 0, 0, 0) },
        ],
        panels: () => [],
        meta: (id) => metas[id] ?? { vault: { view: "vault" }, agent: { view: "agent" }, "hidden-memory": { view: "memory" } }[id],
        ...over,
    };
}

describe("which panes are blacked out", () => {
    it("is the Vault, the Audit, Memory and a browser in the person's own profile", () => {
        for (const view of ["vault", "audit", "memory"]) expect(blackedOutPane({ view })).toBe(true);
        expect(blackedOutPane({ view: "browser" })).toBe(true);
        expect(blackedOutPane({ view: "browser", "browser:identity": "profile:sorrel" })).toBe(false);
        expect(blackedOutPane({ view: "browser", "browser:identity": "incognito:0123456789ab" })).toBe(false);
    });

    it("keeps agent panes, terminals and editors capturable", () => {
        for (const view of ["agent", "term", "editor", "settings", "kanban"]) expect(blackedOutPane({ view })).toBe(false);
        expect(blackedOutPane(undefined)).toBe(false);
    });
});

describe("measure", () => {
    it("gives the rectangle of each blacked-out pane on screen, not the others", () => {
        expect(measure(source()).rects).toEqual([r(0, 0, 400, 600)]);
    });

    it("includes an overlay that shows private content, such as a Vault request card in an agent pane", () => {
        const m = measure(source({ panels: () => [r(420, 500, 300, 80)] }));
        expect(m.rects).toEqual([r(0, 0, 400, 600), r(420, 500, 300, 80)]);
    });

    it("clips to the viewport and drops what is off it", () => {
        const m = measure(source({ panes: () => [], panels: () => [r(900, 550, 300, 300), r(2000, 0, 10, 10)] }));
        expect(m.rects).toEqual([r(900, 550, 100, 50)]);
    });
});

describe("a capture's watch", () => {
    function frames() {
        const queue: (() => void)[] = [];
        return { next: (cb: () => void) => queue.push(cb), run: () => queue.splice(0).forEach((cb) => cb()) };
    }

    it("says nothing changed when every frame matched", () => {
        const f = frames();
        const w = new CaptureWatch(() => measure(source()), f.next, Infinity, () => 0);
        f.run();
        f.run();
        expect(w.end().changed).toBe(false);
    });

    it("says it changed when a Vault opened for one frame and closed again", () => {
        const f = frames();
        let open = false;
        const w = new CaptureWatch(() => measure(source({ panes: () => (open ? [{ blockId: "vault", rect: r(0, 0, 400, 600) }] : []) })), f.next, Infinity, () => 0);
        open = true;
        f.run();
        open = false;
        f.run();
        expect(w.end().changed).toBe(true);
    });
});

describe("the window's answer", () => {
    function deps(over: Partial<MaskBridgeDeps> = {}) {
        const answers: { phase: string; mask: WindowCaptureMask }[] = [];
        const d: MaskBridgeDeps = {
            now: () => 1000,
            windowId: () => "win-1",
            title: () => "AgentMux",
            read: (): Measured => measure(source()),
            nextFrame: () => {},
            refusal: () => undefined,
            answer: async (_id, phase, mask) => void answers.push({ phase, mask }),
            ...over,
        };
        return { d, answers };
    }

    it("answers begin and end with the rectangles, the size and the scale", async () => {
        const { d, answers } = deps();
        const watches = new Map();
        await answerAsk({ capture_id: "c1", phase: "begin", expires_at_ms: BigInt(5000) }, d, watches);
        await answerAsk({ capture_id: "c1", phase: "end", expires_at_ms: BigInt(5000) }, d, watches);
        expect(answers.map((a) => a.phase)).toEqual(["begin", "end"]);
        expect(answers[0].mask).toMatchObject({ window_id: "win-1", title: "AgentMux", width: 1000, height: 600, scale: 1.25, rects: [r(0, 0, 400, 600)], changed: false });
        expect(answers[1].mask.changed).toBe(false);
    });

    it("refuses while a pane is being dragged, an overlay it can't measure", async () => {
        const { d, answers } = deps({ refusal: () => "a pane is being dragged" });
        await answerAsk({ capture_id: "c2", phase: "begin", expires_at_ms: BigInt(5000) }, d, new Map());
        expect(answers[0].mask.refuse).toBe("a pane is being dragged");
    });

    it("says changed for an end with no begin, and answers nothing once the ask expired", async () => {
        const { d, answers } = deps();
        await answerAsk({ capture_id: "c3", phase: "end", expires_at_ms: BigInt(5000) }, d, new Map());
        expect(answers[0].mask.changed).toBe(true);
        await answerAsk({ capture_id: "c4", phase: "begin", expires_at_ms: BigInt(10) }, d, new Map());
        expect(answers).toHaveLength(1);
    });
});

describe("the private panels", () => {
    it("are each still drawn with the selector the mask looks for", () => {
        for (const p of PRIVATE_PANELS) {
            const text = readFileSync(resolve(__dirname, "../..", p.file), "utf8");
            expect(text, p.file).toContain(p.selector.slice(1));
        }
    });
});
