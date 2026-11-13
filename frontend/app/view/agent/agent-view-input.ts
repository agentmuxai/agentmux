// Copyright 2024, Command Line Inc.
// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * What an agent pane takes in from the person: scroll jumps, in-session search and its keys, zoom,
 * file drops and the pane's width. Split out of AgentPresentationView; the hooks are called in the
 * order they were there.
 */

import { snapshot as layoutSnapshot } from "@/app/store/agent-pane-layout-store";
import { paneZoom } from "@/app/store/pane-zoom-local";
import { createMemo, createSignal, onCleanup } from "solid-js";
import type { AgentViewModel } from "./agent-model";
import type { PaneFoundation } from "./agent-view-foundation";
import type { PaneLate } from "./agent-view-late";
import type { useShownDocument } from "./agent-view-transcript";
import { useAgentDropAttach } from "./hooks/useAgentDropAttach";
import { useAgentKeyboard } from "./hooks/useAgentKeyboard";
import { useInSessionSearch } from "./hooks/useInSessionSearch";
import { useJektsAwaitingYou } from "./hooks/useJektsAwaitingYou";
import { usePtyWidth } from "./hooks/usePtyWidth";
import { useScrollToNode } from "./hooks/useScrollToNode";
import { messagesControlsFor, showInPanelIfHeld } from "./messages/messages-state";
import { showMessage } from "./messages/show-message";
import { createSearchReveal } from "./search-reveal";
import { focusSearchField, registerSearchControls, restoreFocusToMagnifier } from "./search/search-bridge";
import { topVisibleNodeId } from "./search/top-row";
import { rawIdAtOrAfter, topRowId } from "./search/transcript-index";

/** How long "Show message" marks the row it scrolled to. */
const MARK_MS = 2400;

export function usePaneInput(p: { found: PaneFoundation; model: AgentViewModel; shown: ReturnType<typeof useShownDocument>; late: PaneLate }) {
    const { found, model, late } = p;
    const { paneModel, block } = found;
    const { viewControls } = p.shown;

    // Jump command as a signal: AgentDocumentView reacts and scrolls inside its own container (hooks/useScrollToNode.ts).
    const scroll = useScrollToNode();

    // A match inside a run the pane's view hides opens that run until the search ends (search-reveal.ts).
    const reveal = createSearchReveal({
        shown: p.shown.shownDocument,
        setPins: (update) => found.agentAtoms().documentStateAtom[1]((prev) => ({ ...prev, pinnedNodes: update(prev.pinnedNodes) })),
        jumpTo: scroll.jumpTo,
    });

    // "Show message" (the card, the panel's strip and rows): to the panel when the conversation has no row for the message,
    // else to its row, marked for a moment so a row already on screen still answers (messages/show-message.ts).
    const [marked, setMarked] = createSignal<string | null>(null);
    let markTimer: ReturnType<typeof setTimeout> | undefined;
    const mark = (nodeId: string): void => {
        clearTimeout(markTimer);
        setMarked(nodeId);
        markTimer = setTimeout(() => setMarked(null), MARK_MS);
    };
    onCleanup(() => clearTimeout(markTimer));
    const hasRow = (nodeId: string): boolean =>
        p.shown.shownDocument().some((n) => n.id === nodeId || (n.type === "hidden_run" && n.hiddenIds.includes(nodeId)));
    const show = (nodeId: string): void =>
        void showMessage(nodeId, {
            panel: messagesControlsFor(model.blockId),
            nodes: paneModel.document,
            hasRow,
            jump: reveal.jumpTo,
            mark,
        });

    // A sensitive jekt that waits on the person: a card above the composer and a wait for the pane (jekts-awaiting-you.ts).
    useJektsAwaitingYou({ blockId: model.blockId, getDocument: p.shown.getDocument, jumpTo: show });

    // In-session search over the loaded document slice: matches, navigation, highlight. A new query starts at the
    // first match from the row at the top of the screen; a query with no match closes what a jump opened.
    const search = useInSessionSearch({
        document: paneModel.document,
        // A match inside a message only the Messages panel holds opens it there: the conversation has no row for it.
        jumpTo: (id) => void (showInPanelIfHeld(model.blockId, paneModel.document(), id) || reveal.jumpTo(id)),
        onClose: () => {
            reveal.release();
            restoreFocusToMagnifier(model.blockId);
        },
        onNoMatch: reveal.release,
        topNodeId: () => {
            // The drawn rows answer for the streaming tail too, which the layout slice has no rows for.
            const at = layoutSnapshot(model.blockId);
            const top = topVisibleNodeId(late.root) ?? (at ? topRowId(found.layoutView()?.rows ?? [], at.scrollTop) : null);
            // The rows on screen include hidden-run lines and dividers that the document holds under no id of their own.
            return rawIdAtOrAfter(p.shown.shownDocument(), top, new Set(paneModel.document().map((n) => n.id)));
        },
    });

    // One way in: Ctrl+F, the header magnifier and the palette open the bar; with it open, they put the caret in its
    // field with the text selected. Esc, the close button and the magnifier close it (search/search-bridge.ts).
    const openSearch = () => (search.visible() ? focusSearchField(model.blockId) : search.setVisible(true));
    onCleanup(
        registerSearchControls(model.blockId, {
            visible: search.visible,
            open: openSearch,
            toggle: () => (search.visible() ? search.close() : search.setVisible(true)),
        })
    );

    // Pane-scoped Ctrl+F and F3 (hooks/useAgentKeyboard.ts).
    useAgentKeyboard({
        blockId: model.blockId,
        onToggleSearch: openSearch,
        searchOpen: search.visible,
        onStepSearch: (backward) => (backward ? search.prev() : search.next()),
        onCycleView: viewControls.cycle,
        onFollowDefaultView: viewControls.followDefault,
    });

    // Per-pane zoom: `term:zoom` in block meta, the key terminal panes use; the level just asked for while its
    // write is on its way, else the stored one. The universal zoom framework owns the input (keymodel.ts,
    // app.tsx) and the persistence, so this file has no zoom handlers of its own.
    const zoomFactor = createMemo(() => paneZoom(model.blockId, block()?.meta));

    // File drops: this pane's hook for the window-level controller (tray, or copy + `@filename`).
    useAgentDropAttach({
        blockId: model.blockId,
        rootRef: () => late.root,
    });

    // Pane width -> PTY cols, so tools in the agent CLI wrap to the pane instead of the opening `cols: 80`
    // (hooks/usePtyWidth.ts). Lines already captured keep their wrap.
    usePtyWidth({
        blockId: model.blockId,
        elementRef: () => late.root,
        log: found.shell.log,
    });

    // The row the transcript marks: the search's current match, else the message "Show message" just took the person to.
    const highlightId = createMemo(() => search.highlightId() ?? marked());

    return { scroll, search, zoomFactor, show, highlightId };
}
