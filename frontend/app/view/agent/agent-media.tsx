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
 * The agent pane's render-path contexts in one place: dormancy, and the
 * working directory its own messages resolve images against — the actual
 * launch cwd (`cmd:cwd`) first, as the stash drawer does, else the
 * definition's `working_directory`.
 */
export function AgentPaneProviders(props: {
    dormant: Accessor<boolean>;
    block: Accessor<{ meta?: Record<string, unknown> } | null | undefined>;
    agent: Accessor<{ working_directory?: string } | null | undefined>;
    /** Reads an unloaded tool result back (tool-result-loader.tsx). */
    loadToolResult?: ToolResultLoader;
    children: JSX.Element;
}): JSX.Element {
    const baseDir = (): string => (props.block()?.meta?.["cmd:cwd"] as string) || props.agent()?.working_directory || "";
    return (
        <AgentDormancyProvider dormant={props.dormant}>
            <AgentMediaProvider baseDir={baseDir}>
                {props.loadToolResult ? (
                    <ToolResultLoaderProvider load={props.loadToolResult}>{props.children}</ToolResultLoaderProvider>
                ) : (
                    props.children
                )}
            </AgentMediaProvider>
        </AgentDormancyProvider>
    );
}
