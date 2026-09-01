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
 * without focusing the pane".
 */

import { createSignal, For, onCleanup, onMount, Show } from "solid-js";
import type { JSX } from "solid-js";
import { ConfirmModal } from "@/element/confirm-modal";
import { invokeCommand, listenEvent } from "@/app/platform/ipc";

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

export function PaneMediaCaptureIndicator(): JSX.Element {
    const [capturing, setCapturing] = createSignal<CapturingPane[]>([]);
    const [confirmRevoke, setConfirmRevoke] = createSignal<string | null>(null);

    onMount(() => {
        let dispose: (() => void) | undefined;
        void listenEvent<CaptureChange>("pane-media-capture-changed", (p) => {
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
        setConfirmRevoke(null);
        try {
            await invokeCommand("pane_media_revoke", { blockId });
        } catch (e) {
            console.error("[pane-media] revoke failed", e);
        }
        // Don't optimistically clear the row: the authoritative signal is CEF's
        // next OnMediaAccessChange. Clearing here would show "not capturing"
        // even if the revoke failed — precisely the lie this indicator exists
        // to prevent.
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
                        "z-index": 9998,
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
                                    background: "var(--error-color, #c22a2a)",
                                    color: "#fff",
                                    "font-size": "12px",
                                    "box-shadow": "0 2px 8px rgba(0,0,0,0.35)",
                                }}
                            >
                                <span aria-hidden="true">●</span>
                                <span>{describeCapture(pane.hasVideo, pane.hasAudio)}</span>
                                <button
                                    onClick={() => setConfirmRevoke(pane.blockId)}
                                    style={{
                                        background: "rgba(255,255,255,0.18)",
                                        color: "#fff",
                                        border: "none",
                                        "border-radius": "4px",
                                        padding: "2px 8px",
                                        cursor: "pointer",
                                        "font-size": "12px",
                                    }}
                                >
                                    Stop
                                </button>
                            </div>
                        )}
                    </For>
                </div>
            </Show>

            <Show when={confirmRevoke()}>
                {(blockId) => (
                    <ConfirmModal
                        open={true}
                        scope="window"
                        title="Stop camera and microphone access?"
                        // The reload is not an implementation detail to hide.
                        // CEF cannot terminate a live capture any other way
                        // (§3.7), so stopping it genuinely costs the page's
                        // state, and the user should know before clicking.
                        description="The page will be reloaded to stop any capture already in progress. Anything unsaved on that page will be lost."
                        confirmLabel="Stop and reload"
                        destructive={true}
                        onConfirm={() => void doRevoke(blockId())}
                        onCancel={() => setConfirmRevoke(null)}
                    />
                )}
            </Show>
        </>
    );
}
