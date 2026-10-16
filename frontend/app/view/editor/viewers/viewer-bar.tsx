// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The Editor's toolbar row for a non-text viewer: the viewer's own controls
 * first, then what every viewer offers (show the file as another viewer,
 * follow the folder, reveal it, open it with the system's app), and the
 * file's facts at the end. Only for the tab in front.
 * docs/specs/SPEC_EDITOR_REVAMP_UNIFIED_VIEWERS_2026_10_15.md §4.3.
 */

import { IconButton, SegmentedControl } from "@/app/element/ui";
import { For, Show, type JSX } from "solid-js";
import { VIEWER_LABELS, type ViewerId } from "./viewer-choice";

export interface ViewerBarProps {
    /** The viewer's own controls (zoom, background…). */
    start?: JSX.Element;
    /** Facts about the file: dimensions, size, type. */
    facts?: string[];
    viewer: ViewerId;
    /** The viewers this file can be shown in; a switch when there are two or more. */
    options: ViewerId[];
    onReopenWith: (viewer: ViewerId) => void;
    /** Set for a viewer that can follow its folder. */
    followFolder?: { on: boolean; toggle: () => void };
    /** Absent for dropped bytes, which have no file on disk. */
    onReveal?: () => void;
    onOpenWithSystem?: () => void;
}

export function ViewerBar(props: ViewerBarProps): JSX.Element {
    return (
        <div class="editor-mode-toolbar editor-viewer-bar" role="toolbar" aria-label="Viewer">
            <Show when={props.start}>
                <div class="editor-viewer-bar-group">{props.start}</div>
            </Show>
            <Show when={props.options.length > 1}>
                <SegmentedControl<ViewerId>
                    density="compact"
                    ariaLabel="Show the file as"
                    options={props.options.map((id) => ({ value: id, label: VIEWER_LABELS[id].label, title: `Show as ${VIEWER_LABELS[id].label.toLowerCase()}` }))}
                    value={props.viewer}
                    onChange={(v) => props.onReopenWith(v)}
                />
            </Show>
            <div class="editor-viewer-bar-group">
                <Show when={props.followFolder}>
                    {(f) => (
                        <IconButton
                            icon="arrows-rotate"
                            density="compact"
                            pressed={f().on}
                            label={f().on ? "Following newer files in this folder (click to stop)" : "Follow newer files in this folder"}
                            onClick={() => f().toggle()}
                        />
                    )}
                </Show>
                <Show when={props.onReveal}>
                    {(reveal) => <IconButton icon="folder-open" density="compact" label="Reveal in folder" onClick={() => reveal()()} />}
                </Show>
                <Show when={props.onOpenWithSystem}>
                    {(open) => <IconButton icon="arrow-up-right-from-square" density="compact" label="Open with the system's app" onClick={() => open()()} />}
                </Show>
            </div>
            <Show when={props.facts?.length}>
                <div class="editor-viewer-bar-facts" role="status" aria-live="polite">
                    <For each={props.facts}>{(f) => <span>{f}</span>}</For>
                </div>
            </Show>
        </div>
    );
}
