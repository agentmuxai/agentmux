// Copyright 2024, Command Line Inc.
// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * What an agent pane takes in from the person: scroll jumps, in-session search and its keys, zoom,
 * file drops and the pane's width. Split out of AgentPresentationView; the hooks are called in the
 * order they were there.
 */

import { paneZoom } from "@/app/store/pane-zoom-local";
import { createMemo } from "solid-js";
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
import { createSearchReveal } from "./search-reveal";

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

    // A sensitive jekt that waits on the person: a card above the composer and a wait for the pane (jekts-awaiting-you.ts).
    useJektsAwaitingYou({ blockId: model.blockId, getDocument: p.shown.getDocument, jumpTo: scroll.jumpTo });

    // In-session search over the loaded document slice: matches, navigation, highlight.
    const search = useInSessionSearch({
        document: paneModel.document,
        jumpTo: reveal.jumpTo,
        onClose: reveal.release,
    });

    // Pane-scoped Ctrl+F: a second press closes the bar and clears the state (hooks/useAgentKeyboard.ts).
    useAgentKeyboard({
        blockId: model.blockId,
        onToggleSearch: () => (search.visible() ? search.close() : search.setVisible(true)),
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

    return { scroll, search, zoomFactor };
}
