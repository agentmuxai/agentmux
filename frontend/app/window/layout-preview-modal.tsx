// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The preview shown before a layout file opens
// (docs/specs/SPEC_LAYOUT_FILES_2026_09_25.md §3.5): where it opens (a new
// window by default, or added to this one), what tabs and panes it has, what
// can't be reproduced here, and every terminal command in it — which run
// only if the user leaves "Run these commands" ticked. The box starts ticked
// only for a trusted file (saved by this install, in its layouts folder).

import { ConfirmModal } from "@/app/element/confirm-modal";
import type { LayoutPreviewResult } from "@/types/rpc/LayoutPreviewResult";
import { createSignal, For, Show, type JSX } from "solid-js";

export interface LayoutOpenChoice {
    runCommands: boolean;
    /** Open in a new window rather than adding tabs to this one. */
    newWindow: boolean;
}

export interface LayoutPreviewModalProps {
    preview: LayoutPreviewResult;
    /** Resolves when the layout has opened (or failed and been reported). */
    onOpen: (choice: LayoutOpenChoice) => Promise<void>;
    close: () => void;
}

export function LayoutPreviewModal(props: LayoutPreviewModalProps): JSX.Element {
    const [runCommands, setRunCommands] = createSignal(props.preview.trusted);
    const [newWindow, setNewWindow] = createSignal(true);
    const tabs = () => `${props.preview.tabs.length} ${props.preview.tabs.length === 1 ? "tab" : "tabs"}`;
    return (
        <ConfirmModal
            open
            title={`Open layout “${props.preview.name || "Layout"}”`}
            description={
                newWindow()
                    ? `Opens ${tabs()} in a new window.`
                    : `Adds ${tabs()} to this window. Nothing that's open is replaced.`
            }
            confirmLabel="Open"
            onConfirm={async () => {
                await props.onOpen({ runCommands: runCommands(), newWindow: newWindow() });
                props.close();
            }}
            onCancel={props.close}
        >
            <div class="layout-preview" data-testid="layout-preview">
                <section>
                    <label>
                        <input
                            type="radio"
                            name="layout-open-where"
                            data-testid="layout-preview-new-window"
                            checked={newWindow()}
                            onChange={() => setNewWindow(true)}
                        />{" "}
                        In a new window
                    </label>{" "}
                    <label>
                        <input
                            type="radio"
                            name="layout-open-where"
                            data-testid="layout-preview-this-window"
                            checked={!newWindow()}
                            onChange={() => setNewWindow(false)}
                        />{" "}
                        Add to this window
                    </label>
                </section>
                <For each={props.preview.tabs}>
                    {(tab) => (
                        <section>
                            <strong>{tab.name || "Tab"}</strong>
                            <ul>
                                <For each={tab.panes}>{(pane) => <li>{pane}</li>}</For>
                            </ul>
                        </section>
                    )}
                </For>
                <Show when={props.preview.notes.length > 0}>
                    <section data-testid="layout-preview-notes">
                        <strong>Can't be reproduced exactly here</strong>
                        <ul>
                            <For each={props.preview.notes}>{(note) => <li>{note}</li>}</For>
                        </ul>
                    </section>
                </Show>
                <Show when={props.preview.commands.length > 0}>
                    <section data-testid="layout-preview-commands">
                        <strong>Terminal commands</strong>
                        <Show when={!props.preview.trusted}>
                            <p>This file wasn't saved by this AgentMux. Check these commands before running them.</p>
                        </Show>
                        <ul>
                            <For each={props.preview.commands}>
                                {(cmd) => (
                                    <li>
                                        <code>{cmd}</code>
                                    </li>
                                )}
                            </For>
                        </ul>
                        <label>
                            <input
                                type="checkbox"
                                data-testid="layout-preview-run-commands"
                                checked={runCommands()}
                                onChange={(e) => setRunCommands(e.currentTarget.checked)}
                            />{" "}
                            Run these commands
                        </label>
                    </section>
                </Show>
            </div>
        </ConfirmModal>
    );
}
