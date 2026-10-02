// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/*
 * OS file drops into panes: one controller per window, and a registry of the
 * panes that accept files. docs/specs/SPEC_DRAG_AND_DROP_CONSOLIDATION_2026_09_27.md §4, §5.3.
 *
 * A pane opts in by registering a live hook for its block id. While files are
 * dragged over the window:
 *   - every visible pane whose `accept` says ok is Armed (a faint outline);
 *   - the pane under the cursor is Target (tint, border, "what will happen");
 *   - a pane that accepts files but can't take these shows Blocked (the reason).
 * Decisions (`dropEffect`, which pane gets the drop) are made synchronously on
 * every event from the pane actually under the pointer; only the visual state
 * is batched per animation frame. A drop re-runs `accept` on the files it
 * actually carries before anything is consumed.
 */

import { createSignal, type Accessor } from "solid-js";
import { baseName, consumeDragPaths, isFileDrag, peekDragPaths } from "@/util/dnd";

export interface DragFiles {
    count: number;
    /** MIME per item; "" when the OS gave none. */
    types: string[];
    /** File names, once known (the host's paths, or the drop's FileList). */
    names?: string[];
}

export type DropVerdict = { ok: true; message: string; icon?: string } | { ok: false; reason: string };

export interface FileDropHook {
    accept(drag: DragFiles): DropVerdict;
    /**
     * `paths` are host paths for the dropped files; empty when the host has
     * none (no native file drop, virtual files). `files` is always the drop's
     * FileList, so a hook can fall back to the bytes.
     */
    drop(input: { paths: string[]; files: File[] }): void | Promise<void>;
    /** Set only if the hook can't work from bytes alone. */
    needsPaths?: boolean;
    /** Shown when a `needsPaths` hook gets no paths. */
    onNoPaths?: (count: number) => void;
}

export type PaneDropState =
    | { state: "armed" }
    | { state: "target"; message: string; icon?: string }
    | { state: "blocked"; reason: string };

// ── Registry ────────────────────────────────────────────────────────────

const hooks = new Map<string, FileDropHook>();

/** Register a pane's live hook; returns the disposer (call it on cleanup). */
export function registerFileDropTarget(blockId: string, hook: FileDropHook): () => void {
    hooks.set(blockId, hook);
    return () => {
        if (hooks.get(blockId) === hook) hooks.delete(blockId);
    };
}

// ── Visual state ────────────────────────────────────────────────────────

const [states, setStates] = createSignal<ReadonlyMap<string, PaneDropState>>(new Map());

/** The drop state to draw on `blockId`'s pane, if any. */
export function paneDropState(blockId: Accessor<string | undefined>): Accessor<PaneDropState | undefined> {
    return () => {
        const id = blockId();
        return id ? states().get(id) : undefined;
    };
}

// ── In-app path drags ───────────────────────────────────────────────────
//
// A Hangar row dragged inside the app is a drag of host PATHS, not of OS
// files: the renderer can't make a native file drag from the DOM. The
// controller treats it exactly like an OS file drop that carries paths, so
// the agent composer, the Editor, Media and terminal hooks take it unchanged
// (SPEC_FILE_BROWSER_PANE_2026_10_01.md §8.2). The paths live here, not only
// in the DataTransfer, because `getData` is unreadable until the drop.

/** The DataTransfer type that marks an in-app path drag. */
export const PATHS_MIME = "application/x-agentmux-paths";

let pathDrag: { paths: string[]; sourceBlockId?: string } | null = null;

/** Start a path drag from `dragstart` (Hangar rows). */
export function beginPathDrag(dt: DataTransfer, paths: string[], sourceBlockId?: string): void {
    pathDrag = { paths, sourceBlockId };
    dt.setData(PATHS_MIME, JSON.stringify(paths));
    // Text elsewhere (a text field, another app) gets the paths themselves.
    dt.setData("text/plain", paths.join("\n"));
    dt.effectAllowed = "copyMove";
}

/** End it from `dragend`, whether or not anything took the drop. */
export function endPathDrag(): void {
    pathDrag = null;
    end();
}

/** The paths of the path drag underway, if any. */
export function pathDragPaths(): string[] | undefined {
    return pathDrag?.paths;
}

/** The block a path drag started in, while one is underway. */
export function pathDragSource(): string | undefined {
    return pathDrag?.sourceBlockId;
}

