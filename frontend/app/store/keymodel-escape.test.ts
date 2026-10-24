// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * One Escape does one thing: an open modal, then an active search, then picked Pane Tabs
 * (Ctrl/Cmd+click), and only a key none of those took goes on to the rest (un-magnify).
 * docs/specs/SPEC_PANE_TAB_MULTI_SELECT_MOVE_AND_COLLAPSED_EXPAND_2026_10_24.md §4.6.
 */
import { beforeEach, describe, expect, it, vi } from "vitest";

const h = vi.hoisted(() => ({
    modalOpen: false,
    closeTopModal: vi.fn(),
    searchOpen: false,
    searchSet: vi.fn(),
}));

vi.mock("@/app/hook/useVoiceInput", () => ({ getVoiceSession: () => null }));
vi.mock("@/app/host/host-caps", () => ({ hostHas: () => true }));
vi.mock("@/app/view/term/term-models", () => ({ basicTermModels: () => [] }));
vi.mock("@/app/store/global", () => ({
    atoms: {},
    createTab: vi.fn(),
    getBlockComponentModel: () => ({
        viewModel: { searchAtoms: { isOpen: Object.assign(() => h.searchOpen, { _set: h.searchSet }) } },
    }),
    getFocusedBlockId: () => "b1",
    openOrFocusPaneByView: vi.fn(),
    settingsAtom: () => ({}),
    replaceBlock: vi.fn(),
    setIsTermMultiInput: vi.fn(),
}));
vi.mock("@/app/store/zoom", () => ({ zoomAllPanesReset: vi.fn(), zoomIn: vi.fn(), zoomOut: vi.fn(), zoomReset: vi.fn() }));
vi.mock("@/app/tab/tab-rename-request", () => ({ requestTabRename: vi.fn() }));
vi.mock("@/app/view/agent/composer-focus", () => ({ requestComposerFocus: vi.fn() }));
vi.mock("@/layout/lib/layoutKeyboard", () => ({ resizeFocusedInDirection: vi.fn(), swapFocusedInDirection: vi.fn() }));
vi.mock("@/layout/index", () => ({ getLayoutModelForStaticTab: () => null, NavigateDirection: {} }));
vi.mock("./modalmodel", () => ({
    modalsModel: { hasOpenModals: () => h.modalOpen, closeTopModal: h.closeTopModal },
    openModal: vi.fn(),
}));
vi.mock("@/app/modals/command-palette", () => ({ CommandPaletteModal: {} }));
vi.mock("@/app/modals/replace-pane-confirm", () => ({ ReplacePaneConfirm: {} }));
vi.mock("./keymodel-blockcreate", () => ({
    handleCmdN: vi.fn(),
    handleSplitHorizontal: vi.fn(),
    handleSplitVertical: vi.fn(),
}));
vi.mock("./keymodel-nav", () => ({
    cyclePaneFocus: vi.fn(),
    genericClose: vi.fn(),
    getFocusedBlockInStaticTab: () => "b1",
    handleCmdI: vi.fn(),
    simpleCloseStaticTab: vi.fn(),
    switchBlockByBlockNum: vi.fn(),
    switchBlockInDirection: vi.fn(),
    switchTab: vi.fn(),
    switchTabAbs: vi.fn(),
    switchTabLast: vi.fn(),
    moveActiveTab: vi.fn(),
    globalRefocus: vi.fn(),
    globalRefocusWithTimeout: vi.fn(),
}));
vi.mock("@/app/keybindings/registry", () => ({ setUserKeybindings: () => [] }));
vi.mock("./command-registry", () => ({ commandRegistry: { run: vi.fn(() => false), get: () => null } }));
vi.mock("./window-appearance", () => ({ openNewWindowFromHere: vi.fn() }));

import { clearPaneTabSelection, paneTabSelection, resetPaneTabSelection, togglePaneTabSelected } from "@/app/element/pane-tab-selection";
import { keyCommands } from "./keymodel-dispatch";
import { registerGlobalKeys } from "./keymodel";

const escape = () => keyCommands.get("app:escape")!({} as never);

beforeEach(() => {
    resetPaneTabSelection();
    h.modalOpen = false;
    h.searchOpen = false;
    h.closeTopModal.mockReset();
    h.searchSet.mockReset();
    registerGlobalKeys();
});

describe("app:escape", () => {
    it("clears picked Pane Tabs, and the key is taken so un-magnify does not also run", () => {
        togglePaneTabSelected("a");
        togglePaneTabSelected("b");
        expect(escape()).toBe(true);
        expect(paneTabSelection().size).toBe(0);
    });

    it("is not taken when nothing is picked, so un-magnify and the rest still get Escape", () => {
        expect(escape()).toBe(false);
    });

    it("closes an open modal first and leaves the pick for the next Escape", () => {
        togglePaneTabSelected("a");
        h.modalOpen = true;
        expect(escape()).toBe(true);
        expect(h.closeTopModal).toHaveBeenCalledTimes(1);
        expect(paneTabSelection().size).toBe(1);
    });

    it("closes an active search before it clears the pick", () => {
        togglePaneTabSelected("a");
        h.searchOpen = true;
        expect(escape()).toBe(true);
        expect(h.searchSet).toHaveBeenCalledWith(false);
        expect(paneTabSelection().size).toBe(1);
        h.searchOpen = false;
        expect(escape()).toBe(true);
        expect(clearPaneTabSelection()).toBe(false);
    });
});
