// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * A pane's document tabs: the state (doc-tabs.ts) held in a signal, saved
 * to the block's meta as it changes, restored from it, and the keys every
 * document-tab pane shares. docs/specs/SPEC_DOCUMENT_TABS_2026_10_02.md §4.3,
 * §5.2, §5.3, §5.6.
 *
 * `DocTabsModel` is what the shared strip, keys and moves need of a pane's
 * tabs. `DocTabsController` is one; the Editor's store is another, through
 * `editorDocTabs` (editor-doc-tabs.ts), which keeps the Editor's own commands
 * and events. docs/specs/PLAN_EDITOR_ON_SHARED_DOC_TABS_2026_10_10.md.
 */

import { createSignal, type Accessor } from "solid-js";
import {
    activateDoc,
    activeTab,
    attachDoc,
    closeDoc,
    closeOthers,
    closeToRight,
    cycleDoc,
    detachDoc,
    DOC_TABS_META,
    emptyDocTabs,
    hydrateDocTabs,
    moveDoc,
    moveDocTo,
    openDoc,
    persistDocTabs,
    promoteDoc,
    reopenClosed,
    setPinned,
    updateDoc,
    type DocDropAt,
    type DocTab,
    type DocTabsState,
    type OpenArgs,
    type PersistedDocTabs,
} from "./doc-tabs";
import { keyPlatform } from "@/app/keybindings";
import { noteResolved } from "@/app/keybindings/app-api";
import { matchPaneKey } from "@/app/keybindings/registry";

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
    /** Show the strip with one tab too (Media, as the Editor does). */
    alwaysShowStrip?: boolean;
}

/** Where a controller keeps its state between runs: the block's meta. */
export interface DocTabsHost {
    meta(): Record<string, unknown> | undefined;
    setMeta(patch: Record<string, unknown>): Promise<void> | void;
}

/** A pane's document tabs, as the shared strip (DocTabStrip), keys
 *  (`runDocTabAction`) and moves (`docTabsHost`) use them. */
export interface DocTabsModel<P> {
    tabs(): DocTab<P>[];
    activeId(): string | null;
    showStrip(): boolean;
    activate(id: string): void;
    /** False when refused (the last tab of a keepOne pane). A tab with
     *  unsaved changes may ask first. */
    close(id: string): boolean;
    /** A preview tab kept (double-click). */
    promote(id: string): void;
    cycle(delta: number): void;
    move(id: string, delta: number): void;
    /** A tab dragged to just before or after another. False when nothing moved. */
    moveTo(id: string, targetId: string, position: "before" | "after"): boolean;
    /** False when there was nothing to reopen. */
    reopen(): boolean;
    /** Whether "+" and Ctrl+T open a new document here. */
    canAddNew(): boolean;
    newDocument(): boolean;
    /** Take tab `id` out, to move it to another pane. Null if there is none. */
    detach(id: string): DocTab<P> | null;
    /** Put in a tab moved from another pane (`attachDoc`). */
    attach(tab: DocTab<P>, at?: DocDropAt): void;
}

/** Writes to block meta are batched this long. */
export const DOC_TABS_SAVE_DELAY_MS = 300;

/**
 * Writes a pane's tabs to its block once changes settle: after a pause, never
 * the same record twice, and at once on `flush` (a pane closing). With no
 * tabs it writes null.
 */
export class DocTabsSaver<P> {
    private timer: ReturnType<typeof setTimeout> | null = null;
    private written: string;

    constructor(
        private readonly read: () => DocTabsState<P> | null | undefined,
        private readonly serialize: (payload: P) => unknown,
        private readonly write: (record: PersistedDocTabs | null) => void
    ) {
        this.written = JSON.stringify(null);
    }

    /** The record the block holds now (what the pane was restored from):
     *  not written again until the tabs change. */
    holds(record: unknown): void {
        this.written = JSON.stringify(record ?? null);
    }

    schedule(): void {
        if (this.timer) clearTimeout(this.timer);
        this.timer = setTimeout(() => this.save(), DOC_TABS_SAVE_DELAY_MS);
    }

    /** Write now if a save is pending. */
    flush(): void {
        if (this.timer) this.save();
    }

    private save(): void {
        if (this.timer) clearTimeout(this.timer);
        this.timer = null;
        const state = this.read();
        if (!state) return;
        const record = state.tabs.length > 0 ? persistDocTabs(state, this.serialize) : null;
        const text = JSON.stringify(record);
        if (text === this.written) return;
        this.written = text;
        this.write(record);
    }
}

export class DocTabsController<P> implements DocTabsModel<P> {
    readonly state: Accessor<DocTabsState<P>>;
    private readonly setState: (s: DocTabsState<P>) => void;
    private readonly saver: DocTabsSaver<P>;
    /** Told when a tab is closed (and not reopenable as itself), so the pane
     *  can release what it held for it. */
    onClosed: ((tab: DocTab<P>) => void) | null = null;

    constructor(
        readonly spec: DocTabsSpec<P>,
        private readonly host: DocTabsHost,
        /** Used when the block has no saved tabs (a pane opened on a path). */
        initial?: P[]
    ) {
        const raw = host.meta()?.[DOC_TABS_META];
        const saved = hydrateDocTabs<P>(raw, (st) => spec.deserialize(st));
        // A restored tab's title and icon come from its document, as this
        // build names it, not from the save: a change (Media's tabs lost their
        // icons) reaches tabs saved before it.
        let start = saved
            ? { ...saved, tabs: saved.tabs.map((t) => ({ ...t, title: spec.titleOf(t.payload), icon: spec.iconOf?.(t.payload) })) }
            : emptyDocTabs<P>();
        if (!saved && initial) {
            for (const p of initial) start = openDoc(start, this.argsFor(p));
        }
        [this.state, this.setState] = createSignal(start);
        this.saver = new DocTabsSaver(this.state, (p) => spec.serialize(p), (record) => void host.setMeta({ [DOC_TABS_META]: record }));
        this.saver.holds(raw);
    }