function isPathDrag(e: DragEvent): boolean {
    const types = e.dataTransfer?.types;
    return pathDrag != null && !!types && Array.from(types).includes(PATHS_MIME);
}

function dragFilesFromPaths(paths: string[]): DragFiles {
    return { count: paths.length, types: paths.map(() => ""), names: paths.map(baseName) };
}

// ── Helpers ─────────────────────────────────────────────────────────────

function dragFilesFromTransfer(dt: DataTransfer | null | undefined): DragFiles {
    const items = dt?.items ? Array.from(dt.items).filter((i) => i.kind === "file") : [];
    return { count: items.length || dt?.files?.length || 0, types: items.map((i) => i.type ?? "") };
}

function dragFilesFromList(files: File[]): DragFiles {
    return { count: files.length, types: files.map((f) => f.type ?? ""), names: files.map((f) => f.name) };
}

function paneAt(target: EventTarget | null): string | undefined {
    const el = target instanceof Element ? target : null;
    return el?.closest?.('[data-role="pane"]')?.getAttribute("data-blockid") ?? undefined;
}

function visiblePanes(): string[] {
    const out: string[] = [];
    document.querySelectorAll('[data-role="pane"][data-blockid]').forEach((el) => {
        const id = el.getAttribute("data-blockid");
        if (id && hooks.has(id) && (el as HTMLElement).getClientRects().length > 0) out.push(id);
    });
    return out;
}

/** The host paths match the dropped files (same base names, same order-insensitive set). */
export function pathsMatchFiles(paths: string[], files: File[]): boolean {
    if (paths.length !== files.length) return false;
    const a = paths.map(baseName).sort();
    const b = files.map((f) => f.name).sort();
    return a.every((n, i) => n === b[i]);
}

/**
 * Next animation frame, or 50 ms, whichever comes first. rAF alone isn't
 * enough: Chromium pauses it for a hidden or occluded window, and a drag's
 * visuals must still settle (and never leave a render stuck as queued).
 */
function schedule(cb: () => void): void {
    let done = false;
    const run = () => {
        if (done) return;
        done = true;
        cb();
    };
    if (typeof requestAnimationFrame === "function") requestAnimationFrame(run);
    setTimeout(run, 50);
}

/**
 * No `dragover` for this long ends the visuals (the cursor went over a native
 * surface, or the drag left without a `dragleave`). Well above the HTML
 * spec's dragover cadence while the cursor is still (every 350 ms ± 200 ms),
 * so a user holding a file over a pane never loses the indicator; a missed
 * leave costs at most this long of a stale highlight.
 */
const IDLE_MS = 1200;

// ── Controller ──────────────────────────────────────────────────────────

interface Session {
    files: DragFiles;
    verdicts: Map<string, DropVerdict>;
    target?: string;
    idle?: ReturnType<typeof setTimeout>;
    renderQueued: boolean;
}

let session: Session | null = null;

function verdictFor(s: Session, blockId: string): DropVerdict | undefined {
    const hook = hooks.get(blockId);
    if (!hook) return undefined;
    let v = s.verdicts.get(blockId);
    if (!v) {
        v = hook.accept(s.files);
        s.verdicts.set(blockId, v);
    }
    return v;
}

function render(): void {
    const s = session;
    if (!s) {
        setStates(new Map());
        return;
    }
    const next = new Map<string, PaneDropState>();
    for (const [id, v] of s.verdicts) {
        if (v.ok === false) {
            if (id === s.target) next.set(id, { state: "blocked", reason: v.reason });
            continue;
        }
        next.set(id, id === s.target ? { state: "target", message: v.message, icon: v.icon } : { state: "armed" });
    }
    setStates(next);
}

function queueRender(): void {
    const s = session;
    if (!s || s.renderQueued) return;
    s.renderQueued = true;
    schedule(() => {
        if (session === s) s.renderQueued = false;
        render();
    });
}

function begin(dt: DataTransfer | null | undefined, paths?: string[]): Session {
    const s: Session = { files: paths ? dragFilesFromPaths(paths) : dragFilesFromTransfer(dt), verdicts: new Map(), renderQueued: false };
    session = s;
    // Every visible accepting pane up front, so valid ones are Armed at once.
    for (const id of visiblePanes()) verdictFor(s, id);
    queueRender();
    // An in-app drag already knows its names.
    if (paths) return s;
    // Names arrive a moment later from the host; re-ask every verdict once.
    void peekDragPaths().then((paths) => {
        if (session !== s || paths.length === 0) return;
        s.files = { ...s.files, names: paths.map(baseName) };
        const ids = [...s.verdicts.keys()];
        s.verdicts.clear();
        for (const id of ids) verdictFor(s, id);
        queueRender();
    });
    return s;
}

