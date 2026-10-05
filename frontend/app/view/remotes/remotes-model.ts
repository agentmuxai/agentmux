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
import { META_REMOTES_EXPAND } from "./remotes-sections";

/** What the Add remote form gives; only `alias` is required. */
export interface NewRemote {
    alias: string;
    hostname: string;
    user: string;
    port: string;
    identityfile: string;
    proxyjump: string;
}

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

    /** The host another pane asked this tab to show (`remotes:expand`, open-remotes.ts). */
    readonly expandRequest: Accessor<string>;
    private readonly setMeta?: PaneTabHostContext["setMeta"];

    constructor(
        ctx: Pick<PaneTabHostContext, "blockId"> & Partial<Pick<PaneTabHostContext, "meta" | "setMeta">>,
        opts: { subscribe?: boolean } = {}
    ) {
        this.blockId = ctx.blockId;
        this.expandRequest = () => (ctx.meta?.()?.[META_REMOTES_EXPAND] as string | undefined) ?? "";
        this.setMeta = ctx.setMeta;
        [this.records, this.setRecords] = createSignal<RemoteRecord[]>([]);
        [this.loaded, this.setLoaded] = createSignal(false);
        [this.error, this.setError] = createSignal("");
        [this.filter, this.setFilter] = createSignal("");
        [this.expanded, this.setExpanded] = createSignal<string | null>(null);
        [this.notice, this.setNotice] = createSignal("");
        [this.testResults, this.setTestResults] = createSignal<Record<string, true | string>>({});
        [this.testing, this.setTesting] = createSignal<string | null>(null);
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

    /** Expand the requested host's row and clear the request, so it acts once. */
    takeExpandRequest(name: string): void {
        this.setExpanded(name);
        void this.setMeta?.({ [META_REMOTES_EXPAND]: null });
    }

    toggleExpanded(name: string): void {
        this.setExpanded(this.expanded() === name ? null : name);
    }

    /** Run a row action; a failure shows in the pane instead of vanishing. */
    async run(what: string, action: () => Promise<void>): Promise<void> {
        // Cleared first, so an action may leave a notice of its own.
        this.setNotice("");
        try {
            await action();
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

    /** `ask` (clears the key: the global setting applies), `always` or `never`. */
    async setHelperPolicy(name: string, policy: string): Promise<void> {
        await this.setSettings(name, { "conn:helper": policy === "ask" ? null : policy });
    }

    /** Remove the helper from the host; srv asks the user first, naming the sessions that end. */
    async removeHelper(name: string): Promise<void> {
        // The question waits up to two minutes for the user, and the removal runs over ssh.
        try {
            await RpcApi.RemoteHelperRemoveCommand(TabRpcClient, { connection: name, blockid: this.blockId }, { timeout: 240_000 });
        } catch (e) {
            // "Keep It" is the user's answer, not a failure.
            if (!String(e instanceof Error ? e.message : e).includes("kept:")) throw e;
        }
        this.scheduleRefresh();
    }

    async forget(name: string): Promise<void> {
        await RpcApi.RemoteForgetCommand(TabRpcClient, { connection: name });
        if (this.expanded() === name) this.setExpanded(null);
    }

    // ── Adding and testing (§4.4) ───────────────────────────────────────────

    /** Append the host to ~/.ssh/config; srv shows the user the exact block
     *  first. Resolves `false` when they cancel. Expands the new row. */
    async addRemote(host: NewRemote): Promise<boolean> {
        try {
            // The question waits up to two minutes for the user.
            await RpcApi.RemoteAddCommand(TabRpcClient, { ...host, blockid: this.blockId }, { timeout: 180_000 });
        } catch (e) {
            if (String(e instanceof Error ? e.message : e).includes("kept:")) return false;
            throw e;
        }
        this.setExpanded(host.alias.trim());
        await this.refresh();
        return true;
    }

    /** The last connection test per remote: `true`, or ssh's message. */
    readonly testResults: Accessor<Record<string, true | string>>;
    private readonly setTestResults: (v: Record<string, true | string>) => void;
    readonly testing: Accessor<string | null>;
    private readonly setTesting: (v: string | null) => void;

    /** Log in and run `true`, ssh's prompts going to the user's window. */
    async testConnection(name: string): Promise<void> {
        this.setTesting(name);
        try {
            const res = await RpcApi.RemoteTestCommand(TabRpcClient, { connection: name, blockid: this.blockId }, { timeout: 200_000 });
            this.setTestResults({ ...this.testResults(), [name]: res.ok ? true : res.message || "ssh failed" });
        } finally {
            if (this.testing() === name) this.setTesting(null);
        }
    }

    /** Open the ssh config file defining `name` in an editor beside this pane. */
    async editInSshConfig(name: string): Promise<void> {
        const where = await RpcApi.RemoteSshLocateCommand(TabRpcClient, { connection: name });
        if (!where) throw new Error(`your ssh config doesn't define ${name}`);
        await TabRpcClient.rpcCall(
            "pane.open",
            { view: "editor", file: where.path, split_direction: "right", split_reference_block_id: this.blockId },
            {}
        );
        this.setNotice(`${name} is defined on line ${where.line} of ${where.path}.`);
    }

    dispose(): void {
        this.disposed = true;
        if (this.refreshTimer) clearTimeout(this.refreshTimer);
        this.unsubscribe?.();
        this.unsubscribe = null;
    }
}
