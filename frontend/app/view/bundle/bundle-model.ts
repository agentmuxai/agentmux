// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Armory Bundle Format (ABF) bundle manager — first-class management of
// ABF bundles. User-facing name is "Armory Bundle Format (ABF)" (short
// form "ABF"); the type/table stay `Memory` / `db_bundles`
// (SPEC_MEMORY_IDENTITY_ARCH §4.1). See
// docs/specs/SPEC_ABF_V0_2_PROVIDER_AWARE_COMPONENTS_AND_NATIVE_MEMORY_2026_08_10.md
// for the format itself.
//
// A bundle is the agent's capability stack: system instructions, context
// files, MCP servers, skills — plus, as of
// ARCHITECTURE_MANDATORY_ABF_RETHINK_2026_08_14.md §7, the provider
// (harness) + model it's meant to run on, set once at creation and readonly
// thereafter. This reverses SPEC_MEMORY_IDENTITY_ARCH §4.1a's "presets are
// provider-agnostic, reusable across any provider" decision on purpose: an
// ABF is now a self-contained, portable unit (can be exported and
// reconstitute the same agent elsewhere) rather than a config fragment
// meant to be mixed into agents of any provider.
//
// This module is the ViewModel for `view: "memory"` panes. It owns the
// list of memories, the currently-selected one, and the in-flight edit
// draft. CRUD goes through the v7 RPC commands
// (listmemories / upsertmemory / deletememory).

import { BlockNodeModel } from "@/app/block/blocktypes";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { getWaveObjectAtom, makeORef } from "@/app/store/wos";
import { waveEventSubscribe } from "@/app/store/wps";
import { createMemo, createSignal, type Accessor } from "solid-js";

/** What the form fields look like in flight. Maps 1:1 to the Memory
 *  shape but with everything optional + JSON-array fields exposed as
 *  parsed arrays for ergonomic editing. The shape is converted back to
 *  Memory on save. */
export interface BundleDraft {
    id?: string;
    name: string;
    description: string;
    /** The CLI/harness this ABF runs on (e.g. "claude"), and the resolved
     *  model vendor (e.g. "anthropic"). Readonly once set — enforced by
     *  the backend (`bundle.upsert`), not just this form: an ABF's whole
     *  portability guarantee (it's self-describing about what it needs to
     *  run) depends on these never silently changing after creation. Empty
     *  string means "not yet set" (only valid pre-creation, on a brand-new
     *  draft with no id). See
     *  ARCHITECTURE_MANDATORY_ABF_RETHINK_2026_08_14.md §7 — this reverses
     *  SPEC_MEMORY_IDENTITY_ARCH §4.1a's "presets are provider-agnostic"
     *  decision on purpose, trading cross-provider reusability for
     *  self-contained portability. */
    provider: string;
    model: string;
    instructions: string;
    /** Preserved verbatim through the edit round-trip — ABF v0.2 §2.2
     *  provider-scoped variants, not yet editable through this form (no
     *  authoring UI exists yet). Round-tripping this (rather than
     *  omitting it, which would default to "{}" on save and silently
     *  wipe out any variants an import brought in) is required as of
     *  reagent P1, PR #2523: without it, editing ANY field of an
     *  already-imported bundle through this form would discard its
     *  provider variants, since bundle_memory_upsert's ON CONFLICT UPDATE
     *  unconditionally overwrites the column. */
    instructions_by_provider: string;
    /** Edited as `[{ path, content }]`; serialized to JSON on save. */
    context_files: Array<{ path: string; content: string }>;
    /** Edited as raw JSON string for now (advanced). */
    mcp_servers: string;
    /** Edited as comma-separated ids for now. */
    skills: string;
    /** Preserved from the stored bundle; not surfaced as an editable field yet. */
    is_global?: boolean;
}

/** Empty draft for the "+ New Bundle" flow.
 *
 *  All JSON-array fields default to `"[]"` (not `""`). The backend's
 *  `db_bundles.skills` column is JSON-encoded; a literal `""` would
 *  trip downstream `JSON.parse(skills)` readers. Reagent P1 on
 *  PR #747 (2026-05-08). */
export function emptyDraft(): BundleDraft {
    return {
        id: undefined,
        name: "",
        description: "",
        provider: "",
        model: "",
        instructions: "",
        instructions_by_provider: "{}",
        context_files: [],
        mcp_servers: "[]",
        skills: "[]",
    };
}

