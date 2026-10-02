// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Document tabs: the documents inside one pane (an Editor's files, a
 * Hangar's folders, a Media pane's files, a Browser's pages), on one shared
 * model every such pane type uses. The third layer of tab, below window tabs
 * and pane tabs. docs/specs/SPEC_DOCUMENT_TABS_2026_10_02.md §1, §5.
 *
 * This module is pure: the state, and the commands that change it. The
 * controller (doc-tabs-controller.ts) holds it for a pane and persists it.
 */

/** One document tab. `P` is the pane type's own data about the document. */
export interface DocTab<P> {
    /** Stable for the tab's life (not across restarts). */
    id: string;
    /** The document's identity (a path, a URL): opening an already-open key
     *  activates that tab instead of adding another. */
    key: string;
    title: string;
    icon?: string;
    /** Replaced by the next preview open (§4.2). At most one per pane. */
    preview: boolean;
    pinned: boolean;
    /** Unsaved changes (the Editor's buffers). */
    dirty?: boolean;
    payload: P;
}

export interface DocTabsState<P> {
    /** Display order: pinned tabs first. */
    tabs: DocTab<P>[];
    activeId: string | null;
    /** Most recently active first: closing the active tab activates the
     *  one used before it. */
    mru: string[];
    /** Recently closed, newest first (Ctrl+Shift+T). */
    closed: DocTab<P>[];
}

/** Closed tabs remembered per pane. */
export const MAX_CLOSED = 10;

export function emptyDocTabs<P>(): DocTabsState<P> {
    return { tabs: [], activeId: null, mru: [], closed: [] };
}

let nextId = 0;
/** A fresh tab id. */
export function newDocTabId(): string {
    return `dt-${Date.now().toString(36)}-${(nextId++).toString(36)}`;
}

export function activeTab<P>(s: DocTabsState<P>): DocTab<P> | undefined {
    return s.tabs.find((t) => t.id === s.activeId);
}

function withActive<P>(s: DocTabsState<P>, id: string | null): DocTabsState<P> {
    if (id == null) return { ...s, activeId: null };
    return { ...s, activeId: id, mru: [id, ...s.mru.filter((m) => m !== id)] };
}

/** Pinned tabs before unpinned, each group keeping its order. */
function pinnedFirst<P>(tabs: DocTab<P>[]): DocTab<P>[] {
    return [...tabs.filter((t) => t.pinned), ...tabs.filter((t) => !t.pinned)];
}

export interface OpenArgs<P> {
    key: string;
    title: string;
    icon?: string;
    payload: P;
    /** A preview tab (§4.2): replaces the pane's current preview. */
    preview?: boolean;
    /** Default true. */
    activate?: boolean;
    /** Where to put a new tab: after the active one (default) or at the end. */
    at?: "afterActive" | "end";
}

/**
 * Open a document. An open key activates its tab (a non-preview open of a
 * preview tab makes it a normal tab). A preview open replaces the existing
 * preview tab, in its place. Otherwise a new tab goes after the active one.
 */
export function openDoc<P>(s: DocTabsState<P>, a: OpenArgs<P>, id: string = newDocTabId()): DocTabsState<P> {
    const activate = a.activate !== false;
    const existing = s.tabs.find((t) => t.key === a.key);
    if (existing) {
        const tabs = s.tabs.map((t) => (t.id === existing.id && t.preview && !a.preview ? { ...t, preview: false } : t));
        const next = { ...s, tabs };
        return activate ? withActive(next, existing.id) : next;
    }
    const tab: DocTab<P> = { id, key: a.key, title: a.title, icon: a.icon, preview: !!a.preview, pinned: false, payload: a.payload };
    let tabs = s.tabs;
    const preview = a.preview ? tabs.find((t) => t.preview) : undefined;
    let mru = s.mru;
    if (preview) {
        // The new preview takes the old one's place.
        tabs = tabs.map((t) => (t.id === preview.id ? tab : t));
        mru = mru.filter((m) => m !== preview.id);
    } else {
        const after = a.at === "end" ? -1 : tabs.findIndex((t) => t.id === s.activeId);
        const at = after < 0 ? tabs.length : after + 1;
        tabs = [...tabs.slice(0, at), tab, ...tabs.slice(at)];
    }
    const next = { ...s, tabs: pinnedFirst(tabs), mru };
    if (activate || s.activeId == null || (preview && s.activeId === preview.id)) return withActive(next, id);
    return next;
}

export function activateDoc<P>(s: DocTabsState<P>, id: string): DocTabsState<P> {
    return s.tabs.some((t) => t.id === id) ? withActive(s, id) : s;
}

/**
 * Close a tab. Refused (returns `s` unchanged) when `keepOne` and it is the
 * last tab. The closed tab goes on the reopen list unless it was a preview.
 * The tab used before it becomes active.
 */
export function closeDoc<P>(s: DocTabsState<P>, id: string, opts: { keepOne?: boolean } = {}): DocTabsState<P> {
    const tab = s.tabs.find((t) => t.id === id);
    if (!tab || (opts.keepOne && s.tabs.length <= 1)) return s;
    const tabs = s.tabs.filter((t) => t.id !== id);
    const mru = s.mru.filter((m) => m !== id);
    const closed = tab.preview ? s.closed : [tab, ...s.closed].slice(0, MAX_CLOSED);
    let activeId = s.activeId;
    if (activeId === id) {
        const index = s.tabs.findIndex((t) => t.id === id);
        activeId = mru.find((m) => tabs.some((t) => t.id === m)) ?? tabs[Math.min(index, tabs.length - 1)]?.id ?? null;
    }
    return withActive({ ...s, tabs, mru, closed }, activeId);
}

