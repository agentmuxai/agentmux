// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Where the caret goes when a modal opens and closes.
 *
 * A window's or a tab's modal takes it as it opens and gives it back to the element that had it. A pane's
 * modal does the same while its pane is the selected one. One that opens while the person is in another pane
 * (a question from srv, an agent or a page) leaves the caret where it is: it waits until they select its pane
 * (a click in it, "Go to pane"), and if it closes before that it gives nothing back, since it took nothing.
 * Until then a press on its backdrop is the person going to the pane, and closes nothing.
 */

import { createEffect, createSignal } from "solid-js";

import { focusManager } from "@/app/store/focusManager";
import { initialFocusTarget, paneToWaitFor } from "./modal-focus-trap";

export type ModalInitialFocus = HTMLElement | (() => HTMLElement | null) | undefined;

export interface ModalCaret {
    /** Call before the region is locked: locking blurs what has the caret. */
    remember: () => void;
    /** The modal is mounted in `region` with the resolved `scope`. */
    opened: (scope: string, region: HTMLElement) => void;
    closed: () => void;
    /** The modal is on screen and has not taken the caret: its pane is not the selected one yet. */
    waiting: () => boolean;
}

/** Whether the element that had the caret can have it back: still in the page, and neither the page itself nor a
 *  control disabled since (a form's button that turned into "Waiting for you" before the modal opened). */
function canTakeCaretBack(el: HTMLElement | null): el is HTMLElement {
    return !!el && el.isConnected && el !== el.ownerDocument.body && typeof el.focus === "function" && !el.matches(":disabled");
}

/**
 * A backdrop's handlers. A press on the backdrop of a modal still waiting for its pane (`waiting`) is the person
 * going to that pane: the press selects it, and the click it ends in closes nothing, so a question they have not
 * read is not answered "no". Every other click goes to `dismiss`.
 */
export function createBackdropDismiss(
    waiting: () => boolean,
    dismiss: (ev: MouseEvent) => void
): { press: () => void; click: (ev: MouseEvent) => void } {
    let selectsPane = false;
    return {
        press: () => {
            selectsPane = waiting();
        },
        click: (ev) => {
            if (!selectsPane) return dismiss(ev);
            selectsPane = false;
        },
    };
}

/** Call in the modal's component body. `panel`: the modal's panel once rendered; `initialFocus`: the caller's choice. */
export function createModalCaret(panel: () => HTMLElement | undefined, initialFocus: () => ModalInitialFocus): ModalCaret {
    let previousFocus: HTMLElement | null = null;
    let tookFocus = false;
    const [awaitedPane, setAwaitedPane] = createSignal<string | null>(null);

    // Next frame, so the Portal has rendered. `latePane`: the modal was already on screen when its pane was
    // selected. A control of it the person clicked keeps the caret, and if they went on to another pane before
    // this frame the modal waits for them again.
    const take = (latePane?: string): void => {
        tookFocus = true;
        requestAnimationFrame(() => {
            const el = panel();
            if (!el || !el.isConnected) return;
            if (latePane != null) {
                if (focusManager.blockFocusAtom() !== latePane) {
                    tookFocus = false;
                    return void setAwaitedPane(latePane);
                }
                if (el.contains(document.activeElement)) return;
            }
            const chosen = initialFocus();
            const target = (typeof chosen === "function" ? chosen() : chosen) ?? initialFocusTarget(el);
            (target ?? el).focus();
        });
    };

    createEffect(() => {
        const pane = awaitedPane();
        if (pane == null || focusManager.blockFocusAtom() !== pane) return;
        setAwaitedPane(null);
        take(pane);
    });

    return {
        remember: () => {
            previousFocus = (document.activeElement as HTMLElement) ?? null;
        },
        opened: (scope, region) => {
            const pane = scope === "pane" ? paneToWaitFor(region, focusManager.blockFocusAtom) : null;
            if (pane == null) return take();
            tookFocus = false;
            previousFocus = null;
            setAwaitedPane(pane);
        },
        closed: () => {
            setAwaitedPane(null);
            if (!tookFocus) return;
            // On the next tick, so Solid's cleanup has finished and the element isn't moved by an unrelated update.
            queueMicrotask(() => {
                if (canTakeCaretBack(previousFocus)) {
                    previousFocus.focus();
                } else {
                    // The opener is gone (the modal confirmed closing the pane it was opened from) or cannot take
                    // the caret, or the modal took the caret late: it goes to the selection, not <body>.
                    // SPEC_FOCUS_FOLLOWS_SELECTION_2026_10_08.md R1.
                    focusManager.ensureSelectionFocused("modal-closed");
                }
                previousFocus = null;
            });
        },
        waiting: () => awaitedPane() != null,
    };
}