/** Hydrate a draft from a stored Memory. JSON fields are parsed; on
 *  parse failure we fall back to safe empties so the UI stays usable
 *  even if the row is malformed. */
export function draftFromBundle(m: Bundle): BundleDraft {
    let context_files: Array<{ path: string; content: string }> = [];
    try {
        const parsed = JSON.parse(m.context_files ?? "[]");
        if (Array.isArray(parsed)) context_files = parsed;
    } catch {
        // Fall back to empty list; user can re-add files.
    }
    return {
        id: m.id,
        name: m.name,
        description: m.description ?? "",
        provider: m.provider ?? "",
        model: m.model ?? "",
        instructions: m.instructions ?? "",
        // Preserved, not parsed — this form has no field that edits
        // per-provider variants yet, so the draft only needs to carry
        // the raw JSON through unchanged (see the field's own doc
        // comment on BundleDraft for why dropping it would be lossy).
        instructions_by_provider:
            m.instructions_by_provider && m.instructions_by_provider.trim().length > 0
                ? m.instructions_by_provider
                : "{}",
        context_files,
        // Both JSON-array fields use the same empty-string-aware
        // fallback. A legacy row with mcp_servers = "" would
        // otherwise load empty into the textarea, looking
        // unconfigured. Reagent P2 (PR #749).
        mcp_servers:
            m.mcp_servers && m.mcp_servers.trim().length > 0 ? m.mcp_servers : "[]",
        skills: m.skills && m.skills.trim().length > 0 ? m.skills : "[]",
        is_global: m.is_global ?? false,
    };
}

/** Serialize a draft into the wire shape for `upsertmemory`.
 *
 *  The backend deserializes directly into the Rust `Memory` struct,
 *  which has no serde defaults for `created_at` / `updated_at`. Send
 *  0 for both — the upsert handler server-sets `created_at = now`
 *  when it sees 0 and always overwrites `updated_at` with now. Codex
 *  P1 (PR #749). */
export function draftToWire(d: BundleDraft): Bundle {
    return {
        id: d.id ?? "",
        name: d.name.trim(),
        description: d.description.trim(),
        // Preserve the global flag so editing a global bundle does not
        // silently strip it (the upsert ON CONFLICT overwrites is_global).
        is_global: d.is_global ?? false,
        // Sent through as-is (readonly-once-set is backend-enforced, not
        // stripped here) — see BundleDraft.provider's doc comment.
        provider: d.provider,
        model: d.model,
        instructions: d.instructions,
        instructions_by_provider: d.instructions_by_provider || "{}",
        context_files: JSON.stringify(d.context_files),
        mcp_servers: d.mcp_servers || "[]",
        // Same JSON-array invariant as mcp_servers — never write an
        // empty string to the skills column. Reagent P1 (PR #747).
        skills: d.skills || "[]",
        created_at: 0,
        updated_at: 0,
    };
}

export class BundleViewModel implements ViewModel {
    viewType = "memory";
    blockId: string;
    nodeModel: BlockNodeModel | null;

    // Cross-window reactivity (SPEC_ARMORY_REACTIVE_UPDATES_2026_09_02.md) —
    // a bundle create/edit/delete made elsewhere refreshes this list without
    // a manual reopen. Same `memories:changed` event GlobalBrainViewModel
    // now also subscribes to; see that model's own comment on why one
    // umbrella event firing a refresh in both tabs is fine, not a bug.
    private unsubChanged: () => void;

    // "layer-group" (not "brain") — matches the ABF tab icon in the Armory
    // rail (armory-view.tsx) so the standalone bundle pane and the Armory nav
    // stay visually consistent; the brain icon is reserved for native memory.
    viewIcon: Accessor<string> = () => "layer-group";
    viewName: Accessor<string>;
    viewText: Accessor<string | HeaderElem[]> = () => "Bundles";
    noPadding: Accessor<boolean> = () => false;

    get viewComponent(): ViewComponent {
        return null; // overridden by the barrel via Object.defineProperty
    }

    blockAtom: Accessor<Block | undefined>;
    /** The specific agent this memory pane belongs to, if any
     *  (`meta.agentId`, same field `IdentityPaneViewModel.agentId` /
     *  `AgentViewModel` read) — closes the DATA GAP documented in
     *  `bundle-summary.tsx`'s module comment. `undefined` when this block
     *  was opened without agent context. */
    agentId: Accessor<string | undefined>;

