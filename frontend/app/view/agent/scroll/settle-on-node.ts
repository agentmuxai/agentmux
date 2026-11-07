// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Landing a jump on a row the list has not measured yet (SPEC_AGENT_PANE_SEARCH_AND_DIM_MODE_2026_11_06 §4.3).
 *
 * A row far from the window is only an estimate until it mounts and the ResizeObserver reports its size, so a
 * jump aimed by the estimate lands short or long. The settler writes the aim at once (no smooth scroll: a
 * smooth scroll with a re-aim on top is the combination virtualised lists are reported to bounce on), then
 * re-aims whenever the layout slice's position for the row has moved from where it was aimed, until the aim has
 * held for a few frames in a row or the time is up. Anything the person does ends it at once, and so does a
 * newer jump, a zoom change or a resize: the person, not the search, wins.
 *
 * The positions it reads are the layout slice's (unzoomed CSS px), never a raw DOM pixel, and the writes go
 * through `scrollTranscriptTo`, the one scroll writer.
 */

/** Longest a jump keeps re-aiming. */
export const SETTLE_NODE_MAX_MS = 600;
/** Frames in a row with the aim unchanged before the jump is taken as landed. */
export const SETTLE_NODE_QUIET_FRAMES = 3;
/** An aim that differs by less than this many px is the same aim. */
const SAME_AIM_PX = 1;

export interface NodeSettlerHost {
    /** The scroll position that puts the target where it belongs now; null while the row has no place yet. */
    aim: () => number | null;
    /** Move the transcript there (unzoomed CSS px). */
    write: (top: number) => void;
    /** The person is scrolling: a wheel, a touch, a key, a drag. */
    userActive: () => boolean;
    /** Changes when a zoom change or a resize voids the aim. */
    viewKey: () => string;
    frame: (cb: () => void) => number;
    cancelFrame: (id: number) => void;
    now: () => number;
}

export interface NodeSettler {
    /** Aim at the target and keep re-aiming until it lands; a jump in progress is dropped. */
    start: () => void;
    /** Stop at once, leaving the position where it is. */
    cancel: () => void;
    active: () => boolean;
}

export function createNodeSettler(host: NodeSettlerHost): NodeSettler {
    let raf = 0;
    let running = false;
    // Bumped by every start and cancel: a frame that finds it changed belongs to a jump that is over.
    let token = 0;

    const stop = (): void => {
        token++;
        running = false;
        if (raf) host.cancelFrame(raf);
        raf = 0;
    };

    return {
        start() {
            stop();
            const mine = token;
            running = true;
            const startedAt = host.now();
            const key = host.viewKey();
            const wasActive = host.userActive();
            let aimed = host.aim();
            if (aimed !== null) host.write(aimed);
            let quiet = 0;
            const tick = (): void => {
                raf = 0;
                if (mine !== token) return;
                // Input that began after the jump ends it; one already open when the jump started cannot be told
                // apart from the person's last flick, so it is let be.
                if ((!wasActive && host.userActive()) || host.viewKey() !== key) return stop();
                const aim = host.aim();
                if (aim !== null && (aimed === null || Math.abs(aim - aimed) >= SAME_AIM_PX)) {
                    host.write(aim);
                    aimed = aim;
                    quiet = 0;
                } else if (aim !== null) {
                    quiet++;
                }
                if (quiet >= SETTLE_NODE_QUIET_FRAMES || host.now() - startedAt >= SETTLE_NODE_MAX_MS) return stop();
                raf = host.frame(tick);
            };
            raf = host.frame(tick);
        },
        cancel: stop,
        active: () => running,
    };
}
