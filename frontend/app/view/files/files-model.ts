// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The Files pane's state: the folder shown, its listing (paged from srv and
 * kept live by a watcher), history, sort, selection, and the operations a user
 * can run on what's selected, with undo.
 * docs/specs/SPEC_FILE_BROWSER_PANE_2026_10_01.md §5–§7, §9.1.
 *
 * Block meta holds only what should survive a restart: `files:path`,
 * `files:sort`, `files:sortdir`, `files:hidden`, `files:sidebar`. Selection,
 * scroll and history are per session (§6.2).
 */

import type { PaneTabHostContext } from "@/app/block/pane-tab-registry";
import { makeORef } from "@/app/store/mos";
import { muxEventSubscribe } from "@/app/store/mps";
import { WpsEvent } from "@/app/store/mps-events";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import type { FsEntry } from "@/types/rpc/FsEntry";
import type { FsError } from "@/types/rpc/FsError";
import type { FsOpResult } from "@/types/rpc/FsOpResult";
import type { FsPlace } from "@/types/rpc/FsPlace";
import { isMacOS, isWindows } from "@/util/platformutil";
import { batch, createMemo, createSignal } from "solid-js";
import { baseName, isWithin, joinPath, normalizePath, parentOf, samePath } from "./files-path";
import { EMPTY_SELECTION, pruneSelection, type Selection } from "./files-selection";
import { sortEntries, type SortDir, type SortKey } from "./files-sort";

export const META_PATH = "files:path";
export const META_SELECT = "files:select";
export const META_SORT = "files:sort";
export const META_SORTDIR = "files:sortdir";
export const META_HIDDEN = "files:hidden";
export const META_SIDEBAR = "files:sidebar";

/** Entries per `fs.list` page (srv caps at 5000). */
const PAGE = 1000;
/** No first page by now: say we're waiting (on macOS, likely for a prompt). */
export const SLOW_LISTING_MS = 400;

export type Phase =
    /** Listing the folder; nothing to show yet. */
    | "loading"
    /** Rows on screen (more pages may still be coming). */
    | "ready"
    /** The folder couldn't be listed: `error()` says why. */
    | "error"
    /** macOS: a protected folder the pane hasn't been allowed into this
     *  release; it waits for the user to open it (§9.1.4 items 2 and 7). */
    | "gated";

export interface AgentPlace {
    name: string;
    path: string;
}

type UndoEntry =
    | { kind: "trash"; paths: string[] }
    | { kind: "rename"; from: string; to: string }
    | { kind: "create"; path: string };

export interface StatusMessage {
    text: string;
    tone: "info" | "error";
    /** Offers Undo for the operation the message reports. */
    undo?: boolean;
}

/** Results that failed, as one sentence, or null when all succeeded. */
function failures(results: FsOpResult[]): string | null {
    const failed = results.filter((r) => !r.ok);
    if (failed.length === 0) return null;
    const first = failed[0];
    const more = failed.length > 1 ? ` (and ${failed.length - 1} more)` : "";
    return `${baseName(first.path)}: ${first.error ?? "failed"}${more}`;
}

/** macOS folders that need permission (Files & Folders), by place id. */
const MAC_PROTECTED_PLACE_IDS = new Set(["desktop", "documents", "downloads"]);

/** Folders listed without trouble in this release of the app: safe to list
 *  again without warning. Keyed by version, since macOS forgets grants when
 *  the app's identity changes with each release (§9.1.1 fact 8). */
function safeRootsKey(): string {
    let version = "";
    try {
        // `window.api` rather than getApi(), which warns when it's missing (tests).
        version = window.api?.getAboutModalDetails?.()?.version ?? "";
    } catch {
        // No host API: one key for all.
    }
    return `files:tcc-safe:${version}`;
}

function loadSafeRoots(): Set<string> {
    try {
        const raw = localStorage.getItem(safeRootsKey());
        return new Set(raw ? (JSON.parse(raw) as string[]) : []);
    } catch {
        return new Set();
    }
}

