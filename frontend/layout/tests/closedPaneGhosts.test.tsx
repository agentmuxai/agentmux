// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The frame that stands in for a pane that just closed (`ClosedPaneGhosts`, settlingPass.ts): where it is
 * drawn, for how long it fades, and that nothing can reach it.
 * SPEC_PANE_REFLOW_GLIDE_2026_11_01.md §6.3.
 */

import { render } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, describe, expect, it, vi } from "vitest";

vi.mock("@/app/store/global", () => ({ getSettingsKeyAtom: () => () => undefined }));
vi.mock("@/app/drag/CrossWindowDragMonitor", () => ({ setCurrentDragPayload: () => {} }));
vi.mock("@atlaskit/pragmatic-drag-and-drop/element/adapter", () => ({ dropTargetForElements: () => () => {} }));

import { setAnimationSettings } from "@/app/motion/motion";
import type { SettlingPass } from "../lib/settlingPass";
import { ClosedPaneGhosts } from "../lib/tilelayout-shared";

const passWith = (...ids: string[]): SettlingPass => ({
    moves: new Map(),
    ghosts: ids.map((id, i) => ({ id, rect: { left: 600.4 + i, top: 400, width: 599.6, height: 400 } })),
    enters: [],
    gliding: false,
});

afterEach(() => setAnimationSettings({}));

describe("ClosedPaneGhosts (the frame of a pane that just closed)", () => {
    it("draws nothing while no pass is open", () => {
        const { container } = render(() => <ClosedPaneGhosts layoutModel={{ settlingPass: () => undefined } as any} />);
        expect(container.querySelectorAll(".tile-ghost")).toHaveLength(0);
        expect(container.querySelector(".ghost-container")!.getAttribute("aria-hidden")).toBe("true");
    });

    it("draws a frame at the closed pane's rect, in the layout's own rounding, for the glide's length", () => {
        const [pass, setPass] = createSignal<SettlingPass | undefined>(passWith("c"));
        const { container } = render(() => <ClosedPaneGhosts layoutModel={{ settlingPass: pass } as any} />);
        const ghost = container.querySelector<HTMLElement>(".tile-ghost")!;
        expect(ghost.style.transform).toBe("translate3d(600px,400px, 0)");
        expect(ghost.style.width).toBe("600px");
        expect(ghost.style.height).toBe("400px");
        expect(ghost.style.animationDuration).toBe("var(--duration-base)");
        expect(ghost.querySelector(".tile-ghost-frame")).not.toBeNull();
        setPass(undefined);
        expect(container.querySelectorAll(".tile-ghost")).toHaveLength(0);
    });

    it("keeps a ghost's element when a second pane closes during its fade, so the fade does not start again", () => {
        const [pass, setPass] = createSignal<SettlingPass | undefined>(passWith("c"));
        const { container } = render(() => <ClosedPaneGhosts layoutModel={{ settlingPass: pass } as any} />);
        const first = container.querySelector(".tile-ghost");
        setPass(passWith("c", "d"));
        const ghosts = container.querySelectorAll(".tile-ghost");
        expect(ghosts).toHaveLength(2);
        expect(ghosts[0]).toBe(first);
    });

    it("fades for the shorter length under reduced movement, as the glide does", () => {
        setAnimationSettings({ "animation:preset": "custom", "animation:reducemovement": true });
        const { container } = render(() => <ClosedPaneGhosts layoutModel={{ settlingPass: () => passWith("c") } as any} />);
        expect(container.querySelector<HTMLElement>(".tile-ghost")!.style.animationDuration).toBe("var(--duration-fast)");
    });
});
