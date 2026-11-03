// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The audio and video viewer's playback state: loop, speed and the frame step,
 * kept on the media element they are shown for. Seeking alone isn't
 * frame-accurate unless the frame length is known, so while a video plays the
 * shortest gap between two shown frames is measured (a gap that spans several
 * frames, from a dropped frame, 4× speed or a seek while playing, is longer
 * than a frame, never shorter).
 * docs/reports/REPORT_MEDIA_VIEWER_ZOOM_AND_FEATURES_2026_10_09.md §4 row 7.
 */

import { createEffect, createSignal, on, onCleanup, type Accessor } from "solid-js";
import { avKeyAction, DEFAULT_FRAME_SECONDS, plausibleFrameSeconds, stepFrameTime, stepRate, type AvKeyAction } from "./av-controls";

export interface AvPlayback {
    loop: Accessor<boolean>;
    rate: Accessor<number>;
    setRate: (rate: number | ((rate: number) => number)) => void;
    /** `ref` of the `<video>` or `<audio>` the controls drive. */
    bind: (el: HTMLMediaElement) => void;
    /** The element has its first frame: apply the controls to it. */
    loaded: (el: HTMLMediaElement) => void;
    act: (action: AvKeyAction) => void;
    onKeyDown: (e: KeyboardEvent) => void;
}

export function createAvPlayback(opts: { file: Accessor<string>; isAudio: Accessor<boolean> }): AvPlayback {
    const [loop, setLoop] = createSignal(false);
    const [rate, setRate] = createSignal(1);
    // A signal: the element mounts after the file's bytes arrive, so the effect below must rerun then.
    const [mediaEl, setMediaEl] = createSignal<HTMLMediaElement>();
    let measuredFrame: number | null = null;
    let stopFrameWatch: () => void = () => {};
    // A new file starts at normal speed, without a loop, as the browser's own
    // element would; the controls then drive the element they are shown for.
    createEffect(
        on(
            opts.file,
            () => {
                setLoop(false);
                setRate(1);
                measuredFrame = null;
            },
            { defer: true }
        )
    );
    createEffect(() => {
        const el = mediaEl();
        if (!el) return;
        el.loop = loop();
        el.playbackRate = rate();
    });
    onCleanup(() => stopFrameWatch());

    const watchFrameLength = (video: HTMLVideoElement) => {
        stopFrameWatch();
        if (typeof video.requestVideoFrameCallback !== "function") return;
        let lastTime = -1;
        let handle = 0;
        const onFrame: VideoFrameRequestCallback = (_now, meta) => {
            if (lastTime >= 0) {
                const frame = meta.mediaTime - lastTime;
                if (frame > 0 && plausibleFrameSeconds(frame)) measuredFrame = measuredFrame == null ? frame : Math.min(measuredFrame, frame);
            }
            lastTime = meta.mediaTime;
            handle = video.requestVideoFrameCallback(onFrame);
        };
        const forgetLast = () => (lastTime = -1);
        video.addEventListener("seeking", forgetLast);
        handle = video.requestVideoFrameCallback(onFrame);
        stopFrameWatch = () => {
            video.cancelVideoFrameCallback?.(handle);
            video.removeEventListener("seeking", forgetLast);
        };
    };

    const stepFrame = (direction: 1 | -1) => {
        const video = mediaEl();
        if (!(video instanceof HTMLVideoElement)) return;
        video.pause();
        video.currentTime = stepFrameTime(video.currentTime, direction, measuredFrame ?? DEFAULT_FRAME_SECONDS, video.duration);
    };

    const act = (action: AvKeyAction) => {
        if (action === "slower") setRate((r) => stepRate(r, -1));
        else if (action === "faster") setRate((r) => stepRate(r, 1));
        else if (action === "loop") setLoop((l) => !l);
        else if (!opts.isAudio()) stepFrame(action === "frame-back" ? -1 : 1);
    };

    return {
        loop,
        rate,
        setRate,
        bind: (el) => {
            // The browser's own speed menu changes the rate too: follow it, so the readout and the buttons agree.
            el.addEventListener("ratechange", () => setRate(el.playbackRate));
            setMediaEl(el);
        },
        loaded: (el) => {
            el.loop = loop();
            el.playbackRate = rate();
            if (el instanceof HTMLVideoElement) watchFrameLength(el);
        },
        act,
        onKeyDown: (e) => {
            const action = avKeyAction(e);
            if (!action) return;
            e.preventDefault();
            e.stopPropagation();
            act(action);
        },
    };
}
