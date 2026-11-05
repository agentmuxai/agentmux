// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Media-permission prompt for browser panes (camera / microphone).
 *
 * Spec: `docs/specs/SPEC_BROWSER_PANE_CAMERA_ACCESS_2026_09_01.md` §3.5.
 *
 * # Why it is AgentMux's DOM, not the page's
 *
 * The pane asking for permission is arbitrary web content on a native surface.
 * Rendering the prompt from the page would let it draw something that looks
 * exactly like it — the classic permission-spoofing shape. The host emits this
 * event to the DOM of the window that *owns* the pane (not to the pane's page,
 * and not unconditionally to "main" — a torn-off pane lives in its floating
 * window), and the prompt is a pane modal drawn by AgentMux over that one pane
 * (the native surface is clipped for it): chrome the page cannot reach, draw
 * over, or click for the user, and it covers the pane that asked, not the
 * whole window.
 *
 * # Deny is the default everywhere
 *
 * `destructive` puts initial focus on Cancel and makes Allow the non-default
 * action. Dismissal (Escape, backdrop) denies, and so does closing the pane. If
 * this component never mounts or never answers, the host's own timeout denies.
 * There is no path where silence becomes permission.
 */

import { onCleanup, onMount } from "solid-js";
import type { JSX } from "solid-js";
import { ConfirmModal } from "@/element/confirm-modal";
import { openPaneModal, type PaneRequestHandle, type PaneRequestProps } from "@/app/store/pane-requests";
import { getApi } from "@/util/app-api";

/** Mirrors `cef_media_access_permission_types_t`. */
const DEVICE_AUDIO_CAPTURE = 1 << 0;
const DEVICE_VIDEO_CAPTURE = 1 << 1;
const DESKTOP_AUDIO_CAPTURE = 1 << 2;
const DESKTOP_VIDEO_CAPTURE = 1 << 3;

/** How long the host waits before it denies on its own (`PROMPT_TIMEOUT_MS`, crates/cef media_prompt.rs). */
const HOST_TIMEOUT_MS = 60_000;

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

/** The question, drawn over the one pane that asked it. */
function MediaPermissionAsk(props: { req: PermissionRequest; onAnswer: (allow: boolean) => void } & PaneRequestProps): JSX.Element {
    return (
        <ConfirmModal
            open={true}
            scope="pane"
            // Attributed to the SITE, not to AgentMux — the user is
            // deciding about the page, and a prompt that reads like an
            // app prompt trains people to accept them.
            title={question(props.req)}
            description="This page is running in a browser pane. While it is capturing, an indicator appears with a Stop button."
            confirmLabel="Allow"
            cancelLabel="Don't allow"
            destructive={true}
            onConfirm={() => props.onAnswer(true)}
            onCancel={() => props.onAnswer(false)}
        />
    );
}

const question = (req: PermissionRequest): string =>
    `${displayOrigin(req.origin)} wants to use your ${describeRequestedDevices(req.requested)}`;

interface Asking {
    requestId: number;
    handle: PaneRequestHandle;
    timer: ReturnType<typeof setTimeout>;
}

/** Listens for the host's requests; renders nothing itself (each prompt is a modal in its pane). */
export function PaneMediaPermissionPrompt(): JSX.Element {
    // The open prompt of each pane: panes ask independently, but one pane has one question at a time.
    const asking = new Map<string, Asking>();

    /** Ends pane `blockId`'s prompt `requestId` with `allow` (a no-op if it already ended). */
    const finish = (blockId: string, requestId: number, allow: boolean) => {
        const cur = asking.get(blockId);
        if (!cur || cur.requestId !== requestId) return;
        asking.delete(blockId);
        clearTimeout(cur.timer);
        cur.handle.close();
        void respond(requestId, allow);
    };

    onMount(() => {
        let dispose: (() => void) | undefined;
        void getApi().listen<PermissionRequest>("pane-media-permission-request", (req) => {
            if (!req || typeof req.requestId !== "number" || typeof req.blockId !== "string") return;
            // A second request from the same pane would otherwise replace the open one, and the
            // user's click would land on a different question than the one they read. The displaced
            // request is denied immediately rather than left parked.
            const open = asking.get(req.blockId);
            if (open) finish(req.blockId, open.requestId, false);
            const deny = () => finish(req.blockId, req.requestId, false);
            asking.set(req.blockId, {
                requestId: req.requestId,
                handle: openPaneModal(
                    req.blockId,
                    MediaPermissionAsk,
                    { req, onAnswer: (allow: boolean) => finish(req.blockId, req.requestId, allow) },
                    // Closing the pane with the question open denies it.
                    { source: "media-permission", text: question(req), onGone: deny }
                ),
                // The host has denied by now: take the stale question away rather than leave it for
                // a pane that was out of sight.
                timer: setTimeout(deny, HOST_TIMEOUT_MS),
            });
        }).then((d) => {
            dispose = d;
        });
        onCleanup(() => {
            dispose?.();
            for (const [blockId, a] of [...asking]) finish(blockId, a.requestId, false);
        });
    });

    const respond = async (requestId: number, allow: boolean) => {
        try {
            await getApi().browserPanes.respondMediaPermission(requestId, allow);
        } catch (e) {
            // The host's timeout still denies, so a failed response degrades to
            // denial rather than a stuck page.
            console.error("[pane-media] failed to deliver permission response", e);
        }
    };

    return null;
}