function saveSafeRoot(root: string): void {
    try {
        const roots = loadSafeRoots();
        roots.add(root);
        localStorage.setItem(safeRootsKey(), JSON.stringify([...roots]));
    } catch {
        // Storage full or unavailable: the pane will just explain again.
    }
}

export class FilesModel {
    readonly blockId: string;
    private readonly ctx: PaneTabHostContext;

    readonly path: () => string;
    readonly setPath: (p: string) => void;
    /** The listing as srv sent it, unsorted. */
    readonly rawEntries: () => FsEntry[];
    private readonly setRawEntries: (e: FsEntry[]) => void;
    readonly phase: () => Phase;
    private readonly setPhase: (p: Phase) => void;
    readonly error: () => FsError | null;
    private readonly setError: (e: FsError | null) => void;
    /** Still listing after SLOW_LISTING_MS. */
    readonly slow: () => boolean;
    private readonly setSlow: (v: boolean) => void;
    /** More pages are still coming. */
    readonly partial: () => boolean;
    private readonly setPartial: (v: boolean) => void;
    readonly selection: () => Selection;
    readonly setSelection: (s: Selection) => void;
    /** The row being renamed, by name. */
    readonly renaming: () => string | null;
    readonly setRenaming: (n: string | null) => void;
    /** A row the model wants scrolled into view (a new item about to be
     *  renamed, OpenFiles' selection, the folder Up came out of); the view
     *  follows it. A fresh object each time, so the same name asks again. */
    readonly revealRequest: () => { name: string } | null;
    private readonly setRevealRequest: (r: { name: string } | null) => void;
    readonly status: () => StatusMessage | null;
    private readonly setStatusSignal: (s: StatusMessage | null) => void;
    readonly places: () => FsPlace[];
    private readonly setPlaces: (p: FsPlace[]) => void;
    readonly agents: () => AgentPlace[];
    private readonly setAgents: (a: AgentPlace[]) => void;
    readonly canBack: () => boolean;
    readonly canForward: () => boolean;
    private readonly setHistoryVersion: (fn: (n: number) => number) => void;

    /** Sorted, hidden entries filtered unless shown. */
    readonly entries: () => FsEntry[];
    /** Names in display order (selection and type-ahead work on these). */
    readonly order: () => string[];

    private back: string[] = [];
    private forward: string[] = [];
    private generation = 0;
    private watchId: string | null = null;
    private watchedPath: string | null = null;
    private unsubscribe: (() => void) | null = null;
    private undoStack: UndoEntry[] = [];
    private statusTimer: ReturnType<typeof setTimeout> | null = null;
    private slowTimer: ReturnType<typeof setTimeout> | null = null;
    /** A change on disk while the pane was hidden: re-list on show. */
    private staleWhileHidden = false;
    private disposed = false;
    /** Protected macOS places the user clicked Open on in this pane: the
     *  click is the consent, whether or not macOS then allowed the read, so
     *  Try again and Refresh work after a denial (ReAgent on #4201). */
    private readonly openedPlaces = new Set<string>();
    /** Called once the first listing has painted (or failed): the view's
     *  settled-content hold (§6.5). */
    onFirstSettled: (() => void) | null = null;
    /** Puts keyboard focus on the list; set by the view while it's mounted. */
    focusList: (() => void) | null = null;

