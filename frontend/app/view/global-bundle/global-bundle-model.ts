// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// GlobalBundleViewModel — drives the Global section of the Armory "Memory" tab
// (Phase 4 of SPEC_ARMORY_NAMING_CONSOLIDATION_2026_09_09.md folds it into the
// Bundles tab): the workspace-wide global bundles that every agent inherits at
// launch.
//
// A "section" is a Memory bundle with is_global=true. The global bundles is
// the ordered list of those sections; their instructions concatenate into
// each agent's startup instructions file at launch (backend:
// format_global_bundle_block) — CLAUDE.md, AGENTS.md, GEMINI.md, or similar
// depending on the agent's provider (agent_config.rs's build_config_files,
// resolved per-provider since
// docs/specs/SPEC_PROVIDER_AWARE_STARTUP_INSTRUCTIONS_2026_08_24.md; see
// `filenameGroupsAtom`/`noFileProvidersAtom` below for that mapping — no
// longer rendered by GlobalBundleManager as of
// docs/specs/SPEC_ARMORY_GLOBAL_MEMORY_DECLUTTER_2026_09_15.md §3 (the user
// didn't want it in that view), kept here since the mapping itself is still
// real, tested domain logic that may be surfaced elsewhere later). Section
// order is the sort_order column, mutated via reorderglobalbrain.
//
// This model is block-free (same shape as BundleViewModel) and drives off
// the bundle_* RPCs. Mutations refresh the list afterwards, and it
// subscribes to memories:changed so an agent's GlobalMemoryWrite (or another
// window) shows up live.
//
// Since 2026-09-24 the view is a tile grid that opens one entry at a time in
// a full view (SPEC_MEMORY_FOLLOWS_THE_AGENT_2026_09_24.md §2.3/§2.4), so
// editing state is ONE MemoryDraftModel for whatever is open — the inline
// per-card editors and their two parallel sets of draft signals (ordinary vs
// system) are gone. A save carries `base_sha256` (SHA-256 of
// `name + "\0" + instructions`, the server's own content_hash), and a live
// change never replaces an open draft.
//
// Spec: docs/specs/archive/SPEC_TRUST_CENTER_GLOBAL_BRAIN_2026_06_19.md.

import { createMemo, createSignal, type Accessor } from "solid-js";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { muxEventSubscribe } from "@/app/store/mps";
import { PROVIDERS } from "@/app/view/agent/providers";
import type { Bundle, GlobalMemoryVersionMeta } from "@/app/store/rpc-api";
import { MemoryDraftModel } from "@/app/view/memory-editor/memory-draft-model";
import type { MemoryHistorySource } from "@/app/view/memory-editor/memory-history-model";
import { sha256Hex } from "@/util/sha256";

/** What the Global Memory full view is showing — one entry, a new-entry
 *  draft, the read-only CLAUDE.md, or the combined preview. `null` = the
 *  tile grid. */
export type GlobalMemoryView =
    | { kind: "entry"; id: string }
    | { kind: "new" }
    | { kind: "claude-config" }
    | { kind: "preview" };

/** The editable part of an entry. */
export interface GlobalMemoryDraft {
    name: string;
    instructions: string;
}

/** The server's `bundle_versions::content_hash` — what `base_sha256` means
 *  for a Global Memory save. */
export function globalMemoryContentHash(v: GlobalMemoryDraft): Promise<string> {
    return sha256Hex(`${v.name}\0${v.instructions}`);
}

/** `globalmemory:history/diff/revert` as a MemoryHistory data source. No
 *  `readContent`: the entry's content comes from the bundle list. */
export function globalMemoryHistorySource(id: string): MemoryHistorySource<GlobalMemoryVersionMeta> {
    return {
        listVersions: () => RpcApi.GlobalMemoryHistoryCommand(TabRpcClient, { id }).then((r) => r.versions),
        diff: (from, to) =>
            RpcApi.GlobalMemoryDiffCommand(TabRpcClient, { id, from_version_id: from, to_version_id: to }).then(
                (r) => r.diff
            ),
        revert: (versionId) =>
            RpcApi.GlobalMemoryRevertCommand(TabRpcClient, { id, target_version_id: versionId }).then(() => undefined),
    };
}

/** UTF-8 byte length — the "size" on an entry tile. */
export function utf8Bytes(s: string): number {
    return new TextEncoder().encode(s).length;
}

