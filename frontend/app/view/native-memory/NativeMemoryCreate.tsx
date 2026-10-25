// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * New file and Create MEMORY.md for one agent's Personal memory: the create
 * actions the agent pane's memory view had, now where every agent's memory is
 * browsed.
 */

import { Button, Field, TextInput } from "@/app/element/ui";
import { createSignal, Show, type JSX } from "solid-js";
import { createMemoryFile, MEMORY_MD_TEMPLATE } from "./memory-file-create";

interface NativeMemoryCreateProps {
    agentId: string;
    /** Filenames the folder holds now; a create refuses to overwrite one. */
    filenames: readonly string[];
    /** Whether the folder has its MEMORY.md index, which Create MEMORY.md would add. */
    hasIndex: boolean;
    /** srv refuses a write into a folder found by a guess; the actions wait for the agent's launch. */
    readOnly: boolean;
    /** The folder's file list isn't known (loading, or it failed): a name can't be checked against
     *  what is there, and a write replaces a file of the same name. */
    listUnknown: boolean;
    /** The file is written: the caller refreshes its list and opens it. */
    onCreated: (filename: string, agentId: string) => void;
}

const LIST_UNKNOWN_TITLE = "Wait until this agent's files have loaded.";
const READ_ONLY_TITLE = "This agent's memory folder isn't confirmed yet. You can add files after it launches.";

export function NativeMemoryCreate(props: NativeMemoryCreateProps): JSX.Element {
    const [naming, setNaming] = createSignal(false);
    const [name, setName] = createSignal("");
    const [busy, setBusy] = createSignal(false);
    const [error, setError] = createSignal<string | null>(null);

    const startNaming = () => {
        setName("");
        setError(null);
        setNaming(true);
    };
    const stopNaming = () => {
        setNaming(false);
        setError(null);
    };

    const create = async (rawFilename: string, content: string): Promise<boolean> => {
        if (props.listUnknown) {
            setError(LIST_UNKNOWN_TITLE);
            return false;
        }
        setBusy(true);
        setError(null);
        const agentId = props.agentId;
        const created = await createMemoryFile(agentId, rawFilename, content, props.filenames);
        setBusy(false);
        if ("error" in created) {
            setError(created.error);
            return false;
        }
        props.onCreated(created.filename, agentId);
        return true;
    };

    const submitName = async () => {
        if (await create(name(), "")) setNaming(false);
    };

    return (
        <div class="native-memory-create">
            <Show
                when={naming()}
                fallback={
                    <div class="native-memory-create-actions">
                        <Button
                            density="compact"
                            icon="plus"
                            disabled={props.readOnly || props.listUnknown || busy()}
                            title={props.readOnly ? READ_ONLY_TITLE : props.listUnknown ? LIST_UNKNOWN_TITLE : undefined}
                            onClick={startNaming}
                        >
                            New file
                        </Button>
                        <Show when={!props.hasIndex}>
                            <Button
                                density="compact"
                                tone="accent"
                                icon="file-lines"
                                busy={busy()}
                                disabled={props.readOnly}
                                title={props.readOnly ? READ_ONLY_TITLE : "Starts the index Claude loads every session"}
                                onClick={() => void create("MEMORY.md", MEMORY_MD_TEMPLATE)}
                            >
                                Create MEMORY.md
                            </Button>
                        </Show>
                    </div>
                }
            >
                <Field
                    label="File name"
                    layout="stacked"
                    hint="Letters, numbers, - and _; .md is added"
                    error={error() ?? undefined}
                >
                    <TextInput
                        density="compact"
                        placeholder="filename.md"
                        autofocus
                        value={name()}
                        onInput={(e) => {
                            setName(e.currentTarget.value);
                            setError(null);
                        }}
                        onKeyDown={(e) => {
                            if (e.key === "Enter") {
                                e.preventDefault();
                                void submitName();
                            } else if (e.key === "Escape") {
                                e.preventDefault();
                                stopNaming();
                            }
                        }}
                    />
                </Field>
                <div class="native-memory-create-actions">
                    <Button density="compact" onClick={stopNaming}>
                        Cancel
                    </Button>
                    <Button density="compact" tone="accent" busy={busy()} onClick={() => void submitName()}>
                        Create
                    </Button>
                </div>
            </Show>
            <Show when={error() && !naming()}>
                <div class="native-memory-create-error" role="alert">
                    {error()}
                </div>
            </Show>
        </div>
    );
}

NativeMemoryCreate.displayName = "NativeMemoryCreate";
