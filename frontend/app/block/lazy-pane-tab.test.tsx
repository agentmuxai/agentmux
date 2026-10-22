// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { cleanup, fireEvent, render, screen } from "@solidjs/testing-library";
import { createRoot, onCleanup } from "solid-js";
import { afterEach, describe, expect, it, vi } from "vitest";

const claims = vi.hoisted(() => [] as string[]);
vi.mock("@/app/store/focusManager", () => ({
    focusManager: {
        claimFocusOnMount: (blockId: string, giveFocus: () => boolean) => {
            claims.push(blockId);
            giveFocus();
        },
    },
}));
import { lazyPaneTab, memberMismatch } from "./lazy-pane-tab";
import type { PaneTabHostContext, PaneTabInstance } from "./pane-tab-registry";

const ctx = { blockId: "b1", meta: () => ({}) } as unknown as PaneTabHostContext;

/** A load the test resolves (or rejects) when it wants. */
function deferredLoad() {
    let resolve!: (m: { create(ctx: PaneTabHostContext): PaneTabInstance }) => void;
    let reject!: (e: Error) => void;
    const load = vi.fn(
        () =>
            new Promise<{ create(ctx: PaneTabHostContext): PaneTabInstance }>((res, rej) => {
                resolve = res;
                reject = rej;
            })
    );
    return { load, resolve: (m: { create(ctx: PaneTabHostContext): PaneTabInstance }) => resolve(m), reject: (e: Error) => reject(e) };
}

const flush = () => new Promise((r) => setTimeout(r, 0));

afterEach(() => {
    cleanup();
    vi.restoreAllMocks();
});

