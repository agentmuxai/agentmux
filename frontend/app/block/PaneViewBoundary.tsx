// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The Suspense boundary of a pane's view, which leaves the view where it is while a read in it is pending.
 *
 * A plain `<Suspense>` swaps its children for its fallback: the children stay alive but leave the document.
 * Around a pane's view that took the whole view out for every pending `createResource` read anywhere in it
 * (a refetch, a part of the view that mounts later and reads one): the pane's content had no size for that
 * time, every scroller in it lost its place, and the pane flashed (the Kanban drawer #47462, the transcript's
 * scroller #47406).
 *
 * Here the view is put into a box of its own (`host`) that is not part of what the Suspense returns, so the
 * swap takes nothing of the view with it. The view is still created under the boundary, so the boundary
 * counts its pending reads as before, and `fallback` is shown over the view for as long as one is pending.
 * The host has no box (`display: contents`, block.scss): the view is laid out as the content box's own child.
 *
 * What stays as it was: while a read is pending the boundary holds the effects of the view (Solid resumes
 * them when it resolves). The view is on screen now for that time, so a long pending read shows a view that
 * is drawn but not reacting through its effects; `fallback` is there to say so.
 */

import { createMemo, onCleanup, Suspense, type JSX } from "solid-js";
import { insert } from "solid-js/web";

export function PaneViewBoundary(props: { fallback: JSX.Element; children: JSX.Element }): JSX.Element {
    const host = (<div class="pane-view-host" />) as HTMLDivElement;
    return (
        <>
            {host}
            <Suspense fallback={<div class="pane-view-pending">{props.fallback}</div>}>
                <ViewIn host={host}>{props.children}</ViewIn>
            </Suspense>
        </>
    );
}

/** Creates the view under the boundary and puts it into `host`: it gives the boundary nothing to swap. */
function ViewIn(props: { host: HTMLElement; children: JSX.Element }): JSX.Element {
    const view = createMemo(() => props.children);
    insert(props.host, view);
    onCleanup(() => props.host.replaceChildren());
    return undefined;
}
