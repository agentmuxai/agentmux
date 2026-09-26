// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Pane Tab contract Phase 6: trusted, locally installed third-party pane tabs,
 * loaded from widgets.json (docs/specs/SPEC_PANE_TAB_CONTRACT_V1_2026_09_24.md
 * §4). The real import step goes through a blob URL, which a test runner
 * can't import, so it is injected here; everything around it — path
 * resolution, the Solid import rewrite, validation, registration — is real.
 */

import { createRoot } from "solid-js";
import { afterEach, describe, expect, it, vi } from "vitest";
import { getPaneTab, registerPaneTab } from "./pane-tab-registry";
import { loadWidgets, resolveWidgetModulePath, rewriteWidgetImports, type WidgetLoaderDeps } from "./widget-loader";

const SAMPLE = "../../../docs/examples/widgets/hello/index.js";

const unregisters: (() => void)[] = [];
afterEach(() => {
    while (unregisters.length) unregisters.pop()!();
});

function deps(exported: Record<string, unknown>, over: Partial<WidgetLoaderDeps> = {}): WidgetLoaderDeps {
    return {
        widgetsDir: "C:/Users/me/.agentmux/widgets",
        readText: vi.fn(async () => "export default {}"),
        importModule: vi.fn(async (_source: string, path: string) => ({ default: exported[path] })),
        register: (m) => {
            const u = registerPaneTab(m);
            unregisters.push(u);
            return u;
        },
        ...over,
    };
}

const entry = (view: string, module: string): WidgetConfigType =>
    ({ label: "Hello", icon: "hand", module, blockdef: { meta: { view } } }) as WidgetConfigType;

describe("resolveWidgetModulePath", () => {
    it("resolves a relative module under the widgets directory", () => {
        expect(resolveWidgetModulePath("hello/index.js", "C:/w")).toBe("C:/w/hello/index.js");
        expect(resolveWidgetModulePath("hello\\index.js", "C:\\w\\")).toBe("C:\\w\\hello\\index.js");
    });

    it("keeps an absolute path", () => {
        expect(resolveWidgetModulePath("/opt/w/a.js", "C:/w")).toBe("/opt/w/a.js");
        expect(resolveWidgetModulePath("D:/w/a.js", "C:/w")).toBe("D:/w/a.js");
    });
});

describe("rewriteWidgetImports", () => {
    const shims = { "solid-js": "blob:solid", "solid-js/web": "blob:web", "solid-js/store": "blob:store" };

    it("points the widget's Solid imports at the app's own Solid", () => {
        const src = [
            `import { createSignal } from "solid-js";`,
            `import { render } from 'solid-js/web';`,
            `import { createStore } from "solid-js/store";`,
            `import "solid-js";`,
            `const lazy = import("solid-js/web");`,
            `export * from "solid-js";`,
        ].join("\n");
        expect(rewriteWidgetImports(src, shims)).toBe(
            [
                `import { createSignal } from "blob:solid";`,
                `import { render } from 'blob:web';`,
                `import { createStore } from "blob:store";`,
                `import "blob:solid";`,
                `const lazy = import("blob:web");`,
                `export * from "blob:solid";`,
            ].join("\n")
        );
    });

    it("leaves other imports, and look-alike strings, alone", () => {
        const src = `import x from "solid-js-extra";\nconst s = "solid-js";\nimport y from "./solid-js";`;
        expect(rewriteWidgetImports(src, shims)).toBe(src);
    });
});

