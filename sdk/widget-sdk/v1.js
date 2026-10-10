// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * @agentmuxai/widget-sdk v1: the client for a sandboxed AgentMux widget
 * (docs/specs/SPEC_USER_WIDGETS_AND_WIDGET_API_2026_10_09.md §6, §7).
 *
 *   import { connect } from "/agentmux/widget-sdk/v1.js";
 *   const am = await connect();
 *   am.ui.setTitle("Hello");
 *
 * The widget's page runs in a sandboxed iframe with no access to AgentMux
 * itself; everything goes through this client, over the MessagePort AgentMux
 * hands the page when it loads. No dependencies.
 */

export const SDK_VERSION = "1.0.0";
export const PROTOCOL = 1;

const ERROR_NAMES = {
    [-32700]: "parse_error",
    [-32600]: "invalid_request",
    [-32601]: "method_not_found",
    [-32602]: "invalid_params",
    1001: "permission_denied",
    1002: "limit_exceeded",
    1003: "not_found",
    1004: "network_error",
    1005: "unavailable",
    1006: "cancelled",
    1007: "not_ready",
    1099: "internal",
};

/** An error answer from AgentMux: `code`, `name` (e.g. "permission_denied"),
 *  `message`, `data` (e.g. `{ permission: "storage" }`). */
export class AgentMuxError extends Error {
    constructor(code, message, data) {
        super(message);
        this.code = code;
        this.name = ERROR_NAMES[code] ?? "error";
        this.data = data ?? null;
    }
}

/** Base64 for binary data (files, `net.fetch` bodies). */
export const base64 = {
    /** @param {Uint8Array | ArrayBuffer} bytes */
    encode(bytes) {
        const u8 = bytes instanceof Uint8Array ? bytes : new Uint8Array(bytes);
        let s = "";
        for (let i = 0; i < u8.length; i += 0x8000) s += String.fromCharCode.apply(null, u8.subarray(i, i + 0x8000));
        return btoa(s);
    },
    /** @param {string} text @returns {Uint8Array} */
    decode(text) {
        const bin = atob(text);
        const out = new Uint8Array(bin.length);
        for (let i = 0; i < bin.length; i++) out[i] = bin.charCodeAt(i);
        return out;
    },
};

function applyThemeVars(theme) {
    if (!theme || typeof document === "undefined") return;
    const root = document.documentElement;
    for (const [name, value] of Object.entries(theme.vars ?? {})) root.style.setProperty(name, String(value));
    root.dataset.amTheme = theme.mode;
    root.style.colorScheme = theme.mode === "light" ? "light" : "dark";
}

/** Wait for AgentMux to hand this page its port. */
function waitForPort(timeoutMs) {
    return new Promise((resolve, reject) => {
        if (typeof window === "undefined" || window.parent === window) {
            reject(new Error("This page isn't running inside AgentMux: open it as an AgentMux widget."));
            return;
        }
        const timer = setTimeout(() => {
            window.removeEventListener("message", onMessage);
            reject(new Error(`AgentMux didn't connect within ${timeoutMs} ms. Is this page an installed, approved widget?`));
        }, timeoutMs);
        function onMessage(ev) {
            if (ev.source !== window.parent) return;
            const d = ev.data;
            if (!d || d.type !== "agentmux:connect" || !ev.ports || !ev.ports[0]) return;
            if (Array.isArray(d.protocols) && !d.protocols.includes(PROTOCOL)) {
                clearTimeout(timer);
                window.removeEventListener("message", onMessage);
                reject(new Error(`This AgentMux speaks widget protocol ${d.protocols.join(", ")}; this SDK needs ${PROTOCOL}.`));
                return;
            }
            clearTimeout(timer);
            window.removeEventListener("message", onMessage);
            resolve(ev.ports[0]);
        }
        window.addEventListener("message", onMessage);
    });
}

class Client {
    constructor(port) {
        this.port = port;
        this.nextId = 1;
        this.pending = new Map();
        this.listeners = new Map();
        this.closed = false;
        port.onmessage = (ev) => this.receive(ev.data);
        port.start?.();
    }

    receive(msg) {
        if (!msg || msg.jsonrpc !== "2.0") return;
        if (msg.id != null && this.pending.has(msg.id)) {
            const p = this.pending.get(msg.id);
            this.pending.delete(msg.id);
            if (msg.error) p.reject(new AgentMuxError(msg.error.code, msg.error.message, msg.error.data));
            else p.resolve(msg.result);
            return;
        }
        if (msg.method && msg.id == null) {
            if (msg.method === "dispose") this.closed = true;
            for (const cb of this.listeners.get(msg.method) ?? []) {
                try {
                    cb(msg.params ?? {});
                } catch (e) {
                    console.error(`[agentmux widget] a "${msg.method}" handler threw`, e);
                }
            }
        }
    }

