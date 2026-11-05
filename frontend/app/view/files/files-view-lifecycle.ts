// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * What the Files pane does when it appears and when the folder or the request changes: the first
 * listing, the list's size, the refresh on being shown, the breadcrumb's scroll, and the selection
 * an agent asked for. Split out of FilesView; the hooks are called in the order they were there.
 */

import { createEffect, on, onCleanup, onMount } from "solid-js";
import { trackAgentFocus } from "./files-agent-mention";
import type { FilesLocal } from "./files-view-state";

export function useFilesLifecycle(fv: FilesLocal): void {
    const { model, ctx, refs, setViewHeight, setViewWidth, setScrollTop } = fv;

    trackAgentFocus();
    model.focusList = () => refs.list?.focus();
    onCleanup(() => (model.focusList = null));

    onMount(() => {
        void model.start();
        const listEl = refs.list;
        if (listEl) {
            const ro = new ResizeObserver(() => {
                setViewHeight(listEl.clientHeight ?? 400);
                setViewWidth(listEl.clientWidth ?? 600);
            });
            ro.observe(listEl);
            onCleanup(() => ro.disconnect());
        }
    });

    createEffect(
        on(
            () => ctx.visibility(),
            (v) => {
                if (v === "active") model.onShown();
            }
        )
    );

    // A long path shows its end: the folder you're in matters more than the
    // drive it's on.
    createEffect(
        on(model.path, () => {
            queueMicrotask(() => {
                if (refs.crumbs) refs.crumbs.scrollLeft = refs.crumbs.scrollWidth;
            });
        })
    );

    // OpenFiles' selection request, whenever it lands in the block's meta:
    // pane.open writes the meta in more than one step, so it can arrive
    // after the first listing (or come back after the pane cleared it).
    createEffect(
        on(
            () => ctx.meta()?.["files:select"],
            (req) => {
                if (req != null) model.applyRequestedSelection();
            }
        )
    );

    // A new folder starts at the top.
    createEffect(
        on(model.path, () => {
            if (refs.list) refs.list.scrollTop = 0;
            setScrollTop(0);
        })
    );
}
