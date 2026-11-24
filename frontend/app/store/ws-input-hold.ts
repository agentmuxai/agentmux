// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// The window's keystrokes and resizes (`blockinput`, `setblocktermsize`) are accepted by srv only from a connection it
// has seen proven (crates/srv/src/server/window_input.rs). A frame typed while the connection's proof is still being
// made (the first connect, or a reconnect after srv restarted) is held here, in order, and sent when the proof settles,
// so nothing the person typed is refused for arriving a moment early.
//
// The hold is bounded. At the bound the newest frame is not kept and the person is told, once in a while, that the pane
// is not connected yet: input is never lost without a word.

import { t } from "@/app/i18n/i18n";
import { pushNotification } from "./flash-notifications";

/** The most frames held at once, and the most payload bytes in them (a paste in pieces still fits). */
export const HOLD_MAX_FRAMES = 4096;
export const HOLD_MAX_BYTES = 1 << 20;
/** The longest a frame is held for a proof that does not come: then it is sent and srv answers it. */
export const HOLD_MAX_MS = 10_000;
/** The person is told at most this often. */
const NOTICE_EVERY_MS = 10_000;

type Frame = { wscommand?: string; inputdata64?: string };

/** A frame srv refuses from a connection it has not seen proven. */
export function isInputFrame(cmd: Frame | null | undefined): boolean {
    return cmd?.wscommand === "blockinput" || cmd?.wscommand === "setblocktermsize";
}

let lastNotice = 0;

/** Says that input was not kept (`kind` "full": the hold is full) or not accepted ("refused": srv did not take it). */
export function noteInputNotSent(kind: "full" | "refused", now: number = Date.now()): void {
    if (now - lastNotice < NOTICE_EVERY_MS) return;
    lastNotice = now;
    pushNotification({
        icon: "fa-triangle-exclamation",
        title: t("windowInput.title"),
        message: kind === "full" ? t("windowInput.full") : t("windowInput.refused"),
        timestamp: new Date(now).toISOString(),
        type: "warning",
        expiration: now + 8000,
    });
}

/** Forget the last notice: for tests. */
export function resetInputNoticeForTests(): void {
    lastNotice = 0;
}

export class InputHold<F extends Frame = Frame> {
    private frames: F[] = [];
    private bytes = 0;
    private timer: ReturnType<typeof setTimeout> | null = null;

    /** `onTimeout` runs when the hold has waited `HOLD_MAX_MS`: the owner sends what is held. */
    constructor(
        private readonly onTimeout: () => void,
        private readonly notify: (kind: "full") => void = noteInputNotSent,
    ) {}

    get size(): number {
        return this.frames.length;
    }

    /** Keeps `frame` behind the ones already held. False when the hold is full and the frame was not kept (the person was told). */
    hold(frame: F): boolean {
        const cost = (frame.inputdata64?.length ?? 0) + 64;
        if (this.frames.length >= HOLD_MAX_FRAMES || this.bytes + cost > HOLD_MAX_BYTES) {
            this.notify("full");
            return false;
        }
        this.frames.push(frame);
        this.bytes += cost;
        this.timer ??= setTimeout(() => {
            this.timer = null;
            this.onTimeout();
        }, HOLD_MAX_MS);
        return true;
    }

    /** Everything held, oldest first, and the hold is empty. */
    take(): F[] {
        if (this.timer != null) clearTimeout(this.timer);
        this.timer = null;
        const frames = this.frames;
        this.frames = [];
        this.bytes = 0;
        return frames;
    }
}
