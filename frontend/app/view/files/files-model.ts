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

import type { PaneTabHostContext } from "@/app/store/pane-tab-registry";
import { getSettingsKeyAtom } from "@/app/store/block-atom-cache";
import { makeORef } from "@/app/store/mos";
import { onMuxEvent } from "@/app/store/mps";
import { WpsEvent } from "@/app/store/mps-events";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { paneZoom } from "@/app/store/pane-zoom-local";
import { seedPlacesZoom } from "./files-zoom-parts";
import type { FsEntry } from "@/types/rpc/FsEntry";
import type { FsError } from "@/types/rpc/FsError";
import type { FsOpResult } from "@/types/rpc/FsOpResult";
import type { FsGitState } from "@/types/rpc/FsGitState";
import type { FsGitStatus } from "@/types/rpc/FsGitStatus";
import type { FsPlace } from "@/types/rpc/FsPlace";
import { isMacOS, isWindows } from "@/util/platformutil";
import { isSshConnection } from "@/app/view/term/ssh-connection";
import { isValidAgentColor, pickAgentColor } from "@/app/view/agent/agent-color";
import { compareLastUsed } from "@/app/view/agent/agent-recency";
import { blockRoleColor, isLightThemeActive } from "@/app/block/pane-identity";
import type { RemoteRecord } from "@/app/store/rpc-api/remotes";
import { displayName, groupRemotes, remoteColor } from "@/app/view/remotes/remotes-sections";
import { batch, createMemo, createSignal } from "solid-js";
import { baseName, isWithin, joinPath, joinRel, normalizePath, parentOf, samePath } from "./files-path";
import { treeMove } from "@/app/fs/tree/tree-keys";
import { flattenTree, isUnderRel, labelOf, parentRel, type TreeRow } from "@/app/fs/tree/tree-rows";
import { crawlFolders, rankMatches, whereOf } from "./files-find";
import { ColumnsState } from "./files-columns-state";
import { EMPTY_SELECTION, pruneSelection, type Selection } from "./files-selection";
import { addRecent } from "./files-pins";
import { childNamesOf, namesIn } from "./files-names";
import { landedAsPlanned, movedOk, planMoveUndo, restorable, type MovedItem, type UndoPlan } from "./files-undo-move";
import { clipboard, FilesOps, opFinishedText, sameVolume, setClipboard, type OpView } from "./files-ops";
import { systemIconsAvailable } from "./files-icons";
import { sortEntries, type SortDir, type SortKey } from "./files-sort";

// In files-path.ts, so the pane's manifest (files.tsx) needn't load the model.
import { META_PATH } from "./files-path";
export { META_PATH };
const META_SELECT = "files:select";
const META_SORT = "files:sort";
const META_SORTDIR = "files:sortdir";
const META_HIDDEN = "files:hidden";
const META_SIDEBAR = "files:sidebar";
const META_PREVIEW = "files:preview";
/** Set by srv when a container agent opens a folder of its workspace: the one folder this pane may show. */
const META_ROOT = "files:root";
const META_VIEW = "files:view";
/** A move of more items than this is not offered Undo: each folder among them is listed to be sure of it. */
const MAX_UNDO_ITEMS = 200;
/** The most folders a pane keeps open in place at once: each one is watched, and srv allows 64 watches to a pane. */
const MAX_EXPANDED = 40;
/** The most matches a find shows. */
const MAX_FOUND = 200;
/** The tree view: folders open in place (`true`), on the details view. */
const META_TREE = "files:tree";
/** The SSH connection the pane is browsing (the key terminals use too);
 *  absent for this computer. Remote terminals spec §6.3. */
const META_CONN = "connection";

/** Entries per `fs.list` page (srv caps at 5000). */
const PAGE = 1000;
/** No first page by now: say we're waiting (on macOS, likely for a prompt). */
const SLOW_LISTING_MS = 400;
/** Ask git this long after a listing settles (a burst of changes re-lists
 *  many times; each `git status` is a process). */
const GIT_DELAY_MS = 300;

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
    /** The agent's identity color (#rrggbb), the one its pane border uses. */
    color?: string;
}

type UndoEntry =
    | { kind: "trash"; paths: string[] }
    | { kind: "rename"; from: string; to: string }
    | { kind: "create"; path: string }
    /** A finished move that is certain to have put each item at `<destination>/<its name>` (files-undo-move.ts). */
    | { kind: "move"; items: MovedItem[] };

/** An SSH host in the Remote section of Places. */
export interface FilesRemote {
    /** The connection name. */
    name: string;
    /** Its nickname, else the name. */
    label: string;
    color?: string;
}

/** The Remote section's hosts: SSH remotes in the order Connectors > Remotes shows them
 *  (pinned, SSH hosts, recent), hidden ones left out. */
export function filesRemotes(records: RemoteRecord[]): FilesRemote[] {
    return groupRemotes(records.filter((r) => r.kind === "ssh"))
        .filter((g) => g.section !== "hidden")
        .flatMap((g) => g.records)
        .map((r) => ({ name: r.name, label: displayName(r), color: remoteColor(r) }));
}

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

/**
 * An agent's colour as text in the current theme: its hue, at the lightness made for text on that theme's
 * surface (the Kanban's name colour, pane-color-scheme.ts), not the raw colour, which is 1.7:1 on a light theme
 * for a yellow. Falls back to the raw colour when it isn't a hue it can map.
 */
export function readableAgentColor(hex: string): string {
    return blockRoleColor({ "frame:activebordercolor": hex } as Block["meta"], isLightThemeActive(), "identity", { widget: false }) ?? hex;
}

/** An agent's identity color: its stored `ui:color` (what seeds the pane
 *  border), else the deterministic pick the pane would get on first open. */
