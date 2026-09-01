// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Media-permission prompt for browser panes (camera / microphone).
 *
 * Spec: `docs/specs/SPEC_BROWSER_PANE_CAMERA_ACCESS_2026_09_01.md` §3.5.
 *
 * # Why this lives in the main window's DOM
 *
 * The pane asking for permission is arbitrary web content on a native surface.
 * Rendering the prompt inside that pane would let the page draw something that
 * looks exactly like it — the classic permission-spoofing shape. The host emits
 * this event to the DOM of the window that *owns* the pane (not to the pane's
 * page, and not unconditionally to "main" — a torn-off pane lives in its
 * floating window), so the prompt is AgentMux chrome the page cannot reach,
 * draw over, or click for the user.
 *
 * # Deny is the default everywhere
 *
 * `destructive` puts initial focus on Cancel and makes Allow the non-default
 * action. Dismissal (Escape, backdrop) denies. If this component never mounts
 * or never answers, the host's own timeout denies. There is no path where
 * silence becomes permission.
 */

import { createSignal, onCleanup, onMount, Show } from "solid-js";
import type { JSX } from "solid-js";
import { ConfirmModal } from "@/element/confirm-modal";
import { invokeCommand, listenEvent } from "@/app/platform/ipc";

/** Mirrors `cef_media_access_permission_types_t`. */
const DEVICE_AUDIO_CAPTURE = 1 << 0;
const DEVICE_VIDEO_CAPTURE = 1 << 1;
const DESKTOP_AUDIO_CAPTURE = 1 << 2;
const DESKTOP_VIDEO_CAPTURE = 1 << 3;

interface PermissionRequest {
    requestId: number;
    blockId: string;
    origin: string;
    requested: number;
}

/**
 * Human names for exactly the bits requested.
 *
 * Named precisely rather than generically ("wants to use your camera and
 * microphone", not "wants media access") because the whole decision rests on
 * the user knowing what they are agreeing to. Unknown bits are surfaced rather
 * than dropped — silently under-reporting what a page asked for would be the
 * worst possible failure here.
 */
export function describeRequestedDevices(bits: number): string {
    const names: string[] = [];
    if (bits & DEVICE_VIDEO_CAPTURE) names.push("camera");
    if (bits & DEVICE_AUDIO_CAPTURE) names.push("microphone");
    if (bits & DESKTOP_VIDEO_CAPTURE) names.push("screen contents");
    if (bits & DESKTOP_AUDIO_CAPTURE) names.push("system audio");

    const known =
        DEVICE_AUDIO_CAPTURE | DEVICE_VIDEO_CAPTURE | DESKTOP_AUDIO_CAPTURE | DESKTOP_VIDEO_CAPTURE;
    if (bits & ~known) names.push("other media devices");

    if (names.length === 0) return "media devices";
    if (names.length === 1) return names[0];
    return `${names.slice(0, -1).join(", ")} and ${names[names.length - 1]}`;
}

/**
 * Origin as shown to the user.
 *
 * Displays the host only — a full URL invites spoofing via a long path that
 * pushes the real origin out of view, and the grant is origin-scoped anyway, so
 * the host is the honest unit. Falls back to the raw string if it will not
 * parse, rather than showing nothing.
 */
export function displayOrigin(origin: string): string {
    try {
        return new URL(origin).host || origin;
    } catch {
        return origin || "This page";
    }
}

export function PaneMediaPermissionPrompt(): JSX.Element {
    const [request, setRequest] = createSignal<PermissionRequest | null>(null);

    onMount(() => {
        let dispose: (() => void) | undefined;
        void listenEvent<PermissionRequest>("pane-media-permission-request", (payload) => {
            if (!payload || typeof payload.requestId !== "number") return;
            // One prompt at a time. A second request while one is open would
            // otherwise replace it, and the user's click would land on a
            // different question than the one they read. The displaced request
            // is denied immediately rather than left parked.
            const current = request();
            if (current) {
                void respond(current.requestId, false);
            }
            setRequest(payload);
        }).then((d) => {
            dispose = d;
        });
        onCleanup(() => dispose?.());
    });

    const respond = async (requestId: number, allow: boolean) => {
        try {
            await invokeCommand("pane_media_permission_respond", { requestId, allow });
        } catch (e) {
            // The host's timeout still denies, so a failed response degrades to
            // denial rather than a stuck page.
            console.error("[pane-media] failed to deliver permission response", e);
        }
    };

    const answer = (allow: boolean) => {
        const req = request();
        if (!req) return;
        setRequest(null);
        void respond(req.requestId, allow);
    };

    return (
        <Show when={request()}>
            {(req) => (
                <ConfirmModal
                    open={true}
                    scope="window"
                    // Attributed to the SITE, not to AgentMux — the user is
                    // deciding about the page, and a prompt that reads like an
                    // app prompt trains people to accept them.
                    title={`${displayOrigin(req().origin)} wants to use your ${describeRequestedDevices(req().requested)}`}
                    description="This page is running in a browser pane. While it is capturing, an indicator appears with a Stop button."
                    confirmLabel="Allow"
                    cancelLabel="Don't allow"
                    destructive={true}
                    onConfirm={() => answer(true)}
                    onCancel={() => answer(false)}
                />
            )}
        </Show>
    );
}
