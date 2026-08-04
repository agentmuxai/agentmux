// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Tests for InAppLoginPanel — specifically the phase-gating fix (reagent
 * P1 on PR #2410): the URL/paste box must disappear once "Use terminal
 * instead" moves the session past waiting-authorize, not just when
 * authUrl itself is cleared. Before this fix, the box stayed mounted
 * showing a dead authorize link — a paste at that point silently wrote
 * the code as a plain auth_token via the host's non-CLI fallback path
 * (cli_login_stdin already cleared), misleading the user while the real
 * login ran in the separately-opened terminal.
 */

import { cleanup, render, screen } from "@solidjs/testing-library";
import { afterEach, describe, expect, it, vi } from "vitest";

vi.mock("@/app/store/global", () => ({
    getApi: () => ({ openExternal: vi.fn(), setProviderAuth: vi.fn() }),
}));
vi.mock("@/util/clipboard", () => ({
    readText: vi.fn(),
    writeText: vi.fn(),
}));

import { InAppLoginPanel, type InAppLoginPhase } from "./InAppLoginPanel";

afterEach(() => {
    cleanup();
});

const AUTH_URL = "https://claude.com/cai/oauth/authorize?code=true";

function renderPanel(phase: InAppLoginPhase) {
    render(() => (
        <InAppLoginPanel
            providerId="claude"
            providerLabel="Claude"
            authUrl={AUTH_URL}
            phase={phase}
            onCancel={() => {}}
            onUseTerminal={() => {}}
        />
    ));
}

describe("InAppLoginPanel — URL/paste box gated on phase, not just authUrl (reagent P1 on PR #2410)", () => {
    it("shows the URL/paste box while phase is 'starting'", () => {
        renderPanel("starting");
        expect(screen.getByText(/authorize in your browser/i)).toBeInTheDocument();
    });

    it("shows the URL/paste box while phase is 'waiting-authorize'", () => {
        renderPanel("waiting-authorize");
        expect(screen.getByText(/authorize in your browser/i)).toBeInTheDocument();
    });

    it("hides the URL/paste box once phase moves to 'fallback' (Use terminal instead was clicked), even though authUrl is still set", () => {
        renderPanel("fallback");
        expect(screen.queryByText(/authorize in your browser/i)).not.toBeInTheDocument();
        expect(screen.queryByPlaceholderText(/paste the authorization code/i)).not.toBeInTheDocument();
    });

    it("hides the URL/paste box once phase moves to 'terminal-polling'", () => {
        renderPanel("terminal-polling");
        expect(screen.queryByText(/authorize in your browser/i)).not.toBeInTheDocument();
    });

    it("hides the \"Use terminal instead\" button once already in fallback/terminal-polling (nothing left to fall back from)", () => {
        renderPanel("terminal-polling");
        expect(screen.queryByRole("button", { name: /use terminal instead/i })).not.toBeInTheDocument();
    });
});
