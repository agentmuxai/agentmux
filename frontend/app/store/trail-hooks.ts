// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// How the code that moves the person (a link funnel, a reveal) tells the window's trail of places about it
// without importing it (pane-history.ts pulls in the layout and the tabs). The window installs the real runners
// at start; until then, and in any test that has not, they run the work as it is and add nothing in front of it.
// SPEC_BACK_FORWARD_FOLLOW_THE_WHOLE_TRAIL_2026_11_26.md §3.

type Runner = <T>(run: () => Promise<T> | T) => Promise<T>;

/** Runs `run` and answers its own promise: no tick is added. */
const asItIs: Runner = (run) => Promise.resolve(run());

let gesture: Runner = asItIs;
let quiet: Runner = asItIs;

/** Run what a person's click does as one step of the trail. */
export const asGesture: Runner = (run) => gesture(run);

/** Run what an agent or the host does to the person's selection without a step of the trail. */
export const withoutTrail: Runner = (run) => quiet(run);

/** The window's trail takes over; the returned function puts the plain runners back. */
export function installTrailRunners(runGesture: Runner, withoutHistory: Runner): () => void {
    gesture = runGesture;
    quiet = withoutHistory;
    return () => {
        gesture = asItIs;
        quiet = asItIs;
    };
}