async function loadAgentColor(definitionId: string): Promise<string | undefined> {
    if (!definitionId) return undefined;
    try {
        const stored = await RpcApi.GetAgentContentCommand(TabRpcClient, {
            agent_id: definitionId,
            content_type: "ui:color",
        });
        const color = stored?.content?.trim();
        if (isValidAgentColor(color)) return color;
    } catch {
        // Fall through to the deterministic pick.
    }
    return pickAgentColor(definitionId);
}

export class FilesModel {
    readonly blockId: string;
    private readonly ctx: PaneTabHostContext;
    /** Whether rows show the system's own icons: setting `files:icons` (default "system") and a source on this computer. */
    readonly systemIcons = (): boolean => (getSettingsKeyAtom("files:icons")() ?? "system") === "system" && systemIconsAvailable();
    /** Display scale times this pane's zoom: an icon's CSS size times this picks its pixel size. */
    readonly iconScale = (): number => (globalThis.window?.devicePixelRatio || 1) * paneZoom(this.blockId, this.ctx.meta());

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
    /** The SSH connection shown, or "" for this computer. */
    readonly connection: () => string;
    private readonly setConnection: (c: string) => void;
    /** SSH hosts for the Remote section of Places. */
    readonly remotes: () => FilesRemote[];
    private readonly setRemotes: (r: FilesRemote[]) => void;
    readonly agents: () => AgentPlace[];
    /** What git says about the folder shown: markers per entry, the branch. */
    readonly git: () => FsGitStatus | null;
    private readonly setGit: (g: FsGitStatus | null) => void;
    /** Each entry's git state, by name. */
    readonly gitStateOf: () => ReadonlyMap<string, FsGitState>;
    private gitGeneration = 0;
    private gitTimer: ReturnType<typeof setTimeout> | null = null;
    private readonly setAgents: (a: AgentPlace[]) => void;
    readonly canBack: () => boolean;
    readonly canForward: () => boolean;
    private readonly setHistoryVersion: (fn: (n: number) => number) => void;

    /** Sorted, hidden entries filtered unless shown. */
    readonly entries: () => FsEntry[];
    /** The columns view: which column the keys are in, the columns, and the moves between them. */
    readonly cols: ColumnsState;
    /** The tree view's rows (empty unless it is on), and each by its name. */
    readonly treeRows: () => TreeRow[];
    readonly treeInfo: () => ReadonlyMap<string, TreeRow>;
    /** Finding by name in the folders under the shown one is on (the toolbar's second button), what has been indexed of them, and
     *  a number that changes when more has been (the ranking reads it). */
    private readonly deep: () => boolean;
    private readonly setDeep: (v: boolean) => void;
    private findEntries: FsEntry[] = [];
    private readonly findVersion: () => number;
    private readonly setFindVersion: (fn: (n: number) => number) => void;
    private findRun = 0;
    private findTimer: ReturnType<typeof setTimeout> | null = null;
    private readonly ranked: () => FsEntry[];
    /** The folders opened in place, by path from the shown folder (`src/lib`), and what has been listed of each. */
    private readonly expanded: () => ReadonlySet<string>;
    private readonly setExpanded: (s: ReadonlySet<string>) => void;
    private readonly kids: () => ReadonlyMap<string, FsEntry[]>;
    private readonly setKids: (m: ReadonlyMap<string, FsEntry[]>) => void;
    /** A watch on each opened folder, by the same path, and the ones being set up. */
    private readonly treeWatches = new Map<string, string>();
    /** The newest listing started of each opened folder, so a slower, older one is dropped when it ends. */
    private readonly kidsLoad = new Map<string, number>();
    private kidsLoadSeq = 0;
    private readonly treeWatching = new Set<string>();
    /** The filter typed into the toolbar (spec §5.3.1, Ctrl+F). */
    readonly filter: () => string;
    private readonly setFilterSignal: (v: string) => void;
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
    /** Copy and move jobs started from this pane (§7.1). */
    readonly ops: FilesOps;
    /** Puts keyboard focus on the list; set by the view while it's mounted. */
    focusList: (() => void) | null = null;

