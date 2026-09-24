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
        settingsAtom: () => ({}),
    };
    return { createTab, openNewWindow, openModal, CommandPaletteModal, store, menu: { items: [] as MenuItem[] } };
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
vi.mock("@/app/hook/useVoiceInput", () => ({ getVoiceSession: vi.fn() }));
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

import { registerGlobalKeys } from "@/app/store/keymodel";
import { COMMAND_PALETTE_KEY, NEW_TAB_KEY, NEW_WINDOW_KEY } from "@/app/store/keymodel-bindings";
import { appHandleKeyDown, globalChordMap, globalKeyMap } from "@/app/store/keymodel-dispatch";
import { adaptFromReactOrNativeKeyEvent, formatKeyDescription, setKeyUtilPlatform } from "@/util/keyutil";
import { HamburgerMenu } from "./hamburger-menu";

type Platform = "darwin" | "win32" | "linux";
type Keys = { key: string; ctrlKey?: boolean; altKey?: boolean; shiftKey?: boolean; metaKey?: boolean };

const ITEMS: {
    label: string;
    binding: string;
    shown: Record<Platform, string>;
    // The physical keys a user presses after reading the label.
    press: Record<Platform, Keys>;
    // The action both the menu item and the key binding must perform.
    action: () => ReturnType<typeof vi.fn>;
    // A label this item used to show, which does NOT fire the binding.
    oldLabel?: { platforms: Platform[]; keys: Keys };
}[] = [
    {
        label: "New Tab",
        binding: NEW_TAB_KEY,
        shown: { darwin: "⌘T", win32: "Alt+T", linux: "Alt+T" },
        press: {
            darwin: { key: "t", metaKey: true },
            win32: { key: "t", altKey: true },
            linux: { key: "t", altKey: true },
        },
        action: () => h.createTab,
        oldLabel: { platforms: ["win32", "linux"], keys: { key: "t", ctrlKey: true } },
    },
    {
        label: "New Window",
        binding: NEW_WINDOW_KEY,
        shown: { darwin: "⌃⇧N", win32: "Ctrl+Shift+N", linux: "Ctrl+Shift+N" },
        press: {
            darwin: { key: "N", ctrlKey: true, shiftKey: true },
            win32: { key: "N", ctrlKey: true, shiftKey: true },
            linux: { key: "N", ctrlKey: true, shiftKey: true },
        },
        action: () => h.openNewWindow,
        oldLabel: { platforms: ["darwin"], keys: { key: "N", metaKey: true, shiftKey: true } },
    },
    {
        label: "Command Palette",
        binding: COMMAND_PALETTE_KEY,
        shown: { darwin: "⌃P", win32: "Ctrl+P", linux: "Ctrl+P" },
        press: {
            darwin: { key: "p", ctrlKey: true },
            win32: { key: "p", ctrlKey: true },
            linux: { key: "p", ctrlKey: true },
        },
        action: () => h.openModal,
        oldLabel: { platforms: ["darwin"], keys: { key: "p", metaKey: true } },
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
        globalKeyMap.clear();
        globalChordMap.clear();
        registerGlobalKeys();
        h.menu.items = [];
        render(() => <HamburgerMenu />);
        vi.clearAllMocks();
    });

    afterEach(() => {
        cleanup();
        setKeyUtilPlatform("darwin");
    });

    describe.each(ITEMS)("$label", (spec) => {
        it("shows the formatted binding that keymodel registers", () => {
            expect(globalKeyMap.has(spec.binding), `keymodel registers ${spec.binding}`).toBe(true);
            expect(menuItem(spec.label).shortcut).toBe(formatKeyDescription(spec.binding, platform));
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

    it("every shortcut the menu shows is the label of a registered binding", () => {
        const registeredLabels = new Set([...globalKeyMap.keys()].map((key) => formatKeyDescription(key, platform)));
        const shown = h.menu.items.filter((item) => item.shortcut);
        expect(shown.map((item) => item.label)).toEqual(["New Tab", "New Window", "Command Palette"]);
        for (const item of shown) {
            expect(registeredLabels.has(item.shortcut), `"${item.label}" shows ${item.shortcut}`).toBe(true);
        }
    });
});