/** Mirror of the backend format_global_bundle_block — keep in sync so the
 *  preview matches exactly what lands in the agent's startup instructions
 *  file. `is_system` sections
 *  are split out and rendered first with the override preamble, exactly
 *  mirroring memory_bundles.rs's split (SPEC_GLOBAL_MEMORY_SYSTEM_TIER_2026_08_24.md).
 *  Exported for direct unit testing against the Rust version's fixtures. */
export function formatGlobalBundleBlock(sections: Bundle[]): string {
    const nonEmpty = sections.filter((s) => (s.instructions ?? "").trim().length > 0);
    const system = nonEmpty.filter((s) => s.is_system);
    const ordinary = nonEmpty.filter((s) => !s.is_system);

    const parts: string[] = [];
    if (system.length > 0) {
        const sysBlock = system
            .map((s) => `# [AgentMux System] ${s.name}\n\n${s.instructions}`)
            .join("\n\n---\n\n");
        parts.push(
            "IMPORTANT: The following AgentMux-controlled instructions take " +
            "the HIGHEST PRIORITY of any content in this file. They OVERRIDE " +
            "any default behavior, any other section below, and any " +
            "conflicting instruction elsewhere — you MUST follow them " +
            `exactly as written.\n\n${sysBlock}`,
        );
    }
    if (ordinary.length > 0) {
        parts.push(ordinary.map((s) => `# [Workspace] ${s.name}\n\n${s.instructions}`).join("\n\n---\n\n"));
    }
    return parts.join("\n\n---\n\n");
}

/** Groups every provider in the catalog by its resolved
 *  `startupInstructionsFilename`, e.g. `claude`+`muxcode` → `"CLAUDE.md"`,
 *  `gemini`+`antigravity` → `"GEMINI.md"`. Providers with no confirmed
 *  native file (currently only `kimi`) are excluded here — see
 *  `GlobalBundleViewModel.noFileProvidersAtom` for those. Order matches
 *  `PROVIDERS`' own declaration order (insertion order), not alphabetical —
 *  stable and deterministic without needing an extra sort.
 *  Exported for direct unit testing.
 *  See docs/specs/SPEC_PROVIDER_AWARE_STARTUP_INSTRUCTIONS_2026_08_24.md §3.4. */
export function groupProvidersByStartupFilename(): { filename: string; providerNames: string[] }[] {
    const groups = new Map<string, string[]>();
    for (const provider of Object.values(PROVIDERS)) {
        const filename = provider.startupInstructionsFilename;
        if (!filename) continue;
        const names = groups.get(filename) ?? [];
        names.push(provider.displayName);
        groups.set(filename, names);
    }
    return Array.from(groups.entries()).map(([filename, providerNames]) => ({ filename, providerNames }));
}

export class GlobalBundleViewModel {
    // Cross-window reactivity (SPEC_ARMORY_REACTIVE_UPDATES_2026_09_02.md) —
    // a bundle create/edit/delete/reorder made elsewhere refreshes this view
    // without a manual reopen, same pattern BundleMcpModel/BundleSkillModel
    // already use for mcp:changed/skills:changed.
    private unsubChanged: () => void;

    private _all = createSignal<Bundle[]>([]);
    /** Every bundle (global + per-agent), used to derive sections + candidates. */
    allAtom: Accessor<Bundle[]> = this._all[0];
    private setAll = this._all[1];

    /** The open full view; `null` = the tile grid. */
    private _view = createSignal<GlobalMemoryView | null>(null);
    viewAtom: Accessor<GlobalMemoryView | null> = this._view[0];
    private setView = this._view[1];

    /** The one draft for whatever entry is open (or being created). */
    readonly draft: MemoryDraftModel<GlobalMemoryDraft>;

    /** Bumped on every refresh, so an open full view can reload its history
     *  in place when the entry changes underneath it. */
    private _refreshNonce = createSignal(0);
    refreshNonceAtom: Accessor<number> = this._refreshNonce[0];
    private setRefreshNonce = this._refreshNonce[1];

    /** The entry the full view has open, if it (still) exists. */
    openEntryAtom: Accessor<Bundle | null>;

    private _error = createSignal<string | null>(null);
    errorAtom: Accessor<string | null> = this._error[0];
    setError = this._error[1];

