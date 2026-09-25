// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { createRoot } from "solid-js";
import { describe, expect, it, vi } from "vitest";

const claimFocusOnMount = vi.hoisted(() => vi.fn());
vi.mock("@/app/store/focusManager", () => ({ focusManager: { claimFocusOnMount } }));
vi.mock("@/app/platform/ipc", () => ({ invokeCommand: vi.fn(() => Promise.resolve()) }));
vi.mock("@/app/store/block-component-registry", () => ({ isBlockDormant: () => () => false }));

import { usePaneRectSync } from "./use-pane-rect-sync";

// jsdom has no ResizeObserver; the hook's onMount observes the placeholder.
vi.stubGlobal(
    "ResizeObserver",
    class {
        observe() {}
        disconnect() {}
    },
);

// SPEC_MACOS_BROWSER_PANE_KEYBOARD_FOCUS_2026_09_24: a browser pane opened
// with focus (`muxsh web`) must take the caret once its page exists, the way
// terminal/editor panes do on mount — the giveFocus() attempted at layout
// insert found no page yet, so nothing ever moved the keyboard into it.

describe("usePaneRectSync — claim focus once the page is created", () => {
    it("claims focus for its block after browser_pane_create resolves", async () => {
        const giveFocus = vi.fn(() => true);
        const model = { blockId: "b1", closed: false, giveFocus, urlAtom: () => "", onError: vi.fn(), onLoad: vi.fn() } as any;
        const placeholder = document.createElement("div");
        document.body.appendChild(placeholder);

        let sync!: ReturnType<typeof usePaneRectSync>;
        const dispose = createRoot((d) => {
            sync = usePaneRectSync({ model, placeholderRef: () => placeholder, windowLabel: "main", diag: () => {} });
            return d;
        });

        await sync.createPane("https://example.com");

        expect(sync.paneCreated()).toBe(true);
        expect(claimFocusOnMount).toHaveBeenCalledWith("b1", expect.any(Function));
        // The claim's focus action is the pane's own giveFocus (browser_pane_focus).
        claimFocusOnMount.mock.calls[0][1]();
        expect(giveFocus).toHaveBeenCalled();
        dispose();
    });
});
