// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Tests for forceProviderLogin (SPEC_REAUTH_FROM_AUTH_ERROR §11).
 *
 * The whole point of this helper is that it NEVER consults CheckCliAuth — it
 * unconditionally runs the provider login. These tests pin:
 *   - runCliLogin is called with the provider's login command + auth env;
 *   - a captured URL is pushed to setAuthUrl AND opened via openOAuthBrowserPane,
 *     and the call resolves "opened";
 *   - a null URL resolves "no-url" (the CALLER surfaces the visible error —
 *     retro-agent-auth-relogin-noop-2026-07-01 §5.1) and does not call setAuthUrl;
 *   - CheckCliAuth is never invoked (no auth-status gate).
 */

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const hub = vi.hoisted(() => ({
    runCliLogin: vi.fn(),
    checkCliAuth: vi.fn(),
    openPane: vi.fn(),
}));

vi.mock("@/app/store/global", () => ({
    getApi: () => ({ runCliLogin: hub.runCliLogin, checkCliAuth: hub.checkCliAuth }),
}));
vi.mock("./open-oauth-pane", () => ({ openOAuthBrowserPane: hub.openPane }));

import { forceProviderLogin } from "./force-login";

const URL = "https://claude.ai/oauth/authorize?client_id=abc";
const provider = {
    authLoginCommand: ["auth", "login"],
    requiresLoginTty: true,
    authConfigDirEnvVar: "CLAUDE_CONFIG_DIR",
} as any;

beforeEach(() => {
    hub.runCliLogin.mockReset();
    hub.checkCliAuth.mockReset();
    hub.openPane.mockReset().mockResolvedValue("pane");
});
afterEach(() => vi.clearAllMocks());

describe("forceProviderLogin", () => {
    it("runs runCliLogin with the login command + auth env, then opens the captured URL", async () => {
        hub.runCliLogin.mockResolvedValue(URL);
        const setAuthUrl = vi.fn();
        const log = vi.fn();

        const outcome = await forceProviderLogin({
            provider,
            cliPath: "C:/cli/claude.cmd",
            authEnv: { CLAUDE_CONFIG_DIR: "C:/auth" },
            setAuthUrl,
            log,
        });

        expect(hub.runCliLogin).toHaveBeenCalledWith(
            "C:/cli/claude.cmd",
            ["auth", "login"],
            { CLAUDE_CONFIG_DIR: "C:/auth" },
            true,
            // reagent P1 on PR #2410: threaded through so the host can
            // baseline-check the RIGHT credential file for THIS provider
            // (capture_cred_baseline no longer hardcodes CLAUDE_CONFIG_DIR).
            "CLAUDE_CONFIG_DIR",
        );
        expect(setAuthUrl).toHaveBeenCalledWith(URL);
        expect(hub.openPane).toHaveBeenCalledWith(URL);
        expect(outcome).toBe("opened");
    });

    it("NEVER consults the auth-status check (the whole point of forcing)", async () => {
        hub.runCliLogin.mockResolvedValue(URL);
        await forceProviderLogin({ provider, cliPath: "x", authEnv: {}, setAuthUrl: vi.fn(), log: vi.fn() });
        expect(hub.checkCliAuth).not.toHaveBeenCalled();
    });

    it("resolves 'no-url' and does not set an auth URL when no URL is captured", async () => {
        hub.runCliLogin.mockResolvedValue(null);
        const setAuthUrl = vi.fn();
        const log = vi.fn();

        const outcome = await forceProviderLogin({ provider, cliPath: "x", authEnv: {}, setAuthUrl, log });

        expect(setAuthUrl).not.toHaveBeenCalled();
        expect(hub.openPane).not.toHaveBeenCalled();
        expect(outcome).toBe("no-url");
        // The helper still logs, but must NOT pretend a browser opened — the
        // caller owns the user-visible error (never fail silently, retro §5.1).
        expect(log).toHaveBeenCalledWith("auth", expect.stringMatching(/no login URL captured/i), "warn");
    });

    it("still resolves (URL set) even when the pane falls back to the system browser", async () => {
        hub.runCliLogin.mockResolvedValue(URL);
        hub.openPane.mockResolvedValue("external");
        const setAuthUrl = vi.fn();
        const log = vi.fn();

        await forceProviderLogin({ provider, cliPath: "x", authEnv: {}, setAuthUrl, log });

        expect(setAuthUrl).toHaveBeenCalledWith(URL);
        expect(log).toHaveBeenCalledWith("auth", expect.stringMatching(/system browser/i));
    });

    it("reagent P2 on PR #2410: does not open a browser/pane for an attempt already cancelled by the time the URL is captured, but still resolves 'opened' (not 'no-url') so an awaited-session caller's own isCancelled check — not a tier-2/3 fallthrough — is what ends the attempt", async () => {
        hub.runCliLogin.mockResolvedValue(URL);
        const setAuthUrl = vi.fn();
        const log = vi.fn();

        const outcome = await forceProviderLogin({
            provider,
            cliPath: "x",
            authEnv: {},
            setAuthUrl,
            log,
            isCancelled: () => true,
        });

        expect(hub.openPane).not.toHaveBeenCalled();
        expect(setAuthUrl).not.toHaveBeenCalled();
        expect(outcome).toBe("opened");
        expect(log).toHaveBeenCalledWith("auth", expect.stringMatching(/cancelled before a browser/i), "warn");
    });
});
