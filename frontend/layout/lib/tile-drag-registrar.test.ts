// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Which header a tile's whole-pane drag is bound to, and when it rebinds.
 * docs/specs/SPEC_DRAG_AND_DROP_CONSOLIDATION_2026_09_27.md §5.7 and §5.8.
 */

import { afterEach, describe, expect, it, vi } from "vitest";
import { createTileDragRegistrar } from "./tile-drag-registrar";

const header = () => {
    const el = document.createElement("div");
    el.setAttribute("data-role", "block-header");
    return el;
};

const flush = () => new Promise((r) => setTimeout(r, 0));

function setup(dragging = () => false) {
    const root = document.createElement("div");
    document.body.appendChild(root);
    const bound: HTMLElement[] = [];
    const unbound: HTMLElement[] = [];
    const bind = vi.fn((el: HTMLElement) => {
        bound.push(el);
        return () => unbound.push(el);
    });
    const registrar = createTileDragRegistrar({
        root,
        find: () => root.querySelector<HTMLElement>('[data-role="block-header"]'),
        bind,
        dragging,
    });
    return { root, bound, unbound, bind, registrar };
}

afterEach(() => {
    document.body.innerHTML = "";
});

describe("tile drag registrar", () => {
    it("binds the live header, never a detached ErrorBoundary fallback", () => {
        const live = header();
        const fallback = header(); // created by the fallback render, never inserted
        const { root, bound, registrar } = setup();
        root.appendChild(live);
        registrar.register();
        expect(bound).toEqual([live]);
        expect(bound).not.toContain(fallback);
        registrar.dispose();
    });

    it("binds a header that mounts after the tile, without polling", async () => {
        const { root, bound, registrar } = setup();
        registrar.register();
        expect(bound).toEqual([]);
        const live = header();
        root.appendChild(live);
        await flush();
        expect(bound).toEqual([live]);
        registrar.dispose();
    });

    it("rebinds when a Show gate replaces the header", async () => {
        const first = header();
        const { root, bound, unbound, registrar } = setup();
        root.appendChild(first);
        registrar.register();
        const second = header();
        root.replaceChild(second, first);
        await flush();
        expect(unbound).toEqual([first]);
        expect(bound).toEqual([first, second]);
        registrar.dispose();
    });

    it("ignores DOM changes that leave the same header in place", async () => {
        const live = header();
        const { root, bind, registrar } = setup();
        root.appendChild(live);
        registrar.register();
        live.appendChild(document.createElement("span"));
        root.appendChild(document.createElement("div"));
        await flush();
        expect(bind).toHaveBeenCalledTimes(1);
        registrar.dispose();
    });

    it("never tears down mid-drag, and retries the skipped rebind when the drag ends", async () => {
        let dragging = false;
        const first = header();
        const { root, bound, unbound, registrar } = setup(() => dragging);
        root.appendChild(first);
        registrar.register();
        dragging = true;
        const second = header();
        root.replaceChild(second, first);
        await flush();
        expect(unbound).toEqual([]);
        expect(bound).toEqual([first]);
        dragging = false;
        registrar.dragEnded();
        expect(unbound).toEqual([first]);
        expect(bound).toEqual([first, second]);
        registrar.dispose();
    });

    it("a drag end with nothing skipped doesn't rebind", () => {
        const live = header();
        const { root, bind, registrar } = setup();
        root.appendChild(live);
        registrar.register();
        registrar.dragEnded();
        expect(bind).toHaveBeenCalledTimes(1);
        registrar.dispose();
    });

    it("dispose unbinds and stops observing", async () => {
        const live = header();
        const { root, bind, unbound, registrar } = setup();
        root.appendChild(live);
        registrar.register();
        registrar.dispose();
        expect(unbound).toEqual([live]);
        root.replaceChild(header(), live);
        await flush();
        expect(bind).toHaveBeenCalledTimes(1);
    });
});
