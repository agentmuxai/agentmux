// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/*
 * Full-pane preview of one image of an agent's message, with ←/→ to step through the message's other
 * images, Esc to close and Open in editor for the one shown. The shell and styles are the attachment
 * preview's (attachments/AttachmentLightbox.tsx); the images are local files, not attachment ids
 * (SPEC_AGENT_PANE_IMAGE_ARROWS_2026_11_06.md).
 */

import { Modal } from "@/app/element/modal";
import { tip } from "@/app/element/tip";
import { Button, IconButton } from "@/app/element/ui";
import { formatBytes } from "@/util/format-bytes";
import { Show, type JSX } from "solid-js";
import { createMessageLightbox, type MessageImageLightboxProps, type MessageLightboxState } from "./message-image-lightbox-state";

const BLOCK = "agent-attachment-lightbox";

function Picture(props: { s: MessageLightboxState }): JSX.Element {
    const s = props.s;
    return (
        <Show
            when={s.entry()?.kind === "ready" ? (s.entry() as { url: string }).url : undefined}
            fallback={
                <span class={`${BLOCK}__missing`}>
                    {s.entry()?.kind === "missing" || s.entry()?.kind === "too-large"
                        ? "This image is no longer available."
                        : "Loading…"}
                </span>
            }
        >
            {(url) => (
                <img
                    src={url()}
                    alt={s.item()?.alt || s.item()?.name || ""}
                    onLoad={(e) => s.setSize(s.item()!.path, e.currentTarget.naturalWidth, e.currentTarget.naturalHeight)}
                />
            )}
        </Show>
    );
}

function Meta(props: { s: MessageLightboxState }): JSX.Element {
    const e = () => props.s.entry();
    const bytes = () => (e()?.kind === "ready" ? formatBytes((e() as { bytes: number }).bytes) : undefined);
    return (
        <Show when={bytes()}>
            <span class={`${BLOCK}__meta`}>{[props.s.size(), bytes()].filter(Boolean).join(" · ")}</span>
        </Show>
    );
}

function Actions(props: { s: MessageLightboxState; onClose: () => void }): JSX.Element {
    const s = props.s;
    return (
        <span class={`${BLOCK}__actions`}>
            <IconButton icon="chevron-left" label="Previous image" tooltip={false} title="Previous (←)" onClick={() => s.go(s.index() - 1)} />
            <span class="pos">
                {Math.min(s.index(), s.count() - 1) + 1} / {s.count()}
            </span>
            <IconButton icon="chevron-right" label="Next image" tooltip={false} title="Next (→)" onClick={() => s.go(s.index() + 1)} />
            <Button title="Open this image in the editor (E)" onClick={s.openEditor}>
                Open in editor
            </Button>
            <IconButton icon="xmark" label="Close preview" tooltip={false} title="Close (Esc)" onClick={props.onClose} />
        </span>
    );
}

export function MessageImageLightbox(props: MessageImageLightboxProps): JSX.Element {
    const s = createMessageLightbox(props);
    return (
        // A pane modal: it covers the pane the message is in (Escape and a click outside close it), not the
        // window. The arrows are this dialog's own keys.
        <Modal
            open={true}
            scope="pane"
            size="fit"
            panelClass={`${BLOCK}__panel ${BLOCK}__panel--message`}
            onClose={props.onClose}
            ariaLabel={s.label()}
            initialFocus={() => s.dialog() ?? null}
        >
            <div ref={s.setDialog} class={`${BLOCK}__dialog ${BLOCK}__dialog--message`} tabIndex={-1} onKeyDown={s.onKeyDown}>
                <div class={`${BLOCK}__image ${BLOCK}__image--message`}>
                    <Picture s={s} />
                </div>
                <div class={`${BLOCK}__bar`}>
                    <span class={`${BLOCK}__title`} {...tip(s.item()?.name)}>
                        {s.item()?.name}
                        <Show when={s.item()?.alt && s.item()?.alt !== s.item()?.name}>
                            <span class="num"> {s.item()?.alt}</span>
                        </Show>
                    </span>
                    <Meta s={s} />
                    <Actions s={s} onClose={props.onClose} />
                </div>
                {/* One polite announcement per step. */}
                <span class={`${BLOCK}__live`} aria-live="polite">
                    {s.label()}
                </span>
            </div>
        </Modal>
    );
}
