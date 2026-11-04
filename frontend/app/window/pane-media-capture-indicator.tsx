// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Live capture indicator + revoke for browser panes.
 *
 * Spec: `docs/specs/SPEC_BROWSER_PANE_CAMERA_ACCESS_2026_09_01.md` §4, §3.7.
 *
 * §4 makes three things non-negotiable once a pane can capture at all:
 * visible while live, revocable in one click, and a revoke that actually stops
 * the stream rather than only preventing future ones. This component is the
 * first two; the host command it calls is the third.
 *
 * # Driven by real capture state, not by grants
 *
 * The `pane-media-capture-changed` event comes from CEF's own
 * `OnMediaAccessChange`, so the indicator reflects what is *actually being
 * captured right now*. Deriving it from grants would leave it lit after a page
 * stopped using the camera — an indicator that is on when nothing is happening
 * teaches people to ignore it, which is worse than not having one.
 *
 * # Why it lives in window chrome rather than on the pane
 *
 * Panes are native surfaces; drawing over one means the airspace-clip
 * machinery, and anything rendered *inside* the pane could be obscured or
 * imitated by the page. Chrome-level placement also satisfies §4's "visible
 * without focusing the pane". Only the "Stop?" question is asked in the pane
 * itself, as a pane modal over the one pane being stopped.
 */

import { createSignal, For, onCleanup, onMount, Show } from "solid-js";
import type { JSX } from "solid-js";
import { ConfirmModal } from "@/element/confirm-modal";
import { openPaneModal, type PaneRequestHandle, type PaneRequestProps } from "@/app/store/pane-requests";
import { Button } from "@/element/ui";
import { revealBlock } from "@/app/util/reveal-block";
import { getApi } from "@/util/app-api";
import { Z_CAPTURE_INDICATOR } from "@/app/zindex";

interface CaptureChange {
    blockId: string;
    hasVideo: boolean;
    hasAudio: boolean;
}

interface CapturingPane {
    blockId: string;
    hasVideo: boolean;
    hasAudio: boolean;
}

/** What to call what's live, most-sensitive first. */
export function describeCapture(hasVideo: boolean, hasAudio: boolean): string {
    if (hasVideo && hasAudio) return "Camera and microphone in use";
    if (hasVideo) return "Camera in use";
    if (hasAudio) return "Microphone in use";
    return "";
}

/** "Stop and reload?", drawn over the pane whose capture would stop. */
function StopCaptureConfirm(props: { onStop: () => void; onCancel: () => void } & PaneRequestProps): JSX.Element {
    return (
        <ConfirmModal
            open={true}
            scope="pane"
            title="Stop camera and microphone access?"
            // The reload is not an implementation detail to hide.
            // CEF cannot terminate a live capture any other way
            // (§3.7), so stopping it genuinely costs the page's
            // state, and the user should know before clicking.
            description="The page will be reloaded to stop any capture already in progress. Anything unsaved on that page will be lost."
            confirmLabel="Stop and reload"
            destructive={true}
            onConfirm={props.onStop}
            onCancel={props.onCancel}
        />
    );
}

export function PaneMediaCaptureIndicator(): JSX.Element {
    const [capturing, setCapturing] = createSignal<CapturingPane[]>([]);
    // The open "Stop?" question of each pane: a second click on Stop doesn't stack another.
    const asking = new Map<string, PaneRequestHandle>();

    onMount(() => {
        let dispose: (() => void) | undefined;
        void getApi().listen<CaptureChange>("pane-media-capture-changed", (p) => {
            if (!p || typeof p.blockId !== "string") return;
            setCapturing((prev) => {
                const rest = prev.filter((c) => c.blockId !== p.blockId);
                // Neither device live → the pane drops off the list entirely,
                // rather than lingering as an "off" row.
                if (!p.hasVideo && !p.hasAudio) return rest;
                return [...rest, { blockId: p.blockId, hasVideo: p.hasVideo, hasAudio: p.hasAudio }];
            });
        }).then((d) => {
            dispose = d;
        });
        onCleanup(() => dispose?.());
    });

    const doRevoke = async (blockId: string) => {
        try {
            await getApi().browserPanes.revokeMedia(blockId);
        } catch (e) {
            console.error("[pane-media] revoke failed", e);
        }
        // Don't optimistically clear the row: the authoritative signal is CEF's
        // next OnMediaAccessChange. Clearing here would show "not capturing"
        // even if the revoke failed — precisely the lie this indicator exists
        // to prevent.
    };

    const askRevoke = (blockId: string) => {
        if (asking.has(blockId)) return;
        const done = () => {
            asking.get(blockId)?.close();
            asking.delete(blockId);
        };
        const onStop = () => {
            done();
            void doRevoke(blockId);
        };
        // The person asked by clicking Stop, so nothing else waits on them.
        asking.set(blockId, openPaneModal(blockId, StopCaptureConfirm, { onStop, onCancel: done }, { source: "media-stop", waits: false, onGone: done }));
        // The pane may be in another tab: show it, so the question has somewhere to appear.
        void revealBlock(blockId, { source: "media-stop", focusCaret: false, localOnly: true });
    };

    return (
        <>
            <Show when={capturing().length > 0}>
                <div
                    class="pane-media-capture-indicator"
                    role="status"
                    aria-live="polite"
                    style={{
                        position: "fixed",
                        bottom: "12px",
                        right: "12px",
                        "z-index": Z_CAPTURE_INDICATOR,
                        display: "flex",
                        "flex-direction": "column",
                        gap: "6px",
                    }}
                >
                    <For each={capturing()}>
                        {(pane) => (
                            <div
                                style={{
                                    display: "flex",
                                    "align-items": "center",
                                    gap: "10px",
                                    padding: "6px 10px",
                                    "border-radius": "6px",
                                    background: "var(--error-color)",
                                    color: "#fff",
                                    "font-size": "12px",
                                    "box-shadow": "0 2px 8px rgba(0,0,0,0.35)",
                                }}
                            >
                                <span aria-hidden="true">●</span>
                                <span>{describeCapture(pane.hasVideo, pane.hasAudio)}</span>
                                <Button
                                    density="compact"
                                    onClick={() => askRevoke(pane.blockId)}
                                    style={{
                                        "--ui-fg": "#fff",
                                        "--ui-fg-hover": "#fff",
                                        "--ui-bg": "rgba(255,255,255,0.18)",
                                        "--ui-line": "transparent",
                                        "--ui-line-hover": "#fff",
                                        "--ui-tone": "#fff",
                                        "font-size": "12px",
                                    }}
                                >
                                    Stop
                                </Button>
                            </div>
                        )}
                    </For>
                </div>
            </Show>
        </>
    );
}
