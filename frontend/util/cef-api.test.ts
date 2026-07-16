// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, test, expect, beforeEach, afterEach } from "vitest";
import { isCef } from "./cef-api";

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
