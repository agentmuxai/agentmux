// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * What the tab strip draws, pinned as markup: a pane header's strip with every per-tab option, the
 * strip while a tab from another pane is dragged over it, a document strip, and the add button.
 * The drag, reveal, selection and gesture behaviour is pinned in the other PaneTabStrip tests. A
 * refactor of the strip must leave the committed snapshots as they are; regenerate with
 * `vitest -u` only for a change meant to look different.
 */

import { cleanup, render } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const dropTargetCalls: any[] = [];

vi.mock("@atlaskit/pragmatic-drag-and-drop/element/adapter", () => ({
    draggable: () => () => {},
    dropTargetForElements: (config: any) => {
        dropTargetCalls.push(config);
        return () => {};
    },
}));
vi.mock("@/app/drag/CrossWindowDragMonitor", () => ({ setCurrentDragPayload: () => {} }));

import { paneTabItemType } from "@/app/drag/drag-types";
import { endDrag } from "@/app/drag/drag-session";
import type { PaneTabProgress } from "@/app/store/pane-tab-progress";
import { PaneTabStrip } from "./PaneTabStrip";

interface T {
    id: string;
    label: string;
    dirty?: boolean;
    closing?: boolean;
}

const TABS: T[] = [
    { id: "a", label: "alpha" },
    { id: "b", label: "beta", dirty: true },
    { id: "c", label: "gamma", closing: true },
];

const markup = (el: Element | null): string => (el ? el.outerHTML : "(absent)").replace(/></g, ">\n<") + "\n";
const snap = (name: string) => `./__snapshots__/PaneTabStrip.markup.${name}.html`;
const strip = (c: HTMLElement) => c.querySelector(".pane-tab-strip");

beforeEach(() => {
    endDrag("cancel");
    dropTargetCalls.length = 0;
});
afterEach(() => cleanup());

describe("PaneTabStrip markup", () => {
    it("a pane header's strip: icons, tooltips, attention, colours, a closing tab, close buttons and a labelled add", async () => {
        const v = render(() => (
            <PaneTabStrip
                tabs={TABS}
                activeId="b"
                getId={(t: T) => t.id}
                getLabel={(t: T) => t.label}
                getIcon={(t: T) => <i class={`fa-solid fa-${t.id}`} />}
                getTooltip={(t: T) => `${t.label} tooltip`}
                getTooltipDetail={(t: T) => (t.id === "a" ? "Working on the parser" : undefined)}
                getTooltipSeat={(t: T) => (t.id === "a" ? "Leader · product" : undefined)}
                getAttention={(t: T) => !!t.dirty}
                getTabClass={(t: T) => ({ "pane-tab--custom": t.id === "a" })}
                getColor={(t: T) => ({ underline: "#ff8800", background: "#221100", activeBackground: "#331100", neutralBackground: "#111111" })}
                isClosing={(t: T) => !!t.closing}
                onActivate={vi.fn()}
                onClose={vi.fn()}
                onAdd={vi.fn()}
                addLabel="New Agent"
                zoomFactor={() => 1.25}
            />
        ));
        await expect(markup(strip(v.container))).toMatchFileSnapshot(snap("header"));
    });

    it("a strip with a custom label and a bare add button", async () => {
        const v = render(() => (
            <PaneTabStrip
                tabs={TABS.slice(0, 2)}
                activeId="a"
                getId={(t: T) => t.id}
                getLabel={(t: T) => t.label}
                renderLabel={(t: T) => <input class="rename" value={t.label} />}
                onActivate={vi.fn()}
                onAdd={vi.fn()}
                addTitle="Open a file"
            />
        ));
        await expect(markup(strip(v.container))).toMatchFileSnapshot(snap("custom-label"));
    });

    it("pills that show their work: a progress line in each state, the selected mark, and the hidden line on an expanded active pill", async () => {
        const progress: Record<string, PaneTabProgress | null> = {
            a: { state: "working", text: "Working" },
            b: { state: "waiting", text: "Waiting for you" },
            c: { state: "signed_out", text: "Signed out" },
        };
        const v = render(() => (
            <PaneTabStrip
                tabs={TABS}
                activeId="a"
                getId={(t: T) => t.id}
                getLabel={(t: T) => t.label}
                onActivate={vi.fn()}
                useProgress={(t: T) => () => progress[t.id]}
                collapsed={() => false}
                selection={{ isSelected: (id) => id === "b", toggle: vi.fn(), clear: vi.fn(), dragIds: (id) => [id] }}
            />
        ));
        await expect(markup(strip(v.container))).toMatchFileSnapshot(snap("progress-selection"));
    });

    it("a document strip", async () => {
        const v = render(() => (
            <PaneTabStrip
                tabs={TABS.slice(0, 2)}
                activeId="a"
                getId={(t: T) => t.id}
                getLabel={(t: T) => t.label}
                onActivate={vi.fn()}
                onClose={vi.fn()}
                docDrag={{ docType: "editor", blockId: "block-1", onReorder: vi.fn() }}
            />
        ));
        await expect(markup(strip(v.container))).toMatchFileSnapshot(snap("doc"));
    });

    it("a tab dragged here from another pane: the strip lights up and shows where the tab lands", async () => {
        const v = render(() => (
            <div data-role="block-header">
                <div class="block-frame-default-header-tabstrip">
                    <PaneTabStrip
                        tabs={TABS.slice(0, 2)}
                        activeId="a"
                        getId={(t: T) => t.id}
                        getLabel={(t: T) => t.label}
                        onActivate={vi.fn()}
                        paneKey="pane-self"
                        onReceiveForeignTab={vi.fn()}
                    />
                </div>
            </div>
        ));
        const config = dropTargetCalls[0];
        expect(config).toBeDefined();
        const header = v.container.querySelector('[data-role="block-header"]')!;
        expect(header.classList.contains("pane-header--foreign-hover")).toBe(false);
        config.onDragEnter({ source: { data: { type: paneTabItemType, blockId: "x", label: "delta", sourceNodeId: "other" } } });
        expect(header.classList.contains("pane-header--foreign-hover")).toBe(true);
        await expect(markup(header)).toMatchFileSnapshot(snap("foreign-hover"));
        config.onDragLeave();
        expect(header.classList.contains("pane-header--foreign-hover")).toBe(false);
        expect(v.container.querySelector(".pane-tab--incoming")).toBeNull();
    });
});