    constructor(ctx: PaneTabHostContext) {
        this.ctx = ctx;
        this.blockId = ctx.blockId;
        // Places gets a level of its own the first time: it keeps the size it has had, once.
        seedPlacesZoom(ctx.meta(), (patch) => void ctx.setMeta(patch));
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
        const conn = ctx.meta()?.[META_CONN];
        [this.connection, this.setConnection] = createSignal(isSshConnection(conn) ? String(conn).trim() : "");
        [this.remotes, this.setRemotes] = createSignal<FilesRemote[]>([]);
        [this.agents, this.setAgents] = createSignal<AgentPlace[]>([]);
        [this.git, this.setGit] = createSignal<FsGitStatus | null>(null);
        this.gitStateOf = createMemo(() => new Map((this.git()?.entries ?? []).map((e) => [e.name, e.state])));
        const [historyVersion, setHistoryVersion] = createSignal(0);
        this.setHistoryVersion = setHistoryVersion;
        this.canBack = () => (historyVersion(), this.back.length > 0);
        this.canForward = () => (historyVersion(), this.forward.length > 0);

        [this.filter, this.setFilterSignal] = createSignal("");
        [this.deep, this.setDeep] = createSignal(false);
        [this.findVersion, this.setFindVersion] = createSignal(0);
        this.ranked = createMemo(() => (this.findVersion(), rankMatches(this.findEntries, this.filter(), MAX_FOUND)));
        [this.expanded, this.setExpanded] = createSignal<ReadonlySet<string>>(new Set());
        [this.kids, this.setKids] = createSignal<ReadonlyMap<string, FsEntry[]>>(new Map());
        this.treeRows = createMemo(() =>
            this.treeOn()
                ? flattenTree(
                      this.rawEntries(),
                      this.expanded(),
                      (rel) => this.kids().get(rel),
                      (e) => this.showHidden() || !e.hidden,
                      (list) => sortEntries(list, this.sortKey(), this.sortDir())
                  )
                : []
        );
        this.treeInfo = createMemo(() => new Map(this.treeRows().map((r) => [r.entry.name, r])));
        this.cols = new ColumnsState({
            on: () => this.columnsOn(),
            expanded: () => this.expanded(),
            setExpanded: (next) => this.setExpanded(next),
            kids: () => this.kids(),
            setKids: (next) => this.setKids(next),
            rawEntries: () => this.rawEntries(),
            shows: (e) => this.showHidden() || !e.hidden,
            sort: (list) => sortEntries(list, this.sortKey(), this.sortDir()),
            filter: () => this.filter(),
            selection: () => this.selection(),
            setSelection: (next) => this.setSelection(next),
            reveal: (name) => this.setRevealRequest({ name }),
            open: (rel) => {
                void this.watchKid(rel);
                void this.loadKids(rel);
            },
            close: (rel) => this.unwatchKid(rel),
            listable: (rel) => !this.needsConsent(this.pathOf(rel)),
            maxDepth: MAX_EXPANDED,
        });
        this.entries = createMemo(() => {
            if (this.columnsOn()) return this.cols.entries();
            if (this.treeOn()) return this.treeRows().map((r) => r.entry);
            if (this.deepOn()) return this.ranked();
            const needle = this.filter().trim().toLowerCase();
            const shown = this.rawEntries().filter(
                (e) => (this.showHidden() || !e.hidden) && (needle === "" || e.name.toLowerCase().includes(needle))
            );
            return sortEntries(shown, this.sortKey(), this.sortDir());
        });
        this.order = createMemo(() => this.entries().map((e) => e.name));

        this.ops = new FilesOps(ctx.blockId);
        this.ops.onFinished = (op) => this.onOpFinished(op);
        this.unsubscribe = onMuxEvent(WpsEvent.FilesChanged, (data) => {
                const dir = data.dir;
                if (!this.path()) return;
                const inTree = samePath(dir, this.path()) ? undefined : this.expandedAt(dir);
                if (!samePath(dir, this.path()) && inTree === undefined) return;
                if (this.ctx.visibility() !== "active") {
                    this.staleWhileHidden = true;
                    return;
                }
                if (inTree !== undefined) void this.loadKids(inTree);
                else void this.list(this.path(), { silent: true });
            }, makeORef("block", ctx.blockId));
    }

    // ── Settings kept in block meta ──────────────────────────────────────────

    readonly sortKey = (): SortKey => {
        const v = this.ctx.meta()?.[META_SORT];
        return v === "modified" || v === "size" || v === "kind" ? v : "name";
    };
    readonly sortDir = (): SortDir => (this.ctx.meta()?.[META_SORTDIR] === "desc" ? "desc" : "asc");
    readonly showHidden = (): boolean => this.ctx.meta()?.[META_HIDDEN] === true;
    readonly showSidebar = (): boolean => this.ctx.meta()?.[META_SIDEBAR] !== false;
    /** The folder this pane is confined to (a container agent's workspace folder, opened by the agent):
     *  shown read-only, with previews and Open in Editor but no programs, places or changes, and never above this folder. */
    readonly root = (): string | undefined => {
        const r = this.ctx.meta()?.[META_ROOT];
        return typeof r === "string" && r !== "" ? r : undefined;
    };
    /** True, with a note in the status line, when this pane is confined and so can't do `what`. */
    refuseInRoot(what: string): boolean {
        if (!this.root()) return false;
        this.setStatus({ text: `${what} isn't available here: this folder is shown read-only from an agent's workspace.`, tone: "info" });
        return true;
    }
    /** The preview reads files through this computer's media stream, so not
     *  on a host (it would show whatever is at that path here). */
    readonly showPreview = (): boolean => this.ctx.meta()?.[META_PREVIEW] === true && !this.connection();
    /** Details (rows and columns) or a grid of thumbnails (§5.2). */
    readonly viewMode = (): "details" | "grid" | "columns" => {
        const v = this.ctx.meta()?.[META_VIEW];
        return v === "grid" || v === "columns" ? v : "details";
    };
    /** Columns (Miller) are chosen: one list per level, the folder chosen in each shown in the next. */
    readonly columnsOn = (): boolean => this.viewMode() === "columns";

    /** Clicking a column header: sort by it, or flip its direction. */
    setSort(key: SortKey): void {
        const dir: SortDir = this.sortKey() === key && this.sortDir() === "asc" ? "desc" : "asc";
        void this.ctx.setMeta({ [META_SORT]: key === "name" ? null : key, [META_SORTDIR]: dir === "asc" ? null : dir });
    }

    toggleHidden(): void {
        const next = !this.showHidden();
        void this.ctx.setMeta({ [META_HIDDEN]: next ? true : null });
        // What was indexed was indexed with the old setting; the setting is read from the pane's meta only after it
        // has been saved, so the new one is handed over.
        if (this.deep()) this.startFind(next);
    }

    toggleSidebar(): void {
        void this.ctx.setMeta({ [META_SIDEBAR]: this.showSidebar() ? false : null });
    }

    toggleViewMode(): void {
        this.leaveColumns();
        void this.ctx.setMeta({ [META_VIEW]: this.viewMode() === "grid" ? null : "grid" });
    }

    /** Chooses the columns view, or goes back to the list. */
    toggleColumns(): void {
        // Folders opened in place by the tree are not a chain: start from the shown folder alone.
        this.resetTree();
        this.setSelection(EMPTY_SELECTION);
        void this.ctx.setMeta(this.columnsOn() ? { [META_VIEW]: null } : { [META_VIEW]: "columns", [META_TREE]: null });
    }

    /** The folders opened in place and the rows chosen belong to the view being left: start the next one clean. */
    private leaveColumns(): void {
        if (!this.columnsOn()) return;
        this.resetTree();
        this.setSelection(EMPTY_SELECTION);
    }

