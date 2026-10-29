// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Pane Tab contract Phase 3: ONE visibility signal per block — pane-stack
 * dormancy and window-tab display, in both window-tab modes (kept laid out,
 * or content-visibility hidden).
 */

import { render } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { describe, expect, it, vi } from "vitest";

const dormant = new Map<string, ReturnType<typeof createSignal<boolean>>>();
vi.mock("@/app/store/block-component-registry", () => ({
    isBlockDormant: (id: string) => {
        if (!dormant.has(id)) dormant.set(id, createSignal(false));
        return dormant.get(id)![0];
    },
}));

import { setBlockMinimized } from "@/app/block/pane-minimized";
import { WindowTabDisplayedProvider } from "@/app/workspace/window-tab-visibility";
import { usePaneTabVisibility, type PaneTabVisibility } from "./pane-tab-visibility";

function setup(blockId: string) {
    const [displayed, setDisplayed] = createSignal(true);
    let visibility!: () => PaneTabVisibility;
    function Probe() {
        visibility = usePaneTabVisibility(blockId);
        return null;
    }
    render(() => (
        <WindowTabDisplayedProvider value={displayed}>
            <Probe />
        </WindowTabDisplayedProvider>
    ));
    const setDormant = (v: boolean) => {
        if (!dormant.has(blockId)) dormant.set(blockId, createSignal(false));
        dormant.get(blockId)![1](v);
    };
    return { visibility: () => visibility(), setDisplayed, setDormant };
}

describe("usePaneTabVisibility", () => {
    it("is active on a displayed window tab when the pane shows this block", () => {
        expect(setup("v1").visibility()).toBe("active");
    });

    it("is dormant while the block is a hidden kept-alive pane-stack member", () => {
        const s = setup("v2");
        s.setDormant(true);
        expect(s.visibility()).toBe("dormant");
        s.setDormant(false);
        expect(s.visibility()).toBe("active");
    });

    it("is dormant while its pane is minimized, even as the front member, and active again once restored", () => {
        const s = setup("v-min");
        setBlockMinimized("v-min", true);
        expect(s.visibility()).toBe("dormant");
        setBlockMinimized("v-min", false);
        expect(s.visibility()).toBe("active");
    });

    it("stays windowHidden, not dormant, when its pane is minimized in a window tab that is not displayed", () => {
        const s = setup("v-min2");
        setBlockMinimized("v-min2", true);
        s.setDisplayed(false);
        expect(s.visibility()).toBe("windowHidden");
        setBlockMinimized("v-min2", false);
    });

    it("is windowHidden while its window tab is not displayed, whatever the pane stack says", () => {
        const s = setup("v3");
        s.setDisplayed(false);
        expect(s.visibility()).toBe("windowHidden");
        s.setDormant(true);
        expect(s.visibility()).toBe("windowHidden");
        s.setDisplayed(true);
        expect(s.visibility()).toBe("dormant");
    });

    it("outside any window tab (a floating or torn-off window) counts as displayed", () => {
        let v!: () => PaneTabVisibility;
        function Probe() {
            v = usePaneTabVisibility("v4");
            return null;
        }
        render(() => <Probe />);
        expect(v()).toBe("active");
    });
});