describe("lazyPaneTab", () => {
    it("keeps the static part and gives the stand-in exactly the declared members", () => {
        const { load } = deferredLoad();
        const manifest = lazyPaneTab({ apiVersion: 1, view: "demo", label: "Demo", icon: "star", members: ["liveTitle", "headerActions"], load });
        expect(manifest.label).toBe("Demo");
        expect("members" in manifest).toBe(false);
        createRoot((dispose) => {
            const instance = manifest.create(ctx);
            expect(Object.keys(instance).sort()).toEqual(["component", "dispose", "headerActions", "liveTitle"]);
            dispose();
        });
    });

    it("forwards to the real instance once it loads, in the stand-in's reactive root", async () => {
        const { load, resolve } = deferredLoad();
        const cleaned = vi.fn();
        const manifest = lazyPaneTab({ apiVersion: 1, view: "demo", label: "Demo", icon: "star", members: ["liveTitle", "headerActions"], load });
        let instance!: PaneTabInstance;
        const dispose = createRoot((d) => {
            instance = manifest.create(ctx);
            return d;
        });
        expect(instance.liveTitle!()).toEqual({ text: "Demo", placeholder: true });
        expect(instance.headerActions!()).toEqual([]);
        resolve({
            create: () => {
                onCleanup(cleaned);
                return {
                    component: () => <div>real view</div>,
                    liveTitle: () => ({ text: "Loaded" }),
                    headerActions: () => [{ elemtype: "iconbutton", icon: "x" } as IconButtonDecl],
                };
            },
        });
        await flush();
        expect(instance.liveTitle!()).toEqual({ text: "Loaded" });
        expect(instance.headerActions!()).toHaveLength(1);
        render(() => <instance.component ctx={ctx} />);
        expect(screen.getByText("real view")).toBeTruthy();
        dispose();
        expect(cleaned).toHaveBeenCalled();
    });

    it("never builds the real instance for a pane closed before its code arrived", async () => {
        const { load, resolve } = deferredLoad();
        const create = vi.fn(() => ({ component: () => null }));
        const manifest = lazyPaneTab({ apiVersion: 1, view: "demo", label: "Demo", icon: "star", members: [], load });
        createRoot((dispose) => {
            manifest.create(ctx);
            dispose();
        });
        resolve({ create });
        await flush();
        expect(create).not.toHaveBeenCalled();
    });

    it("shows a failed load in the pane, with a retry that loads again", async () => {
        vi.spyOn(console, "error").mockImplementation(() => {});
        const load = vi
            .fn()
            .mockRejectedValueOnce(new Error("chunk missing"))
            .mockResolvedValueOnce({ create: () => ({ component: () => <div>second try</div> }) });
        const manifest = lazyPaneTab({ apiVersion: 1, view: "demo", label: "Demo", icon: "star", members: [], load });
        render(() => {
            const instance = manifest.create(ctx);
            return <instance.component ctx={ctx} />;
        });
        await flush();
        expect(screen.getByRole("alert").textContent).toContain("Demo failed to load: chunk missing");
        fireEvent.click(screen.getByText("Try again"));
        await flush();
        expect(load).toHaveBeenCalledTimes(2);
        expect(screen.getByText("second try")).toBeTruthy();
    });

    it("reports a real instance whose members differ from the declared ones", async () => {
        const error = vi.spyOn(console, "error").mockImplementation(() => {});
        const manifest = lazyPaneTab({
            apiVersion: 1,
            view: "demo",
            label: "Demo",
            icon: "star",
            members: ["liveTitle"],
            load: async () => ({ create: () => ({ component: () => null, focus: () => true }) }),
        });
        createRoot(() => manifest.create(ctx));
        await flush();
        expect(error.mock.calls[0][0]).toContain("demo: its lazy manifest's members differ");
        expect(error.mock.calls[0][0]).toContain("+focus, -liveTitle");
    });

    it("hands focus to the real instance through the focus manager, and only in a visible pane", async () => {
        const focus = vi.fn(() => true);
        const manifest = lazyPaneTab({
            apiVersion: 1,
            view: "demo",
            label: "Demo",
            icon: "star",
            members: ["focus"],
            load: async () => ({ create: () => ({ component: () => null, focus }) }),
        });
        claims.length = 0;
        const visible = { ...ctx, visibility: () => "active" } as PaneTabHostContext;
        createRoot(() => manifest.create(visible));
        await flush();
        expect(claims).toEqual(["b1"]);
        expect(focus).toHaveBeenCalledOnce();

        const hidden = { ...ctx, visibility: () => "dormant" } as PaneTabHostContext;
        createRoot(() => manifest.create(hidden));
        await flush();
        expect(claims).toEqual(["b1"]);
    });

    it("memberMismatch ignores component, dispose and members left undefined", () => {
        expect(memberMismatch(["focus"], { component: () => null, dispose: () => {}, focus: () => true, rename: undefined })).toEqual([]);
    });

    it("fills the pane with the pane surface while the code loads, not nothing", async () => {
        const { load, resolve } = deferredLoad();
        const manifest = lazyPaneTab({ apiVersion: 1, view: "demo", label: "Demo", icon: "star", members: [], load });
        const { container } = render(() => {
            const instance = manifest.create(ctx);
            return <instance.component ctx={ctx} />;
        });
        expect(container.querySelector(".lazy-pane-standin")).not.toBeNull();
        resolve({ create: () => ({ component: () => <div>real view</div> }) });
        await flush();
        expect(container.querySelector(".lazy-pane-standin")).toBeNull();
        expect(screen.getByText("real view")).toBeTruthy();
    });

    it("loads once for the panes waiting on it, and builds later panes at once, with no stand-in", async () => {
        const { load, resolve } = deferredLoad();
        const real = () => ({ component: () => <div>real view</div>, liveTitle: () => ({ text: "Loaded" }) });
        const create = vi.fn(real);
        const manifest = lazyPaneTab({ apiVersion: 1, view: "demo", label: "Demo", icon: "star", members: ["liveTitle"], load });
        const waiting = createRoot(() => [manifest.create(ctx), manifest.create(ctx)]);
        expect(load).toHaveBeenCalledOnce();
        resolve({ create });
        await flush();
        expect(create).toHaveBeenCalledTimes(2);
        expect(waiting.map((i) => i.liveTitle!())).toEqual([{ text: "Loaded" }, { text: "Loaded" }]);

        // The module is here: the next pane is the real instance, synchronously.
        const { container } = render(() => {
            const instance = manifest.create(ctx);
            expect(instance.liveTitle!()).toEqual({ text: "Loaded" });
            return <instance.component ctx={ctx} />;
        });
        expect(container.querySelector(".lazy-pane-standin")).toBeNull();
        expect(screen.getByText("real view")).toBeTruthy();
        expect(load).toHaveBeenCalledOnce();
    });
});
