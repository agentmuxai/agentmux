// @vitest-environment jsdom
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The dragged tab itself in its landing place (tabLanding.ts): the shell kept from its pane when the drag
 * begins (the elements around its content and a header with only this tab), the shell built from it for the
 * place, and where the tab's content stands while the place is shown.
 */

import { createRoot } from "solid-js";
import { afterEach, describe, expect, it } from "vitest";
import {
    buildTabShell,
    captureTabShell,
    registerTabMember,
    resetTabLandingForTests,
    showTabLanding,
    tabLandingMountOf,
    tabMemberLands,
    tabShellOf,
} from "./tabLanding";

/** A pane of two tabs as the shared chrome draws it, `tab-1` in front. */
function mountPane(): HTMLElement {
    document.body.innerHTML = `
        <div class="tile-node" id="pane-a">
            <div class="tile-leaf">
                <div class="pane-stack pane-stack-focused" style="--pane-ring-color: red;">
                    <div class="block-frame-default-header" data-role="block-header" id="header-1">
                        <div class="block-frame-header-scroll">
                            <div class="pane-tab pane-tab--active pane-tab--dragging" data-tab-id="tab-1">One</div>
                            <div class="pane-tab" data-tab-id="tab-2">Two</div>
                        </div>
                        <div class="block-frame-end-icons">x</div>
                    </div>
                    <div class="pane-progress-bar-slot"></div>
                    <div class="pane-stack-body term-body">
                        <div class="behind"></div>
                        <div class="pane-stack-content">
                            <div class="pane-leaf-keepalive-slot">
                                <div class="pane-leaf-member"><div class="block" data-blockid="tab-1">content</div></div>
                            </div>
                            <div class="pane-leaf-keepalive-slot">
                                <div class="pane-leaf-member"><div class="block" data-blockid="tab-2">other</div></div>
                            </div>
                        </div>
                    </div>
                </div>
            </div>
        </div>`;
    return document.getElementById("pane-a")!;
}

afterEach(() => {
    resetTabLandingForTests();
    document.body.innerHTML = "";
});

describe("the shell a dragged tab keeps from its pane", () => {
    it("keeps the elements between the pane and the content, and a header with only this tab in front", () => {
        mountPane();
        captureTabShell("tab-1", "pane-a");
        const kept = tabShellOf("tab-1")!;
        expect(kept.levels.map((l) => l.className)).toEqual([
            "pane-stack pane-stack-focused",
            "pane-stack-body term-body",
            "pane-stack-content",
        ]);
        expect(kept.levels[0].style).toContain("--pane-ring-color");
        // The header is the pane root's own child, and a copy: the pane's header is untouched.
        expect(kept.header?.level).toBe(0);
        const pills = kept.header!.el.querySelectorAll(".pane-tab");
        expect(pills.length).toBe(1);
        expect((pills[0] as HTMLElement).dataset.tabId).toBe("tab-1");
        expect(pills[0].classList.contains("pane-tab--active")).toBe(true);
        expect(pills[0].classList.contains("pane-tab--dragging")).toBe(false);
        expect(kept.header!.el.hasAttribute("id")).toBe(false);
        expect(kept.header!.el.querySelector(".block-frame-end-icons")).not.toBeNull();
        expect(document.querySelectorAll("#pane-a .pane-tab").length).toBe(2);
        // Kept for that tab only.
        expect(tabShellOf("tab-2")).toBeUndefined();
    });

    it("builds the place's shell from it: the same nesting, the header first in the pane, the content innermost", () => {
        mountPane();
        captureTabShell("tab-1", "pane-a");
        const { root, content } = buildTabShell(tabShellOf("tab-1"));
        expect(root.className).toBe("pane-stack pane-stack-focused");
        expect(root.style.getPropertyValue("--pane-ring-color")).toBe("red");
        expect(root.firstElementChild?.getAttribute("data-role")).toBe("block-header");
        expect(root.lastElementChild?.className).toBe("pane-stack-body term-body");
        expect(content.className).toBe("pane-stack-content");
        expect(content.parentElement).toBe(root.lastElementChild);
        expect(content.childElementCount).toBe(0);
        // Each place gets its own copy of the header.
        const again = buildTabShell(tabShellOf("tab-1"));
        expect(again.root.firstElementChild).not.toBe(root.firstElementChild);
    });

    it("keeps nothing for a tab that is not on screen, and a pane with no shell gets a bare content element", () => {
        mountPane();
        captureTabShell("tab-9", "pane-a");
        expect(tabShellOf("tab-9")).toBeUndefined();
        captureTabShell("tab-1", "no-such-pane");
        expect(tabShellOf("tab-1")).toBeUndefined();
        const { root, content } = buildTabShell(undefined);
        expect(root).toBe(content);
        expect(root.className).toBe("pane-landing-content");
    });

    it("leaves a header the block draws itself with the block", () => {
        document.body.innerHTML = `
            <div class="tile-node" id="pane-b"><div class="tile-leaf">
                <div class="block" data-blockid="tab-1"><div data-role="block-header"></div></div>
            </div></div>`;
        captureTabShell("tab-1", "pane-b");
        const kept = tabShellOf("tab-1")!;
        expect(kept.levels).toEqual([]);
        expect(kept.header).toBeUndefined();
    });
});

describe("where a dragged tab's content stands", () => {
    it("lands only a tab whose content is mounted and can be moved, counted per mount", () => {
        expect(tabMemberLands("tab-1")).toBe(false);
        const first = registerTabMember("tab-1");
        // Mounted in its new pane before it is unmounted in the old one.
        const second = registerTabMember("tab-1");
        first();
        expect(tabMemberLands("tab-1")).toBe(true);
        second();
        expect(tabMemberLands("tab-1")).toBe(false);
    });

    it("is the place's content element while the place is shown, for that tab only", () => {
        createRoot((dispose) => {
            const el = document.createElement("div");
            expect(tabLandingMountOf("tab-1")).toBeUndefined();
            const hide = showTabLanding("tab-1", el);
            expect(tabLandingMountOf("tab-1")).toBe(el);
            expect(tabLandingMountOf("tab-2")).toBeUndefined();
            // A newer place is not taken down by an older one going.
            const newer = document.createElement("div");
            const hideNewer = showTabLanding("tab-1", newer);
            hide();
            expect(tabLandingMountOf("tab-1")).toBe(newer);
            hideNewer();
            expect(tabLandingMountOf("tab-1")).toBeUndefined();
            dispose();
        });
    });
});