    private _bundles = createSignal<Bundle[]>([]);
    bundlesAtom: Accessor<Bundle[]> = this._bundles[0];
    setMemories = this._bundles[1];

    private _selectedId = createSignal<string | null>(null);
    selectedIdAtom: Accessor<string | null> = this._selectedId[0];
    setSelectedId = this._selectedId[1];

    private _draft = createSignal<BundleDraft | null>(null);
    draftAtom: Accessor<BundleDraft | null> = this._draft[0];
    setDraft = this._draft[1];

    private _saving = createSignal<boolean>(false);
    savingAtom: Accessor<boolean> = this._saving[0];
    setSaving = this._saving[1];

    private _error = createSignal<string | null>(null);
    errorAtom: Accessor<string | null> = this._error[0];
    setError = this._error[1];

    private _validating = createSignal<boolean>(false);
    validatingAtom: Accessor<boolean> = this._validating[0];
    setValidating = this._validating[1];

    /** Result of the last "Validate" click, or null if never run / cleared
     *  by a subsequent edit. Cleared whenever the draft is replaced
     *  (startNew/startEdit/cancelDraft) or saved — a stale report from a
     *  DIFFERENT bundle must never linger onto the next one. */
    private _validation = createSignal<BundleValidationReport | null>(null);
    validationAtom: Accessor<BundleValidationReport | null> = this._validation[0];
    setValidation = this._validation[1];

    /** Memo: the currently-selected Memory row, or null. */
    selectedAtom: Accessor<Bundle | null>;

    // `nodeModel` is optional: when this ViewModel backs a `view: "memory"`
    // block pane the BlockRegistry passes the real (blockId, nodeModel)
    // pair; when it backs the context-free <BundleManager/> component
    // (window modal / extracted manager) there is no block, so both are
    // absent. The block is used only for the cosmetic header title —
    // every other code path drives off `bundle_*` RPCs and is
    // block-independent.
    constructor(blockId?: string, nodeModel?: BlockNodeModel) {
        this.blockId = blockId ?? "";
        this.nodeModel = nodeModel ?? null;
        this.blockAtom = blockId
            ? getWaveObjectAtom(makeORef("block", blockId))
            : () => undefined;
        this.viewName = createMemo(() => {
            const block = this.blockAtom();
            return (block?.meta?.["frame:title"] as string) ?? "Bundles";
        });
        this.agentId = createMemo(() => {
            const block = this.blockAtom();
            return block?.meta?.["agentId"] as string | undefined;
        });
        this.selectedAtom = createMemo(() => {
            const id = this.selectedIdAtom();
            if (!id) return null;
            return this.bundlesAtom().find((m) => m.id === id) ?? null;
        });

        // Kick off initial load. Errors land in errorAtom for UI surfacing.
        void this.refresh();
        this.unsubChanged = waveEventSubscribe({
            eventType: "memories:changed",
            handler: () => void this.refresh(),
        });
    }

    /** Re-fetch the full list. Called on mount and after each mutation.
     *  Excludes is_system rows — this is the generic Bundle Manager
     *  (Armory "Memories" tab), not the dedicated Global Memory system-tier
     *  editor; showing a system entry here would let a human open it and
     *  have any edit silently rejected by bundle_memory_upsert's guard.
     *  reagent P1, PR #2782 — see
     *  docs/specs/SPEC_GLOBAL_MEMORY_SYSTEM_TIER_2026_08_24.md. */
    async refresh(): Promise<void> {
        try {
            const list = await RpcApi.ListBundlesCommand(TabRpcClient, {});
            this.setMemories(list.filter((m) => !m.is_system));
            this.setError(null);
        } catch (e) {
            this.setError(`Failed to load bundles: ${(e as Error).message ?? e}`);
        }
    }

    /** Open the form for a new memory. Clears any stale error so the
     *  user starts on a clean banner. */
    startNew(): void {
        this.setError(null);
        this.setValidation(null);
        this.setDraft(emptyDraft());
        this.setSelectedId(null);
    }

