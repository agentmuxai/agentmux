// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/*
 * Preview of one image or video of an agent's message, over the whole tab it is in, with ←/→ to step through the
 * message's other images and videos, Esc to close and Open in editor for the one shown. A click on an image
 * switches between fitting the tab and its actual size. The shell and styles are the attachment preview's
 * (attachments/AttachmentLightbox.tsx); the files are local, not attachment ids
 * (SPEC_AGENT_PANE_IMAGE_ARROWS_2026_11_06.md).
 */

import { showMediaCopyMenu, sourceFromUrl } from "@/app/element/media-copy";
import { Modal } from "@/app/element/modal";
import { t } from "@/app/i18n/i18n";
import { tip } from "@/app/element/tip";
import { Button, CloseButton, IconButton } from "@/app/element/ui";
import { formatBytes } from "@/util/format-bytes";
import { Show, type JSX } from "solid-js";
import { createMessageLightbox, type MessageImageLightboxProps, type MessageLightboxState } from "./message-image-lightbox-state";

const BLOCK = "agent-attachment-lightbox";

function Picture(props: { s: MessageLightboxState }): JSX.Element {
    const s = props.s;
    const url = () => (s.entry()?.kind === "ready" ? (s.entry() as { url: string }).url : undefined);
    return (
        <Show
            when={url()}
            fallback={
                <span class={`${BLOCK}__missing`}>
                    {s.entry()?.kind === "missing" || s.entry()?.kind === "too-large"
                        ? s.isVideo()
                            ? t("mediaPreview.videoGone")
                            : t("mediaPreview.imageGone")
                        : "Loading…"}
                </span>
            }
        >
            {(src) => (
                <Show
                    when={s.isVideo()}
                    fallback={
                        <img
                            src={src()}
                            alt={s.item()?.alt || s.item()?.name || ""}
                            classList={{ "is-actual": s.actual() }}
                            {...tip(s.actual() ? "Fit to the window (1)" : "Actual size (1)")}
                            onClick={s.toggleActual}
                            onContextMenu={(e) => showMediaCopyMenu(e, sourceFromUrl(src(), { path: s.item()?.path }))}
                            onLoad={(e) => s.setSize(s.item()!.path, e.currentTarget.naturalWidth, e.currentTarget.naturalHeight)}
                        />
                    }
                >
                    <video
                        ref={s.setVideo}
                        src={src()}
                        controls
                        playsinline
                        aria-label={s.item()?.alt || s.item()?.name}
                        onContextMenu={(e) =>
                            showMediaCopyMenu(e, sourceFromUrl(src(), { kind: "video", path: s.item()?.path, element: () => e.currentTarget }))
                        }
                        onLoadedMetadata={(e) =>
                            s.setSize(s.item()!.path, e.currentTarget.videoWidth, e.currentTarget.videoHeight)
                        }
                    />
                </Show>
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
            <IconButton icon="chevron-left" label="Previous" tooltip={false} title="Previous (←)" onClick={() => s.go(s.index() - 1)} />
            <span class="pos">
                {Math.min(s.index(), s.count() - 1) + 1} / {s.count()}
            </span>
            <IconButton icon="chevron-right" label="Next" tooltip={false} title="Next (→)" onClick={() => s.go(s.index() + 1)} />
            <Button title="Open this file in the editor (E)" onClick={s.openEditor}>
                Open in editor
            </Button>
            <CloseButton label="Close preview" tooltip={false} title="Close (Esc)" onClick={props.onClose} />
        </span>
    );
}

/** The keys this preview answers to, for the item shown. */
function hint(s: MessageLightboxState): string {
    const step = s.count() > 1 ? "← → step · " : "";
    return s.isVideo()
        ? `${step}Space play · J L skip 10 s · E editor · Esc close`
        : `${step}1 or click: actual size · E editor · Esc close`;
}

export function MessageImageLightbox(props: MessageImageLightboxProps): JSX.Element {
    const s = createMessageLightbox(props);
    return (
        // A tab modal: it covers the whole tab the message is in, not just its pane (Escape and a click outside
        // close it; the tab bar and other tabs stay live). The arrows are this dialog's own keys.
        <Modal
            open={true}
            scope="tab"
            size="fit"
            panelClass={`${BLOCK}__panel ${BLOCK}__panel--fit`}
            onClose={s.close}
            ariaLabel={s.label()}
            initialFocus={() => s.dialog() ?? null}
        >
            <div ref={s.setDialog} class={`${BLOCK}__dialog ${BLOCK}__dialog--fit`} tabIndex={-1} onKeyDown={s.onKeyDown}>
                <div class={`${BLOCK}__image ${BLOCK}__image--fit`} classList={{ "is-actual": s.actual() }}>
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
                    <Actions s={s} onClose={s.close} />
                </div>
                <div class={`${BLOCK}__hint`}>{hint(s)}</div>
                {/* One polite announcement per step. */}
                <span class={`${BLOCK}__live`} aria-live="polite">
                    {s.label()}
                </span>
            </div>
        </Modal>
    );
}
