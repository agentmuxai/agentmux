// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Tower's state behind its native pane tab (tower.tsx): the latest sample,
// polled from srv (`tower.sample`) only while the pane is visible
// (SPEC_TOWER_TASK_MANAGER_PANE_2026_10_08.md §6.2).

import type { PaneTabHostContext } from "@/app/block/pane-tab-registry";
import { RpcApi, type TowerSnapshot } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { type Accessor, createEffect, createMemo, createSignal, on, onCleanup, type Setter } from "solid-js";
import { createStore, reconcile } from "solid-js/store";
import type { CpuMode, Sort, TowerView } from "./tower-util";

/** A sample with no CPU rates yet (the first one) is followed up this soon,
 *  instead of a whole interval of dashes. */
const FIRST_RATE_DELAY_MS = 1000;
const RETRY_DELAY_MS = 5000;

export class TowerViewModel {
    viewType = "tower";
    blockId: string;
    view: Accessor<TowerView>;
    cpuMode: Accessor<CpuMode>;
    snapshot: Accessor<TowerSnapshot | null>;
    error: Accessor<string | null>;
    sort: Accessor<Sort>;
    setSort: Setter<Sort>;
    expanded: Accessor<ReadonlySet<string>>;
    filter: Accessor<string>;
    setFilter: Setter<string>;
    viewName: Accessor<string>;
    private setMeta: (patch: Record<string, unknown>) => void;
    private setSnapshot: (snap: TowerSnapshot) => void;
    private setError: Setter<string | null>;
    private setExpanded: Setter<ReadonlySet<string>>;
    private timer: ReturnType<typeof setTimeout> | undefined;
    /** Bumped on every start and stop, so a reply from an older poll loop
     *  is dropped. */
    private generation = 0;

    // Built by `create(ctx)` in the instance's own reactive root (host rule 8),
    // so the effect and cleanup below live and die with the pane tab.
    constructor(ctx: PaneTabHostContext) {
        this.blockId = ctx.blockId;
        this.setMeta = (patch) => void ctx.setMeta(patch);
        this.view = createMemo<TowerView>(() => (ctx.meta()?.["tower:view"] === "host" ? "host" : "tasks"));
        this.cpuMode = createMemo<CpuMode>(() => (ctx.meta()?.["tower:cpu"] === "core" ? "core" : "machine"));
        this.viewName = createMemo(() => (this.view() === "host" ? "Tower · Host" : "Tower"));
        // A store merged by `id`: a task or process still there keeps its
        // object across polls, so its row (an expanded command line, a text
        // selection) survives the refresh instead of being rebuilt.
        const [state, setState] = createStore<{ snap: TowerSnapshot | null }>({ snap: null });
        this.snapshot = () => state.snap;
        this.setSnapshot = (snap) => setState("snap", reconcile(snap, { key: "id", merge: true }));
        [this.error, this.setError] = createSignal<string | null>(null);
        [this.sort, this.setSort] = createSignal<Sort>({ key: "cpu", desc: true });
        [this.expanded, this.setExpanded] = createSignal<ReadonlySet<string>>(new Set<string>());
        [this.filter, this.setFilter] = createSignal("");

        // Poll while visible; switching views restarts at once with the right
        // request.
        createEffect(
            on([() => ctx.visibility() === "active", this.view], ([visible, view]) => {
                this.stop();
                if (visible) this.start(view === "host");
            })
        );
        onCleanup(() => this.stop());
    }

    setView(view: TowerView): void {
        this.setMeta({ "tower:view": view === "host" ? "host" : null });
    }

    setCpuMode(mode: CpuMode): void {
        this.setMeta({ "tower:cpu": mode === "core" ? "core" : null });
    }

    toggleExpanded(taskId: string): void {
        const next = new Set(this.expanded());
        if (!next.delete(taskId)) next.add(taskId);
        this.setExpanded(next);
    }

    /** The task a host-list row belongs to, by id. */
    taskLabel = (taskId: string): string | undefined => this.snapshot()?.tasks.find((t) => t.id === taskId)?.label;

    async commandLine(processId: string): Promise<string | undefined> {
        const r = await RpcApi.TowerCommandLineCommand(TabRpcClient, { id: processId });
        return r.command_line;
    }

    dispose(): void {
        this.stop();
    }

    private start(host: boolean): void {
        const gen = ++this.generation;
        void this.poll(gen, host);
    }

    private stop(): void {
        this.generation++;
        if (this.timer !== undefined) clearTimeout(this.timer);
        this.timer = undefined;
    }

    private async poll(gen: number, host: boolean): Promise<void> {
        let delay = RETRY_DELAY_MS;
        try {
            const snap = await RpcApi.TowerSampleCommand(TabRpcClient, { host });
            if (gen !== this.generation) return;
            this.setSnapshot(snap);
            this.setError(null);
            const noRates = snap.tasks.length > 0 && snap.tasks.every((t) => t.cpu == null);
            delay = noRates ? FIRST_RATE_DELAY_MS : snap.interval_ms;
        } catch (e) {
            if (gen !== this.generation) return;
            this.setError(e instanceof Error ? e.message : String(e));
        }
        if (gen === this.generation) {
            this.timer = setTimeout(() => void this.poll(gen, host), delay);
        }
    }
}
