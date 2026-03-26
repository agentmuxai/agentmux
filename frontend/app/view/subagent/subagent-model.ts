// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { BlockNodeModel } from "@/app/block/blocktypes";
import { waveEventSubscribe } from "@/app/store/wps";
import { callBackendService, getWaveObjectAtom, makeORef } from "@/store/wos";
import { createSignal, type Accessor, type Setter } from "solid-js";

// ── Types ────────────────────────────────────────────────────────────────

export interface SubagentInfo {
    agent_id: string;
    slug: string;
    jsonl_path: string;
    parent_agent: string;
    session_id: string;
    last_event_at: number;
    status: "active" | "completed";
    event_count: number;
    model: string | null;
}

export interface SubagentEvent {
    agent_id: string;
    event_type: SubagentEventType;
    timestamp: number;
}

export type SubagentEventType =
    | { type: "text"; content: string }
    | { type: "tool_use"; name: string; input_summary: string }
    | { type: "tool_result"; is_error: boolean; preview: string }
    | { type: "progress"; output: string };

// ── ViewModel ────────────────────────────────────────────────────────────

export class SubagentViewModel implements ViewModel {
    viewType = "subagent";
    blockId: string;
    nodeModel: BlockNodeModel;

    viewIcon: Accessor<string> = () => "diagram-subtask";
    viewName: Accessor<string>;
    noPadding: Accessor<boolean> = () => true;

    get viewComponent(): ViewComponent {
        return null; // set by barrel to avoid circular import
    }

    // State signals
    private _events = createSignal<SubagentEvent[]>([]);
    eventsAtom: Accessor<SubagentEvent[]> = this._events[0];
    private setEvents: Setter<SubagentEvent[]> = this._events[1];

    private _info = createSignal<SubagentInfo | null>(null);
    infoAtom: Accessor<SubagentInfo | null> = this._info[0];
    private setInfo: Setter<SubagentInfo | null> = this._info[1];

    private _status = createSignal<"active" | "completed" | "loading">("loading");
    statusAtom: Accessor<"active" | "completed" | "loading"> = this._status[0];
    private setStatus: Setter<"active" | "completed" | "loading"> = this._status[1];

    private _autoScroll = createSignal<boolean>(true);
    autoScrollAtom: Accessor<boolean> = this._autoScroll[0];
    setAutoScroll: Setter<boolean> = this._autoScroll[1];

    // Event subscriptions
    private unsubs: (() => void)[] = [];
    private subagentId: string = "";

    constructor(blockId: string, nodeModel: BlockNodeModel) {
        this.blockId = blockId;
        this.nodeModel = nodeModel;

        // Read subagent:id from block metadata
        const blockDataAtom = getWaveObjectAtom<Block>(makeORef("block", blockId));
        const blockData = blockDataAtom();
        this.subagentId = blockData?.meta?.["subagent:id"] ?? "";

        this.viewName = () => {
            const info = this.infoAtom();
            if (info?.slug) return info.slug;
            if (this.subagentId) return this.subagentId.substring(0, 7);
            return "Subagent";
        };

        // Subscribe to subagent events
        const unsubActivity = waveEventSubscribe({
            eventType: "subagent:activity",
            handler: (event: WaveEvent) => {
                const data = event?.data as any;
                if (data?.agentId === this.subagentId || !this.subagentId) {
                    const newEvents = data?.events as SubagentEvent[] ?? [];
                    if (newEvents.length > 0) {
                        this.setEvents((prev) => [...prev, ...newEvents]);
                    }
                    if (data?.totalEvents != null) {
                        this.setInfo((prev) =>
                            prev ? { ...prev, event_count: data.totalEvents } : prev
                        );
                    }
                }
            },
        });
        if (unsubActivity) this.unsubs.push(unsubActivity);

        const unsubSpawned = waveEventSubscribe({
            eventType: "subagent:spawned",
            handler: (event: WaveEvent) => {
                const data = event?.data as any;
                if (data?.agentId === this.subagentId) {
                    this.setInfo({
                        agent_id: data.agentId,
                        slug: data.slug ?? "",
                        jsonl_path: "",
                        parent_agent: data.parentAgent ?? "",
                        session_id: data.sessionId ?? "",
                        last_event_at: Date.now(),
                        status: "active",
                        event_count: 0,
                        model: data.model ?? null,
                    });
                    this.setStatus("active");
                }
            },
        });
        if (unsubSpawned) this.unsubs.push(unsubSpawned);

        const unsubCompleted = waveEventSubscribe({
            eventType: "subagent:completed",
            handler: (event: WaveEvent) => {
                const data = event?.data as any;
                if (data?.agentId === this.subagentId) {
                    this.setStatus("completed");
                    this.setInfo((prev) =>
                        prev ? { ...prev, status: "completed" } : prev
                    );
                }
            },
        });
        if (unsubCompleted) this.unsubs.push(unsubCompleted);

        // Load initial history
        this.loadHistory();
    }

    loadHistory = async (): Promise<void> => {
        if (!this.subagentId) {
            this.setStatus("active");
            return;
        }
        try {
            const result = await callBackendService("subagent", "GetHistory", [
                this.subagentId,
                500,
            ]);
            const events = (result as SubagentEvent[]) ?? [];
            this.setEvents(events);
            this.setStatus("active");
        } catch {
            this.setStatus("active");
        }

        // Also load info
        try {
            const allActive = await callBackendService("subagent", "ListActive", []);
            const list = (allActive as SubagentInfo[]) ?? [];
            const match = list.find((s) => s.agent_id === this.subagentId);
            if (match) {
                this.setInfo(match);
                this.setStatus(match.status);
            }
        } catch {
            // ignore
        }
    };

    dispose(): void {
        for (const unsub of this.unsubs) {
            unsub();
        }
        this.unsubs = [];
    }
}
