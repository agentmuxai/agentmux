// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Loads user widgets: the widget packages srv lists as approved
 * (docs/specs/SPEC_USER_WIDGETS_AND_WIDGET_API_2026_10_09.md §8, §9), and,
 * through srv, the v1 `widgets.json` module entries of Pane Tab contract
 * Phase 6 (docs/specs/SPEC_PANE_TAB_CONTRACT_V1_2026_09_24.md §4), which srv
 * lists as implied trusted packages.
 *
 * Nothing loads because a file appeared on disk: srv lists a package as
 * `approved` only after the user approved it in AgentMux's own UI, and it
 * serves only the approved bytes.
 *
 * A trusted package's module is read over `widgets.readfile` (the approved
 * bytes), its `solid-js` imports are pointed at the app's own Solid (one
 * reactive runtime), and it is imported from a blob URL. Its default export
 * is a pane tab manifest for its one pane, or `{ panes: { <name>: manifest } }`.
 * A sandboxed package's panes are iframe hosts (`sandboxed-widget-host.ts`).
 *
 * When a package's approved version changes (an update, an edit approved
 * again), its pane types are unregistered and registered again; open panes
 * rebuild (`block.tsx`).
 */

import { createEffect, createRoot, createSignal } from "solid-js";
import * as solid from "solid-js";
import * as solidStore from "solid-js/store";
import * as solidWeb from "solid-js/web";
import { RpcApi } from "@/app/store/rpc-api";
import type { WidgetPackageInfo, WidgetPaneInfo } from "@/app/store/rpc-api/widgets";
import { TabRpcClient } from "@/app/store/rpc-util";
import { startWidgetPackages, widgetPackages } from "@/app/store/widget-packages-store";
import { registerPaneTab, type PaneTabManifest } from "./pane-tab-registry";
import { makeSandboxedPaneManifest } from "./sandboxed-widget-host";

/** The contract version this host implements for trusted widgets. A widget
 *  built for another is rejected rather than half-working. */
export const PANE_TAB_API_VERSION = 1;

const SHARED_MODULES = ["solid-js", "solid-js/web", "solid-js/store"] as const;

export interface PackageLoaderDeps {
    /** A trusted package's module text, as approved. */
    readModule(pkg: WidgetPackageInfo, path: string): Promise<string>;
    importModule(source: string, label: string): Promise<{ default?: unknown }>;
    register?(manifest: PaneTabManifest): () => void;
    /** A sandboxed pane's manifest (an iframe host). */
    sandboxed?(pkg: WidgetPackageInfo, pane: WidgetPaneInfo): PaneTabManifest;
}

/** A loaded package: the version it was loaded at and how to take it down. */
export interface LoadedPackage {
    hash: string;
    unregister: (() => void)[];
}

export type PackageLoadResult = { id: string; ok: true; views: string[] } | { id: string; ok: false; reason: string };

/** Points the widget's bare `solid-js` imports (static, side-effect, dynamic,
 *  re-export) at `shims`. Anything else is left exactly as written. */