    /** Open the form for editing an existing memory. Refuses on the blank
     *  singleton because the backend rejects mutations to it anyway —
     *  better to show a disabled UI than to surface a backend error after
     *  the user types. Successful entry clears any stale error from a
     *  previous failed action (e.g. clicking the blank singleton, then
     *  clicking a real memory should not leave the "system-managed"
     *  banner showing alongside the new edit form). Reagent P2 (#747). */
    startEdit(memory: Bundle): void {
        if (memory.is_blank) {
            this.setError("The blank bundle is system-managed and cannot be edited.");
            return;
        }
        this.setError(null);
        this.setValidation(null);
        this.setDraft(draftFromBundle(memory));
        this.setSelectedId(memory.id);
    }

    /** Discard the current draft AND clear any stale error. The save
     *  path leaves an error banner up if it failed; cancelling the
     *  form should also clear it so the read-only view comes back
     *  clean. Reagent P2 (#747). */
    cancelDraft(): void {
        this.setDraft(null);
        this.setError(null);
        this.setValidation(null);
    }

    /** Persist the current draft (creates if id is empty, else updates). */
    async saveDraft(): Promise<void> {
        const draft = this.draftAtom();
        if (!draft) return;
        if (!draft.name.trim()) {
            this.setError("Bundle name is required.");
            return;
        }
        this.setSaving(true);
        this.setError(null);
        try {
            const saved = await RpcApi.UpsertBundleCommand(TabRpcClient, draftToWire(draft));
            // Refresh the list either way — the saved row should appear.
            await this.refresh();
            // Race-condition guard (reagent P1, PR #749 round 6): use
            // OBJECT IDENTITY (=== draft) rather than `!== null`. The
            // user can replace the draft mid-flight by:
            //   - clicking another list item   → cancelDraft → null
            //   - clicking "+ New Bundle"      → startNew → fresh draft
            //   - clicking the Edit button     → startEdit → other draft
            // All three cases must skip the post-save navigation. Only
            // identity-equal-to-our-snapshot means the user is still
            // looking at this save. Round 5 used `!== null` which let
            // the New-Memory and Edit-other cases through, silently
            // discarding the user's new draft.
            if (this.draftAtom() === draft) {
                // Draft still active = no mid-flight replace. Clear it
                // and select the saved row. The refresh ran first so
                // selectedAtom resolves to the saved memory immediately,
                // no empty-state flash. Reagent P2 (PR #749).
                this.setDraft(null);
                this.setValidation(null);
                this.setSelectedId(saved.id);
            }
        } catch (e) {
            this.setError(`Save failed: ${(e as Error).message ?? e}`);
        } finally {
            this.setSaving(false);
        }
    }

    /** Run the structural ABF validator against the current draft — works
     *  on unsaved edits (including a brand-new bundle with no id yet), not
     *  just what's already persisted, since `ValidateBundleCommand` accepts
     *  the same payload shape `saveDraft` sends. Does not save; purely
     *  advisory. */
    async validateDraft(): Promise<void> {
        const draft = this.draftAtom();
        if (!draft) return;
        this.setValidating(true);
        this.setError(null);
        try {
            const report = await RpcApi.ValidateBundleCommand(TabRpcClient, draftToWire(draft));
            // Same identity-equality race guard as saveDraft (reagent P1,
            // PR #749 round 6 originally, now PR #2532): Cancel, "+ New
            // Bundle", and another row's Edit button are all still
            // clickable while this request is in flight (validatingAtom
            // does not disable them). Without this check, a report for
            // the OLD draft would land on whatever draft the user has
            // switched to by the time the RPC resolves — the exact
            // staleness this feature exists to prevent.
            if (this.draftAtom() === draft) {
                this.setValidation(report);
            }
        } catch (e) {
            this.setError(`Validate failed: ${(e as Error).message ?? e}`);
        } finally {
            this.setValidating(false);
        }
    }

    async deleteMemory(id: string): Promise<void> {
        const target = this.bundlesAtom().find((m) => m.id === id);
        if (target?.is_blank) {
            this.setError("The blank bundle is system-managed and cannot be deleted.");
            return;
        }
        this.setError(null);
        try {
            await RpcApi.DeleteBundleCommand(TabRpcClient, { id });
            if (this.selectedIdAtom() === id) this.setSelectedId(null);
            this.setDraft(null);
            await this.refresh();
        } catch (e) {
            this.setError(`Delete failed: ${(e as Error).message ?? e}`);
        }
    }

