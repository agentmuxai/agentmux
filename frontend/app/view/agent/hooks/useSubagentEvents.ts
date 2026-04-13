// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * useSubagentEvents — subscribes to subagent:spawned / subagent:completed
 * wave events and maintains the corresponding SubagentLinkNode entries
 * in the document.
 *
 * Step 12 of specs/SPEC_AGENT_VIEW_MODULARIZATION_2026_04_13.md.
 *
 * On `subagent:spawned`: appends a new subagent_link node to the
 * document with status "active".
 *
 * On `subagent:completed`: flips the status of the matching node from
 * "active" to "completed".
 */

import { onCleanup, onMount } from "solid-js";
import { waveEventSubscribe } from "@/app/store/wps";
import type { SignalPair } from "../state";
import type { DocumentNode, SubagentLinkNode } from "../types";
import type { LogFn } from "./useAgentControllerStatus";

export interface UseSubagentEventsOptions {
    documentAtom: SignalPair<DocumentNode[]>;
    log: LogFn;
}

export function useSubagentEvents(opts: UseSubagentEventsOptions): void {
    const [, setDoc] = opts.documentAtom;

    onMount(() => {
        const unsubSpawned = waveEventSubscribe({
            eventType: "subagent:spawned",
            handler: (event: WaveEvent) => {
                const data = event?.data as any;
                if (!data?.agentId) return;
                const linkNode: SubagentLinkNode = {
                    type: "subagent_link",
                    id: `subagent_${data.agentId}`,
                    subagentId: data.agentId,
                    slug: data.slug ?? "",
                    parentAgent: data.parentAgent ?? "",
                    sessionId: data.sessionId ?? "",
                    status: "active",
                    model: data.model ?? null,
                };
                setDoc((prev) => [...prev, linkNode]);
                opts.log("subagent", `spawned: ${data.slug || data.agentId}`);
            },
        });
        onCleanup(() => unsubSpawned());

        const unsubCompleted = waveEventSubscribe({
            eventType: "subagent:completed",
            handler: (event: WaveEvent) => {
                const data = event?.data as any;
                if (!data?.agentId) return;
                const nodeId = `subagent_${data.agentId}`;
                setDoc((prev) =>
                    prev.map((n) =>
                        n.id === nodeId && n.type === "subagent_link"
                            ? { ...n, status: "completed" as const }
                            : n
                    )
                );
            },
        });
        onCleanup(() => unsubCompleted());
    });
}