    constructor(ctx: PaneTabHostContext) {
        this.ctx = ctx;
        this.blockId = ctx.blockId;
        [this.path, this.setPath] = createSignal("");
        [this.rawEntries, this.setRawEntries] = createSignal<FsEntry[]>([]);
        [this.phase, this.setPhase] = createSignal<Phase>("loading");
        [this.error, this.setError] = createSignal<FsError | null>(null);
        [this.slow, this.setSlow] = createSignal(false);
        [this.partial, this.setPartial] = createSignal(false);
        [this.selection, this.setSelection] = createSignal<Selection>(EMPTY_SELECTION);
        [this.renaming, this.setRenaming] = createSignal<string | null>(null);
        [this.status, this.setStatusSignal] = createSignal<StatusMessage | null>(null);
        [this.revealRequest, this.setRevealRequest] = createSignal<{ name: string } | null>(null, { equals: false });
        [this.places, this.setPlaces] = createSignal<FsPlace[]>([]);
        [this.agents, this.setAgents] = createSignal<AgentPlace[]>([]);
        const [historyVersion, setHistoryVersion] = createSignal(0);
        this.setHistoryVersion = setHistoryVersion;
        this.canBack = () => (historyVersion(), this.back.length > 0);
        this.canForward = () => (historyVersion(), this.forward.length > 0);

        this.entries = createMemo(() => {
            const shown = this.showHidden() ? this.rawEntries() : this.rawEntries().filter((e) => !e.hidden);
            return sortEntries(shown, this.sortKey(), this.sortDir());
        });
        this.order = createMemo(() => this.entries().map((e) => e.name));

        this.unsubscribe = muxEventSubscribe({
            eventType: WpsEvent.FilesChanged,
            scope: makeORef("block", ctx.blockId),
            handler: (event) => {
                const dir = (event as { data?: { dir?: unknown } })?.data?.dir;
                if (typeof dir !== "string" || !this.path() || !samePath(dir, this.path())) return;
                if (this.ctx.visibility() !== "active") {
                    this.staleWhileHidden = true;
                    return;
                }
                void this.list(this.path(), { silent: true });
            },
        });
    }

    // ── Settings kept in block meta ──────────────────────────────────────────

    readonly sortKey = (): SortKey => {
        const v = this.ctx.meta()?.[META_SORT];
        return v === "modified" || v === "size" || v === "kind" ? v : "name";
    };
    readonly sortDir = (): SortDir => (this.ctx.meta()?.[META_SORTDIR] === "desc" ? "desc" : "asc");
    readonly showHidden = (): boolean => this.ctx.meta()?.[META_HIDDEN] === true;
    readonly showSidebar = (): boolean => this.ctx.meta()?.[META_SIDEBAR] !== false;

    /** Clicking a column header: sort by it, or flip its direction. */
    setSort(key: SortKey): void {
        const dir: SortDir = this.sortKey() === key && this.sortDir() === "asc" ? "desc" : "asc";
        void this.ctx.setMeta({ [META_SORT]: key === "name" ? null : key, [META_SORTDIR]: dir === "asc" ? null : dir });
    }

    toggleHidden(): void {
        void this.ctx.setMeta({ [META_HIDDEN]: this.showHidden() ? null : true });
    }

    toggleSidebar(): void {
        void this.ctx.setMeta({ [META_SIDEBAR]: this.showSidebar() ? false : null });
    }

    // ── Start ────────────────────────────────────────────────────────────────

    /** Where a new pane starts: its saved folder, or Home (§9.1.5: never
     *  Documents or Desktop). Restoring never lists a protected macOS folder
     *  on its own (§9.1.4 item 7). */
    async start(): Promise<void> {
        const saved = this.ctx.meta()?.[META_PATH];
        await this.loadPlaces();
        const home = this.places().find((p) => p.kind === "home")?.path ?? "~";
        const target = typeof saved === "string" && saved !== "" ? saved : home;
        await this.navigate(target, { push: false });
        void this.loadAgents();
    }

    private async loadPlaces(): Promise<void> {
        try {
            const res = await RpcApi.FsPlacesCommand(TabRpcClient, {});
            if (!this.disposed) this.setPlaces(res.places);
        } catch {
            // Places stay empty; the breadcrumb still works.
        }
    }

    /** Each named agent's working folder, for the Agents section of Places
     *  (§8.3): "where did that agent put things?" in one click. */
    private async loadAgents(): Promise<void> {
        try {
            const rows = await RpcApi.ListNamedAgentsCommand(TabRpcClient, { limit: 200 });
            const seen = new Set<string>();
            const agents: AgentPlace[] = [];
            for (const r of rows) {
                const dir = r.working_directory;
                if (!dir || seen.has(dir.toLowerCase())) continue;
                seen.add(dir.toLowerCase());
                agents.push({ name: r.instance_name || r.definition_name || baseName(dir), path: dir });
            }
            agents.sort((a, b) => a.name.localeCompare(b.name));
            if (!this.disposed) this.setAgents(agents);
        } catch {
            // No agents section.
        }
    }

