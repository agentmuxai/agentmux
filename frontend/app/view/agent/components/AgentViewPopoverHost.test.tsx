// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The View popover's host: it hangs from the eye in the composer row and stays there while the view changes
 * (changing an option used to rebuild it on another anchor, which put it over the neighbouring pane).
 */

import { cleanup, render } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, describe, expect, it, vi } from "vitest";

const mounted: { anchor: Element | undefined; pane: HTMLElement | undefined }[] = [];
vi.mock("./AgentViewPopover", () => ({
    AgentViewPopover: (props: { anchor: Element | undefined; pane?: HTMLElement }) => {
        mounted.push({ anchor: props.anchor, pane: props.pane });
        return <div data-testid="popover" />;
    },
}));

import { AgentViewPopoverHost } from "./AgentPaneRegions";

afterEach(() => {
    cleanup();
    mounted.length = 0;
});

function pane(withEye: boolean): HTMLElement {
    const root = document.createElement("div");
    root.innerHTML = `<div class="agent-composer-strip">${withEye ? `<button data-agent-view-chip></button>` : ""}</div>`;
    document.body.appendChild(root);
    return root;
}

const controlsOf = (open: () => boolean, resolved: () => unknown) => ({ popoverOpen: open, resolved }) as never;

describe("AgentViewPopoverHost", () => {
    it("hangs from the eye", () => {
        const root = pane(true);
        render(() => <AgentViewPopoverHost controls={controlsOf(() => true, () => ({ view: {}, source: "default" }))} rootEl={() => root} />);
        expect(mounted).toHaveLength(1);
        expect(mounted[0].anchor).toBe(root.querySelector("[data-agent-view-chip]"));
        expect(mounted[0].pane, "the panel is kept inside its own pane").toBe(root);
    });

    it("hangs from the strip when the eye is not drawn", () => {
        const root = pane(false);
        render(() => <AgentViewPopoverHost controls={controlsOf(() => true, () => ({ view: {}, source: "default" }))} rootEl={() => root} />);
        expect(mounted[0].anchor).toBe(root.querySelector(".agent-composer-strip"));
    });

    it("is not rebuilt on another anchor when an option changes the view", () => {
        const root = pane(true);
        const [resolved, setResolved] = createSignal<unknown>({ view: { jekt: "show" }, source: "default" });
        render(() => <AgentViewPopoverHost controls={controlsOf(() => true, resolved)} rootEl={() => root} />);
        setResolved({ view: { jekt: "hide" }, source: "pane" });
        setResolved({ view: { jekt: "show" }, source: "pane" });
        expect(mounted).toHaveLength(1);
        expect(mounted[0].anchor).toBe(root.querySelector("[data-agent-view-chip]"));
    });

    it("draws nothing while the popover is closed", () => {
        const root = pane(true);
        render(() => <AgentViewPopoverHost controls={controlsOf(() => false, () => ({ view: {}, source: "default" }))} rootEl={() => root} />);
        expect(mounted).toHaveLength(0);
    });
});
