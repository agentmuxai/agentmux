// Copyright 2026-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it, vi } from "vitest";
import {
    buildShellDrawerClipboardItems,
    formatSize,
    SHELL_PASTE_MAX_BYTES,
    type ShellDrawerMenuDeps,
} from "./shell-drawer-menu";

function setup(over: Partial<ShellDrawerMenuDeps> & { selection?: string; clipboard?: string | null } = {}) {
    const paste = vi.fn();
    const notify = vi.fn();
    const writeClipboard = vi.fn(() => Promise.resolve());
    const deps: ShellDrawerMenuDeps = {
        getTerminal: () => ({ getSelection: () => over.selection ?? "", paste }),
        isAgentLocked: () => false,
        readClipboard: () => Promise.resolve(over.clipboard === undefined ? "hello" : over.clipboard),
        writeClipboard,
        notify,
        ...over,
    };
    const [copy, pasteItem] = buildShellDrawerClipboardItems(deps);
    return { copy, pasteItem, paste, notify, writeClipboard };
}

const flush = () => new Promise((r) => setTimeout(r, 0));

describe("shell drawer Copy", () => {
    it("is disabled with no terminal selection", () => {
        expect(setup({ selection: "" }).copy.enabled).toBe(false);
    });

    it("copies the terminal selection captured at menu-build time", () => {
        const { copy, writeClipboard } = setup({ selection: "ls -la" });
        expect(copy.enabled).toBe(true);
        copy.click!();
        expect(writeClipboard).toHaveBeenCalledWith("ls -la");
    });

    it("is disabled when there is no terminal yet", () => {
        expect(setup({ getTerminal: () => undefined }).copy.enabled).toBe(false);
    });
});

describe("shell drawer Paste", () => {
    it("pastes the clipboard through terminal.paste", async () => {
        const { pasteItem, paste } = setup({ clipboard: "echo hi\nls" });
        expect(pasteItem.enabled).toBe(true);
        pasteItem.click!();
        await flush();
        expect(paste).toHaveBeenCalledWith("echo hi\nls");
    });

    it("advertises the size limit in the label (the JS menu renders no sublabels)", () => {
        const { pasteItem } = setup();
        expect(pasteItem.label).toBe("Paste (up to 1 MB)");
        expect(pasteItem.sublabel).toBeUndefined();
    });

    it("is disabled, with the reason, while the agent holds the shell", () => {
        const { pasteItem } = setup({ isAgentLocked: () => true });
        expect(pasteItem.enabled).toBe(false);
        expect(pasteItem.label).toBe("Paste (agent is using this shell)");
    });

    it("is disabled when there is no terminal yet", () => {
        expect(setup({ getTerminal: () => undefined }).pasteItem.enabled).toBe(false);
    });

    it("does nothing for an empty or unreadable clipboard", async () => {
        for (const clipboard of ["", null]) {
            const { pasteItem, paste, notify } = setup({ clipboard });
            pasteItem.click!();
            await flush();
            expect(paste).not.toHaveBeenCalled();
            expect(notify).not.toHaveBeenCalled();
        }
    });

    it("swallows a clipboard read failure", async () => {
        const err = vi.spyOn(console, "error").mockImplementation(() => {});
        const { pasteItem, paste } = setup({ readClipboard: () => Promise.reject(new Error("denied")) });
        pasteItem.click!();
        await flush();
        expect(paste).not.toHaveBeenCalled();
        err.mockRestore();
    });

    it("pastes exactly at the limit", async () => {
        const { pasteItem, paste, notify } = setup({ clipboard: "a".repeat(SHELL_PASTE_MAX_BYTES) });
        pasteItem.click!();
        await flush();
        expect(paste).toHaveBeenCalledTimes(1);
        expect(notify).not.toHaveBeenCalled();
    });

    it("refuses one byte over the limit, and tells the user the limit and what to do instead", async () => {
        const { pasteItem, paste, notify } = setup({ clipboard: "a".repeat(SHELL_PASTE_MAX_BYTES + 1) });
        pasteItem.click!();
        await flush();
        expect(paste).not.toHaveBeenCalled();
        expect(notify).toHaveBeenCalledTimes(1);
        const n = notify.mock.calls[0][0];
        expect(n.type).toBe("warning");
        expect(n.message).toContain("1 MB");
        expect(n.message).toMatch(/file/i);
    });

    it("measures the limit in UTF-8 bytes, not characters", async () => {
        // 3 bytes each: under the limit in characters, over it in bytes.
        const { pasteItem, paste, notify } = setup({ clipboard: "€".repeat(Math.ceil(SHELL_PASTE_MAX_BYTES / 3) + 1) });
        pasteItem.click!();
        await flush();
        expect(paste).not.toHaveBeenCalled();
        expect(notify).toHaveBeenCalledTimes(1);
    });
});

describe("formatSize", () => {
    it("formats sizes for the limit message", () => {
        expect(formatSize(1024 * 1024)).toBe("1 MB");
        expect(formatSize(3.2 * 1024 * 1024)).toBe("3.2 MB");
        expect(formatSize(512 * 1024)).toBe("512 KB");
        expect(formatSize(10)).toBe("1 KB");
    });
});
