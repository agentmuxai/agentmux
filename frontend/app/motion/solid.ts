// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The motion primitives bound to a Solid component: every motion started through
// useMotion() is cancelled when the component's owner is cleaned up, so an element that
// unmounts mid-motion never leaves a copy on screen or an original hidden
// (docs/specs/SPEC_MOTION_PRIMITIVES_2026_10_29.md §3.5).

import { onCleanup } from "solid-js";
import type { MotionHandle } from "./primitives";
import * as motion from "./primitives";

/** Call inside a component or an owner (createRoot, an effect). */
export function useMotion() {
    const live = new Set<MotionHandle>();
    onCleanup(() => {
        for (const handle of [...live]) handle.cancel();
        live.clear();
    });
    /** Track `handle` until it ends, and give it back. */
    const track = (handle: MotionHandle): MotionHandle => {
        live.add(handle);
        void handle.finished.then(() => live.delete(handle));
        return handle;
    };
    return {
        play: (...args: Parameters<typeof motion.play>) => track(motion.play(...args)),
        move: (...args: Parameters<typeof motion.move>) => track(motion.move(...args)),
        travel: (...args: Parameters<typeof motion.travel>) => track(motion.travel(...args)),
        mergeInto: (...args: Parameters<typeof motion.mergeInto>) => track(motion.mergeInto(...args)),
        enter: (...args: Parameters<typeof motion.enter>) => track(motion.enter(...args)),
        exit: (...args: Parameters<typeof motion.exit>) => track(motion.exit(...args)),
        pulse: (...args: Parameters<typeof motion.pulse>) => track(motion.pulse(...args)),
        glide: (...args: Parameters<typeof motion.glide>) => track(motion.glide(...args)),
        capture: motion.capture,
    };
}
