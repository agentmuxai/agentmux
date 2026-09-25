// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * MemoryDraftModel — the edit lifecycle shared by every memory editor
 * (Armory → Personal / Global full views, the Stash memory tab): a draft, the
 * content it was based on, and that base's SHA-256.
 *
 * Two jobs, both from docs/specs/SPEC_MEMORY_FOLLOWS_THE_AGENT_2026_09_24.md
 * §2.4 ("Unsaved edits are never lost"):
 *
 *   1. A save carries its base (`base_sha256`). The server refuses a save
 *      whose base has moved with a `conflict:` error instead of
 *      overwriting; that becomes a "refused" conflict here.
 *   2. A change landing while a dirty draft is open (`observeExternal`)
 *      never replaces the draft. It raises a "changed" conflict — the
 *      banner — but ONLY when the new content's hash differs from the
 *      draft's base, not on every change event (a refresh that re-reads
 *      identical content, e.g. the draft's own base or an unrelated file's
 *      write for the same agent, stays silent).
 *
 * Generic over the edited value so Global Memory (name + instructions) and
 * native memory files (one string) share it; `hash` defines what the base
 * means for each and must match the server's own hash of the same row.
 */

import { createMemo, createSignal, type Accessor } from "solid-js";

export type MemoryConflictReason = "changed" | "refused";

export interface MemoryConflict<T> {
    reason: MemoryConflictReason;
    /** What is saved now — `null` when it no longer exists, `undefined`
     *  while it is still being fetched (a "refused" save). */
    current: T | null | undefined;
}

export interface MemoryDraftOptions<T> {
    /** Hex SHA-256 of a value, computed exactly as the server does. */
    hash: (value: T) => Promise<string>;
    equals: (a: T, b: T) => boolean;
    /** Persist `value`. `baseSha256` is null for a brand-new entry (nothing
     *  to be based on). Must reject with an error whose message contains
     *  `conflict:` when the server refuses a moved base. */
    save: (value: T, baseSha256: string | null) => Promise<void>;
    /** Re-read what is saved now, for the conflict banner after a refused
     *  save. `null` = it no longer exists. */
    fetchCurrent?: () => Promise<T | null>;
}

/** The server's refusal marker — see check_memory_base_sha256
 *  (native_memory_handlers.rs) and StoreError::Conflict. */
export function isConflictError(e: unknown): boolean {
    const msg = e instanceof Error ? e.message : String(e ?? "");
    return msg.includes("conflict:");
}

export class MemoryDraftModel<T> {
    private readonly opts: MemoryDraftOptions<T>;

    private _editing = createSignal(false);
    editingAtom: Accessor<boolean> = this._editing[0];
    private setEditing = this._editing[1];

    private _draft = createSignal<T | null>(null);
    draftAtom: Accessor<T | null> = this._draft[0];
    private setDraftValue = this._draft[1];

    /** The value the draft started from (or was last rebased onto). */
    private _base = createSignal<T | null>(null);
    baseAtom: Accessor<T | null> = this._base[0];
    private setBase = this._base[1];

    private _saving = createSignal(false);
    savingAtom: Accessor<boolean> = this._saving[0];
    private setSaving = this._saving[1];

    private _error = createSignal<string | null>(null);
    errorAtom: Accessor<string | null> = this._error[0];
    setError = this._error[1];

    private _conflict = createSignal<MemoryConflict<T> | null>(null);
    conflictAtom: Accessor<MemoryConflict<T> | null> = this._conflict[0];
    private setConflict = this._conflict[1];

    /** True while editing with changes relative to the base. A brand-new
     *  entry (no base) is dirty as soon as it's open, so an accidental Esc
     *  asks first. */
    dirtyAtom: Accessor<boolean>;

    /** Resolves to the base's hash; a promise so a save fired before the
     *  (async) hash settles still sends the right base. `null` = no base. */
    private baseSha: Promise<string | null> = Promise.resolve(null);

    constructor(opts: MemoryDraftOptions<T>) {
        this.opts = opts;
        this.dirtyAtom = createMemo(() => {
            if (!this.editingAtom()) return false;
            const draft = this.draftAtom();
            const base = this.baseAtom();
            if (draft === null) return false;
            if (base === null) return true;
            return !this.opts.equals(draft, base);
        });
    }

    /** Open the editor on `current`, which becomes the base. */
    startEdit(current: T): void {
        this.rebase(current);
        this.setDraftValue(() => current);
        this.setConflict(null);
        this.setError(null);
        this.setEditing(true);
    }

