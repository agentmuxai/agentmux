// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Inline media for the agent's own messages
 * (SPEC_AGENT_PANE_RICH_OUTPUT_2026_09_27.md §4.1): the pane provides the
 * agent's working directory, and `MarkdownBlock` turns it into `<Markdown
 * media>`. Context rather than a prop for the same reason as
 * `agent-dormancy.tsx`: the render path runs through performance-critical
 * virtualization code that shouldn't grow a prop it only forwards.
 *
 * Only `MarkdownBlock` reads this. Tool results and file previews render
 * `<Markdown>` without `media`, so content the agent didn't write can't make
 * the pane fetch anything.
 */

import type { MarkdownMediaOpts } from "@/app/element/markdown-media";
import { createContext, useContext, type Accessor, type JSX } from "solid-js";
import { AgentDormancyProvider } from "./agent-dormancy";
import { AgentPaneFilesProvider, type AgentPaneFiles } from "./agent-open-in-editor";
import { ToolResultLoaderProvider, type ToolResultLoader } from "./tool-result-loader";

/** Outside an agent pane: no inline media. */
const AgentMediaContext = createContext<Accessor<MarkdownMediaOpts | undefined>>(() => undefined);

export function AgentMediaProvider(props: { baseDir: Accessor<string>; children: JSX.Element }): JSX.Element {
    const media = () => ({ baseDir: props.baseDir() });
    return <AgentMediaContext.Provider value={media}>{props.children}</AgentMediaContext.Provider>;
}

export function useAgentMedia(): Accessor<MarkdownMediaOpts | undefined> {
    return useContext(AgentMediaContext);
}

/**
 * The agent pane's render-path contexts in one place: dormancy, the
 * working directory its own messages resolve images against — the actual
 * launch cwd (`cmd:cwd`) first, as the stash drawer does, else the
 * definition's `working_directory` — and, for a host agent, what "Open in
 * editor" needs (`agent-open-in-editor.tsx`).
 */
export function AgentPaneProviders(props: {
    dormant: Accessor<boolean>;
    block: Accessor<{ meta?: Record<string, unknown> } | null | undefined>;
    agent: Accessor<{ working_directory?: string; agent_type?: string } | null | undefined>;
    /** The pane's block id; without one, no "Open in editor". */
    blockId?: string;
    /** Reads an unloaded tool result back (tool-result-loader.tsx). */
    loadToolResult?: ToolResultLoader;
    children: JSX.Element;
}): JSX.Element {
    const baseDir = (): string => (props.block()?.meta?.["cmd:cwd"] as string) || props.agent()?.working_directory || "";
    // A container agent's paths are inside its container: nothing to open here.
    const files = (): AgentPaneFiles | undefined =>
        props.blockId && props.agent()?.agent_type !== "container"
            ? { blockId: props.blockId, baseDir: baseDir() }
            : undefined;
    return (
        <AgentDormancyProvider dormant={props.dormant}>
            <AgentMediaProvider baseDir={baseDir}>
                <AgentPaneFilesProvider files={files}>
                    {props.loadToolResult ? (
                        <ToolResultLoaderProvider load={props.loadToolResult}>{props.children}</ToolResultLoaderProvider>
                    ) : (
                        props.children
                    )}
                </AgentPaneFilesProvider>
            </AgentMediaProvider>
        </AgentDormancyProvider>
    );
}