    /** The tree view is chosen (it is the details view with folders that open in place). */
    readonly treeEnabled = (): boolean => this.ctx.meta()?.[META_TREE] === true && this.viewMode() === "details";
    /** The tree is drawn: chosen, and no filter typed (a filter shows the matches of the shown folder alone). */
    readonly treeOn = (): boolean => this.treeEnabled() && this.filter().trim() === "";

    /** Finding in subfolders is chosen (the details view only), whether or not anything is typed. */
    readonly deepMode = (): boolean => this.deep() && this.viewMode() === "details";
    /** The rows are the best matches of what is typed, from the shown folder and the folders under it. */
    readonly deepOn = (): boolean => this.deepMode() && this.filter().trim() !== "";
    /** A found row's folder, from the shown folder ("" at the first level); undefined when the rows are not found ones. */
    readonly findWhere = (name: string): string | undefined => (this.deepOn() ? whereOf(name) : undefined);

    toggleDeep(): void {
        if (this.connection()) {
            this.notOnHost("Searching subfolders");
            return;
        }
        if (this.deep()) this.stopFind();
        else {
            this.setDeep(true);
            this.startFind();
        }
    }

    /** Indexes the folders under the shown one, a few at a time, within limits; what is found shows as it comes. */
    private startFind(hidden: boolean = this.showHidden()): void {
        const run = ++this.findRun;
        this.findEntries = [];
        this.setFindVersion((n) => n + 1);
        void crawlFolders(
            (rel) => this.listForFind(rel, run),
            (batch) => {
                if (run !== this.findRun) return;
                for (const e of batch) this.findEntries.push(e);
                this.scheduleFindRefresh(run);
            },
            () => run !== this.findRun || this.disposed,
            hidden
        ).then((res) => {
            if (run !== this.findRun) return;
            this.setFindVersion((n) => n + 1);
            if (res.end === "limit") {
                this.setStatus({ text: `Searched ${res.folders} folders and stopped at the limit: some folders were not searched.`, tone: "info" }, 8000);
            }
        });
    }

    /** Ranking is redone at most every 150 ms while folders are still coming in. */
    private scheduleFindRefresh(run: number): void {
        if (this.findTimer) return;
        this.findTimer = setTimeout(() => {
            this.findTimer = null;
            if (run === this.findRun) this.setFindVersion((n) => n + 1);
        }, 150);
    }

    private stopFind(): void {
        this.findRun++;
        if (this.findTimer) clearTimeout(this.findTimer);
        this.findTimer = null;
        this.findEntries = [];
        batch(() => {
            this.setDeep(false);
            this.setFindVersion((n) => n + 1);
        });
    }

    /** One folder's entries for the index, or null when it can't be read, or is somewhere macOS asks before it is listed. */
    private async listForFind(rel: string, run: number): Promise<FsEntry[] | null> {
        const dir = rel === "" ? this.path() : this.pathOf(rel);
        if (rel !== "" && this.needsConsent(dir)) return null;
        const got: FsEntry[] = [];
        let cursor: string | undefined;
        try {
            do {
                const res = await RpcApi.FsListCommand(TabRpcClient, { path: dir, cursor, limit: PAGE, ...this.on() });
                if (run !== this.findRun || res.error) return run !== this.findRun || got.length === 0 ? null : got;
                got.push(...res.entries);
                cursor = res.cursor ?? undefined;
            } while (cursor && got.length < 5 * PAGE);
        } catch {
            return got.length > 0 ? got : null;
        }
        return got;
    }

    toggleTree(): void {
        if (this.treeEnabled()) {
            this.resetTree();
            void this.ctx.setMeta({ [META_TREE]: null });
        } else {
            this.leaveColumns();
            void this.ctx.setMeta({ [META_TREE]: true, [META_VIEW]: null });
        }
    }

    togglePreview(): void {
        void this.ctx.setMeta({ [META_PREVIEW]: this.showPreview() ? null : true });
    }

    /** Narrows the list to names containing `text` (case-insensitive). Per
     *  folder: navigating clears it. */
    setFilter(text: string): void {
        this.setFilterSignal(text);
    }

    // ── Start ────────────────────────────────────────────────────────────────

    /** Where a new pane starts: its saved folder, or Home (§9.1.5: never
     *  Documents or Desktop). Restoring never lists a protected macOS folder
     *  on its own (§9.1.4 item 7). */
    async start(): Promise<void> {
        const saved = this.ctx.meta()?.[META_PATH];
        if (!this.root()) await this.loadPlaces();
        // On a host, its own home (`~`, which srv resolves there).
        const home = this.connection() ? "~" : (this.places().find((p) => p.kind === "home")?.path ?? "~");
        const target = typeof saved === "string" && saved !== "" ? saved : home;
        await this.navigate(target, { push: false, restore: true });
        void this.loadAgents();
        void this.loadRemotes();
    }

    private async loadPlaces(): Promise<void> {
        try {
            const res = await RpcApi.FsPlacesCommand(TabRpcClient, {});
            if (!this.disposed) this.setPlaces(res.places);
        } catch {
            // Places stay empty; the breadcrumb still works.
        }
    }

    /** The SSH hosts in Remotes (pinned, then ssh config and settings, then
     *  recent; not hidden), with their nicknames and colours, for the Remote
     *  section of Places (SPEC_REMOTES_PANE_2026_10_05.md §4.7). */
    private async loadRemotes(): Promise<void> {
        try {
            const records = await RpcApi.RemotesListCommand(TabRpcClient, { timeout: 5000 });
            if (!this.disposed) this.setRemotes(filesRemotes(records ?? []));
        } catch {
            // No Remote section.
        }
    }

    // ── Connections (remote terminals spec §6.3) ──────────────────────────────

    /** Where a file request runs: the SSH connection shown, and this pane
     *  for any ssh prompt; nothing for this computer. */
    private on(): { connection?: string; block_id?: string } {
        const c = this.connection();
        // Named for a confined pane too: srv checks every listing and change against its root.
        return c ? { connection: c, block_id: this.blockId } : this.root() ? { block_id: this.blockId } : {};
    }

