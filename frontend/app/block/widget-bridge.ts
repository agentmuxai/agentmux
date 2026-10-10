// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The widget bridge's request handler: one place where each protocol 1 method
 * (docs/specs/SPEC_USER_WIDGETS_AND_WIDGET_API_2026_10_09.md §6.3) is
 * checked and answered. Pure apart from the `BridgeHost` it acts through, so
 * the protocol is tested without an iframe.
 */

import type { WidgetPackageInfo, WidgetPaneInfo } from "@/app/store/rpc-api/widgets";
import type { PaneTabHostContext } from "./pane-tab-registry";

export const ERR = {
    INVALID_PARAMS: -32602,
    METHOD_NOT_FOUND: -32601,
    PERMISSION_DENIED: 1001,
    LIMIT_EXCEEDED: 1002,
    NOT_FOUND: 1003,
    UNAVAILABLE: 1005,
    NOT_READY: 1007,
    INTERNAL: 1099,
} as const;

const META_LIMIT_BYTES = 64 * 1024;
const MAX_ACTIONS = 4;
const MAX_MENU_ITEMS = 20;

export interface BridgeState {
    ready: boolean;
    inFlight: number;
    lastMetaSent?: string;
}

export interface BridgeHost {
    pkg: WidgetPackageInfo;
    pane: WidgetPaneInfo;
    ctx: PaneTabHostContext;
    setTitle(t: { text: string; icon?: string } | null): void;
    setActions(a: { id: string; icon: string; title: string }[]): void;
    setMenu(items: ({ id: string; label: string; disabled?: boolean } | { separator: true })[]): void;
    toast?(text: string, kind: "info" | "success" | "warning" | "error"): void;
    openUrl?(url: string): Promise<void>;
    openPane?(view: string, meta: Record<string, unknown>, split: "right" | "down" | "tab"): Promise<string>;
    agentmuxVersion?(): string;
    hostName?(): string;
}

export type BridgeReply = { result: unknown } | { error: { code: number; message: string; data?: unknown } };

const ok = (result: unknown = {}): BridgeReply => ({ result });
const fail = (code: number, message: string, data?: unknown): BridgeReply => ({ error: { code, message, ...(data ? { data } : {}) } });

/**
 * The value an app variable resolves to, as a concrete CSS value: a widget's
 * document can't see the app's variables, so a value that is itself a
 * `var(...)` (or a font shorthand) would mean nothing there. A hidden probe
 * element applies the variable to `property` and reads the computed result.
 */
function resolveVar(name: string, property: "color" | "font-family" | "border-top-left-radius", fallback: string): string {
    const probe = document.createElement("span");
    probe.style.cssText = "position:absolute;visibility:hidden;pointer-events:none;border-style:solid;";
    probe.style.setProperty(property, `var(${name})`);
    document.documentElement.appendChild(probe);
    try {
        const value = getComputedStyle(probe).getPropertyValue(property).trim();
        // An unset variable leaves the property at its initial value.
        return value && value !== "0px" && value !== "rgba(0, 0, 0, 0)" ? value : fallback;
    } finally {
        probe.remove();
    }
}

/** The app's colors and fonts as the widget's CSS variables (§6.3 `Theme`). */
export function bridgeTheme(pkg: Pick<WidgetPackageInfo, "default_hue">): { mode: "dark" | "light"; vars: Record<string, string> } {
    const light = document.documentElement.classList.contains("light") || document.documentElement.dataset.theme === "light";
    const color = (name: string, fallback: string) => resolveVar(name, "color", fallback);
    return {
        mode: light ? "light" : "dark",
        vars: {
            "--am-bg": color("--main-bg-color", light ? "#ffffff" : "#1e1e1e"),
            "--am-fg": color("--main-text-color", light ? "#1f1f1f" : "#e6e6e6"),
            "--am-muted": color("--secondary-text-color", "#9a9a9a"),
            "--am-accent": color("--accent-color", "#58c142"),
            "--am-border": color("--border-color", "#3a3a3a"),
            "--am-error": color("--error-color", "#e5484d"),
            "--am-warning": color("--warning-color", "#f5a623"),
            "--am-success": color("--success-color", "#46a758"),
            "--am-font": resolveVar("--font-sans", "font-family", "system-ui, sans-serif"),
            "--am-font-mono": resolveVar("--font-mono", "font-family", "ui-monospace, monospace"),
            "--am-radius": resolveVar("--radius-md", "border-top-left-radius", "6px"),
            "--am-pane-hue": String(pkg.default_hue ?? 210),
        },
    };
}

