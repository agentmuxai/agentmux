// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The selected tab's own buttons (renderActions): drawn in the selected tab only, right of its label and before its
 * close button; they move with the selection, and a press on one is not a press on the tab.
 */

import { cleanup, fireEvent, render } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, describe, expect, it, vi } from "vitest";

import { PaneTabStrip } from "./PaneTabStrip";

afterEach(() => cleanup());

interface T {
    id: string;
    label: string;
}
const TABS: T[] = [
    { id: "a", label: "alpha" },
    { id: "b", label: "beta" },
    { id: "c", label: "gamma" },
];

const strip = (activeId: () => string) =>
    render(() => (
        <PaneTabStrip
            tabs={TABS}
            activeId={activeId()}
            getId={(t: T) => t.id}
            getLabel={(t: T) => t.label}
            onActivate={vi.fn()}
            onClose={vi.fn()}
            renderActions={(t: T) => <button class="probe-action" data-for={t.id}>act</button>}
        />
    ));

const probes = (c: HTMLElement) => [...c.querySelectorAll<HTMLElement>(".probe-action")];
const tabOf = (c: HTMLElement, id: string) => c.querySelector<HTMLElement>(`.pane-tab[data-tab-id="${id}"]`)!;

describe("a tab's own buttons", () => {
    it("are drawn in the selected tab only", () => {
        const { container } = strip(() => "b");
        expect(probes(container)).toHaveLength(1);
        expect(tabOf(container, "b").contains(probes(container)[0])).toBe(true);
        expect(tabOf(container, "a").querySelector(".probe-action")).toBeNull();
    });

    it("leave when another tab is selected, and arrive on it", () => {
        const [active, setActive] = createSignal("a");
        const { container } = strip(active);
        expect(probes(container).map((p) => p.dataset.for)).toEqual(["a"]);
        setActive("c");
        expect(probes(container).map((p) => p.dataset.for)).toEqual(["c"]);
        expect(tabOf(container, "c").contains(probes(container)[0])).toBe(true);
    });

    it("sit right of the label and before the close button", () => {
        const { container } = strip(() => "a");
        const tab = tabOf(container, "a");
        const kids = [...tab.children].map((el) => (el.matches(".pane-tab-label") ? "label" : el.matches(".probe-action") ? "action" : el.matches(".pane-tab-close") ? "close" : "other"));
        expect(kids.indexOf("label")).toBeGreaterThanOrEqual(0);
        expect(kids.indexOf("action")).toBeGreaterThan(kids.indexOf("label"));
        expect(kids.indexOf("close")).toBeGreaterThan(kids.indexOf("action"));
    });

    it("are drawn in a lone tab", () => {
        const { container } = render(() => (
            <PaneTabStrip
                tabs={[TABS[0]]}
                activeId="a"
                getId={(t: T) => t.id}
                getLabel={(t: T) => t.label}
                onActivate={vi.fn()}
                renderActions={() => <button class="probe-action">act</button>}
            />
        ));
        expect(probes(container)).toHaveLength(1);
    });

    it("are not drawn by a strip that gives none", () => {
        const { container } = render(() => (
            <PaneTabStrip tabs={TABS} activeId="a" getId={(t: T) => t.id} getLabel={(t: T) => t.label} onActivate={vi.fn()} />
        ));
        expect(probes(container)).toHaveLength(0);
    });

    it("a press on one does not activate the strip's tab handlers, because the component stops it", () => {
        const onActivate = vi.fn();
        const { container } = render(() => (
            <PaneTabStrip
                tabs={TABS}
                activeId="a"
                getId={(t: T) => t.id}
                getLabel={(t: T) => t.label}
                onActivate={onActivate}
                renderActions={() => (
                    <div onMouseDown={(e) => e.stopPropagation()} onClick={(e) => e.stopPropagation()}>
                        <button class="probe-action">act</button>
                    </div>
                )}
            />
        ));
        fireEvent.click(probes(container)[0]);
        expect(onActivate).not.toHaveBeenCalled();
    });
});
