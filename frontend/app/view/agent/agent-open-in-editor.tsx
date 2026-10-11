// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * "Open in editor" for a file the agent read or wrote
 * (SPEC_AGENT_PANE_TOOL_PEEK_ON_LINE_AND_MD_OPEN_IN_EDITOR_2026_10_11.md §2):
 * the agent pane provides its block id, so the editor opens split to its
 * right, and its working directory, which a relative path resolves against.
 * Context rather than a prop, as in `agent-media.tsx`: the render path runs
 * through the virtualized list, which shouldn't grow a prop it only forwards.
 *
 * A container agent's paths are inside its container, so its pane provides
 * nothing and no button shows.
 */

import { IconButton } from "@/app/element/ui";
import { TabRpcClient } from "@/app/store/rpc-util";
import { createContext, Show, useContext, type Accessor, type JSX } from "solid-js";

export interface AgentPaneFiles {
    /** The agent pane's block: the editor opens to its right. */
    blockId: string;
    /** What a relative path resolves against. */
    baseDir: string;
}

/** Outside an agent pane, or for a container agent: nothing to open. */
const AgentPaneFilesContext = createContext<Accessor<AgentPaneFiles | undefined>>(() => undefined);

export function AgentPaneFilesProvider(props: {
    files: Accessor<AgentPaneFiles | undefined>;
    children: JSX.Element;
}): JSX.Element {
    return <AgentPaneFilesContext.Provider value={props.files}>{props.children}</AgentPaneFilesContext.Provider>;
}

export function useAgentPaneFiles(): Accessor<AgentPaneFiles | undefined> {
    return useContext(AgentPaneFilesContext);
}

/** `path` as is when absolute (POSIX, `~`, a Windows drive or UNC path), else under `baseDir`. */
export function resolveAgentPath(path: string, baseDir: string): string {
    if (!baseDir || /^(\/|~|[A-Za-z]:[\\/]|\\\\)/.test(path)) return path;
    const sep = baseDir.includes("\\") && !baseDir.includes("/") ? "\\" : "/";
    return baseDir.replace(/[\\/]+$/, "") + sep + path;
}

/** An editor pane on `path`, split right of the agent pane, its file tree collapsed. */
export async function openInEditorBeside(files: AgentPaneFiles, path: string): Promise<void> {
    await TabRpcClient.rpcCall(
        "pane.open",
        {
            view: "editor",
            file: resolveAgentPath(path, files.baseDir),
            split_direction: "right",
            split_reference_block_id: files.blockId,
            tree_expanded: false,
        },
        {}
    );
}

/** The button, for a file preview's header row; nothing outside a host agent's pane. */
export function OpenInEditorButton(props: { path: string }): JSX.Element {
    const files = useAgentPaneFiles();
    return (
        <Show when={files() && props.path ? files() : undefined}>
            {(f) => (
                <IconButton
                    class="agent-tool-open-in-editor"
                    icon="file-lines"
                    label="Open in editor"
                    density="compact"
                    onClick={(e: MouseEvent) => {
                        // The row sits in the tool's panel; a click here isn't a click on the tool.
                        e.stopPropagation();
                        openInEditorBeside(f(), props.path).catch((err) =>
                            console.warn("[agent] couldn't open the file in an editor", err)
                        );
                    }}
                />
            )}
        </Show>
    );
}
