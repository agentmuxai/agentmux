// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Moving a document tab from one pane to another of the same type (an
 * Editor's file to another Editor, a Media file to another Media pane).
 *
 * Each document pane registers a `DocTabHost` for its block: what it can say
 * about a tab, and how it gives one up or takes one in. A drop of a document
 * tab on another pane goes through `dropDocTab`, the one path for every type:
 * both sides are asked first, and only then does the tab leave one pane and
 * enter the other. `controllerHost` is the host for any pane on a
 * `DocTabsController` (Media); the Editor has its own (editor-doc-tabs.ts).
 * docs/reports/REPORT_DOC_TAB_DRAG_AND_DROP_2026_10_09.md §3.3.
 */

import { dropTargetForElements } from "@atlaskit/pragmatic-drag-and-drop/element/adapter";
import { asDocTabDragData } from "@/app/drag/drag-types";
import { notifyDrop } from "@/app/drag/file-drop-actions";
import type { DocDropAt, DocTab } from "./doc-tabs";
import type { DocTabsController } from "./doc-tabs-controller";
import "./doc-tabs.scss";

/** A tab on its way between two panes: the tab itself, and whatever state
 *  of its type lives outside the tab (the Editor's unsaved text). */
export interface DocTransfer {
    docType: string;
    tab: DocTab<unknown>;
    live?: unknown;
}

export interface DocTabHost {
    /** The pane type: tabs move only between hosts of one type. */
    readonly docType: string;
    /** Tab `tabId` as it is now, or null if there is no such tab. */
    peek(tabId: string): DocTab<unknown> | null;
    /** Why `tab` can't leave this pane, or null. */
    refuseGive(tab: DocTab<unknown>): string | null;
    /** Why `tab` can't come into this pane, or null. */
    refuseTake(tab: DocTab<unknown>): string | null;
    /** Take the tab out of this pane, with its live state. */
    give(tabId: string): DocTransfer | null;
    /** Put a given tab in, beside `at` or after the active tab. */
    take(transfer: DocTransfer, at?: DocDropAt): void;
}

const hosts = new Map<string, DocTabHost>();

/** Register block `blockId`'s host. The returned function unregisters it, only
 *  if it is still the one registered: a keep-alive block can be mounted twice
 *  for a moment during a move, and the first mount's cleanup must not remove
 *  the second's. */
export function registerDocTabHost(blockId: string, host: DocTabHost): () => void {
    hosts.set(blockId, host);
    return () => {
        if (hosts.get(blockId) === host) hosts.delete(blockId);
    };
}

export function docTabHost(blockId: string): DocTabHost | undefined {
    return hosts.get(blockId);
}

export type MoveResult = { moved: true } | { moved: false; reason?: string };

/** Why a tab can't move to a pane that has its document open already. Moved
 *  in anyway, it would merge into that tab and vanish from where it was. */
export const ALREADY_OPEN_THERE = "It is already open there.";

/**
 * Move tab `tabId` from block `sourceBlockId` to block `targetBlockId`, beside
 * `at` or after the target's active tab. Nothing changes unless both panes
 * agree; a refusal comes back with the reason.
 */
export function moveDocTab(sourceBlockId: string, tabId: string, targetBlockId: string, at?: DocDropAt): MoveResult {
    const from = hosts.get(sourceBlockId);
    const to = hosts.get(targetBlockId);
    if (sourceBlockId === targetBlockId || !from || !to || from.docType !== to.docType) return { moved: false };
    const tab = from.peek(tabId);
    if (!tab) return { moved: false };
    const reason = from.refuseGive(tab) ?? to.refuseTake(tab);
    if (reason) return { moved: false, reason };
    const transfer = from.give(tabId);
    if (!transfer) return { moved: false };
    to.take(transfer, at);
    return { moved: true };
}

/** `moveDocTab` for a drop the user made: a refusal is shown, not swallowed. */
export function dropDocTab(sourceBlockId: string, tabId: string, targetBlockId: string, at?: DocDropAt): boolean {
    const title = hosts.get(sourceBlockId)?.peek(tabId)?.title ?? "the tab";
    const result = moveDocTab(sourceBlockId, tabId, targetBlockId, at);
    if (result.moved === false && result.reason) notifyDrop.cantMove(title, result.reason);
    return result.moved;
}

/**
 * The host of a pane whose tabs are a `DocTabsController`'s. Everything a tab
 * is travels in the tab, so there is no live state. A pane with the tab's
 * document open already refuses it. `refuseGive` adds the type's own rule.
 * `isPlaceholder` names a tab that shows nothing yet (Media's "Click to load
 * media"): when one is in front, an arriving tab with a document takes its
 * place rather than joining it; another placeholder joins it.
 */
export function controllerHost<P>(
    ctl: DocTabsController<P>,
    docType: string,
    opts: { refuseGive?: (tab: DocTab<P>) => string | null; isPlaceholder?: (tab: DocTab<P>) => boolean } = {}
): DocTabHost {
    return {
        docType,
        peek: (tabId) => ctl.tabs().find((t) => t.id === tabId) ?? null,
        refuseGive: (tab) => opts.refuseGive?.(tab as DocTab<P>) ?? null,
        refuseTake: (tab) => (ctl.tabs().some((t) => t.key === tab.key) ? ALREADY_OPEN_THERE : null),
        give: (tabId) => {
            const tab = ctl.detach(tabId);
            return tab ? { docType, tab } : null;
        },
        take: (transfer, at) => {
            const tab = transfer.tab as DocTab<P>;
            const front = ctl.active();
            const placeholder = front && opts.isPlaceholder?.(front) && !opts.isPlaceholder(tab) ? front : undefined;
            ctl.attach(tab, at ?? (placeholder ? { targetId: placeholder.id, position: "after" } : undefined));
            // Gone, not closed: nothing to reopen. Only once the tab is in.
            if (placeholder && ctl.tabs().some((t) => t.id === tab.id)) ctl.detach(placeholder.id);
        },
    };
}

/**
 * Make `el`, a document pane's root, take a document tab of its type from
 * another pane, dropped anywhere on it; it joins after the active tab. A drop
 * on a tab of the pane's strip is that tab's instead (it says where). Other
 * drags are declined and go on to the targets around the pane (a pane tab
 * joins the pane stack). While one hovers, the pane's tab strip shows the
 * drop look. Returns the cleanup.
 */
export function registerDocTabDropZone(el: HTMLElement, docType: string, blockId: string): () => void {
    const setHover = (on: boolean) => el.classList.toggle("doc-tab-host--drop-hover", on);
    const cleanup = dropTargetForElements({
        element: el,
        canDrop: ({ source }) => {
            const data = asDocTabDragData(source.data);
            return !!data && data.docType === docType && data.sourceBlockId !== blockId;
        },
        onDragEnter: () => setHover(true),
        onDragLeave: () => setHover(false),
        onDrop: ({ source, location }) => {
            setHover(false);
            // pragmatic-dnd tells every accepting target under the pointer,
            // outermost included. A drop on one of the strip's tabs is that
            // tab's (it lands beside it); this zone acts only when it is the
            // innermost target, or the move would be tried twice.
            if (location.current.dropTargets[0]?.element !== el) return;
            const data = asDocTabDragData(source.data);
            if (!data) return;
            // On the next task: the move unmounts the dragged pill, which is
            // the drag's live source, while pragmatic-dnd is still
            // dispatching this drop.
            setTimeout(() => void dropDocTab(data.sourceBlockId, data.tabId, blockId), 0);
        },
    });
    return () => {
        setHover(false);
        cleanup();
    };
}