describe("loadWidgets", () => {
    it("loads, validates and registers the sample widget", async () => {
        const sample = (await import(SAMPLE)).default;
        const d = deps({ "C:/Users/me/.agentmux/widgets/hello/index.js": sample });
        const results = await loadWidgets({ "ext@hello": entry("ext:hello", "hello/index.js") }, d, new Set());

        expect(results).toEqual([{ key: "ext@hello", ok: true, view: "ext:hello" }]);
        expect(d.readText).toHaveBeenCalledWith("C:/Users/me/.agentmux/widgets/hello/index.js");
        const m = getPaneTab("ext:hello")!;
        expect(m.label).toBe("Hello");
        expect(m.create).toBeTypeOf("function");
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

    it("rejects a widget built for another contract version", async () => {
        const sample = (await import(SAMPLE)).default;
        const d = deps({ "C:/Users/me/.agentmux/widgets/hello/index.js": { ...sample, apiVersion: 2 } });
        const [r] = await loadWidgets({ "ext@hello": entry("ext:hello", "hello/index.js") }, d, new Set());
        expect(r.ok).toBe(false);
        expect((r as { reason: string }).reason).toMatch(/apiVersion 2/);
        expect(getPaneTab("ext:hello")).toBeUndefined();
    });

    it("rejects a module whose view isn't the one widgets.json names, or isn't ext:", async () => {
        const sample = (await import(SAMPLE)).default;
        const d = deps({
            "C:/Users/me/.agentmux/widgets/a.js": { ...sample, view: "ext:other" },
            "C:/Users/me/.agentmux/widgets/b.js": { ...sample, view: "term" },
        });
        const results = await loadWidgets(
            { "ext@a": entry("ext:hello", "a.js"), "ext@b": entry("term", "b.js") },
            d,
            new Set()
        );
        expect(results.map((r) => r.ok)).toEqual([false, false]);
        expect(getPaneTab("ext:hello")).toBeUndefined();
    });

    it("rejects a module without create, and one that fails to read or import", async () => {
        const d = deps(
            { "C:/Users/me/.agentmux/widgets/a.js": { apiVersion: 1, view: "ext:a", label: "A", icon: "a" } },
            {
                readText: vi.fn(async (p: string) => {
                    if (p.endsWith("gone.js")) throw new Error("ENOENT");
                    return "";
                }),
            }
        );
        const results = await loadWidgets(
            { "ext@a": entry("ext:a", "a.js"), "ext@gone": entry("ext:gone", "gone.js") },
            d,
            new Set()
        );
        expect(results.map((r) => r.ok)).toEqual([false, false]);
        expect((results[0] as { reason: string }).reason).toMatch(/create/);
        expect((results[1] as { reason: string }).reason).toMatch(/ENOENT/);
    });

    // ReAgent P1 on #3767: a failed widget used to be marked loaded, so fixing
    // it and reloading widgets.json never retried it.
    it("retries a widget that failed, on the next pass, once it's fixed", async () => {
        const sample = (await import(SAMPLE)).default;
        const path = "C:/Users/me/.agentmux/widgets/hello/index.js";
        const exported: Record<string, unknown> = { [path]: { ...sample, apiVersion: 2 } };
        const d = deps(exported);
        const loaded = new Set<string>();
        const widgets = { "ext@hello": entry("ext:hello", "hello/index.js") };

        const [first] = await loadWidgets(widgets, d, loaded);
        expect(first.ok).toBe(false);
        expect(getPaneTab("ext:hello")).toBeUndefined();

        exported[path] = sample; // the user fixes the widget
        const [second] = await loadWidgets(widgets, d, loaded);
        expect(second.ok).toBe(true);
        expect(getPaneTab("ext:hello")).toBeDefined();
    });

    it("doesn't load the same widget twice when two passes overlap", async () => {
        const sample = (await import(SAMPLE)).default;
        const d = deps({ "C:/Users/me/.agentmux/widgets/hello/index.js": sample });
        const loaded = new Set<string>();
        const widgets = { "ext@hello": entry("ext:hello", "hello/index.js") };
        const [a, b] = await Promise.all([loadWidgets(widgets, d, loaded), loadWidgets(widgets, d, loaded)]);
        expect([...a, ...b].filter((r) => r.ok)).toHaveLength(1);
        expect(d.readText).toHaveBeenCalledTimes(1);
    });

    it("skips built-in widgets and ones already loaded", async () => {
        const sample = (await import(SAMPLE)).default;
        const d = deps({ "C:/Users/me/.agentmux/widgets/hello/index.js": sample });
        const loaded = new Set<string>();
        const widgets = {
            "defwidget@terminal": { blockdef: { meta: { view: "term" } } } as WidgetConfigType,
            "ext@hello": entry("ext:hello", "hello/index.js"),
        };
        expect(await loadWidgets(widgets, d, loaded)).toHaveLength(1);
        expect(await loadWidgets(widgets, d, loaded)).toHaveLength(0);
        expect(d.readText).toHaveBeenCalledTimes(1);
    });
});
