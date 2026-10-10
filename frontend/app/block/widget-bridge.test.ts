// @vitest-environment jsdom
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** The widget bridge, protocol 1: every request is checked and answered here
 *  (docs/specs/SPEC_USER_WIDGETS_AND_WIDGET_API_2026_10_09.md §6.3–§6.6). */

import { describe, expect, it, vi } from "vitest";
import type { WidgetPackageInfo } from "@/app/store/rpc-api/widgets";
import { ERR, handleBridgeRequest, originMatches, permissionFor, type BridgeHost, type BridgeState } from "./widget-bridge";

function pkg(over: Partial<WidgetPackageInfo> = {}): WidgetPackageInfo {
    return {
        id: "acme.notes",
        name: "Notes",
        version: "1.0.0",
        description: null,
        author: null,
        homepage: null,
        icon: "note-sticky",
        default_hue: 120,
        kind: "sandboxed",
        permissions: [],
        granted: [],
        state: "approved",
        error: null,
        hash: "h",
        panes: [
            { view: "ext:acme.notes/main", name: "main", label: "Notes", icon: "note-sticky", entry: "index.html", singleton: false, default_meta: {} },
            { view: "ext:acme.notes/list", name: "list", label: "List", icon: "list", entry: "list.html", singleton: false, default_meta: {} },
        ],
        files_url: "/agentmux/widget-files/acme.notes/h/k/",
        implied: false,
        folder: "",
        ...over,
    };
}

function host(p = pkg(), meta: Record<string, unknown> = {}) {
    const current: Record<string, unknown> = { ...meta };
    const h = {
        pkg: p,
        pane: p.panes[0],
        ctx: {
            blockId: "b1",
            meta: () => current,
            setMeta: vi.fn(async (patch: Record<string, unknown>) => {
                for (const [k, v] of Object.entries(patch)) {
                    if (v === null) delete current[k];
                    else current[k] = v;
                }
            }),
            isFocused: () => true,
            visibility: () => "active" as const,
        },
        setTitle: vi.fn(),
        setActions: vi.fn(),
        setMenu: vi.fn(),
        toast: vi.fn(),
        openUrl: vi.fn(async () => {}),
        openPane: vi.fn(async () => "b2"),
        agentmuxVersion: () => "0.60.0",
        hostName: () => "narko",
        currentMeta: () => current,
    };
    return h as typeof h & BridgeHost;
}

async function ready(h: BridgeHost): Promise<BridgeState> {
    const state: BridgeState = { ready: false, inFlight: 0 };
    const r = await handleBridgeRequest(h, state, "hello", { protocol: 1 });
    expect("result" in r).toBe(true);
    return state;
}

type Err = { code: number; message: string; data?: Record<string, unknown> };
const errorOf = (r: unknown): Err => (r as { error: Err }).error;

describe("the handshake", () => {
    it("answers hello with the widget, AgentMux, grants, theme, meta and visibility", async () => {
        const h = host(pkg({ granted: ["storage"] }), { "widget:acme.notes:filter": "open", view: "ext:acme.notes/main" });
        const state: BridgeState = { ready: false, inFlight: 0 };
        const r = (await handleBridgeRequest(h, state, "hello", { protocol: 1 })) as { result: Record<string, any> };
        expect(r.result.protocol).toBe(1);
        expect(r.result.widget).toEqual({ id: "acme.notes", version: "1.0.0", pane: "main" });
        expect(r.result.agentmux).toEqual({ version: "0.60.0", host: "narko" });
        expect(r.result.permissions).toEqual(["storage"]);
        // Its own keys only, without the prefix.
        expect(r.result.meta).toEqual({ filter: "open" });
        expect(r.result.visibility).toBe("active");
        expect(r.result.theme.vars["--am-pane-hue"]).toBe("120");
        expect(state.ready).toBe(true);
    });

    it("refuses everything before hello, and a protocol it doesn't speak", async () => {
        const h = host();
        const state: BridgeState = { ready: false, inFlight: 0 };
        expect(errorOf(await handleBridgeRequest(h, state, "meta.get", {})).code).toBe(ERR.NOT_READY);
        expect(errorOf(await handleBridgeRequest(h, state, "hello", { protocol: 2 })).code).toBe(ERR.INVALID_PARAMS);
        expect(state.ready).toBe(false);
    });
});

describe("meta", () => {
    it("reads and writes only the widget's namespace, and a null deletes", async () => {
        const h = host(pkg(), { "widget:acme.notes:a": 1, "widget:other.x:b": 2, "cmd:cwd": "/" });
        const state = await ready(h);
        expect(((await handleBridgeRequest(h, state, "meta.get", {})) as { result: unknown }).result).toEqual({ meta: { a: 1 } });
        await handleBridgeRequest(h, state, "meta.set", { patch: { a: null, b: "x" } });
        expect(h.ctx.setMeta).toHaveBeenCalledWith({ "widget:acme.notes:a": null, "widget:acme.notes:b": "x" });
        expect(h.currentMeta()).toEqual({ "widget:other.x:b": 2, "cmd:cwd": "/", "widget:acme.notes:b": "x" });
    });

    it("caps a pane's widget meta at 64 KB", async () => {
        const h = host();
        const state = await ready(h);
        const e = errorOf(await handleBridgeRequest(h, state, "meta.set", { patch: { big: "x".repeat(70_000) } }));
        expect(e.code).toBe(ERR.LIMIT_EXCEEDED);
        expect(h.ctx.setMeta).not.toHaveBeenCalled();
    });
});

