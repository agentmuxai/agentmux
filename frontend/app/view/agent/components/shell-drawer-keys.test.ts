// Copyright 2026-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it, vi } from "vitest";
import { handleShellDrawerKeydown } from "./shell-drawer-keys";
import type { ShellDrawerMenuDeps } from "./shell-drawer-menu";

function setup(over: Partial<ShellDrawerMenuDeps> & { selection?: string; clipboard?: string | null } = {}) {
    const paste = vi.fn();
    const writeClipboard = vi.fn(() => Promise.resolve());
    const notify = vi.fn();
    const deps: ShellDrawerMenuDeps = {
        getTerminal: () => ({ getSelection: () => over.selection ?? "", paste }),
        isAgentLocked: () => false,
        readClipboard: () => Promise.resolve(over.clipboard === undefined ? "hello" : over.clipboard),
        writeClipboard,
        notify,
        ...over,
    };
    return { deps, paste, writeClipboard, notify };
}

function key(init: KeyboardEventInit & { key: string; code: string }, type = "keydown") {
    const e = new KeyboardEvent(type, { bubbles: true, cancelable: true, ...init });
    const preventDefault = vi.spyOn(e, "preventDefault");
    const stopPropagation = vi.spyOn(e, "stopPropagation");
    return { e, preventDefault, stopPropagation };
}

const ctrlShiftV = { key: "V", code: "KeyV", ctrlKey: true, shiftKey: true };
const ctrlShiftC = { key: "C", code: "KeyC", ctrlKey: true, shiftKey: true };
const flush = () => new Promise((r) => setTimeout(r, 0));

describe("Ctrl+Shift+V in the shell drawer", () => {
    it("pastes the clipboard, and keeps the event from reaching the global voice hotkey", async () => {
        const { deps, paste } = setup({ clipboard: "echo hi" });
        const { e, preventDefault, stopPropagation } = key(ctrlShiftV);
        expect(handleShellDrawerKeydown(e, deps)).toBe(false);
        expect(preventDefault).toHaveBeenCalled();
        expect(stopPropagation).toHaveBeenCalled();
        await flush();
        expect(paste).toHaveBeenCalledWith("echo hi");
    });

    it("does nothing on keyup, so one chord pastes once", async () => {
        const { deps, paste } = setup();
        const { e, preventDefault } = key(ctrlShiftV, "keyup");
        expect(handleShellDrawerKeydown(e, deps)).toBe(true);
        expect(preventDefault).not.toHaveBeenCalled();
        await flush();
        expect(paste).not.toHaveBeenCalled();
    });

    it("is swallowed but pastes nothing while an agent holds the shell", async () => {
        const { deps, paste } = setup({ isAgentLocked: () => true });
        const { e, stopPropagation } = key(ctrlShiftV);
        expect(handleShellDrawerKeydown(e, deps)).toBe(false);
        expect(stopPropagation).toHaveBeenCalled();
        await flush();
        expect(paste).not.toHaveBeenCalled();
    });

    it("obeys the same size limit as the menu", async () => {
        const { deps, paste, notify } = setup({ clipboard: "a".repeat(1024 * 1024 + 1) });
        handleShellDrawerKeydown(key(ctrlShiftV).e, deps);
        await flush();
        expect(paste).not.toHaveBeenCalled();
        expect(notify).toHaveBeenCalledTimes(1);
    });
});

describe("Ctrl+Shift+C in the shell drawer", () => {
    it("copies the terminal selection", () => {
        const { deps, writeClipboard } = setup({ selection: "ls -la" });
        const { e, preventDefault, stopPropagation } = key(ctrlShiftC);
        expect(handleShellDrawerKeydown(e, deps)).toBe(false);
        expect(preventDefault).toHaveBeenCalled();
        expect(stopPropagation).toHaveBeenCalled();
        expect(writeClipboard).toHaveBeenCalledWith("ls -la");
    });

    it("with no selection writes nothing but is still swallowed (never sent to the shell as ^C)", () => {
        const { deps, writeClipboard } = setup({ selection: "" });
        expect(handleShellDrawerKeydown(key(ctrlShiftC).e, deps)).toBe(false);
        expect(writeClipboard).not.toHaveBeenCalled();
    });
});

describe("everything else is left to xterm", () => {
    it.each([
        ["plain v", { key: "v", code: "KeyV" }],
        ["Ctrl+V (native paste event handles it)", { key: "v", code: "KeyV", ctrlKey: true }],
        ["Ctrl+C (interrupt)", { key: "c", code: "KeyC", ctrlKey: true }],
        ["Ctrl+Shift+X", { key: "X", code: "KeyX", ctrlKey: true, shiftKey: true }],
        ["Shift+V", { key: "V", code: "KeyV", shiftKey: true }],
    ])("%s", (_name, init) => {
        const { deps, paste, writeClipboard } = setup({ selection: "x" });
        const { e, preventDefault } = key(init as any);
        expect(handleShellDrawerKeydown(e, deps)).toBe(true);
        expect(preventDefault).not.toHaveBeenCalled();
        expect(paste).not.toHaveBeenCalled();
        expect(writeClipboard).not.toHaveBeenCalled();
    });
});