const isObj = (x: unknown): x is Record<string, unknown> => x != null && typeof x === "object" && !Array.isArray(x);
const str = (x: unknown, max = 500): x is string => typeof x === "string" && x.length <= max;

/** The permission a method needs, or null (§6.3 table). */
export function permissionFor(method: string, params: Record<string, unknown>, pkg: WidgetPackageInfo): string | null {
    if (method.startsWith("storage.")) return "storage";
    if (method.startsWith("files.")) return "files";
    if (method === "clipboard.writeText") return "clipboard:write";
    if (method === "agents.list") return "agents:read";
    if (method === "agents.send") return "agents:send";
    if (method === "panes.open") {
        const own = pkg.panes.some((p) => p.view === params.view);
        return own ? null : "panes";
    }
    if (method === "net.fetch") {
        try {
            const u = new URL(String(params.url));
            const granted = pkg.granted.filter((g) => g.startsWith("net:")).map((g) => g.slice(4));
            const match = granted.find((origin) => originMatches(origin, u));
            return match ? null : `net:${u.origin}`;
        } catch {
            return "net:";
        }
    }
    return null;
}

/** Does a granted `net:` origin cover `url`? Exact origin, or `*.domain`
 *  for its subdomains (never the bare domain, never another port). */
export function originMatches(granted: string, url: URL): boolean {
    let g: URL;
    try {
        g = new URL(granted.replace("://*.", "://wildcard-placeholder."));
    } catch {
        return false;
    }
    if (g.protocol !== url.protocol || g.port !== url.port) return false;
    if (granted.includes("://*.")) {
        const domain = g.hostname.slice("wildcard-placeholder.".length);
        return url.hostname.endsWith(`.${domain}`);
    }
    return g.hostname === url.hostname;
}

