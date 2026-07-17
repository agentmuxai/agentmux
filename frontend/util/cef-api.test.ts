// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, test, expect, beforeEach, afterEach } from "vitest";
import { isCef, showJsContextMenu } from "./cef-api";

// ── isCef — #52 reload lock-out fix ──────────────────────────────────────────
//
// setupCefApi strips ipc_port/ipc_token from the URL after first read
// (token-leak fix). On a reload the URL has no ipc_port and the host global
// hasn't been re-injected yet, so plain isCef() would return false and
// setupCefApi would bail before awaiting the host's on_load_end re-injection —
// the floating-pool "window.api still undefined after 5s" lock-out. The sticky
// sessionStorage flag (set the moment we first see ipc_port) survives the strip
// and any reload so isCef() stays true and setupCefApi reaches waitForIpcCreds.

describe("isCef", () => {
    const clear = () => {
        try { sessionStorage.clear(); } catch { /* ignore */ }
        delete (window as unknown as { __AGENTMUX_IPC_PORT__?: number }).__AGENTMUX_IPC_PORT__;
        window.history.replaceState({}, "", "/");
    };
    beforeEach(clear);
    afterEach(clear);

    test("plain browser (no creds anywhere) → false", () => {
        expect(isCef()).toBe(false);
    });

    test("first load with ipc_port in URL → true, and remembers it in sessionStorage", () => {
        window.history.replaceState({}, "", "/?ipc_port=51234&windowLabel=main");
        expect(isCef()).toBe(true);
        expect(sessionStorage.getItem("amux:isCef")).toBe("1");
    });

    test("reload after cred-strip stays true via the sticky flag (the #52 fix)", () => {
        // First load sees ipc_port → sets the flag.
        window.history.replaceState({}, "", "/?ipc_port=51234");
        expect(isCef()).toBe(true);
        // Strip + reload: ipc_port gone from URL, host global not yet re-injected.
        window.history.replaceState({}, "", "/?windowLabel=floating-pool-abc&pane-pool=1");
        delete (window as unknown as { __AGENTMUX_IPC_PORT__?: number }).__AGENTMUX_IPC_PORT__;
        // Without the sessionStorage flag this would be false → bail → lock-out.
        expect(isCef()).toBe(true);
    });

    test("host-injected global present → true", () => {
        (window as unknown as { __AGENTMUX_IPC_PORT__?: number }).__AGENTMUX_IPC_PORT__ = 51234;
        expect(isCef()).toBe(true);
    });
});

// ── showJsContextMenu — submenu placement (PR #2198) ─────────────────────────
//
// The second level historically used static `left:100%;top:0` CSS only and was
// cut off at the window edge. It now routes through computeMenuPosition on
// hover (fixed viewport coords, flip/shift/size), held visibility:hidden until
// placed. The static CSS remains as the degraded fallback if the async
// placement rejects, so a submenu is never revealed unpositioned.


describe("showJsContextMenu — submenu placement", () => {
    const removeOverlay = () => {
        document.getElementById("cef-context-menu-overlay")?.remove();
    };
    beforeEach(removeOverlay);
    afterEach(removeOverlay);

    const menuWithSub: NativeContextMenuItem[] = [
        {
            id: "replace-with",
            label: "Replace With...",
            type: "submenu",
            submenu: [
                { label: "Terminal", id: "term" },
                { label: "Browser", id: "web" },
            ],
        },
    ];

    const open = () => {
        showJsContextMenu(menuWithSub, { x: 40, y: 40 }, null);
        const overlay = document.getElementById("cef-context-menu-overlay")!;
        const row = overlay.querySelector<HTMLElement>(".menu-item")!;
        const sub = overlay.querySelector<HTMLElement>(".sub-menu")!;
        return { overlay, row, sub };
    };

    const waitFor = async (cond: () => boolean, ms = 500) => {
        const start = Date.now();
        while (!cond()) {
            if (Date.now() - start > ms) throw new Error("waitFor timed out");
            await new Promise((r) => setTimeout(r, 5));
        }
    };

    test("submenu starts hidden with the static edge-anchored fallback", () => {
        const { row, sub } = open();
        expect(sub.style.display).toBe("none");
        // Degraded fallback: if framework placement ever rejects, these CSS
        // values still anchor the submenu at the row's right edge.
        expect(sub.style.left).toBe("100%");
        expect(sub.style.top).toBe("0px");
        expect(row.style.position).toBe("relative");
    });

    test("hover reveals it only after framework placement (fixed px coords + size cap)", async () => {
        const { row, sub } = open();
        row.dispatchEvent(new MouseEvent("mouseenter"));
        // Shown immediately for measurement, but not yet visible — the
        // pane-overlay clip must register the final rect only.
        expect(sub.style.display).toBe("");
        expect(sub.style.visibility).toBe("hidden");

        await waitFor(() => sub.style.visibility === "");
        expect(sub.style.position).toBe("fixed");
        expect(sub.style.left).toMatch(/^-?\d+px$/);
        expect(sub.style.top).toMatch(/^-?\d+px$/);
        // size() cap applied: taller-than-free-space menus scroll internally.
        expect(sub.style.maxHeight).toMatch(/^\d+px$/);
        expect(sub.style.overflowY).toBe("auto");
    });

    test("mouseleave before placement resolves keeps it hidden (stale-promise guard)", async () => {
        const { row, sub } = open();
        row.dispatchEvent(new MouseEvent("mouseenter"));
        row.dispatchEvent(new MouseEvent("mouseleave"));
        expect(sub.style.display).toBe("none");
        // Give the in-flight computeMenuPosition promise time to settle; the
        // guard must not reveal a submenu whose hover already ended.
        await new Promise((r) => setTimeout(r, 50));
        expect(sub.style.display).toBe("none");
        expect(sub.style.visibility).toBe("hidden");
    });

    test("top-level menu is framework-placed too (fixed coords, then revealed)", async () => {
        const { overlay } = open();
        const menuEl = overlay.querySelector<HTMLElement>(".menu")!;
        expect(menuEl.style.visibility).toBe("hidden");
        await waitFor(() => menuEl.style.visibility === "");
        expect(menuEl.style.position).toBe("fixed");
        expect(menuEl.style.left).toMatch(/^-?\d+px$/);
        expect(menuEl.style.maxHeight).toMatch(/^\d+px$/);
    });
});
