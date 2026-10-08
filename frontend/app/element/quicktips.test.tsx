// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// SPEC_HELP_PANE_FILTER_2026_10_08.md: the Help pane's filter.

import { cleanup, render, screen } from "@solidjs/testing-library";
import { afterEach, describe, expect, it, vi } from "vitest";

vi.mock("@/app/keybindings", () => ({
    shortcutFor: (command: string) => (command === "pane:close" ? "⌘W" : ""),
    shortcutHelp: () => [
        { category: "Panes", entries: [{ label: "Split pane right", keys: ["⌘D"] }, { label: "Close pane", keys: ["⌘W"] }] },
        { category: "Tabs & windows", entries: [{ label: "New tab", keys: ["⌘T"] }] },
    ],
}));

import { QuickTips } from "./quicktips";
import { keyLabelWords } from "@/app/keybindings/help";

afterEach(() => cleanup());

describe("QuickTips filter", () => {
    it("shows everything, including the alpha notice, with no filter", () => {
        render(() => <QuickTips />);
        expect(screen.getByText("Split pane right")).toBeTruthy();
        expect(screen.getByText("New tab")).toBeTruthy();
        expect(screen.getByText("Join Our Discord")).toBeTruthy();
        expect(screen.getByText(/Alpha Software/)).toBeTruthy();
    });

    it("keeps only matching items, drops empty groups and cards, and hides the alpha notice", () => {
        render(() => <QuickTips filter="split" />);
        expect(screen.getByText("Split pane right")).toBeTruthy();
        expect(screen.queryByText("Close pane")).toBeNull();
        expect(screen.queryByText("Tabs & windows")).toBeNull();
        expect(screen.queryByText("More Tips")).toBeNull();
        expect(screen.queryByText("Need More Help?")).toBeNull();
        expect(screen.queryByText(/Alpha Software/)).toBeNull();
    });

    it("matches a group name, so every entry in it shows", () => {
        render(() => <QuickTips filter="tabs" />);
        expect(screen.getByText("New tab")).toBeTruthy();
        expect(screen.queryByText("Split pane right")).toBeNull();
    });

    it("matches keys by their symbol names", () => {
        render(() => <QuickTips filter="cmd t" />);
        expect(screen.getByText("New tab")).toBeTruthy();
    });

    it("says so when nothing matches", () => {
        render(() => <QuickTips filter="zzzz" />);
        expect(screen.getByText(/Nothing in Help matches/)).toBeTruthy();
        expect(screen.queryByText("Keyboard Shortcuts")).toBeNull();
    });
});

describe("keyLabelWords", () => {
    it("adds the names of macOS key symbols", () => {
        expect(keyLabelWords("⇧⌘W")).toBe("⇧⌘W shift cmd command");
        expect(keyLabelWords("Ctrl+T")).toBe("Ctrl+T");
    });
});