    // ── macOS access prompts (§9.1.4) ────────────────────────────────────────

    /** The protected macOS place `path` is in, if any. */
    protectedPlace(raw: string): FsPlace | null {
        if (!isMacOS()) return null;
        // Compare the folder srv will list, not how it was spelled: `~/Documents`
        // or `Desktop/../Documents` from the path box, `mux view` or OpenFiles
        // must not slip past the gate (ReAgent on #4201).
        const home = this.places().find((p) => p.kind === "home")?.path ?? "";
        const path = normalizePath(raw, home);
        for (const p of this.places()) {
            // APFS ignores case by default, so ~/documents is Documents too.
            if (MAC_PROTECTED_PLACE_IDS.has(p.id) && isWithin(path.toLowerCase(), p.path.toLowerCase())) return p;
        }
        if (isWithin(path.toLowerCase(), "/volumes")) return { id: "volumes", label: "Volumes", path: "/Volumes", kind: "known" };
        return null;
    }

    /** Whether listing `path` now could raise a macOS prompt nobody asked
     *  for: inside a protected place this release hasn't listed yet. */
    private needsConsent(path: string): FsPlace | null {
        const place = this.protectedPlace(path);
        if (!place) return null;
        return this.openedPlaces.has(place.path) || loadSafeRoots().has(place.path) ? null : place;
    }

    // ── Navigation ───────────────────────────────────────────────────────────

    /**
     * Shows `target`. `push` records the folder being left in Back history.
     * On macOS, a protected folder this release hasn't listed yet stops at
     * the pane's notice, whichever way the user got there (Places, a row, the
     * breadcrumb, a restored pane); `consented` is the notice's Open button
     * (§9.1.4 items 2 and 7).
     */
    async navigate(target: string, opts: { push?: boolean; consented?: boolean } = {}): Promise<void> {
        const from = this.path();
        if (opts.push !== false && from && !samePath(from, target)) {
            this.back.push(from);
            this.forward = [];
            this.setHistoryVersion((n) => n + 1);
        }
        batch(() => {
            this.setPath(target);
            this.setSelection(EMPTY_SELECTION);
            this.setRenaming(null);
        });
        void this.ctx.setMeta({ [META_PATH]: target });
        if (opts.consented) {
            const place = this.protectedPlace(target);
            if (place) this.openedPlaces.add(place.path);
        }
        const gate = opts.consented ? null : this.needsConsent(target);
        if (gate) {
            this.generation++;
            this.stopWatching();
            batch(() => {
                this.setRawEntries([]);
                this.setError(null);
                this.setPhase("gated");
            });
            this.settleFirst();
            return;
        }
        await this.list(target, { silent: false, consented: true });
    }

    goBack(): void {
        const prev = this.back.pop();
        if (prev == null) return;
        this.forward.push(this.path());
        this.setHistoryVersion((n) => n + 1);
        void this.navigate(prev, { push: false });
    }

    goForward(): void {
        const next = this.forward.pop();
        if (next == null) return;
        this.back.push(this.path());
        this.setHistoryVersion((n) => n + 1);
        void this.navigate(next, { push: false });
    }

    goUp(): void {
        const parent = parentOf(this.path());
        if (!parent) return;
        const child = baseName(this.path());
        void this.navigate(parent).then(() => {
            // Land on the folder we came out of.
            if (this.order().includes(child)) {
                this.setSelection({ names: new Set([child]), focus: child, anchor: child });
                this.setRevealRequest({ name: child });
            }
        });
    }

    refresh(): void {
        void this.list(this.path(), { silent: true });
    }

    /** Re-list a folder that changed while the pane was hidden. */
    onShown(): void {
        if (this.staleWhileHidden) {
            this.staleWhileHidden = false;
            this.refresh();
        }
    }

