// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The header strip of an agent-driven browser pane: the red Auto-approve button
 * next to Take over, its confirmation, and the red strip while it lasts
 * (docs/specs/SPEC_BROWSER_PANE_BYPASS_2026_10_16.md §3); the Log and Pause
 * (PLAN_BROWSER_DRIVER_FIXES_2026_10_17.md §5).
 */

import { cleanup, fireEvent, render, screen } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, describe, expect, it, vi } from "vitest";

import type { BrowserViewModel } from "./browser-model";
import { ActionLogStrip, DrivenByStrip } from "./driven-by-strip";

afterEach(() => cleanup());

function fakeModel(over: Partial<Record<string, unknown>> = {}) {
    const [bypass, setBypassMeta] = createSignal<BrowserBypass | undefined>(undefined);
    const setBypass = vi.fn(async () => {});
    const [actions, setActions] = createSignal<BrowserActions>({ count: 0, entries: [] });
    const [paused, setPausedMeta] = createSignal(false);
    const setPaused = vi.fn(async () => {});
    const model = {
        actionsAtom: actions,
        pausedAtom: paused,
        setPaused,
        allowedOriginsAtom: () => [] as string[],
        bypassAtom: bypass,
        bypassPaused: () => false,
        bypassMinutesLeft: () => 42,
        bypassOriginsPreview: () => ["https://github.com"],
        setBypass,
        takeOver: vi.fn(async () => {}),
        ...over,
    } as unknown as BrowserViewModel;
    return { model, setBypass, setBypassMeta, setActions, setPausedMeta, setPaused };
}

describe("DrivenByStrip", () => {
    it("puts a red Auto-approve button next to Take over", () => {
        const { model } = fakeModel();
        render(() => <DrivenByStrip model={model} agent="Naki" />);
        const bypass = screen.getByText("Auto-approve").closest("button")!;
        const takeOver = screen.getByText("Take over").closest("button")!;
        expect(bypass.className).toContain("ui-tone-danger");
        expect(bypass.nextElementSibling).toBe(takeOver);
    });

    it("confirms with the sites and a duration, and grants only on a pointer click", () => {
        const { model, setBypass } = fakeModel();
        render(() => <DrivenByStrip model={model} agent="Naki" />);
        fireEvent.click(screen.getByText("Auto-approve"));
        expect(screen.getByRole("dialog", { name: "Turn on auto-approve" }).textContent).toContain("github.com");
        fireEvent.click(screen.getByText("4 hours"));
        const turnOn = screen.getByText("Turn on auto-approve").closest("button")!;
        // A key press on the focused button (detail 0) doesn't grant it.
        fireEvent.click(turnOn, { detail: 0 });
        expect(setBypass).not.toHaveBeenCalled();
        fireEvent.click(turnOn, { detail: 1 });
        expect(setBypass).toHaveBeenCalledWith(true, 240);
    });

    it("shows a red strip with Stop auto-approve while it lasts", () => {
        const { model, setBypass, setBypassMeta } = fakeModel();
        render(() => <DrivenByStrip model={model} agent="Naki" />);
        setBypassMeta({ agent: "Naki", origins: ["https://github.com"], minutes: 60, expires_at: Date.now() + 3_600_000 });
        const strip = screen.getByRole("status");
        expect(strip.className).toContain("browser-driven-by--bypass");
        expect(strip.textContent).toContain("clicks go through without asking on github.com");
        expect(strip.textContent).toContain("42 min left");
        expect(screen.queryByText("Auto-approve")).toBeNull();
        fireEvent.click(screen.getByText("Stop auto-approve"));
        expect(setBypass).toHaveBeenCalledWith(false);
    });

    it("shows a failed Stop auto-approve in the red strip", async () => {
        const { model, setBypass, setBypassMeta } = fakeModel();
        setBypass.mockRejectedValueOnce(new Error("srv is down"));
        render(() => <DrivenByStrip model={model} agent="Naki" />);
        setBypassMeta({ agent: "Naki", origins: ["https://github.com"], minutes: 60, expires_at: Date.now() + 3_600_000 });
        fireEvent.click(screen.getByText("Stop auto-approve"));
        expect((await screen.findByRole("alert")).textContent).toBe("Couldn't stop auto-approve: srv is down");
    });

    it("counts the log and opens it in the pane's flow, newest first, never as a popover", () => {
        const { model, setActions } = fakeModel();
        render(() => <DrivenByStrip model={model} agent="Naki" />);
        setActions({
            count: 2,
            entries: [
                { ts: Date.UTC(2026, 9, 17, 9, 0, 0), agent: "Naki", kind: "fill", target: "e4", how: "allowed" },
                {
                    ts: Date.UTC(2026, 9, 17, 9, 0, 5),
                    agent: "Naki",
                    kind: "click",
                    target: '"Delete draft"',
                    how: "auto-approved",
                    url: "https://github.com/example/repo/issues/1?tab=x",
                    detail: "ref e9",
                },
            ],
        });
        const log = screen.getByText("Log (2)").closest("button")!;
        expect(screen.queryByRole("log")).toBeNull();
        fireEvent.click(log);
        const panel = screen.getByRole("log", { name: "What Naki did in this pane" });
        // A sibling of the strip in the pane's flow, not inside a popover.
        expect(panel.parentElement).toBe(screen.getByRole("status").parentElement);
        const rows = panel.querySelectorAll("li");
        expect(rows).toHaveLength(2);
        expect(rows[0].textContent).toContain('"Delete draft" · ref e9');
        expect(rows[0].textContent).toContain("auto-approved");
        expect(rows[0].textContent).toContain("github.com/example/repo/issues/1");
        expect(rows[0].textContent).not.toContain("?tab=x");
        expect(rows[1].textContent).toContain("fill");
        fireEvent.click(screen.getByText("Close"));
        expect(screen.queryByRole("log")).toBeNull();
    });

    it("pauses, says so in the strip, and resumes only on a pointer click", () => {
        const { model, setPaused, setPausedMeta } = fakeModel();
        render(() => <DrivenByStrip model={model} agent="Naki" />);
        fireEvent.click(screen.getByText("Pause"), { detail: 0 });
        expect(setPaused).toHaveBeenCalledWith(true);
        setPausedMeta(true);
        const strip = screen.getByRole("status");
        expect(strip.className).toContain("browser-driven-by--held");
        expect(strip.textContent).toContain("Paused: Naki's actions here wait until you resume");
        const resume = screen.getByText("Resume").closest("button")!;
        fireEvent.click(resume, { detail: 0 });
        expect(setPaused).toHaveBeenCalledTimes(1);
        fireEvent.click(resume, { detail: 1 });
        expect(setPaused).toHaveBeenLastCalledWith(false);
    });

    it("keeps the log reachable after Take over, in a slim strip with only Log", () => {
        const { model, setActions } = fakeModel();
        setActions({
            count: 1,
            entries: [{ ts: Date.UTC(2026, 9, 17, 9, 0, 0), agent: "Naki", kind: "click", target: "e2", how: "allowed" }],
        });
        render(() => <ActionLogStrip model={model} />);
        const strip = screen.getByRole("status");
        expect(strip.textContent).toContain("Naki's actions here");
        expect(screen.queryByText("Take over")).toBeNull();
        expect(screen.queryByText("Pause")).toBeNull();
        expect(screen.queryByText("Auto-approve")).toBeNull();
        fireEvent.click(screen.getByText("Log (1)"));
        const panel = screen.getByRole("log", { name: "What Naki did in this pane" });
        expect(panel.parentElement).toBe(strip.parentElement);
        expect(panel.querySelectorAll("li")).toHaveLength(1);
    });
});
