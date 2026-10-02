// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * A pane's document tabs: the state (doc-tabs.ts) held in a signal, saved
 * to the block's meta as it changes, restored from it, and the keys every
 * document-tab pane shares. docs/specs/SPEC_DOCUMENT_TABS_2026_10_02.md §4.3,
 * §5.2, §5.3, §5.6.
 */

import { createSignal, type Accessor } from "solid-js";
import {
    activateDoc,
    activeTab,
    closeDoc,
    closeOthers,
    closeToRight,
    cycleDoc,
    DOC_TABS_META,
    emptyDocTabs,
    hydrateDocTabs,
    moveDoc,
    openDoc,
    persistDocTabs,
    promoteDoc,
    reopenClosed,
    setPinned,
    updateDoc,
    type DocTab,
    type DocTabsState,
    type OpenArgs,
} from "./doc-tabs";

/** What a pane type says about its documents (§5.6). */
export interface DocTabsSpec<P> {
    keyOf(payload: P): string;
    titleOf(payload: P): string;
    iconOf?(payload: P): string | undefined;
    /** To block meta: small and JSON-safe. */
    serialize(payload: P): unknown;
    /** From block meta; null drops the tab. */
    deserialize(state: unknown): P | null;
    /** Ctrl+T and the strip's "+": the new document, given the active one. */
    newDocument?(active: P | undefined): P | null;
    /** The pane always shows a document: its last tab can't be closed. */
    keepOne?: boolean;
    /** Show the strip with one tab too (the Editor). */
    alwaysShowStrip?: boolean;
}

/** Where a controller keeps its state between runs: the block's meta. */
export interface DocTabsHost {
    meta(): Record<string, unknown> | undefined;
    setMeta(patch: Record<string, unknown>): Promise<void> | void;
}

/** Writes to block meta are batched this long. */
const SAVE_DELAY_MS = 300;

export class DocTabsController<P> {
    readonly state: Accessor<DocTabsState<P>>;
    private readonly setState: (s: DocTabsState<P>) => void;
    private saveTimer: ReturnType<typeof setTimeout> | null = null;
    /** Told when a tab is closed (and not reopenable as itself), so the pane
     *  can release what it held for it. */
    onClosed: ((tab: DocTab<P>) => void) | null = null;

    constructor(
        readonly spec: DocTabsSpec<P>,
        private readonly host: DocTabsHost,
        /** Used when the block has no saved tabs (a pane opened on a path). */
        initial?: P[]
    ) {
        const saved = hydrateDocTabs<P>(host.meta()?.[DOC_TABS_META], (st) => spec.deserialize(st));
        let start = saved ?? emptyDocTabs<P>();
        if (!saved && initial) {
            for (const p of initial) start = openDoc(start, this.argsFor(p));
        }
        [this.state, this.setState] = createSignal(start);
    }

    readonly tabs = (): DocTab<P>[] => this.state().tabs;
    readonly activeId = (): string | null => this.state().activeId;
    readonly active = (): DocTab<P> | undefined => activeTab(this.state());
    readonly showStrip = (): boolean => this.state().tabs.length >= 2 || (!!this.spec.alwaysShowStrip && this.state().tabs.length >= 1);

    private argsFor(payload: P, extra: Partial<OpenArgs<P>> = {}): OpenArgs<P> {
        return {
            key: this.spec.keyOf(payload),
            title: this.spec.titleOf(payload),
            icon: this.spec.iconOf?.(payload),
            payload,
            ...extra,
        };
    }

    private apply(next: DocTabsState<P>): void {
        if (next === this.state()) return;
        const before = this.state();
        this.setState(next);
        // Tabs gone for good: in neither the new list nor the reopen list
        // (a closed preview, a tab past the reopen cap).
        if (this.onClosed) {
            const still = new Set([...next.tabs, ...next.closed].map((t) => t.id));
            for (const t of [...before.tabs, ...before.closed]) if (!still.has(t.id)) this.onClosed(t);
        }
        this.scheduleSave();
    }

    private scheduleSave(): void {
        if (this.saveTimer) clearTimeout(this.saveTimer);
        this.saveTimer = setTimeout(() => {
            this.saveTimer = null;
            void this.host.setMeta({ [DOC_TABS_META]: persistDocTabs(this.state(), (p) => this.spec.serialize(p)) });
        }, SAVE_DELAY_MS);
    }

    /** Write now (a pane closing, tests). */
    flush(): void {
        if (!this.saveTimer) return;
        clearTimeout(this.saveTimer);
        this.saveTimer = null;
        void this.host.setMeta({ [DOC_TABS_META]: persistDocTabs(this.state(), (p) => this.spec.serialize(p)) });
    }

