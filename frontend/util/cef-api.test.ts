// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, test, expect, beforeEach, afterEach, vi } from "vitest";
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

// ── showJsContextMenu — submenu placement (PR #2198, PR #2525) ───────────────
//
// The second level historically used static `left:100%;top:0` CSS only and was
// cut off at the window edge. It now routes through computeMenuPosition on
// hover (fixed viewport coords, flip/shift/size), held visibility:hidden until
// placed. The static CSS remains as the degraded fallback if the async
// placement rejects, so a submenu is never revealed unpositioned.
//
// As of PR #2525 (SPEC_SUBMENU_POSITIONING_AND_HOVER_TIMING_2026_08_10), open/
// close additionally routes through createSubmenuHover's open-delay +
// safe-triangle close instead of firing synchronously on mouseenter/
// mouseleave — see submenu-hover.test.ts for that logic's own unit tests.
// These tests cover the DOM-level integration: the open delay before
// computeMenuPosition is even invoked, and the still-present stale-promise
// guard in the `.then()` continuation below.


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

    test("hover reveals it only after the open delay AND framework placement (fixed px coords + size cap)", async () => {
        vi.useFakeTimers();
        try {
            const { row, sub } = open();
            row.dispatchEvent(new MouseEvent("mouseenter"));
            // Not yet shown — createSubmenuHover's open delay hasn't elapsed,
            // so computeMenuPosition hasn't even been invoked yet.
            expect(sub.style.display).toBe("none");

            // Synchronous advance (not the Async variant, which would also
            // flush the computeMenuPosition microtask in the same step) —
            // fires the open-delay timer and stops right there, catching the
            // "shown for measurement, not yet visible" intermediate state.
            vi.advanceTimersByTime(90);
            expect(sub.style.display).toBe("");
            expect(sub.style.visibility).toBe("hidden");

            // Let the in-flight computeMenuPosition promise resolve.
            await vi.advanceTimersByTimeAsync(0);
            expect(sub.style.visibility).toBe("");
            expect(sub.style.position).toBe("fixed");
            expect(sub.style.left).toMatch(/^-?\d+px$/);
            expect(sub.style.top).toMatch(/^-?\d+px$/);
            // size() cap applied: taller-than-free-space menus scroll internally.
            expect(sub.style.maxHeight).toMatch(/^\d+px$/);
            expect(sub.style.overflowY).toBe("auto");
        } finally {
            vi.useRealTimers();
        }
    });

    test("mouseleave during the open delay cancels it — the submenu never opens", async () => {
        const { row, sub } = open();
        row.dispatchEvent(new MouseEvent("mouseenter"));
        row.dispatchEvent(new MouseEvent("mouseleave"));
        // Well past the open delay — left before it elapsed, so it must
        // never have opened (computeMenuPosition never even called).
        await new Promise((r) => setTimeout(r, 150));
        expect(sub.style.display).toBe("none");
        expect(sub.style.visibility).toBe("");
    });

    test("stale-placement guard: a submenu closed mid-flight is never revealed once placement resolves", async () => {
        vi.useFakeTimers();
        try {
            const { row, sub } = open();
            row.dispatchEvent(new MouseEvent("mouseenter"));
            // Synchronous advance — fires the open-delay timer without also
            // flushing the computeMenuPosition microtask in the same step.
            vi.advanceTimersByTime(90);
            expect(sub.style.display).toBe(""); // shown for measurement
            expect(sub.style.visibility).toBe("hidden");

            // Simulate the hover having ended (by whatever path/timing)
            // while computeMenuPosition's promise is still in flight.
            sub.style.display = "none";

            // Let the in-flight computeMenuPosition promise settle.
            await vi.advanceTimersByTimeAsync(0);

            // The guard (`sub.style.display === "none"` check in the `.then()`)
            // must not re-reveal a submenu whose hover already ended.
            expect(sub.style.display).toBe("none");
            expect(sub.style.visibility).toBe("hidden");
        } finally {
            vi.useRealTimers();
        }
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
