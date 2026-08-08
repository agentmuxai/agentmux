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
import { afterEach, describe, expect, it, vi } from "vitest";
import { AgentRuntimeDropup } from "./AgentRuntimeDropup";

vi.mock("../runtime-apply", () => ({
    applyRuntimeChange: vi.fn().mockResolvedValue(undefined),
}));

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
