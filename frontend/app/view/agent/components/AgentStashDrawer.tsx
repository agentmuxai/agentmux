// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Split out of agent-view.tsx (SPEC_LARGE_FILE_MODULE_ANALYSIS_2026_09_30.md §3.5 step 6).

import { Show, type JSX } from "solid-js";
import { AgentStashModal } from "./AgentStashModal";
import { ResizableDetailsDrawer } from "./ResizableDetailsDrawer";

/**
 * Stash drawer — top-anchored, directly under the pane header
 * where its own backpack toggle lives
 * (SPEC_AGENT_STASH_PANE_MIGRATION_2026_09_22.md §3.1).
 * Replaced the former `agent-stash` MODAL; the header icon
 * (agent-model.ts's endIconButtons) drives `stashOpen` through
 * the three callbacks wired in onMount above.
 *
 * Deliberately OUTSIDE `.agent-view-zoomed`, exactly like the
 * Shell drawer below, for two reasons beyond symmetry: the
 * drag-to-resize math reads `ev.clientY` (visual px) and writes
 * a `height` (layout px), which CSS `zoom` makes disagree —
 * the same coordinate-space trap
 * SPEC_AGENT_SHELL_DRAWER_ZOOM_COORDINATE_SPACE_2026_09_20.md
 * records for the shell — and §3.2a's composer-scale density
 * values are already tuned small, so compounding them with a
 * per-pane zoom would read as either unusable or enormous
 * rather than merely scaled. It stays a flex child of
 * `.agent-view` so the transcript below still shrinks to make
 * room for it.
 */
export const AgentStashDrawer = (props: {
    open: boolean;
    blockId: string;
    persistedHeight: number | undefined;
    agentId: string;
    agentName: string;
    /** Prefer cmd:cwd (the actual launch cwd) over AgentDefinition.working_directory. */
    workingDirectory: string;
    /** The pane has a loadable agent definition (not a quick-launch pane). */
    hasDefinition: boolean;
}): JSX.Element => (
    <Show when={props.open}>
        {/* Rendered as a DIRECT flex child of `.agent-view`, with no
            wrapper div, and that placement is load-bearing rather
            than incidental (reagentx P1 on PR #3540). The 50% height
            cap lives on `.agent-stash-drawer-resizable` — the same
            element that holds BOTH the content body and the resize
            handle — and a percentage `max-height` only resolves
            against a containing block whose height is definite.
            `.agent-view` is `height: 100%` (agent-view.scss), so it
            qualifies; an intermediate auto-height wrapper would NOT,
            and the percentage would compute to `none`. The first cut
            had exactly that wrapper, which let the inner element
            render at its full dragged height while the wrapper
            clipped it — carrying the bottom-edge handle into the
            clipped-away region, where it was invisible and
            unreachable, so a drawer dragged past 50% could never be
            shrunk again. See _stash-drawer.scss for the flex
            compression that keeps the handle on screen instead. */}
        <ResizableDetailsDrawer
            blockId={props.blockId}
            anchor="top"
            classPrefix="agent-stash-drawer"
            persistMetaKey="agent:stashheight"
            persistedHeight={props.persistedHeight}
        >
            <AgentStashModal
                agentId={props.agentId}
                agentName={props.agentName}
                // Prefer cmd:cwd (the actual launch cwd, set by
                // launchAgentDefinition) over
                // AgentDefinition.working_directory, which is often
                // empty or a stale default for template-launched and
                // continuation agents.
                workingDirectory={props.workingDirectory}
                // No loadable definition (quick-launch pane) → default
                // to the Memory tab; the Accounts tab works from
                // agentId alone but Memory is the more useful default
                // for a pane with no saved definition yet.
                initialTab={props.hasDefinition ? "accounts" : "memory"}
                density="compact"
                // No `onClose` — closing is the header icon's job, so
                // the Memory tab hides its footer Close button rather
                // than rendering a dead one (§3.4).
            />
        </ResizableDetailsDrawer>
    </Show>
);
