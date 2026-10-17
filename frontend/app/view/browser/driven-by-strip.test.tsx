// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The header strip of an agent-driven browser pane: the red Bypass button
 * next to Take over, its confirmation, and the red strip while it lasts.
 * docs/specs/SPEC_BROWSER_PANE_BYPASS_2026_10_16.md §3.
 */

import { cleanup, fireEvent, render, screen } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, describe, expect, it, vi } from "vitest";

import type { BrowserViewModel } from "./browser-model";
import { DrivenByStrip } from "./driven-by-strip";

afterEach(() => cleanup());

function fakeModel(over: Partial<Record<string, unknown>> = {}) {
    const [bypass, setBypassMeta] = createSignal<BrowserBypass | undefined>(undefined);
    const setBypass = vi.fn(async () => {});
    const model = {
        allowedOriginsAtom: () => [] as string[],
        bypassAtom: bypass,
        bypassPaused: () => false,
        bypassMinutesLeft: () => 42,
        bypassOriginsPreview: () => ["https://github.com"],
        setBypass,
        takeOver: vi.fn(async () => {}),
        ...over,
    } as unknown as BrowserViewModel;
    return { model, setBypass, setBypassMeta };
}

describe("DrivenByStrip", () => {
    it("puts a red Bypass button next to Take over", () => {
        const { model } = fakeModel();
        render(() => <DrivenByStrip model={model} agent="Naki" />);
        const bypass = screen.getByText("Bypass").closest("button")!;
        const takeOver = screen.getByText("Take over").closest("button")!;
        expect(bypass.className).toContain("ui-tone-danger");
        expect(bypass.nextElementSibling).toBe(takeOver);
    });

    it("confirms with the sites and a duration, and grants only on a pointer click", () => {
        const { model, setBypass } = fakeModel();
        render(() => <DrivenByStrip model={model} agent="Naki" />);
        fireEvent.click(screen.getByText("Bypass"));
        expect(screen.getByRole("dialog", { name: "Turn on bypass" }).textContent).toContain("github.com");
        fireEvent.click(screen.getByText("4 hours"));
        const turnOn = screen.getByText("Turn on bypass").closest("button")!;
        // A key press on the focused button (detail 0) doesn't grant it.
        fireEvent.click(turnOn, { detail: 0 });
        expect(setBypass).not.toHaveBeenCalled();
        fireEvent.click(turnOn, { detail: 1 });
        expect(setBypass).toHaveBeenCalledWith(true, 240);
    });

    it("shows a red strip with End bypass while it lasts", () => {
        const { model, setBypass, setBypassMeta } = fakeModel();
        render(() => <DrivenByStrip model={model} agent="Naki" />);
        setBypassMeta({ agent: "Naki", origins: ["https://github.com"], minutes: 60, expires_at: Date.now() + 3_600_000 });
        const strip = screen.getByRole("status");
        expect(strip.className).toContain("browser-driven-by--bypass");
        expect(strip.textContent).toContain("acts without asking on github.com");
        expect(strip.textContent).toContain("42 min left");
        expect(screen.queryByText("Bypass")).toBeNull();
        fireEvent.click(screen.getByText("End bypass"));
        expect(setBypass).toHaveBeenCalledWith(false);
    });

    it("shows a failed End bypass in the red strip", async () => {
        const { model, setBypass, setBypassMeta } = fakeModel();
        setBypass.mockRejectedValueOnce(new Error("srv is down"));
        render(() => <DrivenByStrip model={model} agent="Naki" />);
        setBypassMeta({ agent: "Naki", origins: ["https://github.com"], minutes: 60, expires_at: Date.now() + 3_600_000 });
        fireEvent.click(screen.getByText("End bypass"));
        expect((await screen.findByRole("alert")).textContent).toBe("Couldn't end bypass: srv is down");
    });
});
