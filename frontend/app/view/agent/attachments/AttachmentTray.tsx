// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/*
 * The composer's attachment tray, above the text box. Spec §5.1–§5.4, §5.9.
 *
 * Collapsed: one row of 64 px tiles; when they don't fit, the last slot is a
 * "+N" tile. Expanded: a scrolling grid with names and sizes. A summary
 * ("30 images · 214 MB / 1 GB", or "attachments" once any isn't an image)
 * sits in the header, with a progress bar and
 * Cancel while anything is processing. Removing shows an Undo line for a few
 * seconds, paused while hovered or focused.
 */

import clsx from "clsx";
import { Show } from "solid-js";
import { createAttachmentTray, type AttachmentTrayProps } from "./attachment-tray-state";
import { TileList, TrayHeader, TrayLightbox, UndoLine } from "./attachment-tray-parts";

export function AttachmentTray(props: AttachmentTrayProps) {
    const t = createAttachmentTray(props);
    const { d, expanded, undo, undoState, announcement } = t;
    return (
        <>
            <Show when={d.count() > 0 || undo()}>
                <div
                    class={clsx("agent-attachment-tray", { "is-expanded": expanded() })}
                    onMouseEnter={undoState.pause}
                    onMouseLeave={undoState.resume}
                    onFocusIn={undoState.pause}
                    onFocusOut={undoState.resume}
                >
                    <Show when={d.count() > 0}>
                        <TrayHeader t={t} />
                    </Show>
                    <TileList t={t} />
                    <UndoLine t={t} />
                </div>
            </Show>
            <span class="sr-only" aria-live="polite">
                {announcement()}
            </span>
            <TrayLightbox t={t} />
        </>
    );
}