    /** Global sections, in injection order (sort_order, then name) —
     *  includes system rows (they're always is_global too). */
    sectionsAtom: Accessor<Bundle[]>;
    /** The AgentMux-controlled, highest-priority subset of sectionsAtom. */
    systemSectionsAtom: Accessor<Bundle[]>;
    /** sectionsAtom minus systemSectionsAtom — what the ordinary editor list renders. */
    ordinarySectionsAtom: Accessor<Bundle[]>;
    /** Combined startup-instructions-file preview block. */
    previewAtom: Accessor<string>;
    /** Providers grouped by resolved startup-instructions filename, e.g.
     *  [{filename: "CLAUDE.md", providerNames: ["Claude Code", "Mux Code"]}, ...].
     *  Static — derived from the PROVIDERS catalog, not per-workspace agent
     *  data, so an operator sees the full mapping up front regardless of
     *  which providers currently have a launched agent. See
     *  docs/specs/SPEC_PROVIDER_AWARE_STARTUP_INSTRUCTIONS_2026_08_24.md §3.4. */
    filenameGroupsAtom: Accessor<{ filename: string; providerNames: string[] }[]>;
    /** Display names of providers with no confirmed startup-instructions
     *  file (currently just Kimi). Not currently rendered by
     *  GlobalBundleManager (SPEC_ARMORY_GLOBAL_MEMORY_DECLUTTER_2026_09_15.md
     *  §3) — kept as tested domain logic, same as filenameGroupsAtom above. */
    noFileProvidersAtom: Accessor<string[]>;

    // Backs the "Claude Code provider config" section — a read-only
    // reference display of the CLAUDE.md in the isolated config dir a
    // spawned agent actually launches with. NOT the file AgentMux's own
    // Global Memory composes into (that's <agent working_directory>/CLAUDE.md,
    // a per-agent path this doesn't cover — already previewed accurately via
    // previewAtom above). See
    // docs/specs/SPEC_SURFACE_CLAUDE_GLOBAL_CONFIG_2026_08_24.md §7.
    //
    // Was a PAIR of signals until 2026-09-01: the sibling ~/.claude host-CLI
    // config signal went away with its block
    // (SPEC_ARMORY_DROP_HOST_CLI_CONFIG_BLOCK_2026_09_01.md), once a spawned
    // agent was proven never to read that file.

    private _claudeGlobalConfig = createSignal<{ path: string; content: string | null; exists: boolean } | null>(null);
    /** The CLAUDE.md at AgentMux's shared Claude provider config dir — the
     *  path a spawned Claude agent's CLAUDE_CONFIG_DIR points at by
     *  DEFAULT (identity-bound agents use a separate, per-identity dir not
     *  covered here). Hand-maintained, independent of AgentMux's own
     *  Global Memory. `null` until the fetch resolves; fetched once at
     *  construction, not re-fetched on `memories:changed` (nothing in this
     *  app can write to this file, so there's no event to react to). See
     *  docs/specs/SPEC_SURFACE_CLAUDE_GLOBAL_CONFIG_2026_08_24.md §5. */
    claudeGlobalConfigAtom: Accessor<{ path: string; content: string | null; exists: boolean } | null> =
        this._claudeGlobalConfig[0];
    private setClaudeGlobalConfig = this._claudeGlobalConfig[1];

    constructor() {
        this.sectionsAtom = createMemo(() =>
            this.allAtom()
                .filter((m) => m.is_global && !m.is_blank)
                .sort(
                    (a, b) =>
                        (a.sort_order ?? 0) - (b.sort_order ?? 0) ||
                        a.name.localeCompare(b.name),
                ),
        );
        this.systemSectionsAtom = createMemo(() =>
            this.sectionsAtom().filter((m) => m.is_system),
        );
        this.ordinarySectionsAtom = createMemo(() =>
            this.sectionsAtom().filter((m) => !m.is_system),
        );
        this.previewAtom = createMemo(() => formatGlobalBundleBlock(this.sectionsAtom()));
        this.openEntryAtom = createMemo(() => {
            const v = this.viewAtom();
            if (v?.kind !== "entry") return null;
            return this.sectionsAtom().find((m) => m.id === v.id) ?? null;
        });
        this.draft = new MemoryDraftModel<GlobalMemoryDraft>({
            hash: globalMemoryContentHash,
            equals: (a, b) => a.name === b.name && a.instructions === b.instructions,
            save: (value, base) => this.persist(value, base),
            fetchCurrent: async () => {
                await this.refresh();
                return this.currentOpenDraft();
            },
        });
        this.filenameGroupsAtom = createMemo(() => groupProvidersByStartupFilename());
        this.noFileProvidersAtom = createMemo(() =>
            Object.values(PROVIDERS)
                .filter((p) => !p.startupInstructionsFilename)
                .map((p) => p.displayName),
        );
        void this.refresh();
        void this.fetchClaudeGlobalConfig();
        this.unsubChanged = muxEventSubscribe({
            eventType: "memories:changed",
            handler: () => void this.refresh(),
        });
    }

