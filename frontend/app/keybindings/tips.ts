// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// The Help pane's hidden tips: gestures that aren't plain keys in the
// shortcut table (defaults.ts), such as modifier + mouse, double- and
// middle-click, drag and drop, and keys a pane handles itself.
// docs/specs/SPEC_HELP_HIDDEN_TIPS_2026_10_10.md. Each row anchors to the
// code it describes; tips.test.ts fails when that code changes, so a tip
// can't quietly go out of date.

import { formatKey, type KeyPlatform } from "./keys";

export type TipArea = "Panes" | "Window" | "Tabs" | "Terminal" | "Editor" | "Files" | "Media" | "Agent" | "Find" | "Browser";

export interface TipRow {
    id: string;
    area: TipArea;
    /**
     * The gesture, as "+"-joined tokens: modifiers (`shift`, `ctrl`, `alt`,
     * `mod` = ⌘ on macOS, Ctrl elsewhere), pointer words (`click`, `dblclick`,
     * `middleclick`, `rightclick`, `drag`, `wheel`, `drop`, `hold`), or a key
     * (`key:Space`). Alternatives are separated by " / ".
     */
    gesture: string;
    /** What it does. A key inside it is written `{key:mod+z}` (the shortcut
     *  table's syntax), so it shows as ⌘Z on macOS and Ctrl+Z elsewhere. */
    label: string;
    /** Where to do it: "on a pane border", "over an image". */
    where?: string;
    /** Only on these operating systems; all when absent. */
    os?: ("win32" | "darwin" | "linux")[];
    /** The code the tip describes: `anchor` must appear in `file` (tips.test.ts). */
    source: { file: string; anchor: string };
}

