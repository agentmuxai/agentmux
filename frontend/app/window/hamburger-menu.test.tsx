// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The hamburger menu's shortcut labels must be the bindings keymodel.ts
 * actually registers. They used to be hand-written, and three were wrong:
 * "Ctrl+T" for New Tab on Windows/Linux (the binding is Cmd:t, i.e. Alt+T),
 * "⌘⇧N" for New Window on macOS and "⌘P" for Command Palette on macOS (both
 * bindings use Ctrl, which is Control on macOS too).
 *
 * These tests run the real registerGlobalKeys and the real key dispatcher
 * (appHandleKeyDown). Only the actions at the edges are mocked, so a label
 * that stops matching its binding fails here.
 */

import { cleanup, render } from "@solidjs/testing-library";
import { createRoot, createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const h = vi.hoisted(() => {
    const createTab = vi.fn();
    const openNewWindow = vi.fn(() => Promise.resolve());
    const openModal = vi.fn();
    const CommandPaletteModal = () => null;
    const store = {
        atoms: { modalOpen: () => false, isTermMultiInput: () => false },
        createTab,
        getAllBlockComponentModels: () => [],
        getApi: () => ({
            openNewWindow,
            setKeyboardChordMode: vi.fn(),
            toggleDevtools: vi.fn(),
            openExternal: vi.fn(),
            closeWindow: vi.fn(() => Promise.resolve()),
        }),
        getBlockComponentModel: () => null,
        getFocusedBlockId: () => null,
        openOrFocusPaneByView: vi.fn(),
        replaceBlock: vi.fn(),
        setControlShiftDelayAtom: vi.fn(),
        setIsTermMultiInput: vi.fn(),
        // A test can swap in a reactive source (`settings.read`).
        settingsAtom: () => settings.read(),
    };
    const settings = { read: (): Record<string, unknown> => ({}) };
    const caps = { multiWindow: true };
    return { createTab, openNewWindow, openModal, CommandPaletteModal, store, settings, caps, menu: { items: [] as MenuItem[] } };
});

// keymodel.ts imports the store as "@/app/store/global", the menu as "@/store/global".
vi.mock("@/app/store/global", () => h.store);
vi.mock("@/store/global", () => h.store);
vi.mock("@/app/store/modalmodel", () => ({
    modalsModel: { hasOpenModals: () => false, closeTopModal: vi.fn() },
    openModal: h.openModal,
}));
vi.mock("@/app/modals/command-palette", () => ({ CommandPaletteModal: h.CommandPaletteModal }));
vi.mock("@/app/store/rpc-api", () => ({ RpcApi: { SetConfigCommand: vi.fn() } }));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("@/app/store/command-registry", () => ({ commandRegistry: { run: vi.fn(() => false) } }));
vi.mock("@/app/hook/useVoiceInput", () => ({ getVoiceSession: vi.fn() }));
vi.mock("@/app/host/host-caps", () => ({ hostHas: (cap: string) => cap === "multiWindow" && h.caps.multiWindow }));
vi.mock("@/app/store/zoom", () => ({ zoomIn: vi.fn(), zoomOut: vi.fn(), zoomReset: vi.fn() }));
vi.mock("@/layout/index", () => ({
    getLayoutModelForStaticTab: () => ({ focusedNode: () => null }),
    NavigateDirection: { Up: 0, Right: 1, Down: 2, Left: 3 },
}));
vi.mock("@/app/store/keymodel-blockcreate", () => ({
    handleCmdN: vi.fn(),
    handleSplitHorizontal: vi.fn(),
    handleSplitVertical: vi.fn(),
}));
vi.mock("@/app/store/keymodel-nav", () => ({
    cyclePaneFocus: vi.fn(),
    genericClose: vi.fn(),
    getFocusedBlockInStaticTab: () => null,
    globalRefocus: vi.fn(),
    globalRefocusWithTimeout: vi.fn(),
    handleCmdI: vi.fn(),
    simpleCloseStaticTab: vi.fn(),
    switchBlockByBlockNum: vi.fn(),
    switchBlockInDirection: vi.fn(),
    switchTab: vi.fn(),
    switchTabAbs: vi.fn(),
}));
// The real FlyoutMenu positions a floating panel, which jsdom can't lay out.
// Capture the items it is given; it renders item.shortcut verbatim.
vi.mock("@/app/element/flyoutmenu", () => ({
    FlyoutMenu: (props: { items: MenuItem[]; children?: unknown }) => {
        h.menu.items = props.items;
        return props.children;
    },
}));

import { formatCommand } from "@/app/keybindings/registry";
import { registerGlobalKeys } from "@/app/store/keymodel";
import { appHandleKeyDown, keyCommands } from "@/app/store/keymodel-dispatch";
import { adaptFromReactOrNativeKeyEvent, setKeyUtilPlatform } from "@/util/keyutil";
import { setPlatform } from "@/util/platformutil";
import { HamburgerMenu } from "./hamburger-menu";

type Platform = "darwin" | "win32" | "linux";
type Keys = { key: string; code: string; ctrlKey?: boolean; altKey?: boolean; shiftKey?: boolean; metaKey?: boolean };

const ITEMS: {
    label: string;
    command: string;
    shown: Record<Platform, string>;
    // The physical keys a user presses after reading the label.
    press: Record<Platform, Keys>;
    // The action both the menu item and the key binding must perform.
    action: () => ReturnType<typeof vi.fn>;
    // A label this item used to show, which no longer fires the binding.
    oldLabel?: { platforms: Platform[]; keys: Keys };
}[] = [
    {
        label: "New Tab",
        command: "tab:new",
        shown: { darwin: "⌘T", win32: "Ctrl+Shift+T", linux: "Ctrl+Shift+T" },
        press: {
            darwin: { key: "t", code: "KeyT", metaKey: true },
            win32: { key: "T", code: "KeyT", ctrlKey: true, shiftKey: true },
            linux: { key: "T", code: "KeyT", ctrlKey: true, shiftKey: true },
        },
        action: () => h.createTab,
        // Alt+T is the shell's transpose-words (report §11.2).
        oldLabel: { platforms: ["win32", "linux"], keys: { key: "t", code: "KeyT", altKey: true } },
    },
    {
        label: "New Window",
        command: "window:new",
        shown: { darwin: "⇧⌘N", win32: "Ctrl+Shift+N", linux: "Ctrl+Shift+N" },
        press: {
            darwin: { key: "N", code: "KeyN", metaKey: true, shiftKey: true },
            win32: { key: "N", code: "KeyN", ctrlKey: true, shiftKey: true },
            linux: { key: "N", code: "KeyN", ctrlKey: true, shiftKey: true },
        },
        action: () => h.openNewWindow,
        oldLabel: { platforms: ["darwin"], keys: { key: "N", code: "KeyN", ctrlKey: true, shiftKey: true } },
    },
    {
        label: "Command Palette",
        command: "view:command-palette",
        shown: { darwin: "⇧⌘P", win32: "Ctrl+Shift+P", linux: "Ctrl+Shift+P" },
        press: {
            darwin: { key: "P", code: "KeyP", metaKey: true, shiftKey: true },
            win32: { key: "P", code: "KeyP", ctrlKey: true, shiftKey: true },
            linux: { key: "P", code: "KeyP", ctrlKey: true, shiftKey: true },
        },
        action: () => h.openModal,
        oldLabel: { platforms: ["darwin"], keys: { key: "p", code: "KeyP", ctrlKey: true } },
    },
    {
        label: "Settings",
        command: "app:settings",
        shown: { darwin: "⌘,", win32: "Ctrl+,", linux: "Ctrl+," },
        press: {
            darwin: { key: ",", code: "Comma", metaKey: true },
            win32: { key: ",", code: "Comma", ctrlKey: true },
            linux: { key: ",", code: "Comma", ctrlKey: true },
        },
        action: () => h.store.openOrFocusPaneByView,
    },
];

function menuItem(label: string): MenuItem {
    const item = h.menu.items.find((it) => it.label === label);
    expect(item, `menu item "${label}"`).toBeDefined();
    return item;
}

function pressKeys(keys: Keys): boolean {
    return appHandleKeyDown(adaptFromReactOrNativeKeyEvent(new KeyboardEvent("keydown", keys)));
}

describe.each(["darwin", "win32", "linux"] as Platform[])("hamburger menu shortcut labels on %s", (platform) => {
    beforeEach(() => {
        setKeyUtilPlatform(platform);
        setPlatform(platform);
        keyCommands.clear();
        registerGlobalKeys();
        h.menu.items = [];
        render(() => <HamburgerMenu />);
        vi.clearAllMocks();
    });

    afterEach(() => {
        cleanup();
        setKeyUtilPlatform("darwin");
        setPlatform("darwin");
    });

    describe.each(ITEMS)("$label", (spec) => {
        it("shows the command's shortcut from the shortcut table", () => {
            const mac = platform === "darwin" ? "mac" : "other";
            expect(keyCommands.has(spec.command), `keymodel handles ${spec.command}`).toBe(true);
            expect(menuItem(spec.label).shortcut).toBe(formatCommand(spec.command, mac));
            expect(menuItem(spec.label).shortcut).toBe(spec.shown[platform]);
        });

        it("pressing the keys on the label does what clicking the item does", () => {
            const action = spec.action();
            expect(pressKeys(spec.press[platform])).toBe(true);
            expect(action).toHaveBeenCalledTimes(1);

            menuItem(spec.label).onClick(new MouseEvent("click"));
            expect(action).toHaveBeenCalledTimes(2);
            expect(action.mock.calls[1]).toEqual(action.mock.calls[0]);
        });

        if (spec.oldLabel?.platforms.includes(platform)) {
            it("the label it used to show does not fire the action", () => {
                pressKeys(spec.oldLabel.keys);
                expect(spec.action()).not.toHaveBeenCalled();
            });
        }
    });

    it("shows a shortcut on exactly the items that have one", () => {
        const shown = h.menu.items.filter((item) => item.shortcut);
        expect(shown.map((item) => item.label)).toEqual(["New Tab", "New Window", "Settings", "Command Palette"]);
    });
});

// SPEC_HOST_API_SEAM_2026_09_26.md: on a host without multiWindow there is no
// "New Window" item, and its key isn't taken, so it reaches the terminal.
describe("on a host without multiWindow", () => {
    beforeEach(() => {
        h.caps.multiWindow = false;
        setKeyUtilPlatform("win32");
        setPlatform("win32");
        keyCommands.clear();
        registerGlobalKeys();
        h.menu.items = [];
        render(() => <HamburgerMenu />);
        vi.clearAllMocks();
    });

    afterEach(() => {
        cleanup();
        h.caps.multiWindow = true;
        setKeyUtilPlatform("darwin");
        setPlatform("darwin");
    });

    it("leaves New Window out of the menu", () => {
        const labels = h.menu.items.map((i) => i.label);
        expect(labels).not.toContain("New Window");
        expect(labels).toContain("New Tab");
    });

    it("declines Ctrl+Shift+N without opening a window", () => {
        expect(pressKeys({ key: "N", code: "KeyN", ctrlKey: true, shiftKey: true })).toBe(false);
        expect(h.openNewWindow).not.toHaveBeenCalled();
    });
});

// SPEC_LAYOUT_FILES_2026_09_25.md §6.1: ☰ → Layouts → "Save layout…", between
// Opacity and the divider above Settings (placement per the 08-13 spec §5.1).
describe("Layouts entry", () => {
    it("sits after Opacity and offers Save layout… and Open layout…", () => {
        render(() => <HamburgerMenu />);
        const labels = h.menu.items.map((i) => i.label);
        const at = labels.indexOf("Layouts");
        expect(at).toBe(labels.indexOf("Opacity") + 1);
        expect(h.menu.items[at + 1].divider).toBe(true);
        expect(h.menu.items[at].subItems?.map((i) => i.label)).toEqual(["Save layout…", "Open layout…"]);
        for (const item of h.menu.items[at].subItems ?? []) {
            expect(typeof item.onClick).toBe("function");
        }
        cleanup();
    });
});

// Theme and Opacity stay open while the user tries values (REPORT_THEME_MENU_
// RUNTIME_PANEL_TOOL_PREVIEW_TWEAKS_2026_10_07.md §1). A choice moves only the
// checkmark: rebuilding the items would remount the open submenu under the
// cursor, so the item list must survive a settings change.
describe("Theme and Opacity", () => {
    const sub = (label: string) => h.menu.items.find((i) => i.label === label)?.subItems ?? [];

    it("keep the menu open; Layouts' items close it", () => {
        render(() => <HamburgerMenu />);
        expect(sub("Theme").length).toBeGreaterThan(1);
        expect(sub("Theme").every((i) => i.keepOpen === true)).toBe(true);
        expect(sub("Opacity").every((i) => i.keepOpen === true)).toBe(true);
        expect(sub("Layouts").some((i) => i.keepOpen)).toBe(false);
        cleanup();
    });

    it("move the checkmark on a settings change without rebuilding the items", () => {
        const [cfg, setCfg] = createSignal<Record<string, unknown>>({ "window:theme": "default" });
        h.settings.read = cfg;
        createRoot((dispose) => {
            render(() => <HamburgerMenu />);
            const before = h.menu.items;
            expect(sub("Theme").filter((i) => i.checked)).toHaveLength(1);

            setCfg({ "window:theme": "__none__" });
            expect(h.menu.items).toBe(before);
            expect(sub("Theme").filter((i) => i.checked)).toHaveLength(0);
            dispose();
        });
        h.settings.read = () => ({});
        cleanup();
    });
});
