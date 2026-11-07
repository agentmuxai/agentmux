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

import { openMediaInEditor, type MarkdownMediaOpts, type MediaGallery } from "@/app/element/markdown-media";
import { createContext, createSignal, Show, useContext, type Accessor, type JSX } from "solid-js";
import { MessageImageLightbox } from "./components/MessageImageLightbox";
import { AgentDormancyProvider } from "./agent-dormancy";
import { AgentPaneFilesProvider, type AgentPaneFiles } from "./agent-open-in-editor";
import { ToolResultLoaderProvider, type ToolResultLoader } from "./tool-result-loader";
import { isContainerAgentType, isContainerRuntime, parseRuntimeKind } from "@/util/runtime-kind";

/** Outside an agent pane: no inline media. */
const AgentMediaContext = createContext<Accessor<MarkdownMediaOpts | undefined>>(() => undefined);

export function AgentMediaProvider(props: { baseDir: Accessor<string>; children: JSX.Element }): JSX.Element {
    // The preview that steps through one message's images: one per pane, opened by an image's click.
    const [gallery, setGallery] = createSignal<MediaGallery>();
    const media = (): MarkdownMediaOpts => ({ baseDir: props.baseDir(), openGallery: setGallery });
    return (
        <AgentMediaContext.Provider value={media}>
            {props.children}
            <Show when={gallery()}>
                {(g) => (
                    <MessageImageLightbox
                        items={g().items}
                        index={g().index}
                        onClose={() => setGallery(undefined)}
                        onOpenEditor={(path) => {
                            setGallery(undefined);
                            openMediaInEditor(path);
                        }}
                    />
                )}
            </Show>
        </AgentMediaContext.Provider>
    );
}

export function useAgentMedia(): Accessor<MarkdownMediaOpts | undefined> {
    return useContext(AgentMediaContext);
}

/**
 * The agent pane's render-path contexts in one place: dormancy, the
 * working directory its own messages resolve images against — the actual
 * launch cwd (`cmd:cwd`) first, else the
 * definition's `working_directory` — and, for a host agent, what "Open in
 * editor" needs (`agent-open-in-editor.tsx`).
 */
export function AgentPaneProviders(props: {
    dormant: Accessor<boolean>;
    /** The pane's block: its `oid` is where "Open in editor" splits from. */
    block: Accessor<{ oid?: string; meta?: Record<string, unknown> } | null | undefined>;
    agent: Accessor<{ working_directory?: string; agent_type?: string } | null | undefined>;
    /** Reads an unloaded tool result back (tool-result-loader.tsx). */
    loadToolResult?: ToolResultLoader;
    children: JSX.Element;
}): JSX.Element {
    const baseDir = (): string => (props.block()?.meta?.["cmd:cwd"] as string) || props.agent()?.working_directory || "";
    // Only for an agent known to run on this host: a container agent's paths
    // are inside its container. The pane's own `agentMode` says which at once;
    // its definition, which may still be loading (or gone), only when known.
    const onHost = (): boolean => {
        const mode = props.block()?.meta?.["agentMode"];
        const kind = parseRuntimeKind(mode);
        if (kind) return !isContainerRuntime(kind);
        const type = props.agent()?.agent_type;
        return type != null && !isContainerAgentType(type);
    };
    const files = (): AgentPaneFiles | undefined => {
        const blockId = props.block()?.oid;
        return blockId && onHost() ? { blockId, baseDir: baseDir() } : undefined;
    };
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
