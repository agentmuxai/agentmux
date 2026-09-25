// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Loads third-party pane tabs — Pane Tab contract v1, Phase 6
 * (docs/specs/SPEC_PANE_TAB_CONTRACT_V1_2026_09_24.md §4, and §5: trusted,
 * locally installed ES modules, no sandbox).
 *
 * A `widgets.json` entry with a `module` names a widget's ES module (relative
 * to `~/.agentmux/widgets/`, or absolute) and, in `blockdef.meta.view`, its
 * `ext:` view. The loader reads the module's text over the existing
 * `readeditorfile` RPC, points its `solid-js` imports at the app's own Solid
 * (one reactive runtime — a second copy's signals wouldn't track inside the
 * app's effects), imports it from a blob URL, validates the default export
 * and registers it. Anything that fails is logged with its reason and never
 * registered.
 */

import { agentmuxHome } from "@/app/view/agent/agent-launch-env";
import { atoms } from "@/app/store/global";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import * as solid from "solid-js";
import { createEffect, createRoot } from "solid-js";
import * as solidStore from "solid-js/store";
import * as solidWeb from "solid-js/web";
import { registerPaneTab, type PaneTabManifest } from "./pane-tab-registry";

/** The contract version this host implements. A widget built for another is
 *  rejected rather than half-working. */
export const PANE_TAB_API_VERSION = 1;

const SHARED_MODULES = ["solid-js", "solid-js/web", "solid-js/store"] as const;

export interface WidgetLoaderDeps {
    widgetsDir: string;
    readText: (path: string) => Promise<string>;
    /** Evaluates the (rewritten) module source; `path` is for messages. */
    importModule: (source: string, path: string) => Promise<{ default?: unknown }>;
    register?: (manifest: PaneTabManifest) => () => void;
}

export type WidgetLoadResult = { key: string; ok: true; view: string } | { key: string; ok: false; reason: string };

export function resolveWidgetModulePath(module: string, widgetsDir: string): string {
    if (/^([A-Za-z]:[\\/]|[\\/])/.test(module)) return module;
    const sep = widgetsDir.includes("\\") && !widgetsDir.includes("/") ? "\\" : "/";
    return widgetsDir.replace(/[\\/]+$/, "") + sep + module.replace(/^[\\/]+/, "");
}

/** Points the widget's bare `solid-js` imports (static, side-effect, dynamic,
 *  re-export) at `shims`. Anything else is left exactly as written. */
export function rewriteWidgetImports(source: string, shims: Record<string, string>): string {
    return source.replace(
        /(\bfrom\s*|\bimport\s*\(\s*|\bimport\s+)(["'])(solid-js(?:\/web|\/store)?)\2/g,
        (whole, lead: string, quote: string, spec: string) =>
            spec in shims ? `${lead}${quote}${shims[spec]}${quote}` : whole
    );
}

function validate(exported: unknown, entry: WidgetConfigType, view: string): PaneTabManifest | string {
    if (exported == null || typeof exported !== "object") return "its default export isn't a pane tab manifest";
    const m = exported as Partial<PaneTabManifest>;
    if (m.apiVersion !== PANE_TAB_API_VERSION) {
        return `it targets pane tab apiVersion ${String(m.apiVersion)}, this AgentMux implements ${PANE_TAB_API_VERSION}`;
    }
    if (m.view !== view) return `it declares view "${String(m.view)}", widgets.json opens "${view}"`;
    if (typeof m.create !== "function") return "it has no create(ctx)";
    if (m.viewModelClass != null) return "a widget can't supply a ViewModel class";
    return {
        ...(m as PaneTabManifest),
        label: typeof m.label === "string" && m.label ? m.label : entry.label || view,
        icon: typeof m.icon === "string" && m.icon ? m.icon : entry.icon || "square",
    };
}

/**
 * Loads every `ext:` widget in `widgets` whose key isn't in `loaded`. A key is
 * in `loaded` while its attempt is in flight (so an overlapping pass skips it)
 * and after it succeeds; a failed attempt removes it, so the next pass — the
 * next widgets.json change — retries a widget the user has since fixed
 * (ReAgent P1 on #3767).
 */
export async function loadWidgets(
    widgets: Record<string, WidgetConfigType> | undefined,
    deps: WidgetLoaderDeps,
    loaded: Set<string>
): Promise<WidgetLoadResult[]> {
    const results: WidgetLoadResult[] = [];
    for (const [key, entry] of Object.entries(widgets ?? {})) {
        if (!entry?.module || loaded.has(key)) continue;
        loaded.add(key);
        const view = entry.blockdef?.meta?.view as string | undefined;
        const path = resolveWidgetModulePath(entry.module, deps.widgetsDir);
        const fail = (reason: string): void => {
            loaded.delete(key);
            results.push({ key, ok: false, reason });
            console.warn(`[widget-loader] ${key} (${path}) not loaded: ${reason}`);
        };
        if (!view?.startsWith("ext:")) {
            fail(`its blockdef view "${String(view)}" isn't an ext: view`);
            continue;
        }
        try {
            const source = await deps.readText(path);
            const mod = await deps.importModule(source, path);
            const manifest = validate(mod?.default, entry, view);
            if (typeof manifest === "string") {
                fail(manifest);
                continue;
            }
            (deps.register ?? registerPaneTab)(manifest);
            results.push({ key, ok: true, view });
            console.log(`[widget-loader] loaded ${key} as ${view} from ${path}`);
        } catch (e) {
            fail(e instanceof Error ? e.message : String(e));
        }
    }
    return results;
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
 * Loads the configured widgets now and again whenever the widget config
 * changes. Call once at startup; the returned promise settles when the first
 * pass has, so app-init can wait for it BEFORE the first render — a persisted
 * `ext:` pane that mounted before its widget registered would build the
 * default view model and never rebuild (`Block` only does on a view change).
 */
export function startWidgetLoader(): Promise<void> {
    if (firstPass) return firstPass;
    const loaded = new Set<string>();
    const deps: WidgetLoaderDeps = {
        widgetsDir: `${agentmuxHome()}/widgets`,
        readText: async (path) => (await RpcApi.ReadEditorFileCommand(TabRpcClient, { path })).content,
        importModule: (source) => importFromBlob(source),
    };
    let first: Promise<unknown> | null = null;
    createRoot(() => {
        createEffect(() => {
            const pass = loadWidgets(atoms.fullConfigAtom()?.widgets, deps, loaded);
            first ??= pass;
        });
    });
    firstPass = (first ?? Promise.resolve()).then(() => undefined);
    return firstPass;
}