    /**
     * Lists `dir` page by page. The first page paints at once; the rest are
     * appended as they come. A newer navigation abandons this one (srv drops
     * the cursor after its TTL). `silent` keeps the current rows on screen
     * while re-listing (a change on disk, Refresh) and keeps the selection.
     */
    async list(dir: string, opts: { silent: boolean; consented?: boolean }): Promise<void> {
        // Only navigate() decides to list a protected macOS folder; Refresh,
        // a change event, New folder or Undo must not list one the user hasn't
        // opened (ReAgent on #4201).
        if (!opts.consented && (this.phase() === "gated" || this.needsConsent(dir))) return;
        const gen = ++this.generation;
        const oldOrder = this.order();
        if (!opts.silent) {
            batch(() => {
                this.setRawEntries([]);
                this.setError(null);
                this.setPhase("loading");
                this.setSlow(false);
            });
            if (this.slowTimer) clearTimeout(this.slowTimer);
            this.slowTimer = setTimeout(() => {
                if (gen === this.generation && this.phase() === "loading") this.setSlow(true);
            }, SLOW_LISTING_MS);
        }
        let cursor: string | undefined;
        const collected: FsEntry[] = [];
        let first = true;
        try {
            do {
                const res = await RpcApi.FsListCommand(TabRpcClient, { path: dir, cursor, limit: PAGE });
                if (gen !== this.generation || this.disposed) return;
                if (res.error) {
                    this.stopWatching();
                    batch(() => {
                        this.setRawEntries([]);
                        this.setError(res.error!);
                        this.setPhase("error");
                        this.setPartial(false);
                    });
                    this.settleFirst();
                    return;
                }
                for (const e of res.entries) collected.push(e);
                cursor = res.cursor ?? undefined;
                if (first) {
                    // Paint the first page at once. Later pages are collected
                    // and the listing is set (and sorted) once at the end: a
                    // re-sort per page is O(pages x n log n) on a 200k-entry
                    // folder (ReAgent on #4201). A silent re-list keeps the old
                    // rows until then, so nothing vanishes and returns.
                    batch(() => {
                        if (first && res.path && !samePath(res.path, this.path())) {
                            // srv resolved `~` or a link: show the real path.
                            this.setPath(res.path);
                            void this.ctx.setMeta({ [META_PATH]: res.path });
                        }
                        if (!opts.silent) this.setRawEntries([...collected]);
                        this.setPhase("ready");
                        this.setPartial(cursor != null);
                    });
                }
                if (first) {
                    first = false;
                    this.settleFirst();
                    const place = this.protectedPlace(this.path());
                    if (place) saveSafeRoot(place.path);
                }
            } while (cursor);
            batch(() => {
                const before = this.order();
                this.setRawEntries([...collected]);
                this.setPartial(false);
                this.setSelection(pruneSelection(this.selection(), opts.silent ? oldOrder : before, this.order()));
            });
            this.applyRequestedSelection();
            void this.watch(this.path());
        } catch (err) {
            if (gen !== this.generation || this.disposed) return;
            batch(() => {
                this.setError({ kind: "other", message: err instanceof Error ? err.message : String(err) });
                this.setPhase("error");
                this.setPartial(false);
            });
            this.settleFirst();
        }
    }

    /** `files:select` (written by the OpenFiles tool): select those entries
     *  once, then clear the request. */
    applyRequestedSelection(): void {
        // Only against a complete listing of the folder it names.
        if (this.phase() !== "ready" || this.partial()) return;
        const req = this.ctx.meta()?.[META_SELECT];
        if (!Array.isArray(req) || req.length === 0) return;
        const present = new Set(this.order());
        const names = (req as unknown[])
            .filter((v): v is string => typeof v === "string")
            .map((v) => (/[\\/]/.test(v) ? baseName(v) : v))
            .filter((n) => present.has(n));
        if (names.length > 0) {
            this.setSelection({ names: new Set(names), focus: names[0], anchor: names[0] });
            // Show the user where the agent pointed (ReAgent on #4201).
            this.setRevealRequest({ name: names[0] });
        }
        void this.ctx.setMeta({ [META_SELECT]: null });
    }

