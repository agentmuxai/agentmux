// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Tests for the Help pane's Ctrl+Wheel zoom (helpview.tsx).
 *
 * This handler was missed entirely by the original research/spec for
 * SPEC_CTRL_SHIFT_SCROLL_ZOOM_ALL_PANES_2026_09_07.md — Codex caught it on
 * PR #3090's review, after the other six duplicated Ctrl+Wheel handlers
 * (term/editor/armory/warden/swarm/AgentShellSubblock) had already been
 * fixed and tested. This file exists so a future change to this same class
 * of gesture doesn't have to rediscover this pane the same way twice.
 */

import { cleanup, render } from "@solidjs/testing-library";
import { afterEach, describe, expect, it, vi } from "vitest";

vi.mock("@/app/store/global", () => ({
    WOS: {
        makeORef: (type: string, id: string) => `${type}:${id}`,
        getWaveObjectAtom: () => () => ({ meta: {} }),
    },
}));

const setMetaMock = vi.fn((..._args: unknown[]) => Promise.resolve(undefined));
vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: { SetMetaCommand: (...args: unknown[]) => setMetaMock(...args) },
}));

const showZoomIndicatorMock = vi.fn();
vi.mock("@/app/store/zoom", () => ({
    showZoomIndicator: (...args: unknown[]) => showZoomIndicatorMock(...args),
}));

import { HelpView, HelpViewModel } from "./helpview";

describe("HelpView zoom", () => {
    afterEach(() => {
        cleanup();
        setMetaMock.mockClear();
        showZoomIndicatorMock.mockClear();
    });

    function renderHelp() {
        const model = new HelpViewModel("test-block");
        return render(() => <HelpView model={model} />);
    }

    it("Ctrl+Wheel writes help:zoom via SetMetaCommand", () => {
        const { container } = renderHelp();
        const root = container.querySelector("[tabindex]") as HTMLElement;
        root.dispatchEvent(new WheelEvent("wheel", { ctrlKey: true, deltaY: -100, bubbles: true, cancelable: true }));
        expect(setMetaMock).toHaveBeenCalledWith(undefined, {
            oref: "block:test-block",
            meta: { "help:zoom": 1.05 },
        });
    });

    it("plain wheel (no Ctrl) does not trigger a zoom RPC call", () => {
        const { container } = renderHelp();
        const root = container.querySelector("[tabindex]") as HTMLElement;
        root.dispatchEvent(new WheelEvent("wheel", { ctrlKey: false, deltaY: -100, bubbles: true, cancelable: true }));
        expect(setMetaMock).not.toHaveBeenCalled();
    });

    // Codex review, PR #3090: this handler had no shiftKey guard at all,
    // so Ctrl+Shift+Scroll over the Help pane zoomed only Help (via
    // stopPropagation()) instead of reaching AppAllPanesZoomHandler
    // (app.tsx) and zooming every pane in the window. This was a real gap
    // missed by the original six-view fix.
    it("Ctrl+Shift+Wheel does not trigger this pane's own zoom RPC call", () => {
        const { container } = renderHelp();
        const root = container.querySelector("[tabindex]") as HTMLElement;
        root.dispatchEvent(
            new WheelEvent("wheel", { ctrlKey: true, shiftKey: true, deltaY: -100, bubbles: true, cancelable: true })
        );
        expect(setMetaMock).not.toHaveBeenCalled();
    });
});
