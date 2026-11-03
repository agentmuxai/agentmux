// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The Editor's audio and video viewer, moved from the retired Media pane:
 * native `<video>` / `<audio>` controls, a fade-in once the
 * first frame is there, and the browser's own error code when a file won't
 * play (a missing codec reads differently from a damaged file). Nothing
 * plays on its own. Only the tab in front is mounted, so a video behind
 * another tab stops. Loop, speed in quarter steps and (for video) a frame step
 * sit in the viewer bar and on YouTube's keys (`L`, `<` `>`, `,` `.`).
 * docs/specs/SPEC_EDITOR_REVAMP_UNIFIED_VIEWERS_2026_10_15.md §4.4.3, §4.13.
 */

import { describeMediaError } from "@/app/element/local-media";
import { formatBytes } from "@/util/format-bytes";
import { EASE_STANDARD } from "@/app/motion/tokens";
import { Button, IconButton } from "@/app/element/ui";
import { createEffect, createSignal, on, onCleanup, Show, type JSX } from "solid-js";
import { createFileBytes } from "../file-bytes";
import { ViewerBar, type ViewerBarProps } from "../viewer-bar";
import {
    avKeyAction,
    DEFAULT_FRAME_SECONDS,
    formatRate,
    plausibleFrameSeconds,
    stepFrameTime,
    stepRate,
    type AvKeyAction,
} from "./av-controls";

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
    const [loop, setLoop] = createSignal(false);
    const [rate, setRate] = createSignal(1);
    let mediaEl: HTMLMediaElement | undefined;
    let frameSeconds = DEFAULT_FRAME_SECONDS;
    let stopFrameWatch: () => void = () => {};
    createEffect(on(bytes.url, () => setReady(false), { defer: true }));
    // A new file starts at normal speed, without a loop, as the browser's own
    // element would; the controls then drive the element they are shown for.
    createEffect(
        on(
            () => props.path + "|" + (props.file?.name ?? ""),
            () => {
                setLoop(false);
                setRate(1);
            },
            { defer: true }
        )
    );
    createEffect(() => {
        if (!mediaEl) return;
        mediaEl.loop = loop();
        mediaEl.playbackRate = rate();
    });
    onCleanup(() => stopFrameWatch());
    const isAudio = () => props.mime.startsWith("audio/");
    const fade = () => ({ opacity: ready() ? 1 : 0, transition: `opacity var(--duration-quick) ${EASE_STANDARD}` });
    const loaded = (el: HTMLMediaElement) => {
        setReady(true);
        if (Number.isFinite(el.duration)) setDuration(el.duration);
        el.loop = loop();
        el.playbackRate = rate();
        if (el instanceof HTMLVideoElement) watchFrameLength(el);
    };

    // Seeking alone isn't frame-accurate unless the frame length is known, so
    // while the video plays the gap between two shown frames is measured.
    const watchFrameLength = (video: HTMLVideoElement) => {
        stopFrameWatch();
        if (typeof video.requestVideoFrameCallback !== "function") return;
        let lastTime = -1;
        let handle = 0;
        const onFrame: VideoFrameRequestCallback = (_now, meta) => {
            if (lastTime >= 0) {
                const frame = meta.mediaTime - lastTime;
                if (frame > 0 && plausibleFrameSeconds(frame)) frameSeconds = frame;
            }
            lastTime = meta.mediaTime;
            handle = video.requestVideoFrameCallback(onFrame);
        };
        handle = video.requestVideoFrameCallback(onFrame);
        stopFrameWatch = () => video.cancelVideoFrameCallback?.(handle);
    };

    const stepFrame = (direction: 1 | -1) => {
        const video = mediaEl;
        if (!(video instanceof HTMLVideoElement)) return;
        video.pause();
        video.currentTime = stepFrameTime(video.currentTime, direction, frameSeconds, video.duration);
    };

    const act = (action: AvKeyAction) => {
        if (action === "slower") setRate((r) => stepRate(r, -1));
        else if (action === "faster") setRate((r) => stepRate(r, 1));
        else if (action === "loop") setLoop((l) => !l);
        else if (!isAudio()) stepFrame(action === "frame-back" ? -1 : 1);
    };

    const onKeyDown = (e: KeyboardEvent) => {
        const action = avKeyAction(e);
        if (!action) return;
        e.preventDefault();
        e.stopPropagation();
        act(action);
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
            <ViewerBar
                {...props.bar}
                facts={facts()}
                start={
                    <>
                        <IconButton
                            icon="repeat"
                            density="compact"
                            pressed={loop()}
                            label={loop() ? "Looping (L to stop)" : "Loop (L)"}
                            onClick={() => act("loop")}
                        />
                        <IconButton icon="backward" density="compact" label="Slower (<)" onClick={() => act("slower")} />
                        <Button density="compact" title="Normal speed" pressed={rate() === 1} onClick={() => setRate(1)}>
                            {formatRate(rate())}
                        </Button>
                        <IconButton icon="forward" density="compact" label="Faster (>)" onClick={() => act("faster")} />
                        <Show when={!isAudio()}>
                            <IconButton icon="angle-left" density="compact" label="Back one frame (,)" onClick={() => act("frame-back")} />
                            <IconButton icon="angle-right" density="compact" label="Forward one frame (.)" onClick={() => act("frame-forward")} />
                        </Show>
                    </>
                }
            />
            <div class="av-viewer-content" onKeyDown={onKeyDown}>
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
                                ref={(el) => (mediaEl = el)}
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
                            ref={(el) => (mediaEl = el)}
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