    private settleFirst(): void {
        const cb = this.onFirstSettled;
        this.onFirstSettled = null;
        cb?.();
    }

    // ── Watching ─────────────────────────────────────────────────────────────

    private async watch(dir: string): Promise<void> {
        if (this.watchedPath && samePath(this.watchedPath, dir)) return;
        this.stopWatching();
        this.watchedPath = dir;
        try {
            const res = await RpcApi.FsWatchCommand(TabRpcClient, { path: dir, block_id: this.blockId });
            if (this.disposed || !this.watchedPath || !samePath(this.watchedPath, dir)) {
                void RpcApi.FsUnwatchCommand(TabRpcClient, { watch_id: res.watch_id }).catch(() => {});
                return;
            }
            this.watchId = res.watch_id;
        } catch {
            // No live updates for this folder; Refresh still works, and the
            // next listing tries to watch again (ReAgent on #4201).
            if (this.watchedPath && samePath(this.watchedPath, dir)) this.watchedPath = null;
        }
    }

    private stopWatching(): void {
        const id = this.watchId;
        this.watchId = null;
        this.watchedPath = null;
        if (id) void RpcApi.FsUnwatchCommand(TabRpcClient, { watch_id: id }).catch(() => {});
    }

    // ── Status line ──────────────────────────────────────────────────────────

    setStatus(msg: StatusMessage | null, ms = 6000): void {
        if (this.statusTimer) clearTimeout(this.statusTimer);
        this.statusTimer = null;
        this.setStatusSignal(msg);
        if (msg) this.statusTimer = setTimeout(() => this.setStatusSignal(null), ms);
    }

    // ── Operations (§7) ──────────────────────────────────────────────────────

    /** Selected entries, in display order; the focused row when nothing is
     *  selected. */
    selectedEntries(): FsEntry[] {
        // Selected names only, never the focused row on its own: a row that
        // shows as unselected must not be deleted, opened or copied
        // (ReAgent on #4201).
        const names = this.selection().names;
        return this.entries().filter((e) => names.has(e.name));
    }

    pathOf(name: string): string {
        return joinPath(this.path(), name);
    }

    async rename(name: string, newName: string): Promise<boolean> {
        if (newName === name) return true;
        const from = this.pathOf(name);
        try {
            const res = await RpcApi.FsRenameCommand(TabRpcClient, { path: from, new_name: newName });
            this.undoStack.push({ kind: "rename", from, to: res.new_path });
            this.setSelection({ names: new Set([newName]), focus: newName, anchor: newName });
            this.refresh();
            return true;
        } catch (err) {
            this.setStatus({ text: errorText(err), tone: "error" });
            return false;
        }
    }

    /** Creates "New folder" (or "New file.txt", "New folder (2)" when taken)
     *  and starts renaming it (§7.4). */
    async createNew(kind: "file" | "dir"): Promise<void> {
        if (this.phase() !== "ready") return;
        const base = kind === "dir" ? "New folder" : "New file";
        const ext = kind === "dir" ? "" : ".txt";
        const taken = new Set(this.rawEntries().map((e) => e.name.toLowerCase()));
        let name = `${base}${ext}`;
        for (let i = 2; taken.has(name.toLowerCase()); i++) name = `${base} (${i})${ext}`;
        try {
            const res = await RpcApi.FsCreateCommand(TabRpcClient, { parent: this.path(), name, kind });
            this.undoStack.push({ kind: "create", path: res.path });
            await this.list(this.path(), { silent: true });
            batch(() => {
                this.setSelection({ names: new Set([name]), focus: name, anchor: name });
                // Scrolled into view first: the rename box only exists on a
                // mounted row, and the keyboard waits on it (ReAgent on #4201).
                this.setRevealRequest({ name });
                this.setRenaming(name);
            });
        } catch (err) {
            this.setStatus({ text: errorText(err), tone: "error" });
        }
    }

