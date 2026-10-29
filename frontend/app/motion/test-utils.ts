// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// A fake Element.animate for jsdom, which has none, so tests can drive the motion primitives:
// every call is recorded, and the test finishes or cancels it when it chooses.

export interface FakeAnimationCall {
    el: Element;
    keyframes: Keyframe[];
    options: KeyframeAnimationOptions;
    /** What the element's animation reports to its handlers. */
    finish(): void;
    cancel(): void;
    readonly ended: boolean;
}

export interface FakeAnimate {
    calls: FakeAnimationCall[];
    /** Finish every animation that is still running. */
    finishAll(): void;
    restore(): void;
}

/** Install the fake on Element.prototype. `fail` makes `animate` throw, as an engine can. */
export function installFakeAnimate(opts: { fail?: boolean } = {}): FakeAnimate {
    const proto = Element.prototype as unknown as { animate?: unknown };
    const had = Object.prototype.hasOwnProperty.call(proto, "animate");
    const previous = proto.animate;
    const calls: FakeAnimationCall[] = [];
    proto.animate = function (this: Element, keyframes: Keyframe[], options: KeyframeAnimationOptions) {
        if (opts.fail) throw new Error("animate failed");
        let ended = false;
        const animation: Record<string, unknown> = { onfinish: null, oncancel: null };
        const end = (handler: string) => {
            if (ended) return;
            ended = true;
            (animation[handler] as (() => void) | null)?.();
        };
        animation.cancel = () => end("oncancel");
        const call: FakeAnimationCall = {
            el: this,
            keyframes,
            options,
            finish: () => end("onfinish"),
            cancel: () => end("oncancel"),
            get ended() {
                return ended;
            },
        };
        calls.push(call);
        return animation;
    };
    return {
        calls,
        finishAll: () => calls.forEach((c) => c.finish()),
        restore() {
            if (had) proto.animate = previous;
            else delete proto.animate;
        },
    };
}

/** Make `document.visibilityState` read `state` until the returned function is called. */
export function setVisibility(state: DocumentVisibilityState): () => void {
    Object.defineProperty(document, "visibilityState", { configurable: true, get: () => state });
    return () => {
        delete (document as unknown as Record<string, unknown>).visibilityState;
    };
}