export async function handleBridgeRequest(host: BridgeHost, state: BridgeState, method: string, params: unknown): Promise<BridgeReply> {
    const p = isObj(params) ? params : {};
    const { pkg, pane, ctx } = host;
    if (method === "hello") {
        if (p.protocol !== 1) return fail(ERR.INVALID_PARAMS, `this AgentMux speaks widget protocol 1, the widget asked for ${String(p.protocol)}`);
        state.ready = true;
        const meta = ownMetaOf(pkg.id, ctx.meta());
        state.lastMetaSent = JSON.stringify(meta);
        return ok({
            protocol: 1,
            widget: { id: pkg.id, version: pkg.version, pane: pane.name },
            agentmux: { version: host.agentmuxVersion?.() ?? "", host: host.hostName?.() ?? "" },
            permissions: pkg.granted,
            theme: bridgeTheme(pkg),
            meta,
            visibility: ctx.visibility(),
            focused: ctx.isFocused(),
        });
    }
    if (!state.ready) return fail(ERR.NOT_READY, "say hello first");

    const needs = permissionFor(method, p, pkg);
    if (needs && !pkg.granted.includes(needs)) {
        return fail(ERR.PERMISSION_DENIED, `${pkg.name} wasn't granted "${needs}"`, { permission: needs });
    }

    switch (method) {
        case "meta.get":
            return ok({ meta: ownMetaOf(pkg.id, ctx.meta()) });
        case "meta.set": {
            if (!isObj(p.patch)) return fail(ERR.INVALID_PARAMS, "meta.set needs { patch: object }");
            const next = { ...ownMetaOf(pkg.id, ctx.meta()) };
            for (const [k, v] of Object.entries(p.patch)) {
                if (!str(k, 200) || !k) return fail(ERR.INVALID_PARAMS, "meta keys are non-empty strings");
                if (v === null) delete next[k];
                else next[k] = v;
            }
            if (new TextEncoder().encode(JSON.stringify(next)).length > META_LIMIT_BYTES) {
                return fail(ERR.LIMIT_EXCEEDED, "a pane's widget meta is limited to 64 KB", { limit: "meta" });
            }
            const prefix = `widget:${pkg.id}:`;
            const patch: Record<string, unknown> = {};
            for (const [k, v] of Object.entries(p.patch)) patch[prefix + k] = v;
            state.lastMetaSent = JSON.stringify(next);
            await ctx.setMeta(patch);
            return ok();
        }
        case "ui.setTitle":
            if (!str(p.text, 200)) return fail(ERR.INVALID_PARAMS, "ui.setTitle needs { text } (at most 200 characters)");
            host.setTitle({ text: p.text, ...(str(p.icon, 60) ? { icon: p.icon } : {}) });
            return ok();
        case "ui.setHeaderActions": {
            const list = Array.isArray(p.actions) ? p.actions : null;
            if (!list || list.length > MAX_ACTIONS) return fail(ERR.INVALID_PARAMS, `ui.setHeaderActions takes up to ${MAX_ACTIONS} actions`);
            const actions = [];
            for (const a of list) {
                if (!isObj(a) || !str(a.id, 60) || !str(a.icon, 60) || !str(a.title, 120)) {
                    return fail(ERR.INVALID_PARAMS, "each action is { id, icon, title }");
                }
                actions.push({ id: a.id, icon: a.icon, title: a.title });
            }
            host.setActions(actions);
            return ok();
        }
        case "ui.setContextMenu": {
            const list = Array.isArray(p.items) ? p.items : null;
            if (!list || list.length > MAX_MENU_ITEMS) return fail(ERR.INVALID_PARAMS, `ui.setContextMenu takes up to ${MAX_MENU_ITEMS} items`);
            const items: ({ id: string; label: string; disabled?: boolean } | { separator: true })[] = [];
            for (const it of list) {
                if (isObj(it) && it.separator === true) items.push({ separator: true });
                else if (isObj(it) && str(it.id, 60) && str(it.label, 120)) items.push({ id: it.id, label: it.label, disabled: it.disabled === true });
                else return fail(ERR.INVALID_PARAMS, "each item is { id, label, disabled? } or { separator: true }");
            }
            host.setMenu(items);
            return ok();
        }
        case "ui.toast": {
            if (!str(p.text, 500)) return fail(ERR.INVALID_PARAMS, "ui.toast needs { text }");
            const kind = ["info", "success", "warning", "error"].includes(String(p.kind)) ? (p.kind as "info") : "info";
            host.toast?.(`${pkg.name}: ${p.text}`, kind);
            return ok();
        }
        case "ui.openUrl": {
            let u: URL;
            try {
                u = new URL(String(p.url));
            } catch {
                return fail(ERR.INVALID_PARAMS, "ui.openUrl needs an http(s) url");
            }
            if (u.protocol !== "http:" && u.protocol !== "https:") return fail(ERR.INVALID_PARAMS, "only http and https links open");
            if (!host.openUrl) return fail(ERR.UNAVAILABLE, "this AgentMux can't open links from widgets");
            await host.openUrl(u.toString());
            return ok();
        }
        case "theme.get":
            return ok(bridgeTheme(pkg));
        case "panes.open": {
            if (!str(p.view, 200)) return fail(ERR.INVALID_PARAMS, "panes.open needs { view }");
            const split = p.split === "down" || p.split === "tab" ? p.split : "right";
            const meta = isObj(p.meta) ? p.meta : {};
            if (!host.openPane) return fail(ERR.UNAVAILABLE, "this AgentMux can't open panes from widgets");
            return ok({ pane: await host.openPane(p.view, meta, split) });
        }
        case "storage.get":
        case "storage.set":
        case "storage.delete":
        case "storage.list":
        case "net.fetch":
        case "files.pick":
        case "files.save":
        case "clipboard.writeText":
        case "agents.list":
        case "agents.send":
            return fail(ERR.UNAVAILABLE, `${method} isn't available in this AgentMux yet`);
        default:
            return fail(ERR.METHOD_NOT_FOUND, `unknown method ${method}`);
    }
}

function ownMetaOf(id: string, meta: Record<string, unknown> | null | undefined): Record<string, unknown> {
    const prefix = `widget:${id}:`;
    const out: Record<string, unknown> = {};
    for (const [k, v] of Object.entries(meta ?? {})) if (k.startsWith(prefix)) out[k.slice(prefix.length)] = v;
    return out;
}