export function rewriteWidgetImports(source: string, shims: Record<string, string>): string {
    return source.replace(
        /(\bfrom\s*|\bimport\s*\(\s*|\bimport\s+)(["'])(solid-js(?:\/web|\/store)?)\2/g,
        (whole, lead: string, quote: string, spec: string) =>
            spec in shims ? `${lead}${quote}${shims[spec]}${quote}` : whole
    );
}

/** The manifest a trusted module exports for `pane`, checked (spec §9). */
export function trustedPaneManifest(exported: unknown, pkg: WidgetPackageInfo, pane: WidgetPaneInfo): PaneTabManifest | string {
    if (exported == null || typeof exported !== "object") return "its default export isn't a pane tab manifest";
    const many = (exported as { panes?: Record<string, unknown> }).panes;
    const raw = many && typeof many === "object" && !("create" in (exported as object)) ? many[pane.name] : exported;
    if (raw == null || typeof raw !== "object") return `it exports no manifest for its pane "${pane.name}"`;
    const m = raw as Partial<PaneTabManifest>;
    if (m.apiVersion !== PANE_TAB_API_VERSION) {
        return `it targets pane tab apiVersion ${String(m.apiVersion)}, this AgentMux implements ${PANE_TAB_API_VERSION}`;
    }
    if (m.view != null && m.view !== pane.view) return `it declares view "${String(m.view)}", its package opens "${pane.view}"`;
    if (typeof m.create !== "function") return "it has no create(ctx)";
    // The pre-contract class path is gone; say so rather than ignore it.
    if ((m as { viewModelClass?: unknown }).viewModelClass != null) return "a widget can't supply a ViewModel class";
    return {
        ...(m as PaneTabManifest),
        view: pane.view,
        label: typeof m.label === "string" && m.label ? m.label : pane.label,
        icon: typeof m.icon === "string" && m.icon ? m.icon : pane.icon,
        defaultHue: m.defaultHue ?? pkg.default_hue ?? undefined,
    };
}

/**
 * Brings the registered pane tabs in line with `packages`: loads each
 * approved package not yet loaded at its current hash, and unloads any that
 * is gone, disabled, changed or no longer approved. `loaded` is the state
 * between passes. Run passes through createWidgetSync, one at a time.
 */
export async function syncWidgetPackages(
    packages: WidgetPackageInfo[],
    deps: PackageLoaderDeps,
    loaded: Map<string, LoadedPackage>
): Promise<PackageLoadResult[]> {
    const want = new Map(packages.filter((p) => p.state === "approved").map((p) => [p.id, p]));
    for (const [id, l] of [...loaded]) {
        if (want.get(id)?.hash !== l.hash) {
            for (const u of l.unregister) u();
            loaded.delete(id);
            console.log(`[widget-loader] unloaded ${id}`);
        }
    }
    const results: PackageLoadResult[] = [];
    const register = deps.register ?? registerPaneTab;
    for (const pkg of want.values()) {
        if (loaded.has(pkg.id)) continue;
        const unregister: (() => void)[] = [];
        try {
            const manifests: PaneTabManifest[] = [];
            for (const pane of pkg.panes) {
                if (pkg.kind === "sandboxed") {
                    const make = deps.sandboxed ?? makeSandboxedPaneManifest;
                    manifests.push(make(pkg, pane));
                    continue;
                }
                const source = await deps.readModule(pkg, pane.entry);
                const mod = await deps.importModule(source, `${pkg.id}/${pane.entry}`);
                const m = trustedPaneManifest(mod?.default, pkg, pane);
                if (typeof m === "string") throw new Error(m);
                manifests.push(m);
            }
            for (const m of manifests) unregister.push(register(m));
            loaded.set(pkg.id, { hash: pkg.hash, unregister });
            results.push({ id: pkg.id, ok: true, views: manifests.map((m) => m.view) });
            console.log(`[widget-loader] loaded ${pkg.id} ${pkg.version} (${pkg.kind})`);
        } catch (e) {
            for (const u of unregister) u();
            const reason = e instanceof Error ? e.message : String(e);
            results.push({ id: pkg.id, ok: false, reason });
            console.warn(`[widget-loader] ${pkg.id} not loaded: ${reason}`);
        }
    }
    return results;
}

/**
 * Runs passes one at a time, each with the newest list it was given. A list
 * that arrives while a pass is loading starts another pass when it ends, so
 * a load that finished against an older list (a new hash, or a package since
 * turned off) is put right at once rather than at the next change.
 */
export function createWidgetSync(
    deps: PackageLoaderDeps,
    loaded: Map<string, LoadedPackage>,
    onResults: (results: PackageLoadResult[]) => void = () => {}
): (packages: WidgetPackageInfo[]) => Promise<void> {
    let latest: WidgetPackageInfo[] = [];
    let running: Promise<void> | null = null;
    let again = false;
    return (packages) => {
        latest = packages;
        if (running) {
            again = true;
            return running;
        }
        running = (async () => {
            do {
                again = false;
                onResults(await syncWidgetPackages(latest, deps, loaded));
            } while (again);
        })().finally(() => {
            running = null;
        });
        return running;
    };
}

// ── Load errors, for Settings → Widgets ─────────────────────────────────────

const [loadErrors, setLoadErrors] = createSignal<Record<string, string>>({});

/** The last load error of each package that failed to load in this window. */
export function widgetLoadErrors(): Record<string, string> {
    return loadErrors();
}

function noteResults(results: PackageLoadResult[]): void {
    if (!results.length) return;
    setLoadErrors((prev) => {
        const next = { ...prev };
        for (const r of results) {
            if ("reason" in r) next[r.id] = r.reason;
            else delete next[r.id];
        }
        return next;
    });
}

// ── The real import step ────────────────────────────────────────────────────

let shimUrls: Record<string, string> | null = null;

/** Blob modules that re-export the app's own Solid instances. */
function sharedModuleShims(): Record<string, string> {
    if (shimUrls) return shimUrls;
    const host = { "solid-js": solid, "solid-js/web": solidWeb, "solid-js/store": solidStore };
    (globalThis as { __agentmuxPaneTabHost?: unknown }).__agentmuxPaneTabHost = host;
    shimUrls = {};
    for (const spec of SHARED_MODULES) {
        const names = Object.keys(host[spec]).filter((n) => n !== "default" && /^[A-Za-z_$][\w$]*$/.test(n));
        const src =
            `const m = globalThis.__agentmuxPaneTabHost[${JSON.stringify(spec)}];\n` +
            `export default m;\n` +
            names.map((n) => `export const ${n} = m.${n};`).join("\n");
        shimUrls[spec] = URL.createObjectURL(new Blob([src], { type: "text/javascript" }));
    }
    return shimUrls;
}

async function importFromBlob(source: string): Promise<{ default?: unknown }> {
    const url = URL.createObjectURL(new Blob([rewriteWidgetImports(source, sharedModuleShims())], { type: "text/javascript" }));
    try {
        return await import(/* @vite-ignore */ url);
    } finally {
        URL.revokeObjectURL(url);
    }
}

let firstPass: Promise<void> | null = null;

/**
 * Loads the approved widgets now and again whenever srv's list changes. Call
 * once at startup; the returned promise settles when the first pass has, so
 * app-init can wait for it before the first render (a pane that mounts before
 * its widget registers rebuilds when it does, but a first frame with the
 * right view is better).
 */
export function startWidgetLoader(): Promise<void> {
    if (firstPass) return firstPass;
    const deps: PackageLoaderDeps = {
        readModule: async (pkg, path) =>
            (await RpcApi.WidgetsReadFileCommand(TabRpcClient, { id: pkg.id, hash: pkg.hash, path })).content,
        importModule: (source) => importFromBlob(source),
    };
    const sync = createWidgetSync(deps, new Map<string, LoadedPackage>(), noteResults);
    firstPass = startWidgetPackages().then(
        () =>
            new Promise<void>((resolve) => {
                createRoot(() => {
                    const list = widgetPackages();
                    let first = true;
                    createEffect(() => {
                        const pass = sync(list());
                        if (first) {
                            first = false;
                            void pass.finally(resolve);
                        }
                    });
                });
            })
    );
    return firstPass;
}
