// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * User widgets load from srv's list of approved packages
 * (docs/specs/SPEC_USER_WIDGETS_AND_WIDGET_API_2026_10_09.md §8, §9). The real
 * import step goes through a blob URL, which a test runner can't import, so it
 * is injected here; everything around it — the Solid import rewrite,
 * validation, registration, reloading and unloading — is real.
 */

import { createRoot } from "solid-js";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { WidgetPackageInfo } from "@/app/store/rpc-api/widgets";
import { getPaneTab, registerPaneTab } from "./pane-tab-registry";
import {
    createWidgetSync,
    rewriteWidgetImports,
    syncWidgetPackages,
    trustedPaneManifest,
    type LoadedPackage,
    type PackageLoaderDeps,
} from "./widget-loader";
import { widgetPanesOf } from "./widget-panes";

const SAMPLE = "../../../docs/examples/widgets/hello/index.js";

const unregisters: (() => void)[] = [];
afterEach(() => {
    while (unregisters.length) unregisters.pop()!();
});

function pkg(over: Partial<WidgetPackageInfo> = {}): WidgetPackageInfo {
    const id = over.id ?? "agentmux.hello";
    return {
        id,
        name: "Hello",
        version: "1.0.0",
        description: null,
        author: null,
        homepage: null,
        icon: "hand",
        default_hue: null,
        kind: "trusted",
        permissions: [],
        granted: [],
        state: "approved",
        error: null,
        hash: "h1",
        panes: [
            { view: `ext:${id}/main`, name: "main", label: "Hello", icon: "hand", entry: "index.js", singleton: false, default_meta: {} },
        ],
        commands: [],
        status_items: [],
        signature: { state: "unsigned", publisher: "acme", fingerprint: null, pinned: null },
        files_url: "/agentmux/widget-files/x/h1/k/",
        implied: false,
        folder: `C:/Users/me/.agentmux/widgets/${id}`,
        ...over,
    };
}

function deps(exported: () => unknown, over: Partial<PackageLoaderDeps> = {}): PackageLoaderDeps {
    return {
        readModule: vi.fn(async () => "export default {}"),
        importModule: vi.fn(async () => ({ default: exported() })),
        register: (m) => {
            const u = registerPaneTab(m);
            unregisters.push(u);
            return u;
        },
        ...over,
    };
}

describe("rewriteWidgetImports", () => {
    const shims = { "solid-js": "blob:a", "solid-js/web": "blob:b", "solid-js/store": "blob:c" };

    it("points the widget's Solid imports at the app's own Solid", () => {
        const src = [
            `import { createSignal } from "solid-js";`,
            `import { render } from 'solid-js/web';`,
            `import "solid-js/store";`,
            `const m = await import("solid-js");`,
            `export { createMemo } from "solid-js";`,
        ].join("\n");
        expect(rewriteWidgetImports(src, shims)).toBe(
            [
                `import { createSignal } from "blob:a";`,
                `import { render } from 'blob:b';`,
                `import "blob:c";`,
                `const m = await import("blob:a");`,
                `export { createMemo } from "blob:a";`,
            ].join("\n")
        );
    });

    it("leaves other imports, and look-alike strings, alone", () => {
        const src = `import x from "solid-jsx"; const s = "from solid-js";`;
        expect(rewriteWidgetImports(src, shims)).toBe(src);
    });
});

describe("trustedPaneManifest", () => {
    it("takes the package's view, and fills label and icon from the package", async () => {
        const sample = (await import(SAMPLE)).default;
        const m = trustedPaneManifest(sample, pkg(), pkg().panes[0]);
        expect(typeof m).toBe("object");
        expect((m as { view: string }).view).toBe("ext:agentmux.hello/main");
    });

    it("rejects another contract version, a mismatched view, and no create", async () => {
        const sample = (await import(SAMPLE)).default;
        const p = pkg();
        expect(trustedPaneManifest({ ...sample, apiVersion: 2 }, p, p.panes[0])).toMatch(/apiVersion 2/);
        expect(trustedPaneManifest({ ...sample, view: "ext:other" }, p, p.panes[0])).toMatch(/declares view/);
        expect(trustedPaneManifest({ apiVersion: 1 }, p, p.panes[0])).toMatch(/create/);
        expect(trustedPaneManifest(null, p, p.panes[0])).toMatch(/default export/);
    });

    it("reads one pane's manifest out of { panes }", async () => {
        const sample = (await import(SAMPLE)).default;
        const p = pkg();
        const m = trustedPaneManifest({ panes: { main: sample } }, p, p.panes[0]);
        expect((m as { view: string }).view).toBe("ext:agentmux.hello/main");
        expect(trustedPaneManifest({ panes: { other: sample } }, p, p.panes[0])).toMatch(/no manifest/);
    });
});

