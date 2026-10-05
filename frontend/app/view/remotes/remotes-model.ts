// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The Remotes pane's model (docs/specs/SPEC_REMOTES_PANE_2026_10_05.md §4).
// The list is srv's (`remoteslist`), so every Remotes tab shows the same one;
// this model only fetches it, re-fetches when something changes, and runs the
// row actions.

import { createSignal, type Accessor } from "solid-js";
import type { PaneTabHostContext } from "@/app/block/pane-tab-registry";
import { muxEventSubscribe } from "@/app/store/mps";
import { WpsEvent } from "@/app/store/mps-events";
import { RpcApi } from "@/app/store/rpc-api";
import type { RemoteRecord } from "@/app/store/rpc-api/remotes";
import { TabRpcClient } from "@/app/store/rpc-util";
import { showHostSessions } from "@/app/view/term/hostSessions";

/** Changes arrive in bursts (a connect publishes several statuses); one fetch per burst. */
const REFRESH_DEBOUNCE_MS = 150;

export class RemotesViewModel {
    viewType = "remotes";
    blockId: string;

    readonly records: Accessor<RemoteRecord[]>;
    private readonly setRecords: (r: RemoteRecord[]) => void;
    readonly loaded: Accessor<boolean>;
    private readonly setLoaded: (v: boolean) => void;
    readonly error: Accessor<string>;
    private readonly setError: (v: string) => void;
    readonly filter: Accessor<string>;
    readonly setFilter: (v: string) => void;
    /** The remote whose detail panel is open, by name. */
    readonly expanded: Accessor<string | null>;
    readonly setExpanded: (v: string | null) => void;
    /** The last failed action, shown above the list until the next one succeeds. */
    readonly notice: Accessor<string>;
    readonly setNotice: (v: string) => void;

    private unsubscribe: (() => void) | null = null;
    private refreshTimer: ReturnType<typeof setTimeout> | null = null;
    private disposed = false;

    constructor(ctx: Pick<PaneTabHostContext, "blockId">, opts: { subscribe?: boolean } = {}) {
        this.blockId = ctx.blockId;
        [this.records, this.setRecords] = createSignal<RemoteRecord[]>([]);
        [this.loaded, this.setLoaded] = createSignal(false);
        [this.error, this.setError] = createSignal("");
        [this.filter, this.setFilter] = createSignal("");
        [this.expanded, this.setExpanded] = createSignal<string | null>(null);
        [this.notice, this.setNotice] = createSignal("");
        if (opts.subscribe !== false) {
            this.unsubscribe = muxEventSubscribe(
                { eventType: WpsEvent.ConnChange, handler: () => this.scheduleRefresh() },
                { eventType: WpsEvent.RemotesChange, handler: () => this.scheduleRefresh() },
                { eventType: WpsEvent.Config, handler: () => this.scheduleRefresh() }
            );
            void this.refresh();
        }
    }

    /** Fetch the list now. */
    async refresh(): Promise<void> {
        try {
            const records = await RpcApi.RemotesListCommand(TabRpcClient);
            if (this.disposed) return;
            this.setRecords(records ?? []);
            this.setError("");
        } catch (e) {
            if (this.disposed) return;
            this.setError(`Couldn't load remotes: ${e instanceof Error ? e.message : String(e)}`);
        } finally {
            if (!this.disposed) this.setLoaded(true);
        }
    }

    scheduleRefresh(): void {
        if (this.disposed) return;
        if (this.refreshTimer) clearTimeout(this.refreshTimer);
        this.refreshTimer = setTimeout(() => {
            this.refreshTimer = null;
            void this.refresh();
        }, REFRESH_DEBOUNCE_MS);
    }

    toggleExpanded(name: string): void {
        this.setExpanded(this.expanded() === name ? null : name);
    }

    /** Run a row action; a failure shows in the pane instead of vanishing. */
    async run(what: string, action: () => Promise<void>): Promise<void> {
        try {
            await action();
            if (!this.disposed) this.setNotice("");
        } catch (e) {
            if (!this.disposed) this.setNotice(`${what} failed: ${e instanceof Error ? e.message : String(e)}`);
        }
    }

    // ── Row actions (§4.2) ──────────────────────────────────────────────────

    /** A terminal on the remote, beside this pane. */
    async newTerminal(name: string): Promise<void> {
        await TabRpcClient.rpcCall(
            "pane.open",
            { view: "term", connection: name, split_direction: "right", split_reference_block_id: this.blockId },
            {}
        );
    }

    /** Hangar on the remote's home folder, beside this pane. */
    async browseFiles(name: string): Promise<void> {
        await TabRpcClient.rpcCall(
            "pane.open",
            { view: "files", cwd: "~", connection: name, split_direction: "right", split_reference_block_id: this.blockId },
            {}
        );
    }

    async connect(name: string): Promise<void> {
        await RpcApi.ConnConnectCommand(TabRpcClient, { host: name, logblockid: this.blockId }, { timeout: 60000 });
    }

    async disconnect(name: string): Promise<void> {
        await RpcApi.ConnDisconnectCommand(TabRpcClient, name, { timeout: 5000 });
    }

    /** The durable sessions on the host: the same list as a pane's "Sessions on <host>…". */
    async showSessions(name: string): Promise<void> {
        await showHostSessions(name, this.blockId);
        this.scheduleRefresh();
    }

    /** Change the remote's settings; `null` removes a key. */
    async setSettings(name: string, values: Record<string, unknown>): Promise<void> {
        await RpcApi.RemoteSetConfigCommand(TabRpcClient, { connection: name, values });
    }

    async setPinned(name: string, pinned: boolean): Promise<void> {
        await this.setSettings(name, { "display:pinned": pinned ? true : null });
    }

    async setHidden(name: string, hidden: boolean): Promise<void> {
        await this.setSettings(name, { "display:hidden": hidden ? true : null });
    }

    async setNickname(name: string, nickname: string): Promise<void> {
        const nick = nickname.trim();
        await this.setSettings(name, { "display:name": nick && nick !== name ? nick : null });
    }

    async setColor(name: string, color: string | null): Promise<void> {
        await this.setSettings(name, { "display:color": color });
    }

    /** `true`, `false`, or `null` to follow the global setting. */
    async setDurable(name: string, durable: boolean | null): Promise<void> {
        await this.setSettings(name, { "term:durable": durable });
    }

    async forget(name: string): Promise<void> {
        await RpcApi.RemoteForgetCommand(TabRpcClient, { connection: name });
        if (this.expanded() === name) this.setExpanded(null);
    }

    dispose(): void {
        this.disposed = true;
        if (this.refreshTimer) clearTimeout(this.refreshTimer);
        this.unsubscribe?.();
        this.unsubscribe = null;
    }
}