    /** Open the editor on a value with no saved base (a new entry). */
    startNew(initial: T): void {
        this.setBase(null);
        this.baseSha = Promise.resolve(null);
        this.setDraftValue(() => initial);
        this.setConflict(null);
        this.setError(null);
        this.setEditing(true);
    }

    setDraft(value: T): void {
        this.setDraftValue(() => value);
    }

    /** Leave editing, dropping the draft. Callers confirm first if dirty. */
    cancel(): void {
        this.setEditing(false);
        this.setDraftValue(null);
        this.setConflict(null);
        this.setError(null);
    }

    /** Persist the draft against its base. Resolves `true` on success. */
    async save(): Promise<boolean> {
        const draft = this.draftAtom();
        if (!this.editingAtom() || draft === null || this.savingAtom()) return false;
        this.setSaving(true);
        this.setError(null);
        try {
            const base = await this.baseSha;
            await this.opts.save(draft, base);
            this.setConflict(null);
            this.rebase(draft);
            // Typing continued while the save was in flight: keep editing the
            // newer text, now based on what was just saved, so nothing typed
            // after pressing Save is lost.
            const latest = this.draftAtom();
            if (latest !== null && !this.opts.equals(latest, draft)) return true;
            this.setEditing(false);
            this.setDraftValue(null);
            return true;
        } catch (e) {
            if (isConflictError(e)) {
                this.setConflict({ reason: "refused", current: undefined });
                void this.fetchCurrentForConflict();
            } else {
                this.setError(`Save failed: ${(e as Error)?.message ?? e}`);
            }
            return false;
        } finally {
            this.setSaving(false);
        }
    }

    /**
     * The saved content changed underneath (a refresh after a change event,
     * or a revert). Not editing: nothing to protect — the caller just shows
     * `current`. Editing with no changes: rebase silently onto `current`, so
     * the editor follows along. Editing with a dirty draft: keep the draft,
     * and raise the banner only if `current` really differs from the base.
     */
    async observeExternal(current: T | null): Promise<void> {
        if (!this.editingAtom()) {
            if (current !== null) this.rebase(current);
            return;
        }
        if (!this.dirtyAtom()) {
            if (current === null) {
                this.raiseConflict(null);
                return;
            }
            this.rebase(current);
            this.setDraftValue(() => current);
            return;
        }
        if (current === null) {
            this.raiseConflict(null);
            return;
        }
        const [baseHash, currentHash] = await Promise.all([this.baseSha, this.opts.hash(current)]);
        // The user may have saved or cancelled while the hashes resolved.
        if (!this.editingAtom()) return;
        if (baseHash !== null && currentHash === baseHash) {
            // Back to (or still at) the base — e.g. a revert undid the other
            // write. Drop a stale "changed" banner; keep a "refused" one,
            // which the user still has to act on.
            if (this.conflictAtom()?.reason === "changed") this.setConflict(null);
            return;
        }
        this.raiseConflict(current);
    }

    /** A refused save stays "refused" (the user still has to act on it);
     *  only its `current` is updated. */
    private raiseConflict(current: T | null): void {
        const reason = this.conflictAtom()?.reason === "refused" ? "refused" : "changed";
        this.setConflict({ reason, current });
    }

    /** "Keep editing": dismiss the banner, keep the draft AND its original
     *  base, so saving still can't silently overwrite the other change. */
    keepEditing(): void {
        this.setConflict(null);
    }

    /** "Save anyway": rebase the draft onto what's saved now, then save — an
     *  explicit, informed overwrite of the other change. */
    async overwrite(): Promise<boolean> {
        const current = this.conflictAtom()?.current;
        if (current === undefined) return false;
        if (current === null) {
            this.setBase(null);
            this.baseSha = Promise.resolve(null);
        } else {
            this.rebase(current);
        }
        this.setConflict(null);
        return this.save();
    }

    /** "Discard my edits": drop the draft (the caller shows current content). */
    discard(): void {
        this.cancel();
    }

    private rebase(value: T): void {
        this.setBase(() => value);
        this.baseSha = this.opts.hash(value);
    }

    private async fetchCurrentForConflict(): Promise<void> {
        if (!this.opts.fetchCurrent) return;
        try {
            const current = await this.opts.fetchCurrent();
            const c = this.conflictAtom();
            if (c && c.current === undefined) this.setConflict({ ...c, current });
        } catch {
            // Leave `current` unknown — the banner still offers Keep editing /
            // Discard; only "Save anyway" and "View change" need it.
        }
    }
}
