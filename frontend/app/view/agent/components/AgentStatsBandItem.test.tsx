// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { cleanup, render } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, describe, expect, it, vi } from "vitest";

const [stats, setStats] = createSignal<{ cpu: number } | null>(null);
vi.mock("@/app/hook/useBlockStats", () => ({ useBlockStats: () => stats }));
vi.mock("@/element/blockstats", () => ({
    BlockStatsBadge: (p: { placement?: string; form?: string; measuring?: boolean; blockId: string }) => (
        <span data-badge={`${p.blockId}:${p.placement}:${p.form}:${p.measuring ? "measuring" : "live"}`} />
    ),
}));

import { TOP_BAND_PRIORITY, topBandItems } from "../top-band/top-band-registry";
import { AgentStatsBandItem } from "./AgentStatsBandItem";

afterEach(() => {
    cleanup();
    setStats(null);
});

describe("AgentStatsBandItem", () => {
    it("joins the band's stats slot once the first figures arrive, not before, so the band is not drawn empty", () => {
        render(() => <AgentStatsBandItem blockId="p1" />);
        expect(topBandItems("p1")()).toEqual([]);
        setStats({ cpu: 1 });
        const [item] = topBandItems("p1")();
        expect(item).toMatchObject({ id: "stats", slot: "stats", priority: TOP_BAND_PRIORITY.stats });
    });

    it("stays one item as the figures change every second", () => {
        render(() => <AgentStatsBandItem blockId="p1" />);
        setStats({ cpu: 1 });
        const first = topBandItems("p1")()[0];
        setStats({ cpu: 2 });
        setStats({ cpu: 3 });
        expect(topBandItems("p1")()).toHaveLength(1);
        expect(topBandItems("p1")()[0]).toBe(first);
    });

    it("has a full and a compact form, each the band placement of the badge, and a measuring copy of each", () => {
        render(() => <AgentStatsBandItem blockId="p1" />);
        setStats({ cpu: 1 });
        const [item] = topBandItems("p1")();
        const draw = (el: unknown) =>
            render(() => el as never)
                .container.querySelector("[data-badge]")
                ?.getAttribute("data-badge");
        expect(draw(item.full({ measuring: false }))).toBe("p1:band:full:live");
        expect(draw(item.compact!({ measuring: false }))).toBe("p1:band:compact:live");
        expect(draw(item.full({ measuring: true }))).toBe("p1:band:full:measuring");
        expect(draw(item.compact!({ measuring: true }))).toBe("p1:band:compact:measuring");
    });

    it("leaves the band when the pane goes", () => {
        const view = render(() => <AgentStatsBandItem blockId="p1" />);
        setStats({ cpu: 1 });
        expect(topBandItems("p1")()).toHaveLength(1);
        view.unmount();
        expect(topBandItems("p1")()).toEqual([]);
    });
});
