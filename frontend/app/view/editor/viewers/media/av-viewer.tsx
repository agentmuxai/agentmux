// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The Editor's audio and video viewer, moved from the retired Media pane:
 * native `<video>` / `<audio>` controls, a fade-in once the
 * first frame is there, and the browser's own error code when a file won't
 * play (a missing codec reads differently from a damaged file). Nothing
 * plays on its own. Only the tab in front is mounted, so a video behind
 * another tab stops.
 * docs/specs/SPEC_EDITOR_REVAMP_UNIFIED_VIEWERS_2026_10_15.md §4.4.3, §4.13.
 */

import { describeMediaError, formatBytes } from "@/app/element/local-media";
import { DURATION_QUICK_MS, EASE_STANDARD } from "@/app/motion/tokens";
import { createEffect, createSignal, on, Show, type JSX } from "solid-js";
import { createFileBytes } from "../file-bytes";
import { ViewerBar, type ViewerBarProps } from "../viewer-bar";

export interface AvViewerProps {
    path: string;
    file?: File;
    revision: number;
    /** From the probe or the dropped file's type: audio/… or video/…. */
    mime: string;
    size?: number;
    bar: Omit<ViewerBarProps, "start" | "facts">;
}

export function AvViewer(props: AvViewerProps): JSX.Element {
    const bytes = createFileBytes(() => ({ path: props.path, file: props.file, revision: props.revision }));
    const [ready, setReady] = createSignal(false);
    const [duration, setDuration] = createSignal<number | null>(null);
    createEffect(on(bytes.url, () => setReady(false), { defer: true }));
    const isAudio = () => props.mime.startsWith("audio/");
    const fade = () => ({ opacity: ready() ? 1 : 0, transition: `opacity ${DURATION_QUICK_MS}ms ${EASE_STANDARD}` });
    const loaded = (el: HTMLMediaElement) => {
        setReady(true);
        if (Number.isFinite(el.duration)) setDuration(el.duration);
    };

    const facts = (): string[] => {
        const out: string[] = [];
        const d = duration();
        if (d != null) out.push(formatDuration(d));
        const size = props.size ?? props.file?.size;
        if (size != null) out.push(formatBytes(size));
        const type = props.mime.split("/")[1]?.toUpperCase();
        if (type) out.push(type);
        return out;
    };

    return (
        <div class="editor-viewer av-viewer">
            <ViewerBar {...props.bar} facts={facts()} />
            <div class="av-viewer-content">
                <Show when={bytes.error()}>
                    <div class="editor-viewer-message" role="alert">
                        {bytes.error()}
                    </div>
                </Show>
                <Show when={!bytes.error() && !bytes.url()}>
                    <div class="editor-viewer-message">Loading…</div>
                </Show>
                <Show when={!bytes.error() && bytes.url()}>
                    <Show
                        when={isAudio()}
                        fallback={
                            <video
                                class="av-viewer-video"
                                style={fade()}
                                src={bytes.url()!}
                                controls
                                preload="metadata"
                                onLoadedData={(e) => loaded(e.currentTarget)}
                                onError={(e) => bytes.setError(`Couldn't play the video: ${describeMediaError(e.currentTarget)}`)}
                            />
                        }
                    >
                        <audio
                            class="av-viewer-audio"
                            style={fade()}
                            src={bytes.url()!}
                            controls
                            preload="metadata"
                            onLoadedData={(e) => loaded(e.currentTarget)}
                            onError={(e) => bytes.setError(`Couldn't play the audio: ${describeMediaError(e.currentTarget)}`)}
                        />
                    </Show>
                </Show>
            </div>
        </div>
    );
}

/** "1:05", "1:02:09". */
export function formatDuration(seconds: number): string {
    const s = Math.round(seconds);
    const h = Math.floor(s / 3600);
    const m = Math.floor((s % 3600) / 60);
    const sec = String(s % 60).padStart(2, "0");
    return h > 0 ? `${h}:${String(m).padStart(2, "0")}:${sec}` : `${m}:${sec}`;
}
