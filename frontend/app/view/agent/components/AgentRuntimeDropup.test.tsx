// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * No prior coverage existed for this component (see
 * docs/specs/SPEC_AGENT_RUNTIME_DROPUP_CLOSE_BUTTON_2026_08_07.md §5).
 * Covers both fixes from that spec plus the pre-existing §9.2 "stays open on
 * select" contract, which had never been pinned down by a test either:
 *
 *   1. Selecting a value does NOT close the panel (regression guard for the
 *      already-shipped SPEC_AGENT_RUNTIME_DROPUP_2026_07_09.md §9.2 decision,
 *      and for the focus-blur bug that was silently defeating it in
 *      practice — see AgentRuntimeDropup.tsx's onMouseDown comment on the
 *      option row).
 *   2. The new close button closes the panel.
 *   3. Click-outside still closes the panel.
 *   4. The close button sits outside the role="listbox" subtree.
 */

import { cleanup, fireEvent, render, screen } from "@solidjs/testing-library";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { AgentRuntimeDropup } from "./AgentRuntimeDropup";

// The process-runtime report the menu listens to; each test sets it.
import { createSignal } from "solid-js";
import type { ProcessRuntime } from "../process-runtime";
const [report, setReport] = createSignal<ProcessRuntime | undefined>(undefined);
vi.mock("../process-runtime", async (importOriginal) => ({
    ...(await importOriginal<typeof import("../process-runtime")>()),
    useProcessRuntime: () => report,
}));

const patchRuntime = vi.fn();
vi.mock("../runtime-apply", () => ({
    applyRuntimeChange: vi.fn().mockResolvedValue(undefined),
    patchRuntime: (...args: unknown[]) => patchRuntime(...args),
}));

beforeEach(() => {
    patchRuntime.mockReset().mockResolvedValue({});
    setReport(undefined);
});
afterEach(() => cleanup());

function renderDropup() {
    return render(() => (
        <AgentRuntimeDropup blockId="block-1" blockAtom={() => undefined} providerId="claude" />
    ));
}

async function openPanel(): Promise<void> {
    const trigger = screen.getByRole("button", { name: /Runtime settings/i });
    await userEvent.click(trigger);
}

describe("AgentRuntimeDropup — stays open across selections", () => {
    // jsdom doesn't reproduce the real-browser quirk this guards against
    // (clicking a non-focusable element blurs the active element to
    // <body>), so the higher-level "stays open" tests below can't actually
    // detect a regression of the onMouseDown fix by themselves — they'd
    // keep passing even with it removed. This test instead asserts the
    // mechanism directly: the row's mousedown handler must call
    // preventDefault(), which is what stops a real browser from blurring
    // focus out of the panel in the first place. dispatchEvent (which
    // fireEvent wraps) returns false when preventDefault() was called on a
    // cancelable event.
    it("calls preventDefault on an option row's mousedown, to stop the browser from blurring focus out of the panel", async () => {
        renderDropup();
        await openPanel();

        const row = screen.getAllByRole("option")[0];
        const notPrevented = fireEvent.mouseDown(row);
        expect(notPrevented).toBe(false);
    });

    it("does not close the panel when an option row is clicked", async () => {
        renderDropup();
        await openPanel();
        expect(screen.getByRole("listbox")).toBeInTheDocument();

        const row = screen.getAllByRole("option")[0];
        await userEvent.click(row);

        // Still present — the real bug this test guards against closed the
        // panel here despite applySelection() never calling setOpen(false).
        expect(screen.getByRole("listbox")).toBeInTheDocument();
    });

    it("does not close when several options are clicked in sequence", async () => {
        renderDropup();
        await openPanel();

        const rows = screen.getAllByRole("option");
        await userEvent.click(rows[0]);
        await userEvent.click(rows[1]);
        await userEvent.click(rows[2]);

        expect(screen.getByRole("listbox")).toBeInTheDocument();
    });
});

describe("AgentRuntimeDropup — close button", () => {
    it("closes the panel when clicked", async () => {
        renderDropup();
        await openPanel();
        expect(screen.getByRole("listbox")).toBeInTheDocument();

        const closeBtn = screen.getByRole("button", { name: "Close" });
        await userEvent.click(closeBtn);

        expect(screen.queryByRole("listbox")).not.toBeInTheDocument();
    });

    it("is not inside the role=listbox subtree", async () => {
        renderDropup();
        await openPanel();

        const listbox = screen.getByRole("listbox");
        const closeBtn = screen.getByRole("button", { name: "Close" });
        expect(listbox.contains(closeBtn)).toBe(false);
    });

    it("does not appear as a role=option row", async () => {
        renderDropup();
        await openPanel();

        for (const option of screen.getAllByRole("option")) {
            expect(option).not.toHaveAttribute("aria-label", "Close");
        }
    });
});