    /** ViewModel teardown — Solid signals are GC'd with the instance. */
    dispose(): void {
        this.unsubChanged();
    }
}

// -- ABF v0.2 §2.2 per-provider instructions: authoring-model seam --------
//
// The draft carries `instructions_by_provider` as a RAW JSON string, not a
// parsed structure, and that is deliberate: reagent P1 on PR #2523 found that
// editing any field of an imported bundle silently wiped its provider variants,
// because `bundle_memory_upsert`'s ON CONFLICT UPDATE overwrites the column
// unconditionally. Round-tripping the raw string is what fixed it.
//
// The authoring UI needs a list, so these three functions are the seam. The
// wipe-safety property has to be enforced here: `parse` reports malformed input
// as `malformed` WITHOUT yielding an empty list, so the UI can refuse to edit
// and keep the original string byte-for-byte rather than serializing `{}` over
// variants it merely failed to understand.

export interface ProviderInstruction {
    provider: string;
    content: string;
}

export interface ParsedInstructionsByProvider {
    entries: ProviderInstruction[];
    /** True when the stored value is not a flat object of string values. The
     *  caller MUST leave the raw string untouched in this case — an empty
     *  `entries` here means "could not read", never "there is nothing". */
    malformed: boolean;
}

/** Parse the stored JSON object into sorted, editable entries.
 *
 *  Empty/whitespace/`{}` are a legitimately empty set, not malformed — that is
 *  the normal state of a bundle nobody has added a variant to. Anything that is
 *  not an object of strings (array, scalar, null, or an object with a
 *  non-string value) is malformed. */
export function parseInstructionsByProvider(raw: string): ParsedInstructionsByProvider {
    const trimmed = (raw ?? "").trim();
    if (trimmed.length === 0) return { entries: [], malformed: false };
    let parsed: unknown;
    try {
        parsed = JSON.parse(trimmed);
    } catch {
        return { entries: [], malformed: true };
    }
    if (parsed === null || typeof parsed !== "object" || Array.isArray(parsed)) {
        return { entries: [], malformed: true };
    }
    const entries: ProviderInstruction[] = [];
    for (const [provider, content] of Object.entries(parsed as Record<string, unknown>)) {
        // Coercing a non-string here would rewrite the user's data on the next
        // save; refusing to parse leaves it exactly as imported.
        if (typeof content !== "string") return { entries: [], malformed: true };
        entries.push({ provider, content });
    }
    entries.sort((a, b) => a.provider.localeCompare(b.provider));
    return { entries, malformed: false };
}

/** Serialize edited entries back to the stored JSON-object form.
 *
 *  Provider keys are trimmed (a stray space would become a distinct variant,
 *  and on export a distinct directory); content is never trimmed, since leading
 *  and trailing whitespace in a system prompt is the author's business. Rows
 *  with a blank key are dropped — they cannot be exported and would serialize
 *  as a `""` key. */
export function serializeInstructionsByProvider(entries: ProviderInstruction[]): string {
    // Null prototype, deliberately: on a plain `{}`, assigning the key
    // "__proto__" hits Object.prototype's inherited legacy setter instead of
    // creating an own property, so JSON.stringify silently omits it. A bundle
    // that imported a `__proto__` override would lose it the moment the user
    // edited ANY row — the same wipe class the raw-string round-trip exists to
    // prevent, arriving through a different door (codex P2, #3063). Provider
    // keys are free text and can arrive from an untrusted .abf, so this is
    // reachable, not theoretical.
    const out: Record<string, string> = Object.create(null);
    for (const { provider, content } of entries) {
        const key = provider.trim();
        if (key.length === 0) continue;
        out[key] = content;
    }
    return JSON.stringify(out);
}

/** Faithful port of `sanitize_context_relative_path` (bundle_export.rs).
 *
 *  Returns the sanitized path a key would be exported under, or `null` if the
 *  backend would reject it (in which case export SKIPS the variant with a
 *  warning). Kept deliberately mechanical rather than "improved", because its
 *  only job is to agree with the Rust — a stricter frontend rule warns about
 *  keys that in fact work, and a looser one stays silent about keys that get
 *  dropped. Mirrors, in order: empty, `\` normalized to `/`, leading `/`
 *  rejected, any `:` rejected, `..` segments rejected, `.`/empty segments
 *  skipped, and "nothing left" rejected.
 *
 *  NOTE what this does NOT reject, because the backend does not either:
 *  nested segments (`a/b` is legal and exports to instructions/a/b/AGENTS.md)
 *  and control characters. */