    readonly tabs = (): DocTab<P>[] => this.state().tabs;
    readonly activeId = (): string | null => this.state().activeId;
    readonly active = (): DocTab<P> | undefined => activeTab(this.state());
    readonly showStrip = (): boolean => this.state().tabs.length >= 2 || (!!this.spec.alwaysShowStrip && this.state().tabs.length >= 1);
    readonly canAddNew = (): boolean => !!this.spec.newDocument;

    private argsFor(payload: P, extra: Partial<OpenArgs<P>> = {}): OpenArgs<P> {
        return {
            key: this.spec.keyOf(payload),
            title: this.spec.titleOf(payload),
            icon: this.spec.iconOf?.(payload),
            payload,
            ...extra,
        };
    }

    /** `movedOut`: a tab that left for another pane, which is not closed. */
    private apply(next: DocTabsState<P>, movedOut?: string): void {
        if (next === this.state()) return;
        const before = this.state();
        this.setState(next);
        // Tabs gone for good: in neither the new list nor the reopen list
        // (a closed preview, a tab past the reopen cap).
        if (this.onClosed) {
            const still = new Set([...next.tabs, ...next.closed].map((t) => t.id));
            for (const t of [...before.tabs, ...before.closed]) if (!still.has(t.id) && t.id !== movedOut) this.onClosed(t);
        }
        this.saver.schedule();
    }

    /** Write now (a pane closing, tests). */
    flush(): void {
        this.saver.flush();
    }

    open(payload: P, opts: { preview?: boolean; activate?: boolean; at?: "afterActive" | "end" } = {}): string {
        const next = openDoc(this.state(), this.argsFor(payload, opts));
        this.apply(next);
        return next.tabs.find((t) => t.key === this.spec.keyOf(payload))!.id;
    }

    /** The active tab now shows `payload` (another file in the same tab). */
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

    /** False when there was nothing to reopen. */
    reopen(): boolean {
        const before = this.state();
        const next = reopenClosed(before);
        this.apply(next);
        return next !== before;
    }

    cycle(delta: number): void {
        this.apply(cycleDoc(this.state(), delta));
    }

    move(id: string, delta: number): void {
        this.apply(moveDoc(this.state(), id, delta));
    }

    /** A tab dragged to just before or after another. False when nothing
     *  moved. */
    moveTo(id: string, targetId: string, position: "before" | "after"): boolean {
        const before = this.state();
        this.apply(moveDocTo(before, id, targetId, position));
        return this.state() !== before;
    }

    /** Take tab `id` out, to move it to another pane. Null if there is no
     *  such tab. */
    detach(id: string): DocTab<P> | null {
        const out = detachDoc(this.state(), id);
        if (!out) return null;
        this.apply(out.state, id);
        return out.tab;
    }

    /** Put in a tab moved from another pane (`attachDoc`). */
    attach(tab: DocTab<P>, at?: DocDropAt): void {
        this.apply(attachDoc(this.state(), tab, at));
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

const DOC_TAB_ACTIONS: Record<string, DocTabKeyAction> = {
    "doctab:new": { kind: "new" },
    "doctab:close": { kind: "close" },
    "doctab:reopen": { kind: "reopen" },
    "doctab:next": { kind: "cycle", delta: 1 },
    "doctab:prev": { kind: "cycle", delta: -1 },
    "doctab:moveRight": { kind: "move", delta: 1 },
    "doctab:moveLeft": { kind: "move", delta: -1 },
};

/** The keys are the `doctab:*` rows of the shortcut table (keybindings/defaults.ts). */
export function docTabKeyAction(
    e: Pick<KeyboardEvent, "key" | "ctrlKey" | "shiftKey" | "altKey" | "metaKey"> & { code?: string }
): DocTabKeyAction | null {
    // Field by field: a KeyboardEvent's fields are prototype getters, which a
    // spread would drop.
    const ev = { key: e.key, code: e.code ?? "", ctrlKey: e.ctrlKey, shiftKey: e.shiftKey, altKey: e.altKey, metaKey: e.metaKey };
    const command = matchPaneKey(ev, "doctabs", keyPlatform());
    if (command) noteResolved(command, "doctabs");
    return command ? (DOC_TAB_ACTIONS[command] ?? null) : null;
}

/** The action for a `doctab:*` command id, or null for any other command. */
export function docTabAction(command: string): DocTabKeyAction | null {
    return DOC_TAB_ACTIONS[command] ?? null;
}

/**
 * Handle a document-tab key on `ctl`. True when it was one (the caller
 * stops it). `onRefused` is told when a close was refused (a keepOne pane's
 * last tab), so the pane can say why.
 */
export function handleDocTabKey<P>(e: KeyboardEvent, ctl: DocTabsModel<P>, onRefused?: () => void): boolean {
    const action = docTabKeyAction(e);
    if (!action) return false;
    runDocTabAction(action, ctl, onRefused);
    e.preventDefault();
    e.stopPropagation();
    return true;
}

/** Carry out a document-tab action on `ctl`: a key's, or the App API's RunCommand. */
export function runDocTabAction<P>(action: DocTabKeyAction, ctl: DocTabsModel<P>, onRefused?: () => void): void {
    const id = ctl.activeId();
    switch (action.kind) {
        case "new":
            if (ctl.canAddNew()) ctl.newDocument();
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
}
