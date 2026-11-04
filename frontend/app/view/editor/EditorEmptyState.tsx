// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { Show, type JSX } from "solid-js";
import { EmptyPaneLogo } from "@/app/element/EmptyPaneLogo";
import { Button } from "@/app/element/ui";

/**
 * Empty-editor state — faded brain mark + the best few shortcuts. Shown in the main column whenever no
 * file is open (replaces the old path input; the file tree is the way to open files).
 */
export function EditorEmptyState(props: {
    shortcuts: { keys: string[]; label: string }[];
    onOpenFromRemote: () => void;
    /** Only where the file tree button is: an editor on a remote host has no tree. */
    showTreeHint: () => boolean;
}): JSX.Element {
    return (
        <div class="editor-empty">
            <EmptyPaneLogo />
            <div class="editor-empty-shortcuts">
                {props.shortcuts.map((s) => (
                    <div class="editor-empty-shortcut">
                        <span class="editor-empty-keys">
                            {s.keys.map((k) => (
                                <kbd>{k}</kbd>
                            ))}
                        </span>
                        <span class="editor-empty-label">{s.label}</span>
                    </div>
                ))}
            </div>
            <div class="editor-empty-actions">
                <Button density="compact" icon="server" onClick={props.onOpenFromRemote}>
                    Open from remote…
                </Button>
            </div>
            <Show when={props.showTreeHint()}>
                <div class="editor-empty-hint">Show the file tree (the folder button above) to browse files.</div>
            </Show>
        </div>
    );
}
