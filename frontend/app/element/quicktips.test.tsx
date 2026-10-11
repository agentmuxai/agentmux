// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// SPEC_HELP_PANE_FILTER_2026_10_08.md: the Help pane's filter.

import { cleanup, render, screen } from "@solidjs/testing-library";
import { afterEach, describe, expect, it, vi } from "vitest";

vi.mock("@/app/keybindings", () => ({
    keyPlatform: () => "other",
    shortcutFor: (command: string) => (command === "pane:close" ? "⌘W" : ""),
    shortcutHelp: () => [
        { category: "Panes", entries: [{ label: "Split pane right", keys: ["⌘D"] }, { label: "Close pane", keys: ["⌘W"] }] },
        { category: "Tabs & windows", entries: [{ label: "New tab", keys: ["⌘T"] }] },
    ],
}));

import { QuickTips } from "./quicktips";
import { keyLabelWords } from "@/app/keybindings/help";
import { setPlatform } from "@/util/platformutil";

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

describe("Mouse and gestures (SPEC_HELP_HIDDEN_TIPS_2026_10_10.md)", () => {
    it("shows the hidden tips, and the filter finds them by gesture and place", () => {
        setPlatform("win32");
        render(() => <QuickTips filter="middle-click" />);
        expect(screen.getByText("Mouse and gestures")).toBeTruthy();
        expect(screen.getByText("Close the tab")).toBeTruthy();
        expect(screen.getByText("Open it in a new tab")).toBeTruthy();
        expect(screen.queryByText("Split pane right")).toBeNull();
    });

    it("hides the card when the filter matches no tip", () => {
        render(() => <QuickTips filter="split" />);
        expect(screen.queryByText("Mouse and gestures")).toBeNull();
    });

    it("leaves out a tip for another operating system", () => {
        setPlatform("darwin");
        render(() => <QuickTips filter="edge" />);
        expect(screen.queryByText(/Give all the size change/)).toBeNull();
        cleanup();
        setPlatform("win32");
        render(() => <QuickTips filter="edge" />);
        expect(screen.getByText(/Give all the size change/)).toBeTruthy();
    });
});

describe("keyLabelWords", () => {
    it("adds the names of macOS key symbols", () => {
        expect(keyLabelWords("⇧⌘W")).toBe("⇧⌘W shift cmd command");
        expect(keyLabelWords("Ctrl+T")).toBe("Ctrl+T");
    });
});
