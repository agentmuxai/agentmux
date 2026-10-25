// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The progress line on a pane tab pill: one shape per state, none on the
 * selected pill of an expanded pane, and the state in words.
 * docs/specs/SPEC_PANE_TAB_PROGRESS_LINE_2026_10_24.md §3, §5.
 */
import { cleanup, render } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { PaneTabProgress } from "@/app/store/pane-tab-progress";
import { WindowTabDisplayedProvider } from "@/app/workspace/window-tab-visibility";
import { PaneTabStrip } from "./PaneTabStrip";

afterEach(() => cleanup());

const TABS = ["a", "b"];
const WORDS: Record<PaneTabProgress["state"], string> = {
    working: "Working",
    waiting: "Waiting for you",
    limited: "Out of quota until Sun 7 PM",
    signed_out: "Signed out",
};

function strip(opts: {
    progress: Record<string, () => PaneTabProgress | null>;
    active?: string;
    collapsed?: boolean;
    displayed?: boolean;
}) {
    return render(() => (
        <WindowTabDisplayedProvider value={() => opts.displayed ?? true}>
            <PaneTabStrip
                tabs={TABS}
                activeId={opts.active ?? "a"}
                getId={(t: string) => t}
                getLabel={(t: string) => t}
                useProgress={(t: string) => opts.progress[t] ?? (() => null)}
                collapsed={() => opts.collapsed ?? false}
                onActivate={vi.fn()}
            />
        </WindowTabDisplayedProvider>
    ));
}

const pill = (c: HTMLElement, id: string) => c.querySelector<HTMLElement>(`.pane-tab[data-tab-id="${id}"]`)!;
const line = (c: HTMLElement, id: string) => pill(c, id).querySelector<HTMLElement>(".pane-tab-progress")!;
const on = (el: HTMLElement) => el.classList.contains("pane-tab-progress--on");

describe("the pill's progress line", () => {
    it("moves along a working pill's track, and says so", () => {
        const { container } = strip({ progress: { b: () => ({ state: "working", text: WORDS.working }) } });
        const l = line(container, "b");
        expect(on(l)).toBe(true);
        expect(l.dataset.state).toBe("working");
        expect(l.classList.contains("pane-tab-progress--sweeping")).toBe(true);
        expect(l.querySelector(".pane-tab-progress-bar")).not.toBeNull();
        expect(pill(container, "b").getAttribute("aria-busy")).toBe("true");
    });

    for (const state of ["waiting", "limited", "signed_out"] as const) {
        it(`is a still line for ${state}, with its own look and no sweep`, () => {
            const { container } = strip({ progress: { b: () => ({ state, text: WORDS[state] }) } });
            const l = line(container, "b");
            expect(on(l)).toBe(true);
            expect(l.dataset.state).toBe(state);
            expect(l.classList.contains("pane-tab-progress--sweeping")).toBe(false);
            expect(pill(container, "b").getAttribute("aria-busy")).toBeNull();
        });
    }

    it("shows nothing on a pill with no progress", () => {
        const { container } = strip({ progress: {} });
        expect(on(line(container, "a"))).toBe(false);
        expect(on(line(container, "b"))).toBe(false);
        expect(pill(container, "a").getAttribute("aria-describedby")).toBeNull();
    });

    it("draws no line on the selected pill of an expanded pane, yet still says the state in words", () => {
        const { container } = strip({
            active: "a",
            progress: { a: () => ({ state: "working", text: WORDS.working }), b: () => ({ state: "working", text: WORDS.working }) },
        });
        expect(on(line(container, "a"))).toBe(false);
        expect(on(line(container, "b"))).toBe(true);
        expect(pill(container, "a").getAttribute("aria-describedby")).toBeTruthy();
    });

    it("draws the line on the selected pill of a collapsed pane, and on each working pill", () => {
        const { container } = strip({
            active: "a",
            collapsed: true,
            progress: { a: () => ({ state: "waiting", text: WORDS.waiting }), b: () => ({ state: "working", text: WORDS.working }) },
        });
        expect(on(line(container, "a"))).toBe(true);
        expect(on(line(container, "b"))).toBe(true);
    });

    it("draws no line for a selected waiting, limited or signed-out agent of an expanded pane", () => {
        for (const state of ["waiting", "limited", "signed_out"] as const) {
            const { container, unmount } = strip({ active: "a", progress: { a: () => ({ state, text: WORDS[state] }) } });
            expect(on(line(container, "a"))).toBe(false);
            unmount();
        }
    });

    it("says the state in words to assistive tech", () => {
        const { container } = strip({ progress: { b: () => ({ state: "limited", text: WORDS.limited }) } });
        const id = pill(container, "b").getAttribute("aria-describedby")!;
        expect(container.querySelector(`#${CSS.escape(id)}`)?.textContent).toBe(WORDS.limited);
    });

    it("does not sweep while its window tab is hidden, and keeps the line", () => {
        const { container } = strip({ displayed: false, progress: { b: () => ({ state: "working", text: WORDS.working }) } });
        const l = line(container, "b");
        expect(on(l)).toBe(true);
        expect(l.classList.contains("pane-tab-progress--sweeping")).toBe(false);
    });

    it("fades out in its last look when the work ends, rather than changing shape", () => {
        const [progress, setProgress] = createSignal<PaneTabProgress | null>({ state: "limited", text: WORDS.limited });
        const { container } = strip({ progress: { b: progress } });
        const l = line(container, "b");
        expect(l.dataset.state).toBe("limited");
        setProgress(null);
        expect(on(l)).toBe(false);
        expect(l.dataset.state).toBe("limited");
    });

    it("follows a state change on the pill", () => {
        const [progress, setProgress] = createSignal<PaneTabProgress | null>({ state: "working", text: WORDS.working });
        const { container } = strip({ progress: { b: progress } });
        setProgress({ state: "waiting", text: WORDS.waiting });
        const l = line(container, "b");
        expect(l.dataset.state).toBe("waiting");
        expect(l.classList.contains("pane-tab-progress--sweeping")).toBe(false);
        expect(pill(container, "b").getAttribute("aria-busy")).toBeNull();
    });

    it("leaves a strip with no progress source exactly as it was", () => {
        const { container } = render(() => (
            <PaneTabStrip tabs={TABS} activeId="a" getId={(t: string) => t} getLabel={(t: string) => t} onActivate={vi.fn()} />
        ));
        expect(container.querySelector(".pane-tab-progress")).toBeNull();
    });
});