    call(method, params = {}) {
        if (this.closed) return Promise.reject(new AgentMuxError(1007, "this widget's pane is closing"));
        const id = this.nextId++;
        return new Promise((resolve, reject) => {
            this.pending.set(id, { resolve, reject });
            this.port.postMessage({ jsonrpc: "2.0", id, method, params });
        });
    }

    on(event, cb) {
        if (!this.listeners.has(event)) this.listeners.set(event, new Set());
        this.listeners.get(event).add(cb);
        return () => this.listeners.get(event)?.delete(cb);
    }
}

/** A `net.fetch` result, with `text()`, `json()` and `bytes()`. */
function wrapResponse(r) {
    return {
        ...r,
        ok: r.status >= 200 && r.status < 300,
        text: () => (r.body != null ? r.body : new TextDecoder().decode(base64.decode(r.bodyBase64 ?? ""))),
        json() {
            return JSON.parse(this.text());
        },
        bytes: () => (r.bodyBase64 != null ? base64.decode(r.bodyBase64) : new TextEncoder().encode(r.body ?? "")),
    };
}

/**
 * Connect to AgentMux. Resolves to the client once the handshake is done.
 *
 * @param {{ applyTheme?: boolean, timeoutMs?: number }} [options]
 *   `applyTheme` (default true) sets the theme's CSS variables on the page and
 *   keeps them current; `timeoutMs` (default 10000).
 */
export async function connect(options = {}) {
    const { applyTheme = true, timeoutMs = 10000 } = options;
    const port = await waitForPort(timeoutMs);
    const c = new Client(port);
    const info = await c.call("hello", { protocol: PROTOCOL, sdk: SDK_VERSION });
    if (applyTheme) {
        applyThemeVars(info.theme);
        c.on("theme", applyThemeVars);
    }
    const call = (method, params) => c.call(method, params);
    return {
        /** What `hello` returned: widget id/version/pane, AgentMux version, granted permissions, theme, meta, visibility, focused. */
        info,
        /** Subscribe to an event (`visibility`, `focus`, `theme`, `meta`, `action`, `dispose`); returns an unsubscribe. */
        on: (event, cb) => c.on(event, cb),
        /** Any method, including ones this SDK has no wrapper for. */
        call,
        meta: {
            get: async () => (await call("meta.get")).meta,
            set: (patch) => call("meta.set", { patch }),
        },
        ui: {
            setTitle: (text, icon) => call("ui.setTitle", icon ? { text, icon } : { text }),
            setHeaderActions: (actions) => call("ui.setHeaderActions", { actions }),
            setContextMenu: (items) => call("ui.setContextMenu", { items }),
            toast: (text, kind) => call("ui.toast", kind ? { text, kind } : { text }),
            openUrl: (url) => call("ui.openUrl", { url }),
        },
        theme: {
            get: () => call("theme.get"),
        },
        panes: {
            open: (view, meta, split) => call("panes.open", { view, ...(meta ? { meta } : {}), ...(split ? { split } : {}) }),
        },
        storage: {
            get: async (key) => (await call("storage.get", { key })).value,
            set: (key, value) => call("storage.set", { key, value }),
            delete: (key) => call("storage.delete", { key }),
            list: async (prefix) => (await call("storage.list", prefix ? { prefix } : {})).keys,
        },
        net: {
            /** Like `fetch`, through AgentMux, to an origin the widget declared. */
            fetch: async (url, init = {}) => {
                const params = { url };
                if (init.method) params.method = init.method;
                if (init.headers) params.headers = init.headers;
                if (init.timeoutMs) params.timeoutMs = init.timeoutMs;
                if (init.body instanceof Uint8Array || init.body instanceof ArrayBuffer) params.bodyBase64 = base64.encode(init.body);
                else if (init.body != null) params.body = String(init.body);
                return wrapResponse(await call("net.fetch", params));
            },
        },
        files: {
            pick: async (opts = {}) => (await call("files.pick", opts)).files.map((f) => ({ ...f, bytes: () => base64.decode(f.dataBase64) })),
            save: async (name, data, type) =>
                (
                    await call("files.save", {
                        name,
                        ...(type ? { type } : {}),
                        dataBase64: typeof data === "string" ? base64.encode(new TextEncoder().encode(data)) : base64.encode(data),
                    })
                ).saved,
        },
        clipboard: {
            writeText: (text) => call("clipboard.writeText", { text }),
        },
        agents: {
            list: async () => (await call("agents.list")).agents,
            send: async (agent, text) => (await call("agents.send", { agent, text })).id,
        },
    };
}

/**
 * Pause work while the pane is hidden: calls `onActive` when it's shown (and
 * now, if it is) and `onDormant` when it's hidden. Returns an unsubscribe.
 */
export function useVisibility(am, { onActive, onDormant } = {}) {
    let last = am.info?.visibility;
    if (last === "active") onActive?.();
    return am.on("visibility", ({ state }) => {
        if (state === last) return;
        last = state;
        if (state === "active") onActive?.();
        else onDormant?.();
    });
}