    /** Fetches the shared Claude provider config's CLAUDE.md once. Failure
     *  is silent (leaves the atom `null`, so the block just doesn't render)
     *  rather than surfacing
     *  through `errorAtom` — this is a supplementary, read-only display, not
     *  something that should block or alarm-color the rest of the Global
     *  Memory tab if it can't be read (e.g. a permissions issue on this one
     *  file shouldn't look like Global Memory itself is broken). */
    private async fetchClaudeGlobalConfig(): Promise<void> {
        try {
            const cfg = await RpcApi.GetClaudeGlobalConfigCommand(TabRpcClient, {});
            this.setClaudeGlobalConfig(cfg);
        } catch {
            // Silent — see doc comment above.
        }
    }


    async refresh(): Promise<void> {
        try {
            const list = await RpcApi.ListBundlesCommand(TabRpcClient, {});
            this.setAll(list);
            this.setError(null);
            this.setRefreshNonce((n) => n + 1);
            // The open entry may have changed underneath its view: hand the
            // new content to the draft model, which updates silently when
            // there's no unsaved draft and raises the banner (only on a real
            // hash change) when there is.
            if (this.viewAtom()?.kind === "entry") {
                void this.draft.observeExternal(this.currentOpenDraft());
            }
        } catch (e) {
            this.setError(`Failed to load global bundles: ${(e as Error).message ?? e}`);
        }
    }

    /** The open entry's saved name/instructions, or null if it's gone. */
    private currentOpenDraft(): GlobalMemoryDraft | null {
        const entry = this.openEntryAtom();
        return entry ? { name: entry.name, instructions: entry.instructions ?? "" } : null;
    }

    /** Open a full view. Callers guard a dirty draft first (`draft.dirtyAtom`). */
    open(view: GlobalMemoryView): void {
        this.draft.cancel();
        this.setError(null);
        this.setView(view);
        if (view.kind === "entry") {
            const entry = this.openEntryAtom();
            // Seed the base so the first save after "Edit" is conditional.
            if (entry) void this.draft.observeExternal({ name: entry.name, instructions: entry.instructions ?? "" });
        } else if (view.kind === "new") {
            this.draft.startNew({ name: "", instructions: "" });
        }
    }

    /** Back to the tile grid. Callers guard a dirty draft first. */
    close(): void {
        this.draft.cancel();
        this.setError(null);
        this.setView(null);
    }

    /** "Edit" in an entry's full view. */
    startEdit(): void {
        const current = this.currentOpenDraft();
        if (current) this.draft.startEdit(current);
    }

    /** Save the open draft — see `persist`. Resolves true on success. The
     *  refresh runs AFTER the draft model has left editing, so the saved
     *  content lands as a quiet rebase, never as a "changed" banner against
     *  the draft that produced it. */
    async save(): Promise<boolean> {
        const draft = this.draft.draftAtom();
        if (draft && !draft.name.trim()) {
            this.draft.setError("Name is required.");
            return false;
        }
        const ok = await this.draft.save();
        if (ok) await this.refresh();
        return ok;
    }