describe("ui", () => {
    it("sets the title, header actions and menu within their limits", async () => {
        const h = host();
        const state = await ready(h);
        await handleBridgeRequest(h, state, "ui.setTitle", { text: "Notes (3)" });
        expect(h.setTitle).toHaveBeenCalledWith({ text: "Notes (3)" });
        await handleBridgeRequest(h, state, "ui.setHeaderActions", { actions: [{ id: "r", icon: "rotate", title: "Refresh" }] });
        expect(h.setActions).toHaveBeenCalledWith([{ id: "r", icon: "rotate", title: "Refresh" }]);
        const five = Array.from({ length: 5 }, (_, i) => ({ id: `a${i}`, icon: "x", title: "x" }));
        expect(errorOf(await handleBridgeRequest(h, state, "ui.setHeaderActions", { actions: five })).code).toBe(ERR.INVALID_PARAMS);
        await handleBridgeRequest(h, state, "ui.setContextMenu", { items: [{ id: "c", label: "Clear" }, { separator: true }] });
        expect(h.setMenu).toHaveBeenCalledWith([{ id: "c", label: "Clear", disabled: false }, { separator: true }]);
    });

    it("toasts under the widget's own name, and opens only http(s) links", async () => {
        const h = host();
        const state = await ready(h);
        await handleBridgeRequest(h, state, "ui.toast", { text: "Saved", kind: "success" });
        expect(h.toast).toHaveBeenCalledWith("Notes: Saved", "success");
        expect(errorOf(await handleBridgeRequest(h, state, "ui.openUrl", { url: "file:///etc/passwd" })).code).toBe(ERR.INVALID_PARAMS);
        expect(errorOf(await handleBridgeRequest(h, state, "ui.openUrl", { url: "javascript:alert(1)" })).code).toBe(ERR.INVALID_PARAMS);
        await handleBridgeRequest(h, state, "ui.openUrl", { url: "https://example.com/x" });
        expect(h.openUrl).toHaveBeenCalledWith("https://example.com/x");
    });
});

describe("permissions", () => {
    it("opens the package's own views freely, any other view only with `panes`", async () => {
        const h = host();
        const state = await ready(h);
        expect("result" in (await handleBridgeRequest(h, state, "panes.open", { view: "ext:acme.notes/list" }))).toBe(true);
        const e = errorOf(await handleBridgeRequest(h, state, "panes.open", { view: "term" }));
        expect(e.code).toBe(ERR.PERMISSION_DENIED);
        expect(e.data).toEqual({ permission: "panes" });
        const granted = host(pkg({ granted: ["panes"] }));
        const s2 = await ready(granted);
        expect("result" in (await handleBridgeRequest(granted, s2, "panes.open", { view: "term" }))).toBe(true);
    });

    it("refuses a method whose permission wasn't granted, naming it", async () => {
        const h = host();
        const state = await ready(h);
        const cases: [string, string][] = [
            ["storage.get", "storage"],
            ["files.pick", "files"],
            ["clipboard.writeText", "clipboard:write"],
            ["agents.list", "agents:read"],
            ["agents.send", "agents:send"],
        ];
        for (const [method, permission] of cases) {
            const e = errorOf(await handleBridgeRequest(h, state, method, { key: "k", text: "t", agent: "a" }));
            expect(e.code, method).toBe(ERR.PERMISSION_DENIED);
            expect(e.data).toEqual({ permission });
        }
        const e = errorOf(await handleBridgeRequest(h, state, "net.fetch", { url: "https://api.github.com/user" }));
        expect(e.data).toEqual({ permission: "net:https://api.github.com" });
    });

    it("matches net origins exactly, subdomains only under *., ports included", () => {
        const u = (s: string) => new URL(s);
        expect(originMatches("https://api.github.com", u("https://api.github.com/x"))).toBe(true);
        expect(originMatches("https://api.github.com", u("http://api.github.com/x"))).toBe(false);
        expect(originMatches("https://api.github.com", u("https://evil.api.github.com/x"))).toBe(false);
        expect(originMatches("https://*.example.com", u("https://a.example.com/x"))).toBe(true);
        expect(originMatches("https://*.example.com", u("https://example.com/x"))).toBe(false);
        expect(originMatches("https://*.example.com", u("https://example.com.evil.net/x"))).toBe(false);
        expect(originMatches("http://127.0.0.1:8188", u("http://127.0.0.1:8188/prompt"))).toBe(true);
        expect(originMatches("http://127.0.0.1:8188", u("http://127.0.0.1:9999/"))).toBe(false);
        const p = pkg({ granted: ["net:https://api.github.com"] });
        expect(permissionFor("net.fetch", { url: "https://api.github.com/user" }, p)).toBeNull();
    });

    it("answers an unknown method with method-not-found", async () => {
        const h = host();
        const state = await ready(h);
        expect(errorOf(await handleBridgeRequest(h, state, "app.quit", {})).code).toBe(ERR.METHOD_NOT_FOUND);
    });
});

describe("the pane host the loader imports", () => {
    // The loader imports "./sandboxed-widget-host" with no extension: a stale
    // .ts beside the real .tsx would win and leave sandboxed widgets unable
    // to run (a merge brought W1's placeholder back once).
    it("is the iframe host, not a placeholder", async () => {
        const host = await import("./sandboxed-widget-host");
        expect(host.IFRAME_SANDBOX).toContain("allow-scripts");
        expect(host.IFRAME_SANDBOX).not.toContain("allow-same-origin");
    });
});
