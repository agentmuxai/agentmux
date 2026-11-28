// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * A toast that closes itself and says when: "Closing in 7 s." with a bar that shrinks to nothing (the wording of the
 * shutdown banner's countdown, ShutdownPendingBanner). One timer per toast, ticking once a second, no layout read.
 * The countdown holds while the pointer or the keyboard focus is on the toast and starts again from the top when
 * both have left it, so a toast being read never closes. With reduced motion the bar is not drawn: the seconds
 * are the countdown.
 */

import { t } from "@/app/i18n/i18n";
import { shouldReduceMotion } from "@/app/motion/motion";
import { createSignal, For, onCleanup, Show, type JSX } from "solid-js";
import { TOAST_COUNTDOWN_MS } from "./toast-live";

export type ToastCountdown = {
    /** Whole seconds left. */
    seconds: () => number;
    /** Changes each time the countdown starts over, to restart the bar. */
    cycle: () => number;
    held: () => boolean;
    /** The pointer or focus is on the toast (true) or has left it (false). */
    hold: (held: boolean) => void;
};

/** `ms` is the countdown's length (absent or 0: none, and nothing runs); `onDone` runs once, at the end. */
export function useToastCountdown(ms: number | undefined, onDone: () => void): ToastCountdown {
    const total = ms && ms > 0 ? Math.ceil(ms / 1000) : 0;
    const [seconds, setSeconds] = createSignal(total);
    const [cycle, setCycle] = createSignal(0);
    const [held, setHeld] = createSignal(false);
    let timer: ReturnType<typeof setInterval> | undefined;
    const stop = () => {
        if (timer !== undefined) clearInterval(timer);
        timer = undefined;
    };
    const tick = () => {
        const left = seconds() - 1;
        setSeconds(left);
        if (left <= 0) {
            stop();
            onDone();
        }
    };
    const start = () => {
        stop();
        setSeconds(total);
        setCycle((c) => c + 1);
        timer = setInterval(tick, 1000);
    };
    if (total > 0) start();
    onCleanup(stop);
    return {
        seconds,
        cycle,
        held,
        hold: (h) => {
            if (total === 0 || h === held()) return;
            setHeld(h);
            if (h) stop();
            else start();
        },
    };
}

/**
 * What a toast does with the pointer, focus and keys: hovering and keyboard focus each hold it (the expiry's pause, and
 * the countdown) and it resumes only when neither does; Esc closes a toast that counts down, at once, from any button in it.
 */
export function useToastHold(opts: {
    countdownMs: number | undefined;
    onRemove: () => void;
    onHold?: () => void;
    onRelease?: () => void;
}) {
    const countdown = useToastCountdown(opts.countdownMs, opts.onRemove);
    let hovering = false;
    let focused = false;
    const sync = () => {
        const held = hovering || focused;
        countdown.hold(held);
        if (held) opts.onHold?.();
        else opts.onRelease?.();
    };
    return {
        countdown,
        hoverIn: () => {
            hovering = true;
            sync();
        },
        hoverOut: () => {
            hovering = false;
            sync();
        },
        focusIn: () => {
            focused = true;
            sync();
        },
        focusOut: (e: FocusEvent & { currentTarget: HTMLElement }) => {
            // Focus moving between this toast's own buttons is still focus inside it.
            if (e.currentTarget.contains(e.relatedTarget as Node | null)) return;
            focused = false;
            sync();
        },
        keyDown: (e: KeyboardEvent) => {
            if (e.key !== "Escape" || !opts.countdownMs) return;
            e.stopPropagation();
            opts.onRemove();
        },
    };
}

/** "Closing in 7 s." and the shrinking bar. */
export function ToastCountdownLine(props: { countdown: ToastCountdown; ms?: number }): JSX.Element {
    const ms = () => props.ms ?? TOAST_COUNTDOWN_MS;
    return (
        <div class="toast-countdown" data-testid="toast-countdown">
            <div class="toast-countdown-text">{t("toast.countdown", { seconds: props.countdown.seconds() })}</div>
            <Show when={!shouldReduceMotion("status")}>
                <For each={[props.countdown.cycle()]}>
                    {() => (
                        <div
                            class="toast-countdown-bar"
                            classList={{ "toast-countdown-bar--held": props.countdown.held() }}
                            style={{ "animation-duration": `${Math.ceil(ms() / 1000)}s` }}
                        />
                    )}
                </For>
            </Show>
        </div>
    );
}
