// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The dialogs of ☰ → Workspaces (SPEC_WORKSPACES_2026_10_23.md §3.2): a name
// for a new or renamed workspace, a list to choose one from, the delete
// confirmation, and what an import opens before it does.

import { ConfirmModal } from "@/app/element/confirm-modal";
import { Button, Field, TextInput } from "@/app/element/ui";
import { Modal, ModalBody, ModalFooter, ModalHeader } from "@/element/modal";
import type { ModalCloseProps } from "@/app/store/modalmodel";
import type { UserWorkspaceInfo, WorkspaceImportPreview } from "@/app/store/services";
import { WORKSPACE_NAME_MAX_LEN, workspaceWindowsLine } from "@/app/store/user-workspace";
import { createSignal, For, Show, type JSX } from "solid-js";
import "./workspace-modals.scss";

/** Asks for a workspace's name: New Workspace… and Rename workspace…. */
export function WorkspaceNameModal(
    props: {
        title: string;
        initial: string;
        confirmLabel: string;
        description?: string;
        /** Called with the trimmed name; the dialog closes first. */
        onSubmit: (name: string) => void | Promise<void>;
    } & ModalCloseProps
): JSX.Element {
    const [name, setName] = createSignal(props.initial);
    const trimmed = () => name().trim();
    const submit = () => {
        if (!trimmed()) return;
        props.close();
        void props.onSubmit(trimmed());
    };
    return (
        <Modal open={true} onClose={props.close} size="sm" ariaLabel={props.title}>
            <ModalHeader title={props.title} description={props.description} />
            <ModalBody>
                <Field label="Name">
                    <TextInput
                        value={name()}
                        autofocus
                        maxLength={WORKSPACE_NAME_MAX_LEN}
                        data-testid="workspace-name-input"
                        ref={(el) => queueMicrotask(() => el.select())}
                        onInput={(e) => setName(e.currentTarget.value)}
                        onKeyDown={(e) => {
                            if (e.key === "Enter") {
                                e.preventDefault();
                                submit();
                            }
                        }}
                    />
                </Field>
            </ModalBody>
            <ModalFooter>
                <Button data-modal-dismiss onClick={props.close}>
                    Cancel
                </Button>
                <Button tone="accent" disabled={!trimmed()} onClick={submit}>
                    {props.confirmLabel}
                </Button>
            </ModalFooter>
        </Modal>
    );
}

/** A list of workspaces to choose one from: Open Workspace… in the palette,
 *  and Delete workspace…. The open one is marked and can't be chosen. */
export function WorkspacePickerModal(
    props: {
        title: string;
        description?: string;
        workspaces: UserWorkspaceInfo[];
        onPick: (w: UserWorkspaceInfo) => void;
    } & ModalCloseProps
): JSX.Element {
    return (
        <Modal open={true} onClose={props.close} size="sm" ariaLabel={props.title}>
            <ModalHeader title={props.title} description={props.description} />
            <ModalBody>
                <div class="workspace-picker" role="list">
                    <For each={props.workspaces}>
                        {(w) => (
                            <Button
                                class="workspace-picker-item"
                                role="listitem"
                                data-testid="workspace-picker-item"
                                disabled={w.current}
                                title={w.current ? "The open workspace" : undefined}
                                onClick={() => {
                                    props.close();
                                    props.onPick(w);
                                }}
                            >
                                <span class="workspace-picker-name">{w.name}</span>
                                <span class="workspace-picker-detail">
                                    {w.current ? "open" : workspaceWindowsLine(w)}
                                </span>
                            </Button>
                        )}
                    </For>
                </div>
            </ModalBody>
            <ModalFooter>
                <Button data-modal-dismiss onClick={props.close}>
                    Cancel
                </Button>
            </ModalFooter>
        </Modal>
    );
}

/** What deleting a workspace removes. */
export function deleteWorkspaceText(w: UserWorkspaceInfo): { title: string; description: string } {
    const windows = w.windows + w.closed;
    return {
        title: `Delete “${w.name}”?`,
        description:
            `Its ${windows} window${windows === 1 ? "" : "s"} and their tabs and panes are deleted. ` +
            `Its agents stay in My Agents, with their conversations.`,
    };
}

/** Confirms deleting a workspace. */
export function DeleteWorkspaceConfirm(
    props: { workspace: UserWorkspaceInfo; onConfirm: () => Promise<void> } & ModalCloseProps
): JSX.Element {
    return (
        <ConfirmModal
            open={true}
            {...deleteWorkspaceText(props.workspace)}
            confirmLabel="Delete workspace"
            destructive
            onConfirm={async () => {
                props.close();
                await props.onConfirm();
            }}
            onCancel={props.close}
        />
    );
}

const count = (n: number, one: string): string => `${n} ${one}${n === 1 ? "" : "s"}`;

/** What Import… is about to open, before it opens it: the windows and tabs,
 *  what can't be reproduced here, and every terminal command, which runs only
 *  if "Run these commands" stays ticked (ticked only for a file this
 *  AgentMux saved). `current`: the open workspace, which closes first. */
export function ImportPreviewModal(
    props: {
        preview: WorkspaceImportPreview;
        current: string;
        onImport: (runCommands: boolean) => void | Promise<void>;
    } & ModalCloseProps
): JSX.Element {
    const [runCommands, setRunCommands] = createSignal(props.preview.trusted);
    const windows = () => {
        const { windows, closed } = props.preview;
        return closed > 0 ? `${count(windows, "window")} (${closed} closed)` : count(windows, "window");
    };
    return (
        <ConfirmModal
            open
            title={`Import “${props.preview.name}”`}
            description={`Opens ${windows()} and ${count(props.preview.tabs.length, "tab")} as a new workspace. ${props.current} closes first; it keeps its windows.`}
            confirmLabel="Import"
            onConfirm={() => {
                props.close();
                void props.onImport(runCommands());
            }}
            onCancel={props.close}
        >
            <div class="workspace-import-preview" data-testid="import-preview">
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
                    <section data-testid="import-preview-notes">
                        <strong>Can't be reproduced exactly here</strong>
                        <ul>
                            <For each={props.preview.notes}>{(note) => <li>{note}</li>}</For>
                        </ul>
                    </section>
                </Show>
                <Show when={props.preview.commands.length > 0}>
                    <section data-testid="import-preview-commands">
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
                                data-testid="import-preview-run-commands"
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
