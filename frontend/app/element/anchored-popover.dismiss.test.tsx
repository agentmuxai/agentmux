// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * `dismissAnchoredPopovers` closes every open dismissable popover as a press outside would (the command palette
 * asks it before it opens, SPEC_COMMAND_PALETTE_TOP_CENTRE_2026_11_17.md R21), and leaves the passive ones (a hover
 * tip, which has no `onDismiss`) and the closed ones alone.
 */

import { cleanup, render } from "@solidjs/testing-library";
import { afterEach, describe, expect, it, vi } from "vitest";

vi.mock("@floating-ui/dom", () => ({ autoUpdate: vi.fn(() => vi.fn()) }));
vi.mock("@/app/platform/pane-overlay", () => ({
    useHiddenTabMarker: () => () => undefined,
    usePaneOverlay: vi.fn(),
}));
vi.mock("@/app/util/menu-position", () => ({
    computeMenuPosition: vi.fn(async () => ({ style: { position: "fixed", left: "0px", top: "0px" } })),
}));

import { AnchoredPopover, dismissAnchoredPopovers } from "./anchored-popover";

afterEach(cleanup);

function popover(onDismiss?: (reason: "outside" | "escape") => void) {
    const anchor = document.createElement("button");
    document.body.appendChild(anchor);
    return render(() => (
        <AnchoredPopover anchor={anchor} placement="top-end" onDismiss={onDismiss}>
            <span>content</span>
        </AnchoredPopover>
    ));
}

describe("dismissAnchoredPopovers", () => {
    it("dismisses every open dismissable popover as an outside press", () => {
        const a = vi.fn();
        const b = vi.fn();
        popover(a);
        popover(b);
        dismissAnchoredPopovers();
        expect(a).toHaveBeenCalledWith("outside");
        expect(b).toHaveBeenCalledWith("outside");
    });

    it("leaves a popover with no onDismiss alone", () => {
        popover();
        expect(() => dismissAnchoredPopovers()).not.toThrow();
    });

    it("forgets a popover that has closed", () => {
        const a = vi.fn();
        const result = popover(a);
        result.unmount();
        dismissAnchoredPopovers();
        expect(a).not.toHaveBeenCalled();
    });
});