    open(payload: P, opts: { preview?: boolean; activate?: boolean; at?: "afterActive" | "end" } = {}): string {
        const next = openDoc(this.state(), this.argsFor(payload, opts));
        this.apply(next);
        return next.tabs.find((t) => t.key === this.spec.keyOf(payload))!.id;
    }

    /** The active tab now shows `payload` (a Hangar tab navigating). */
    replaceActive(payload: P): void {
        const id = this.state().activeId;
        if (!id) {
            this.open(payload);
            return;
        }
        this.apply(updateDoc(this.state(), id, { payload, key: this.spec.keyOf(payload), title: this.spec.titleOf(payload), icon: this.spec.iconOf?.(payload) }));
    }

    update(id: string, patch: Partial<Pick<DocTab<P>, "title" | "icon" | "dirty" | "payload" | "key">>): void {
        this.apply(updateDoc(this.state(), id, patch));
    }

    activate(id: string): void {
        this.apply(activateDoc(this.state(), id));
    }

    /** False when refused (the last tab of a keepOne pane). */
    close(id: string): boolean {
        const before = this.state();
        const next = closeDoc(before, id, { keepOne: this.spec.keepOne });
        this.apply(next);
        return next !== before;
    }

    closeOthers(id: string): void {
        this.apply(closeOthers(this.state(), id));
    }

    closeToRight(id: string): void {
        this.apply(closeToRight(this.state(), id));
    }

    reopen(): boolean {
        const next = reopenClosed(this.state());
        this.apply(next);
        return next !== this.state();
    }

    cycle(delta: number): void {
        this.apply(cycleDoc(this.state(), delta));
    }

    move(id: string, delta: number): void {
        this.apply(moveDoc(this.state(), id, delta));
    }

    pin(id: string, pinned: boolean): void {
        this.apply(setPinned(this.state(), id, pinned));
    }

    promote(id: string): void {
        this.apply(promoteDoc(this.state(), id));
    }

    /** Ctrl+T / "+": the type's new document, after the active tab. */
    newDocument(): boolean {
        const p = this.spec.newDocument?.(this.active()?.payload);
        if (p == null) return false;
        // A new tab even when that document is already open elsewhere: a
        // second view of it, as a browser's new tab is.
        const next = openDoc(this.state(), { ...this.argsFor(p), key: `${this.spec.keyOf(p)}#${Date.now()}` });
        this.apply(next);
        return true;
    }

    dispose(): void {
        this.flush();
    }
}

/** The document-tab keys (§4.3), as an action, or null for any other key. */
export type DocTabKeyAction =
    | { kind: "new" }
    | { kind: "close" }
    | { kind: "cycle"; delta: 1 | -1 }
    | { kind: "reopen" }
    | { kind: "move"; delta: 1 | -1 };

export function docTabKeyAction(e: Pick<KeyboardEvent, "key" | "ctrlKey" | "shiftKey" | "altKey" | "metaKey">): DocTabKeyAction | null {
    // Literal Ctrl on every platform; Cmd and Alt chords belong to the app.
    if (!e.ctrlKey || e.altKey || e.metaKey) return null;
    const k = e.key.toLowerCase();
    if (k === "tab") return { kind: "cycle", delta: e.shiftKey ? -1 : 1 };
    if (e.key === "PageDown") return e.shiftKey ? { kind: "move", delta: 1 } : { kind: "cycle", delta: 1 };
    if (e.key === "PageUp") return e.shiftKey ? { kind: "move", delta: -1 } : { kind: "cycle", delta: -1 };
    if (k === "t") return e.shiftKey ? { kind: "reopen" } : { kind: "new" };
    if (k === "w" && !e.shiftKey) return { kind: "close" };
    return null;
}

/**
 * Handle a document-tab key on `ctl`. True when it was one (the caller
 * stops it). `onRefused` is told when a close was refused (a keepOne pane's
 * last tab), so the pane can say why.
 */
export function handleDocTabKey<P>(e: KeyboardEvent, ctl: DocTabsController<P>, onRefused?: () => void): boolean {
    const action = docTabKeyAction(e);
    if (!action) return false;
    const id = ctl.activeId();
    switch (action.kind) {
        case "new":
            ctl.newDocument();
            break;
        case "close":
            if (id && !ctl.close(id)) onRefused?.();
            break;
        case "cycle":
            ctl.cycle(action.delta);
            break;
        case "reopen":
            ctl.reopen();
            break;
        case "move":
            if (id) ctl.move(id, action.delta);
            break;
    }
    e.preventDefault();
    e.stopPropagation();
    return true;
}
