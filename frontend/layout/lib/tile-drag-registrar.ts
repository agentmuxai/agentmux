// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Keeps a tile's whole-pane drag bound to the header that is actually in the
 * DOM, rebinding when the DOM changes rather than on a 100 ms poll.
 * docs/specs/SPEC_DRAG_AND_DROP_CONSOLIDATION_2026_09_27.md §5.7.
 *
 * `find` must query the live DOM: BlockFrame_Header renders twice (the live
 * header and an ErrorBoundary fallback that is never inserted), so a ref would
 * end up on the detached one.
 */

export interface TileDragRegistrarOptions {
    root: HTMLElement;
    find: () => HTMLElement | null;
    /** Register the drag on `handle`; returns its cleanup. */
    bind: (handle: HTMLElement) => () => void;
    /** A drag is in progress: tearing the registration down now would drop
     *  pragmatic-dnd's onDrop, and `activeDrag` would never reset. */
    dragging: () => boolean;
}

export interface TileDragRegistrar {
    register(): void;
    /** Call when a drag ends: runs the rebind a mid-drag change skipped. */
    dragEnded(): void;
    dispose(): void;
}

export function createTileDragRegistrar(opts: TileDragRegistrarOptions): TileDragRegistrar {
    let handle: HTMLElement | null = null;
    let unbind: (() => void) | null = null;
    let pending = false;

    const register = () => {
        const next = opts.find();
        if (next === handle) {
            pending = false;
            return;
        }
        if (opts.dragging()) {
            pending = true;
            return;
        }
        pending = false;
        unbind?.();
        unbind = null;
        handle = next;
        if (next) unbind = opts.bind(next);
    };

    const observer = new MutationObserver(register);
    observer.observe(opts.root, { childList: true, subtree: true });

    return {
        register,
        dragEnded() {
            if (pending) register();
        },
        dispose() {
            observer.disconnect();
            unbind?.();
            unbind = null;
            handle = null;
        },
    };
}
