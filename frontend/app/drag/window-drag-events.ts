// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * One set of window drag listeners, however many window tabs are mounted.
 * docs/specs/SPEC_DRAG_AND_DROP_CONSOLIDATION_2026_09_27.md §5.2.
 *
 * Subscribers name the drag kinds they act on, and a `dragover` or `drop`
 * reaches only those for the current drag: "files" for an OS file drag,
 * otherwise the drag session's kind.
 *
 * Bubble phase on `window`, like the listeners it replaces, so every
 * element's own handler (pragmatic's drop targets, the drone canvas) has had
 * its say first. The file-drop controller's capture listeners stay separate:
 * they decide file drops before anything else sees them.
 *
 * It also keeps an unhandled OS file drop from navigating the whole window
 * away. Chromium's default for one is to load the dropped file in the
 * top-level frame, which destroys the app (its window controls are part of
 * this page) with no way back short of killing the process
 * (docs/retro/retro-md-drop-window-hijack-and-55-6-relaunch-failure-2026-08-16.md).
 * The file-drop controller already prevents the default for every file drag
 * it sees; this is the backstop for anything it doesn't claim, e.g. the tab
 * strip, the title bar, or a pane without a hook. Only file drags: dropping
 * selected text into the agent composer relies on the browser's default
 * (SPEC_PANE_FILE_DROP_2026_05_30.md §7).
 */

import { isFileDrag } from "@/util/dnd";
import { session, type DragKind } from "./drag-session";

export interface WindowDragSubscriber {
    kinds: readonly DragKind[];
    over?: (e: DragEvent) => void;
    drop?: (e: DragEvent) => void;
}

const subscribers = new Set<WindowDragSubscriber>();

/** What this event drags. File drags first: the file-drop controller keeps its own session. */
function kindOf(e: DragEvent): DragKind | null {
    if (isFileDrag(e)) return "files";
    return session()?.kind ?? null;
}

function dispatch(e: DragEvent, handler: (s: WindowDragSubscriber) => ((e: DragEvent) => void) | undefined): void {
    const kind = kindOf(e);
    if (kind == null) return;
    for (const s of subscribers) {
        if (s.kinds.includes(kind)) handler(s)?.(e);
    }
    if (kind === "files") e.preventDefault();
}

const onDragOver = (e: DragEvent) => dispatch(e, (s) => s.over);
const onDrop = (e: DragEvent) => dispatch(e, (s) => s.drop);

let installedOn: Window | null = null;

/** Install the listeners on this window (once per renderer). */
export function installWindowDragEvents(win: Window = window): () => void {
    if (installedOn === win) return () => {};
    installedOn = win;
    win.addEventListener("dragover", onDragOver);
    win.addEventListener("drop", onDrop);
    return () => {
        win.removeEventListener("dragover", onDragOver);
        win.removeEventListener("drop", onDrop);
        installedOn = null;
    };
}

/** Subscribe for the given drag kinds; returns the disposer. */
export function onWindowDrag(subscriber: WindowDragSubscriber): () => void {
    if (typeof window !== "undefined") installWindowDragEvents();
    subscribers.add(subscriber);
    return () => subscribers.delete(subscriber);
}