function end(): void {
    if (session?.idle) clearTimeout(session.idle);
    session = null;
    render();
}

function touch(s: Session): void {
    if (s.idle) clearTimeout(s.idle);
    s.idle = setTimeout(() => {
        if (session === s) end();
    }, IDLE_MS);
}

function onDragEnterOrOver(e: DragEvent): void {
    const inApp = isPathDrag(e);
    if (!inApp && !isFileDrag(e)) return;
    const s = session ?? begin(e.dataTransfer, inApp ? pathDrag!.paths : undefined);
    touch(s);
    const id = paneAt(e.target);
    const v = id ? verdictFor(s, id) : undefined;
    // Decide now, from the pane actually under the pointer.
    e.preventDefault();
    if (e.dataTransfer) e.dataTransfer.dropEffect = v?.ok ? "copy" : "none";
    if (s.target !== id) {
        s.target = id;
        queueRender();
    }
}

function onDragLeave(e: DragEvent): void {
    // Leaving the window (or entering a native surface the renderer can't see).
    if (session && e.relatedTarget == null) end();
}

async function onDrop(e: DragEvent): Promise<void> {
    if (isPathDrag(e)) {
        e.preventDefault();
        const paths = pathDrag!.paths;
        const id = paneAt(e.target);
        end();
        const hook = id ? hooks.get(id) : undefined;
        if (!hook || !hook.accept(dragFilesFromPaths(paths)).ok) return;
        e.stopPropagation();
        await hook.drop({ paths, files: [] });
        return;
    }
    if (!isFileDrag(e)) return;
    e.preventDefault();
    const files = Array.from(e.dataTransfer?.files ?? []);
    const id = paneAt(e.target);
    end();
    const hook = id ? hooks.get(id) : undefined;
    if (!hook || files.length === 0) return;
    // Re-validate with what the drop really carries, not the hover verdict.
    const verdict = hook.accept(dragFilesFromList(files));
    if (!verdict.ok) return;
    e.stopPropagation();
    let paths = await consumeDragPaths();
    if (paths.length > 0 && !pathsMatchFiles(paths, files)) paths = [];
    if (paths.length === 0 && hook.needsPaths) {
        hook.onNoPaths?.(files.length);
        return;
    }
    await hook.drop({ paths, files });
}

let installedOn: Window | null = null;

/** Install the controller on this window (once per renderer). */
export function installFileDropController(win: Window = window): () => void {
    if (installedOn === win) return () => {};
    installedOn = win;
    const drop = (e: DragEvent) => void onDrop(e);
    win.addEventListener("dragenter", onDragEnterOrOver, true);
    win.addEventListener("dragover", onDragEnterOrOver, true);
    win.addEventListener("dragleave", onDragLeave, true);
    win.addEventListener("drop", drop, true);
    return () => {
        win.removeEventListener("dragenter", onDragEnterOrOver, true);
        win.removeEventListener("dragover", onDragEnterOrOver, true);
        win.removeEventListener("dragleave", onDragLeave, true);
        win.removeEventListener("drop", drop, true);
        installedOn = null;
        end();
    };
}

/**
 * Hand `paths` to `blockId`'s drop hook as if they had been dropped on it:
 * the Files pane's "Attach to <agent>" (spec §8.2, route 2), so a menu item
 * and a drag end in exactly the same place. False when the pane has no hook
 * or refuses them.
 */
export async function dropPathsOnto(blockId: string, paths: string[]): Promise<boolean> {
    const hook = hooks.get(blockId);
    if (!hook || paths.length === 0) return false;
    if (!hook.accept(dragFilesFromPaths(paths)).ok) return false;
    await hook.drop({ paths, files: [] });
    return true;
}

/** Block ids with a drop hook whose pane is on screen now. */
export function visibleDropTargets(): string[] {
    return visiblePanes();
}

/** Test hook: forget every registration and session. */
export function resetFileDropForTests(): void {
    hooks.clear();
    pathDrag = null;
    end();
}