/** Close every tab but `id` (pinned tabs stay). */
export function closeOthers<P>(s: DocTabsState<P>, id: string): DocTabsState<P> {
    let next = s;
    for (const t of s.tabs) if (t.id !== id && !t.pinned) next = closeDoc(next, t.id);
    return activateDoc(next, id);
}

/** Close the unpinned tabs to the right of `id`. */
export function closeToRight<P>(s: DocTabsState<P>, id: string): DocTabsState<P> {
    const at = s.tabs.findIndex((t) => t.id === id);
    if (at < 0) return s;
    let next = s;
    for (const t of s.tabs.slice(at + 1)) if (!t.pinned) next = closeDoc(next, t.id);
    return next;
}

/** Reopen the most recently closed tab (refused if its key is open again). */
export function reopenClosed<P>(s: DocTabsState<P>, id: string = newDocTabId()): DocTabsState<P> {
    const [tab, ...rest] = s.closed;
    if (!tab) return s;
    const reopened = openDoc({ ...s, closed: rest }, { key: tab.key, title: tab.title, icon: tab.icon, payload: tab.payload }, id);
    return reopened;
}

/** Move a tab by `delta` places, within its pinned or unpinned group. */
export function moveDoc<P>(s: DocTabsState<P>, id: string, delta: number): DocTabsState<P> {
    const from = s.tabs.findIndex((t) => t.id === id);
    if (from < 0) return s;
    const tab = s.tabs[from];
    const group = s.tabs.filter((t) => t.pinned === tab.pinned);
    const inGroup = group.findIndex((t) => t.id === id);
    const to = Math.max(0, Math.min(group.length - 1, inGroup + delta));
    if (to === inGroup) return s;
    const reordered = [...group];
    reordered.splice(inGroup, 1);
    reordered.splice(to, 0, tab);
    const others = s.tabs.filter((t) => t.pinned !== tab.pinned);
    return { ...s, tabs: tab.pinned ? [...reordered, ...others] : [...others, ...reordered] };
}

export function setPinned<P>(s: DocTabsState<P>, id: string, pinned: boolean): DocTabsState<P> {
    const tabs = s.tabs.map((t) => (t.id === id ? { ...t, pinned, preview: pinned ? false : t.preview } : t));
    return { ...s, tabs: pinnedFirst(tabs) };
}

/** A preview tab becomes a normal one (edited, double-clicked). */
export function promoteDoc<P>(s: DocTabsState<P>, id: string): DocTabsState<P> {
    return { ...s, tabs: s.tabs.map((t) => (t.id === id ? { ...t, preview: false } : t)) };
}

/** Change what a tab shows: its title, icon, dirty mark, payload, or key
 *  (a Hangar tab navigating to another folder). */
export function updateDoc<P>(
    s: DocTabsState<P>,
    id: string,
    patch: Partial<Pick<DocTab<P>, "title" | "icon" | "dirty" | "payload" | "key">>
): DocTabsState<P> {
    return { ...s, tabs: s.tabs.map((t) => (t.id === id ? { ...t, ...patch } : t)) };
}

/** The tab `delta` places from the active one, wrapping (Ctrl+Tab). */
export function cycleDoc<P>(s: DocTabsState<P>, delta: number): DocTabsState<P> {
    if (s.tabs.length < 2) return s;
    const at = Math.max(0, s.tabs.findIndex((t) => t.id === s.activeId));
    const next = s.tabs[(((at + delta) % s.tabs.length) + s.tabs.length) % s.tabs.length];
    return withActive(s, next.id);
}

// ── Persistence (§5.3) ──────────────────────────────────────────────────

/** The block-meta key holding a pane's document tabs. */
export const DOC_TABS_META = "doctabs";

export interface PersistedDocTabs {
    v: 1;
    /** Index of the active tab in `tabs`. */
    active: number;
    tabs: { key: string; title: string; icon?: string; pinned?: boolean; state: unknown }[];
}

/** What is written to block meta: no ids, no previews, no closed list, and
 *  each payload through the type's `serialize`. */
export function persistDocTabs<P>(s: DocTabsState<P>, serialize: (p: P) => unknown): PersistedDocTabs {
    const kept = s.tabs.filter((t) => !t.preview || t.id === s.activeId);
    return {
        v: 1,
        active: Math.max(0, kept.findIndex((t) => t.id === s.activeId)),
        tabs: kept.map((t) => ({ key: t.key, title: t.title, ...(t.icon ? { icon: t.icon } : {}), ...(t.pinned ? { pinned: true } : {}), state: serialize(t.payload) })),
    };
}

/** Rebuild state from block meta; null when there is nothing usable (an
 *  unknown version, no tabs, or none `deserialize` accepts). */
export function hydrateDocTabs<P>(raw: unknown, deserialize: (state: unknown) => P | null): DocTabsState<P> | null {
    const p = raw as Partial<PersistedDocTabs> | null | undefined;
    if (!p || p.v !== 1 || !Array.isArray(p.tabs)) return null;
    const tabs: DocTab<P>[] = [];
    let activeId: string | null = null;
    p.tabs.forEach((t, i) => {
        if (!t || typeof t.key !== "string") return;
        const payload = deserialize(t.state);
        if (payload == null) return;
        const id = newDocTabId();
        tabs.push({ id, key: t.key, title: typeof t.title === "string" ? t.title : t.key, icon: t.icon, preview: false, pinned: !!t.pinned, payload });
        if (i === p.active) activeId = id;
    });
    if (tabs.length === 0) return null;
    const active = activeId ?? tabs[0].id;
    return { tabs: pinnedFirst(tabs), activeId: active, mru: [active], closed: [] };
}