export function sanitizeProviderKey(provider: string): string | null {
    // The stored key is the trimmed one (see serializeInstructionsByProvider),
    // so validate what will actually be stored, not what was typed.
    const key = provider.trim();
    if (key.length === 0) return null;
    const normalized = key.split("\\").join("/");
    if (normalized.startsWith("/") || normalized.includes(":")) return null;
    const parts: string[] = [];
    for (const component of normalized.split("/")) {
        if (component === "" || component === ".") continue;
        if (component === "..") return null;
        parts.push(component);
    }
    if (parts.length === 0) return null;
    return parts.join("/");
}

/** Why a provider key would not survive export, or `null` if it is fine.
 *
 *  Advisory. The key IS still saved to the bundle (serialization drops only
 *  blank keys) — what it loses is the export: `bundle_export.rs` skips any key
 *  this rejects, with a warning, so the variant silently vanishes from the
 *  `.abf`. Better to say so while the user is typing than at export time. */
export function providerKeyProblem(provider: string): string | null {
    const key = provider.trim();
    if (key.length === 0) return "Provider key cannot be empty.";
    if (sanitizeProviderKey(key) !== null) return null;
    const normalized = key.split("\\").join("/");
    if (normalized.startsWith("/")) {
        return "Provider key cannot start with a path separator.";
    }
    if (normalized.includes(":")) return "Provider key cannot contain a colon.";
    if (normalized.split("/").some((c) => c === "..")) {
        return 'Provider key cannot contain a ".." segment.';
    }
    return "Provider key has no usable path segment.";
}

/** Keys that collide once sanitized.
 *
 *  Compares SANITIZED paths, not raw strings, because that is what collides in
 *  `bundle_export.rs`: it keys `seen_safe_providers` on the sanitized value and
 *  keeps only the sorted-first of a colliding set, warning about the rest. So
 *  `a/b` and `a\b` are the same export path and one of them is silently lost —
 *  comparing raw strings would miss exactly that case. Keys the backend
 *  rejects outright are excluded; `providerKeyProblem` already covers those. */
export function duplicateProviderKeys(entries: ProviderInstruction[]): string[] {
    // Order is part of the contract, not an implementation detail:
    // `bundle_export.rs` sorts the RAW keys ascending and keeps the FIRST of
    // each colliding set, skipping the rest with a warning. Walking the
    // caller's array order instead marks whichever row the form happens to
    // hold first — which can be the row export actually KEEPS, so the warning
    // lands on the survivor while the row that really gets dropped shows
    // nothing at all (reagent P1, #3063).
    //
    // Plain `<`/`>` rather than localeCompare, to match Rust's `String::cmp`
    // (byte order). localeCompare is locale-aware and would disagree — e.g. it
    // ignores punctuation differences that decide this exact comparison.
    // Blank-CONTENT rows are excluded before collision resolution, because
    // export excludes them first too: `if content.trim().is_empty() { continue }`
    // runs BEFORE the sanitize/collision check, so such a row never occupies a
    // collision slot. Ignoring content here re-created the same wrong-row
    // warning the sort above removes — with {"a/b": ""} and {"a\b": "real"},
    // export silently skips the empty "a/b" and writes "a\b" with no collision
    // at all, while we flagged "a\b" as the casualty (reagent P1 round 3,
    // #3063).
    //
    // Deliberately NOT also warning "this row's empty content will not be
    // exported": a row the user just added is empty by definition, so that
    // fires on every new row before they have typed anything.
    const sortedKeys = entries
        .filter((e) => e.content.trim().length > 0)
        .map((e) => e.provider.trim())
        .filter((key) => key.length > 0)
        .sort((a, b) => (a < b ? -1 : a > b ? 1 : 0));

    const seen = new Set<string>();
    const dupes = new Set<string>();
    for (const key of sortedKeys) {
        const safe = sanitizeProviderKey(key);
        if (safe === null) continue;
        if (seen.has(safe)) dupes.add(key);
        else seen.add(safe);
    }
    return [...dupes].sort();
}