describe("AgentRuntimeDropup — click outside", () => {
    it("closes the panel on an outside click", async () => {
        renderDropup();
        await openPanel();
        expect(screen.getByRole("listbox")).toBeInTheDocument();

        fireEvent.mouseDown(document.body);

        expect(screen.queryByRole("listbox")).not.toBeInTheDocument();
    });

    it("does not close on a click inside the panel (option row)", async () => {
        renderDropup();
        await openPanel();

        const row = screen.getAllByRole("option")[0];
        fireEvent.mouseDown(row);

        expect(screen.getByRole("listbox")).toBeInTheDocument();
    });
});

describe("AgentRuntimeDropup — superseded persisted model migration", () => {
    // The live-catalog overlay refreshes CONCRETE option values, so an agent
    // configured before such a bump holds an id no longer in modelOptions().
    // Without migration `modelLabel()` renders the raw id and no row is marked
    // current — the dropdown silently loses its selection display.
    // reagent P1, PR #2990.
    // getRuntimeConfig reads a nested object under "agent:runtime", not
    // dotted keys — see buildRuntimeArgs.ts:153.
    const metaWith = (model: string) => ({
        "agent:runtime": { model, permissionMode: "default", effort: "high" },
    });

    it("migrates a superseded concrete id to its current family member", async () => {
        render(() => (
            <AgentRuntimeDropup
                blockId="block-1"
                blockAtom={() => ({ meta: metaWith("claude-fable-5") }) as any}
                providerId="claude"
            />
        ));

        await vi.waitFor(() => expect(patchRuntime).toHaveBeenCalled());
        // A patch, not a whole config: only the model is being migrated.
        const patch = patchRuntime.mock.calls[0][2] as { model: string };
        expect(patch).toEqual({ model: expect.stringMatching(/^claude-fable-5-1$/) });
    });

    it("leaves an alias selection untouched", async () => {
        render(() => (
            <AgentRuntimeDropup
                blockId="block-1"
                blockAtom={() => ({ meta: metaWith("sonnet") }) as any}
                providerId="claude"
            />
        ));

        // Aliases are never superseded, so nothing should be rewritten.
        await new Promise((r) => setTimeout(r, 20));
        expect(patchRuntime).not.toHaveBeenCalled();
    });

    it("leaves an unrecognised id alone rather than guessing a family", async () => {
        render(() => (
            <AgentRuntimeDropup
                blockId="block-1"
                blockAtom={() => ({ meta: metaWith("some-other-vendor-model-9") }) as any}
                providerId="claude"
            />
        ));

        await new Promise((r) => setTimeout(r, 20));
        expect(patchRuntime).not.toHaveBeenCalled();
    });
});

describe("AgentRuntimeDropup — a change is a patch, and a failure is visible", () => {
    const rowByName = (name: RegExp) => screen.getAllByRole("option").find((o) => name.test(o.textContent ?? ""))!;

    it("sends only what was changed, so two quick selections cannot undo each other", async () => {
        renderDropup();
        await openPanel();
        await userEvent.click(rowByName(/Opus/i));
        await userEvent.click(rowByName(/^max/i));
        expect(patchRuntime).toHaveBeenCalledTimes(2);
        expect(patchRuntime.mock.calls[0][2]).toEqual({ model: "opus" });
        expect(patchRuntime.mock.calls[1][2]).toEqual({ effort: "max" });
    });

    it("says so on the trigger when a change could not be applied, and clears on the next one that can", async () => {
        patchRuntime.mockRejectedValueOnce(new Error("controller resync refused"));
        renderDropup();
        await openPanel();
        const trigger = screen.getByRole("button", { name: /Runtime settings/i });
        expect(trigger.className).not.toContain("agent-runtime-dropup-trigger--error");

        await userEvent.click(rowByName(/Opus/i));
        await vi.waitFor(() => expect(trigger.className).toContain("agent-runtime-dropup-trigger--error"));
        expect(trigger.title).toContain("controller resync refused");
        expect(trigger.getAttribute("aria-label")).toContain("failed to apply");

        await userEvent.click(rowByName(/Sonnet/i));
        await vi.waitFor(() => expect(trigger.className).not.toContain("agent-runtime-dropup-trigger--error"));
    });
});

