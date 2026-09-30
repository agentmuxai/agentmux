// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * A tab's layout model keeps hearing backend layout updates for as long as the
 * model lives, whoever created it. A window tab torn off and dropped back
 * unmounts the view that first asked for the model; the model stays cached,
 * and a pane redocked into the tab arrives as a backend layout update.
 */

import { createRoot } from "solid-js";
import { afterEach, describe, expect, it, vi } from "vitest";

const hub = vi.hoisted(() => ({
    updates: 0,
    models: [] as { disposed: boolean }[],
    setWorkspace: (_ws: { tabids: string[] } | null) => {},
}));
let setLayoutState: (v: number) => void = () => {};

vi.mock("@/app/hook/useDimensions", () => ({ useOnResize: () => {} }));
vi.mock("@/app/store/global", async () => {
    const { createSignal: signal } = await import("solid-js");
    const [workspace, setWorkspace] = signal<{ tabids: string[] } | null>(null);
    hub.setWorkspace = setWorkspace;
    return {
        atoms: { activeTabId: () => "tab-1", workspace },
        MOS: {
            makeORef: (otype: string, oid: string) => `${otype}:${oid}`,
            getMuxObjectAtom: (oref: string) => () => ({ oid: oref.slice("tab:".length) }),
        },
    };
});
vi.mock("@/layout/lib/layoutAtom", async () => {
    const { createSignal: signal } = await import("solid-js");
    return {
        getLayoutStateAtomFromTab: () => {
            const [get, set] = signal(0);
            setLayoutState = set;
            return get;
        },
    };
});
vi.mock("@/layout/lib/layoutModel", async () => {
    const { createRoot: root, getOwner, runWithOwner } = await import("solid-js");
    class FakeLayoutModel {
        private owner: ReturnType<typeof getOwner> = null;
        private disposeRoot: () => void = () => {};
        disposed = false;
        constructor() {
            root((dispose) => {
                this.disposeRoot = dispose;
                this.owner = getOwner();
            });
            hub.models.push(this);
        }
        runInModelRoot<T>(fn: () => T): T {
            return runWithOwner(this.owner, fn) as T;
        }
        onBackendUpdate() {
            hub.updates++;
        }
        dispose() {
            this.disposed = true;
            this.disposeRoot();
        }
    }
    return { LayoutModel: FakeLayoutModel };
});

import {
    deleteLayoutModelForTab,
    getLayoutModelForTabById,
    installLayoutModelEviction,
} from "@/layout/lib/layoutModelHooks";

afterEach(() => {
    deleteLayoutModelForTab("tab-1");
    deleteLayoutModelForTab("tab-2");
    hub.setWorkspace(null);
    hub.updates = 0;
    hub.models = [];
});

describe("layout model backend subscription", () => {
    it("outlives the view that first asked for the model", () => {
        // The tab's view creates the model, then unmounts (the window tab is
        // torn off).
        createRoot((disposeView) => {
            getLayoutModelForTabById("tab-1");
            disposeView();
        });
        const before = hub.updates;
        // Back in the window: a redock lands as a backend layout update.
        setLayoutState(1);
        expect(hub.updates).toBe(before + 1);
        // And the same cached model is reused.
        getLayoutModelForTabById("tab-1");
        expect(hub.models).toHaveLength(1);
    });

    it("stops with the model: a deleted tab's model hears nothing more", () => {
        getLayoutModelForTabById("tab-1");
        deleteLayoutModelForTab("tab-1");
        const before = hub.updates;
        setLayoutState(2);
        expect(hub.updates).toBe(before);
    });
});

describe("layout model eviction", () => {
    it("disposes a tab's model when the tab leaves this window, and a returning tab gets a fresh one", () => {
        const stop = installLayoutModelEviction();
        hub.setWorkspace({ tabids: ["tab-1", "tab-2"] });
        const first = getLayoutModelForTabById("tab-1") as unknown as { disposed: boolean };
        hub.setWorkspace({ tabids: ["tab-2"] }); // tab-1 torn off
        expect(first.disposed).toBe(true);
        hub.setWorkspace({ tabids: ["tab-1", "tab-2"] }); // dropped back
        const again = getLayoutModelForTabById("tab-1");
        expect(again).not.toBe(first);
        stop();
    });

    it("leaves the tabs that stay alone", () => {
        const stop = installLayoutModelEviction();
        hub.setWorkspace({ tabids: ["tab-1", "tab-2"] });
        const kept = getLayoutModelForTabById("tab-2") as unknown as { disposed: boolean };
        hub.setWorkspace({ tabids: ["tab-2"] });
        expect(kept.disposed).toBe(false);
        stop();
    });
});
