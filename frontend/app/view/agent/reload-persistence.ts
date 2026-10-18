// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/*
 * What an agent pane's composer holds that a reload of its window must not
 * lose: the text draft, the sent-message history (ArrowUp recall) and, in
 * attachment-draft.ts, the attached files.
 * docs/specs/SPEC_RENDERER_RECYCLE_WHEN_IDLE_2026_10_13.md §2.
 *
 * Each lives in memory, per block, for the page's lifetime (so it survives
 * the footer unmounting on a pane-tab switch), and is mirrored to
 * sessionStorage, which survives a reload of the same window but not an app
 * restart or another window. Draft writes are debounced; `pagehide` (which a
 * reload fires) flushes whatever is pending.
 */

const DRAFT_KEY = "agentmux:composer-draft:";
const HISTORY_KEY = "agentmux:sent-history:";
/** The ArrowUp history kept per block (in memory and stored). */
export const SENT_HISTORY_MAX = 200;
const DRAFT_WRITE_DELAY_MS = 300;

/** Read a stored JSON value; undefined when absent, unreadable or storage is off. */
export function readSession<T>(key: string): T | undefined {
    try {
        const raw = sessionStorage.getItem(key);
        return raw == null ? undefined : (JSON.parse(raw) as T);
    } catch {
        return undefined;
    }
}

/** Store a JSON value; `undefined` removes it. A full or disabled storage is
 *  ignored (the in-memory copy still serves this page); false says so. */
export function writeSession(key: string, value: unknown): boolean {
    try {
        if (value === undefined) sessionStorage.removeItem(key);
        else sessionStorage.setItem(key, JSON.stringify(value));
        return true;
    } catch {
        return false;
    }
}

// ── Composer text drafts ────────────────────────────────────────────────────

const drafts = new Map<string, string>();
const pendingDrafts = new Set<string>();
let draftTimer: ReturnType<typeof setTimeout> | undefined;

/** Write every pending draft now (the debounce, and `pagehide`). */
export function flushComposerDrafts(): void {
    clearTimeout(draftTimer);
    draftTimer = undefined;
    for (const blockId of pendingDrafts) writeSession(DRAFT_KEY + blockId, drafts.get(blockId) || undefined);
    pendingDrafts.clear();
}

/**
 * Write every draft this page holds, for a recycle that is about to reload it.
 * False when one couldn't be stored: the reload would lose it, so the recycle
 * doesn't happen.
 */
export function saveAllComposerDrafts(): boolean {
    clearTimeout(draftTimer);
    draftTimer = undefined;
    pendingDrafts.clear();
    let saved = true;
    for (const [blockId, text] of drafts) saved = writeSession(DRAFT_KEY + blockId, text || undefined) && saved;
    return saved;
}

export function getComposerDraft(blockId: string): string | undefined {
    if (!drafts.has(blockId)) {
        const stored = readSession<string>(DRAFT_KEY + blockId);
        if (typeof stored === "string") drafts.set(blockId, stored);
    }
    return drafts.get(blockId);
}

export function setComposerDraft(blockId: string, text: string): void {
    drafts.set(blockId, text);
    pendingDrafts.add(blockId);
    draftTimer ??= setTimeout(flushComposerDrafts, DRAFT_WRITE_DELAY_MS);
}

// ── Sent-message history ────────────────────────────────────────────────────

const histories = new Map<string, string[]>();

/** The block's history, oldest first. The same array every call: push to it, then `saveSentHistory`. */
export function getSentHistory(blockId: string): string[] {
    let history = histories.get(blockId);
    if (!history) {
        const stored = readSession<unknown>(HISTORY_KEY + blockId);
        history = Array.isArray(stored) ? stored.filter((m): m is string => typeof m === "string") : [];
        histories.set(blockId, history);
    }
    return history;
}

export function saveSentHistory(blockId: string): void {
    const history = histories.get(blockId);
    if (history) writeSession(HISTORY_KEY + blockId, history.length > 0 ? history : undefined);
}

if (typeof window !== "undefined") window.addEventListener("pagehide", flushComposerDrafts);

/** Test hook: forget the in-memory copies, as a reload does. */
export function resetReloadPersistenceForTests(): void {
    flushComposerDrafts();
    drafts.clear();
    histories.clear();
}
