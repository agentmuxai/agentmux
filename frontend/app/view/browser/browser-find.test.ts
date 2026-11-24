// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { createRoot } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const h = vi.hoisted(() => ({
    calls: [] as string[],
    result: null as null | ((r: { block_id: string; count: number; active: number }) => void),
    unlistened: 0,
}));

vi.mock("@/app/element/search", async () => {
    const { createSignalAtom } = await import("@/util/util");
    return {
        useSearch: (options?: { viewModel?: { searchAtoms?: unknown }; caseSensitive?: boolean }) => {
            const atoms = {
                searchValue: createSignalAtom(""),
                resultsIndex: createSignalAtom(0),
                resultsCount: createSignalAtom(0),
                isOpen: createSignalAtom(false),
                caseSensitive: createSignalAtom(options?.caseSensitive ?? false),
            };
            if (options?.viewModel) options.viewModel.searchAtoms = atoms;
            return { ...atoms, anchorRef: { current: null } };
        },
    };
});
vi.mock("@/util/app-api", () => ({
    getApi: () => ({
        browserPanes: {
            find: async (id: string, text: string, opts: unknown) => void h.calls.push(`find ${id} ${JSON.stringify(text)} ${JSON.stringify(opts)}`),
            stopFind: async (id: string, clear?: boolean) => void h.calls.push(`stopFind ${id} clear=${clear}`),
        },
        listen: async (event: string, cb: (r: { block_id: string; count: number; active: number }) => void) => {
            h.calls.push(`listen ${event}`);
            h.result = cb;
            return () => {
                h.unlistened++;
            };
        },
    }),
}));

import { useBrowserFind } from "./browser-find";
import type { BrowserViewModel } from "./browser-model";

type Bar = ReturnType<typeof useBrowserFind>;

function mount(): { bar: Bar; model: BrowserViewModel; dispose: () => void } {
    const model = { blockId: "block-1" } as BrowserViewModel;
    let bar!: Bar;
    const dispose = createRoot((d) => {
        bar = useBrowserFind(model, () => undefined);
        return d;
    });
    return { bar, model, dispose };
}
const flush = async () => {
    for (let i = 0; i < 6; i++) await Promise.resolve();
};

beforeEach(() => {
    h.calls = [];
    h.result = null;
    h.unlistened = 0;
});
afterEach(() => vi.clearAllMocks());

describe("find in a browser pane's page", () => {
    it("hands the bar's atoms to the model, so the host can open it", async () => {
        const { bar, model, dispose } = mount();
        await flush();
        expect(model.searchAtoms?.isOpen()).toBe(false);
        // The bar says whose it is, so Ctrl+F finds this pane's box among several open bars.
        expect(bar.owner).toBe("block-1");
        dispose();
    });

    it("searches the page as the person types, and ends the search when the box empties", async () => {
        const { bar, dispose } = mount();
        await flush();
        bar.onSearch?.("");
        bar.onSearch?.("hel");
        bar.onSearch?.("hello");
        bar.onSearch?.("hello");
        bar.onSearch?.("");
        expect(h.calls.filter((c) => !c.startsWith("listen"))).toEqual([
            'find block-1 "hel" {"matchCase":false}',
            'find block-1 "hello" {"matchCase":false}',
            "stopFind block-1 clear=true",
        ]);
        dispose();
    });

    it("steps to the next and the previous match of the same text, and only once there is text", async () => {
        const { bar, dispose } = mount();
        await flush();
        bar.onNext?.();
        bar.onSearch?.("cat");
        bar.onNext?.();
        bar.onPrev?.();
        expect(h.calls.filter((c) => !c.startsWith("listen"))).toEqual([
            'find block-1 "cat" {"matchCase":false}',
            'find block-1 "cat" {"forward":true,"findNext":true,"matchCase":false}',
            'find block-1 "cat" {"forward":false,"findNext":true,"matchCase":false}',
        ]);
        dispose();
    });

    it("shows the host's count and current match, for this pane only", async () => {
        const { bar, dispose } = mount();
        await flush();
        bar.onSearch?.("cat");
        h.result?.({ block_id: "block-1", count: 12, active: 3 });
        expect([bar.resultsCount(), bar.resultsIndex()]).toEqual([12, 2]);
        h.result?.({ block_id: "other", count: 99, active: 9 });
        expect([bar.resultsCount(), bar.resultsIndex()]).toEqual([12, 2]);
        // CEF's -1 is "unchanged": a count-only update keeps the current match, a match-only one keeps the count.
        h.result?.({ block_id: "block-1", count: 20, active: -1 });
        expect([bar.resultsCount(), bar.resultsIndex()]).toEqual([20, 2]);
        h.result?.({ block_id: "block-1", count: -1, active: 5 });
        expect([bar.resultsCount(), bar.resultsIndex()]).toEqual([20, 4]);
        h.result?.({ block_id: "block-1", count: 12, active: 3 });
        // A result that arrives after the box emptied is the old search's.
        bar.onSearch?.("");
        h.result?.({ block_id: "block-1", count: 5, active: 1 });
        expect(bar.resultsCount()).toBe(12);
        dispose();
    });

    it("clears the page's highlights when the pane goes, and stops listening", async () => {
        const { bar, dispose } = mount();
        await flush();
        bar.onSearch?.("cat");
        h.calls = [];
        dispose();
        expect(h.calls).toEqual(["stopFind block-1 clear=true"]);
        expect(h.unlistened).toBe(1);
    });
});
