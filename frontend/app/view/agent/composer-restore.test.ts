// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { afterEach, describe, expect, it, vi } from "vitest";

const drafts = vi.hoisted(() => new Map<string, string>());
vi.mock("./reload-persistence", () => ({
    getComposerDraft: (blockId: string) => drafts.get(blockId),
    setComposerDraft: (blockId: string, text: string) => void drafts.set(blockId, text),
}));

import { joinDraft, registerComposerRestore, restoreToComposer } from "./composer-restore";

afterEach(() => drafts.clear());

describe("joinDraft", () => {
    it("is the restored text alone when the composer is empty or blank", () => {
        expect(joinDraft("", "kept")).toBe("kept");
        expect(joinDraft("  \n", "kept")).toBe("kept");
    });

    it("puts the restored text after what the user typed since, after a blank line", () => {
        expect(joinDraft("typed", "kept")).toBe("typed\n\nkept");
        expect(joinDraft("typed \n", "kept")).toBe("typed\n\nkept");
    });
});

describe("restoreToComposer", () => {
    it("hands the text to the mounted footer, and leaves the stored draft to it", () => {
        const write = vi.fn();
        const unregister = registerComposerRestore("b1", write);
        restoreToComposer("b1", "kept");
        expect(write).toHaveBeenCalledWith("kept");
        expect(drafts.has("b1")).toBe(false);
        unregister();
    });

    it("stores it as the draft when no footer is mounted, after the draft already there", () => {
        restoreToComposer("b1", "first");
        expect(drafts.get("b1")).toBe("first");
        restoreToComposer("b1", "second");
        expect(drafts.get("b1")).toBe("first\n\nsecond");
    });

    it("stores it once the footer has unmounted, and only for that pane", () => {
        const write = vi.fn();
        registerComposerRestore("b1", write)();
        restoreToComposer("b1", "kept");
        expect(write).not.toHaveBeenCalled();
        expect(drafts.get("b1")).toBe("kept");
        expect(drafts.has("b2")).toBe(false);
    });

    it("keeps a newer footer's writer when an older footer unregisters", () => {
        const older = vi.fn();
        const newer = vi.fn();
        const unregisterOlder = registerComposerRestore("b1", older);
        registerComposerRestore("b1", newer);
        unregisterOlder();
        restoreToComposer("b1", "kept");
        expect(newer).toHaveBeenCalledWith("kept");
    });

    it("does nothing for an empty text", () => {
        restoreToComposer("b1", "");
        expect(drafts.size).toBe(0);
    });
});
