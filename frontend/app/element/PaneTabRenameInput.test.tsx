// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Tests for PaneTabRenameInput — the inline rename input shared by the
 * agent-fork and terminal tab strips (double-click-to-rename).
 * Spec: docs/specs/SPEC_PANE_TAB_STRIP_COMPACT_SIZING_AND_RENAME_2026_07_22.md §3.3.
 */

import { cleanup, render, screen } from "@solidjs/testing-library";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";

import { PaneTabRenameInput } from "./PaneTabRenameInput";

afterEach(() => cleanup());

describe("PaneTabRenameInput", () => {
    it("autofocuses and selects all text on mount", () => {
        render(() => (
            <PaneTabRenameInput initialValue="Terminal 1" onConfirm={vi.fn()} onCancel={vi.fn()} />
        ));
        const input = screen.getByDisplayValue("Terminal 1") as HTMLInputElement;
        expect(document.activeElement).toBe(input);
        expect(input.selectionStart).toBe(0);
        expect(input.selectionEnd).toBe("Terminal 1".length);
    });

    it("confirms the trimmed value on Enter", async () => {
        const onConfirm = vi.fn();
        const onCancel = vi.fn();
        render(() => (
            <PaneTabRenameInput initialValue="old" onConfirm={onConfirm} onCancel={onCancel} />
        ));
        const user = userEvent.setup();
        const input = screen.getByDisplayValue("old");
        await user.clear(input);
        await user.type(input, "  new name  {Enter}");
        expect(onConfirm).toHaveBeenCalledWith("new name");
        expect(onCancel).not.toHaveBeenCalled();
    });

    it("confirms on blur (unlike Save-As, which cancels on blur)", async () => {
        const onConfirm = vi.fn();
        render(() => (
            <>
                <PaneTabRenameInput initialValue="old" onConfirm={onConfirm} onCancel={vi.fn()} />
                <button>elsewhere</button>
            </>
        ));
        const user = userEvent.setup();
        const input = screen.getByDisplayValue("old");
        await user.clear(input);
        await user.type(input, "new name");
        await user.click(screen.getByText("elsewhere"));
        expect(onConfirm).toHaveBeenCalledWith("new name");
    });

    it("cancels on Escape without confirming, reverting nothing further", async () => {
        const onConfirm = vi.fn();
        const onCancel = vi.fn();
        render(() => (
            <PaneTabRenameInput initialValue="old" onConfirm={onConfirm} onCancel={onCancel} />
        ));
        const user = userEvent.setup();
        const input = screen.getByDisplayValue("old");
        await user.type(input, " more{Escape}");
        expect(onCancel).toHaveBeenCalledTimes(1);
        expect(onConfirm).not.toHaveBeenCalled();
    });

    it("cancels instead of confirming when the value is unchanged", async () => {
        const onConfirm = vi.fn();
        const onCancel = vi.fn();
        render(() => (
            <PaneTabRenameInput initialValue="same" onConfirm={onConfirm} onCancel={onCancel} />
        ));
        const user = userEvent.setup();
        await user.type(screen.getByDisplayValue("same"), "{Enter}");
        expect(onConfirm).not.toHaveBeenCalled();
        expect(onCancel).toHaveBeenCalledTimes(1);
    });

    it("cancels instead of confirming when the trimmed value is empty", async () => {
        const onConfirm = vi.fn();
        const onCancel = vi.fn();
        render(() => (
            <PaneTabRenameInput initialValue="old" onConfirm={onConfirm} onCancel={onCancel} />
        ));
        const user = userEvent.setup();
        const input = screen.getByDisplayValue("old");
        await user.clear(input);
        await user.type(input, "   {Enter}");
        expect(onConfirm).not.toHaveBeenCalled();
        expect(onCancel).toHaveBeenCalledTimes(1);
    });

    it("does not double-fire between Enter and the subsequent blur", async () => {
        const onConfirm = vi.fn();
        render(() => (
            <>
                <PaneTabRenameInput initialValue="old" onConfirm={onConfirm} onCancel={vi.fn()} />
                <button>elsewhere</button>
            </>
        ));
        const user = userEvent.setup();
        const input = screen.getByDisplayValue("old");
        await user.clear(input);
        await user.type(input, "new{Enter}");
        await user.click(screen.getByText("elsewhere"));
        expect(onConfirm).toHaveBeenCalledTimes(1);
    });
});
