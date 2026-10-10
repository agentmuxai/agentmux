// @vitest-environment jsdom
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The widget SDK (sdk/widget-sdk/v1.js) against the real bridge handler,
 * joined by a MessageChannel the way an iframe and its pane host are: the
 * protocol conformance test of the SDK
 * (docs/specs/SPEC_USER_WIDGETS_AND_WIDGET_API_2026_10_09.md §7).
 */

import { afterEach, describe, expect, it, vi } from "vitest";
import type { WidgetPackageInfo } from "@/app/store/rpc-api/widgets";
import { handleBridgeRequest, type BridgeHost, type BridgeState } from "./widget-bridge";
// A plain JS module; its types are sdk/widget-sdk/v1.d.ts.
import { AgentMuxError, base64, connect, useVisibility } from "../../../sdk/widget-sdk/v1.js";

const fakeParent = {} as Window;
const realParent = Object.getOwnPropertyDescriptor(window, "parent");

afterEach(() => {
    if (realParent) Object.defineProperty(window, "parent", realParent);
});

function pkg(granted: string[] = []): WidgetPackageInfo {
    return {
        id: "acme.notes",
        name: "Notes",
        version: "1.2.0",
        description: null,
        author: null,
        homepage: null,
        icon: "note-sticky",
        default_hue: null,
        kind: "sandboxed",
        permissions: granted,
        granted,
        state: "approved",
        error: null,
        hash: "h",
        panes: [{ view: "ext:acme.notes/main", name: "main", label: "Notes", icon: "note-sticky", entry: "index.html", singleton: false, default_meta: {} }],
        commands: [],
        status_items: [{ id: "count", text: "Notes", icon: "note-sticky", tooltip: null, command: null, alignment: "right" }],
        files_url: "/x/",
        implied: false,
        folder: "",
    };
}

/** A pane host speaking the bridge on port1; the "iframe" gets port2. */
function startHost(p: WidgetPackageInfo) {
    let meta: Record<string, unknown> = { "widget:acme.notes:clicks": 4 };
    const setTitle = vi.fn();
    const setStatusItem = vi.fn();
    const host: BridgeHost = {
        pkg: p,
        pane: p.panes[0],
        ctx: {
            blockId: "b1",
            meta: () => meta,
            setMeta: async (patch: Record<string, unknown>) => {
                meta = { ...meta, ...patch };
            },
            isFocused: () => false,
            visibility: () => "active",
        } as never,
        setTitle,
        setActions: vi.fn(),
        setMenu: vi.fn(),
        agentmuxVersion: () => "0.60.0",
        setStatusItem,
    };
    const state: BridgeState = { ready: false, inFlight: 0 };
    const channel = new MessageChannel();
    channel.port1.onmessage = async (ev) => {
        const m = ev.data;
        const reply = await handleBridgeRequest(host, state, m.method, m.params);
        channel.port1.postMessage({ jsonrpc: "2.0", id: m.id, ...reply });
    };
    channel.port1.start();
    Object.defineProperty(window, "parent", { value: fakeParent, configurable: true });
    const handOver = () =>
        window.dispatchEvent(new MessageEvent("message", { data: { type: "agentmux:connect", protocols: [1] }, source: fakeParent as never, ports: [channel.port2] }));
    const notify = (method: string, params: unknown) => channel.port1.postMessage({ jsonrpc: "2.0", method, params });
    return { handOver, notify, setTitle, setStatusItem, meta: () => meta };
}

describe("@agentmuxai/widget-sdk v1", () => {
    it("connects, applies the theme, and reads its pane's meta", async () => {
        const h = startHost(pkg());
        const pending = connect();
        h.handOver();
        const am = await pending;
        expect(am.info.widget).toEqual({ id: "acme.notes", version: "1.2.0", pane: "main" });
        expect(am.info.meta).toEqual({ clicks: 4 });
        expect(document.documentElement.style.getPropertyValue("--am-accent")).not.toBe("");
        await am.ui.setTitle("Notes (4)");
        expect(h.setTitle).toHaveBeenCalledWith({ text: "Notes (4)" });
        await am.meta.set({ clicks: 5 });
        expect(h.meta()["widget:acme.notes:clicks"]).toBe(5);
        expect(await am.meta.get()).toEqual({ clicks: 5 });
    });

    it("rejects with AgentMuxError naming the missing permission", async () => {
        const h = startHost(pkg());
        const pending = connect();
        h.handOver();
        const am = await pending;
        const err = (await am.storage.get("k").catch((e: unknown) => e)) as AgentMuxError;
        expect(err).toBeInstanceOf(AgentMuxError);
        expect(err.name).toBe("permission_denied");
        expect(err.data).toEqual({ permission: "storage" });
    });

    it("delivers events, and useVisibility pauses and resumes", async () => {
        const h = startHost(pkg());
        const pending = connect();
        h.handOver();
        const am = await pending;
        const onActive = vi.fn();
        const onDormant = vi.fn();
        useVisibility(am, { onActive, onDormant });
        expect(onActive).toHaveBeenCalledTimes(1); // it starts active
        const action = new Promise((r) => am.on("action", r));
        h.notify("visibility", { state: "dormant" });
        h.notify("action", { id: "refresh", source: "header" });
        expect(await action).toEqual({ id: "refresh", source: "header" });
        expect(onDormant).toHaveBeenCalledTimes(1);
    });

    it("keeps a command sent before anyone listens for the first listener", async () => {
        const h = startHost(pkg());
        const pending = connect();
        h.handOver();
        const am = await pending;
        // The pane was opened for the command: it arrives right after hello.
        h.notify("command", { id: "refresh", source: "palette" });
        await new Promise((r) => setTimeout(r, 10));
        const seen: unknown[] = [];
        am.on("command", (c) => seen.push(c));
        await Promise.resolve();
        expect(seen).toEqual([{ id: "refresh", source: "palette" }]);
        h.notify("command", { id: "refresh", source: "status" });
        await new Promise((r) => setTimeout(r, 10));
        expect(seen).toHaveLength(2);
    });

    it("updates a declared status item, and refuses one it didn't declare", async () => {
        const h = startHost(pkg());
        const pending = connect();
        h.handOver();
        const am = await pending;
        await am.ui.setStatusItem("count", { text: "3 notes", tone: "warning" });
        expect(h.setStatusItem).toHaveBeenLastCalledWith("count", { text: "3 notes", tone: "warning" });
        await am.ui.setStatusItem("count");
        expect(h.setStatusItem).toHaveBeenLastCalledWith("count", null);
        const err = (await am.ui.setStatusItem("other", { text: "x" }).catch((e: unknown) => e)) as AgentMuxError;
        expect(err.name).toBe("not_found");
    });

    it("says plainly when the page isn't inside AgentMux, or nobody connects", async () => {
        await expect(connect({ timeoutMs: 50 })).rejects.toThrow(/isn't running inside AgentMux/);
        Object.defineProperty(window, "parent", { value: fakeParent, configurable: true });
        await expect(connect({ timeoutMs: 50 })).rejects.toThrow(/didn't connect within 50 ms/);
    });

    it("encodes and decodes base64 for binary data", () => {
        const bytes = new Uint8Array([0, 1, 254, 255, 72, 105]);
        expect(base64.decode(base64.encode(bytes))).toEqual(bytes);
    });
});