describe("syncWidgetPackages", () => {
    it("loads an approved trusted package through its approved bytes", async () => {
        const sample = (await import(SAMPLE)).default;
        const d = deps(() => sample);
        const results = await syncWidgetPackages([pkg()], d, new Map());
        expect(results).toEqual([{ id: "agentmux.hello", ok: true, views: ["ext:agentmux.hello/main"] }]);
        expect(d.readModule).toHaveBeenCalledWith(expect.objectContaining({ id: "agentmux.hello", hash: "h1" }), "index.js");
        expect(getPaneTab("ext:agentmux.hello/main")?.label).toBe("Hello");
    });

    it("loads nothing that isn't approved", async () => {
        const sample = (await import(SAMPLE)).default;
        const d = deps(() => sample);
        for (const state of ["needs_approval", "changed", "invalid", "disabled"] as const) {
            expect(await syncWidgetPackages([pkg({ state })], d, new Map())).toEqual([]);
        }
        expect(d.readModule).not.toHaveBeenCalled();
        expect(getPaneTab("ext:agentmux.hello/main")).toBeUndefined();
    });

    it("unloads a package that is turned off, removed or no longer approved", async () => {
        const sample = (await import(SAMPLE)).default;
        const d = deps(() => sample);
        const loaded = new Map<string, LoadedPackage>();
        await syncWidgetPackages([pkg()], d, loaded);
        expect(getPaneTab("ext:agentmux.hello/main")).toBeDefined();
        await syncWidgetPackages([pkg({ state: "changed" })], d, loaded);
        expect(getPaneTab("ext:agentmux.hello/main")).toBeUndefined();
        expect(loaded.size).toBe(0);
    });

    it("reloads a package whose approved version changed", async () => {
        const sample = (await import(SAMPLE)).default;
        const d = deps(() => sample);
        const loaded = new Map<string, LoadedPackage>();
        await syncWidgetPackages([pkg()], d, loaded);
        const before = getPaneTab("ext:agentmux.hello/main");
        await syncWidgetPackages([pkg({ hash: "h2" })], d, loaded);
        const after = getPaneTab("ext:agentmux.hello/main");
        expect(after).toBeDefined();
        expect(after).not.toBe(before);
        expect(loaded.get("agentmux.hello")?.hash).toBe("h2");
        expect(d.readModule).toHaveBeenCalledTimes(2);
    });

    it("keeps nothing of a package that fails, and retries it on the next pass", async () => {
        const sample = (await import(SAMPLE)).default;
        let exported: unknown = { ...sample, apiVersion: 2 };
        const d = deps(() => exported);
        const loaded = new Map<string, LoadedPackage>();
        const [first] = await syncWidgetPackages([pkg()], d, loaded);
        expect(first.ok).toBe(false);
        expect(getPaneTab("ext:agentmux.hello/main")).toBeUndefined();
        exported = sample; // the author fixes it
        const [second] = await syncWidgetPackages([pkg()], d, loaded);
        expect(second.ok).toBe(true);
    });

    it("adds the package's commands and status items with it, and tracks its panes while open", async () => {
        const sample = (await import(SAMPLE)).default;
        const removed = vi.fn();
        const contribute = vi.fn(() => [removed]);
        const d = deps(() => sample, { contribute });
        const loaded = new Map<string, LoadedPackage>();
        await syncWidgetPackages([pkg()], d, loaded);
        expect(contribute).toHaveBeenCalledWith(expect.objectContaining({ id: "agentmux.hello" }));

        const manifest = getPaneTab("ext:agentmux.hello/main")!;
        const ctx = { blockId: "b9", meta: () => ({}), setMeta: async () => {}, isFocused: () => false, visibility: () => "active" as const };
        const inst = createRoot(() => manifest.create(ctx as never));
        expect(widgetPanesOf("ext:agentmux.hello/main").map(([b]) => b)).toEqual(["b9"]);
        inst.dispose?.();
        expect(widgetPanesOf("ext:agentmux.hello/main")).toEqual([]);

        await syncWidgetPackages([pkg({ state: "disabled" })], d, loaded);
        expect(removed).toHaveBeenCalledTimes(1);
    });

    it("doesn't load the same package twice when two passes overlap", async () => {
        const sample = (await import(SAMPLE)).default;
        const d = deps(() => sample);
        const sync = createWidgetSync(d, new Map<string, LoadedPackage>());
        await Promise.all([sync([pkg()]), sync([pkg()])]);
        expect(d.readModule).toHaveBeenCalledTimes(1);
        expect(getPaneTab("ext:agentmux.hello/main")).toBeDefined();
    });

    it("reloads at once a package whose new version arrived while the old one was loading", async () => {
        const sample = (await import(SAMPLE)).default;
        let release!: () => void;
        const gate = new Promise<void>((r) => (release = r));
        const slowFirst = vi.fn(async () => {
            if (slowFirst.mock.calls.length === 1) await gate;
            return "export default {}";
        });
        // A new version arrives while the old one is loading.
        const loaded = new Map<string, LoadedPackage>();
        const sync = createWidgetSync(deps(() => sample, { readModule: slowFirst }), loaded);
        const first = sync([pkg({ hash: "h1" })]);
        const second = sync([pkg({ hash: "h2" })]);
        release();
        await Promise.all([first, second]);
        expect(loaded.get("agentmux.hello")?.hash).toBe("h2");
    });

    it("unloads at once a package turned off while it was loading", async () => {
        const sample = (await import(SAMPLE)).default;
        const off = new Map<string, LoadedPackage>();
        let release2!: () => void;
        const gate2 = new Promise<void>((r) => (release2 = r));
        const slow = vi.fn(async () => {
            await gate2;
            return "export default {}";
        });
        const sync2 = createWidgetSync(deps(() => sample, { readModule: slow }), off);
        const a = sync2([pkg()]);
        const b = sync2([]);
        release2();
        await Promise.all([a, b]);
        expect(off.size).toBe(0);
        expect(getPaneTab("ext:agentmux.hello/main")).toBeUndefined();
    });

    it("gives a sandboxed package's panes the sandbox host, never its code in the app", async () => {
        const d = deps(() => ({}));
        const sandboxed = vi.fn(() => ({ apiVersion: 1 as const, view: "ext:acme.notes/main", label: "Notes", icon: "note-sticky", create: () => ({ component: () => document.createElement("div"), dispose() {} }) }));
        const results = await syncWidgetPackages(
            [pkg({ id: "acme.notes", kind: "sandboxed", panes: [{ view: "ext:acme.notes/main", name: "main", label: "Notes", icon: "note-sticky", entry: "index.html", singleton: false, default_meta: {} }] })],
            { ...d, sandboxed },
            new Map()
        );
        expect(results[0].ok).toBe(true);
        expect(sandboxed).toHaveBeenCalledTimes(1);
        expect(d.readModule).not.toHaveBeenCalled();
    });

    it("the sample runs through the host context only", async () => {
        const sample = (await import(SAMPLE)).default;
        const setMeta = vi.fn(async () => {});
        // Effects run once the root's setup returns, as they do in the app.
        const { el, inst, dispose } = createRoot((dispose) => {
            const inst = sample.create({
                blockId: "b1",
                meta: () => ({ "hello:clicks": 2 }),
                setMeta,
                isFocused: () => true,
                visibility: () => "active",
            });
            return { inst, el: inst.component({}) as HTMLElement, dispose };
        });
        expect(inst.liveTitle().text).toBe("Hello (2)");
        expect(el.querySelector("p")!.textContent).toBe("Clicked 2 times; shown 0 times; active.");
        inst.onActivate();
        expect(el.querySelector("p")!.textContent).toBe("Clicked 2 times; shown 1 times; active.");
        (el.querySelector("button") as HTMLButtonElement).click();
        expect(setMeta).toHaveBeenCalledWith({ "hello:clicks": 3 });
        dispose();
    });
});