describe("AgentRuntimeDropup — says when the agent is not running what is selected", () => {
    // Defaults with no block meta: bypass / sonnet / high, which a persistent
    // agent is spawned with as `--permission-mode default --model sonnet --effort high`.
    const AGREES: ProcessRuntime = {
        running: true,
        restartPending: false,
        model: "sonnet",
        effort: "high",
        permissionMode: "default",
    };
    const INCIDENT: ProcessRuntime = { ...AGREES, model: undefined, effort: undefined };
    const trigger = () => screen.getByRole("button", { name: /Runtime settings/i });
    const state = () => ({
        differs: trigger().className.includes("agent-runtime-dropup-trigger--differs"),
        pending: trigger().className.includes("agent-runtime-dropup-trigger--pending"),
    });

    afterEach(() => vi.useRealTimers());

    it("shows nothing before the server has reported, or when the process agrees", async () => {
        vi.useFakeTimers();
        renderDropup();
        expect(state()).toEqual({ differs: false, pending: false });
        setReport(AGREES);
        await vi.advanceTimersByTimeAsync(5000);
        expect(state()).toEqual({ differs: false, pending: false });
    });

    it("the incident: a process started on the CLI default under a Sonnet/high menu is flagged", async () => {
        vi.useFakeTimers();
        renderDropup();
        setReport(INCIDENT);
        await vi.advanceTimersByTimeAsync(2000);
        expect(state().differs).toBe(true);
        expect(trigger().title).toContain("not running what is selected");
        expect(trigger().title).toContain("the CLI default");
        expect(trigger().getAttribute("aria-label")).toContain("not running this selection");

        fireEvent.click(trigger());
        expect(screen.getByRole("status").textContent).toContain("Model: selected");
        expect(screen.getByRole("status").textContent).toContain("running the CLI default");
    });

    it("waits a moment before saying so: the old process outlives a change by a beat", async () => {
        vi.useFakeTimers();
        renderDropup();
        setReport(INCIDENT);
        await vi.advanceTimersByTimeAsync(400);
        expect(state().differs).toBe(false);
        setReport(AGREES); // replaced before the delay elapsed
        await vi.advanceTimersByTimeAsync(5000);
        expect(state().differs).toBe(false);
    });

    it("a selection that is on its way in is pending, with no restart button", async () => {
        vi.useFakeTimers();
        renderDropup();
        setReport({ ...INCIDENT, restartPending: true });
        await vi.advanceTimersByTimeAsync(10);
        expect(state()).toEqual({ differs: false, pending: true });
        expect(trigger().title).toContain("Applies after the current turn");
        fireEvent.click(trigger());
        expect(screen.getByRole("status").textContent).toContain("Applies after the current turn");
        expect(screen.queryByRole("button", { name: /Restart to apply/i })).toBeNull();
    });

    it("\"Restart to apply\" re-applies the current selection", async () => {
        vi.useFakeTimers();
        renderDropup();
        setReport(INCIDENT);
        await vi.advanceTimersByTimeAsync(2000);
        fireEvent.click(trigger());
        fireEvent.click(screen.getByRole("button", { name: /Restart to apply/i }));
        expect(patchRuntime).toHaveBeenCalledTimes(1);
        expect(patchRuntime.mock.calls[0][2]).toEqual({});
    });

    it("clears once the restarted process reports the right flags", async () => {
        vi.useFakeTimers();
        renderDropup();
        setReport(INCIDENT);
        await vi.advanceTimersByTimeAsync(2000);
        expect(state().differs).toBe(true);
        setReport(AGREES);
        await vi.advanceTimersByTimeAsync(10);
        expect(state().differs).toBe(false);
    });
});

describe("AgentRuntimeDropup — effort only where it applies", () => {
    const withModel = (model: string) => ({
        meta: { "agent:runtime": { model, permissionMode: "default", effort: "high" } },
    });
    const renderWith = (model: string) =>
        render(() => <AgentRuntimeDropup blockId="block-1" blockAtom={() => withModel(model) as any} providerId="claude" />);

    it("a model that takes effort shows the effort rows and the effort in the label", async () => {
        renderWith("sonnet");
        const trigger = screen.getByRole("button", { name: /Runtime settings/i });
        expect(trigger.textContent).toMatch(/high/);
        await userEvent.click(trigger);
        expect(screen.getAllByRole("option").some((o) => /^max/i.test(o.textContent ?? ""))).toBe(true);
    });

    it("Haiku does not: no effort rows to pick, no effort in the label, and it says why", async () => {
        renderWith("haiku");
        const trigger = screen.getByRole("button", { name: /Runtime settings/i });
        expect(trigger.textContent).not.toMatch(/high/);
        await userEvent.click(trigger);
        expect(screen.getAllByRole("option").some((o) => /^max/i.test(o.textContent ?? ""))).toBe(false);
        expect(screen.getByText(/Not applied — Haiku does not use it/)).toBeTruthy();
    });

    it("decides on the model the process RUNS: a definition's own Haiku under a Sonnet selection (Codex P1 on #4152)", async () => {
        const meta = {
            "agent:runtime": { model: "sonnet", permissionMode: "default", effort: "high" },
            "agent:provider_flags": "--model claude-haiku-4-5-20251001",
        };
        render(() => <AgentRuntimeDropup blockId="block-1" blockAtom={() => ({ meta }) as any} providerId="claude" />);
        const trigger = screen.getByRole("button", { name: /Runtime settings/i });
        expect(trigger.textContent).not.toMatch(/high/);
        await userEvent.click(trigger);
        expect(screen.getByText(/Not applied — Haiku does not use it/)).toBeTruthy();
    });

    it("a concrete Haiku id is treated the same", async () => {
        renderWith("claude-haiku-4-5-20251001");
        expect(screen.getByRole("button", { name: /Runtime settings/i }).textContent).not.toMatch(/high/);
    });
});