export const TIPS: TipRow[] = [
    // ── Panes ──
    {
        id: "pane:border:shiftDrag",
        area: "Panes",
        gesture: "shift+drag",
        where: "on a pane border",
        label: "Move only the border between the two panes beside it (a plain drag resizes the whole group)",
        source: { file: "frontend/layout/lib/TileLayout.win32.tsx", anchor: "!event.shiftKey" },
    },
    {
        id: "pane:header:dblclick",
        area: "Panes",
        gesture: "dblclick",
        where: "on a pane's header",
        label: "Maximize the pane, or restore it",
        source: { file: "frontend/app/block/blockframe.tsx", anchor: "onDblClick={() => props.nodeModel.toggleMagnify()}" },
    },
    {
        id: "pane:header:drag",
        area: "Panes",
        gesture: "drag",
        where: "a pane by its header",
        label: "Move the pane somewhere else in the layout",
        source: { file: "frontend/layout/lib/TileLayout.core.tsx", anchor: "Extra veto for canDrag" },
    },
    {
        id: "pane:numbers",
        area: "Panes",
        gesture: "hold:ctrl+shift",
        label: "Number every pane, for {key:ctrl+shift+1}–9",
        source: { file: "frontend/app/store/keymodel-dispatch.ts", anchor: "export function registerControlShiftTracking" },
    },
    {
        id: "pane:zoom:wheel",
        area: "Panes",
        gesture: "mod+wheel",
        where: "over a pane",
        label: "Zoom that pane; over the title bar, status bar or a pane's header, zoom the app's frame instead",
        source: { file: "frontend/app/app.tsx", anchor: "function isOverChrome(target: HTMLElement): boolean" },
    },
    {
        id: "pane:zoom:allPanes",
        area: "Panes",
        gesture: "mod+shift+wheel",
        where: "over a pane",
        label: "Zoom every pane in the window together (over a browser pane's page, only that page zooms)",
        source: { file: "frontend/app/app.tsx", anchor: "const AppAllPanesZoomHandler = () => {" },
    },
    {
        id: "pane:dropOnTab",
        area: "Panes",
        gesture: "drag",
        where: "a pane onto a window tab",
        label: "Hold over the tab to open it and place the pane, or drop on the tab to move the pane there",
        source: { file: "frontend/app/tab/droppable-tab.tsx", anchor: "onDrop: () => {" },
    },
    {
        id: "pane:tearOff",
        area: "Panes",
        gesture: "drag",
        where: "a pane or pane tab out of the window",
        label: "Open it in a floating window; drag that back over the main window to dock it",
        source: { file: "frontend/app/drag/pane-tab-tearoff.ts", anchor: "an in-window drop never tears off" },
    },

    // ── Window ──
    {
        id: "window:edge:shiftDrag",
        area: "Window",
        gesture: "shift+drag",
        where: "the window by its edge",
        label: "Give all the size change to the panes along that edge (a plain drag scales every pane)",
        os: ["win32"],
        source: { file: "crates/cef/src/client/wndproc.rs", anchor: "GetAsyncKeyState(VK_SHIFT" },
    },
    {
        id: "window:titlebar:dblclick",
        area: "Window",
        gesture: "dblclick",
        where: "on empty space in the title bar",
        label: "Maximize the window, or restore it",
        source: { file: "frontend/app/hook/useWindowDrag.win32.ts", anchor: '"dblclick",' },
    },

    // ── Tabs ──
    {
        id: "tab:rename:dblclick",
        area: "Tabs",
        gesture: "dblclick",
        where: "on a window tab's name",
        label: "Rename the tab (Enter saves, Esc cancels)",
        source: { file: "frontend/app/tab/tab.tsx", anchor: "onDblClick={() => handleRenameTab()}" },
    },
    {
        id: "tab:rightclick",
        area: "Tabs",
        gesture: "rightclick",
        where: "on a window tab",
        label: "Set the tab's colour, or rename it",
        source: { file: "frontend/app/tab/tab.tsx", anchor: "onContextMenu" },
    },
    {
        id: "tab:strip:wheel",
        area: "Tabs",
        gesture: "wheel",
        where: "over the tab strip",
        label: "Scroll the tabs sideways",
        source: { file: "frontend/app/tab/tab-reorder.ts", anchor: "mouse-wheel horizontal scroll" },
    },
    {
        id: "tab:dragOut",
        area: "Tabs",
        gesture: "drag",
        where: "a window tab below the strip",
        label: "Open the tab in a new window (Esc while dragging cancels)",
        source: { file: "frontend/app/tab/tab-reorder.ts", anchor: "Escape-to-abort" },
    },
    {
        id: "tab:middleclick",
        area: "Tabs",
        gesture: "middleclick",
        where: "on a pane, document or editor tab",
        label: "Close the tab",
        source: { file: "frontend/app/element/PaneTabStrip.tsx", anchor: "e.button === 1" },
    },
    {
        id: "tab:preview:dblclick",
        area: "Tabs",
        gesture: "dblclick",
        where: "on a preview tab (italic)",
        label: "Keep it open instead of letting the next preview replace it",
        source: { file: "frontend/app/doc-tabs/DocTabStrip.tsx", anchor: "(preview, double-click to keep)" },
    },

    // ── Terminal ──
    {
        id: "term:url:click",
        area: "Terminal",
        gesture: "click",
        where: "on a URL in a terminal",
        label: "Open it in the system browser",
        source: { file: "frontend/app/view/term/termwrap.ts", anchor: "new WebLinksAddon(" },
    },
    {
        id: "term:path:click",
        area: "Terminal",
        gesture: "click",
        where: "on a file path in a terminal",
        label: "Open it (a path with :line opens VS Code at that line), or show it in the file manager",
        source: { file: "frontend/app/view/term/filelinkprovider.ts", anchor: "provideLinks" },
    },
    {
        id: "term:drop",
        area: "Terminal",
        gesture: "drop",
        where: "files onto a terminal",
        label: "Copy them into the terminal's folder",
        source: { file: "frontend/app/view/term/term.tsx", anchor: "No working directory for this terminal" },
    },
    {
        id: "term:ctrlF",
        area: "Terminal",
        gesture: "ctrl+shift+key:F",
        label: "Find in a terminal ({key:ctrl+f} there goes to the shell)",
        os: ["win32", "linux"],
        source: { file: "frontend/app/keybindings/defaults.ts", anchor: 'other: ["ctrl+shift+f"], when: "terminalFocus"' },
    },

    // ── Editor ──
    {
        id: "editor:tree:click",
        area: "Editor",
        gesture: "click / dblclick",
        where: "on a file in the editor's file tree",
        label: "A click opens a preview tab, a double-click a tab that stays; F2 renames",
        source: { file: "frontend/app/view/editor/file-tree.tsx", anchor: "onDblClick={handleDblClick}" },
    },

    // ── Files ──
    {
        id: "files:select",
        area: "Files",
        gesture: "mod+click / shift+click",
        where: "on rows in the Files pane",
        label: "Add or remove one row / select a range",
        source: { file: "frontend/app/view/files/files-view.tsx", anchor: "e.shiftKey" },
    },
    {
        // Not on macOS: Spotlight takes ⌘Space and the input-source switch ⌃Space
        // by default, so the key never reaches the page (Masty's macOS check, #4631).
        id: "files:toggleFocused",
        area: "Files",
        gesture: "ctrl+key:Space",
        where: "in the Files list",
        label: "Add or remove the focused row without moving it",
        os: ["win32", "linux"],
        source: { file: "frontend/app/view/files/files-view.tsx", anchor: 'e.key === " " && isMod(e)' },
    },
    {
        id: "files:keys",
        area: "Files",
        gesture: "key:Space / key:/ / key:Backspace",
        where: "in the Files list",
        label: "Preview the file / filter / go back; typing a name jumps to it",
        source: { file: "frontend/app/view/files/files-view.tsx", anchor: "new TypeAhead()" },
    },
    {
        id: "files:folder:middleclick",
        area: "Files",
        gesture: "middleclick",
        where: "on a folder in the Files pane",
        label: "Open it in a new tab",
        source: { file: "frontend/app/view/files/files-view.tsx", anchor: "Middle-click a folder: open it in a new tab." },
    },
    {
        id: "files:path:dblclick",
        area: "Files",
        gesture: "dblclick",
        where: "on the Files pane's folder path",
        label: "Type a path",
        source: { file: "frontend/app/view/files/files-view.tsx", anchor: "startEditingPath" },
    },
    {
        id: "files:drag",
        area: "Files",
        gesture: "drag / drop",
        where: "Files rows onto a pane, or files onto the list",
        label: "Send files to an agent, editor, media or terminal pane; dropping copies here, or into the folder under the pointer",
        source: { file: "frontend/app/view/files/files-view.tsx", anchor: "registerFileDropTarget(model.blockId" },
    },

    // ── Media ──
    {
        id: "media:image",
        area: "Media",
        gesture: "wheel / drag / dblclick",
        where: "on an image in the Media pane",
        label: "Zoom around the pointer / pan / switch between fit and zoomed; + − 0 1 and the arrows too",
        source: { file: "frontend/app/view/media/media-view.tsx", anchor: "the wheel zooms about the cursor" },
    },

    // ── Agent ──
    {
        id: "agent:esc",
        area: "Agent",
        gesture: "key:Escape",
        where: "in the agent's message box",
        label: "Clear it ({key:mod+z} brings it back); in an empty box, interrupt the agent",
        source: { file: "frontend/app/view/agent/components/AgentFooter.tsx", anchor: "textarea is empty → send SIGINT" },
    },
    {
        id: "agent:up",
        area: "Agent",
        gesture: "key:↑",
        where: "in an empty message box",
        label: "Take back the last queued message; ↑ and ↓ step through what you sent",
        source: { file: "frontend/app/view/agent/components/AgentFooter.tsx", anchor: "ArrowUp when the composer is empty" },
    },
    {
        id: "agent:suggestion",
        area: "Agent",
        gesture: "key:Tab / key:→",
        where: "in an empty message box",
        label: "Accept the suggested next prompt",
        source: { file: "frontend/app/view/agent/components/AgentFooter.tsx", anchor: "Ghost-text next-prompt suggestion" },
    },
    {
        id: "agent:slash",
        area: "Agent",
        gesture: "key:/ / key:!",
        where: "at the start of a message",
        label: "/ opens command autocomplete; a message starting with ! runs as a shell command",
        source: { file: "frontend/app/view/agent/bang-command.ts", anchor: 'trimmed.startsWith("!")' },
    },
    {
        id: "agent:attach",
        area: "Agent",
        gesture: "drop",
        where: "files onto an agent pane, or paste them",
        label: "Attach them to your next message",
        source: { file: "frontend/app/view/agent/hooks/useAgentDropAttach.ts", anchor: "Drop file(s) onto an agent pane" },
    },
    {
        id: "agent:search",
        area: "Agent",
        gesture: "mod+key:F",
        where: "in an agent pane",
        label: "Search the conversation; Enter and {key:shift+Enter} step through matches",
        source: { file: "frontend/app/view/agent/hooks/useAgentKeyboard.ts", anchor: 'isModKey(e) && e.key === "f"' },
    },
    {
        id: "agent:permission",
        area: "Agent",
        gesture: "key:Enter / shift+key:Enter / key:O·S·P·G",
        where: "in a permission prompt",
        label: "Allow / deny / set the scope: once, session, project or global",
        source: { file: "frontend/app/view/agent/components/AgentDecisionPanel.tsx", anchor: "shiftHeld" },
    },
    {
        id: "agent:link:middleclick",
        area: "Agent",
        gesture: "middleclick",
        where: "on a link in an agent pane",
        label: "Open it in a browser pane (a plain click opens the system browser)",
        source: { file: "frontend/app/element/link-open.ts", anchor: "if (e.button !== 1) return;" },
    },

    // ── Find ──
    {
        id: "find:enter",
        area: "Find",
        gesture: "key:Enter / shift+key:Enter",
        where: "in a find bar",
        label: "Next / previous match",
        source: { file: "frontend/app/element/search.tsx", anchor: "shiftKey" },
    },

    // ── Browser ──
    {
        id: "browser:keys",
        area: "Browser",
        gesture: "mod+key:L / mod+key:R / alt+key:← / alt+key:→",
        where: "in a browser pane",
        label: "Address bar / reload / back / forward",
        source: { file: "crates/cef/src/client/handlers.rs", anchor: "Alt+Left — back" },
    },
];