    /** The draft model's save callback. A new entry is created and appended
     *  to the order, then opened; an existing one goes through its own
     *  tier's (still backend-isolated) upsert RPC with `base_sha256`. */
    private async persist(value: GlobalMemoryDraft, base: string | null): Promise<void> {
        const view = this.viewAtom();
        const name = value.name.trim();
        const instructions = value.instructions;
        if (view?.kind === "new") {
            const saved = await RpcApi.UpsertBundleCommand(TabRpcClient, {
                id: "",
                name,
                is_global: true,
                instructions,
            });
            // Append the new section to the end of the order.
            // ordinarySectionsAtom, not sectionsAtom — see move()'s doc
            // comment below (reagent P1, PR #2782).
            const order = [...this.ordinarySectionsAtom().map((s) => s.id), saved.id];
            await RpcApi.ReorderGlobalBundlesCommand(TabRpcClient, { ids: order });
            await this.refresh();
            this.setView({ kind: "entry", id: saved.id });
            return;
        }
        const existing = this.openEntryAtom();
        if (!existing) throw new Error("Memory no longer exists.");
        const baseField = base !== null ? { base_sha256: base } : {};
        if (existing.is_system) {
            // The dedicated upsertsystemmemory command — never
            // UpsertBundleCommand (SPEC_GLOBAL_MEMORY_SYSTEM_TIER_2026_08_24.md §3.5).
            await RpcApi.UpsertSystemBundleCommand(TabRpcClient, { ...existing, name, instructions, ...baseField });
        } else {
            await RpcApi.UpsertBundleCommand(TabRpcClient, {
                ...existing,
                name,
                instructions,
                is_global: true,
                ...baseField,
            });
        }
    }

    /** Remove a section from the global bundles (clears is_global). The bundle itself
     *  is kept — it stays available in the Memories tab. */
    async remove(id: string): Promise<void> {
        const bundle = this.allAtom().find((m) => m.id === id);
        if (!bundle) return;
        this.setError(null);
        try {
            await RpcApi.UpsertBundleCommand(TabRpcClient, { ...bundle, is_global: false });
            if (this.openEntryAtom()?.id === id) this.close();
            await this.refresh();
        } catch (e) {
            this.setError(`Remove failed: ${(e as Error).message ?? e}`);
        }
    }

    /** Move a section one slot earlier/later and persist the new order. */
    async move(id: string, dir: -1 | 1): Promise<void> {
        // ordinarySectionsAtom, not sectionsAtom — the manager's up/down
        // buttons are indexed against ordinarySectionsAtom (its length
        // bounds i()), and the backend's reorder command silently skips
        // is_system ids. Building this list from the combined sectionsAtom
        // (unsorted with system-first) could swap an ordinary row with an
        // adjacent system id, sending a reorder payload where the backend
        // drops the system id and the two ordinary rows' relative order
        // never actually changes — a button the UI enabled doing nothing.
        // reagent P1, PR #2782.
        const ids = this.ordinarySectionsAtom().map((s) => s.id);
        const i = ids.indexOf(id);
        const j = i + dir;
        if (i === -1 || j < 0 || j >= ids.length) return;
        [ids[i], ids[j]] = [ids[j], ids[i]];
        await this.persistOrder(ids);
    }

    /** Drag-to-reorder: move `id` to `targetId`'s slot among the ORDINARY
     *  entries (system entries never reorder — see move()). No-op for an id
     *  that isn't ordinary or a drop onto itself. */
    async moveTo(id: string, targetId: string): Promise<void> {
        const ids = this.ordinarySectionsAtom().map((s) => s.id);
        const from = ids.indexOf(id);
        const to = ids.indexOf(targetId);
        if (from === -1 || to === -1 || from === to) return;
        ids.splice(from, 1);
        ids.splice(to, 0, id);
        await this.persistOrder(ids);
    }

    private async persistOrder(ids: string[]): Promise<void> {
        this.setError(null);
        try {
            await RpcApi.ReorderGlobalBundlesCommand(TabRpcClient, { ids });
            await this.refresh();
        } catch (e) {
            this.setError(`Reorder failed: ${(e as Error).message ?? e}`);
        }
    }

    dispose(): void {
        this.unsubChanged();
    }

    // ---- System-tier — see
    // docs/specs/SPEC_GLOBAL_MEMORY_SYSTEM_TIER_2026_08_24.md §3.5. No
    // promote/reorder/move here: a system entry has nowhere to be promoted
    // FROM (it's created directly), and its position is always first,
    // enforced server-side regardless of what a reorder call would send.

    /** Delete a system entry outright via the dedicated
     *  deletesystemmemory command — never DeleteBundleCommand. Unlike the
     *  ordinary tier's `remove()`, there's no "demote and keep in
     *  Memories" fallback: a system entry has no life outside this tier. */
    async removeSystem(id: string): Promise<void> {
        this.setError(null);
        try {
            await RpcApi.DeleteSystemBundleCommand(TabRpcClient, { id });
            if (this.openEntryAtom()?.id === id) this.close();
            await this.refresh();
        } catch (e) {
            this.setError(`Remove failed: ${(e as Error).message ?? e}`);
        }
    }
}
