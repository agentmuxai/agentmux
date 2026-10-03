// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Split out of agent-view.tsx (SPEC_LARGE_FILE_MODULE_ANALYSIS_2026_09_30.md §3.5 step 6).

import { Show, type JSX } from "solid-js";
import { AgentShellInfoPanel } from "./AgentShellInfoPanel";
import { AgentShellSubblock } from "./AgentShellSubblock";
import { ResizableDetailsDrawer } from "./ResizableDetailsDrawer";
import { setBlockMeta } from "@/app/store/block-meta";

// Shell drawer's height until the user drags it (then `term:shellheight`
// wins). 80% of the drawers' shared 220px default — the shell opens on its
// own for every `!cmd`, so it should take less of the transcript by default.
const SHELL_DRAWER_DEFAULT_HEIGHT = 176;

/**
 * Details panel — just the shell + control bar now. Activity-log
 * lines write directly into the terminal (handleShellTermReady)
 * instead of a separate panel here. Docked BELOW the composer
 * (SPEC_AGENT_SHELL_BELOW_COMPOSER_2026_08_08.md): the shell
 * stacks under the text input (which shifts up to make room,
 * since this region hugs the pane bottom).
 *
 * Deliberately OUTSIDE `.agent-view-zoomed` — see the note on the root
 * element. The terminal has to render at a 1:1 device-pixel ratio, so it
 * must not be inside the per-pane `zoom`. It stays a flex child of
 * `.agent-view` so the composer still shifts up to make room for it.
 * Same move `agent-view.scss:350` records for the progress bar, and the
 * same cure as SPEC_STATUS_BAR_POPOVER_DOUBLE_ZOOM_OFFSET_2026_08_22.md.
 */
export const AgentShellDrawer = (props: {
    open: boolean;
    blockId: string;
    shellSubBlockId: string | undefined;
    cwd: string | undefined;
    persistedHeight: number | undefined;
    onTermReady: (write: (text: string) => void) => void;
    onTermDispose: () => void;
    onShellExited: () => void;
}): JSX.Element => (
    <Show when={props.open}>
        <div class="agent-composer-details" id={`agent-composer-details-${props.blockId}`}>
            {/* One line: what this shell is, and what the agent
                    has left running. Takes the slot AgentControlBar
                    used to occupy with session UI — see
                    SPEC_AGENT_SHELL_DRAWER_INFO_PANEL_2026_09_19.md §4. */}
            <AgentShellInfoPanel blockId={props.blockId} shellSubBlockId={props.shellSubBlockId} cwd={props.cwd} />
            {/* Drag-to-height drawer wrapping the terminal — the actual
                    scrollable/resizable content. */}
            <ResizableDetailsDrawer
                blockId={props.blockId}
                persistedHeight={props.persistedHeight}
                defaultHeight={SHELL_DRAWER_DEFAULT_HEIGHT}
            >
                {/* Phase 0 spike (#1945):
                        real xterm+PTY terminal, spawned lazily on first
                        drawer open via a headless term sub-block. */}
                <AgentShellSubblock
                    parentBlockId={props.blockId}
                    cwd={props.cwd ?? ""}
                    existingSubBlockId={props.shellSubBlockId}
                    // No `agentPaneZoom` prop any more. The shell used to
                    // divide the pane's zoom out of its own font-size math
                    // to fake independence; now it genuinely IS independent,
                    // because it renders outside `.agent-view-zoomed`.
                    onSubBlockCreated={(subBlockId) => {
                        void setBlockMeta(props.blockId, { "term:shellsubblockid": subBlockId } as MetaType);
                    }}
                    onTermReady={props.onTermReady}
                    onTermDispose={props.onTermDispose}
                    onShellExited={props.onShellExited}
                />
            </ResizableDetailsDrawer>
        </div>
    </Show>
);