const MOD_LABEL: Record<string, [mac: string, other: string]> = {
    mod: ["⌘", "Ctrl"],
    ctrl: ["⌃", "Ctrl"],
    shift: ["⇧", "Shift"],
    alt: ["⌥", "Alt"],
};

const POINTER_LABEL: Record<string, string> = {
    click: "click",
    dblclick: "double-click",
    middleclick: "middle-click",
    rightclick: "right-click",
    drag: "drag",
    wheel: "scroll",
    drop: "drop",
};

/** One gesture alternative ("shift+drag") as key-cap labels: ["Shift", "drag"], or ["⇧", "drag"] on macOS. */
function tokenLabels(alt: string, platform: KeyPlatform): string[] {
    const hold = alt.startsWith("hold:");
    const parts = (hold ? alt.slice(5) : alt).split("+").map((t) => {
        const mod = MOD_LABEL[t];
        if (mod) return platform === "mac" ? mod[0] : mod[1];
        if (t.startsWith("key:")) return t.slice(4);
        return POINTER_LABEL[t] ?? t;
    });
    return hold ? ["hold", ...parts] : parts;
}

/** A tip's gesture, per alternative, as key-cap labels for this platform. */
export function gestureLabels(tip: TipRow, platform: KeyPlatform): string[][] {
    return tip.gesture.split(" / ").map((alt) => tokenLabels(alt.trim(), platform));
}

/** A tip's label for this platform: each `{key:…}` shown as the Help pane shows keys. */
export function tipLabel(tip: TipRow, platform: KeyPlatform): string {
    return tip.label.replace(/\{key:([^}]+)\}/g, (_m, spec: string) => formatKey(spec, platform));
}

/** The tips that apply on `os`, in table order. */
export function tipsFor(os: NodeJS.Platform): TipRow[] {
    return TIPS.filter((t) => !t.os || t.os.includes(os as "win32" | "darwin" | "linux"));
}
