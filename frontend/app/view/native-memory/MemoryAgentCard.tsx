// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * MemoryAgentCard — one agent in the Armory → Memory → Personal grid.
 *
 * Deliberately NOT `AgentCard` (view/agent/components/AgentCard.tsx), despite
 * mirroring its visual language. That component's contract is "launch this
 * agent": its props are `launching`/`disabled`/`installed`/`onLaunch` plus
 * Option E's session-zone "+ New" button, and it fetches install state. Wiring
 * it to "browse this agent's memories" would drag launch semantics into a
 * read-only browser and let a future AgentCard change silently alter this tab.
 * This card carries only what Personal Memory needs. See
 * docs/specs/SPEC_ARMORY_PERSONAL_MEMORY_AGENT_BLOCKS_2026_09_01.md.
 *
 * The `count` prop has FOUR states, and the last two must not be collapsed:
 * loading, a real number, zero, and error. `agent:memory:list` fails with a
 * hard HTTP 500 (not an empty list) when the memory dir can't be resolved —
 * which is exactly how every blank-`working_directory` agent failed before
 * SPEC_FIX_PERSONAL_MEMORY_EMPTY_WORKDIR_2026_09_01.md (#2901). Rendering
 * that as "no memories yet" would hide the next occurrence of that bug class
 * behind a plausible-looking empty state.
 */

import { Show, createMemo, type JSX } from "solid-js";
import { ProviderLogo } from "@/element/ProviderLogo";

/** Per-agent memory-count fetch state. `error` is distinct from `count: 0`. */
export type MemoryCountState =
    | { kind: "loading" }
    | { kind: "count"; files: number }
    | { kind: "error"; message: string };

interface MemoryAgentCardProps {
    agent: AgentDefinition;
    count: MemoryCountState;
    /** Opens this agent's memory detail view. */
    onSelect: (agent: AgentDefinition) => void;
}

/** The count line's text — pulled out so the render and its aria-label can't
 *  drift, and so the four-state mapping is unit-testable on its own. */
export function memoryCountLabel(count: MemoryCountState): string {
    switch (count.kind) {
        case "loading":
            return "Loading…";
        case "error":
            return "Couldn't read memories";
        case "count":
            if (count.files === 0) return "No memories yet";
            return count.files === 1 ? "1 file" : `${count.files} files`;
    }
}

export const MemoryAgentCard = (props: MemoryAgentCardProps): JSX.Element => {
    const name = createMemo(() => props.agent.name || props.agent.slug || props.agent.id);

    const select = () => props.onSelect(props.agent);

    const handleKeyDown = (e: KeyboardEvent) => {
        if (e.key === "Enter" || e.key === " ") {
            e.preventDefault();
            select();
        }
    };

    return (
        <div
            class="memory-agent-card"
            classList={{ "memory-agent-card--error": props.count.kind === "error" }}
            role="button"
            tabIndex={0}
            onClick={select}
            onKeyDown={handleKeyDown}
            // An errored card stays clickable on purpose: the detail view
            // surfaces the real error text, which is more useful than a dead
            // card that only says something went wrong.
            aria-label={`${name()} — ${memoryCountLabel(props.count)}`}
            title={props.count.kind === "error" ? props.count.message : undefined}
        >
            <ProviderLogo provider={props.agent.provider} size={28} class="memory-agent-card-icon" />
            <span class="memory-agent-card-info">
                <span class="memory-agent-card-title">{name()}</span>
                <span
                    class="memory-agent-card-count"
                    classList={{ "memory-agent-card-count--error": props.count.kind === "error" }}
                >
                    <Show when={props.count.kind === "error"}>
                        <i class="fa-sharp fa-solid fa-triangle-exclamation" aria-hidden="true" />{" "}
                    </Show>
                    {memoryCountLabel(props.count)}
                </span>
            </span>
        </div>
    );
};

MemoryAgentCard.displayName = "MemoryAgentCard";
