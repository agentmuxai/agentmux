// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/*
 * Preview of one attachment over the whole tab, with ←/→ to page, Esc to close, and
 * Remove in the composer. Images show the send-copy (≤ 2000 px), which is
 * both what the agent sees and small enough to load instantly, and state the
 * original's and the send-copy's size. Spec §5.1. SVGs show the file itself,
 * text files a scrolling view of their first lines, and everything else the
 * type icon with the details the agent is given
 * (SPEC_AGENT_PANE_FILE_ATTACHMENTS_2026_09_26.md §5).
 */

import { Modal } from "@/app/element/modal";
import { LightboxBar, LightboxBody } from "./attachment-lightbox-parts";
import { createLightbox, type LightboxItem, type LightboxProps } from "./attachment-lightbox-state";

export type { LightboxItem };

export function AttachmentLightbox(props: LightboxProps) {
    const s = createLightbox(props);
    return (
        // A tab modal: it covers the whole tab the attachment is in, not just its pane (Escape and a click
        // outside close it; the tab bar and other tabs stay live). The arrows are this dialog's own keys.
        <Modal
            open={true}
            scope="tab"
            size="fit"
            panelClass="agent-attachment-lightbox__panel agent-attachment-lightbox__panel--fit"
            onClose={props.onClose}
            ariaLabel={`${s.kind() === "image" ? "Image" : "File"} ${s.item()?.number}: ${s.item()?.name}`}
            initialFocus={() => s.dialog() ?? null}
        >
            <div
                ref={s.setDialog}
                class="agent-attachment-lightbox__dialog agent-attachment-lightbox__dialog--fit"
                tabIndex={-1}
                onKeyDown={s.onKeyDown}
            >
                <div class="agent-attachment-lightbox__image agent-attachment-lightbox__image--fit" classList={{ "is-actual": s.actual() }}>
                    <LightboxBody s={s} />
                </div>
                <LightboxBar s={s} p={props} />
            </div>
        </Modal>
    );
}