    /** Delete key: to the OS Trash, no confirmation, with Undo (§7.3). */
    async trash(entries: FsEntry[]): Promise<void> {
        if (entries.length === 0) return;
        const paths = entries.map((e) => this.pathOf(e.name));
        try {
            const res = await RpcApi.FsTrashCommand(TabRpcClient, { paths });
            const done = res.results.filter((r) => r.ok).map((r) => r.path);
            if (done.length > 0) this.undoStack.push({ kind: "trash", paths: done });
            const problem = failures(res.results);
            if (problem) this.setStatus({ text: `Couldn't move to Trash: ${problem}`, tone: "error" });
            else
                this.setStatus({
                    text: done.length === 1 ? `Moved ${baseName(done[0])} to Trash` : `Moved ${done.length} items to Trash`,
                    tone: "info",
                    undo: true,
                });
            this.refresh();
        } catch (err) {
            this.setStatus({ text: errorText(err), tone: "error" });
        }
    }

    /** Shift+Delete, after the view's named confirmation: gone for good. */
    async deletePermanently(entries: FsEntry[]): Promise<void> {
        if (entries.length === 0) return;
        try {
            const res = await RpcApi.FsDeleteCommand(TabRpcClient, { paths: entries.map((e) => this.pathOf(e.name)) });
            const problem = failures(res.results);
            const done = res.results.filter((r) => r.ok).length;
            this.setStatus(
                problem
                    ? { text: `Couldn't delete: ${problem}`, tone: "error" }
                    : { text: done === 1 ? `Deleted ${entries[0].name}` : `Deleted ${done} items`, tone: "info" }
            );
            this.refresh();
        } catch (err) {
            this.setStatus({ text: errorText(err), tone: "error" });
        }
    }

    /** Ctrl+Z, or Undo in the status line: reverses the last rename, new
     *  item, or move to Trash. */
    async undo(): Promise<void> {
        const last = this.undoStack.pop();
        if (!last) {
            this.setStatus({ text: "Nothing to undo", tone: "info" }, 2500);
            return;
        }
        try {
            if (last.kind === "rename") {
                await RpcApi.FsRenameCommand(TabRpcClient, { path: last.to, new_name: baseName(last.from) });
                this.setStatus({ text: `Renamed back to ${baseName(last.from)}`, tone: "info" });
            } else if (last.kind === "create") {
                const res = await RpcApi.FsTrashCommand(TabRpcClient, { paths: [last.path] });
                const problem = failures(res.results);
                this.setStatus(
                    problem ? { text: `Couldn't undo: ${problem}`, tone: "error" } : { text: `Moved ${baseName(last.path)} to Trash`, tone: "info" }
                );
            } else {
                const res = await RpcApi.FsRestoreCommand(TabRpcClient, { paths: last.paths });
                const problem = failures(res.results);
                this.setStatus(
                    problem
                        ? { text: `Couldn't restore: ${problem}`, tone: "error" }
                        : { text: last.paths.length === 1 ? `Restored ${baseName(last.paths[0])}` : `Restored ${last.paths.length} items`, tone: "info" }
                );
            }
        } catch (err) {
            this.setStatus({ text: `Couldn't undo: ${errorText(err)}`, tone: "error" });
        }
        this.refresh();
    }

    dispose(): void {
        this.disposed = true;
        this.generation++;
        this.stopWatching();
        this.unsubscribe?.();
        this.unsubscribe = null;
        if (this.statusTimer) clearTimeout(this.statusTimer);
        if (this.slowTimer) clearTimeout(this.slowTimer);
    }
}

/** An RPC failure as the sentence srv wrote, without the transport's prefix. */
export function errorText(err: unknown): string {
    const msg = err instanceof Error ? err.message : String(err);
    return msg.replace(/^(Error:\s*)+/, "").replace(/^fs\.[a-z]+:\s*/, "");
}

/** Windows rules for names, on Windows. */
export const windowsNames = (): boolean => isWindows();
