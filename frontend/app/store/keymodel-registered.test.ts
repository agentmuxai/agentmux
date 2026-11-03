// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * `registerGlobalKeys` fills the command table `keyCommands` that the shortcut table
 * (keybindings/defaults.ts) points into: a command it forgets is a dead shortcut.
 */
import { describe, expect, it, vi } from "vitest";

vi.mock("@/app/hook/useVoiceInput", () => ({ getVoiceSession: () => null }));
vi.mock("@/util/host-caps", () => ({ hostHas: () => true }));
vi.mock("@/app/view/term/term-models", () => ({ basicTermModels: () => [] }));
vi.mock("@/app/store/global", () => ({
    atoms: {},
    settingsAtom: () => ({}),
    setIsTermMultiInput: vi.fn(),
}));
vi.mock("@/app/store/tab-actions", () => ({ createTab: vi.fn() }));
vi.mock("@/app/store/block-layout-actions", () => ({ replaceBlock: vi.fn() }));
vi.mock("@/app/store/block-component-registry", () => ({
    getBlockComponentModel: () => ({
        viewModel: { searchAtoms: { isOpen: Object.assign(() => false, { _set: vi.fn() }) } },
    }),
    getFocusedBlockId: () => "b1",
    openOrFocusPaneByView: vi.fn(),
}));
vi.mock("@/app/store/zoom", () => ({ zoomAllPanesReset: vi.fn(), zoomIn: vi.fn(), zoomOut: vi.fn(), zoomReset: vi.fn() }));
vi.mock("@/app/tab/tab-rename-request", () => ({ requestTabRename: vi.fn() }));
vi.mock("@/app/agent-model/composer-focus", () => ({ requestComposerFocus: vi.fn() }));
vi.mock("@/layout/lib/layoutKeyboard", () => ({ resizeFocusedInDirection: vi.fn(), swapFocusedInDirection: vi.fn() }));
vi.mock("@/layout/index", () => ({ getLayoutModelForStaticTab: () => null, NavigateDirection: {} }));
vi.mock("./modalmodel", () => ({
    modalsModel: { hasOpenModals: () => false, closeTopModal: vi.fn() },
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

import { keyCommands } from "./keymodel-dispatch";
import { registerGlobalKeys } from "./keymodel";

describe("registerGlobalKeys", () => {
    it("registers these commands, in this order", () => {
        keyCommands.clear();
        registerGlobalKeys();
        expect(Array.from(keyCommands.keys())).toMatchInlineSnapshot(`
          [
            "view:command-palette",
            "app:settings",
            "help:shortcuts",
            "app:escape",
            "window:new",
            "tab:new",
            "tab:close",
            "tab:next",
            "tab:prev",
            "tab:goto:1",
            "tab:goto:2",
            "tab:goto:3",
            "tab:goto:4",
            "tab:goto:5",
            "tab:goto:6",
            "tab:goto:7",
            "tab:goto:8",
            "tab:goto:last",
            "tab:moveLeft",
            "tab:moveRight",
            "tab:rename",
            "pane:new",
            "split:right",
            "split:down",
            "pane:close",
            "pane:magnify",
            "pane:focus:up",
            "pane:focus:down",
            "pane:focus:left",
            "pane:focus:right",
            "pane:focus:next",
            "pane:focus:prev",
            "pane:focus:1",
            "pane:focus:2",
            "pane:focus:3",
            "pane:focus:4",
            "pane:focus:5",
            "pane:focus:6",
            "pane:focus:7",
            "pane:focus:8",
            "pane:focus:9",
            "pane:swap:up",
            "pane:resize:up",
            "pane:swap:down",
            "pane:resize:down",
            "pane:swap:left",
            "pane:resize:left",
            "pane:swap:right",
            "pane:resize:right",
            "pane:refocus",
            "agent:focusComposer",
            "pane:replaceWithLauncher",
            "pane:changeConnection",
            "term:multiInput",
            "pane:voice",
            "pane:find",
            "view:zoom:in",
            "view:zoom:out",
            "view:zoom:reset",
            "view:zoom:resetAll",
            "term:copy",
            "term:paste",
            "term:clear",
          ]
        `);
    });

    it("registers a handler function for every command", () => {
        keyCommands.clear();
        registerGlobalKeys();
        for (const [id, handler] of keyCommands) expect(typeof handler, id).toBe("function");
    });
});