    /** Shows `path` on `connection` (an SSH host, or "" for this computer).
     *  A different connection starts afresh: history, undo and the watcher
     *  belong to the folders they were made in. */
    async openOn(connection: string, path: string): Promise<void> {
        const next = isSshConnection(connection) ? connection.trim() : "";
        const changed = next !== this.connection();
        if (changed) {
            this.stopWatching();
            this.back = [];
            this.forward = [];
            this.undoStack = [];
            this.setHistoryVersion((n) => n + 1);
            this.setConnection(next);
            void this.ctx.setMeta({ [META_CONN]: next || null });
        }
        // On the same connection, a Places click is a navigation like any
        // other: Back returns to the folder it left.
        await this.navigate(path, { push: !changed });
    }

    /** What a remote pane can't do yet, said once. */
    notOnHost(what: string): void {
        this.setStatus({ text: `${what} isn't available on ${this.connection()} yet.`, tone: "error" });
    }

    /** Each named agent's working folder, for the Agents section of Places
     *  (§8.3): "where did that agent put things?" in one click. */
    private async loadAgents(): Promise<void> {
        try {
            const rows = await RpcApi.ListNamedAgentsCommand(TabRpcClient, { limit: 200 });
            const seen = new Set<string>();
            const picked: { name: string; path: string; definitionId: string }[] = [];
            // Most recently used first: an agent shows once, by its newest use.
            for (const r of [...rows].sort(compareLastUsed)) {
                const dir = r.working_directory;
                if (!dir || seen.has(dir.toLowerCase())) continue;
                seen.add(dir.toLowerCase());
                picked.push({
                    name: r.instance_name || r.definition_name || baseName(dir),
                    path: dir,
                    definitionId: r.definition_id ?? "",
                });
            }
            const agents: AgentPlace[] = await Promise.all(
                picked.map(async ({ name, path, definitionId }) => ({
                    name,
                    path,
                    color: await loadAgentColor(definitionId),
                }))
            );
            if (!this.disposed) this.setAgents(agents);
        } catch {
            // No agents section.
        }
    }

    // ── macOS access prompts (§9.1.4) ────────────────────────────────────────

