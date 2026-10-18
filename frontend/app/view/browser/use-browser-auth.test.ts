// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { createRoot } from "solid-js";
import { describe, expect, it, vi } from "vitest";

const hub = vi.hoisted(() => ({
    onAuth: null as ((payload: unknown) => void) | null,
}));

vi.mock("@/app/store/app-api", () => ({
    getApi: () => ({
        listen: (_event: string, handler: (payload: unknown) => void) => {
            hub.onAuth = handler;
            return Promise.resolve(() => {});
        },
        browserPanes: { authCancel: vi.fn(async () => {}), authSubmit: vi.fn(async () => {}) },
    }),
}));

import { recycleBlockers } from "@/app/store/recycle-blockers";
import { useBrowserAuth } from "./use-browser-auth";

describe("useBrowserAuth", () => {
    it("a sign-in prompt blocks a renderer recycle until it is answered", () => {
        const open = vi.fn();
        const dispose = createRoot((d) => {
            useBrowserAuth({ model: { blockId: "b1" } as never, modalLayer: { open } as never, diag: () => {} });
            return d;
        });
        hub.onAuth!({ block_id: "b1", request_id: "r1", origin: "https://x", host: "x", port: 443, realm: "r", is_proxy: false });
        expect(recycleBlockers()).toContain("a browser pane is asking for a sign-in");
        open.mock.calls[0][0].onCancel();
        expect(recycleBlockers()).not.toContain("a browser pane is asking for a sign-in");
        dispose();
    });
});
