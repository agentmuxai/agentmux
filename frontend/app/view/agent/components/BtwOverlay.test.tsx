// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { cleanup, render } from "@solidjs/testing-library";
import { afterEach, describe, expect, it, vi } from "vitest";

const hub = vi.hoisted(() => ({ onChunk: null as ((data: unknown) => void) | null }));

vi.mock("@/app/store/mps", () => ({
    onMuxEvent: (_eventType: string, handler: (data: unknown) => void) => {
        hub.onChunk = handler;
        return () => {};
    },
}));

import { recycleBlockers } from "@/app/store/recycle-blockers";
import { BtwOverlay } from "./BtwOverlay";

const reason = "a /btw answer is still coming in";

describe("BtwOverlay", () => {
    afterEach(cleanup);

    it("an answer still streaming blocks a renderer recycle until it is done", () => {
        render(() => <BtwOverlay blockId="b1" askId={1} question="why?" requestId="r1" error={null} onClose={() => {}} />);
        expect(recycleBlockers()).toContain(reason);
        hub.onChunk!({ event: { type: "done", response: "because" }, done: true });
        expect(recycleBlockers()).not.toContain(reason);
    });

    it("stops blocking once unmounted", () => {
        const { unmount } = render(() => (
            <BtwOverlay blockId="b1" askId={1} question="why?" requestId="r1" error={null} onClose={() => {}} />
        ));
        expect(recycleBlockers()).toContain(reason);
        unmount();
        expect(recycleBlockers()).not.toContain(reason);
    });
});