    /** The protected macOS place `path` is in, if any. */
    protectedPlace(raw: string): FsPlace | null {
        if (!isMacOS() || this.connection()) return null;
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
    async navigate(target: string, opts: { push?: boolean; consented?: boolean; restore?: boolean } = {}): Promise<void> {
        const root = this.root();
        if (root && !isWithin(target, root)) {
            this.setStatus({ text: "This pane shows only the agent's workspace folder it was opened on.", tone: "info" });
            return;
        }
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
            this.setFilterSignal("");
            this.setGit(null);
        });
        this.resetTree();
        if (this.deep()) this.stopFind();
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
        // A folder the person opened (not the one a restored pane came back to, nor one reached by Back or Forward) goes into Places' recents, once
        // its rows are on screen: a folder that couldn't be listed, or is waiting on macOS, is not one.
        if (!opts.restore && !this.root() && this.phase() === "ready") addRecent({ path: this.path(), connection: this.connection() || undefined });
    }

    goBack(): void {
        const prev = this.back.pop();
        if (prev == null) return;
        this.forward.push(this.path());
        this.setHistoryVersion((n) => n + 1);
        void this.navigate(prev, { push: false, restore: true });
    }

    goForward(): void {
        const next = this.forward.pop();
        if (next == null) return;
        this.back.push(this.path());
        this.setHistoryVersion((n) => n + 1);
        void this.navigate(next, { push: false, restore: true });
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
        for (const rel of this.expanded()) void this.loadKids(rel);
        if (this.deep()) this.startFind();
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
                const res = await RpcApi.FsListCommand(
                    TabRpcClient,
                    { path: dir, cursor, limit: PAGE, ...this.on() },
                    // Over ssh, which may first ask the user something.
                    this.connection() ? { timeout: 180_000 } : undefined
                );
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
                this.setSelection(pruneSelection(this.selection(), opts.silent ? oldOrder : before, this.orderKeepingNested()));
            });
            this.dropGoneFolders();
            this.applyRequestedSelection();
            void this.watch(this.path());
            this.refreshGit();
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

    /** The order, with the selected rows inside open folders added: this listing is of the shown folder alone, and
     *  those rows are checked when their own folder is listed again (`loadKids`). */
    private orderKeepingNested(): string[] {
        const sel = this.selection();
        const nested = [...sel.names, sel.focus, sel.anchor].filter((n): n is string => n != null && n.includes("/"));
        return nested.length > 0 ? [...this.order(), ...nested] : this.order();
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
        // A host's folders aren't watched: Refresh (F5) shows changes there.
        if (this.connection()) return;
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

    /** The path of a row: its name is a path from the shown folder in the tree view (`src/a.ts`). */
    pathOf(name: string): string {
        return joinRel(this.path(), name);
    }

    async rename(name: string, newName: string): Promise<boolean> {
        if (this.refuseInRoot("Renaming")) return false;
        if (newName === labelOf(name)) return true;
        const from = this.pathOf(name);
        try {
            const res = await RpcApi.FsRenameCommand(TabRpcClient, { path: from, new_name: newName, ...this.on() });
            this.undoStack.push({ kind: "rename", from, to: res.new_path });
            const renamed = parentRel(name) ? `${parentRel(name)}/${newName}` : newName;
            this.setSelection({ names: new Set([renamed]), focus: renamed, anchor: renamed });
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
        if (this.phase() !== "ready" || this.refuseInRoot("Creating")) return;
        const base = kind === "dir" ? "New folder" : "New file";
        const ext = kind === "dir" ? "" : ".txt";
        const taken = new Set(this.rawEntries().map((e) => e.name.toLowerCase()));
        let name = `${base}${ext}`;
        for (let i = 2; taken.has(name.toLowerCase()); i++) name = `${base} (${i})${ext}`;
        try {
            const res = await RpcApi.FsCreateCommand(TabRpcClient, { parent: this.path(), name, kind, ...this.on() });
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
        if (entries.length === 0 || this.refuseInRoot("Moving to the Trash")) return;
        if (this.connection()) {
            this.setStatus({ text: `${this.connection()} has no Trash: Shift+Delete deletes permanently.`, tone: "info" });
            return;
        }
        const paths = entries.map((e) => this.pathOf(e.name));
        try {
            const res = await RpcApi.FsTrashCommand(TabRpcClient, { paths, block_id: this.blockId });
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
        if (entries.length === 0 || this.refuseInRoot("Deleting")) return;
        try {
            const res = await RpcApi.FsDeleteCommand(TabRpcClient, {
                paths: entries.map((e) => this.pathOf(e.name)),
                ...this.on(),
            });
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

    /** Ctrl+Z, or Undo in the status line: reverses the last rename, move, new
     *  item, or move to Trash. */
    async undo(): Promise<void> {
        if (this.refuseInRoot("Undo")) return;
        const last = this.undoStack.pop();
        if (!last) {
            this.setStatus({ text: "Nothing to undo", tone: "info" }, 2500);
            return;
        }
        try {
            if (last.kind === "rename") {
                await RpcApi.FsRenameCommand(TabRpcClient, { path: last.to, new_name: baseName(last.from), ...this.on() });
                this.setStatus({ text: `Renamed back to ${baseName(last.from)}`, tone: "info" });
            } else if (last.kind === "move") {
                await this.undoMove(last.items);
            } else if (last.kind === "create" && this.connection()) {
                // Undoing a new item moves it to the Trash, and a host has
                // none: deleting it for good is not an undo.
                this.setStatus({ text: `Can't undo creating ${baseName(last.path)}: ${this.connection()} has no Trash`, tone: "error" });
            } else if (last.kind === "create") {
                const res = await RpcApi.FsTrashCommand(TabRpcClient, { paths: [last.path], block_id: this.blockId });
                const problem = failures(res.results);
                this.setStatus(
                    problem ? { text: `Couldn't undo: ${problem}`, tone: "error" } : { text: `Moved ${baseName(last.path)} to Trash`, tone: "info" }
                );
            } else {
                const res = await RpcApi.FsRestoreCommand(TabRpcClient, { paths: last.paths, block_id: this.blockId });
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

    // ── Clipboard and transfers (§7.1) ─────────────────────────────────────

    /** Ctrl+C / Ctrl+X: the selection onto the shared Files clipboard. */
    copyToClipboard(kind: "copy" | "cut", entries: FsEntry[]): void {
        if (this.refuseInRoot("Copying")) return;
        if (entries.length === 0) return;
        setClipboard({ kind, paths: entries.map((e) => this.pathOf(e.name)), connection: this.connection() || undefined });
        const what = entries.length === 1 ? entries[0].name : `${entries.length} items`;
        this.setStatus({ text: kind === "cut" ? `Cut ${what}: paste to move it` : `Copied ${what}: paste to copy it`, tone: "info" }, 3000);
    }

    /** Ctrl+V: copy (or move, after Cut) the clipboard into the folder shown. */
    async paste(): Promise<void> {
        if (this.refuseInRoot("Pasting")) return;
        const c = clipboard();
        if (!c || c.paths.length === 0) {
            this.setStatus({ text: "Nothing to paste", tone: "info" }, 2500);
            return;
        }
        if (this.phase() !== "ready") return;
        // A cut is used up only once srv has taken the move: a refused one
        // (protected place, folder into itself) keeps it (ReAgent on #4221).
        // The clipboard's paths are on the connection they were cut or
        // copied on, which may be another host than this pane's.
        const started = await this.transfer(c.kind === "cut" ? "move" : "copy", c.paths, this.path(), c.connection ?? "");
        if (started && c.kind === "cut" && clipboard() === c) setClipboard(null);
    }

    /** Copy or move `sources` (on `sourceConnection`, "" for this computer)
     *  into the folder shown, on this pane's connection. */
    async transfer(
        kind: "copy" | "move",
        sources: string[],
        destDir: string = this.path(),
        sourceConnection: string = ""
    ): Promise<boolean> {
        if (this.refuseInRoot("Copying and moving")) return false;
        // Where it goes is fixed now: the pane may be moved to another host while the destination is being read.
        const connection = this.connection();
        try {
            // A move on this computer is undoable when the names in the destination show where it will land.
            const planned = kind === "move" && !connection && !sourceConnection ? await this.planUndo(sources, destDir) : null;
            if (this.connection() !== connection) {
                this.setStatus({ text: "The pane changed host before the move began, so nothing was moved.", tone: "info" });
                return false;
            }
            await this.ops.start(
                kind,
                sources,
                destDir,
                {
                    sourceConnection: sourceConnection || undefined,
                    connection: connection || undefined,
                },
                planned ?? undefined
            );
            return true;
        } catch (err) {
            this.setStatus({ text: errorText(err), tone: "error" });
            return false;
        }
    }

    private namesIn(dir: string): Promise<Set<string> | null> {
        return namesIn(dir, this.on());
    }

    /** What a finished copy or move says, and Undo for a move that landed as planned. */
    private onOpFinished(op: OpView): void {
        const text = opFinishedText(op);
        const ms = op.state === "done" && !op.failures?.length ? 4000 : 10000;
        this.setStatus(text, ms);
        // Undo is offered once the destination shows the move landed as planned.
        void this.noteMoveForUndo(op).then((undoable) => {
            if (undoable) this.setStatus({ ...text, undo: true }, ms);
        });
        // The watcher re-lists the folder shown; a move OUT of it (or a
        // copy into a folder this pane isn't watching) may not be seen.
        this.refresh();
    }

    private childNamesOf(path: string): Promise<Set<string> | "file" | null> {
        return childNamesOf(path, this.on());
    }

    private async planUndo(sources: string[], destDir: string): Promise<UndoPlan | null> {
        const destNames = await this.namesIn(destDir);
        const items = destNames && sources.length <= MAX_UNDO_ITEMS ? planMoveUndo(sources, destDir, destNames) : null;
        if (!items || !destNames) return null;
        const children = new Map<string, ReadonlySet<string>>();
        for (const item of items) {
            const names = await this.childNamesOf(item.from);
            if (names === null) return null;
            if (names !== "file") children.set(baseName(item.from).toLowerCase(), names);
        }
        return { items, destDir, before: destNames, children };
    }

    /** A finished move that went to plan joins the undo stack, once the destination shows it landed as planned
     *  (a conflict answered while the job ran puts an item under another name). True when it did. */
    private async noteMoveForUndo(op: { kind: string; state: string; tag?: unknown; failures?: { path: string }[] }): Promise<boolean> {
        const plan = op.tag as UndoPlan | undefined;
        if (op.kind !== "move" || op.state !== "done" || !plan?.items || this.connection()) return false;
        const items = movedOk(plan.items, op.failures?.map((f) => f.path));
        if (items.length === 0) return false;
        const after = await this.namesIn(plan.destDir);
        const childrenAfter = new Map<string, ReadonlySet<string> | null>();
        for (const key of plan.children.keys()) {
            const item = items.find((i) => baseName(i.to).toLowerCase() === key);
            if (!item) continue;
            const names = await this.childNamesOf(item.to);
            childrenAfter.set(key, names === "file" ? null : names);
        }
        // The pane may have been moved to a host while the destination was being read: the stack was cleared then.
        if (this.connection() || !landedAsPlanned(plan, items, after, childrenAfter)) return false;
        this.undoStack.push({ kind: "move", items });
        return true;
    }

    /** Ctrl+Z on a move: each item back where it came from, if it is still where the move left it. */
    private async undoMove(items: MovedItem[]): Promise<void> {
        const dirs = new Set<string>();
        for (const i of items) for (const p of [parentOf(i.from), parentOf(i.to)]) if (p) dirs.add(p);
        const listed = new Map<string, Set<string> | null>();
        for (const d of dirs) listed.set(d, await this.namesIn(d));
        const groups = restorable(items, (d) => listed.get(d) ?? null);
        if (groups.length === 0) {
            this.setStatus({ text: "Can't undo that move: the items aren't where it left them.", tone: "error" });
            return;
        }
        // Each group moves back as its own job; srv asks if something has since taken a name.
        for (const g of groups) await this.ops.start("move", g.paths, g.parent);
    }

    /** What dropping `sources` here does: move within a drive when the drag
     *  came from another Files pane, copy otherwise (as file managers do). */
    dropKind(sources: string[], fromFiles: boolean, destDir: string = this.path()): "copy" | "move" {
        // Onto a host: always a copy. The sources are on this computer (drags
        // out of a host pane are off), and their roots looking alike (`/` and
        // `/`) says nothing about it being one disk.
        if (this.connection()) return "copy";
        return fromFiles && sameVolume(sources, destDir) ? "move" : "copy";
    }

    // ── Tree view: folders opened in place ───────────────────────────────────

    /** Forgets every opened folder and stops watching them (the folder shown changed, or the tree was turned off). */
    private resetTree(): void {
        this.cols.reset();
        if (this.expanded().size === 0 && this.treeWatches.size === 0) return;
        for (const rel of [...this.treeWatches.keys()]) this.unwatchKid(rel);
        this.treeWatching.clear();
        batch(() => {
            this.setExpanded(new Set());
            this.setKids(new Map());
        });
    }

    /** The opened folder that is `dir`, by its path from the shown folder. */
    private expandedAt(dir: string): string | undefined {
        for (const rel of this.expanded()) if (samePath(this.pathOf(rel), dir)) return rel;
        return undefined;
    }

    /** Opens the folder row `rel` in place. A protected macOS folder is opened (the pane's notice) instead. */
    async expand(rel: string): Promise<void> {
        const row = this.treeInfo().get(rel);
        if (!row?.expandable || row.expanded) return;
        const path = this.pathOf(rel);
        if (this.needsConsent(path)) {
            void this.navigate(path);
            return;
        }
        if (this.expanded().size >= MAX_EXPANDED) {
            this.setStatus({ text: `Only ${MAX_EXPANDED} folders can stay open in place: close one first.`, tone: "info" });
            return;
        }
        this.setExpanded(new Set([...this.expanded(), rel]));
        void this.watchKid(rel);
        await this.loadKids(rel);
    }

    /** Closes the folder row `rel`, and the folders opened inside it. */
    collapse(rel: string): void {
        if (!this.expanded().has(rel)) return;
        const gone = [...this.expanded()].filter((r) => isUnderRel(r, rel));
        const next = new Set(this.expanded());
        const kids = new Map(this.kids());
        for (const r of gone) {
            next.delete(r);
            kids.delete(r);
            this.unwatchKid(r);
        }
        batch(() => {
            // Rows that were inside it are gone from the list: the selection falls back to the folder.
            const sel = this.selection();
            const inside = (n: string | null | undefined): boolean => n != null && n.startsWith(`${rel}/`);
            if ([...sel.names].some(inside) || inside(sel.focus) || inside(sel.anchor)) {
                const names = new Set([...sel.names].filter((n) => !inside(n)));
                names.add(rel);
                this.setSelection({ names, focus: rel, anchor: rel });
            }
            this.setExpanded(next);
            this.setKids(kids);
        });
    }

    toggleExpanded(rel: string): void {
        if (this.expanded().has(rel)) this.collapse(rel);
        else void this.expand(rel);
    }

    /** Right and Left in the tree: open the focused folder or step into it; close it or step out to its folder (the keys of
     *  app/fs/tree/tree-keys.ts, which the Editor's tree answers the same way). */
    treeArrow(dir: "right" | "left"): void {
        const focus = this.selection().focus;
        if (!focus || !this.treeInfo().has(focus)) return;
        const rows = this.treeRows().map((r) => ({
            id: r.entry.name,
            expandable: r.expandable,
            expanded: r.expanded,
            parent: r.depth > 0 ? parentRel(r.entry.name) : null,
        }));
        const move = treeMove(rows, focus, dir === "right" ? "ArrowRight" : "ArrowLeft", 1);
        if (move?.kind === "expand") void this.expand(move.id);
        else if (move?.kind === "collapse") this.collapse(move.id);
        else if (move?.kind === "select") {
            this.setSelection({ names: new Set([move.id]), focus: move.id, anchor: move.id });
            this.setRevealRequest({ name: move.id });
        }
    }

    /** Closes an opened folder whose row is gone from the listing of the folder it was in (renamed, moved or deleted),
     *  with the folders opened inside it. Not an error: the person's own operation did it. */
    private dropGoneFolders(): void {
        for (const rel of this.expanded()) {
            if (!this.expanded().has(rel)) continue;
            const parent = parentRel(rel);
            const list = parent === "" ? this.rawEntries() : this.kids().get(parent);
            if (list && !list.some((e) => e.name === labelOf(rel) && e.is_dir)) this.collapse(rel);
        }
    }

    /** Lists the opened folder `rel` (all its pages) and keeps what it holds. A folder that can't be listed closes. */
    private async loadKids(rel: string): Promise<void> {
        const root = this.path();
        // A listing that a later one of the same folder overtook is dropped, so an older answer can't overwrite a newer one.
        const mine = ++this.kidsLoadSeq;
        this.kidsLoad.set(rel, mine);
        const dir = this.pathOf(rel);
        const collected: FsEntry[] = [];
        let cursor: string | undefined;
        try {
            do {
                const res = await RpcApi.FsListCommand(TabRpcClient, { path: dir, cursor, limit: PAGE, ...this.on() }, this.connection() ? { timeout: 180_000 } : undefined);
                if (this.disposed || !samePath(root, this.path()) || !this.expanded().has(rel) || this.kidsLoad.get(rel) !== mine) return;
                if (res.error) {
                    this.collapse(rel);
                    // A folder that is gone was renamed, moved or deleted: closed, not an error.
                    if (res.error.kind !== "not_found") this.setStatus({ text: `Couldn't open ${labelOf(rel)}: ${res.error.message}`, tone: "error" });
                    return;
                }
                collected.push(...res.entries);
                cursor = res.cursor ?? undefined;
            } while (cursor);
        } catch (err) {
            if (!this.disposed && samePath(root, this.path()) && this.expanded().has(rel)) {
                this.collapse(rel);
                this.setStatus({ text: `Couldn't open ${labelOf(rel)}: ${errorText(err)}`, tone: "error" });
            }
            return;
        }
        if (this.kidsLoad.get(rel) !== mine) return;
        const next = new Map(this.kids());
        next.set(rel, collected);
        batch(() => {
            const before = this.order();
            this.setKids(next);
            this.setSelection(pruneSelection(this.selection(), before, this.order()));
        });
        this.dropGoneFolders();
    }

    private async watchKid(rel: string): Promise<void> {
        // A host's folders aren't watched: Refresh (F5) shows changes there.
        if (this.connection() || this.treeWatches.has(rel) || this.treeWatching.has(rel)) return;
        this.treeWatching.add(rel);
        const path = this.pathOf(rel);
        try {
            const res = await RpcApi.FsWatchCommand(TabRpcClient, { path, block_id: this.blockId });
            if (this.disposed || !this.treeWatching.has(rel) || !this.expanded().has(rel) || this.treeWatches.has(rel)) {
                void RpcApi.FsUnwatchCommand(TabRpcClient, { watch_id: res.watch_id }).catch(() => {});
            } else {
                this.treeWatches.set(rel, res.watch_id);
            }
        } catch {
            // No live updates for this folder; Refresh still works.
        } finally {
            this.treeWatching.delete(rel);
        }
    }

    private unwatchKid(rel: string): void {
        this.treeWatching.delete(rel);
        const id = this.treeWatches.get(rel);
        this.treeWatches.delete(rel);
        if (id) void RpcApi.FsUnwatchCommand(TabRpcClient, { watch_id: id }).catch(() => {});
    }

    // ── Git markers ──────────────────────────────────────────────────────────

    /** Ask git about the folder shown, once things settle: a burst of
     *  changes on disk (a build, a checkout) re-lists many times, and each
     *  `git status` is a process. The answer for a folder the pane has left
     *  is dropped. */
    private refreshGit(): void {
        if (this.gitTimer) clearTimeout(this.gitTimer);
        if (this.root()) return;
        // git runs on this computer, not on the host.
        if (this.connection()) {
            this.setGit(null);
            return;
        }
        const dir = this.path();
        const gen = ++this.gitGeneration;
        this.gitTimer = setTimeout(() => {
            this.gitTimer = null;
            RpcApi.FsGitStatusCommand(TabRpcClient, { path: dir, block_id: this.blockId }).then(
                (g) => {
                    if (gen !== this.gitGeneration || this.disposed || !samePath(dir, this.path())) return;
                    this.setGit(g.in_repo && !g.error ? g : null);
                },
                () => {
                    if (gen === this.gitGeneration) this.setGit(null);
                }
            );
        }, GIT_DELAY_MS);
    }

    dispose(): void {
        this.findRun++;
        if (this.findTimer) clearTimeout(this.findTimer);
        for (const rel of [...this.treeWatches.keys()]) this.unwatchKid(rel);
        if (this.gitTimer) clearTimeout(this.gitTimer);
        this.ops.dispose();
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
